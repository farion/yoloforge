#!/usr/bin/env python3
"""Run a Factory client through a verified OpenSSH host alias and supervised tunnel."""
import argparse
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time


def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def interrupted(*_):
    raise KeyboardInterrupt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, help="Existing OpenSSH Host alias")
    parser.add_argument("--ssh-config", help="Optional explicit OpenSSH config file")
    parser.add_argument("--local-port", type=int, default=17432)
    parser.add_argument("--remote-port", type=int, default=7432)
    parser.add_argument("--via-internet", action="store_true",
                        help="Mark this SSH route as an internet connection in the Desktop overview")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.@-]*", args.host):
        parser.error("Invalid OpenSSH alias")
    if not all(1 <= port <= 65535 for port in (args.local_port, args.remote_port)):
        parser.error("Ports must be between 1 and 65535")
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    command = command or ["yoloforge"]
    # Keep the control socket name short enough for Unix sockets.
    parent = os.environ.get("XDG_RUNTIME_DIR", "/tmp/opencode")
    Path(parent).mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="yf-ssh-", dir=parent) as directory:
        control = str(Path(directory) / "ctl")
        base = ["ssh"] + (["-F", args.ssh_config] if args.ssh_config else [])
        ssh = subprocess.Popen([*base, "-M", "-S", control, "-N", "-T",
                                "-o", "ExitOnForwardFailure=yes", "-o", "BatchMode=yes",
                                "-o", "StrictHostKeyChecking=yes", "-o", "ServerAliveInterval=15",
                                "-o", "ServerAliveCountMax=3", "-L",
                                f"127.0.0.1:{args.local_port}:127.0.0.1:{args.remote_port}", args.host])
        child = None
        previous = signal.signal(signal.SIGTERM, interrupted)
        try:
            deadline = time.monotonic() + 30
            while True:
                if ssh.poll() is not None:
                    raise RuntimeError("SSH tunnel failed; check host key and SSH access")
                if Path(control).exists() and subprocess.run([*base, "-S", control, "-O", "check", args.host],
                                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3).returncode == 0:
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError("SSH tunnel did not become ready")
                time.sleep(0.1)
            endpoint = f"http://127.0.0.1:{args.local_port}"
            env = os.environ.copy()
            env.pop("YOLOFORGE_INTERNET_TUNNEL_ENDPOINT", None)
            if args.via_internet:
                env["YOLOFORGE_INTERNET_TUNNEL_ENDPOINT"] = endpoint
            child = subprocess.Popen([command[0], "--server", endpoint, *command[1:]], env=env)
            while child.poll() is None:
                if ssh.poll() is not None:
                    raise RuntimeError("SSH connection ended; Factory client stopped, remote work ownership unchanged")
                time.sleep(0.1)
            return child.returncode
        finally:
            signal.signal(signal.SIGTERM, previous)
            stop(child)
            stop(ssh)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)

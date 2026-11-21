#!/usr/bin/env python3
"""Real OpenSSH tunnel to an isolated container; never reads user SSH configuration."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True).strip()


def main():
    name = "yf-ssh-test-" + uuid.uuid4().hex[:12]
    Path("/tmp/opencode").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=name, dir="/tmp/opencode") as directory:
        directory = Path(directory)
        for key in ("client", "host", "wrong"):
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(directory / key)], check=True)
        token = directory / "token"
        token.write_text(uuid.uuid4().hex * 2)
        token.chmod(0o444)
        try:
            docker("run", "-d", "--name", name, "-p", "127.0.0.1::2222", "--mount", f"type=bind,src={token},dst=/bootstrap,readonly",
                   "-e", "FACTORY_TOKEN_FILE=/bootstrap", "yoloforge-server:development")
            docker("run", "-d", "--name", name + "-ssh", "--network", "container:" + name,
                   "--mount", f"type=bind,src={directory},dst=/keys,readonly", "yoloforge-ssh-test:development")
            ports = json.loads(docker("inspect", "--format", '{{json .NetworkSettings.Ports}}', name))
            port = ports["2222/tcp"][0]["HostPort"]
            deadline = time.monotonic() + 20
            while True:
                try:
                    with socket.create_connection(("127.0.0.1", int(port)), timeout=1) as sock:
                        if sock.recv(256).startswith(b"SSH-"):
                            break
                except OSError:
                    pass
                if time.monotonic() > deadline:
                    raise RuntimeError("SSH fixture not ready")
                time.sleep(0.2)
            known = directory / "known_hosts"
            known.write_text(f"[127.0.0.1]:{port} " + (directory / "host.pub").read_text())
            config = directory / "config"
            config.write_text(f"Host factory-test\n HostName 127.0.0.1\n Port {port}\n User root\n IdentityFile {directory / 'client'}\n IdentitiesOnly yes\n UserKnownHostsFile {known}\n GlobalKnownHostsFile /dev/null\n")
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                local_port = sock.getsockname()[1]
            env = {**os.environ, "FACTORY_TOKEN_FILE": str(token)}
            env.pop("FACTORY_TOKEN", None)
            command = ["python3", str(ROOT / "scripts/factory-connect.py"), "--host", "factory-test", "--ssh-config", str(config),
                       "--local-port", str(local_port), "--", str(ROOT / "target/debug/factory"), "--snapshot"]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=40)
            assert result.returncode == 0, result.stderr
            assert json.loads(result.stdout)["protocol_version"] == 1
            known.write_text(f"[127.0.0.1]:{port} " + (directory / "wrong.pub").read_text())
            rejected = subprocess.run(command, env=env, capture_output=True, text=True, timeout=40)
            assert rejected.returncode != 0 and "HOST IDENTIFICATION HAS CHANGED" in rejected.stderr
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", local_port))
            print("PASS authenticated Factory through real SSH; changed host key rejected; tunnel cleaned up")
        finally:
            subprocess.run(["docker", "rm", "-f", name + "-ssh", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()

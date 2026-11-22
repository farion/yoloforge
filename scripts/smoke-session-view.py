#!/usr/bin/env python3
"""Read-only PTY acceptance against an existing Factory with an OpenCode session.

Uses FACTORY_TOKEN / FACTORY_TOKEN_FILE; opens the first project's first session.
Run with: uv run --with pyte python scripts/smoke-session-view.py
"""
import argparse
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import termios
import time
import codecs
import json
import urllib.request
import pyte

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--server", default="http://127.0.0.1:17432")
parser.add_argument("--native", action="store_true", help="Open the original OpenCode terminal and verify return to YoloForge")
parser.add_argument("--missing-cli", action="store_true", help="Verify terminal restoration if the local OpenCode executable is missing")
args = parser.parse_args()
if args.missing_cli:
    args.native = True
env = {**os.environ, "TERM": "xterm-256color"}
expected_title = None
if args.native:
    token = env.get("FACTORY_TOKEN") or Path(env["FACTORY_TOKEN_FILE"]).read_text().strip()
    req = urllib.request.Request(args.server + "/v1/snapshot", headers={"Authorization": "Bearer " + token, "x-factory-protocol": "1"})
    snapshot = json.load(urllib.request.urlopen(req, timeout=5))
    stream = next(s for s in snapshot["streams"] if s["project_id"] == snapshot["projects"][0]["id"])
    command = next(c for c in reversed(snapshot["commands"]) if c["stream_id"] == stream["id"] and c["kind"] == "run_agent")
    root = (command.get("agent_spec") or {}).get("continuation") or {}
    expected_title = "Factory " + root.get("root_command_id", command["id"])
    config = json.loads(env.get("OPENCODE_CLI_CONFIG_CONTENT", "{}"))
    config.setdefault("keybinds", {})["app.exit"] = "f12"
    env["OPENCODE_CLI_CONFIG_CONTENT"] = json.dumps(config)
    if args.missing_cli:
        env["PATH"] = "/nonexistent-yoloforge-native-test"
pid, fd = pty.fork()
if pid == 0:
    os.execve(str(ROOT / "target/debug/yoloforge"), ["yoloforge", "--server", args.server], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 160, 0, 0))
screen = pyte.Screen(160, 42)
terminal = pyte.Stream(screen)
decoder = codecs.getincrementaldecoder("utf-8")("replace")


def read_until(expected, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if expected.decode() in "\n".join(screen.display):
            return
        if select.select([fd], [], [], .1)[0]:
            terminal.feed(decoder.decode(os.read(fd, 65536)))
    raise AssertionError("TUI did not show " + expected.decode() + "\nTop of screen:\n" + "\n".join(screen.display[:8]))


try:
    read_until(b"Sessions")
    os.write(fd, b"4")
    read_until(b"OpenCode oeffnen")
    if args.native:
        os.write(fd, b"\r")
        if args.missing_cli:
            read_until(b"OpenCode-Terminal konnte nicht gestartet werden")
            print("PASS missing native CLI restores YoloForge terminal and displays the error")
        else:
            # OpenCode truncates the tab title and wraps the complete ID in its sidebar.
            read_until(expected_title[:24].encode(), timeout=40)
            os.write(fd, b"\x1b[24~")
            read_until(b"Zurueck von OpenCode", timeout=20)
            print("PASS original OpenCode terminal opens the recorded session and returns to YoloForge")
    os.write(fd, b"v")
    read_until(b"Unterhaltung und Werkzeuge")
    read_until(b"synchronisiert")
    os.write(fd, b"c")
    read_until(b"Ctrl-S")
    os.write(fd, b"\x1b")
    read_until(b"Unterhaltung und Werkzeuge")
    os.write(fd, b"\x1b")
    read_until(b"Sessions")
    print("PASS stream opens live OpenCode transcript; controls are secondary and Esc returns to session")
finally:
    os.kill(pid, signal.SIGTERM)
    os.waitpid(pid, 0)
    os.close(fd)

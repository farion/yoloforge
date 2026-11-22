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
import pyte

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--server", default="http://127.0.0.1:17432")
args = parser.parse_args()
env = {**os.environ, "TERM": "xterm-256color"}
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
    raise AssertionError("TUI did not show " + expected.decode())


try:
    read_until(b"Sessions")
    os.write(fd, b"4")
    read_until(b"OpenCode-Session")
    os.write(fd, b"\r")
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

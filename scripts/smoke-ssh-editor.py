#!/usr/bin/env python3
"""PTY acceptance of the local SSH editor; uses an isolated config and never connects SSH."""
import codecs
import fcntl
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import pty
import select
import signal
import struct
import tempfile
import termios
import time
import urllib.request
import pyte

root = Path(__file__).resolve().parents[1]
directory = Path(tempfile.mkdtemp(prefix="yf-ssh-editor-", dir="/tmp/opencode"))
config = directory / "ssh.json"
server = os.environ.get("FACTORY_URL", "http://127.0.0.1:17432")
token = os.environ.get("FACTORY_TOKEN") or Path(os.environ["FACTORY_TOKEN_FILE"]).read_text().strip()
request = urllib.request.Request(server + "/v1/snapshot", headers={"Authorization": "Bearer " + token, "x-factory-protocol": "1"})
snapshot = json.load(urllib.request.urlopen(request, timeout=10))
runner = snapshot["runners"][0]["id"]
env = {**os.environ, "TERM": "xterm-256color", "YOLOFORGE_SSH_CONFIG": str(config)}
pid, fd = pty.fork()
if pid == 0:
    executable = str(profile_dir(root=root) / "yoloforge")
    os.execve(executable, [executable, "--server", server], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))
screen = pyte.Screen(160, 40)
stream = pyte.Stream(screen)
decoder = codecs.getincrementaldecoder("utf-8")("replace")

def wait(text):
    deadline = time.monotonic() + 15
    while text not in "\n".join(screen.display):
        assert time.monotonic() < deadline, f"Missing screen text: {text}"
        if select.select([fd], [], [], .1)[0]:
            stream.feed(decoder.decode(os.read(fd, 65536)))

try:
    wait("Verbunden")
    os.write(fd, b"1s")
    wait("Lokale SSH-Verbindung")
    for value in ["acceptance-host", "http://127.0.0.1:7432", "/home/test/runner.sqlite3", "/home/test/service.json"]:
        os.write(fd, b"\x1b[200~" + value.encode() + b"\x1b[201~\t")
    os.write(fd, b"\x13")
    wait("SSH-Verbindung gespeichert")
    saved = json.loads(config.read_text())
    assert saved[runner] == {"host": "acceptance-host", "server": "http://127.0.0.1:7432", "journal": "/home/test/runner.sqlite3", "opencode_registration": "/home/test/service.json"}
    (directory / "screen.txt").write_text("\n".join(screen.display))
    os.write(fd, b"\x1b")
    wait("Runner / Live-Status")
    os.write(fd, b"s")
    wait("acceptance-host")
    os.write(fd, b"\x04")
    wait("SSH-Verbindung entfernt")
    assert json.loads(config.read_text()) == {}
    print("PASS SSH profile editor: paste, save, pinned runner, reopen and delete; no SSH connection")
    print("Evidence:", directory)
finally:
    os.kill(pid, signal.SIGTERM)
    os.waitpid(pid, 0)
    os.close(fd)

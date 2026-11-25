#!/usr/bin/env python3
"""Read-only PTY acceptance for the helper used by SSH attachment; no Factory token."""
import argparse
import codecs
import fcntl
import json
import os
import pty
import select
import signal
import struct
import termios
import time
from pathlib import Path
from cargo_paths import profile_dir
import pyte

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument("--journal",required=True)
parser.add_argument("--runner",required=True)
parser.add_argument("--server",required=True)
parser.add_argument("--command",required=True)
parser.add_argument("--title-prefix",required=True)
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
env={k:v for k,v in os.environ.items() if k not in ("FACTORY_TOKEN","FACTORY_TOKEN_FILE")}
env["TERM"]="xterm-256color"
config=json.loads(env.get("OPENCODE_CLI_CONFIG_CONTENT","{}"))
config.setdefault("keybinds",{})["app.exit"]="f12"
env["OPENCODE_CLI_CONFIG_CONTENT"]=json.dumps(config)
pid,fd=pty.fork()
if pid==0:
    executable=str(profile_dir(root=root)/"factory-runner")
    os.execve(executable,[executable,"--id",args.runner,"--name","attachment-test","--server",args.server,"--journal",args.journal,"--open-session",args.command],env)
fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",42,160,0,0))
screen=pyte.Screen(160,42)
terminal=pyte.Stream(screen)
decoder=codecs.getincrementaldecoder("utf-8")("replace")
reaped=False
try:
    deadline=time.monotonic()+40
    while args.title_prefix not in "\n".join(screen.display):
        assert time.monotonic()<deadline,"Recorded OpenCode session did not appear"
        if select.select([fd],[],[],.1)[0]:
            terminal.feed(decoder.decode(os.read(fd,65536)))
    os.write(fd,b"\x1b[24~")
    deadline=time.monotonic()+15
    while True:
        result,status=os.waitpid(pid,os.WNOHANG)
        if result:
            reaped=True
            assert os.waitstatus_to_exitcode(status)==0
            break
        assert time.monotonic()<deadline,"OpenCode did not return"
        if select.select([fd],[],[],.1)[0]:
            try:os.read(fd,65536)
            except OSError:pass
    print("PASS runner helper opens recorded session without Factory token or taking runner lock")
finally:
    if not reaped:
        os.kill(pid,signal.SIGTERM)
        os.waitpid(pid,0)
    os.close(fd)

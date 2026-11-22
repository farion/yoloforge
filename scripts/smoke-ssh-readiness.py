#!/usr/bin/env python3
"""Isolated loopback SSH server and pinned host key; run the real read-only runner helper."""
import argparse
import getpass
import os
from pathlib import Path
import shlex
import socket
import subprocess
import tempfile
import time

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument("--runner",required=True)
parser.add_argument("--journal",type=Path,required=True)
parser.add_argument("--server",required=True)
parser.add_argument("--command",required=True)
parser.add_argument("--model",help="Optional explicit provider/model for read-only readiness")
args=parser.parse_args()
directory=Path(tempfile.mkdtemp(prefix="yf-ssh-lab-",dir="/tmp/opencode"))
for name in ["host","client"]:
    subprocess.run(["ssh-keygen","-q","-t","ed25519","-N","","-f",str(directory/name)],check=True,capture_output=True)
(directory/"authorized_keys").write_text((directory/"client.pub").read_text());(directory/"authorized_keys").chmod(0o600)
with socket.socket() as listener:
    listener.bind(("127.0.0.1",0));port=listener.getsockname()[1]
helper=[str(root/"target/debug/factory-runner"),"--id",args.runner,"--name","ssh-readiness-test","--server",args.server,"--journal",str(args.journal.resolve()),"--open-session",args.command,"--session-readiness"]
if args.model:helper.extend(["--inspect-model",args.model])
# Only this fixed, read-only helper is allowed through the temporary SSH listener.
config=directory/"sshd_config"
# The private 0700 lab lives below /tmp, whose parent permissions StrictModes rejects.
config.write_text(f"Port {port}\nListenAddress 127.0.0.1\nHostKey {directory}/host\nPidFile {directory}/pid\nAuthorizedKeysFile {directory}/authorized_keys\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nPubkeyAuthentication yes\nAuthenticationMethods publickey\nAllowUsers {getpass.getuser()}\nStrictModes no\nDisableForwarding yes\nPermitTTY no\nPrintMotd no\nForceCommand {shlex.join(helper)}\n")
known=directory/"known_hosts";known.write_text(f"[127.0.0.1]:{port} "+(directory/"host.pub").read_text())
env={k:v for k,v in os.environ.items() if k not in ["FACTORY_TOKEN","FACTORY_TOKEN_FILE","OPENCODE_PASSWORD"]}
log=(directory/"sshd.log").open("w")
daemon=subprocess.Popen(["/usr/sbin/sshd","-D","-e","-f",str(config)],env=env,stdout=log,stderr=subprocess.STDOUT)
try:
    deadline=time.monotonic()+10
    while True:
        if daemon.poll() is not None:raise RuntimeError("Isolated SSH server failed: "+(directory/"sshd.log").read_text())
        try:
            with socket.create_connection(("127.0.0.1",port),timeout=.2):break
        except OSError:
            assert time.monotonic()<deadline,"SSH server did not start";time.sleep(.05)
    command=["ssh","-T","-F","/dev/null","-o","BatchMode=yes","-o","IdentitiesOnly=yes","-o","StrictHostKeyChecking=yes","-o","UserKnownHostsFile="+str(known),"-i",str(directory/"client"),"-p",str(port),getpass.getuser()+"@127.0.0.1",shlex.join(helper)]
    result=subprocess.run(command,env=env,capture_output=True,text=True,timeout=40)
    assert result.returncode==0,result.stderr
    assert any(prefix+args.command in result.stdout for prefix in ["Recorded execution: ","Recorded workspace: "])
    assert "OpenCode-Service erreichbar" in result.stdout
    (directory/"readiness.txt").write_text(result.stdout)
    print("PASS real SSH traversal, pinned host key and runner/session identity; no Factory token or runner restart")
    print("Evidence:",directory)
finally:
    daemon.terminate();daemon.wait(timeout=10);log.close()

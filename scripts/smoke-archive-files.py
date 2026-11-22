#!/usr/bin/env python3
"""Real Factory persistence and CLI binary export after source deletion/server restart; no model."""
import base64
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import urllib.request
import uuid

root=Path(__file__).resolve().parents[1]
directory=Path(tempfile.mkdtemp(prefix="yf-archive-files-",dir="/tmp/opencode"))
env={**os.environ,"FACTORY_TOKEN":uuid.uuid4().hex*2};env.pop("FACTORY_TOKEN_FILE",None)
def rid():return str(uuid.uuid4())
def start():
    process=subprocess.Popen([str(root/"target/debug/factoryd"),"--listen","127.0.0.1:0","--database",str(directory/"db")],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    assert select.select([process.stdout],[],[],10)[0]
    return process,"http://"+process.stdout.readline().strip().split()[-1]
server,endpoint=start()
try:
    def api(method,path,body=None,registration=None):
        headers={"Authorization":"Bearer "+env["FACTORY_TOKEN"],"x-factory-protocol":"1","Content-Type":"application/json"}
        if registration:headers["x-factory-runner-registration"]=registration
        req=urllib.request.Request(endpoint+path,headers=headers,method=method,data=None if body is None else json.dumps(body).encode())
        return json.load(urllib.request.urlopen(req,timeout=10))
    project=api("POST","/v1/projects",{"request_id":rid(),"name":"Archive files","repository":"/unused","base_branch":"main"})
    api("PATCH","/v1/projects/"+project["id"]+"/profile",{"request_id":rid(),"expected_revision":1,"profile":{"toolchain":"","start_command":"","test_command":"","provider_id":"test","model_id":"test"}})
    runner=rid();registration=api("POST","/v1/runners/register",{"id":runner,"instance_id":rid(),"name":"Archive fixture","location":"local","version":"test","capabilities":["container-freeze-v1"],"opencode":{"status":"not_configured"}})["registration_id"]
    stream=api("POST","/v1/streams",{"request_id":rid(),"project_id":project["id"],"goal":"Archive binary test","runner_id":runner,"ticket_ids":[]})
    prefix="/v1/runners/"+runner+"/commands/"
    prep=api("POST","/v1/streams/"+stream["id"]+"/commands",{"request_id":rid(),"expected_revision":1,"kind":"prepare_workspace"})
    api("POST",prefix+prep["id"]+"/ack",{"request_id":rid(),"ownership_epoch":1},registration)
    api("POST",prefix+prep["id"]+"/result",{"request_id":rid(),"ownership_epoch":1,"result":{"workspace":{"path":"/unused","branch":stream["branch"],"base_commit":"a"*40}}},registration)
    command=api("POST","/v1/streams/"+stream["id"]+"/commands",{"request_id":rid(),"expected_revision":4,"kind":"run_agent","prompt":"Synthetic archive; no model"})
    api("POST",prefix+command["id"]+"/ack",{"request_id":rid(),"ownership_epoch":1},registration)
    payload=b"\x00\xff\x80PDF-fixture\n"*1000;digest=hashlib.sha256(payload).hexdigest()
    source=directory/"response.bin";source.write_bytes(payload)
    metadata={"uri":source.as_uri(),"name":"../../untrusted-name.bin","mime":"application/octet-stream","status":"captured","sha256":digest,"data":base64.b64encode(payload).decode()}
    session="ses_factory_"+command["id"].replace("-","")
    body={"ownership_epoch":1,"batch":{"session_id":session,"revision":1,"captured_unix_ms":1,"final_scan_complete":True,"messages":[{"id":"msg_binary","content":[{"type":"file","uri":source.as_uri(),"mime":"application/octet-stream"}],"__yoloforge_files_v1":[metadata]}]}}
    first=api("PUT",prefix+command["id"]+"/archive",body,registration);assert api("PUT",prefix+command["id"]+"/archive",body,registration)==first
    source.unlink();server.terminate();server.wait(timeout=10);server,endpoint=start()
    output=directory/"archive.jsonl";files=directory/"files"
    result=subprocess.run([str(root/"target/debug/yoloforge"),"--server",endpoint,"archive",command["id"],"--output",str(output),"--attachments",str(files)],env=env,capture_output=True,text=True,timeout=20)
    assert result.returncode==0,result.stderr
    assert (files/digest).read_bytes()==payload
    assert not (directory.parent/"untrusted-name.bin").exists()
    manifest=[json.loads(line) for line in (files/"manifest.jsonl").read_text().splitlines()]
    assert manifest[0]["sha256"]==digest and manifest[0]["sample_revision"]==1
    assert json.loads(output.read_text().splitlines()[-1])["export_complete"]
    print("PASS archive binary persisted, replayed, survived source deletion/server restart, and exported with verified digest")
    print("Evidence:",directory)
finally:
    server.terminate();server.wait(timeout=10)

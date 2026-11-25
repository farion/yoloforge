#!/usr/bin/env python3
"""Real server ntfy outbox: uncertain POST, restart, durable backoff and confirmation; no models."""
import http.server
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import select
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid

root=Path(__file__).resolve().parents[1]
directory=Path(tempfile.mkdtemp(prefix="yf-push-",dir="/tmp/opencode"))
env={**os.environ,"FACTORY_TOKEN":uuid.uuid4().hex*2};env.pop("FACTORY_TOKEN_FILE",None)
requests=[];lock=threading.Lock();first=threading.Event();second=threading.Event()
class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body=self.rfile.read(int(self.headers["Content-Length"]))
        assert self.path=="/private-test-topic"
        assert self.headers["Click"]=="yoloforge://questions"
        assert self.headers.get("Authorization") is None
        assert b"PRIVATE-QUESTION" not in body
        with lock:
            requests.append((time.monotonic(),body));count=len(requests)
        if count==1:
            first.set();self.connection.shutdown(socket.SHUT_RDWR);self.connection.close();return
        self.send_response(200);self.send_header("Content-Length","2");self.end_headers();self.wfile.write(b"{}");second.set()
    def log_message(self,*args):pass
provider=http.server.ThreadingHTTPServer(("127.0.0.1",0),Provider)
thread=threading.Thread(target=provider.serve_forever,daemon=True);thread.start()
config=directory/"push.json";config.write_text(json.dumps({"endpoint":f"http://127.0.0.1:{provider.server_port}/private-test-topic"}))
def start():
    p=subprocess.Popen([str(profile_dir(root=root)/"factoryd"),"--listen","127.0.0.1:0","--database",str(directory/"db"),"--push-config",str(config)],env=env,stdout=subprocess.PIPE,stderr=(directory/"server.log").open("a"),text=True)
    assert select.select([p.stdout],[],[],10)[0];return p,"http://"+p.stdout.readline().strip().split()[-1]
def rid():return str(uuid.uuid4())
server,endpoint=start()
try:
    def api(method,path,body=None,registration=None):
        headers={"Authorization":"Bearer "+env["FACTORY_TOKEN"],"x-factory-protocol":"1","Content-Type":"application/json"}
        if registration:headers["x-factory-runner-registration"]=registration
        request=urllib.request.Request(endpoint+path,method=method,headers=headers,data=None if body is None else json.dumps(body).encode())
        return json.load(urllib.request.urlopen(request,timeout=10))
    project=api("POST","/v1/projects",{"request_id":rid(),"name":"Push acceptance","repository":"/unused","base_branch":"main"})
    api("PATCH","/v1/projects/"+project["id"]+"/profile",{"request_id":rid(),"expected_revision":1,"profile":{"toolchain":"","start_command":"","test_command":"","provider_id":"test","model_id":"test"}})
    runner=rid();registration=api("POST","/v1/runners/register",{"id":runner,"instance_id":rid(),"name":"Push fixture","location":"local","version":"test","capabilities":[],"opencode":{"status":"not_configured"}})["registration_id"]
    stream=api("POST","/v1/streams",{"request_id":rid(),"project_id":project["id"],"goal":"Push test","runner_id":runner,"ticket_ids":[]});prefix="/v1/runners/"+runner+"/commands/"
    prep=api("POST","/v1/streams/"+stream["id"]+"/commands",{"request_id":rid(),"expected_revision":1,"kind":"prepare_workspace"})
    api("POST",prefix+prep["id"]+"/ack",{"request_id":rid(),"ownership_epoch":1},registration)
    api("POST",prefix+prep["id"]+"/result",{"request_id":rid(),"ownership_epoch":1,"result":{"workspace":{"path":"/unused","branch":stream["branch"],"base_commit":"a"*40}}},registration)
    command=api("POST","/v1/streams/"+stream["id"]+"/commands",{"request_id":rid(),"expected_revision":4,"kind":"run_agent","prompt":"No model: fixture"})
    api("POST",prefix+command["id"]+"/ack",{"request_id":rid(),"ownership_epoch":1},registration)
    session="ses_factory_"+command["id"].replace("-","")
    api("POST",prefix+command["id"]+"/questions",{"ownership_epoch":1,"questions":[{"external_id":"push_test","kind":"form","payload":{"id":"push_test","sessionID":session,"title":"PRIVATE-QUESTION","fields":[{"key":"name","type":"string","required":True}]}}]},registration)
    assert first.wait(15),"No initial push"
    server.terminate();server.wait(timeout=10)
    with sqlite3.connect(directory/"db") as db:
        pending,attempts,next_attempt=db.execute("SELECT pending,attempts,next_attempt FROM push_delivery").fetchone()
        assert pending>0 and attempts==1 and next_attempt>time.time()*1000
    server,endpoint=start();assert second.wait(45),"Durable retry missing after restart"
    deadline=time.monotonic()+5
    while True:
        with sqlite3.connect(directory/"db") as db:cursor,pending=db.execute("SELECT cursor,pending FROM push_delivery").fetchone()
        if cursor>0 and pending==0:break
        assert time.monotonic()<deadline;time.sleep(.1)
    with lock:
        assert len(requests)==2 and requests[1][0]-requests[0][0]>=29
        assert requests[0][1]==requests[1][1]
    print("PASS ntfy wakeup: no private payload, lost response, restart, persisted backoff and confirmation")
    print("Evidence:",directory)
finally:
    server.terminate();server.wait(timeout=10);provider.shutdown();provider.server_close()

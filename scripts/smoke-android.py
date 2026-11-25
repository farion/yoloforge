#!/usr/bin/env python3
"""Android UI against isolated Rust Factory: enrollment, notifications, answer and prompt. No models."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import select
import subprocess
import tempfile
import urllib.request
import uuid

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument("--serial",required=True,help="Dedicated Android emulator/device; app test storage is reset")
args=parser.parse_args()
Path("/tmp/opencode").mkdir(mode=0o700,parents=True,exist_ok=True)
directory=Path(tempfile.mkdtemp(prefix="yf-android-api-",dir="/tmp/opencode"))
env={**os.environ,"FACTORY_TOKEN":uuid.uuid4().hex*2};env.pop("FACTORY_TOKEN_FILE",None)
server=subprocess.Popen([str(profile_dir(root=root)/"factoryd"),"--listen","127.0.0.1:0","--database",str(directory/"db")],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
adb=["adb","-s",args.serial]
port=None
try:
    assert select.select([server.stdout],[],[],10)[0]
    endpoint="http://"+server.stdout.readline().strip().split()[-1];port=endpoint.rsplit(":",1)[1]
    def rid():return str(uuid.uuid4())
    def api(method,path,body=None,registration=None):
        headers={"Authorization":"Bearer "+env["FACTORY_TOKEN"],"x-factory-protocol":"1","Content-Type":"application/json"}
        if registration:headers["x-factory-runner-registration"]=registration
        req=urllib.request.Request(endpoint+path,headers=headers,method=method,data=None if body is None else json.dumps(body).encode())
        return json.load(urllib.request.urlopen(req,timeout=10))
    project=api("POST","/v1/projects",{"request_id":rid(),"name":"Android acceptance","repository":"/unused","base_branch":"main"})
    api("PATCH","/v1/projects/"+project["id"]+"/profile",{"request_id":rid(),"expected_revision":1,"profile":{"toolchain":"","start_command":"","test_command":"","provider_id":"test","model_id":"test"}})
    runner=rid();registration=api("POST","/v1/runners/register",{"id":runner,"instance_id":rid(),"name":"Android fixture","location":"local","version":"test","capabilities":["container-freeze-v1"],"opencode":{"status":"not_configured"}})["registration_id"]
    stream=api("POST","/v1/streams",{"request_id":rid(),"project_id":project["id"],"goal":"Android UI acceptance","runner_id":runner,"ticket_ids":[]})
    prefix="/v1/runners/"+runner+"/commands/"
    prep=api("POST","/v1/streams/"+stream["id"]+"/commands",{"request_id":rid(),"expected_revision":1,"kind":"prepare_workspace"})
    api("POST",prefix+prep["id"]+"/ack",{"request_id":rid(),"ownership_epoch":1},registration)
    api("POST",prefix+prep["id"]+"/result",{"request_id":rid(),"ownership_epoch":1,"result":{"workspace":{"path":"/unused","branch":stream["branch"],"base_commit":"a"*40}}},registration)
    command=api("POST","/v1/streams/"+stream["id"]+"/commands",{"request_id":rid(),"expected_revision":4,"kind":"run_agent","prompt":"Synthetic question; no model execution"})
    api("POST",prefix+command["id"]+"/ack",{"request_id":rid(),"ownership_epoch":1},registration)
    session="ses_factory_"+command["id"].replace("-","")
    data=b"\x00\xffAndroid archive proof"
    attachment={"uri":"file:///unused/proof.bin","name":"proof.bin","mime":"application/octet-stream","status":"captured","sha256":hashlib.sha256(data).hexdigest(),"data":base64.b64encode(data).decode()}
    api("PUT",prefix+command["id"]+"/archive",{"ownership_epoch":1,"batch":{"session_id":session,"revision":1,"captured_unix_ms":1,"final_scan_complete":False,"messages":[{"id":"msg_android_archive","text":"Android archive proof","__yoloforge_files_v1":[attachment]}]}},registration)
    api("POST",prefix+command["id"]+"/questions",{"ownership_epoch":1,"questions":[{"external_id":"android_name","kind":"form","payload":{"id":"android_name","sessionID":session,"title":"Name eingeben","fields":[{"key":"name","type":"string","required":True}]}}]},registration)
    code=api("POST","/v1/devices/enrollment",{"role":"operator","runner_id":None,"ttl_seconds":300})["code"]
    for apk in ["debug/app-debug.apk","androidTest/debug/app-debug-androidTest.apk"]:
        subprocess.run(adb+["install","-r",str(root/"android/app/build/outputs/apk"/apk)],check=True,capture_output=True)
    subprocess.run(adb+["shell","pm","grant","dev.yoloforge.mobile","android.permission.POST_NOTIFICATIONS"],check=True,capture_output=True)
    subprocess.run(adb+["reverse","tcp:"+port,"tcp:"+port],check=True,capture_output=True)
    result=subprocess.run(adb+["shell","am","instrument","-w","-e","class","dev.yoloforge.mobile.AppFlowTest","-e","factoryEndpoint","http://127.0.0.1:"+port,"-e","enrollmentCode",code,"dev.yoloforge.mobile.test/androidx.test.runner.AndroidJUnitRunner"],capture_output=True,text=True,timeout=150)
    print(result.stdout);result.check_returncode();assert "OK (1 test)" in result.stdout,"Android UI acceptance failed"
    print("PASS Android UI -> real Rust API: enrollment, question notification, typed answer, prompt; evidence:",directory)
finally:
    if port:subprocess.run(adb+["reverse","--remove","tcp:"+port],capture_output=True)
    server.terminate();server.wait(timeout=10)

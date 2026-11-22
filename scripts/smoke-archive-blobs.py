#!/usr/bin/env python3
"""Real Factory/runner/CLI, synthetic OpenCode: large files, restart, external-source recovery."""
import base64,hashlib,http.server,json,os,select,secrets,subprocess,tempfile,threading,time,urllib.parse,urllib.request,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
Path('/tmp/opencode').mkdir(exist_ok=True)
base=Path(tempfile.mkdtemp(prefix='yf-archive-blobs-',dir='/tmp/opencode'))
env={**os.environ,'FACTORY_TOKEN':secrets.token_hex(32)};env.pop('FACTORY_TOKEN_FILE',None)
sessions={};messages={};writes=[]
large=b'\x00\xffarchive-source\n'*(350000)
external=b'\x80external-source\n'*(410000)
outside=base/'external';outside.mkdir();external_path=outside/'outside.bin';external_path.write_bytes(external)
class Native(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def reply(self,value,status=200):
        data=json.dumps(value).encode();self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
    def do_GET(self):
        path=urllib.parse.urlsplit(self.path).path
        if path=='/api/info':return self.reply({'version':'2.0.27'})
        if path=='/api/session/active':return self.reply({'data':{}})
        parts=path.strip('/').split('/');session=parts[2] if len(parts)>2 else ''
        if session not in sessions:return self.reply({},404)
        if len(parts)==3:return self.reply({'data':sessions[session]})
        if parts[3]=='message':
            if len(parts)==5:
                found=next((m for m in messages.get(session,[]) if m['id']==parts[4]),None)
                return self.reply({'data':found},200 if found else 404)
            data=messages.get(session,[])
            if 'type=assistant' in self.path:data=[m for m in data if m['type']=='assistant']
            return self.reply({'data':data,'cursor':{'next':None}})
        return self.reply({'data':[]})
    def do_POST(self):
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])));writes.append(self.path)
        if self.path=='/api/session':sessions[body['id']]=body;return self.reply({'data':{'id':body['id']}})
        if self.path.endswith('/prompt'):
            session=self.path.split('/')[3];workspace=Path(sessions[session]['location']['directory']);(workspace/'large.bin').write_bytes(large)
            messages[session]=[{'id':body['id'],'type':'user','text':body['text'],'time':{'created':1}},
                {'id':'msg_result','type':'assistant','finish':'stop','time':{'created':2,'completed':3},'content':[{'type':'text','text':'Synthetic completed artifact task'},
                    {'type':'file','uri':(workspace/'large.bin').as_uri(),'name':'large.bin','mime':'application/octet-stream'},
                    {'type':'file','uri':external_path.as_uri(),'name':'outside.bin','mime':'application/octet-stream'}]}]
            sessions[session]['outcome']='succeeded';return self.reply({'data':{'id':body['id'],'payload':{'text':body['text']}}})
        self.reply({},400)
native=http.server.ThreadingHTTPServer(('127.0.0.1',0),Native);threading.Thread(target=native.serve_forever,daemon=True).start()
(base/'registration').write_text(json.dumps({'url':f'http://127.0.0.1:{native.server_port}','password':'fixture'}))
def rid():return str(uuid.uuid4())
def start_server(address):
    p=subprocess.Popen([str(ROOT/'target/debug/factoryd'),'--listen',address,'--database',str(base/'db')],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    assert select.select([p.stdout],[],[],10)[0];return p,'http://'+p.stdout.readline().strip().split()[-1]
server,url=start_server('127.0.0.1:0');runner=None
def api(method,path,body=None,registration=None):
    headers={'Authorization':'Bearer '+env['FACTORY_TOKEN'],'x-factory-protocol':'1','Content-Type':'application/json'}
    if registration:headers['x-factory-runner-registration']=registration
    req=urllib.request.Request(url+path,method=method,headers=headers,data=None if body is None else json.dumps(body).encode())
    with urllib.request.urlopen(req,timeout=20) as response:return json.load(response)
def wait(predicate):
    end=time.monotonic()+90
    while time.monotonic()<end:
        result=predicate()
        if result:return result
        time.sleep(.2)
    raise AssertionError('Acceptance timeout; evidence: '+str(base))
try:
    repo=base/'repo';repo.mkdir()
    for args in [['init','-b','main'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','--allow-empty','-m','base']]:subprocess.run(['git','-C',str(repo),*args],check=True,capture_output=True)
    project=api('POST','/v1/projects',{'request_id':rid(),'name':'Large archive','repository':str(repo),'base_branch':'main'})
    api('PATCH','/v1/projects/'+project['id']+'/profile',{'request_id':rid(),'expected_revision':1,'profile':{'provider_id':'fixture','model_id':'fixture','toolchain':'','start_command':'','test_command':''}})
    (base/'repositories').write_text(json.dumps({project['id']:str(repo)}));runner_id=rid()
    args=[str(ROOT/'target/debug/factory-runner'),'--id',runner_id,'--name','archive-fixture','--server',url,'--journal',str(base/'journal'),'--workspace-root',str(base/'work'),'--repositories',str(base/'repositories'),'--opencode-registration',str(base/'registration')]
    log=(base/'runner.log').open('w');runner=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT)
    wait(lambda:any(r['id']==runner_id for r in api('GET','/v1/snapshot')['runners']))
    stream=api('POST','/v1/streams',{'request_id':rid(),'project_id':project['id'],'runner_id':runner_id,'goal':'Archive fixture','ticket_ids':[]})
    def enqueue(kind,prompt=None):
        current=next(s for s in api('GET','/v1/snapshot')['streams'] if s['id']==stream['id'])
        return api('POST','/v1/streams/'+stream['id']+'/commands',{'request_id':rid(),'expected_revision':current['revision'],'kind':kind,'prompt':prompt})
    def done(id):return next((c for c in api('GET','/v1/snapshot')['commands'] if c['id']==id and c['state']=='succeeded'),None)
    prepare=enqueue('prepare_workspace');wait(lambda:done(prepare['id']))
    command=enqueue('run_agent','Synthetic fixture; no model');wait(lambda:done(command['id']))
    files=api('GET','/v1/commands/'+command['id']+'/files')['files'];assert len(files)==1 and files[0]['sha256']==hashlib.sha256(large).hexdigest()
    runner.terminate();runner.wait(timeout=10);runner=None
    (base/'work'/stream['id']/'large.bin').unlink();sessions.clear();messages.clear()
    subprocess.run([*args,'--rescan-archive-files',command['id']],env=env,check=True,capture_output=True)
    server.terminate();server.wait(timeout=10);server,url=start_server(url.removeprefix('http://'))
    runner=subprocess.Popen([*args,'--archive-source-root',str(outside)],env=env,stdout=log,stderr=subprocess.STDOUT)
    wait(lambda:len(api('GET','/v1/commands/'+command['id']+'/files')['files'])==2)
    external_path.unlink()
    result=json.loads(subprocess.check_output([str(ROOT/'target/debug/yoloforge'),'--server',url,'archive',command['id'],'--output',str(base/'archive.jsonl'),'--attachments',str(base/'files')],env=env,text=True))
    assert result['files_extracted']==2,result
    for data in [large,external]:assert (base/'files'/hashlib.sha256(data).hexdigest()).read_bytes()==data
    assert len(writes)==2,writes
    print('PASS >4 MiB native attachment capture, immutable chunks, source deletion and verified CLI export')
    print('PASS Factory/runner restart and explicit external-root reconstruction from central archive after native session deletion; no repeated prompts')
    runner.terminate();runner.wait(timeout=10);runner=None
    server.terminate();server.wait(timeout=10);server,url=start_server(url.removeprefix('http://'))
    registration=api('POST','/v1/runners/register',{'id':runner_id,'instance_id':rid(),'name':'lost-journal fixture','location':'local','version':'test','capabilities':[],'opencode':{'status':'not_configured'}})['registration_id']
    uncertain=enqueue('run_agent','Previously accepted command with lost write-ahead evidence')
    api('POST','/v1/runners/'+runner_id+'/commands/'+uncertain['id']+'/ack',{'request_id':rid(),'ownership_epoch':1},registration)
    (base/'journal').rename(base/'journal-retained-evidence')
    for attempt in range(2):
        server.terminate();server.wait(timeout=10);server,url=start_server(url.removeprefix('http://'))
        runner=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT)
        observed=wait(lambda:next((c for c in api('GET','/v1/snapshot')['commands'] if c['id']==uncertain['id'] and 'quarantined' in (c.get('agent_progress') or {}).get('summary','')),None))
        assert observed['state']=='accepted' and observed['agent_progress']['phase']=='unknown'
        runner.terminate();runner.wait(timeout=10);runner=None
    assert len(writes)==2,writes
    print('PASS lost accepted-command journal remains quarantined across restart, with no native prompt replay')
    print('Evidence:',base)
finally:
    if runner:runner.terminate();runner.wait(timeout=10)
    server.terminate();server.wait(timeout=10);native.shutdown();native.server_close()

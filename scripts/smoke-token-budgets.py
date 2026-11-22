#!/usr/bin/env python3
"""Real HTTP/CLI budget fencing and restart acceptance; synthetic usage, no models."""
import json
import os
from pathlib import Path
import secrets
import select
import subprocess
import tempfile
import urllib.error
import urllib.request
import uuid

ROOT=Path(__file__).resolve().parents[1]
Path('/tmp/opencode').mkdir(exist_ok=True)
base=Path(tempfile.mkdtemp(prefix='yf-token-budgets-',dir='/tmp/opencode'))
env={**os.environ,'FACTORY_TOKEN':secrets.token_hex(32)};env.pop('FACTORY_TOKEN_FILE',None)
def rid():return str(uuid.uuid4())
def start(address):
    process=subprocess.Popen([str(ROOT/'target/debug/factoryd'),'--listen',address,'--database',str(base/'db')],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    assert select.select([process.stdout],[],[],10)[0]
    return process,'http://'+process.stdout.readline().strip().split()[-1]
process,url=start('127.0.0.1:0')
try:
    def api(method,path,body=None,registration=None,status=200):
        headers={'Authorization':'Bearer '+env['FACTORY_TOKEN'],'x-factory-protocol':'1','Content-Type':'application/json'}
        if registration:headers['x-factory-runner-registration']=registration
        request=urllib.request.Request(url+path,method=method,headers=headers,data=None if body is None else json.dumps(body).encode())
        try:
            with urllib.request.urlopen(request,timeout=10) as response:
                assert response.status in (200,201),response.status
                assert status==200
                return json.load(response)
        except urllib.error.HTTPError as error:
            assert error.code==status,(error.code,error.read().decode())
    def cli(*args):
        return json.loads(subprocess.check_output([str(ROOT/'target/debug/yoloforge'),'--server',url,*map(str,args)],env=env,text=True))
    p=api('POST','/v1/projects',{'request_id':rid(),'name':'Token budget acceptance','repository':'/unused','base_branch':'main'})
    project=p['id']
    api('PATCH','/v1/projects/'+project+'/profile',{'request_id':rid(),'expected_revision':1,'profile':{'provider_id':'test','model_id':'test','toolchain':'','start_command':'','test_command':''}})
    runner=rid()
    registration=api('POST','/v1/runners/register',{'id':runner,'instance_id':rid(),'name':'Synthetic usage','location':'local','version':'test','capabilities':['session-lifecycle-v1'],'opencode':{'status':'not_configured'}})['registration_id']
    stream=api('POST','/v1/streams',{'request_id':rid(),'project_id':project,'goal':'Budget fencing','runner_id':runner,'ticket_ids':[]})
    prefix='/v1/runners/'+runner+'/commands/'
    def revision():return next(s['revision'] for s in api('GET','/v1/snapshot')['streams'] if s['id']==stream['id'])
    def enqueue(kind,status=200):return api('POST','/v1/streams/'+stream['id']+'/commands',{'request_id':rid(),'expected_revision':revision(),'kind':kind,'prompt':'Synthetic acceptance; do not execute' if kind=='run_agent' else None},status=status)
    def ack(c,status=200):return api('POST',prefix+c['id']+'/ack',{'request_id':rid(),'ownership_epoch':1},registration,status)
    prep=enqueue('prepare_workspace');ack(prep)
    api('POST',prefix+prep['id']+'/result',{'request_id':rid(),'ownership_epoch':1,'result':{'workspace':{'path':'/unused','branch':stream['branch'],'base_commit':'a'*40}}},registration)
    def limit(value):
        budget=cli('token-budget','show',project)
        args=['token-budget','set',project,'--expected-revision',budget['revision'],'--work',value,'--orchestration','unlimited','--memory','unlimited','--request-id',rid()]
        saved=cli(*args);assert cli(*args)==saved
        return saved,args
    limit('0');enqueue('run_agent',400)
    limit('17');command=enqueue('run_agent')
    limit('0');ack(command,409)
    limit('17');ack(command)
    saved,args=limit('0');assert ack(command)['state']=='accepted'
    session='ses_factory_'+command['id'].replace('-','')
    api('PUT',prefix+command['id']+'/archive',{'ownership_epoch':1,'batch':{'session_id':session,'revision':1,'captured_unix_ms':1,'final_scan_complete':True,'messages':[{'id':'msg_budget_usage','type':'assistant','model':{'providerID':'test','id':'test'},'time':{'completed':1},'tokens':{'input':12,'output':5,'reasoning':2,'cache':{'read':3,'write':0}},'cost':0}]}},registration)
    api('POST',prefix+command['id']+'/result',{'request_id':rid(),'ownership_epoch':1,'result':{'agent':{'session_id':session,'message_id':'msg_factory_'+command['id'].replace('-',''),'phase':'succeeded','summary':'Synthetic usage fixture'}}},registration)
    limit('17');enqueue('run_agent',400)
    saved,args=limit('18')
    process.terminate();process.wait(timeout=10)
    process,url=start(url.removeprefix('http://'))
    assert cli(*args)==saved
    usage=cli('usage',project);assert usage['groups'][0]['tokens']['input']==12
    assert enqueue('run_agent')['state']=='pending'
    print('PASS optional limits, zero gate, pending recheck, accepted work preservation, exact observed threshold, replay and server restart')
    print('Evidence:',base)
finally:
    process.terminate();process.wait(timeout=10)

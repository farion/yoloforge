#!/usr/bin/env python3
"""Dedicated emulator UI: enrollment, image picker, SAF export, optional published APK upgrade.

Resets only dev.yoloforge.mobile on the explicitly selected emulator. No model calls.
"""
import argparse,base64,hashlib,json,os,re,secrets,select,struct,subprocess,tempfile,time,urllib.request,uuid,xml.etree.ElementTree as ET,zlib
from pathlib import Path
from cargo_paths import profile_dir
root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--serial',required=True)
parser.add_argument('--upgrade-to',help='Exercise the in-app latest-release installer and require this version afterwards')
parser.add_argument('--large-files',action='store_true',help='Require the supplemental chunked archive-file UI (0.1.3-dev or newer)')
args=parser.parse_args()
assert args.serial.startswith('emulator-'),'This destructive fixture reset is only for a dedicated emulator'
base=Path(tempfile.mkdtemp(prefix='yf-android-device-',dir='/tmp/opencode'))
adb=['adb','-s',args.serial]
def run(*cmd):return subprocess.check_output([*map(str,cmd)],stderr=subprocess.STDOUT,timeout=30)
def shell(*cmd):return run(*adb,'shell',*cmd).decode()
def hierarchy():
    shell('uiautomator','dump','/sdcard/yf-acceptance-ui.xml')
    return ET.fromstring(run(*adb,'exec-out','cat','/sdcard/yf-acceptance-ui.xml'))
def node(label,seconds=30):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        tree=hierarchy()
        for n in tree.iter('node'):
            if any(n.get(k,'').casefold()==label.casefold() for k in ['text','content-desc']) or n.get('content-desc','').casefold().startswith(label.casefold()+', '):return n
        time.sleep(.3)
    raise AssertionError('UI element unavailable: '+label)
def tap(label,seconds=30):
    n=node(label,seconds);x1,y1,x2,y2=map(int,re.findall(r'\d+',n.attrib['bounds']))
    shell('input','tap',str((x1+x2)//2),str((y1+y2)//2))
    time.sleep(.5)  # Allow system drawer/activity transitions before resolving the next bounds.
def enter(label,value):
    tap(label);shell('input','text',value.replace(' ','%s'));shell('input','keyevent','4')
def api(method,path,body=None,registration=None):
    headers={'Authorization':'Bearer '+env['FACTORY_TOKEN'],'x-factory-protocol':'1','Content-Type':'application/json'}
    if registration:headers['x-factory-runner-registration']=registration
    request=urllib.request.Request(url+path,headers=headers,method=method,data=None if body is None else json.dumps(body).encode())
    with urllib.request.urlopen(request,timeout=10) as response:return json.load(response)
def rid():return str(uuid.uuid4())
env={**os.environ,'FACTORY_TOKEN':secrets.token_hex(32)};env.pop('FACTORY_TOKEN_FILE',None)
server=subprocess.Popen([str(profile_dir(root=root)/'factoryd'),'--listen','127.0.0.1:0','--database',str(base/'db')],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
port=None
try:
    assert select.select([server.stdout],[],[],10)[0]
    url='http://'+server.stdout.readline().strip().split()[-1];port=url.rsplit(':',1)[1]
    project=api('POST','/v1/projects',{'request_id':rid(),'name':'Device acceptance','repository':'/unused','base_branch':'main'})
    api('PATCH','/v1/projects/'+project['id']+'/profile',{'request_id':rid(),'expected_revision':1,'profile':{'provider_id':'test','model_id':'test','toolchain':'','start_command':'','test_command':''}})
    runner=rid();registration=api('POST','/v1/runners/register',{'id':runner,'instance_id':rid(),'name':'Device fixture','location':'local','version':'test','capabilities':[],'opencode':{'status':'not_configured'}})['registration_id']
    stream=api('POST','/v1/streams',{'request_id':rid(),'project_id':project['id'],'goal':'Device archive','runner_id':runner,'ticket_ids':[]})
    prefix='/v1/runners/'+runner+'/commands/'
    prep=api('POST','/v1/streams/'+stream['id']+'/commands',{'request_id':rid(),'expected_revision':1,'kind':'prepare_workspace'})
    api('POST',prefix+prep['id']+'/ack',{'request_id':rid(),'ownership_epoch':1},registration)
    api('POST',prefix+prep['id']+'/result',{'request_id':rid(),'ownership_epoch':1,'result':{'workspace':{'path':'/unused','branch':stream['branch'],'base_commit':'a'*40}}},registration)
    command=api('POST','/v1/streams/'+stream['id']+'/commands',{'request_id':rid(),'expected_revision':4,'kind':'run_agent','prompt':'Synthetic archive fixture; no execution'})
    api('POST',prefix+command['id']+'/ack',{'request_id':rid(),'ownership_epoch':1},registration)
    filename='yf-proof-'+uuid.uuid4().hex[:8]+'.bin';data=b'\x00\xffSAF export acceptance'
    attachment={'uri':'file:///unused/'+filename,'name':filename,'mime':'application/octet-stream','status':'captured','sha256':hashlib.sha256(data).hexdigest(),'data':base64.b64encode(data).decode()}
    large_name='yf-large-'+uuid.uuid4().hex[:8]+'.bin';large_uri='file:///unused/'+large_name
    api('PUT',prefix+command['id']+'/archive',{'ownership_epoch':1,'batch':{'session_id':'ses_factory_'+command['id'].replace('-',''),'revision':1,'captured_unix_ms':1,'final_scan_complete':False,'messages':[{'id':'msg_device_archive','text':'Device archive proof','content':[{'type':'file','uri':large_uri,'mime':'application/octet-stream'}] if args.large_files else [],'__yoloforge_files_v1':[attachment]}]}},registration)
    if args.large_files:
        large_data=b'\x00\xffAndroid supplemental archive\n'*180000
        blob={'sha256':hashlib.sha256(large_data).hexdigest(),'bytes':len(large_data),'uri':large_uri,'name':large_name,'mime':'application/octet-stream','message_id':'msg_device_archive','source_revision':1,'captured_unix_ms':100}
        for offset in range(0,len(large_data),1024*1024):
            receipt=api('PUT',prefix+command['id']+'/files',{'ownership_epoch':1,'chunk':{'file':blob,'offset':offset,'data':base64.b64encode(large_data[offset:offset+1024*1024]).decode()}},registration)
        assert receipt['complete']
    code=api('POST','/v1/devices/enrollment',{'role':'operator','runner_id':None,'ttl_seconds':300})['code']
    shell('pm','clear','dev.yoloforge.mobile')
    shell('pm','grant','dev.yoloforge.mobile','android.permission.ACCESS_LOCAL_NETWORK')
    run(*adb,'reverse','tcp:'+port,'tcp:'+port)
    shell('am','start','-n','dev.yoloforge.mobile/.MainActivity')
    enter('Factory-Adresse (HTTPS)','http://127.0.0.1:'+port)
    enter('Einmaliger Anmeldecode',code);tap('Anmelden')
    node('App-Updates');tap('Device acceptance');tap('Neuer Prompt')
    enter('Was soll bearbeitet werden?','Device image acceptance')
    def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',32,32,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+b'\xff\0\0'*32)*32))+chunk(b'IEND',b'')
    image_name='yf-image-'+uuid.uuid4().hex[:8]+'.png';(base/image_name).write_bytes(png)
    run(*adb,'push',base/image_name,'/sdcard/Download/'+image_name)
    shell('am','broadcast','-a','android.intent.action.MEDIA_SCANNER_SCAN_FILE','-d','file:///sdcard/Download/'+image_name)
    tap('Bild anhaengen');tap('Show roots');tap('Downloads');tap(image_name)
    tap('Senden');node('Neuer Prompt')
    snapshot=api('GET','/v1/snapshot');task=next(t for t in snapshot['tickets'] if t.get('prompt')=='Device image acceptance')
    assert len(task['images'])==1 and base64.b64decode(task['images'][0]['data'])==png
    print('PASS real Android image picker -> encrypted draft -> authenticated Rust task upload',flush=True)
    session_label=next(n.get('text') for n in hierarchy().iter('node') if n.get('text','').startswith('SESSION: DEVICE ARCHIVE'))
    tap(session_label);tap('Vollstaendiges Archiv');tap('Anhang speichern: '+filename)
    tap('Save');node('Anhang gespeichert und SHA256 geprueft')
    exported=run(*adb,'exec-out','cat','/sdcard/Download/'+filename)
    assert exported==data
    print('PASS real SAF create-document dialog and byte-identical binary export',flush=True)
    if args.large_files:
        tap('Archivdatei speichern: '+large_name+' ('+str(len(large_data))+' Bytes)');tap('Save')
        node('Anhang gespeichert und SHA256 geprueft',seconds=90)
        assert run(*adb,'exec-out','cat','/sdcard/Download/'+large_name)==large_data
        print('PASS Android chunked >4 MiB download, full SHA256 verification and SAF export',flush=True)
    if args.upgrade_to:
        before=shell('dumpsys','package','dev.yoloforge.mobile');assert 'versionName='+args.upgrade_to+'\n' not in before
        tap('Zurueck');tap('Projekte');tap('App-Updates')
        shell('appops','set','dev.yoloforge.mobile','REQUEST_INSTALL_PACKAGES','allow')
        tap('Update herunterladen und installieren',seconds=90)
        tap('Update',seconds=180)
        end=time.monotonic()+180
        scanned=False
        while time.monotonic()<end:
            current=shell('dumpsys','package','dev.yoloforge.mobile')
            if 'versionName='+args.upgrade_to+'\n' in current:break
            labels={n.get('text','') for n in hierarchy().iter('node')}
            if 'Scan app' in labels and not scanned:
                tap('Scan app');scanned=True
            if 'Install' in labels:tap('Install')
            time.sleep(1)
        else:raise AssertionError('Published APK installation did not complete')
        shell('am','start','-n','dev.yoloforge.mobile/.MainActivity')
        node('Device acceptance',seconds=60)
        after=shell('dumpsys','package','dev.yoloforge.mobile');assert 'versionName='+args.upgrade_to+'\n' in after
        assert int(re.search(r'versionCode=(\d+)',after)[1])>int(re.search(r'versionCode=(\d+)',before)[1])
        print('PASS in-app published signed APK upgrade; enrolled Factory identity preserved',flush=True)
    (base/'screen.xml').write_bytes(ET.tostring(hierarchy()))
    print('Evidence:',base,flush=True)
finally:
    print('Retained acceptance directory:',base,flush=True)
    if port:subprocess.run(adb+['reverse','--remove','tcp:'+port],capture_output=True)
    server.terminate();server.wait(timeout=10)

#!/usr/bin/env python3
"""Explicit operator setup for Android signing secrets and the private hosting acceptance repo.

Run with: uv run --with pynacl python scripts/configure-github-release.py FLAGS
Credentials are read through Git's configured helper and never written or printed.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import urllib.error
import urllib.request

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--configure-signing',action='store_true')
parser.add_argument('--create-test-repository',action='store_true')
parser.add_argument('--signing-dir',type=Path,default=Path.home()/'.local/share/yoloforge/signing/android')
args=parser.parse_args()
if not (args.configure_signing or args.create_test_repository):parser.error('Select an explicit setup action')
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args,**kwargs):return None
opener=urllib.request.build_opener(NoRedirect())
result=subprocess.run(['git','credential','fill'],input='protocol=https\nhost=github.com\n\n',capture_output=True,text=True,timeout=15)
if result.returncode:raise SystemExit('GitHub credential helper unavailable')
credential=dict(line.split('=',1) for line in result.stdout.splitlines() if '=' in line)
token=credential.get('password')
if not token:raise SystemExit('GitHub credential missing')
def api(method,path,body=None,missing=False):
    request=urllib.request.Request('https://api.github.com'+path,method=method,data=None if body is None else json.dumps(body).encode(),headers={'Authorization':'Bearer '+token,'Accept':'application/vnd.github+json','Content-Type':'application/json','X-GitHub-Api-Version':'2022-11-28'})
    try:
        with opener.open(request,timeout=30) as response:
            data=response.read(2*1024*1024)
            return json.loads(data) if data else {}
    except urllib.error.HTTPError as error:
        if missing and error.code==404:return None
        raise SystemExit(f'GitHub setup request failed: HTTP {error.code}; credentials and response body withheld') from None
    except urllib.error.URLError:
        raise SystemExit('GitHub setup transport failed; retry reconciles existing resources') from None
assert api('GET','/user')['login']=='farion','Expected the repository owner account'
if args.configure_signing:
    from nacl.public import PublicKey,SealedBox
    directory=args.signing_dir.expanduser().resolve();config=json.loads((directory/'signing.json').read_text())
    password=(directory/config['password_file']).read_text().strip()
    env={**os.environ,'YOLOFORGE_KEY_PASSWORD':password}
    keytool=Path(env.get('JAVA_HOME','/usr/lib/jvm/java-17-openjdk-amd64'))/'bin/keytool'
    certificate=subprocess.run([str(keytool),'-exportcert','-keystore',str(directory/config['keystore']),'-alias',config['alias'],'-storepass:env','YOLOFORGE_KEY_PASSWORD'],env=env,capture_output=True,timeout=20)
    assert certificate.returncode==0,'Cannot read signing certificate'
    assert hashlib.sha256(certificate.stdout).hexdigest()==(root/'android/release-certificate.sha256').read_text().strip(),'Signing identity differs from pinned release certificate'
    prefix='/repos/farion/yoloforge/actions'
    public=api('GET',prefix+'/secrets/public-key');box=SealedBox(PublicKey(base64.b64decode(public['key'])))
    secrets={'ANDROID_KEYSTORE_BASE64':base64.b64encode((directory/config['keystore']).read_bytes()).decode(),'ANDROID_STORE_PASSWORD':password,'ANDROID_KEY_ALIAS':config['alias']}
    for name,value in secrets.items():
        api('PUT',prefix+'/secrets/'+name,{'key_id':public['key_id'],'encrypted_value':base64.b64encode(box.encrypt(value.encode())).decode()})
        print('Configured encrypted GitHub secret:',name)
    variable={'name':'ANDROID_RELEASE_SIGNING','value':'enabled'}
    existing=api('GET',prefix+'/variables/ANDROID_RELEASE_SIGNING',missing=True)
    api('PATCH' if existing is not None else 'POST',prefix+'/variables'+('/ANDROID_RELEASE_SIGNING' if existing is not None else ''),variable)
    assert api('GET',prefix+'/variables/ANDROID_RELEASE_SIGNING')['value']=='enabled'
    print('Android CI signing enabled; local signing files retained')
if args.create_test_repository:
    path='/repos/farion/yoloforge-acceptance';repository=api('GET',path,missing=True)
    if repository is None:
        repository=api('POST','/user/repos',{'name':'yoloforge-acceptance','private':True,'description':'Disposable synthetic YoloForge hosting acceptance fixtures','auto_init':False})
    assert repository['private'] and repository['full_name']=='farion/yoloforge-acceptance','Existing repository identity/privacy differs'
    print('Private acceptance repository ready:',repository['html_url'])

#!/usr/bin/env python3
"""Create/use a stable private Android signing identity and package a verified release APK."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import re
import subprocess
import tomllib

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument("--signing-dir",type=Path,default=Path(os.environ.get("XDG_DATA_HOME",str(Path.home()/".local/share")))/"yoloforge/signing/android")
parser.add_argument("--init-signing",action="store_true",help="Create a new signing identity; refuses an existing directory")
parser.add_argument("--initialize-only",action="store_true")
args=parser.parse_args()
directory=args.signing_dir.expanduser().resolve()
def private_write(path,content):
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,"w") as file:file.write(content);file.flush();os.fsync(file.fileno())
if args.init_signing:
    directory.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    directory.mkdir(mode=0o700)
    password=secrets.token_urlsafe(48)
    private_write(directory/"password",password+"\n")
    env={**os.environ,"YOLOFORGE_KEY_PASSWORD":password}
    keytool=Path(env.get("JAVA_HOME","/usr/lib/jvm/java-17-openjdk-amd64"))/"bin/keytool"
    subprocess.run([str(keytool),"-genkeypair","-keystore",str(directory/"release.p12"),"-storetype","PKCS12","-alias","yoloforge-android","-keyalg","RSA","-keysize","4096","-sigalg","SHA256withRSA","-validity","10000","-dname","CN=YoloForge Android, O=YoloForge","-storepass:env","YOLOFORGE_KEY_PASSWORD","-keypass:env","YOLOFORGE_KEY_PASSWORD"],env=env,check=True,capture_output=True)
    os.chmod(directory/"release.p12",0o600)
    private_write(directory/"signing.json",json.dumps({"alias":"yoloforge-android","keystore":"release.p12","password_file":"password"},indent=2)+"\n")
    private_write(directory/"BACKUP.txt","Back up this whole private directory securely, including the keystore and password.\nThis is the stable signing identity for dev.yoloforge.mobile. Losing it prevents updates to installed signed APKs.\nNever commit or publish these files.\n")
    print("Created stable Android signing identity:",directory)
if args.initialize_only:
    raise SystemExit(0)
config=json.loads((directory/"signing.json").read_text())
password=(directory/config["password_file"]).read_text().strip()
env={**os.environ,"YOLOFORGE_ANDROID_KEYSTORE":str(directory/config["keystore"]),"YOLOFORGE_ANDROID_KEY_ALIAS":config["alias"],"YOLOFORGE_ANDROID_STORE_PASSWORD":password,"YOLOFORGE_ANDROID_KEY_PASSWORD":password}
env.setdefault("JAVA_HOME","/usr/lib/jvm/java-17-openjdk-amd64")
subprocess.run([str(root/"android/gradlew"),"--no-daemon",":app:assembleRelease",":app:lintRelease"],cwd=root/"android",env=env,check=True)
sdk=Path(env.get("ANDROID_HOME",env.get("ANDROID_SDK_ROOT",str(Path.home()/"Android/Sdk"))))
apksigner=sdk/"build-tools/36.0.0/apksigner"
apk=root/"android/app/build/outputs/apk/release/app-release.apk"
verification=subprocess.run([str(apksigner),"verify","--verbose","--print-certs",str(apk)],env=env,text=True,capture_output=True,check=True).stdout
print(verification)
fingerprint=re.search(r"certificate SHA-256 digest: ([0-9a-f]{64})",verification)
assert fingerprint and fingerprint.group(1)==(root/"android/release-certificate.sha256").read_text().strip(),"APK signer differs from the pinned release identity"
version=tomllib.loads((root/"Cargo.toml").read_text())["workspace"]["package"]["version"]
dist=root/"dist";dist.mkdir(exist_ok=True)
target=dist/f"yoloforge-android-v{version}.apk"
data=apk.read_bytes()
if target.exists():
    assert target.read_bytes()==data,"Existing release APK differs; move the old artifact aside explicitly before replacing it"
else:
    with target.open("xb") as file:file.write(data)
digest=hashlib.sha256(data).hexdigest()
checksum=target.with_suffix(".apk.sha256")
expected=f"{digest}  {target.name}\n"
if checksum.exists():assert checksum.read_text()==expected
else:checksum.write_text(expected)
print("Verified signed APK:",target)
print("SHA256:",digest)

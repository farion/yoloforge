#!/usr/bin/env python3
"""Online WAL backup and no-clobber restore, using real Factory processes."""
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import sqlite3
import subprocess
import tempfile
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    Path("/tmp/opencode").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="yf-backup-", dir="/tmp/opencode") as directory:
        directory = Path(directory)
        token = uuid.uuid4().hex * 2
        secret = directory / "token"
        secret.write_text(token + "\n")
        secret.chmod(0o600)
        env = {**os.environ, "FACTORY_TOKEN_FILE": str(secret)}
        env.pop("FACTORY_TOKEN", None)

        def start(path):
            process = subprocess.Popen([str(profile_dir() / "factoryd"), "--listen", "127.0.0.1:0", "--database", str(path)],
                                       env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            line = process.stdout.readline()
            if "listening on" not in line:
                process.kill()
                raise RuntimeError("Factory startup failed")
            return process, "http://" + line.strip().split()[-1]

        def stop(process):
            process.terminate()
            process.wait(timeout=10)
            process.stdout.close()
            process.stderr.close()

        source, backup, restored = (directory / name for name in ("source.db", "backup.db", "restored.db"))
        process, url = start(source)
        try:
            request = urllib.request.Request(url + "/v1/projects", data=json.dumps({"request_id": str(uuid.uuid4()), "name": "backup-proof", "repository": "/repo", "base_branch": "main"}).encode(),
                headers={"Authorization": "Bearer " + token, "x-factory-protocol": "1", "Content-Type": "application/json"})
            with urllib.request.urlopen(request) as response:
                project = json.load(response)
            assert source.with_name(source.name + "-wal").exists()
            output = subprocess.check_output(["python3", str(ROOT / "scripts/factory-backup.py"), "backup", "--source", str(source), "--destination", str(backup)], text=True)
            assert json.loads(output)["schema_version"] >= 6
            assert backup.stat().st_mode & 0o077 == 0
        finally:
            stop(process)
        subprocess.run(["python3", str(ROOT / "scripts/factory-backup.py"), "restore", "--source", str(backup), "--destination", str(restored)], check=True, stdout=subprocess.DEVNULL)
        before = restored.read_bytes()
        assert subprocess.run(["python3", str(ROOT / "scripts/factory-backup.py"), "restore", "--source", str(backup), "--destination", str(restored)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0
        assert restored.read_bytes() == before
        process, url = start(restored)
        try:
            request = urllib.request.Request(url + "/v1/snapshot", headers={"Authorization": "Bearer " + token, "x-factory-protocol": "1"})
            with urllib.request.urlopen(request) as response:
                assert json.load(response)["projects"][0]["id"] == project["id"]
            with sqlite3.connect(restored) as db:
                assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",)
        finally:
            stop(process)
        print("PASS token-file startup, live WAL backup, no-clobber restore and restart")


if __name__ == "__main__":
    main()

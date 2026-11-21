#!/usr/bin/env python3
"""Install a server release archive with its bundled Compose file in an isolated local project."""
import argparse
import hashlib
import os
from pathlib import Path
import secrets
import subprocess
import tarfile
import tempfile
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    with args.archive.open("rb") as file:
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    expected = args.archive.with_name(args.archive.name + ".sha256").read_text().split()[0]
    assert digest == expected, "Archive checksum differs"
    project = "yoloforge-package-test-" + uuid.uuid4().hex[:12]
    Path("/tmp/opencode").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=project, dir="/tmp/opencode") as temp:
        root = Path(temp)
        with tarfile.open(args.archive) as archive:
            archive.extractall(root, filter="data")
        package, = root.iterdir()
        assert (package / "factoryd").is_file()
        assert (package / "Dockerfile").is_file()
        token = root / "bootstrap.token"
        token.write_text(secrets.token_hex(32))
        token.chmod(0o444)
        env = {**os.environ, "YOLOFORGE_VERSION": project, "FACTORY_BOOTSTRAP_FILE": str(token), "FACTORY_PORT": "0"}
        command = ["docker", "compose", "-p", project, "-f", str(package / "compose.yaml")]
        try:
            subprocess.run([*command, "up", "-d", "--build", "--wait", "--wait-timeout", "90"], env=env, check=True)
            subprocess.run([*command, "exec", "-T", "factory", "factoryd", "--version"], env=env, check=True)
            subprocess.run([*command, "exec", "-T", "factory", "factory-backup", "backup", "--source", "/data/factory.sqlite3", "--destination", "/data/smoke-backup.sqlite3"], env=env, check=True)
            print("PASS server archive checksum, standalone Compose build/start and online backup")
        finally:
            subprocess.run([*command, "down", "--volumes", "--rmi", "local"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            subprocess.run(["docker", "image", "rm", "yoloforge-server:" + project], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Isolated Docker server, scoped device keys, restart and online backup."""
import argparse
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="yoloforge-server:development")
    args = parser.parse_args()
    name = "yf-deploy-test-" + uuid.uuid4().hex[:12]
    volume = name + "-data"
    Path("/tmp/opencode").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=name, dir="/tmp/opencode") as directory:
        directory = Path(directory)
        token = uuid.uuid4().hex * 2
        secret = directory / "bootstrap"
        secret.write_text(token)
        # Parent is 0700; file is readable to the isolated container UID through its bind mount.
        secret.chmod(0o444)
        try:
            docker("volume", "create", volume)
            docker("run", "-d", "--name", name, "--read-only", "--tmpfs", "/tmp:size=16m", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges", "-p", "127.0.0.1::7432", "--mount", f"type=volume,src={volume},dst=/data",
                   "--mount", f"type=bind,src={secret},dst=/run/secrets/bootstrap,readonly",
                   "-e", "FACTORY_TOKEN_FILE=/run/secrets/bootstrap", args.image)
            port = json.loads(docker("inspect", "--format", '{{json .NetworkSettings.Ports}}', name))["7432/tcp"][0]
            assert port["HostIp"] == "127.0.0.1"
            url = "http://127.0.0.1:" + port["HostPort"]
            assert docker("inspect", "--format", "{{.Config.User}}", name) == "10001:10001"

            def ready():
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    try:
                        with urllib.request.urlopen(url + "/healthz", timeout=2) as response:
                            if json.load(response)["status"] == "ready":
                                return
                    except (OSError, ValueError):
                        pass
                    time.sleep(0.2)
                raise RuntimeError("Container never became ready: " + docker("logs", name))

            def cli(token_file, *arguments):
                env = {**os.environ, "FACTORY_TOKEN_FILE": str(token_file)}
                env.pop("FACTORY_TOKEN", None)
                result = subprocess.run([str(profile_dir() / "factory"), "--server", url, *arguments], env=env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                if result.returncode:
                    raise RuntimeError("Factory CLI failed: " + result.stderr)
                return json.loads(result.stdout) if result.stdout.strip() else None

            ready()
            try:
                urllib.request.urlopen(url + "/v1/snapshot")
                raise AssertionError("Unauthenticated snapshot accepted")
            except urllib.error.HTTPError as error:
                assert error.code == 401
            device_token = directory / "device.token"
            device = cli(secret, "device", "create", "--role", "operator", "--name", "docker-client", "--output", str(device_token))
            assert device_token.stat().st_mode & 0o077 == 0
            project = cli(device_token, "project", "create", "--name", "docker-persistent", "--repository", "/repo")
            backup = json.loads(docker("exec", name, "python3", "/usr/local/bin/factory-backup", "backup", "--source", "/data/factory.sqlite3", "--destination", "/data/backup.sqlite3"))
            assert backup["schema_version"] >= 6
            docker("restart", "--time", "10", name)
            port = json.loads(docker("inspect", "--format", '{{json .NetworkSettings.Ports}}', name))["7432/tcp"][0]
            url = "http://127.0.0.1:" + port["HostPort"]
            ready()
            assert cli(device_token, "--snapshot")["projects"][0]["id"] == project["id"]
            cli(secret, "device", "revoke", device["id"])
            try:
                cli(device_token, "--snapshot")
                raise AssertionError("Revoked device accepted")
            except RuntimeError as error:
                assert "401" in str(error)
            print("PASS non-root read-only Docker server, loopback port, device keys, restart, revoke and WAL backup")
        finally:
            subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            subprocess.run(["docker", "volume", "rm", volume], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()

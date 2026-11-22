#!/usr/bin/env python3
"""Real model acceptance: new folder -> automatic task -> inline image -> written result.

Uses the existing authenticated Docker lab without changing its service or credentials.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import secrets
import select
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
import zlib

ROOT = Path(__file__).resolve().parents[1]


def inside(binaries):
    registration = json.load(sys.stdin)
    directory = Path(tempfile.mkdtemp(prefix="factory-image-", dir="/home/factory/workspace"))
    env = {**os.environ, "FACTORY_TOKEN": secrets.token_hex(32)}
    env.pop("FACTORY_TOKEN_FILE", None)
    processes = []
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(url, method="GET", body=None, auth=None):
        req = urllib.request.Request(url, method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": auth or "Bearer " + env["FACTORY_TOKEN"],
                     "Content-Type": "application/json", "x-factory-protocol": "1"})
        with opener.open(req, timeout=15) as response:
            value = response.read()
            return json.loads(value) if value else None

    def spawn(name, *args):
        p = subprocess.Popen([str(binaries / name), *args], env=env,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        processes.append(p)
        return p

    try:
        server = spawn("factoryd", "--listen", "127.0.0.1:0", "--database", str(directory / "factory.db"))
        assert select.select([server.stdout], [], [], 10)[0]
        endpoint = "http://" + server.stdout.readline().strip().split()[-1]
        api = lambda method, path, body=None: request(endpoint + path, method, body)
        runner = str(uuid.uuid4())
        reg = directory / "service.json"
        reg.write_text(json.dumps(registration))
        reg.chmod(0o600)
        spawn("factory-runner", "--server", endpoint, "--id", runner, "--name", "Image acceptance",
              "--journal", str(directory / "runner.db"), "--workspace-root", str(directory / "work"))
        source = str(directory / "new-project")
        setup = api("POST", "/v1/projects/setup", {
            "request_id": str(uuid.uuid4()), "project_id": None, "expected_revision": 0,
            "name": "Image prompt acceptance", "repository": source, "base_branch": "main",
            "profile": {"provider_id": "openai", "model_id": "gpt-6-astra", "toolchain": "", "start_command": "", "test_command": ""},
            "setup": {"runner_id": runner, "repository_source": source, "repository_path": "",
                      "service_registration": str(reg), "harness": None}})
        def chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
        png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 32, 32, 8, 2, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress((b"\0" + b"\xff\0\0" * 32) * 32)) + chunk(b"IEND", b"")
        task = api("POST", "/v1/tasks", {"request_id": str(uuid.uuid4()), "id": None,
            "expected_revision": 0, "project_id": setup["project"]["id"],
            "prompt": "Look at the attached solid-colour image. Use the shell tool to write its dominant colour as one lowercase English word into image-proof.txt in your assigned worktree. Do not guess without inspecting the image. Do not ask questions or delegate. Report the colour and the file you wrote.",
            "images": [{"name": "colour.png", "mime": "image/png", "data": base64.b64encode(png).decode()}]})
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            snapshot = api("GET", "/v1/snapshot")
            streams = {s["id"] for s in snapshot["streams"] if task["id"] in s["ticket_ids"]}
            commands = [c for c in snapshot["commands"] if c["stream_id"] in streams]
            for c in commands:
                assert c["state"] != "failed", c.get("result")
            for q in snapshot.get("questions", []):
                if q["state"] == "pending" and q["external"]["kind"] == "permission":
                    raise AssertionError("Model requires a permission answer; inspect retained acceptance workspace")
            completed = [c for c in commands if c["kind"] == "run_agent" and c["state"] == "succeeded"]
            if completed:
                assert len(completed) == 1
                prepared = next(c for c in commands if c["kind"] == "prepare_workspace")
                work = Path(prepared["result"]["workspace"]["path"])
                assert (work / "image-proof.txt").read_text().strip().lower() == "red"
                assert (Path(source) / ".git").is_dir()
                assert (Path(source) / "image-proof.txt").read_text().strip().lower() == "red", "Result was not published to the requested project folder"
                assert not subprocess.check_output(["git", "-C", source, "status", "--porcelain"], text=True).strip()
                archive = api("GET", "/v1/commands/" + completed[0]["id"] + "/archive")
                assert archive["status"]["final_scan_complete"]
                print("PASS real model receives image, writes correct result and automatically publishes into the requested project folder", flush=True)
                return
            time.sleep(.3)
        raise AssertionError("Image task did not finish within four minutes")
    finally:
        for p in reversed(processes):
            if p.poll() is None:
                p.terminate()
                p.wait(timeout=10)
        print("Acceptance evidence retained:", directory, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inside", type=Path)
    args = parser.parse_args()
    if args.inside:
        inside(args.inside)
        return
    state = json.loads((ROOT / "spikes/opencode/.runtime/state.json").read_text())
    container = state["container"]
    target = "/tmp/factory-image-binaries-" + secrets.token_hex(8)
    subprocess.run(["docker", "exec", container, "mkdir", target], check=True, capture_output=True)
    try:
        for name in ["factoryd", "factory-runner"]:
            subprocess.run(["docker", "cp", str(ROOT / "target/debug" / name), container + ":" + target + "/" + name], check=True, capture_output=True)
        subprocess.run(["docker", "cp", str(Path(__file__).resolve()), container + ":" + target + "/test.py"], check=True, capture_output=True)
        result = subprocess.run(["docker", "exec", "-i", container, "python3", target + "/test.py", "--inside", target],
            input=json.dumps({"url": "http://127.0.0.1:4096", "password": state["password"]}), text=True, timeout=300)
        assert result.returncode == 0, "Real image-prompt acceptance failed"
    finally:
        subprocess.run(["docker", "exec", container, "rm", "-rf", target], check=True, capture_output=True)


if __name__ == "__main__":
    main()

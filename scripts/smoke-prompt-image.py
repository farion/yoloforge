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


def inside(binaries, provider, model, registration=None, followup=False, memory=False):
    local = registration is not None
    registration = registration if local else json.load(sys.stdin)
    directory = Path(tempfile.mkdtemp(prefix="factory-image-", dir="/tmp/opencode" if local else "/home/factory/workspace"))
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
            "profile": {"provider_id": provider, "model_id": model, "toolchain": "", "start_command": "", "test_command": ""},
            "setup": {"runner_id": runner, "repository_source": source, "repository_path": "",
                       "service_registration": str(reg), "harness": None}})
        def verify_memory(command):
            if not memory:
                return
            deadline=time.monotonic()+240
            while time.monotonic()<deadline:
                notes=api("GET","/v1/projects/"+setup["project"]["id"]+"/knowledge")
                matching=[n for n in notes if n.get("source_command")==command["id"] and n["source"].startswith("Automatic extraction ")]
                if matching:
                    snapshot=api("GET","/v1/snapshot")
                    branch=next(s["branch"] for s in snapshot["streams"] if s["id"]==command["stream_id"])
                    assert all(n.get("branch")==branch and n["priority"]==1 for n in matching)
                    assert len(matching)==1
                    print("PASS real memory extraction retains source command, evaluation provenance and branch scope",flush=True)
                    return
                time.sleep(.5)
            raise AssertionError("No reusable memory extracted for finalized proof-file convention")
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
                if q["state"] == "pending" and q["external"]["kind"] == "permission" and q.get("orchestration_note", "").startswith("human:"):
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
                routing = [c for c in snapshot["commands"] if (c.get("agent_spec") or {}).get("role") == "orchestrator"]
                assert routing and any(c["state"] == "succeeded" for c in routing), "No successful model routing evaluation"
                print("PASS real orchestrator routes image task before work starts", flush=True)
                if followup:
                    root = completed[0]["id"]
                    follow = api("POST", "/v1/tasks", {"request_id":str(uuid.uuid4()),"id":None,"expected_revision":0,
                        "project_id":setup["project"]["id"],"requested_session":root,"images":[],
                        "prompt":"Continue this project. Keep image-proof.txt unchanged. Project convention for future work: all *-proof.txt files contain exactly one lowercase English color word. The requested color for a new answer-proof.txt is blue. Before writing it, you MUST use the question tool to ask 'Which color for answer-proof.txt?' with choices blue and red, and wait for the answer. Then write the answered color as one lowercase word into answer-proof.txt. Do not delegate. Report the reusable project convention in your final summary."})
                    follow_deadline = time.monotonic() + 240
                    while time.monotonic() < follow_deadline:
                        snapshot = api("GET", "/v1/snapshot")
                        ticket = next(t for t in snapshot["tickets"] if t["id"] == follow["id"])
                        command = next((c for c in snapshot["commands"] if c["id"] == ticket.get("execution_command") and c["kind"] == "run_agent"), None)
                        if command:
                            assert command["state"] != "failed", command.get("result")
                            if command["state"] == "succeeded":
                                assert command["agent_spec"]["continuation"]["root_command_id"] == root
                                assert command["stream_id"] == completed[0]["stream_id"]
                                questions=[q for q in snapshot["questions"] if q["command_id"] == command["id"]]
                                assert questions and any(q["state"]=="answered" and q.get("orchestration_note", "").startswith("answer queued:") for q in questions), "No automatically answered question"
                                assert (Path(source)/"answer-proof.txt").read_text().strip()=="blue"
                                assert (Path(source)/"image-proof.txt").read_text().strip()=="red"
                                print("PASS real fixed-session continuation, automatic question answer and follow-up publication",flush=True)
                                verify_memory(command)
                                return
                        time.sleep(.3)
                    raise AssertionError("Follow-up with orchestrated question timed out")
                verify_memory(completed[0])
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
    parser.add_argument("--provider", required=True, help="Explicit provider technical ID")
    parser.add_argument("--model", required=True, help="Explicit model technical ID; no default or fallback")
    parser.add_argument("--local-registration", type=Path, help="Use an existing local service without starting/stopping it; retain separate Factory evidence under /tmp/opencode")
    parser.add_argument("--followup", action="store_true", help="Also verify fixed-session continuation and an automatically answered real model question")
    parser.add_argument("--memory", action="store_true", help="Verify automatic branch-scoped knowledge extraction after the follow-up")
    args = parser.parse_args()
    if args.memory:
        args.followup=True
    if args.local_registration:
        inside(ROOT / "target/debug", args.provider, args.model, json.loads(args.local_registration.read_text()), args.followup, args.memory)
        return
    if args.inside:
        inside(args.inside, args.provider, args.model, followup=args.followup, memory=args.memory)
        return
    state = json.loads((ROOT / "spikes/opencode/.runtime/state.json").read_text())
    container = state["container"]
    target = "/tmp/factory-image-binaries-" + secrets.token_hex(8)
    subprocess.run(["docker", "exec", container, "mkdir", target], check=True, capture_output=True)
    try:
        for name in ["factoryd", "factory-runner"]:
            subprocess.run(["docker", "cp", str(ROOT / "target/debug" / name), container + ":" + target + "/" + name], check=True, capture_output=True)
        subprocess.run(["docker", "cp", str(Path(__file__).resolve()), container + ":" + target + "/test.py"], check=True, capture_output=True)
        result = subprocess.run(["docker", "exec", "-i", container, "python3", target + "/test.py", "--inside", target,"--provider",args.provider,"--model",args.model] + (["--followup"] if args.followup else []) + (["--memory"] if args.memory else []),
            input=json.dumps({"url": "http://127.0.0.1:4096", "password": state["password"]}), text=True, timeout=900 if args.memory else 600 if args.followup else 300)
        assert result.returncode == 0, "Real image-prompt acceptance failed"
    finally:
        subprocess.run(["docker", "exec", container, "rm", "-rf", target], check=True, capture_output=True)


if __name__ == "__main__":
    main()

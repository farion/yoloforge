#!/usr/bin/env python3
"""Real model acceptance: new folder -> automatic task -> inline image -> written result.

Uses the existing authenticated Docker lab without changing its service or credentials.
"""
import argparse
import base64
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
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


def inside(binaries, provider, model, registration=None, followup=False, memory=False, native_pause=False, memory_correction=False, supervisor=False):
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
                    return matching[0]
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
                if supervisor:
                    project=setup["project"]["id"];root=completed[0]["id"]
                    policy=api("GET","/v1/projects/"+project+"/supervisor")
                    assert policy and policy["enabled"]
                    api("PUT","/v1/projects/"+project+"/supervisor",{"request_id":str(uuid.uuid4()),"expected_revision":policy["revision"],"stream_id":policy["stream_id"],"enabled":True,"evaluation_budget":3,"interval_seconds":30})
                    # Automatic closure intentionally preserves a 15-minute follow-up grace.
                    supervisor_deadline=time.monotonic()+1200
                    while time.monotonic()<supervisor_deadline:
                        closure=api("GET","/v1/commands/"+root+"/closure")
                        if closure:
                            evaluations=api("GET","/v1/projects/"+project+"/supervisor/evaluations")
                            applied=[e for e in evaluations if e["target_command_id"]==root and e["resolved"] and (e.get("decision") or {}).get("action")=="close" and e.get("knowledge_id")]
                            assert len(applied)==1,evaluations
                            assert api("GET","/v1/commands/"+root+"/archive")["status"]["final_scan_complete"]
                            notes=api("GET","/v1/projects/"+project+"/knowledge")
                            assert any(n["id"]==applied[0]["knowledge_id"] and n.get("source_command")==root for n in notes)
                            print("PASS real supervisor closes achieved session goal with retained archive and source-linked memory",flush=True)
                            return
                        time.sleep(.5)
                    raise AssertionError("Supervisor did not close the achieved goal within its bounded evaluation budget")
                if native_pause:
                    root=completed[0]["id"];stream_id=completed[0]["stream_id"]
                    def fixed(prompt):
                        return api("POST","/v1/tasks",{"request_id":str(uuid.uuid4()),"id":None,"expected_revision":0,
                            "project_id":setup["project"]["id"],"requested_session":root,"images":[],"prompt":prompt})
                    def wait_for(predicate, seconds=180):
                        end=time.monotonic()+seconds
                        while time.monotonic()<end:
                            value=predicate(api("GET","/v1/snapshot"))
                            if value:return value
                            time.sleep(.3)
                        raise AssertionError("Native lifecycle acceptance timed out")
                    def execution(s,t):
                        ticket=next(ti for ti in s["tickets"] if ti["id"]==t["id"])
                        return next((c for c in s["commands"] if c["id"]==ticket.get("execution_command")),None)
                    def control(action):
                        s=api("GET","/v1/snapshot");stream=next(sv for sv in s["streams"] if sv["id"]==stream_id)
                        return api("POST","/v1/streams/"+stream_id+"/control",{"request_id":str(uuid.uuid4()),"expected_revision":stream["revision"],"action":action})
                    paused_task=fixed("Use one shell tool call to run exactly: printf started > native-pause-proof.txt; sleep 90. Do not delegate or ask questions. After the sleep report completion.")
                    paused_command=wait_for(lambda s: execution(s,paused_task) if (work/"native-pause-proof.txt").exists() else None)
                    control("pause")
                    interrupted=wait_for(lambda s: next((c for c in s["commands"] if c["id"]==paused_command["id"] and c["state"]=="failed"),None))
                    assert interrupted["result"]["agent"]["phase"]=="interrupted",interrupted.get("result")
                    archive=api("GET","/v1/commands/"+interrupted["id"]+"/archive")
                    assert archive["status"]["final_scan_complete"]
                    s=api("GET","/v1/snapshot")
                    assert next(c for c in s["controls"] if c["stream_id"]==stream_id)["state"]=="session_interrupted"
                    assert (work/"native-pause-proof.txt").read_text()=="started"
                    control("resume")
                    resumed_task=fixed("The prior command was intentionally interrupted. Do not rerun its sleep. Keep image-proof.txt unchanged. Replace native-pause-proof.txt with exactly the word resumed. Do not ask questions or delegate; report the result.")
                    resumed=wait_for(lambda s: next((c for c in s["commands"] if (execution(s,resumed_task) or {}).get("id")==c["id"] and c["state"] in ("succeeded","failed")),None))
                    assert resumed["state"]=="succeeded",resumed.get("result")
                    assert resumed["agent_spec"]["continuation"]=={"root_command_id":root,"previous_command_id":interrupted["id"]}
                    assert (Path(source)/"native-pause-proof.txt").read_text().strip()=="resumed"
                    assert (Path(source)/"image-proof.txt").read_text().strip()=="red"
                    print("PASS native interrupt, final archive, explicit release and new prompt in the same session; no process-stop claim",flush=True)
                    return
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
                                previous_note=verify_memory(command)
                                if memory_correction:
                                    corrected=api("POST","/v1/tasks",{"request_id":str(uuid.uuid4()),"id":None,"expected_revision":0,
                                        "project_id":setup["project"]["id"],"requested_session":root,"images":[],
                                        "prompt":"Change the previous proof-file convention: all *-proof.txt files must now contain exactly one UPPERCASE English color word, replacing the earlier lowercase convention. Update image-proof.txt to RED and answer-proof.txt to BLUE. Verify both file contents. Do not ask questions or delegate. Explicitly explain the old-to-new convention correction and retain the two color meanings in your final summary."})
                                    correction_deadline=time.monotonic()+240
                                    while time.monotonic()<correction_deadline:
                                        s=api("GET","/v1/snapshot")
                                        task_state=next(t for t in s["tickets"] if t["id"]==corrected["id"])
                                        changed=next((c for c in s["commands"] if c["id"]==task_state.get("execution_command")),None)
                                        if changed:
                                            assert changed["state"]!="failed",changed.get("result")
                                            if changed["state"]=="succeeded":break
                                        time.sleep(.3)
                                    else:raise AssertionError("Correction task timed out")
                                    replacement=verify_memory(changed)
                                    assert replacement.get("supersedes")==previous_note["id"],replacement
                                    notes=api("GET","/v1/projects/"+setup["project"]["id"]+"/knowledge")
                                    old=next(n for n in notes if n["id"]==previous_note["id"])
                                    assert old["retired"] and old["superseded_by"]==replacement["id"]
                                    assert replacement["branch"]==old["branch"] and replacement["source_command"]==changed["id"]
                                    assert (Path(source)/"image-proof.txt").read_text().strip()=="RED"
                                    assert (Path(source)/"answer-proof.txt").read_text().strip()=="BLUE"
                                    print("PASS real model corrects contradicted branch memory atomically with linked provenance",flush=True)
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
    parser.add_argument("--native-pause",action="store_true",help="Real native interrupt, archive, release and fixed-session continuation acceptance")
    parser.add_argument("--memory-correction",action="store_true",help="Verify real model supersedes a contradicted prior branch observation")
    parser.add_argument("--supervisor",action="store_true",help="Real autonomous closure of a completed task with archived memory")
    args = parser.parse_args()
    if args.native_pause and (args.followup or args.memory or args.memory_correction):
        parser.error("--native-pause must run separately from question/memory acceptance")
    if args.supervisor and (args.native_pause or args.followup or args.memory or args.memory_correction):
        parser.error("--supervisor must run separately from continuation acceptance")
    if args.memory_correction:
        args.memory=True
    if args.memory:
        args.followup=True
    if args.local_registration:
        inside(profile_dir(), args.provider, args.model, json.loads(args.local_registration.read_text()), args.followup, args.memory, args.native_pause, args.memory_correction, args.supervisor)
        return
    if args.inside:
        inside(args.inside, args.provider, args.model, followup=args.followup, memory=args.memory, native_pause=args.native_pause, memory_correction=args.memory_correction, supervisor=args.supervisor)
        return
    state = json.loads((ROOT / "spikes/opencode/.runtime/state.json").read_text())
    container = state["container"]
    target = "/tmp/factory-image-binaries-" + secrets.token_hex(8)
    subprocess.run(["docker", "exec", container, "mkdir", target], check=True, capture_output=True)
    try:
        for name in ["factoryd", "factory-runner"]:
            subprocess.run(["docker", "cp", str(profile_dir() / name), container + ":" + target + "/" + name], check=True, capture_output=True)
        subprocess.run(["docker", "cp", str(Path(__file__).resolve()), container + ":" + target + "/test.py"], check=True, capture_output=True)
        subprocess.run(["docker", "cp", str(ROOT / "scripts/cargo_paths.py"), container + ":" + target + "/cargo_paths.py"], check=True, capture_output=True)
        result = subprocess.run(["docker", "exec", "-i", container, "python3", target + "/test.py", "--inside", target,"--provider",args.provider,"--model",args.model] + (["--followup"] if args.followup else []) + (["--memory"] if args.memory else []) + (["--native-pause"] if args.native_pause else []) + (["--memory-correction"] if args.memory_correction else []) + (["--supervisor"] if args.supervisor else []),
            input=json.dumps({"url": "http://127.0.0.1:4096", "password": state["password"]}), text=True, timeout=900 if args.memory else 600 if args.followup else 300)
        assert result.returncode == 0, "Real image-prompt acceptance failed"
    finally:
        subprocess.run(["docker", "exec", container, "rm", "-rf", target], check=True, capture_output=True)


if __name__ == "__main__":
    main()

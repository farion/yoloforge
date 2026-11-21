#!/usr/bin/env python3
"""Real Factory/OpenCode writing test in the retained authenticated Docker lab.

Uses two real model calls. Never recreates the lab or its credential volume.
Only this invocation's Factory processes, sessions and workspace are removed.
"""
import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import base64

ROOT = Path(__file__).resolve().parents[1]


def check(value, message):
    if not value:
        raise RuntimeError(message)


def wait(predicate, seconds=180):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.3)
    raise RuntimeError("agent smoke deadline exceeded")


def inside(bin_dir):
    registration = json.load(sys.stdin)
    directory = Path(tempfile.mkdtemp(prefix="factory-agent-", dir="/home/factory/workspace"))
    processes = []
    sessions = []
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    auth = "Basic " + base64.b64encode(("opencode:" + registration["password"]).encode()).decode()
    token = secrets.token_hex(32)
    env = {**os.environ, "FACTORY_TOKEN": token}
    env.pop("FACTORY_TOKEN_FILE", None)
    runner_tokens = {}
    endpoint = None

    def request(url, method, body, authorization):
        headers = {"Authorization": authorization, "Content-Type": "application/json", "x-factory-protocol": "1"}
        req = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
        with opener.open(req, timeout=10) as response:
            data = response.read()
            return json.loads(data) if data else None

    def oc(method, path, body=None):
        return request(registration["url"] + path, method, body, auth)

    def api(method, path, body=None):
        return request(endpoint + path, method, body, "Bearer " + token)

    def cli(*args):
        result = subprocess.run([str(bin_dir / "factory"), "--server", endpoint, *args], env=env, capture_output=True, text=True, timeout=15)
        check(result.returncode == 0, "Factory CLI failed")
        return json.loads(result.stdout)

    def spawn(binary, *args):
        child_env = env
        if binary == "factory-runner":
            child_env = {**env, "FACTORY_TOKEN": runner_tokens[args[args.index("--id") + 1]]}
        process = subprocess.Popen([str(bin_dir / binary), *args], env=child_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        processes.append(process)
        return process

    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    def daemon(address):
        process = spawn("factoryd", "--listen", address, "--database", str(directory / "factory.db"))
        import select
        check(select.select([process.stdout], [], [], 10)[0], "Factory daemon startup timed out")
        line = process.stdout.readline()
        check("listening on" in line, "Factory daemon did not start")
        return process, line.strip().split()[-1]

    try:
        reg = directory / "registration.json"
        reg.write_text(json.dumps(registration))
        reg.chmod(0o600)
        repo = directory / "source"
        repo.mkdir()
        for args in [("init", "-b", "main"), ("config", "user.name", "Factory Test"),
                     ("config", "user.email", "factory@example.invalid")]:
            subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)
        (repo / "README.md").write_text("Disposable Factory agent test.\n")
        subprocess.run(["git", "-C", str(repo), "add", "."], check=True, capture_output=True)
        subprocess.run(["git", "-C", str(repo), "commit", "-m", "initial"], check=True, capture_output=True)
        server, address = daemon("127.0.0.1:0")
        endpoint = "http://" + address
        project = cli("project", "create", "--name", "Agent smoke", "--repository", str(repo))
        cli("project", "configure", project["id"], "--expected-revision", "1", "--toolchain", "Python 3",
            "--provider", "openai", "--model", "gpt-6-astra", "--test-command", "python3 -m py_compile proof.py")
        mapping = directory / "repositories.json"
        mapping.write_text(json.dumps({project["id"]: str(repo)}))
        runners = []
        for location in ["local", "server"]:
            runner = str(uuid.uuid4())
            enrollment = api("POST", "/v1/devices/enrollment", {"role": "runner", "runner_id": runner, "ttl_seconds": 60})
            runner_tokens[runner] = api("POST", "/v1/enroll", {"code": enrollment["code"], "name": location + "-runner"})["token"]
            arguments = ("--server", endpoint, "--id", runner, "--name", location, "--location", location,
                         "--journal", str(directory / (location + ".db")), "--repositories", str(mapping), "--opencode-registration", str(reg))
            runners.append((spawn("factory-runner", *arguments), arguments))
        wait(lambda: len(api("GET", "/v1/snapshot")["runners"]) == 2, 30)
        streams = []
        for process, arguments in runners:
            runner = arguments[arguments.index("--id") + 1]
            stream = cli("stream", "create", "--project", project["id"], "--goal", "write isolated proof", "--runner", runner)
            command = cli("stream", "prepare", stream["id"], "--expected-revision", "1")
            streams.append((stream, command))

        def command_state(command_id):
            return next(c for c in api("GET", "/v1/snapshot")["commands"] if c["id"] == command_id)

        def completed(command_id):
            command = command_state(command_id)
            check(command["state"] != "failed", "Factory command failed: " + str(command.get("result")))
            return command if command["state"] == "succeeded" else None

        workspaces = [wait(lambda c=c: completed(c["id"]))["result"]["workspace"] for _, c in streams]
        commands = []
        markers = []
        for (stream, _), workspace in zip(streams, workspaces):
            marker = secrets.token_hex(12)
            markers.append(marker)
            # Keep execution active long enough to restart Factory while the tool runs.
            script = "from pathlib import Path; import time; p=Path('invocations.txt'); p.open('a').write('once\\n'); Path('started').touch(); time.sleep(25); Path('proof.py').write_text(" + repr("VALUE = '" + marker + "'\n") + "); import py_compile; py_compile.compile('proof.py', doraise=True)"
            import shlex
            shell = "python3 -c " + shlex.quote(script)
            prompt = "Integration test. First use your interactive question tool to ask for a verification token as ONE free-text question without predefined options. Wait for the answer. Then use the shell tool exactly once, foreground, to execute this exact command in the current worktree: " + shell + ". Do not use other tools or commands, do not delegate, do not retry or background it. After it completes, report the received verification token verbatim, the created file and successful Python compilation."
            revision = next(s["revision"] for s in api("GET", "/v1/snapshot")["streams"] if s["id"] == stream["id"])
            command = cli("stream", "run", stream["id"], "--expected-revision", str(revision), "--prompt", prompt)
            commands.append(command)
            sessions.append("ses_factory_" + command["id"].replace("-", ""))

        for session in sessions:
            def exists():
                try:
                    return oc("GET", "/api/session/" + session)
                except urllib.error.HTTPError as error:
                    if error.code == 404:
                        return None
                    raise
            wait(exists)
            # Only disposable sessions: exercise both Factory forms and permissions.
            oc("PATCH", "/api/session/" + session, {"permissions": [
                {"action": "*", "resource": "*", "effect": "deny"},
                {"action": "question", "resource": "*", "effect": "allow"},
                {"action": "shell", "resource": "*", "effect": "ask"}]})
        ids = {c["id"] for c in commands}
        def pending_questions(kind):
            found = [q for q in cli("question", "list") if q["command_id"] in ids and q["external"]["kind"] == kind and q["state"] == "pending"]
            return found if len(found) == 2 else None
        forms = wait(lambda: pending_questions("form"))
        for process, _ in runners:
            stop(process)
        tokens = {}
        for question in forms:
            field = question["external"]["payload"]["fields"][0]
            answer_token = "factory_" + secrets.token_hex(12)
            tokens[question["command_id"]] = answer_token
            check(field["type"] == "string" or (field["type"] == "multiselect" and field.get("custom")), "expected free-text form")
            value = [answer_token] if field["type"] == "multiselect" else answer_token
            answer_args = ("question", "answer", question["id"], "--expected-revision", str(question["revision"]),
                           "--answer-json", json.dumps({field["key"]: value}), "--request-id", str(uuid.uuid4()))
            queued = cli(*answer_args)
            check(cli(*answer_args) == queued, "answer request replay changed queued result")
        stop(server)
        server, _ = daemon(address)
        runners = [(spawn("factory-runner", *arguments), arguments) for _, arguments in runners]
        permissions = wait(lambda: pending_questions("permission"))
        for question in permissions:
            cli("question", "answer", question["id"], "--expected-revision", str(question["revision"]), "--decision", "once")
        print("PASS persisted agent forms answered after restart; shell permissions answered through Factory", flush=True)
        wait(lambda: all((Path(w["path"]) / "started").exists() for w in workspaces), 180)
        check(all(s in oc("GET", "/api/session/active")["data"] for s in sessions), "agents did not overlap")
        print("PASS two real agents execute concurrently in isolated worktrees", flush=True)
        stop(runners[0][0])
        stop(server)
        server, _ = daemon(address)
        runners[0] = (spawn("factory-runner", *runners[0][1]), runners[0][1])
        results = [wait(lambda c=c: completed(c["id"]), 180) for c in commands]
        for command, result, workspace, marker, session in zip(commands, results, workspaces, markers, sessions):
            check(result["result"]["agent"]["phase"] == "succeeded", "agent did not finish")
            check(tokens[command["id"]] in result["result"]["agent"]["summary"], "agent did not consume the Factory answer")
            path = Path(workspace["path"])
            check(marker in (path / "proof.py").read_text(), "agent did not write expected content")
            check((path / "invocations.txt").read_text() == "once\n", "shell command executed more than once")
            messages = oc("GET", "/api/session/" + session + "/message")["data"]
            check(len([m for m in messages if m.get("type") == "user"]) == 1, "prompt duplicated after restart")
            check(any(path.glob("__pycache__/proof*.pyc")), "Python validation did not run")
            excerpt = api("GET", "/v1/commands/" + command["id"] + "/excerpt")
            check(excerpt and excerpt["session_id"] == session, "session excerpt not stored")
            check(any(m["kind"] == "assistant" and "[Tool:" in m["text"] for m in excerpt["messages"]), "tool output missing from excerpt")
            check(any(tokens[command["id"]] in m["text"] for m in excerpt["messages"]), "final answered context missing from excerpt")
            archive = api("GET", "/v1/commands/" + command["id"] + "/archive")
            check(archive["status"]["final_scan_complete"], "terminal result preceded final archive")
            archived = []
            while archive["entries"]:
                archived.extend(entry["message"] for entry in archive["entries"])
                archive = api("GET", "/v1/commands/" + command["id"] + "/archive?after=" + str(archive["next_after"]))
            check(any(m["id"] == "msg_factory_" + command["id"].replace("-", "") for m in archived), "original prompt missing from archive")
            check(any(tokens[command["id"]] in json.dumps(m) for m in archived), "final context missing from archive")
        check(not (repo / "proof.py").exists(), "agent wrote into source checkout")
        questions = [q for q in cli("question", "list") if q["command_id"] in ids]
        check(len(questions) == 4 and all(q["state"] == "answered" for q in questions), "question outcomes not durably confirmed")
        print("PASS real file edits and tests; daemon/runner restart did not duplicate prompts or shell execution", flush=True)
        stop(server)
        server, _ = daemon(address)
        check(all(api("GET", "/v1/commands/" + c["id"] + "/excerpt") for c in commands), "session excerpts lost after restart")
        check(all(api("GET", "/v1/commands/" + c["id"] + "/archive")["status"]["final_scan_complete"] for c in commands), "session archive lost after restart")
        print("PASS real session text/tool excerpts persist through daemon restart", flush=True)
        print("PASS paginated untruncated session archives and final-scan markers survive restart", flush=True)
        print("PASS both real agents use separately scoped persistent runner credentials", flush=True)
    finally:
        for process in reversed(processes):
            stop(process)
        clean = True
        for session in sessions:
            try:
                oc("POST", "/api/session/" + session + "/interrupt?resume=false")
                wait(lambda: session not in oc("GET", "/api/session/active")["data"], 15)
                oc("DELETE", "/api/session/" + session)
            except Exception:
                clean = False
        if clean:
            shutil.rmtree(directory)
        else:
            print("Cleanup incomplete; test workspace retained:", directory, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inside", type=Path)
    args = parser.parse_args()
    if args.inside:
        inside(args.inside)
        return
    state = json.loads((ROOT / "spikes/opencode/.runtime/state.json").read_text())
    container = state["container"]
    directory = "/tmp/factory-agent-binaries-" + secrets.token_hex(8)
    subprocess.run(["docker", "exec", container, "mkdir", directory], check=True, capture_output=True)
    try:
        for name in ["factoryd", "factory-runner", "factory"]:
            subprocess.run(["docker", "cp", str(ROOT / "target/debug" / name), container + ":" + directory + "/" + name], check=True, capture_output=True)
        subprocess.run(["docker", "cp", str(Path(__file__).resolve()), container + ":" + directory + "/smoke-agent.py"], check=True, capture_output=True)
        result = subprocess.run(["docker", "exec", "-i", container, "python3", directory + "/smoke-agent.py", "--inside", directory],
                                input=json.dumps({"url": "http://127.0.0.1:4096", "password": state["password"]}), text=True, timeout=600)
        check(result.returncode == 0, "real agent integration failed")
    finally:
        subprocess.run(["docker", "exec", container, "rm", "-rf", directory], check=True, capture_output=True)


if __name__ == "__main__":
    main()

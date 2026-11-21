#!/usr/bin/env python3
"""One real model call in a disposable dedicated OpenCode container.

Reads an already-valid OpenAI access token through an explicitly selected lab
registration; never copies its refresh token, database or sessions. No lab restart.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import subprocess
import tempfile
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def check(value, message):
    if not value:
        raise RuntimeError(message)


def run(*args, env=None, timeout=60):
    result = subprocess.run(args, capture_output=True, text=True, env=env, timeout=timeout)
    check(result.returncode == 0, f"{args[0]} failed (subprocess output withheld)")
    return result.stdout.strip()


def private_json(path, body):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as f:
        json.dump(body, f)


def http(url, auth, method, path, body=None):
    req = urllib.request.Request(url + path, method=method,
                                 headers={"Authorization": auth, "Content-Type": "application/json", "x-factory-protocol": "1"},
                                 data=None if body is None else json.dumps(body).encode())
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=10) as response:
        data = response.read()
        return json.loads(data) if data else None


def basic(reg):
    return "Basic " + base64.b64encode(("opencode:" + reg["password"]).encode()).decode()


def wait(predicate, seconds=180):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.3)
    raise RuntimeError("contained agent acceptance deadline exceeded")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--credential-source", type=Path, default=ROOT / "spikes/opencode/.runtime/registration.json")
    parser.add_argument("--image", default="yoloforge-contained-opencode:2.0.27")
    parser.add_argument("--model", default="gpt-6-astra")
    args = parser.parse_args()
    check(os.getuid() != 0, "Run acceptance as a non-root host user")
    source = json.loads(args.credential_source.read_text())
    credentials = http(source["url"], basic(source), "GET", "/api/credential")["data"]
    values = [c["value"] for c in credentials if c["integrationID"] == "openai" and c["active"]]
    check(len(values) == 1 and values[0]["type"] == "oauth", "Expected one active OpenAI OAuth credential")
    credential = dict(values[0])
    check(credential["expires"] > (time.time() + 1800) * 1000, "Source access token expires too soon; refresh via the original login first")
    credential["refresh"] = ""  # disposable service must never rotate the source refresh credential
    del credentials, values, source
    directory = Path(tempfile.mkdtemp(prefix="contained-agent-", dir="/tmp/opencode"))
    processes, logs = [], []
    container = None
    volume = "yf-contained-" + secrets.token_hex(12)
    volume_created = False
    succeeded = False
    runner = str(uuid.uuid4())
    token = secrets.token_hex(32)
    env = {**os.environ, "FACTORY_TOKEN": token}
    env.pop("FACTORY_TOKEN_FILE", None)
    endpoint = None

    def docker(*parts):
        # Same explicit local daemon as the runner, never the user's selected remote context.
        clean = {k: v for k, v in os.environ.items() if k not in ("DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH")}
        return run("docker", "--host", "unix:///var/run/docker.sock", *parts, env=clean, timeout=90)

    def spawn(name, *parts, child_env=None):
        log = open(directory / f"{name}-{len(logs)}.log", "w+")
        logs.append(log)
        process = subprocess.Popen([str(ROOT / "target/debug" / name), *parts], env=child_env or env,
                                   stdout=log, stderr=subprocess.STDOUT)
        processes.append(process)
        return process, log

    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    def daemon(address):
        process, log = spawn("factoryd", "--listen", address, "--database", str(directory / "factory.db"))
        def ready():
            check(process.poll() is None, "Factory daemon exited; see private test logs")
            log.seek(0)
            lines = log.read().splitlines()
            return next((line.split()[-1] for line in lines if "listening on" in line), None)
        return process, wait(ready, 15)

    def api(method, path, body=None):
        return http(endpoint, "Bearer " + token, method, path, body)

    def cli(*parts):
        return json.loads(run(str(ROOT / "target/debug/yoloforge"), "--server", endpoint, *parts, env=env))

    def snapshot():
        return api("GET", "/v1/snapshot")

    def revision(stream):
        return next(s["revision"] for s in snapshot()["streams"] if s["id"] == stream)

    def completed(command):
        item = next(c for c in snapshot()["commands"] if c["id"] == command)
        check(item["state"] != "failed", "Agent command failed; see private journal/logs")
        return item if item["state"] == "succeeded" else None

    try:
        server, address = daemon("127.0.0.1:0")
        endpoint = "http://" + address
        repo = directory / "source"
        repo.mkdir()
        for command in [("init", "-b", "main"), ("config", "user.name", "Freezer Test"), ("config", "user.email", "freezer@example.invalid")]:
            run("git", "-C", str(repo), *command)
        (repo / "README.md").write_text("Disposable contained agent test.\n")
        run("git", "-C", str(repo), "add", ".")
        run("git", "-C", str(repo), "commit", "-m", "initial")
        project = cli("project", "create", "--name", "Contained real agent", "--repository", str(repo))
        cli("project", "configure", project["id"], "--expected-revision", "1", "--provider", "openai", "--model", args.model)
        mapping = directory / "repositories.json"
        private_json(mapping, {project["id"]: str(repo)})
        enrollment = api("POST", "/v1/devices/enrollment", {"role": "runner", "runner_id": runner, "ttl_seconds": 60})
        runner_token = api("POST", "/v1/enroll", {"code": enrollment["code"], "name": "contained-acceptance"})["token"]
        runner_env = {**env, "FACTORY_TOKEN": runner_token}
        runner_args = ["--server", endpoint, "--id", runner, "--name", "contained-acceptance", "--journal", str(directory / "runner.db"), "--repositories", str(mapping)]
        worker, _ = spawn("factory-runner", *runner_args, child_env=runner_env)
        wait(lambda: snapshot()["runners"], 20)
        stream = cli("stream", "create", "--project", project["id"], "--goal", "freeze and resume the same tool", "--runner", runner)["id"]
        prepared = cli("stream", "prepare", stream, "--expected-revision", "1")
        work = Path(wait(lambda: completed(prepared["id"]))["result"]["workspace"]["path"])
        stop(worker)
        labels = ["--label", "io.yoloforge.stream=" + stream, "--label", "io.yoloforge.runner=" + runner]
        docker("volume", "create", *labels, volume)
        volume_created = True
        mount_home = f"type=volume,source={volume},target=/home/factory"
        docker("run", "--rm", "--user", "0", "--mount", mount_home, args.image, "chown", "-R", f"{os.getuid()}:{os.getgid()}", "/home/factory")
        password = secrets.token_urlsafe(32)
        envfile = directory / "opencode.env"
        with os.fdopen(os.open(envfile, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as f:
            f.write("OPENCODE_PASSWORD=" + password + "\nHOME=/home/factory\n")
        container = docker("create", *labels, "--read-only", "--user", f"{os.getuid()}:{os.getgid()}",
                           "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--cgroupns", "private", "--ipc", "private", "--restart", "no",
                           "--tmpfs", "/tmp:rw,nosuid,nodev", "--env-file", str(envfile), "-p", "127.0.0.1::4096", "--mount", mount_home,
                           "--mount", f"type=bind,source={work},target={work}", "--mount", f"type=bind,source={repo / '.git'},target={repo / '.git'},readonly",
                           "--workdir", str(work), args.image)
        docker("start", container)
        port = json.loads(docker("inspect", container))[0]["NetworkSettings"]["Ports"]["4096/tcp"][0]["HostPort"]
        reg = {"url": "http://127.0.0.1:" + port, "password": password}
        def oc(method, path, body=None):
            return http(reg["url"], basic(reg), method, path, body)
        def oc_ready():
            try:
                return oc("GET", "/api/info")["version"] == "2.0.27"
            except (OSError, ValueError):
                return False
        wait(oc_ready, 90)
        oc("POST", "/api/credential", {"integrationID": "openai", "value": credential, "label": "Disposable access-only acceptance", "activate": True})
        del credential
        registration = directory / "registration.json"
        private_json(registration, reg)
        containment = directory / "containment.json"
        private_json(containment, {"stream_id": stream, "container_id": container, "docker_socket": "/var/run/docker.sock"})
        runner_args += ["--opencode-registration", str(registration), "--containment", str(containment)]
        stop(server)
        server, _ = daemon(address)  # drop the previous live registration lease
        worker, _ = spawn("factory-runner", *runner_args, child_env=runner_env)
        wait(lambda: snapshot()["runners"], 20)
        marker = secrets.token_hex(16)
        script = """from pathlib import Path
import os, signal, time, py_compile
Path('invocations').open('a').write('once\\n')
pid=os.fork()
if pid==0:
    os.setsid()
    if os.fork()!=0: os._exit(0)
    signal.signal(signal.SIGTERM,signal.SIG_IGN)
    signal.signal(signal.SIGHUP,signal.SIG_IGN)
    while not Path('release').exists():
        Path('detached').open('a').write('x')
        time.sleep(.1)
    Path('child_done').touch()
    os._exit(0)
os.waitpid(pid,0)
Path('started').touch()
while not Path('release').exists():
    Path('foreground').open('a').write('x')
    time.sleep(.1)
while not Path('child_done').exists(): time.sleep(.1)
Path('proof.py').write_text('VALUE = '+repr(MARKER)+'\\n')
py_compile.compile('proof.py',doraise=True)
print(MARKER)
""".replace("MARKER", repr(marker))
        shell = "python3 -c " + shlex.quote(script)
        prompt = "Integration test: execute exactly this command once using your shell tool in the current worktree, foreground. Allow up to 240 seconds for the external test harness to create the release file. Do not create release yourself, do not retry, background, delegate or use other tools. After it finishes report the printed token and compilation success. Command:\n" + shell
        command = cli("stream", "run", stream, "--expected-revision", str(revision(stream)), "--prompt", prompt)
        session = "ses_factory_" + command["id"].replace("-", "")
        def started():
            check(worker.poll() is None, "Contained runner exited; private logs retained")
            for q in cli("question", "list"):
                if q["command_id"] == command["id"] and q["state"] == "pending":
                    check(q["external"]["kind"] == "permission", "Unexpected interactive model form")
                    cli("question", "answer", q["id"], "--expected-revision", str(q["revision"]), "--decision", "once")
            return (work / "started").exists() and (work / "detached").exists()
        wait(started)
        check(session in oc("GET", "/api/session/active")["data"], "Tool was not active before pause")
        cli("stream", "pause", stream, "--expected-revision", str(revision(stream)))
        def control_state(state):
            return next((c for c in snapshot()["controls"] if c["stream_id"] == stream and c["state"] == state), None)
        frozen = wait(lambda: control_state("frozen"), 45)
        check(frozen["proof"]["container_id"] == container, "Wrong frozen container")
        def counts():
            return tuple((work / name).stat().st_size for name in ("foreground", "detached"))
        before = counts()
        time.sleep(2)
        check(counts() == before, "Tool writes continued while frozen")
        stop(worker)
        stop(server)
        server, _ = daemon(address)
        worker, _ = spawn("factory-runner", *runner_args, child_env=runner_env)
        wait(lambda: snapshot()["runners"], 20)
        wait(lambda: control_state("frozen")["proof"]["observed_unix_ms"] > frozen["proof"]["observed_unix_ms"], 45)
        check(counts() == before, "Tool progressed during Factory/runner restart")
        check(json.loads(docker("inspect", container))[0]["State"]["Paused"], "Container not paused after restart")
        print("PASS real OpenCode tool and detached child frozen across Factory/runner restart", flush=True)
        cli("stream", "resume", stream, "--expected-revision", str(revision(stream)))
        thawed = wait(lambda: control_state("released"), 45)
        check(thawed["proof"]["started_at"] == frozen["proof"]["started_at"], "Runtime incarnation changed")
        wait(lambda: all(a > b for a, b in zip(counts(), before)), 15)
        (work / "release").touch()
        result = wait(lambda: completed(command["id"]), 180)
        check((work / "invocations").read_text() == "once\n", "Tool was executed more than once")
        check(marker in (work / "proof.py").read_text(), "Tool proof missing")
        check(result["result"]["agent"]["session_id"] == session, "Session identity changed")
        exported = cli("archive", command["id"], "--output", str(directory / "archive.jsonl"))
        check(exported["final_scan_complete"] and exported["versions"] > 0, "Final archive incomplete")
        check(marker in (directory / "archive.jsonl").read_text(), "Final archive does not contain proof")
        print("PASS same real session thawed, single tool invocation completed and full final archive exported", flush=True)
        succeeded = True
    finally:
        for process in reversed(processes):
            stop(process)
        # Docker IDs are created by this invocation; never target the credential-source lab.
        if container:
            docker("rm", "-f", container)
        if volume_created:
            docker("volume", "rm", volume)
        for log in logs:
            log.close()
        if succeeded:
            shutil.rmtree(directory)
        else:
            print("Acceptance failed; private diagnostic workspace retained:", directory, flush=True)


if __name__ == "__main__":
    main()

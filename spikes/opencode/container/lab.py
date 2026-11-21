#!/usr/bin/env python3
"""Disposable Docker integration lab; never mounts host OpenCode state or credentials."""
import argparse
import base64
import concurrent.futures
import hashlib
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

ROOT = Path(__file__).resolve().parents[3]
RUNTIME = ROOT / "spikes/opencode/.runtime"
IMAGE = "yolofactory-opencode-spike:2.0.27"
LABEL = "io.yolofactory.spike"


def run(*args, capture=True, timeout=180, input_text=None):
    result = subprocess.run(args, capture_output=capture, text=True, timeout=timeout, cwd=ROOT,
                            input=input_text)
    if result.returncode:
        # Do not dump subprocess output: authentication tools may print credentials.
        raise RuntimeError(f"{args[0]} failed with exit code {result.returncode}")
    return result.stdout.strip() if capture else None


def private_json(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(data, stream)


class Api:
    def __init__(self, state):
        self.url = state["url"]
        self.auth = "Basic " + base64.b64encode(
            ("opencode:" + state["password"]).encode()
        ).decode()
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def call(self, method, path, body=None, expected=200):
        request = urllib.request.Request(
            self.url + path,
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Authorization": self.auth, "Content-Type": "application/json"},
            method=method,
        )
        try:
            response = self.opener.open(request, timeout=10)
        except urllib.error.HTTPError as error:
            if error.code == expected:
                error.close()
                return None
            raise RuntimeError(f"{method} {path}: HTTP {error.code}") from None
        with response:
            if response.status != expected:
                raise RuntimeError(f"{method} {path}: HTTP {response.status}, expected {expected}")
            body = response.read()
            return json.loads(body) if body else None

    def ready(self):
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            try:
                info = self.call("GET", "/api/info")
                check(info["version"] == "2.0.27", "unexpected API version")
                # API listener readiness does not imply provider catalog readiness on first boot.
                self.call("GET", "/api/integration/openai")
                return
            except (OSError, RuntimeError):
                time.sleep(0.5)
        raise RuntimeError("isolated API did not become ready within 90 seconds")


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def build(binary):
    binary = Path(binary).expanduser().resolve()
    check(run(str(binary), "--version") == "opencode v2.0.27", "expected local binary 2.0.27")
    Path("/tmp/opencode").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="factory-build-", dir="/tmp/opencode") as directory:
        shutil.copy2(binary, Path(directory) / "opencode")
        shutil.copy2(Path(__file__).with_name("Dockerfile"), Path(directory) / "Dockerfile")
        run("docker", "build", "-t", IMAGE, directory, capture=False, timeout=300)
    with binary.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    print("Built isolated image; binary SHA256:", digest)


def up():
    RUNTIME.mkdir(mode=0o700, parents=True, exist_ok=True)
    check(not (RUNTIME / "state.json").exists(), "lab already exists; use down before up")
    suffix = secrets.token_hex(6)
    state = {"container": "yf-spike-" + suffix, "volume": "yf-spike-data-" + suffix,
             "password": secrets.token_urlsafe(32), "label": suffix}
    private_json(RUNTIME / "state.json", state)
    # Persist ownership metadata before provisioning so partial failures remain recoverable.
    run("docker", "volume", "create", "--label", f"{LABEL}={suffix}", state["volume"])
    env_path = RUNTIME / "server.env"
    fd = os.open(env_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write("OPENCODE_PASSWORD=" + state["password"] + "\n")
    run("docker", "run", "-d", "--name", state["container"], "--label", f"{LABEL}={suffix}",
        "--init", "--env-file", str(env_path), "--publish", "127.0.0.1::4096",
        "--mount", f"type=volume,source={state['volume']},target=/home/factory", IMAGE)
    refresh_endpoint(state)
    Api(state).ready()
    print("Lab ready:", state["container"], state["url"])
    return state


def refresh_endpoint(state):
    port = run("docker", "port", state["container"], "4096/tcp")
    check(port.startswith("127.0.0.1:"), "container port is not loopback-only")
    state["url"] = "http://" + port
    # Atomic replacement; file contains the lab-only credential, never a host credential.
    private_json(RUNTIME / "state.next.json", state)
    (RUNTIME / "state.next.json").replace(RUNTIME / "state.json")
    private_json(RUNTIME / "registration.next.json", {k: state[k] for k in ("url", "password")})
    (RUNTIME / "registration.next.json").replace(RUNTIME / "registration.json")


def load():
    return json.loads((RUNTIME / "state.json").read_text())


def owned(state, kind, name):
    result = subprocess.run(["docker", kind, "inspect", name], capture_output=True, text=True)
    if result.returncode:
        return False
    obj = json.loads(result.stdout)[0]
    labels = obj.get("Config", {}).get("Labels", {}) if kind == "container" else obj.get("Labels", {})
    check((labels or {}).get(LABEL) == state["label"], "refusing to touch an unrelated Docker resource")
    return True


def down():
    if not (RUNTIME / "state.json").exists():
        return
    state = load()
    if owned(state, "container", state["container"]):
        run("docker", "rm", "-f", state["container"])
    if owned(state, "volume", state["volume"]):
        run("docker", "volume", "rm", state["volume"])
    for name in ("state.json", "state.next.json", "registration.json", "registration.next.json", "server.env"):
        (RUNTIME / name).unlink(missing_ok=True)
    print("Removed lab container, disposable volume and local lab credentials")


def prepare_headless_login(state):
    check(owned(state, "container", state["container"]), "lab container missing")
    # Install as the container user without rebuilding/restarting or touching credentials.
    # This directory is only put on PATH for the login CLI below.
    helper = Path(__file__).with_name("headless-open").read_text()
    run("docker", "exec", "-i", state["container"], "sh", "-c",
        'umask 077; mkdir -p "$HOME/.local/share/factory-lab/browser" && '
        'cat > "$HOME/.local/share/factory-lab/browser/xdg-open" && '
        'chmod 700 "$HOME/.local/share/factory-lab/browser/xdg-open"', input_text=helper)


def headless_command(*args):
    return ["sh", "-c", 'export PATH="$HOME/.local/share/factory-lab/browser:$PATH"; exec "$@"',
            "factory-headless-login", *args]


def lifecycle(state):
    api = Api(state)
    session = api.call("POST", "/api/session", {
        "title": "Factory isolated restart probe", "location": {"directory": "/home/factory/workspace"},
        "permissions": [{"action": "factory_spike", "resource": "*", "effect": "ask"}],
    })["data"]["id"]
    base = "/api/session/" + session
    prompt = {"id": "msg_factory_restart_" + session, "text": "Parked integration probe",
              "resume": False, "delivery": "queue"}
    api.call("POST", base + "/prompt", prompt)
    # Concurrent identical delivery: a retry and an original request may overlap.
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        replies = list(pool.map(lambda _: api.call("POST", base + "/prompt", prompt), range(2)))
    check(all(reply["data"]["id"] == prompt["id"] for reply in replies), "concurrent identities differ")
    changed = api.call("POST", base + "/prompt", {**prompt, "text": "Conflicting body"})
    check(changed["data"] == replies[0]["data"], "same ID with new body changed admitted prompt")
    print("OBSERVED same message ID with different body: HTTP 200, original input retained")
    form = api.call("POST", base + "/form", {
        "title": "Durable question", "fields": [{"key": "answer", "type": "string"}],
    })["data"]["id"]
    permission = api.call("POST", base + "/permission", {
        "action": "factory_spike", "resources": ["synthetic"],
    })["data"]["id"]
    # A session.shell call can consume parked input in 2.0.27; do not wake the
    # session while testing durability. This write is performed outside OpenCode.
    run("docker", "exec", state["container"], "sh", "-c",
        "printf factory-checkpoint > /home/factory/workspace/checkpoint.txt")
    check(len(api.call("GET", base + "/inbox")["data"]) == 1,
          "parked prompt consumed before restart by shell execution")
    print("PASS concurrent retry / first payload retained / checkpoint written")

    for action in ("restart", "kill"):
        # Restore test inputs before EACH restart so the SIGKILL case measures
        # newly pending requests, not absence inherited from the earlier restart.
        if not any(item["id"] == form for item in api.call("GET", base + "/form")["data"]):
            api.call("POST", base + "/form", {
                "id": form, "title": "Durable question",
                "fields": [{"key": "answer", "type": "string"}],
            })
        if not any(item["id"] == permission for item in api.call("GET", base + "/permission")["data"]):
            api.call("POST", base + "/permission", {
                "id": permission, "action": "factory_spike", "resources": ["synthetic"],
            })
        check(owned(state, "container", state["container"]), "lab container missing")
        if action == "restart":
            run("docker", "restart", "--time", "10", state["container"])
        else:
            run("docker", "kill", "--signal", "KILL", state["container"])
            run("docker", "start", state["container"])
        # Docker may reassign an ephemeral published port when restarting a container.
        refresh_endpoint(state)
        api = Api(state)
        api.ready()
        check(api.call("GET", base)["data"]["id"] == session, "session not restored")
        inbox = api.call("GET", base + "/inbox")["data"]
        check(len(inbox) == 1 and inbox[0]["id"] == prompt["id"], "parked prompt lost or duplicated")
        api.call("POST", base + "/prompt", prompt)
        check(len(api.call("GET", base + "/inbox")["data"]) == 1, "restart retry duplicated")
        forms = api.call("GET", base + "/form")["data"]
        print(f"OBSERVED {action}: pending form restored={any(item['id'] == form for item in forms)}")
        permissions = api.call("GET", base + "/permission")["data"]
        # Permission durability is measured rather than assumed; losing it affects resync design.
        print(f"OBSERVED {action}: pending permission restored={any(p['id'] == permission for p in permissions)}")
        check(run("docker", "exec", state["container"], "cat", "/home/factory/workspace/checkpoint.txt")
              == "factory-checkpoint", "workspace checkpoint lost")
        print(f"PASS {action}: authenticated API / session / inbox dedup / workspace persisted")

    forms = api.call("GET", base + "/form")["data"]
    if not any(item["id"] == form for item in forms):
        # Simulate reconstruction from a durable Factory record, not an OpenCode replay.
        api.call("POST", base + "/form", {
            "id": form, "title": "Reconstructed question",
            "fields": [{"key": "answer", "type": "string"}],
        })
        print("PASS missing form reconstructed from saved ID/schema")
    api.call("POST", base + f"/form/{form}/reply", {"answer": {"answer": "after restart"}}, expected=204)
    api.call("DELETE", base, expected=204)
    print("PASS restored form answered; lifecycle session removed")


def model_probe(api, model):
    session = api.call("POST", "/api/session", {
        "title": "Factory subscription probe",
        "location": {"directory": "/home/factory/workspace"},
        "model": {"providerID": "openai", "id": model},
        "permissions": [{"action": "*", "resource": "*", "effect": "deny"}],
    })["data"]["id"]
    base = "/api/session/" + session
    try:
        marker = "FACTORY_OK_" + secrets.token_hex(4)
        api.call("POST", base + "/prompt", {
            "id": "msg_factory_model_" + session,
            "text": f"This is a connectivity test. Do not use any tools. Reply with exactly: {marker}",
        })
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            messages = api.call("GET", base + "/message")["data"]
            assistants = [m for m in messages if m.get("type") == "assistant"]
            for message in assistants:
                error = message.get("error")
                if error:
                    # Print only a bounded classification, not provider bodies or credentials.
                    kind = str(error.get("type", "unknown"))
                    if not all(c.isalnum() or c in "._-" for c in kind):
                        kind = "unknown"
                    text = str(error.get("message", "")).lower()
                    category = next((word for word in ("unsupported", "not supported", "unauthorized",
                                    "rate limit", "quota", "not found", "expired", "connection", "fetch")
                                     if word in text), "unclassified")
                    raise RuntimeError(f"model error: type={kind[:80]}, status={error.get('status')}, category={category}")
            matched = any(
                m.get("time", {}).get("completed") and m.get("finish") == "stop"
                and "".join(c.get("text", "") for c in m.get("content", [])
                            if c.get("type") == "text").strip() == marker
                for m in assistants
            )
            if matched and session not in api.call("GET", "/api/session/active")["data"]:
                print("PASS real model response:", model)
                return
            time.sleep(0.5)
        raise RuntimeError("model response did not finish within 120 seconds")
    finally:
        # Only touch the session created above. Do not delete the account or lab volume.
        api.call("POST", base + "/interrupt?resume=false")
        api.call("DELETE", base, expected=204)


def verify_login(state, model):
    check(owned(state, "container", state["container"]), "lab container missing")
    refresh_endpoint(state)
    api = Api(state)
    api.ready()
    connections = api.call("GET", "/api/integration/openai")["data"].get("connections", [])
    credentials = {c["id"] for c in connections if c.get("type") == "credential"}
    check(credentials, "no saved OpenAI connection; run login first")
    models = api.call("GET", "/api/model")["data"]
    check(any(m.get("providerID") == "openai" and m.get("id") == model for m in models),
          "requested model not offered by this server")
    check(not api.call("GET", "/api/session/active")["data"],
          "lab has active sessions; refusing restart test")
    model_probe(api, model)
    check(not api.call("GET", "/api/session/active")["data"],
          "lab has active sessions; refusing restart test")
    run("docker", "restart", "--time", "10", state["container"])
    refresh_endpoint(state)
    api = Api(state)
    api.ready()
    restored = api.call("GET", "/api/integration/openai")["data"].get("connections", [])
    check(credentials <= {c["id"] for c in restored if c.get("type") == "credential"},
          "saved OpenAI connection missing after restart")
    print("PASS OpenAI credential identity persisted across container restart")
    model_probe(api, model)
    print("PASS login usable after restart; container and login volume retained")


def question_probe(state, model):
    check(owned(state, "container", state["container"]), "lab container missing")
    refresh_endpoint(state)
    api = Api(state)
    api.ready()
    session = api.call("POST", "/api/session", {
        "title": "Factory agent question probe",
        "agent": "build",
        "location": {"directory": "/home/factory/workspace"},
        "model": {"providerID": "openai", "id": model},
        "permissions": [{"action": "*", "resource": "*", "effect": "deny"},
                        {"action": "question", "resource": "*", "effect": "allow"}],
    })["data"]["id"]
    base = "/api/session/" + session
    try:
        api.call("POST", base + "/prompt", {
            "id": "msg_factory_question_" + session,
            "text": "This is an integration test of your interactive question tool. "
                    "Use that tool now to ask the user for a verification token, as one free-text question. "
                    "Do not merely ask in your text response. Do not provide predefined answers. "
                    "Do not use shell, files, networking or other tools. "
                    "Wait for the tool answer, then reply with exactly VERIFIED: followed by the token, "
                    "without spaces or any other text.",
        })
        deadline = time.monotonic() + 120
        form = None
        while time.monotonic() < deadline:
            forms = api.call("GET", base + "/form")["data"]
            if forms:
                check(len(forms) == 1, "expected exactly one agent question")
                form = forms[0]
                break
            messages = api.call("GET", base + "/message")["data"]
            check(not any(m.get("error") for m in messages), "agent failed before asking question")
            if any(m.get("type") == "assistant" and m.get("finish") == "stop" for m in messages):
                raise RuntimeError("agent finished without opening an interactive form")
            time.sleep(0.5)
        check(form is not None, "agent question did not arrive within 120 seconds")
        check(len(form["fields"]) == 1, "expected one answer field")
        field = form["fields"][0]
        # The marker is generated only after the model asks; it cannot be copied from the prompt.
        token = "factory_" + secrets.token_hex(8)
        if field["type"] == "string" and (not field.get("options") or field.get("custom")):
            answer = token
        elif field["type"] == "multiselect" and field.get("custom"):
            answer = [token]
        else:
            raise RuntimeError("agent form does not allow the requested free-text answer")
        print("PASS agent-created form received; field type:", field["type"])
        api.call("POST", base + f"/form/{form['id']}/reply", {
            "answer": {field["key"]: answer},
        }, expected=204)
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            messages = api.call("GET", base + "/message")["data"]
            check(not any(m.get("error") for m in messages), "agent failed after answering question")
            matched = any(
                m.get("type") == "assistant" and m.get("finish") == "stop"
                and m.get("time", {}).get("completed")
                and "".join(c.get("text", "") for c in m.get("content", [])
                            if c.get("type") == "text").strip() == "VERIFIED:" + token
                for m in messages
            )
            if matched and session not in api.call("GET", "/api/session/active")["data"]:
                check(not api.call("GET", base + "/form")["data"], "answered form still pending")
                print("PASS agent consumed API answer and completed with the previously unknown token")
                return
            time.sleep(0.5)
        raise RuntimeError("agent did not complete with the supplied token within 120 seconds")
    finally:
        api.call("POST", base + "/interrupt?resume=false")
        api.call("DELETE", base, expected=204)
        print("Question test session removed; login and lab retained")


def process_identity(container, pid):
    check(str(pid).isdigit() and int(pid) > 1, "invalid probe PID")
    result = subprocess.run(["docker", "exec", container, "cat", f"/proc/{pid}/stat"],
                            capture_output=True, text=True, timeout=10)
    if result.returncode:
        return None
    # /proc stat's command name can contain spaces or parentheses.
    fields = result.stdout.rsplit(")", 1)[1].split()
    if fields[0] == "Z":
        return None  # A zombie cannot execute or write.
    return fields[19]  # starttime: distinguishes PID reuse


def interrupt_probe(state, model):
    check(owned(state, "container", state["container"]), "lab container missing")
    refresh_endpoint(state)
    api = Api(state)
    api.ready()
    container = state["container"]
    directory = "/home/factory/workspace/factory-interrupt-" + secrets.token_hex(8)
    run("docker", "exec", container, "mkdir", directory)
    worker = Path(__file__).with_name("interrupt-worker.sh").read_text()
    run("docker", "exec", "-i", container, "sh", "-c", 'cat > "$1/worker.sh"',
        "factory-probe", directory, input_text=worker)
    base = None
    identities = {}
    try:
        session = api.call("POST", "/api/session", {
            "title": "Factory active interrupt probe", "agent": "build",
            "location": {"directory": directory},
            "model": {"providerID": "openai", "id": model},
            "permissions": [{"action": "*", "resource": "*", "effect": "deny"},
                            {"action": "shell", "resource": "*", "effect": "allow"}],
        })["data"]["id"]
        base = "/api/session/" + session
        api.call("POST", base + "/prompt", {
            "id": "msg_factory_interrupt_" + session,
            "text": "Integration test: invoke the shell tool once, in the foreground, with exactly "
                    f"`sh {directory}/worker.sh`. Wait for completion; do not background it or use "
                    "other tools. The test harness will interrupt the execution. Do not retry it.",
        })
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            for name in ("parent", "child"):
                result = subprocess.run(["docker", "exec", container, "cat", f"{directory}/{name}.pid"],
                                        capture_output=True, text=True, timeout=10)
                if result.returncode == 0:
                    pid = result.stdout.strip()
                    identity = process_identity(container, pid)
                    if identity:
                        identities[pid] = identity
            messages = api.call("GET", base + "/message")["data"]
            check(not any(m.get("error") for m in messages), "agent failed before starting worker")
            running = any(c.get("type") == "tool" and c.get("state", {}).get("status") == "running"
                          for m in messages for c in m.get("content", []))
            if len(identities) == 2 and running:
                break
            time.sleep(0.5)
        check(len(identities) == 2 and running, "agent did not start the expected parent/child worker")
        before = run("docker", "exec", container, "cat", directory + "/heartbeat.txt")
        time.sleep(1)
        growing = run("docker", "exec", container, "cat", directory + "/heartbeat.txt")
        check(len(growing) > len(before), "child writer not progressing before interrupt")
        print("PASS agent tool running; parent and child alive; heartbeat growing")
        started = time.monotonic()
        result = api.call("POST", base + "/interrupt?resume=false")
        check(result.get("interrupted") is True, "active agent interrupt was not acknowledged")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            alive = any(process_identity(container, pid) == identity for pid, identity in identities.items())
            active = session in api.call("GET", "/api/session/active")["data"]
            if not alive and not active:
                break
            time.sleep(0.2)
        check(not alive and not active, "interrupt left a worker or agent execution alive")
        stopped = run("docker", "exec", container, "cat", directory + "/heartbeat.txt")
        time.sleep(2)
        check(run("docker", "exec", container, "cat", directory + "/heartbeat.txt") == stopped,
              "workspace writes continued after interrupt")
        print(f"PASS interrupted=true; tracked workers gone; no writes for 2s ({time.monotonic()-started:.1f}s total)")
        status = api.call("GET", base)["data"]
        check(status.get("outcome") == "interrupted", "session outcome is not interrupted")
        # Explicit continuation: one fresh message, no attempt to revive the killed shell.
        marker = "RESUMED_" + secrets.token_hex(8)
        api.call("POST", base + "/prompt", {
            "id": "msg_factory_resume_" + session,
            "text": "The interrupt test is complete. Do not use tools or restart the command. "
                    f"Reply with exactly {marker}",
        })
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            messages = api.call("GET", base + "/message")["data"]
            matched = any(m.get("type") == "assistant" and m.get("finish") == "stop"
                          and m.get("time", {}).get("completed")
                          and "".join(c.get("text", "") for c in m.get("content", [])
                                      if c.get("type") == "text").strip() == marker for m in messages)
            if matched and session not in api.call("GET", "/api/session/active")["data"]:
                check(run("docker", "exec", container, "cat", directory + "/heartbeat.txt") == stopped,
                      "resume restarted the writer")
                print("PASS same session resumed explicitly; interrupted worker not restarted")
                return
            time.sleep(0.5)
        raise RuntimeError("explicit continuation did not finish")
    finally:
        try:
            if base:
                api.call("POST", base + "/interrupt?resume=false")
        finally:
            # Kill only captured probe process identities, never unrelated/reused PIDs.
            for pid, identity in identities.items():
                if process_identity(container, pid) == identity:
                    run("docker", "exec", container, "kill", "-KILL", pid)
            if base:
                api.call("DELETE", base, expected=204)
            run("docker", "exec", container, "rm", "-rf", "--", directory)
            print("Interrupt probe removed; login and lab retained")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build", "up", "test", "login", "verify-login", "verify-question", "verify-interrupt", "down"])
    parser.add_argument("--binary", default=shutil.which("opencode"))
    parser.add_argument("--model", default="gpt-6-astra", help="OpenAI model for live probes (real subscription usage)")
    args = parser.parse_args()
    if args.command == "build":
        check(args.binary is not None, "opencode binary required")
        build(args.binary)
    elif args.command == "up":
        up()
    elif args.command == "down":
        down()
    elif args.command == "verify-login":
        verify_login(load(), args.model)
    elif args.command == "verify-question":
        question_probe(load(), args.model)
    elif args.command == "verify-interrupt":
        interrupt_probe(load(), args.model)
    elif args.command == "login":
        state = load()
        prepare_headless_login(state)
        # User-facing interactive flow: output intentionally goes directly to the terminal.
        run("docker", "exec", "-it", state["container"],
            *headless_command("opencode", "auth", "login", "openai",
                              "--server", "http://127.0.0.1:4096", "--method", "chatgpt-headless"),
            capture=False, timeout=600)
    else:
        check(not (RUNTIME / "state.json").exists(), "test requires fresh lab; existing login lab will not be deleted")
        try:
            state = up()
            run("cargo", "run", "--locked", "-p", "factory-opencode-spike", "--", "inspect",
                str(RUNTIME / "registration.json"), capture=False)
            lifecycle(state)
        finally:
            down()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        print(f"Lab error: {error}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("Interrupted; a manually started lab is retained.", file=sys.stderr)
        sys.exit(130)

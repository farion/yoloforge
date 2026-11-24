#!/usr/bin/env python3
"""Run disposable OpenCode processes against the Factory-access proof of concept."""

import argparse
import base64
import concurrent.futures
import contextlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from relay import Access, Relay, CODEX, NO_REDIRECT
from repo_config import preflight

VERSION = "2.4.1"
NAMES = ("Arbeit", "Privat")
PROBE = "Reply with exactly FACTORY_MODEL_OK. Do not use any tools."


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(url, method="GET", data=None, authorization=None, timeout=90):
    req = urllib.request.Request(url, method=method,
                                 data=json.dumps(data).encode() if data is not None else None)
    req.add_header("Content-Type", "application/json")
    if authorization:
        req.add_header("Authorization", authorization)
    try:
        with NO_REDIRECT.open(req, timeout=timeout) as result:
            body = result.read()
            return json.loads(body) if body else None
    except urllib.error.HTTPError as error:
        error.close()
        raise OSError(f"{method}: HTTP {error.code}") from None


class OpenCode:
    def __init__(self, binary, base, config, repo_config=None):
        self.workspace = base / "workspace"
        self.workspace.mkdir(mode=0o700, parents=True)
        if repo_config:
            (self.workspace / "opencode.jsonc").write_text(repo_config)
        preflight(self.workspace)
        for folder in ("home", "config/opencode", "data", "state", "cache", "runtime", "tmp"):
            (base / folder).mkdir(mode=0o700, parents=True, exist_ok=True)
        config_file = base / "config/opencode/opencode.json"
        config_file.touch(mode=0o600)
        config_file.write_text(json.dumps(config))
        self.env = {
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "HOME": str(base / "home"),
            "XDG_CONFIG_HOME": str(base / "config"),
            "XDG_DATA_HOME": str(base / "data"),
            "XDG_STATE_HOME": str(base / "state"),
            "XDG_CACHE_HOME": str(base / "cache"),
            "XDG_RUNTIME_DIR": str(base / "runtime"),
            "TMPDIR": str(base / "tmp"),
            "OPENCODE_PASSWORD": secrets.token_urlsafe(32),
        }
        self.binary = binary

    def __enter__(self):
        port = free_port()
        self.url = f"http://127.0.0.1:{port}"
        self.authorization = "Basic " + base64.b64encode(
            ("opencode:" + self.env["OPENCODE_PASSWORD"]).encode()).decode()
        self.process = subprocess.Popen(
            [self.binary, "serve", "--hostname", "127.0.0.1", "--port", str(port)],
            cwd=self.workspace, env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        try:
            for _ in range(120):
                require(self.process.poll() is None, "Private OpenCode stopped during startup")
                try:
                    require(self.call("/api/info", timeout=1)["version"] == VERSION,
                            f"Expected OpenCode {VERSION}")
                    return self
                except OSError:
                    time.sleep(0.25)
            raise RuntimeError("Private OpenCode did not start")
        except BaseException:
            self.__exit__()
            raise

    def __exit__(self, *_):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()

    def call(self, path, method="GET", data=None, timeout=90):
        return request(self.url + path, method, data, self.authorization, timeout)


@contextlib.contextmanager
def listening(server):
    with server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            server.shutdown()
            thread.join()


def oauth(central, label):
    path = "/api/integration/openai/connect/oauth"
    attempt = central.call(path, "POST", {"methodID": "chatgpt-headless", "label": label})["data"]
    # Deliberate, interactive display only. Do not put OAuth URLs/codes in test logs.
    print(f"OpenAI {label}: {attempt['url']}\n{attempt['instructions']}", flush=True)
    completion = {}
    if attempt["mode"] == "code":
        completion["code"] = input("Authorization code: ").strip()
    central.call(path + "/" + attempt["attemptID"] + "/complete", "POST", completion, timeout=300)
    entry = next((c for c in credentials(central) if c["label"] == label
                  and c["integrationID"] == "openai"), None)
    require(entry is not None, "Login completed but the named credential was not saved")
    return entry


def wait_for_oauth(central):
    for _ in range(120):
        entries = central.call("/api/integration", timeout=2)["data"]
        if any(e.get("id") == "openai" and any(m.get("id") == "chatgpt-headless"
               for m in e.get("methods", [])) for e in entries):
            return
        time.sleep(0.25)
    raise RuntimeError("OpenCode did not advertise its ChatGPT headless OAuth method")


def credentials(central):
    result = central.call("/api/credential")
    return result["data"]


def fake_credentials():
    return [
        {"id": "demo-" + name, "integrationID": "openai", "active": name == "Privat",
         "value": {"type": "oauth", "access": "fake-token-" + name, "refresh": "not-for-runners",
                   "expires": (time.time() + 3600) * 1000, "metadata": {"accountID": "fake-" + name}}}
        for name in NAMES
    ]


class FakeCodex(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        account = self.headers.get("ChatGPT-Account-Id", "").removeprefix("fake-")
        if (self.path != "/responses" or account not in NAMES
                or self.headers.get("Authorization") != f"Bearer fake-token-{account}"
                or body.get("store") is not False or body.get("stream") is not True):
            self.send_error(400)
            return
        with self.server.lock:
            self.server.calls.append(account)
        supplied = {t["name"] for t in body.get("tools", []) if t.get("type") == "function"}
        recorded = any(x.get("type") == "function_call_output" for x in body["input"])
        match = re.search(r"probe file: (/[\w/.-]+)", json.dumps(body["input"]))
        if match and "read" in supplied and not recorded:
            item = {"id": "fc_probe", "type": "function_call", "call_id": "call_probe",
                    "name": "read", "arguments": json.dumps({"path": match.group(1)})}
            events = [
                ("response.output_item.added", {"output_index": 0, "item": {**item, "arguments": ""}}),
                ("response.function_call_arguments.delta", {"item_id": item["id"], "delta": item["arguments"]}),
                ("response.output_item.done", {"output_index": 0, "item": item}),
            ]
        else:
            value = "FACTORY_" + account.upper()
            if recorded:
                with self.server.lock:
                    self.server.tool_outputs.extend(x.get("output") for x in body["input"]
                                                    if x.get("type") == "function_call_output")
            item = {"id": "msg_probe", "type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": value, "annotations": []}]}
            events = [
                ("response.output_item.added", {"output_index": 0, "item": {**item, "content": []}}),
                ("response.output_text.delta", {"item_id": item["id"], "output_index": 0, "content_index": 0, "delta": value}),
                ("response.output_text.done", {"item_id": item["id"], "output_index": 0, "content_index": 0, "text": value}),
                ("response.output_item.done", {"output_index": 0, "item": item}),
            ]
        events.insert(0, ("response.created", {"response": {"id": "resp_probe", "output": []}}))
        events.append(("response.completed", {"response": {"id": "resp_probe", "output": [item]}}))
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for seq, (event, value) in enumerate(events):
            data = json.dumps({"type": event, "sequence_number": seq, **value})
            self.wfile.write(f"event: {event}\ndata: {data}\n\n".encode())
            self.wfile.flush()


def worker_config(relay, key, model):
    return {
        "policies": [
            {"action": "provider.use", "resource": "*", "effect": "deny"},
            *[{"action": "provider.use", "resource": "factory-" + n.lower(), "effect": "allow"} for n in NAMES],
        ],
        "providers": {
            "factory-" + n.lower(): {
                "package": "@opencode/ai/providers/openai-compatible/responses",
                "settings": {"baseURL": relay.url + "/v1/" + n.lower(), "apiKey": key},
                "models": {model: {"capabilities": {"tools": True}}},
            }
            for n in NAMES
        },
    }


def probe(worker, name, model, interactive, tool=False):
    args = {"providerID": "factory-" + name.lower(), "id": model}
    session = worker.call("/api/session", "POST", {
        "title": "Factory access spike", "location": {"directory": str(worker.workspace)}, "model": args,
    })["data"]["id"]
    path = "/api/session/" + session
    expected = "FACTORY_MODEL_OK" if interactive else "FACTORY_" + name.upper()
    if not tool:
        result = worker.call(path + "/generate", "POST", {"prompt": PROBE})
        require(result["data"]["text"].strip() == expected,
                "Response did not match the selected access" + ("" if interactive else f": {result!r}"))
        return
    file = worker.workspace / "local-probe.txt"
    file.write_text("RUNNER_LOCAL_TOOL_OK\n")
    worker.call(path + "/prompt", "POST", {"text": f"Read this probe file: {file}"})
    worker.call("/api/experimental/session/" + session + "/wait", "POST", {})
    messages = worker.call(path + "/message")
    require(expected in json.dumps(messages), "Agent did not finish the local tool round trip")


def test_preflight(root):
    path = root / "validation"
    (path / ".opencode").mkdir(parents=True)
    file = path / ".opencode/opencode.jsonc"
    file.write_text('{"instructions": ["https://example.com/x"], /* ok */ "agents":{"build":{},},}')
    preflight(path)
    for content, field in (('{"providers":{}}', "providers"),
                           ('{"agents":{"build":{"model":"other/x"}}}', "agents.build.model"),
                           ('{"commands":{"review":{"model":"other/x"}}}', "commands.review.model")):
        file.write_text(content)
        try:
            preflight(path)
        except ValueError as error:
            require(str(error).startswith(f".opencode/opencode.jsonc: {field} "), "Invalid diagnostic")
        else:
            raise RuntimeError("Preflight accepted " + field)


def assert_rejected(url, token, model, status):
    try:
        request(url, "POST", {"model": model}, f"Bearer {token}", timeout=3)
    except OSError as error:
        require(str(error) == f"POST: HTTP {status}", "Unexpected relay rejection")
    else:
        raise RuntimeError("Relay did not refuse invalid input")


def smoke(args, stack, root):
    require(not Path("/etc/opencode").exists() and all(
        not (p / f).exists()
        for p in root.parents
        for f in ("opencode.json", "opencode.jsonc", ".opencode")
    ), "This local spike requires clean ancestor/system configuration; use a disposable environment")
    test_preflight(root)
    if args.chatgpt:
        central = stack.enter_context(OpenCode(args.binary, root / "central", {}))
        require(not credentials(central), "The central spike must start without inherited credentials")
        wait_for_oauth(central)
        entries = [oauth(central, name) for name in NAMES]
        require(entries[0]["value"]["metadata"]["accountID"] != entries[1]["value"]["metadata"]["accountID"],
                "Two distinct ChatGPT accounts are needed for the concurrent acceptance test")
        upstream = CODEX
        get_credentials = lambda: credentials(central)
    else:
        upstream_mock = stack.enter_context(listening(ThreadingHTTPServer(("127.0.0.1", 0), FakeCodex)))
        upstream_mock.tool_outputs, upstream_mock.calls, upstream_mock.lock = [], [], threading.Lock()
        upstream = f"http://127.0.0.1:{upstream_mock.server_port}/responses"
        entries = fake_credentials()
        get_credentials = lambda: entries
    routes = {n.lower(): Access(e["id"], args.model) for n, e in zip(NAMES, entries)}
    keys = [secrets.token_urlsafe(32) for _ in NAMES]
    relay = stack.enter_context(listening(Relay(routes, {key: set(routes) for key in keys}, get_credentials, upstream)))
    # Put an unmistakable setting in the parent *environment*, not in the
    # user's configuration. The child gets a freshly built environment.
    host_config = root / "outside.json"
    host_config.write_text('{"model":"foreign/never-use"}')
    old_config = os.environ.get("OPENCODE_CONFIG")
    os.environ["OPENCODE_CONFIG"] = str(host_config)
    try:
        workers = [stack.enter_context(OpenCode(args.binary, root / f"worker-{i}", worker_config(relay, key, args.model),
                                                '{"share":"disabled"}'))
                   for i, key in enumerate(keys)]
    finally:
        if old_config is None:
            os.environ.pop("OPENCODE_CONFIG")
        else:
            os.environ["OPENCODE_CONFIG"] = old_config
    for worker in workers:
        require(not credentials(worker), "A runner contains provider credentials")
        documents = [e for e in worker.call("/api/config") if e["type"] == "document"]
        require(any(e["path"] == str(worker.workspace / "opencode.jsonc") and
                    e["info"].get("share") == "disabled" for e in documents)
                and all(e["path"] != str(host_config) for e in documents),
                "Expected the repo setting, but no ambient host setting")
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for names in ((NAMES[0], NAMES[0]), NAMES):
            futures = [pool.submit(probe, worker, name, args.model, args.chatgpt)
                       for worker, name in zip(workers, names)]
            for future in futures:
                try:
                    future.result()
                except (RuntimeError, OSError):
                    if not args.chatgpt:
                        print("Synthetic upstream calls:", upstream_mock.calls)
                    raise
    if not args.chatgpt:
        probe(workers[0], NAMES[0], args.model, False, tool=True)
        require(any("RUNNER_LOCAL_TOOL_OK" in str(output) for output in upstream_mock.tool_outputs),
                f"Mock provider did not observe the local tool result: {upstream_mock.tool_outputs!r}")
        # The other account is still active and healthy, but must not be used.
        entries[0]["value"]["expires"] = 0
        observed = len(upstream_mock.calls)
        assert_rejected(relay.url + "/v1/arbeit/responses", keys[0], args.model, 503)
        require(len(upstream_mock.calls) == observed, "Expired access triggered another provider request")
    for route, key, model, status in (
        ("arbeit", "invalid", args.model, 403), ("arbeit", keys[0], "unapproved-model", 403),
        ("missing", keys[0], args.model, 403),
    ):
        assert_rejected(relay.url + "/v1/" + route + "/responses", key, model, status)
    print("PASS:", "ChatGPT OAuth" if args.chatgpt else "synthetic upstream + local tool",
          "— two OpenCode processes, named access routing, empty runner credential stores")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=shutil.which("opencode"))
    parser.add_argument("--model", default="gpt-6.2-sol")
    parser.add_argument("--chatgpt", action="store_true", help="interactive, uses two real ChatGPT accounts")
    args = parser.parse_args()
    require(args.binary, "Pass --binary for OpenCode " + VERSION)
    require(not args.chatgpt or sys.stdin.isatty(), "--chatgpt requires an interactive terminal")
    args.binary = str(Path(args.binary).resolve(strict=True))
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="factory-managed-access-", dir="/tmp/opencode") as directory:
        with contextlib.ExitStack() as stack:
            smoke(args, stack, Path(directory))

"""Probe skill and MCP scoping on one private OpenCode 2.4.1 service."""

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

VERSION = "2.4.1"


def write_json(path, data):
    path.write_text(json.dumps(data), encoding="utf-8")


def fixture_mcp(label, started):
    Path(started).touch()
    for line in sys.stdin:
        request = json.loads(line)
        if "id" not in request:
            continue
        method = request.get("method")
        if method == "tools/list":
            Path(started).with_suffix(".tools").touch()
        if method == "tools/call":
            Path(started).with_suffix(".call").touch()
        response = {
            "initialize": {
                "protocolVersion": request.get("params", {}).get("protocolVersion"),
                "serverInfo": {"name": label, "version": "1"},
                "capabilities": {"tools": {}},
            },
            "tools/list": {
                "tools": [{"name": label, "inputSchema": {"type": "object"}}]
            },
            "tools/call": {
                "content": [{"type": "text", "text": f"called-{label}"}],
            },
        }.get(method, {})
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": response}), flush=True)


class FakeModel(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/fixture-mcp":
            self.mcp_request(request)
            return
        output = [entry["output"] for entry in request["input"]
                  if entry.get("type") == "function_call_output"]
        self.server.requests.append((request, output))
        tools = {tool.get("name") for tool in request.get("tools", [])}
        inputs = json.dumps(request["input"])
        tool, arguments = None, {}
        if "invoke-runtime-test" in inputs and "runtime-test_runtime-test" in tools:
            tool = "runtime-test_runtime-test"
        elif "invoke-remote" in inputs and "fixture-remote_lookup" in tools:
            tool = "fixture-remote_lookup"
        elif "request-" in inputs and "skill" in tools:
            tool = "skill"
            arguments = {"id": "yf-catalog-probe-v1" if "request-central" in inputs
                         else "unselected" if "request-unselected" in inputs else "shared"}
        if not output and tool:
            item = {"id": "fc_probe", "type": "function_call", "call_id": "call_probe",
                    "name": tool, "arguments": json.dumps(arguments)}
            events = [
                ("response.output_item.added", {"output_index": 0, "item": {**item, "arguments": ""}}),
                ("response.function_call_arguments.delta", {"item_id": item["id"], "delta": item["arguments"]}),
            ]
        else:
            item = {"id": "msg_probe", "type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "done", "annotations": []}]}
            events = [
                ("response.output_item.added", {"output_index": 0, "item": {**item, "content": []}}),
                ("response.output_text.delta", {"item_id": item["id"], "output_index": 0,
                                                "content_index": 0, "delta": "done"}),
            ]
        events.insert(0, ("response.created", {"response": {"id": "resp_probe", "output": []}}))
        events.extend([
            ("response.output_item.done", {"output_index": 0, "item": item}),
            ("response.completed", {"response": {"id": "resp_probe", "output": [item]}}),
        ])
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for seq, (event, value) in enumerate(events):
            data = json.dumps({"type": event, "sequence_number": seq, **value})
            self.wfile.write(f"event: {event}\ndata: {data}\n\n".encode())

    def mcp_request(self, request):
        if self.headers.get("Authorization") != "Bearer fixture-proxy-lease":
            self.send_error(403)
            return
        if "id" not in request:
            self.send_response(202)
            self.end_headers()
            return
        result = {
            "initialize": {
                "protocolVersion": request.get("params", {}).get("protocolVersion"),
                "serverInfo": {"name": "fixture-remote", "version": "1"},
                "capabilities": {"tools": {}},
            },
            "tools/list": {"tools": [{"name": "lookup", "inputSchema": {"type": "object"}}]},
            "tools/call": {"content": [{"type": "text", "text": "remote-probe-ok"}]},
        }.get(request.get("method"), {})
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({"jsonrpc":"2.0","id":request["id"],"result":result}).encode())

    def do_GET(self):
        self.send_response(405)
        self.end_headers()


def probe(binary, root, model):
    for parent in root.parents:
        for name in ("opencode.json", "opencode.jsonc", ".opencode.json", ".opencode"):
            path = parent / name
            if path.exists() or path.is_symlink():
                raise RuntimeError(f"Unexpected ambient configuration: {path}")
    system = Path("/etc/opencode")
    if system.exists() or system.is_symlink():
        raise RuntimeError("Unexpected system configuration /etc/opencode")

    env = {"PATH": "/usr/local/bin:/usr/bin:/bin", "OPENCODE_PASSWORD": secrets.token_hex(32)}
    for key in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME",
                "XDG_CACHE_HOME", "XDG_RUNTIME_DIR", "TMPDIR"):
        directory = root / key.lower()
        directory.mkdir(mode=0o700)
        env[key] = str(directory)
    config = Path(env["XDG_CONFIG_HOME"]) / "opencode"
    config.mkdir()
    (config / "skills").mkdir()
    write_json(config / "opencode.json", {"update": "disable", "policies": [
        {"action": "mcp.use", "resource": "blocked", "effect": "deny"},
        {"action": "skill.use", "resource": "blocked", "effect": "deny"},
    ], "permissions": [{"action":"skill","resource":"yf-catalog-*","effect":"deny"}], "providers": {"fixture": {
        "package": "@opencode/ai/providers/openai-compatible/responses",
        "settings": {"baseURL": f"http://127.0.0.1:{model.server_port}/v1", "apiKey": "fixture"},
        "models": {"probe": {"capabilities": {"tools": True}}},
    }}})
    version = subprocess.check_output([binary, "--version"], env=env, cwd=root, text=True, timeout=5)
    if version.strip() != f"opencode v{VERSION}":
        raise RuntimeError(f"Expected OpenCode {VERSION}, got {version.strip()}")

    def git(*args):
        return subprocess.run(["git", *args], env={**env, "GIT_CONFIG_NOSYSTEM": "1"},
                              cwd=root, check=True, capture_output=True).stdout

    git("init", "repo")
    git("-C", "repo", "-c", "user.name=Profile Probe", "-c", "user.email=probe@example.invalid",
        "commit", "--allow-empty", "-m", "Fixture")
    for name in ("alpha", "beta", "plain"):
        git("-C", "repo", "worktree", "add", "--detach", str(root / name))

    def mcp(label):
        return {"type": "local", "command": [
            sys.executable, str(Path(__file__).resolve()), "--fixture-mcp", label,
            str(root / f"started-{label}"),
        ]}

    def profile(directory, label):
        for skill, description in (("shared", label), ("blocked", "Must stay hidden"),
                                   ("unselected", "Not allowed by this session")):
            path = root / directory / ".opencode" / "skills" / skill
            path.mkdir(parents=True, exist_ok=True)
            (path / "SKILL.md").write_text(
                f"---\nname: {skill}\ndescription: {description}\n---\n\nProbe: {description}.\n",
                encoding="utf-8",
            )
        write_json(root / directory / "opencode.json", {
            "mcp": {"servers": {label: mcp(label), "blocked": mcp("blocked")}},
            "policies": [
                {"action": action, "resource": "blocked", "effect": "allow"}
                for action in ("mcp.use", "skill.use")
            ],
        })

    profile("alpha", "alpha")
    profile("beta", "beta")
    with socket.socket() as port_socket:
        port_socket.bind(("127.0.0.1", 0))
        port = port_socket.getsockname()[1]
    auth = base64.b64encode(f"opencode:{env['OPENCODE_PASSWORD']}".encode()).decode()

    def api(path, directory=None, body=None, method=None):
        if directory:
            path += "?" + urlencode({"location[directory]": str(root / directory)})
        request = Request(f"http://127.0.0.1:{port}/api/{path}", method=method,
                          data=None if body is None else json.dumps(body).encode(),
                          headers={"Authorization": f"Basic {auth}", "Content-Type": "application/json"})
        with build_opener(ProxyHandler({})).open(request, timeout=10) as response:
            return None if response.status == 204 else json.load(response)

    with (root / "service.log").open("w+") as log:
        service = subprocess.Popen(
            [binary, "serve", "--hostname", "127.0.0.1", "--port", str(port)],
            cwd=root, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
            start_new_session=True,
        )
        try:
            for _ in range(60):
                if service.poll() is not None:
                    raise RuntimeError("OpenCode stopped during startup")
                try:
                    api("info")
                    break
                except URLError:
                    time.sleep(0.1)
            else:
                raise RuntimeError("OpenCode did not become ready")

            def create(directory):
                rules = [{"action": "skill", "resource": "*", "effect": "deny"}]
                if directory != "plain":
                    rules.append({"action": "skill", "resource": "shared", "effect": "allow"})
                result = api("session", body={
                    "location": {"directory": str(root / directory)}, "permissions": rules,
                    "model": {"providerID": "fixture", "id": "probe"},
                })
                assert result["data"]["location"]["directory"] == str(root / directory), result
                session = result["data"]["id"]
                assert api(f"session/{session}")["data"]["permissions"] == rules, result
                return session

            def check(directory, label):
                for _ in range(30):
                    skills = api("skill", directory)
                    servers = api("mcp", directory)
                    if isinstance(skills, dict):
                        assert skills["location"]["directory"] == str(root / directory), skills
                        skills = skills["data"]
                    if isinstance(servers, dict) and "data" in servers:
                        servers = servers["data"]
                    if isinstance(servers, list):
                        servers = {server["name"]: server for server in servers}
                    status = servers.get(label, {}).get("status")
                    if isinstance(status, dict):
                        status = status.get("status", status.get("type", status.get("_tag")))
                    if not label or (skills and status == "connected"):
                        break
                    time.sleep(0.1)
                our_skills = {s.get("id", s.get("name")): s["description"] for s in skills
                              if s.get("id", s.get("name")) in {"shared", "blocked"}}
                assert our_skills == ({"shared": label} if label else {}), (
                    directory, skills, servers, api("location", directory), api("config", directory)
                )
                assert set(servers) == ({label} if label else set()), servers
                if label:
                    assert status == "connected", servers
                    assert any(entry["id"] == "shared" and entry["path"] == str(
                        root / directory / ".opencode/skills/shared/SKILL.md"
                    ) for entry in skills), skills

            with ThreadPoolExecutor(max_workers=3) as pool:
                sessions = list(pool.map(create, ("alpha", "beta", "plain")))
                list(pool.map(lambda pair: check(*pair),
                              [("alpha", "alpha"), ("beta", "beta"), ("plain", None)]))
            check("plain", None)
            assert (root / "started-alpha").exists() and (root / "started-beta").exists()
            assert not (root / "started-blocked").exists(), "Denied MCP process started"
            print("PASS: parallel worktrees have distinct skill/MCP catalogs; global denies held")

            for requested in ("shared", "unselected"):
                session = create("alpha")
                before = len(model.requests)
                api(f"session/{session}/prompt", body={"text": f"request-{requested}"})
                api(f"experimental/session/{session}/wait", body={})
                calls = [(call, output) for call, output in model.requests[before:]
                         if any(tool.get("name") == "skill" for tool in call.get("tools", []))]
                assert len(calls) >= 2, calls
                advertised = json.dumps(calls[0][0]["input"])
                assert "Not allowed by this session" not in advertised, advertised
                output = json.dumps(calls[-1][1])
                assert ("Probe: alpha." in output) == (requested == "shared"), output
                assert "Probe: Not allowed by this session." not in output, output
            print("PASS: the agent's skill tool advertises and loads only approved skills")

            def activate(session, skill):
                return api(f"experimental/session/{session}/skill",
                           body={"id": skill, "resume": False})

            def skill_messages(session):
                return [message["text"].strip()
                        for message in api(f"session/{session}/message")["data"]
                        if message["type"] == "skill"]

            # The experimental direct activation API checks the global policy,
            # but is not a test of agent-side session permissions.
            for session, skill in [(sessions[0], "blocked"), (sessions[2], "shared")]:
                try:
                    activate(session, skill)
                except HTTPError as error:
                    assert error.code == 404, error
                else:
                    raise AssertionError(f"Unexpectedly activated {skill}")
            activate(sessions[0], "shared")
            assert skill_messages(sessions[0]) == ["Probe: alpha."]
            profile("alpha", "changed")
            api("location/reload", "alpha", {})
            with ThreadPoolExecutor(max_workers=2) as pool:
                list(pool.map(lambda pair: check(*pair), [("alpha", "changed"), ("beta", "beta")]))
            activate(sessions[0], "shared")
            activate(sessions[1], "shared")
            assert skill_messages(sessions[0]) == ["Probe: changed.", "Probe: alpha."]
            assert skill_messages(sessions[1]) == ["Probe: beta."]
            assert not (root / "started-blocked").exists(), "Denied MCP process started after reload"
            print("PASS: the same session reads the new skill revision after a location reload")

            path = "experimental/mcp/runtime-test"
            before = git("-C", "plain", "status", "--porcelain")
            config = {**mcp("runtime-test"), "codemode": False, "pool": False}
            api(path, "plain", {"config": config}, method="PUT")
            for _ in range(30):
                runtime = api("mcp", "plain")["data"]
                if any(s["name"] == "runtime-test" and s["status"]["status"] == "connected" for s in runtime):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError(runtime)
            check("beta", "beta")
            assert git("-C", "plain", "status", "--porcelain") == before
            session = api("session", body={"location": {"directory": str(root / "plain")},
                "model": {"providerID": "fixture", "id": "probe"},
                "permissions": [{"action": "runtime-test_*", "resource": "*", "effect": "allow"}],
            })["data"]["id"]
            before_model = len(model.requests)
            api(f"session/{session}/prompt", body={"text": "invoke-runtime-test"})
            api(f"experimental/session/{session}/wait", body={})
            assert any("called-runtime-test" in json.dumps(output) for _, output in model.requests[before_model:])
            print("PASS: a runtime-only MCP connects and calls a tool without editing the worktree")

            api("location/reload", "plain", {})
            assert api("mcp", "plain")["data"] == []
            api(path, "plain", {"config": config}, method="PUT")
            assert api("mcp", "plain")["data"][0]["name"] == "runtime-test"
            print("PASS: a runtime-only MCP can be reattached after a location reload")

            api("experimental/mcp/fixture-remote", "plain", {"config":{
                "type": "remote", "url": f"http://127.0.0.1:{model.server_port}/fixture-mcp",
                "headers": {"Authorization": "Bearer fixture-proxy-lease"},
                "oauth": False, "codemode": False, "pool": False,
            }}, method="PUT")
            statuses = api("mcp", "plain")["data"]
            assert any(s["name"] == "fixture-remote" and s["status"]["status"] == "connected" for s in statuses)
            session = api("session", body={"location": {"directory": str(root / "plain")},
                "model": {"providerID": "fixture", "id": "probe"},
                "permissions": [{"action": "fixture-remote_*", "resource": "*", "effect": "allow"}],
            })["data"]["id"]
            before_model = len(model.requests)
            api(f"session/{session}/prompt", body={"text": "invoke-remote"})
            api(f"experimental/session/{session}/wait", body={})
            assert any("remote-probe-ok" in json.dumps(output) for _, output in model.requests[before_model:])
            print("PASS: a remote Streamable HTTP MCP accepts the configured proxy bearer header")

            (root / "test-location").mkdir()
            api("experimental/mcp/yf-local-probe", "test-location", {"config": mcp("only-list")}, method="PUT")
            for _ in range(30):
                statuses = api("mcp", "test-location")["data"]
                if any(s["name"] == "yf-local-probe" and s["status"]["status"] == "connected" for s in statuses):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError(statuses)
            api("experimental/mcp/yf-local-probe", "test-location", method="DELETE")
            assert api("mcp", "test-location")["data"] == []
            assert (root / "started-only-list.tools").exists()
            assert not (root / "started-only-list.call").exists()
            print("PASS: a local MCP can be checked and removed from an isolated, non-Git location")

            central = Path(env["XDG_CONFIG_HOME"]) / "opencode/skills/yf-catalog-probe-v1"
            central.mkdir()
            (central / "SKILL.md").write_text(
                "---\nname: Central probe\ndescription: Central runtime fixture\n---\n\nCentral content token.\n",
                encoding="utf-8",
            )
            for _ in range(30):
                if any(skill["id"] == "yf-catalog-probe-v1" for skill in api("skill", "alpha")["data"]):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("A newly added central skill was not discovered")
            print("PASS: existing locations discover a new private global skill without a reload")
            for approved in [True, False]:
                permissions = [{"action":"skill","resource":"yf-catalog-*","effect":"deny"}]
                if approved:
                    permissions.append({"action":"skill","resource":"yf-catalog-probe-v1","effect":"allow"})
                session = api("session", body={"location":{"directory":str(root / "alpha")},
                    "model":{"providerID":"fixture","id":"probe"},"permissions":permissions})["data"]["id"]
                before_model = len(model.requests)
                api(f"session/{session}/prompt", body={"text":"request-central"})
                api(f"experimental/session/{session}/wait", body={})
                calls = model.requests[before_model:]
                assert any("Central content token" in json.dumps(output) for _,output in calls) == approved
            print("PASS: only sessions with an explicit version-specific grant can load a central skill")
        except Exception:
            log.seek(0)
            print(log.read()[-4000:], file=sys.stderr)
            raise
        finally:
            try:
                os.killpg(service.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                service.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(service.pid, signal.SIGKILL)
                service.wait()


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--fixture-mcp":
        fixture_mcp(*sys.argv[2:])
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("binary", type=Path, help="Path to an OpenCode 2.4.1 binary")
        args = parser.parse_args()
        with tempfile.TemporaryDirectory(prefix="yf-profile-probe-", dir="/tmp/opencode") as temp:
            with ThreadingHTTPServer(("127.0.0.1", 0), FakeModel) as model:
                model.requests = []
                thread = threading.Thread(target=model.serve_forever, daemon=True)
                thread.start()
                try:
                    probe(args.binary.resolve(strict=True), Path(temp).resolve(), model)
                finally:
                    model.shutdown()
                    thread.join()

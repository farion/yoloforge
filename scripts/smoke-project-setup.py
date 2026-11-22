#!/usr/bin/env python3
"""Real TUI/server/runner project setup; fake OpenCode health, no model invocation."""
import os
import json
import time
import uuid
import tempfile
import subprocess
import urllib.request
import http.server
import threading
import pty
import select
import signal
import fcntl
import termios
import struct
import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--auto", action="store_true", help="Exercise native-service discovery/start with a fixture executable")
    parser.add_argument("--empty", action="store_true", help="Create a project with Ctrl-S only: no repository, path, model or config input")
    parser.add_argument("--tasks", action="store_true", help="Also exercise prompt/image entry and project pause/resume")
    parser.add_argument("--native", action="store_true", help="Start a real isolated OpenCode service; never use the shared desktop service")
    parser.add_argument("--new-folder", action="store_true", help="Create a missing project folder rather than importing an existing repository")
    options = parser.parse_args()
    if options.new_folder:
        options.empty = True
    if options.empty:
        options.auto = True
    class Health(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            assert self.path == "/api/info"
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"version":"2.0.27"}')
        def log_message(self, *args):
            pass
    health = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Health)
    threading.Thread(target=health.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="project-setup-", dir="/tmp/opencode") as tmp:
        base = Path(tmp)
        repo = base / "repo"
        repo.mkdir()
        def git(*args):
            return subprocess.check_output(["git", "-C", str(repo), *args], stderr=subprocess.DEVNULL, text=True).strip()
        git("init", "-b", "main")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "initial")
        registration = base / "service.json"
        registration.write_text(json.dumps({"url": f"http://127.0.0.1:{health.server_port}", "password": "fixture"}))
        registration.chmod(0o600)
        env = {**os.environ, "FACTORY_TOKEN": uuid.uuid4().hex * 2, "TERM": "xterm-256color"}
        env.pop("FACTORY_TOKEN_FILE", None)
        if options.native:
            import shutil
            opencode = shutil.which("opencode")
            assert opencode, "OpenCode binary required"
            for key, child in [("HOME", "home"), ("XDG_STATE_HOME", "state"), ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache")]:
                env[key] = str(base / child)
                (base / child).mkdir()
            for key in list(env):
                if key.startswith("OPENCODE_"):
                    env.pop(key)
            env["OPENCODE_DB"] = str(base / "opencode.db")
            import socket
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]
            subprocess.run([opencode, "service", "set", "port", str(port)], env=env, cwd=base, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=20, check=True)
        elif options.auto:
            bindir = base / "bin"
            bindir.mkdir()
            native = base / "state" / "opencode" / "service.json"
            executable = bindir / "opencode"
            executable.write_text("#!/usr/bin/env python3\nimport sys, pathlib, shutil\nassert sys.argv[1:] == ['service', 'start']\np = pathlib.Path(" + repr(str(native)) + ")\np.parent.mkdir(parents=True, exist_ok=True)\nshutil.copyfile(" + repr(str(registration)) + ", p)\nwith open(" + repr(str(base / "starts")) + ", 'a') as f: f.write('start\\n')\n")
            executable.chmod(0o700)
            env["PATH"] = str(bindir) + os.pathsep + env["PATH"]
            env["XDG_STATE_HOME"] = str(base / "state")
        server = subprocess.Popen([str(ROOT / "target/debug/factoryd"), "--listen", "127.0.0.1:0", "--database", str(base / "db")], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        worker = None
        pid = fd = None
        try:
            assert select.select([server.stdout], [], [], 10)[0]
            url = "http://" + server.stdout.readline().strip().split()[-1]
            def snapshot():
                req = urllib.request.Request(url + "/v1/snapshot", headers={"Authorization": "Bearer " + env["FACTORY_TOKEN"], "x-factory-protocol": "1"})
                return json.load(urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=5))
            runner = str(uuid.uuid4())
            worker = subprocess.Popen([str(ROOT / "target/debug/factory-runner"), "--server", url, "--id", runner, "--name", "Setup host", "--journal", str(base / "journal"), "--workspace-root", str(base / "work")], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.monotonic() + 15
            while not snapshot()["runners"]:
                assert time.monotonic() < deadline
                time.sleep(.1)
            pid, fd = pty.fork()
            if pid == 0:
                os.execve(str(ROOT / "target/debug/factory"), ["factory", "--server", url], env)
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 150, 0, 0))
            def pump(seconds=.3):
                deadline = time.monotonic() + seconds
                data = b""
                while time.monotonic() < deadline:
                    if select.select([fd], [], [], .02)[0]:
                        data += os.read(fd, 65536)
                return data
            def send(value):
                os.write(fd, value.encode())
                pump()
            pump(1.5)
            send("2n")
            if options.new_folder:
                send("\t" + str(base / "requested/new-project"))
            config = {"$schema": "https://opencode.ai/config.json", "mcp": {"servers": {}}, "instructions": ["AGENTS.md"]}
            if not options.empty:
                send(" Wizard Project\t" + str(repo) + "\t\t \t" + str(repo) + "\t" + ("" if options.auto else str(registration)) + "\topenai\ttest\t")
                send("\x1b[200~" + json.dumps(config) + "\x1b[201~")
                send("\t\t\t\tmcp-fixture\t\x1b[B\tmcp-program\t")
                send("\x1b[200~argument with spaces\n--flag\x1b[201~\tRULES.md\x01")
                config["mcp"]["servers"]["mcp-fixture"] = {"type": "local", "command": ["mcp-program", "argument with spaces", "--flag"]}
                config["instructions"].append("RULES.md")
            send("\x13")
            deadline = time.monotonic() + 40
            while True:
                pump(.1)
                state = snapshot()
                if state["commands"] and state["commands"][0]["state"] in ("succeeded", "failed"):
                    break
                assert time.monotonic() < deadline, "Project setup did not finish"
            command = state["commands"][0]
            assert command["state"] == "succeeded", command.get("result")
            work = Path(command["result"]["workspace"]["path"])
            if options.empty:
                assert not (work / "opencode.json").exists()
                assert state["projects"][0]["repository"] == (str(base / "requested/new-project") if options.new_folder else "factory:managed")
                if options.new_folder:
                    assert (base / "requested/new-project/.git").is_dir()
                assert state["projects"][0]["profile"]["model_id"] == ""
                assert state["projects"][0]["setup"]["repository_path"] == ""
            else:
                assert json.loads((work / "opencode.json").read_text()) == config
            assert not (repo / "opencode.json").exists()
            assert len(state["projects"]) == len(state["streams"]) == 1
            assert state["projects"][0]["setup"]["runner_id"] == runner
            if options.auto and not options.native:
                assert (base / "starts").read_text() == "start\n"
            print("PASS zero-input project creates managed Git/worktree and verifies discovered OpenCode" if options.empty else "PASS TUI project and structured MCP editor preserve config; real runner prepares workspace and verifies OpenCode")
            if options.tasks:
                deadline = time.monotonic() + 20
                while not snapshot()["scheduler_policies"]:
                    pump(.2)
                    assert time.monotonic() < deadline, "Automatic policy missing"
                send("\x1b")
                send("2l")
                send("\x7f2\x13")
                assert snapshot()["projects"][0]["max_sessions"] == 2
                assert snapshot()["scheduler_policies"][0]["max_parallel"] == 2
                send("\x1b")
                send("2 ")
                send("\x1b[B")
                send("\x13")
                assert snapshot()["projects"][0]["paused"]
                send("\x1b")
                image = base / "example.png"
                import base64
                image.write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a3e0AAAAASUVORK5CYII="))
                send("3n")
                send("Build from this image")
                send("\x1b[200~" + str(image) + "\x1b[201~")
                send("\r")
                state = snapshot()
                assert len(state["tickets"]) == 1
                task = state["tickets"][0]
                assert task["prompt"] == "Build from this image"
                assert base64.b64decode(task["images"][0]["data"]) == image.read_bytes()
                pump(6)
                assert len(snapshot()["commands"]) == 1, "Paused task was started"
                worker.terminate()
                worker.wait(timeout=10)
                send("2 ")
                send("\x1b[A")
                send("\x13")
                assert not snapshot()["projects"][0]["paused"]
                deadline = time.monotonic() + 10
                while len(snapshot()["commands"]) == 1:
                    pump(.2)
                    assert time.monotonic() < deadline, "Prompt was not scheduled after resume"
                state = snapshot()
                assert any(s["goal"] == "Orchestrator" for s in state["streams"])
                assert not any(task["id"] in s["ticket_ids"] for s in state["streams"]), "Task bypassed the orchestrator"
                print("PASS TUI session limit, prompt and image persist; pause holds work and resume prepares the orchestrator")
        finally:
            if pid:
                os.kill(pid, signal.SIGTERM)
                os.waitpid(pid, 0)
            if fd is not None:
                os.close(fd)
            for process in [worker, server]:
                if process and process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
            if options.native and (base / "state/opencode/service.json").exists():
                subprocess.run([opencode, "service", "stop"], env=env, cwd=base, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20, check=True)
            health.shutdown()


if __name__ == "__main__":
    main()

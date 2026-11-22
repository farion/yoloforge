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
    options = parser.parse_args()
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
        if options.auto:
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
                assert state["projects"][0]["repository"] == "factory:managed"
                assert state["projects"][0]["profile"]["model_id"] == ""
                assert state["projects"][0]["setup"]["repository_path"] == ""
            else:
                assert json.loads((work / "opencode.json").read_text()) == config
            assert not (repo / "opencode.json").exists()
            assert len(state["projects"]) == len(state["streams"]) == 1
            assert state["projects"][0]["setup"]["runner_id"] == runner
            if options.auto:
                assert (base / "starts").read_text() == "start\n"
            print("PASS zero-input project creates managed Git/worktree and verifies discovered OpenCode" if options.empty else "PASS TUI project and structured MCP editor preserve config; real runner prepares workspace and verifies OpenCode")
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
            health.shutdown()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Real processes: two runners, two CLI clients, TUI PTY, and daemon restart."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import subprocess
import struct
import termios
import tempfile
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def wait_for(predicate, timeout=25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = predicate()
            if result:
                return result
        except (OSError, ValueError):
            pass
        time.sleep(0.2)
    raise RuntimeError("condition did not become true within deadline")


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    if process.stdout:
        process.stdout.close()


def tui_check(url, env):
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(str(ROOT / "target/debug/factory"), ["factory", "--server", url], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    output = b""
    exited = False
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if select.select([fd], [], [], 0.2)[0]:
                output += os.read(fd, 65536)
            if b"smoke-local" in output and b"smoke-server" in output:
                break
        check(b"smoke-local" in output and b"smoke-server" in output, "TUI did not render both live runners")
        for key, expected in [(b"2", b"smoke-project"), (b"3", b"smoke-ticket"), (b"4", b"smoke-stream"), (b"5", b"CheckOpenCode"), (b"6", b"Fragen")]:
            output = b""
            for attempt in range(3):
                # Resize may coincide with and swallow a key event in a PTY.
                # Repeating a view-selection key is read-only and idempotent.
                os.write(fd, key)
                time.sleep(0.1)
                # Full frames avoid matching only the changed suffix of a name.
                fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120 + int(key) + attempt, 0, 0))
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline and expected not in output:
                    if select.select([fd], [], [], 0.2)[0]:
                        output += os.read(fd, 65536)
                if expected in output:
                    break
            check(expected in output, "TUI did not render view " + key.decode() + ": " + expected.decode())
        os.write(fd, b"q")
        def finished():
            nonlocal exited
            # Drain terminal restoration/full-frame output so the child cannot
            # block on the PTY buffer while exiting.
            while select.select([fd], [], [], 0)[0]:
                try:
                    if not os.read(fd, 65536):
                        break
                except OSError:
                    break
            found, status = os.waitpid(pid, os.WNOHANG)
            if found:
                exited = True
                check(os.waitstatus_to_exitcode(status) == 0, "TUI exited with error")
            else:
                # A resize can coincide with the first key event; send q again
                # after draining the terminal instead of racing the redraw.
                os.write(fd, b"q")
            return found
        wait_for(finished, timeout=5)
        print("PASS Ratatui renders runners, projects, tickets, streams, commands, question view and exits on q")
    finally:
        if not exited:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--opencode-registration", type=Path)
    args = parser.parse_args()
    env = {**os.environ, "FACTORY_TOKEN": uuid.uuid4().hex + uuid.uuid4().hex, "TERM": "xterm-256color"}
    processes = []
    Path("/tmp/opencode").mkdir(exist_ok=True)
    temporary = tempfile.TemporaryDirectory(prefix="factory-smoke-", dir="/tmp/opencode")
    database = str(Path(temporary.name) / "factory.db")
    def spawn(binary, *arguments):
        process = subprocess.Popen([str(ROOT / "target/debug" / binary), *arguments], env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, cwd=ROOT)
        processes.append(process)
        return process
    def daemon(address):
        process = spawn("factoryd", "--listen", address, "--database", database)
        check(select.select([process.stdout], [], [], 10)[0], "daemon did not announce readiness")
        line = process.stdout.readline()
        check("listening on" in line, "daemon startup failed")
        return process, line.strip().split()[-1]
    try:
        server, address = daemon("127.0.0.1:0")
        url = "http://" + address
        def snapshot():
            request = urllib.request.Request(url + "/v1/snapshot", headers={
                "Authorization": "Bearer " + env["FACTORY_TOKEN"], "x-factory-protocol": "1"})
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with opener.open(request, timeout=2) as response:
                return json.load(response)
        extra = ["--opencode-registration", str(args.opencode_registration.resolve())] if args.opencode_registration else []
        local_id = str(uuid.uuid4())
        mapping = Path(temporary.name) / "repositories.json"
        mapping.write_text("{}")
        local_args = ("--server", url, "--id", local_id, "--name", "smoke-local", "--journal", str(Path(temporary.name) / "local.db"), "--repositories", str(mapping), *extra)
        local = spawn("factory-runner", *local_args)
        remote = spawn("factory-runner", "--server", url, "--id", str(uuid.uuid4()), "--name", "smoke-server", "--location", "server", "--journal", str(Path(temporary.name) / "remote.db"))
        wait_for(lambda: len(snapshot()["runners"]) == 2)
        before = snapshot()
        def cli(*args):
            result = subprocess.run([str(ROOT / "target/debug/factory"), "--server", url, *args],
                                    env=env, capture_output=True, text=True, timeout=10)
            check(result.returncode == 0, "CLI mutation failed")
            return json.loads(result.stdout)
        project = cli("project", "create", "--name", "smoke-project", "--repository", "git@example.org:factory.git")
        request_id = str(uuid.uuid4())
        create = ("ticket", "create", "--project", project["id"], "--title", "smoke-ticket", "--request-id", request_id)
        ticket = cli(*create)
        check(cli(*create) == ticket, "CLI retry duplicated ticket")
        changed = cli("ticket", "update", ticket["id"], "--expected-revision", "1", "--status", "ready")
        check(changed["revision"] == 2 and changed["status"] == "ready", "CLI update failed")
        cursor = snapshot()["event_cursor"]
        check(cursor == 3, "mutations/event count differ")
        print("PASS CLI project/ticket creation, request deduplication and revision update")
        stream = cli("stream", "create", "--project", project["id"], "--goal", "smoke-stream", "--runner", local_id, "--ticket", ticket["id"])
        command = cli("stream", "check", stream["id"], "--expected-revision", "1")
        wait_for(lambda: snapshot()["commands"][0]["state"] in ("succeeded", "failed"))
        completed = snapshot()
        expected_state = "succeeded" if args.opencode_registration else "failed"
        check(completed["commands"][0]["state"] == expected_state, "unexpected readiness result")
        cursor = completed["event_cursor"]
        print("PASS stream command delivered, acknowledged, executed and durably reported")
        if args.opencode_registration:
            check(next(r for r in before["runners"] if r["name"] == "smoke-local")["opencode"]["status"] == "ready", "OpenCode adapter did not report ready")
            print("PASS actual OpenCode registration read and version checked")
        clients = [subprocess.Popen([str(ROOT / "target/debug/factory"), "--server", url, "--snapshot"],
                                   env=env, stdout=subprocess.PIPE, text=True) for _ in range(2)]
        for client in clients:
            output, _ = client.communicate(timeout=10)
            check(client.returncode == 0 and len(json.loads(output)["runners"]) == 2, "CLI snapshot failed")
        print("PASS two independent clients see both runners")
        tui_check(url, env)
        check(local.poll() is None and remote.poll() is None, "closing TUI stopped a runner")
        stop(server)
        _, same_address = daemon(address)
        check(same_address == address, "restart changed bind address")
        wait_for(lambda: len(snapshot()["runners"]) == 2)
        after = snapshot()
        check(after["boot_id"] != before["boot_id"], "server boot identity did not change")
        check({r["id"] for r in after["runners"]} == {r["id"] for r in before["runners"]}, "runner identity changed")
        check(after["projects"] == [project] and after["tickets"] == [changed] and after["event_cursor"] == cursor,
              "project/ticket state did not survive restart")
        check(cli(*create) == ticket and snapshot()["event_cursor"] == cursor, "post-restart request replay failed")
        check(after["streams"] == completed["streams"] and after["commands"] == completed["commands"], "work state lost on restart")
        # Queue work while its owner is stopped, then restart both processes.
        stop(local)
        cli("stream", "check", stream["id"], "--expected-revision", str(after["streams"][0]["revision"]))
        for process in processes:
            if process.poll() is None and process.args[0].endswith("/factoryd"):
                stop(process)
        daemon(address)
        local = spawn("factory-runner", *local_args)
        wait_for(lambda: len(snapshot()["commands"]) == 2 and snapshot()["commands"][1]["state"] == expected_state)
        check(snapshot()["commands"][0] == completed["commands"][0], "completed work changed during restart")
        print("PASS queued command survives daemon and runner restart with the same journal")
        print("PASS SQLite project/ticket state, event cursor and request journal survive restart")
        print("PASS daemon restart: both runners re-register with stable IDs")
        repository = Path(temporary.name) / "source"
        repository.mkdir()
        def git(*arguments):
            result = subprocess.run(["git", "-C", str(repository), *arguments], capture_output=True, text=True, timeout=10)
            check(result.returncode == 0, "test Git setup failed")
        git("init", "-b", "main")
        git("config", "user.name", "Factory Smoke")
        git("config", "user.email", "factory@example.invalid")
        (repository / "file.txt").write_text("initial\n")
        git("add", "file.txt")
        git("commit", "-m", "initial")
        (repository / "file.txt").write_text("user changes\n")
        cli("repository", "register", "--project", project["id"], "--directory", str(repository), "--repositories", str(mapping))
        second_stream = cli("stream", "create", "--project", project["id"], "--goal", "second workspace", "--runner", local_id)
        def prepare(stream_id):
            revision = next(s["revision"] for s in snapshot()["streams"] if s["id"] == stream_id)
            command = cli("stream", "prepare", stream_id, "--expected-revision", str(revision))
            def finished_workspace():
                current = next(c for c in snapshot()["commands"] if c["id"] == command["id"])
                if current["state"] == "failed":
                    raise RuntimeError("workspace preparation failed: " + str(current["result"]))
                return current if current["state"] == "succeeded" else None
            return wait_for(finished_workspace)["result"]["workspace"]
        first_workspace = prepare(stream["id"])
        second_workspace = prepare(second_stream["id"])
        first_path = Path(first_workspace["path"])
        second_path = Path(second_workspace["path"])
        check(first_path != second_path, "streams share a worktree")
        (first_path / "file.txt").write_text("stream A\n")
        (second_path / "file.txt").write_text("stream B\n")
        stop(local)
        for process in processes:
            if process.poll() is None and process.args[0].endswith("/factoryd"):
                stop(process)
        daemon(address)
        local = spawn("factory-runner", *local_args)
        check(prepare(stream["id"]) == first_workspace, "workspace identity changed after restart")
        check((first_path / "file.txt").read_text() == "stream A\n", "replay overwrote stream A")
        check((second_path / "file.txt").read_text() == "stream B\n", "stream B was modified")
        check((repository / "file.txt").read_text() == "user changes\n", "source checkout was modified")
        print("PASS two isolated Git worktrees; preparation after restart preserves dirty files")
    finally:
        for process in reversed(processes):
            stop(process)
        temporary.cleanup()


if __name__ == "__main__":
    main()

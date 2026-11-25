#!/usr/bin/env python3
"""Real Ratatui management and answer dialogs through PTY and Factory HTTP/SQLite.

Synthetic pending questions; no OpenCode service or model calls needed.
"""
import fcntl
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import urllib.request
import urllib.error
import uuid


def screen_text(data):
    """Decode cursor-addressed full frames (Ratatui omits unchanged spaces)."""
    rows = [[" "] * 200 for _ in range(60)]
    row = column = 0
    for part in re.split(r"(\x1b\[[0-?]*[ -/]*[@-~])", data.decode("utf-8", errors="replace")):
        if part.startswith("\x1b["):
            if part[-1] in "Hf":
                coords = part[2:-1].split(";")
                row = int(coords[0] or "1") - 1
                column = int(coords[1] or "1") - 1 if len(coords) > 1 else 0
            elif part == "\x1b[2J":
                rows = [[" "] * 200 for _ in range(60)]
            continue
        for char in part:
            if char == "\n":
                row += 1
            elif char == "\r":
                column = 0
            elif char.isprintable():
                if 0 <= row < len(rows) and 0 <= column < len(rows[row]):
                    rows[row][column] = char
                column += 1
    return "\n".join("".join(row) for row in rows)

ROOT = Path(__file__).resolve().parents[1]


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    env = {**os.environ, "FACTORY_TOKEN": uuid.uuid4().hex * 2, "TERM": "xterm-256color"}
    Path("/tmp/opencode").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="factory-tui-answer-", dir="/tmp/opencode") as directory:
        server = subprocess.Popen([str(profile_dir() / "factoryd"), "--listen", "127.0.0.1:0", "--database", directory + "/db"],
                                  env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        pid = None
        fd = None
        exited = False
        try:
            check(select.select([server.stdout], [], [], 10)[0], "daemon did not start")
            line = server.stdout.readline()
            check("listening on" in line, "daemon startup failed")
            url = "http://" + line.strip().split()[-1]
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

            def api(method, path, body=None, registration=None):
                if registration:
                    # The synthetic runner has no heartbeat loop; keep its lease
                    # alive explicitly as this PTY acceptance grows beyond 60s.
                    api("POST", "/v1/runners/" + runner + "/heartbeat", {"registration_id": registration, "opencode": {"status": "not_configured"}})
                headers = {"Authorization": "Bearer " + env["FACTORY_TOKEN"], "x-factory-protocol": "1", "Content-Type": "application/json"}
                if registration:
                    headers["x-factory-runner-registration"] = registration
                request = urllib.request.Request(url + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
                with opener.open(request, timeout=5) as response:
                    return json.load(response)

            def rid():
                return str(uuid.uuid4())

            project = api("POST", "/v1/projects", {"request_id": rid(), "name": "dialog-project", "repository": "/unused", "base_branch": "main"})
            api("PATCH", "/v1/projects/" + project["id"] + "/profile", {"request_id": rid(), "expected_revision": 1,
                "profile": {"toolchain": "", "start_command": "", "test_command": "", "provider_id": "test", "model_id": "test"}})
            runner = rid()
            registration = api("POST", "/v1/runners/register", {"id": runner, "instance_id": rid(), "name": "dialog-fixture", "location": "local", "version": "test", "capabilities": ["container-freeze-v1"], "opencode": {"status": "not_configured"}})["registration_id"]
            stream = api("POST", "/v1/streams", {"request_id": rid(), "project_id": project["id"], "goal": "dialog test", "runner_id": runner, "ticket_ids": []})
            prefix = "/v1/runners/" + runner + "/commands/"
            prep = api("POST", "/v1/streams/" + stream["id"] + "/commands", {"request_id": rid(), "expected_revision": 1, "kind": "prepare_workspace"})
            api("POST", prefix + prep["id"] + "/ack", {"request_id": rid(), "ownership_epoch": 1}, registration)
            api("POST", prefix + prep["id"] + "/result", {"request_id": rid(), "ownership_epoch": 1, "result": {"workspace": {"path": "/unused", "branch": stream["branch"], "base_commit": "a" * 40}}}, registration)
            command = api("POST", "/v1/streams/" + stream["id"] + "/commands", {"request_id": rid(), "expected_revision": 4, "kind": "run_agent", "prompt": "Synthetic questions; never dispatched to OpenCode"})
            api("POST", prefix + command["id"] + "/ack", {"request_id": rid(), "ownership_epoch": 1}, registration)
            session = "ses_factory_" + command["id"].replace("-", "")
            external = [
                {"external_id": "form_one", "kind": "form", "payload": {"id": "form_one", "sessionID": session, "title": "Name eingeben", "fields": [{"key": "name", "type": "string", "required": True}]}},
                {"external_id": "per_one", "kind": "permission", "payload": {"id": "per_one", "sessionID": session, "action": "shell", "resources": ["printf dialog-test"]}},
                {"external_id": "form_conflict", "kind": "form", "payload": {"id": "form_conflict", "sessionID": session, "title": "Konfliktfrage", "fields": [{"key": "text", "type": "string"}]}},
            ]
            questions = api("POST", prefix + command["id"] + "/questions", {"ownership_epoch": 1, "questions": external}, registration)
            api("PUT", prefix + command["id"] + "/excerpt", {"ownership_epoch": 1, "excerpt": {
                "session_id": session, "revision": 1, "captured_unix_ms": 123, "older_omitted": True,
                "messages": [{"id": "msg_excerpt_user", "kind": "user", "text": "Archived user request", "truncated": False},
                              {"id": "msg_excerpt_assistant", "kind": "assistant", "text": "[Tool: shell | completed]\nGrüße from archived tool output", "truncated": True}]}}, registration)
            api("PUT", prefix + command["id"] + "/archive", {"ownership_epoch": 1, "batch": {
                "session_id": session, "revision": 1, "captured_unix_ms": 123, "final_scan_complete": False,
                "messages": [{"id": "msg_raw_first", "text": "RAW_START " + "Grüße archive " * 1500 + " RAW_END"},
                             {"id": "msg_raw_second", "text": "Second raw archive message"}]}}, registration)
            pid, fd = pty.fork()
            if pid == 0:
                os.execve(str(profile_dir() / "factory"), ["factory", "--server", url], env)
            width = 125

            def resize():
                nonlocal width
                width = 126 if width == 125 else 125
                fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 34, width, 0, 0))

            def drain():
                data = b""
                while select.select([fd], [], [], 0)[0]:
                    try:
                        chunk = os.read(fd, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    data += chunk
                return data

            def pump(seconds=0.3):
                deadline = time.monotonic() + seconds
                data = b""
                while time.monotonic() < deadline:
                    data += drain()
                    time.sleep(0.02)
                return data

            def send(keys):
                os.write(fd, keys)
                pump()

            def visible(text):
                drain()
                resize()
                data = pump(0.5)
                check(text in screen_text(data), "TUI did not render " + text)

            def state(index):
                return next(q for q in api("GET", "/v1/snapshot")["questions"] if q["id"] == questions[index]["id"])

            def queued(index):
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline:
                    pump(0.1)
                    question = state(index)
                    if question["state"] == "answer_queued":
                        return question
                raise RuntimeError("TUI answer was not queued")

            resize()
            pump(1.5)
            send(b"2N")  # legacy metadata-only form; new-project wizard has separate acceptance
            visible("Projekt anlegen")
            send("Grüße Projekt".encode())
            send(b"\t/unused-management\t\x13")
            pump(1.3)
            created = next(p for p in api("GET", "/v1/snapshot")["projects"] if p["name"] == "Grüße Projekt")
            send(b"\x1b")
            send(b"\x1b[Be")
            visible("Projekt bearbeiten")
            send(b" edited\t-v2\t-tui\x13")
            pump(1.3)
            metadata = next(p for p in api("GET", "/v1/snapshot")["projects"] if p["id"] == created["id"])
            check(metadata["name"] == "Grüße Projekt edited" and metadata["repository"] == "/unused-management-v2" and metadata["base_branch"] == "main-tui", "project metadata edit failed")
            send(b"\x1b")
            send(b"\r")
            visible("Projektprofil")
            send(b"test-provider\ttest-model\tpython3\tpython3 app.py\tpython3 -m unittest\x13")
            pump(1.3)
            configured = next(p for p in api("GET", "/v1/snapshot")["projects"] if p["id"] == created["id"])
            check(configured["profile"]["model_id"] == "test-model", "TUI profile not saved")
            send(b"\x1b")
            send(b"3\x1b[Cn")  # Select the second project, then create its ticket.
            visible("Prompt")
            send("\x1b[200~TUI ticket\nBeschreibung\nzweite Zeile\x1b[201~".encode())
            send(b"\r")
            pump(1.3)
            ticket = next(t for t in api("GET", "/v1/snapshot")["tickets"] if t["project_id"] == created["id"])
            check(ticket["prompt"] == "TUI ticket\nBeschreibung\nzweite Zeile", "multiline paste changed")
            send(b"\r")
            send(b" edited")
            api("PATCH", "/v1/tickets/" + ticket["id"], {"request_id": rid(), "expected_revision": ticket["revision"], "description": "other client update"})
            send(b"\x13")
            visible("Entwurf erhalten")
            pump(1.3)
            send(b"\x12")  # Explicitly rebase local changes onto the latest revision.
            visible("zweite Zeile edited")
            send(b"\x13")
            pump(1.3)
            updated = next(t for t in api("GET", "/v1/snapshot")["tickets"] if t["id"] == ticket["id"])
            check(updated["prompt"] == "TUI ticket\nBeschreibung\nzweite Zeile edited", "prompt conflict lost the local edit")
            send(b"4n")
            visible("Stream anlegen")
            send(b"TUI goal\t \t \x13")  # Explicit runner and ticket selection.
            pump(1.3)
            created_stream = next(s for s in api("GET", "/v1/snapshot")["streams"] if s["project_id"] == created["id"])
            check(created_stream["runner_id"] == runner and created_stream["ticket_ids"] == [ticket["id"]], "stream selection incorrect")
            print("PASS TUI project/profile, multiline ticket, conflict rebase and stream assignment")
            send(b"\x1b")
            send(b"\x1b[D")  # Return to the original project for question tests.
            send(b"5\x1b[B\r")
            visible("Auftragsdetails")
            send(b"\t")
            visible("Synthetic questions; never dispatched to OpenCode")
            send(b"\t")
            visible("Beim Einreihen gespeichertes Profil")
            send(b"\t")
            pump(1.3)
            visible("question.created")
            send(b"\t")
            visible("Sessionauszug")
            visible("GEKUERZT")
            visible("Grüße from archived tool output")
            send(b"p")
            visible("Archived user request")
            send(b"n")
            visible("Grüße from archived tool output")
            send(b"\t")
            pump(1.0)
            visible("Vollstaendiges Nachrichtenarchiv")
            visible("RAW_START")
            send(b"]\x1b[F")
            visible("RAW_END")
            send(b"n")
            visible("Second raw archive message")
            api("PUT", prefix + command["id"] + "/archive", {"ownership_epoch": 1, "batch": {
                "session_id": session, "revision": 2, "captured_unix_ms": 124, "final_scan_complete": False,
                "messages": [{"id": "msg_raw_new", "text": "Newer archive revision"}]}}, registration)
            send(b"n")
            visible("Archivende dieses Revisionsstands")
            visible("Second raw archive message")
            send(b"p")
            visible("RAW_START")
            send(b"r")
            pump(0.6)
            send(b"n")
            send(b"n")
            visible("Newer archive revision")
            print("PASS TUI raw archive navigation, untruncated Unicode parts and pinned revision refresh")
            send(b"\x1b")
            print("PASS read-only command prompt, captured profile and durable question history")
            send(b"6\r")
            visible("Frage beantworten")
            send("\x1b[200~Grüße \x1b[201~".encode())
            send(b"q")  # q is ordinary input while a form is open.
            send(b"\x13")  # Ctrl-S
            send(b"\x1b")
            first = queued(0)
            check(first["answer"] == {"name": "Grüße q"}, "Unicode/paste answer changed")
            print("PASS TUI Unicode form input and asynchronous answer submission")
            send(b"\x1b[B\r")
            send(b"\x13")
            check(state(1)["state"] == "pending", "permission submitted without a decision")
            send(b"1\x13")
            check(queued(1)["answer"] == {"decision": "once"}, "permission decision incorrect")
            send(b"\x1b")
            print("PASS TUI permission requires explicit selection")
            send(b"\x1b[B\r")
            send(b"retained draft")
            api("POST", "/v1/questions/" + questions[2]["id"] + "/answer", {"request_id": rid(), "expected_revision": 1, "answer": {"text": "other client"}})
            pump(1.3)
            send(b"\x13")
            visible("Entwurf bleibt erhalten")
            check(state(2)["answer"] == {"text": "other client"}, "TUI overwrote competing answer")
            send(b"\x1b")
            send(b"\r")
            visible("retained draft")
            send(b"\x1b")
            send(b"4c")
            visible("Stream steuern")
            send(b"\t\t\x13")
            pump(1.3)
            snapshot = api("GET", "/v1/snapshot")
            control = snapshot["controls"][0]
            check(control["state"] == "requested", "TUI pause not persisted")
            check(snapshot["streams"][0]["status"] == "pausing", "pause status missing")
            api("POST", prefix + command["id"] + "/control", {"control_request": control["request_id"], "ownership_epoch": 1,
                "state": "held_before_execution", "summary": "Synthetic runner confirms no prompt submitted"}, registration)
            pump(1.3)
            send(b"\x1b")
            send(b"c")
            send(b"\t\t\t\x13")
            pump(1.3)
            check(api("GET", "/v1/snapshot")["controls"][0]["state"] == "released", "TUI scheduling resume failed")
            print("PASS TUI durable pause and explicit pre-execution resume")
            send(b"\x1b")
            def stream_revision():
                return next(s["revision"] for s in api("GET", "/v1/snapshot")["streams"] if s["id"] == stream["id"])
            def stream_control():
                return next(c for c in api("GET", "/v1/snapshot")["controls"] if c["stream_id"] == stream["id"])
            route = "/v1/streams/" + stream["id"] + "/control"
            pause = api("POST", route, {"request_id": rid(), "expected_revision": stream_revision(), "action": "pause"})
            proof = {"container_id": "a" * 64, "started_at": "2026-11-21T10:00:00Z", "frozen": True, "observed_unix_ms": 1000}
            api("POST", prefix + command["id"] + "/control", {"control_request": pause["request_id"], "ownership_epoch": 1, "state": "frozen", "summary": "Fixture frozen", "proof": proof}, registration)
            resume = api("POST", route, {"request_id": rid(), "expected_revision": stream_revision(), "action": "resume"})
            def unknown(request):
                return api("POST", prefix + command["id"] + "/control", {"control_request": request, "ownership_epoch": 1, "state": "interrupt_unverified", "summary": "Fixture thaw uncertain"}, registration)
            unknown(resume["request_id"])
            pump(1.3)
            send(b"r")
            visible("Container-Recovery")
            visible(resume["request_id"])
            send(b"\x13")
            visible("Begruendung erforderlich")
            reason = "Laufzeit geprüft – neuer Versuch"
            send(("\x1b[200~" + reason + "\x1b[201~").encode())
            competing = api("POST", route + "/retry", {"request_id": rid(), "expected_revision": stream_revision(), "previous_request": resume["request_id"], "expected_proof": proof, "reason": "Other operator"})
            unknown(competing["request_id"])
            pump(1.3)
            visible("Stand geaendert")
            visible(resume["request_id"])
            send(b"\x13")
            check(stream_control()["request_id"] == competing["request_id"], "TUI silently retargeted recovery")
            send(b"\x12")
            visible(competing["request_id"])
            visible(reason)
            send(b"\x13")
            pump(1.3)
            visible("Neuer Versuch gespeichert")
            recovered = stream_control()
            check(recovered["state"] == "resume_requested" and recovered["retry"] == {"previous_request": competing["request_id"], "reason": reason}, "TUI recovery provenance differs")
            send(b"\x13")
            check(stream_control()["request_id"] == recovered["request_id"], "TUI duplicated confirmed recovery")
            unknown(recovered["request_id"])
            pump(1.3)
            send(b"\x12")
            send(b"\x1b")
            print("PASS TUI recovery pins evidence, preserves Unicode reason and requires explicit rebase after competing request")
            send(b"7")
            pump(1.3)
            visible("Geraete / Admin")
            send(b"n\x13")
            pump(1.0)
            def enrollment_code():
                drain()
                resize()
                rendered = screen_text(pump(0.5))
                found = re.search(r"yf_[0-9a-f]{64}", rendered)
                check(found is not None, "enrollment code not rendered")
                return found.group(0)
            code = enrollment_code()
            credential = api("POST", "/v1/enroll", {"code": code, "name": "Grüße Tablet"})
            check(credential["device"]["role"] == "operator", "TUI default role is not operator")
            try:
                api("POST", "/v1/enroll", {"code": code, "name": "reused"})
                raise RuntimeError("enrollment code reused")
            except urllib.error.HTTPError as error:
                check(error.code == 404, "unexpected enrollment replay status")
            def credential_status(path, token, expected):
                request = urllib.request.Request(url + path, headers={"Authorization": "Bearer " + token, "x-factory-protocol": "1"})
                try:
                    opener.open(request, timeout=5).close()
                    raise RuntimeError("restricted device access accepted")
                except urllib.error.HTTPError as error:
                    check(error.code == expected, "unexpected device access status")
            credential_status("/v1/devices", credential["token"], 403)
            send(b"\x1b")
            pump(1.3)
            visible("Grüße Tablet")
            drain()
            resize()
            check(code not in screen_text(pump(0.5)), "dismissed enrollment code still displayed")
            send(b"r")
            visible(credential["device"]["id"])
            send(b"\x13")
            pump(1.0)
            check(next(d for d in api("GET", "/v1/devices") if d["id"] == credential["device"]["id"])["revoked"], "TUI revoke was not persisted")
            credential_status("/v1/snapshot", credential["token"], 401)
            send(b"n\x1b[C\x13")
            visible("Gueltige Runner-UUID erforderlich")
            runner_scope = rid()
            send(("\t\x1b[200~" + runner_scope + "\x1b[201~\x13").encode())
            pump(1.0)
            runner_credential = api("POST", "/v1/enroll", {"code": enrollment_code(), "name": "runner-device"})
            check(runner_credential["device"]["role"] == "runner" and runner_credential["device"]["runner_id"] == runner_scope, "TUI runner scope differs")
            send(b"\x1b")
            pump(1.3)
            visible("runner-device")
            send(b"\x1b")
            print("PASS TUI device roles, one-use enrollment, runner scope, hidden codes and live revocation")
            send(b"5\x1b[B\r\t\t\t")
            pump(1.3)
            visible("stream.control")
            send(b"\t\t")
            pump(0.7)
            send(b"n")
            visible("Second raw archive message")
            send(b"\x1b[Z\x1b[Z")
            server.terminate()
            server.wait(timeout=8)
            pump(1.3)
            visible("GETRENNT")
            visible("stream.control")
            send(b"\t")
            visible("Grüße from archived tool output")
            send(b"\t")
            visible("Second raw archive message")
            send(b"n")
            visible("GETRENNT")
            visible("Second raw archive message")
            send(b"\x1b")
            send(b"7")
            visible("runner-device")
            visible("veraltet")
            send(b"n")
            visible("Geraete / Admin")
            send(b"\x1b")
            send(b"6\x1b[B\x1b[B\r")
            visible("retained draft")
            print("PASS concurrent answer preserves TUI draft; dialog remains responsive offline")
            send(b"\x1b")
            send(b"4r")
            visible("Container-Recovery")
            visible("GETRENNT")
            visible(reason)
            send(b"\x13")
            visible("Request: noch nicht erzeugt")
            send(b"\x1b")
            print("PASS TUI recovery retains its draft while offline and blocks new submission")
            send(b"q")
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                pump(0.1)
                found, status = os.waitpid(pid, os.WNOHANG)
                if found:
                    exited = True
                    check(os.waitstatus_to_exitcode(status) == 0, "TUI exited with error")
                    break
            check(exited, "TUI did not exit")
        finally:
            if pid and not exited:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            if fd is not None:
                os.close(fd)
            if server.poll() is None:
                server.terminate()
                server.wait(timeout=8)
            server.stdout.close()


if __name__ == "__main__":
    main()

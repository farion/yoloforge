#!/usr/bin/env python3
"""Isolated desktop API smoke; optional native window-only screenshots. No model calls."""
import argparse
import base64
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import socket
import struct
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import zlib

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--screenshots", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="yoloforge-gui-", dir="/tmp/opencode" if Path("/tmp/opencode").exists() else None) as temporary:
        directory = Path(temporary)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        server = f"http://127.0.0.1:{port}"
        token = uuid.uuid4().hex + uuid.uuid4().hex
        env = dict(os.environ, FACTORY_TOKEN=token, XDG_DATA_HOME=str(directory / "data"), XDG_CONFIG_HOME=str(directory / "config"))
        env.pop("FACTORY_TOKEN_FILE", None)
        process = subprocess.Popen([str(profile_dir() / "factoryd"), "--listen", f"127.0.0.1:{port}", "--database", str(directory / "db")], env=env, stdout=subprocess.DEVNULL)

        def api(path, body=None, method=None, registration=None):
            headers = {"Authorization": f"Bearer {token}", "x-factory-protocol": "1", "Content-Type": "application/json"}
            if registration:
                headers["x-factory-runner-registration"] = registration
            request = urllib.request.Request(server + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method or ("GET" if body is None else "POST"))
            with urllib.request.urlopen(request, timeout=10) as response:
                return json.load(response)

        def rid():
            return str(uuid.uuid4())

        try:
            for _ in range(100):
                try:
                    api("/v1/snapshot")
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(.05)
            else:
                raise AssertionError("Factory did not become ready")
            project = api("/v1/projects", {"request_id": rid(), "name": "core-engine", "repository": "factory:managed", "base_branch": "main"})
            api("/v1/projects", {"request_id": rid(), "name": "client-app", "repository": "factory:managed", "base_branch": "main"})
            file = {"name": "notizen.txt", "mime": "text/plain", "data": base64.b64encode("Grüße — Testdatei".encode()).decode()}
            submission = {"task": {"request_id": rid(), "id": None, "expected_revision": 0, "project_id": project["id"], "requested_session": None,
                                   "prompt": "Datei-Import prüfen\nDie Eingabe vollständig prüfen und mögliche Formatfehler erklären. Keine Dateien ungeprüft verwerfen.", "images": []}, "draft": True, "files": [file]}
            draft = api("/v1/tasks/submit", submission)
            assert draft["draft"] and draft["files"] == [file]
            assert api("/v1/tasks/submit", submission) == draft
            released = json.loads(json.dumps(submission))
            released["task"].update(request_id=rid(), id=draft["id"], expected_revision=draft["revision"])
            released["draft"] = False
            result = api("/v1/tasks/submit", released)
            assert not result["draft"] and result["files"] == [file]
            # A small generated image exercises real asynchronous preview loading.
            w, h = 240, 140
            rows = []
            for y in range(h):
                row = bytearray()
                for x in range(w):
                    bar = 20 < x < 210 and y > 120 - (x // 30) * 12 and y < 120
                    row.extend((64, 139, 220) if bar else (235, 240, 247))
                rows.append(b"\0" + row)
            def chunk(kind, data):
                return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
            png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b"")
            for title, draft_mode, images in [("Metriken-Auswertung vorbereiten", True, [{"name": "metriken.png", "mime": "image/png", "data": base64.b64encode(png).decode()}]), ("Indexierung verbessern", False, [])]:
                task = {"request_id": rid(), "id": None, "expected_revision": 0, "project_id": project["id"], "requested_session": None, "prompt": title + "\nAnalysiere die Quelldateien, erläutere Änderungen und berichte die Ergebnisse mit Tests. Dieser Durchlauf verwendet nur synthetische Testdaten.", "images": images}
                api("/v1/tasks/submit", {"task": task, "draft": draft_mode, "files": []})
            assert api("/v1/me")["role"] == "admin"
            assert api("/v1/instances") == []
            assert api("/v1/notifications")["unread"] == 0
            assert api("/v1/notifications/read", {"request_id": rid(), "ids": [], "read": True}) is None
            # Synthetic runner evidence drives real HTTP question/archive/presence paths.
            # No OpenCode process or model is started by this test.
            profile = {"provider_id": "test-provider", "model_id": "test-model", "toolchain": "Rust", "start_command": "", "test_command": "cargo test"}
            api(f"/v1/projects/{project['id']}/profile", {"request_id": rid(), "expected_revision": project["revision"], "profile": profile}, method="PATCH")
            runner = rid()
            registration = api("/v1/runners/register", {"capabilities": ["session-lifecycle-v1", "task-files-v1"], "id": runner, "instance_id": rid(), "name": "Synthetischer Test-Runner", "location": "local", "version": "smoke", "opencode": {"status": "ready", "version": "test"}})["registration_id"]
            stream = api("/v1/streams", {"request_id": rid(), "project_id": project["id"], "goal": "Datei-Import prüfen", "runner_id": runner, "ticket_ids": [result["id"]]})
            prepare = api(f"/v1/streams/{stream['id']}/commands", {"request_id": rid(), "expected_revision": stream["revision"], "kind": "prepare_workspace"})
            api(f"/v1/runners/{runner}/commands/{prepare['id']}/ack", {"request_id": rid(), "ownership_epoch": 1}, registration=registration)
            api(f"/v1/runners/{runner}/commands/{prepare['id']}/result", {"request_id": rid(), "ownership_epoch": 1, "result": {"workspace": {"path": "/synthetic/workspace", "branch": stream["branch"], "base_commit": "a" * 40}}}, registration=registration)
            current_stream = next(s for s in api("/v1/snapshot")["streams"] if s["id"] == stream["id"])
            command = api(f"/v1/streams/{stream['id']}/commands", {"request_id": rid(), "expected_revision": current_stream["revision"], "kind": "run_agent", "prompt": "Datei-Import prüfen"})
            assert command["agent_spec"]["files"] == [file], "File must reach immutable agent payload"
            api(f"/v1/runners/{runner}/commands/{command['id']}/ack", {"request_id": rid(), "ownership_epoch": 1}, registration=registration)
            session = "ses_factory_" + command["id"].replace("-", "")
            external = [{"external_id": f"form_smoke_{i}", "kind": "form", "payload": {"id": f"form_smoke_{i}", "sessionID": session, "title": title, "fields": [{"key": "choice", "title": title, "type": "string", "required": True, "options": [{"value": "yes", "label": "Ja"}, {"value": "no", "label": "Nein"}]}]}} for i, title in enumerate(["Dateiformat beibehalten?", "Zusätzliche Tests ergänzen?"])]
            api(f"/v1/runners/{runner}/commands/{command['id']}/questions", {"ownership_epoch": 1, "questions": external}, registration=registration)
            api(f"/v1/runners/{runner}/commands/{command['id']}/archive", {"ownership_epoch": 1, "batch": {"session_id": session, "revision": 1, "captured_unix_ms": int(time.time() * 1000), "final_scan_complete": False, "messages": [{"id": "msg_smoke_user", "sessionID": session, "type": "user", "text": "Bitte die angehängte Datei prüfen."}, {"id": "msg_smoke_agent", "sessionID": session, "type": "assistant", "text": "Die Datei wurde analysiert. Zwei Entscheidungen sind offen.", "parts": [{"type": "tool", "tool": "read", "input": {"file": "notizen.txt"}, "output": "Grüße — Testdatei", "status": "completed"}]}]}}, method="PUT", registration=registration)
            api(f"/v1/runners/{runner}/instances", {"instances": [{"id": "b" * 64, "runner_id": runner, "commands": [command["id"]], "observed_unix_ms": 0}]}, method="PUT", registration=registration)
            assert len(api("/v1/instances")) == 1
            assert len(api("/v1/notifications")["entries"]) == 2
            assert len(api("/v1/snapshot")["questions"]) == 2
            assert len(api(f"/v1/commands/{command['id']}/archive")["entries"]) == 1
            assert len(api(f"/v1/commands/{command['id']}/archive/window")["entries"]) == 2
            snapshot = api("/v1/snapshot")
            assert len(snapshot["tickets"]) == 3
            compact = api("/v1/desktop/snapshot")
            assert compact["payloads_deferred"]
            assert all(not f["data"] for t in compact["tickets"] for f in t.get("files", []))
            assert api(f"/v1/tasks/{result['id']}")["files"] == [file]
            assert len(json.dumps(compact)) < len(json.dumps(snapshot))
            headers = api("/v1/desktop/events")
            assert any("Anhangbytes" in json.dumps(e) for e in headers["events"])
            assert all(file["data"] not in json.dumps(e) for e in headers["events"])
            if args.screenshots:
                args.screenshots.mkdir(parents=True, exist_ok=True)
                output = args.screenshots.resolve() / "aufgaben.png"
                assert not output.exists(), "Use a fresh screenshot directory; existing evidence is never overwritten"
                subprocess.run([str(profile_dir() / "yoloforge-gui"), "--server", server, "--screenshot", str(output), "--exit-after-ms", "6500"], env=env, check=True, timeout=25)
                assert output.read_bytes().startswith(b"\x89PNG"), "No native window screenshot"
                settings_files = list((directory / "data").rglob("desktop.json"))
                assert len(settings_files) == 1
                settings_path = settings_files[0]
                for page, filename in [("Questions", "fragen"), ("Sessions", "sessions"), ("Orchestrator", "orchestrator"), ("Notifications", "meldungen"), ("Diagnostics", "diagnose")]:
                    settings = json.loads(settings_path.read_text())
                    settings["page"] = page
                    settings_path.write_text(json.dumps(settings))
                    output = args.screenshots.resolve() / (filename + ".png")
                    assert not output.exists()
                    subprocess.run([str(profile_dir() / "yoloforge-gui"), "--server", server, "--screenshot", str(output), "--exit-after-ms", "5000"], env=env, check=True, timeout=25)
                    assert output.read_bytes().startswith(b"\x89PNG")
                assert len(api("/v1/snapshot")["tickets"]) == 3, "GUI must not submit work by observing"
                assert all(q["state"] == "pending" for q in api("/v1/snapshot")["questions"]), "GUI must not answer by observing"
            print("PASS: draft/release, exact replay, file bytes, image submission, presence, access and notifications" + (", native egui window" if args.screenshots else ""))
        finally:
            process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    main()

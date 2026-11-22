#!/usr/bin/env python3
"""Cross-platform CLI authentication and snapshot smoke against an isolated HTTP fixture."""
import argparse
import http.server
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import threading
import uuid
import urllib.parse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--stack-limit-mib",type=int,help="Unix-only child stack regression check, e.g. 1 MiB like the Windows main stack")
    args = parser.parse_args()
    if args.stack_limit_mib is not None:
        if os.name=="nt" or args.stack_limit_mib<1:parser.error("Stack limit requires Unix and at least 1 MiB")
        import resource
        resource.setrlimit(resource.RLIMIT_STACK,(args.stack_limit_mib*1024*1024,resource.getrlimit(resource.RLIMIT_STACK)[1]))
    token = secrets.token_hex(32)
    snapshot = {"protocol_version": 1, "server_version": "fixture", "boot_id": str(uuid.uuid4()),
                "revision": 0, "runners": [], "projects": [], "tickets": [], "event_cursor": 0}
    requests = []
    command_id = str(uuid.uuid4())
    archive_text = "Vollständiger Tooltext 🛠\n" * 1024

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            requests.append(self.path)
            if self.headers.get("Authorization") != "Bearer " + token:
                self.send_response(401)
                body = b'{"code":"unauthorized","message":"credential required"}'
            elif self.headers.get("x-factory-protocol") != "1":
                self.send_response(426)
                body = b'{}'
            elif self.path == "/v1/snapshot":
                self.send_response(200)
                body = json.dumps(snapshot).encode()
            elif self.path.startswith(f"/v1/commands/{command_id}/archive?"):
                query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
                after = int(query["after"][0])
                if after:
                    assert query["through_revision"] == ["1"]
                self.send_response(200)
                body = json.dumps({"status": {"session_id": "ses_fixture", "revision": 1, "captured_unix_ms": 1, "final_scan_complete": True},
                                   "next_after": 1, "entries": [] if after else [{"sequence": 1, "sample_revision": 1, "captured_unix_ms": 1,
                                                                                "message": {"id": "msg_fixture", "text": archive_text}}]}).encode()
            else:
                self.send_response(404)
                body = b'{}'
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        parent = None
        if os.name != "nt":
            parent = "/tmp/opencode"
            Path(parent).mkdir(exist_ok=True)
        try:
            with tempfile.TemporaryDirectory(prefix="yoloforge-client-", dir=parent) as directory:
                path = Path(directory) / "operator.token"
                path.write_text(token + "\n", encoding="ascii")
                env = {**os.environ, "FACTORY_TOKEN_FILE": str(path)}
                env.pop("FACTORY_TOKEN", None)
                command = [str(args.binary.resolve()), "--server", f"http://127.0.0.1:{server.server_port}", "--snapshot"]
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
                assert result.returncode == 0, result.stderr
                assert json.loads(result.stdout)["boot_id"] == snapshot["boot_id"]
                assert requests == ["/v1/snapshot"]
                path.write_text("invalid-credential-" + secrets.token_hex(32), encoding="ascii")
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
                assert result.returncode != 0
                assert token not in result.stdout + result.stderr
                assert "invalid-credential-" not in result.stdout + result.stderr
                path.write_text(token, encoding="ascii")
                output = Path(directory) / "archive.jsonl"
                export = [*command[:-1], "archive", command_id, "--output", str(output)]
                result = subprocess.run(export, env=env, capture_output=True, text=True, timeout=15)
                assert result.returncode == 0, result.stderr
                lines = [json.loads(line) for line in output.read_text(encoding="utf-8").splitlines()]
                assert lines[1]["message"]["text"] == archive_text
                assert lines[-1]["export_complete"] and lines[-1]["versions"] == 1
                previous = output.read_bytes()
                result = subprocess.run(export, env=env, capture_output=True, text=True, timeout=15)
                assert result.returncode != 0 and output.read_bytes() == previous
                print("PASS native client token-file authentication, protocol header, snapshot and rejected credentials")
                print("PASS bounded-revision archive export, complete Unicode content, footer and no-clobber output")
        finally:
            server.shutdown()
            worker.join(timeout=5)


if __name__ == "__main__":
    main()

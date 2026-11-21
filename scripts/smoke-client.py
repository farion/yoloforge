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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    token = secrets.token_hex(32)
    snapshot = {"protocol_version": 1, "server_version": "fixture", "boot_id": str(uuid.uuid4()),
                "revision": 0, "runners": [], "projects": [], "tickets": [], "event_cursor": 0}
    requests = []

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
                print("PASS native client token-file authentication, protocol header, snapshot and rejected credentials")
        finally:
            server.shutdown()
            worker.join(timeout=5)


if __name__ == "__main__":
    main()

"""Loopback-only Responses relay for the managed-access spike, not production."""

import hmac
import json
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable

CODEX = "https://chatgpt.com/backend-api/codex/responses"
MAX_REQUEST = 4 * 1024 * 1024
NO_REDIRECT = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


NO_REDIRECT.add_handler(NoRedirect())


@dataclass(frozen=True, repr=False)
class Access:
    credential_id: str
    model: str


class Relay(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, routes: dict[str, Access], grants: dict[str, set[str]],
                 credentials: Callable, upstream: str = CODEX):
        super().__init__(("127.0.0.1", 0), Handler)
        self.routes, self.grants = routes, grants
        self.credentials, self.upstream = credentials, upstream

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_port}"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass  # No credentials, request bodies, or upstream errors in logs.

    def fail(self, status, message):
        body = json.dumps({"error": {"message": message}}).encode()
        self.close_connection = True
        self.send_response(status)
        self.send_header("Connection", "close")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        route = self.path.split("/")
        if len(route) != 4 or route[1] != "v1" or route[3] != "responses":
            return self.fail(404, "Unknown Factory route")
        name = route[2]
        authorization = self.headers.get("Authorization", "").encode()
        if not any(hmac.compare_digest(authorization, f"Bearer {key}".encode()) and name in allowed
                   for key, allowed in self.server.grants.items()):
            return self.fail(403, "Runner is not authorized for this access")
        access = self.server.routes.get(name)
        if access is None:
            return self.fail(404, "Factory access unavailable")
        try:
            size = int(self.headers.get("Content-Length", "0"))
            if self.headers.get("Transfer-Encoding") or not 0 < size <= MAX_REQUEST:
                return self.fail(413, "Invalid request size")
            body = json.loads(self.rfile.read(size))
        except ValueError:
            return self.fail(400, "Invalid Responses request")
        if not isinstance(body, dict) or body.get("model") != access.model:
            return self.fail(403, "Model does not match the Factory route")
        # No use of an upstream endpoint, authorization, or account header
        # supplied by a runner. Do not support server-side state in this spike.
        if "previous_response_id" in body or "conversation" in body:
            return self.fail(400, "Stateful Responses requests are not supported")
        body.update(store=False, stream=True)
        body.pop("max_output_tokens", None)  # rejected by the ChatGPT endpoint
        body.setdefault("instructions", "Follow the instructions in the input.")
        try:
            entries = self.server.credentials()
            entry = next(e for e in entries if e["id"] == access.credential_id
                         and e["integrationID"] == "openai")
            value = entry["value"]
            account = value["metadata"]["accountID"]
            if (value["type"] != "oauth" or not account or not value["access"]
                    or value["expires"] <= time.time() * 1000):
                raise ValueError
            request = urllib.request.Request(self.server.upstream, json.dumps(body).encode(), {
                "Authorization": "Bearer " + value["access"],
                "ChatGPT-Account-Id": account,
                "Originator": "opencode",
                "Accept": "text/event-stream",
                "Content-Type": "application/json",
            })
        except (OSError, ValueError, KeyError, TypeError, StopIteration):
            return self.fail(503, "Factory access is not ready")
        try:
            upstream = NO_REDIRECT.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            error.close()
            return self.fail(502, f"Model provider rejected the request (HTTP {error.code})")
        except OSError:
            return self.fail(502, "Model provider unavailable")
        with upstream:
            if upstream.headers.get_content_type() != "text/event-stream":
                return self.fail(502, "Expected a Responses event stream")
            self.close_connection = True
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Connection", "close")
            self.end_headers()
            try:
                while chunk := upstream.read1(8192):
                    self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                    self.wfile.flush()
                self.wfile.write(b"0\r\n\r\n")
            except OSError:
                pass  # Either peer disconnected; never start another request.

"""Read the listening address of an isolated Factory smoke-test process."""

import os
import json
import re
import select
import sqlite3
import time


def listening_address(process, timeout=30):
    deadline = time.monotonic() + timeout
    output = bytearray()
    while time.monotonic() < deadline:
        if not select.select([process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
            break
        chunk = os.read(process.stdout.fileno(), 4096)
        if not chunk:
            break
        output.extend(chunk)
        match = re.search(rb"^factoryd \S+ listening on (\S+)\r?\n", output, re.MULTILINE)
        if match:
            return match[1].decode("ascii")
    raise RuntimeError("Factory did not announce its listening address:\n" + output.decode(errors="replace"))


def seed_android_fixture_model(database):
    """Allow manually driven commands in an isolated Android fixture, without making model calls."""
    with sqlite3.connect(database) as db:
        db.execute("UPDATE factory_model_settings SET data=? WHERE id=1", (json.dumps({
            "revision": 1,
            "default_model": {"access_id": "test", "model_id": "test"},
            "accesses": [{"id": "test", "name": "Fixture", "models": ["test"]}],
        }),))

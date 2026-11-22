#!/usr/bin/env python3
"""Test the native image/text clipboard in disposable Xvfb, never the user's desktop."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
build = subprocess.run(["cargo", "test", "-p", "factory-tui", "--bin", "yoloforge", "--locked", "--no-run", "--message-format=json"], cwd=ROOT, capture_output=True, text=True)
if build.returncode:
    raise RuntimeError(build.stderr)
artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith("{")]
binary = next(a["executable"] for a in artifacts if a.get("reason") == "compiler-artifact" and a.get("executable") and a["profile"]["test"])
subprocess.run(["docker", "run", "--rm", "--mount", f"type=bind,src={binary},dst=/test,readonly",
    "-e", "FACTORY_ISOLATED_CLIPBOARD_TEST=1", "ubuntu:24.04", "sh", "-c",
    "apt-get update -qq && apt-get install -y -qq xvfb xauth >/dev/null && xvfb-run -a /test --ignored isolated_native_clipboard_roundtrip --nocapture"], check=True, timeout=180)
print("PASS native X11 image/text clipboard roundtrip in isolated Xvfb")

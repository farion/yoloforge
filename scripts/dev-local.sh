#!/usr/bin/env bash
# Isolated local UI/runner playground; credentials and SQLite survive restarts.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
mode="${1:-help}"
case "$mode" in
  server|gui|runner|snapshot) shift ;;
  *) echo "Usage: bash scripts/dev-local.sh {server|gui|runner|snapshot} [extra arguments]"
     echo "Start server in terminal 1, gui in terminal 2, optionally runner in terminal 3."
     exit 0 ;;
esac
data="$PWD/.factory-data/local-test"
mkdir -p "$data"
chmod 700 "$data"
python3 - "$data" <<'PY'
import os, secrets, sys, tempfile, uuid
directory = sys.argv[1]
for name, value in [('access.token', secrets.token_hex(32)), ('runner-id', str(uuid.uuid4()))]:
    fd, temporary = tempfile.mkstemp(dir=directory)
    try:
        with os.fdopen(fd, 'w') as f:
            f.write(value + '\n')
            f.flush()
            os.fsync(f.fileno())
        try:
            os.link(temporary, os.path.join(directory, name))
        except FileExistsError:
            pass
    finally:
        os.unlink(temporary)
PY
unset FACTORY_TOKEN
export FACTORY_TOKEN_FILE="$data/access.token"
url="http://127.0.0.1:17432"
case "$mode" in
  server) exec cargo run --locked --bin factoryd -- --listen 127.0.0.1:17432 --database "$data/factory.sqlite3" "$@" ;;
  gui) exec cargo run --locked -p factory-gui --bin yoloforge-gui -- --server "$url" "$@" ;;
  snapshot) exec cargo run --locked --bin factory -- --server "$url" --snapshot "$@" ;;
  runner)
    key="$data/factory.sqlite3.opencode/model.key"
    if [[ ! -f "$key" ]]; then
      echo "Erst den lokalen Factory-Server starten." >&2
      exit 1
    fi
    python3 - "$data/runner-opencode.json" "$key" <<'PY'
import json, os, pathlib, sys, tempfile
settings, key = sys.argv[1:]
with tempfile.NamedTemporaryFile(mode="w", dir=pathlib.Path(settings).parent, delete=False) as f:
    json.dump({"port": 17434, "model_key_file": key}, f)
os.replace(f.name, settings)
PY
    exec cargo run --locked --bin factory-runner -- --server "$url" \
      --id "$(cat "$data/runner-id")" --name Local-Test --location local \
      --journal "$data/runner.sqlite3" --workspace-root "$data/worktrees" \
      --managed-opencode "$data/runner-opencode.json" "$@" ;;
esac

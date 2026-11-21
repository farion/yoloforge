#!/bin/sh
# Bounded foreground shell + child writer. Files are scoped to a fresh probe directory.
set -eu
cd "$(dirname "$0")"
printf '%s\n' "$$" > parent.pid
child=''
cleanup() {
    trap - EXIT INT TERM
    if [ -n "$child" ]; then
        kill "$child" 2>/dev/null || true
        wait "$child" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 143' INT TERM
sh -c '
    trap "exit 143" INT TERM
    n=0
    while [ "$n" -lt 480 ]; do
        printf "tick\n" >> heartbeat.txt
        n=$((n + 1))
        sleep 0.25
    done
' &
child=$!
printf '%s\n' "$child" > child.pid
wait "$child"

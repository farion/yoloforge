#!/usr/bin/env python3
"""Consistent online SQLite backup; restore only to a NEW database path."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile


def validate(db):
    if db.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
        raise RuntimeError("SQLite integrity check failed")
    if db.execute("PRAGMA foreign_key_check").fetchone():
        raise RuntimeError("SQLite foreign key check failed")
    version = db.execute("PRAGMA user_version").fetchone()[0]
    if version < 1 or not db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='projects'").fetchone():
        raise RuntimeError("Not a versioned Factory database")
    return version


def copy_database(source, target):
    source, target = Path(source).resolve(strict=True), Path(target).absolute()
    if target.exists() or target.is_symlink() or any(Path(str(target) + suffix).exists() for suffix in ("-wal", "-shm")):
        raise RuntimeError("Destination and sidecars must not exist; stop the service and use a new path")
    # Read-only source avoids accidental creation or migrations. SQLite's backup API includes WAL data.
    original = sqlite3.connect(source.as_uri() + "?mode=ro", uri=True, timeout=30)
    temporary = None
    try:
        fd, temporary = tempfile.mkstemp(prefix=".factory-backup-", dir=target.parent)
        os.close(fd)
        destination = sqlite3.connect(temporary)
        try:
            original.backup(destination, pages=256, sleep=0.05)
            version = validate(destination)
            destination.commit()
        finally:
            destination.close()
        with open(temporary, "rb") as file:
            os.fsync(file.fileno())
            digest = hashlib.file_digest(file, "sha256").hexdigest()
        # Atomic and no-clobber, including a concurrent creator of the destination.
        os.link(temporary, target)
        directory = os.open(target.parent, os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return {"path": str(target), "schema_version": version, "sha256": digest, "bytes": target.stat().st_size}
    finally:
        original.close()
        if temporary:
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["backup", "restore"])
    parser.add_argument("--source", required=True)
    parser.add_argument("--destination", required=True)
    args = parser.parse_args()
    print(json.dumps(copy_database(args.source, args.destination), indent=2))


if __name__ == "__main__":
    main()

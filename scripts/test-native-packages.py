#!/usr/bin/env python3
"""Run packaged native CLIs in isolation; never starts services or GUI windows."""
import argparse
from pathlib import Path
import subprocess
import tarfile
import tempfile
import zipfile


def check_archive(archive):
    with tempfile.TemporaryDirectory(prefix="yoloforge-native-package-") as directory:
        root = Path(directory)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as contents:
                contents.extractall(root)
        else:
            with tarfile.open(archive) as contents:
                contents.extractall(root, filter="data")
        package, = root.iterdir()
        binaries = sorted(
            path for path in package.rglob("*")
            if path.is_file() and path.name in {
                "factoryd", "factoryd.exe", "factory-runner", "factory-runner.exe",
                "opencode", "opencode.exe",
                "yoloforge-gui", "yoloforge-gui.exe",
            }
        )
        if not binaries:
            raise ValueError(f"No native binaries in {archive.name}")
        for binary in binaries:
            # Absolute executable path and a fresh cwd: no sibling build assets.
            subprocess.run([str(binary), "--version"], cwd=root, check=True, timeout=30)
        print(f"PASS {archive.name}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    archives = sorted(
        path for path in args.directory.iterdir()
        if path.name.endswith((".tar.gz", ".zip"))
    )
    if not archives:
        parser.error("No native package archives found")
    for archive in archives:
        check_archive(archive)


if __name__ == "__main__":
    main()

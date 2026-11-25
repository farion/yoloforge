#!/usr/bin/env python3
"""Reject incomplete release sets or checksum mismatches before publishing."""
import argparse
import hashlib
from pathlib import Path

PLATFORMS = [
    ("linux", "x86_64"), ("linux", "aarch64"),
    ("macos", "aarch64"),
    ("windows", "x86_64"),
]


def expected_assets(version):
    return {
        f"yoloforge-{kind}-{version}-{os}-{arch}" + (".zip" if os == "windows" else ".tar.gz")
        for os, arch in PLATFORMS
        for kind in (["gui", "runner", "server"] if os == "linux" else ["gui", "runner"])
    }


def verify(directory, version):
    expected = expected_assets(version)
    actual = {path.name for path in directory.iterdir() if path.is_file()}
    required = expected | {name + ".sha256" for name in expected}
    if actual != required:
        raise ValueError(f"Release assets differ: missing={sorted(required - actual)}, unexpected={sorted(actual - required)}")
    sums = []
    for name in sorted(expected):
        path = directory / name
        if path.stat().st_size == 0:
            raise ValueError(f"Empty archive: {name}")
        with path.open("rb") as file:
            digest = hashlib.file_digest(file, "sha256").hexdigest()
        checksum = f"{digest}  {name}\n"
        if (directory / (name + ".sha256")).read_text(encoding="ascii") != checksum:
            raise ValueError(f"Checksum mismatch: {name}")
        sums.append(checksum)
    (directory / "SHA256SUMS").write_text("".join(sums), encoding="ascii")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    verify(args.directory, args.version)
    print(f"Verified all {len(expected_assets(args.version))} GUI, runner and server packages.")


if __name__ == "__main__":
    main()

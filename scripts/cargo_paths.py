"""Resolve build output through Cargo, including user config and environment."""
from functools import lru_cache
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


@lru_cache(maxsize=None)
def target_dir(root=ROOT):
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return Path(json.loads(result.stdout)["target_directory"])


def profile_dir(profile="debug", root=ROOT):
    return target_dir(root) / profile


if __name__ == "__main__":
    print(profile_dir())

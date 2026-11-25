#!/usr/bin/env python3
"""Check Cargo path resolution without starting any Factory services."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from cargo_paths import profile_dir, target_dir


class CargoPathsTests(unittest.TestCase):
    def tearDown(self):
        target_dir.cache_clear()

    def test_metadata_is_authoritative_and_cached(self):
        with patch("cargo_paths.subprocess.run") as run:
            run.return_value.stdout = json.dumps({"target_directory": "/shared/build files"})
            self.assertEqual(profile_dir(root=Path("/repo")), Path("/shared/build files/debug"))
            self.assertEqual(profile_dir("release", Path("/repo")), Path("/shared/build files/release"))
            run.assert_called_once()
            self.assertEqual(run.call_args.kwargs["cwd"], Path("/repo"))

    def test_cargo_error_is_not_replaced_with_a_stale_local_binary(self):
        with patch("cargo_paths.subprocess.run", side_effect=subprocess.CalledProcessError(1, "cargo")):
            with self.assertRaises(subprocess.CalledProcessError):
                target_dir()

    def test_real_cargo_honors_config_environment_and_default(self):
        with tempfile.TemporaryDirectory(prefix="cargo-paths-", dir=os.environ.get("TMPDIR")) as temp:
            root = Path(temp)
            home = root / "cargo-home"
            home.mkdir()
            repo = root / "repo"
            (repo / "src").mkdir(parents=True)
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "cargo-paths-fixture"\nversion = "0.1.0"\nedition = "2021"\n'
            )
            (repo / "src/main.rs").write_text("fn main() {}\n")
            env = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_")}
            env["CARGO_HOME"] = str(home)
            with patch.dict(os.environ, env, clear=True):
                self.assertEqual(target_dir(repo), repo / "target")
                (home / "config.toml").write_text(
                    "[build]\ntarget-dir = " + json.dumps(str(root / "config target")) + "\n"
                )
                target_dir.cache_clear()
                self.assertEqual(target_dir(repo), root / "config target")
                os.environ["CARGO_TARGET_DIR"] = str(root / "env target")
                target_dir.cache_clear()
                self.assertEqual(profile_dir(root=repo), root / "env target/debug")


if __name__ == "__main__":
    unittest.main()

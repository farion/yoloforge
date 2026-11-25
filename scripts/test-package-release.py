#!/usr/bin/env python3
"""Stateless package contract tests; never starts a GUI, Factory or runner."""
import hashlib
import importlib.util
from pathlib import Path
import plistlib
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest import mock
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "v" + tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
SPEC = importlib.util.spec_from_file_location("verify_release", ROOT / "scripts/verify-release.py")
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)
NATIVE_SPEC = importlib.util.spec_from_file_location("native_packages", ROOT / "scripts/test-native-packages.py")
NATIVE = importlib.util.module_from_spec(NATIVE_SPEC)
NATIVE_SPEC.loader.exec_module(NATIVE)


class Packages(unittest.TestCase):
    def test_package_contents_and_complete_release(self):
        with tempfile.TemporaryDirectory(prefix="yoloforge-packages-") as directory:
            root = Path(directory)
            binaries = root / "binaries"
            binaries.mkdir()
            for name in ["factoryd", "factory-runner", "yoloforge-gui", "factory-runner.exe", "yoloforge-gui.exe"]:
                path = binaries / name
                path.write_bytes(b"binary fixture")
                # Deliberately non-executable: Unix archives must set their own modes,
                # including when these contract tests run on Windows.
                path.chmod(0o644)
            output = root / "dist"
            for os, arch in VERIFY.PLATFORMS:
                kinds = ["gui", "runner", "server"] if os == "linux" else ["gui", "runner"]
                for kind in kinds:
                    subprocess.run([
                        sys.executable, str(ROOT / "scripts/package-release.py"),
                        "--kind", kind, "--os", os, "--arch", arch,
                        "--version", VERSION, "--binaries", str(binaries), "--output", str(output),
                    ], check=True, capture_output=True)
                    name = f"yoloforge-{kind}-{VERSION}-{os}-{arch}"
                    archive = output / (name + (".zip" if os == "windows" else ".tar.gz"))
                    with archive.open("rb") as file:
                        digest = hashlib.file_digest(file, "sha256").hexdigest()
                    self.assertEqual(archive.with_name(archive.name + ".sha256").read_text(), f"{digest}  {archive.name}\n")
                    if os == "windows":
                        with zipfile.ZipFile(archive) as contents:
                            names = contents.namelist()
                    else:
                        with tarfile.open(archive) as contents:
                            names = contents.getnames()
                            if os == "macos" and kind == "gui":
                                info = plistlib.loads(contents.extractfile(name + "/YoloForge.app/Contents/Info.plist").read())
                                self.assertEqual(info["CFBundleExecutable"], "yoloforge-gui")
                            binary_path = name + ("/YoloForge.app/Contents/MacOS/" if os == "macos" and kind == "gui" else "/")
                            binary_path += {"gui": "yoloforge-gui", "runner": "factory-runner", "server": "factoryd"}[kind]
                            self.assertTrue(contents.getmember(binary_path).mode & 0o111)
                    names = {entry.removeprefix(name + "/") for entry in names}
                    self.assertFalse(any(entry.endswith((".sqlite3", ".token", ".log")) for entry in names))
                    if kind == "gui":
                        prefix = "YoloForge.app/Contents/MacOS/" if os == "macos" else ""
                        suffix = ".exe" if os == "windows" else ""
                        self.assertIn(prefix + "yoloforge-gui" + suffix, names)
                        self.assertIn(prefix + "factory-runner" + suffix, names)
                        self.assertEqual("factoryd" in names, os == "linux")
                        self.assertEqual("yoloforge.desktop" in names, os == "linux")
                    if kind == "runner":
                        self.assertEqual("factory-runner@.service" in names, os == "linux")
                    if kind == "server":
                        for required in ("Dockerfile", "compose.yaml", "factory-backup.py", "install-opencode.sh", "docs/deployment.md"):
                            self.assertIn(required, names)
                    with mock.patch.object(NATIVE.subprocess, "run") as startup:
                        NATIVE.check_archive(archive)
                    expected_binaries = 3 if kind == "gui" and os == "linux" else 2 if kind == "gui" else 1
                    self.assertEqual(startup.call_count, expected_binaries)
                    for call in startup.call_args_list:
                        self.assertEqual(call.args[0][1:], ["--version"])
                        self.assertTrue(Path(call.args[0][0]).is_absolute())
                        self.assertTrue(call.kwargs["check"])
                        self.assertEqual(call.kwargs["timeout"], 30)
            VERIFY.verify(output, VERSION)
            (output / "SHA256SUMS").unlink()
            for kind in ("gui", "runner"):
                legacy = output / f"yoloforge-{kind}-{VERSION}-macos-x86_64.tar.gz"
                legacy.write_bytes(b"unexpected Intel package")
                with self.assertRaisesRegex(ValueError, "unexpected=.*macos-x86_64"):
                    VERIFY.verify(output, VERSION)
                legacy.unlink()
            next(output.glob("*.sha256")).write_text("invalid\n")
            with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
                VERIFY.verify(output, VERSION)
            next(output.glob("*.zip")).unlink()
            with self.assertRaisesRegex(ValueError, "Release assets differ"):
                VERIFY.verify(output, VERSION)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Package independently installable YoloForge components; never include runtime state."""
import argparse
import hashlib
from pathlib import Path
import shutil
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kind", choices=["server", "tui", "runner"], required=True)
    parser.add_argument("--os", choices=["linux", "windows"], required=True)
    parser.add_argument("--arch", choices=["x86_64", "aarch64"], required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--binaries", type=Path, default=ROOT / "target/release")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if args.version != "v" + version:
        parser.error("Tag and Cargo version differ")
    if args.os == "windows" and (args.kind != "tui" or args.arch != "x86_64"):
        parser.error("Only the x86_64 TUI is currently supported on Windows")
    binary = {"server": "factoryd", "tui": "yoloforge", "runner": "factory-runner"}[args.kind]
    if args.os == "windows":
        binary += ".exe"
    name = f"yoloforge-{args.kind}-{args.version}-{args.os}-{args.arch}"
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / (name + (".zip" if args.os == "windows" else ".tar.gz"))
    if archive.exists():
        parser.error("Archive already exists; use a fresh output directory")
    with tempfile.TemporaryDirectory(prefix="yoloforge-package-") as temp:
        package = Path(temp) / name
        package.mkdir()
        shutil.copy2(args.binaries / binary, package / binary)
        shutil.copy2(ROOT / "LICENSE", package / "LICENSE")
        if args.kind == "server":
            for source in ("deploy/package/Dockerfile", "deploy/package/compose.yaml", "scripts/factory-backup.py"):
                shutil.copy2(ROOT / source, package / Path(source).name)
            shutil.copytree(ROOT / "docs", package / "docs")
            (package / "README.md").write_text(
                f"# YoloForge server {args.version}\n\n"
                "Contains the Linux server binary and a Compose build using that binary.\n"
                "No Rust compiler or Git checkout is needed on the server.\n\n"
                f"Set YOLOFORGE_VERSION={args.version} and FACTORY_BOOTSTRAP_FILE to the private token file.\n"
                "Use a stable Compose project name across upgrades:\n\n"
                "    docker compose -p yoloforge up -d --build\n\n"
                "See docs/deployment.md for ownership, backup and upgrade instructions.\n",
                encoding="utf-8")
        elif args.kind == "tui":
            shutil.copy2(ROOT / "docs/client-installation.md", package / "README.md")
        else:
            for source in ("deploy/factory-runner@.service", "deploy/runner.env.example", "docs/deployment.md"):
                shutil.copy2(ROOT / source, package / Path(source).name)
        if args.os == "windows":
            with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as output:
                for path in sorted(package.rglob("*")):
                    if path.is_file():
                        output.write(path, path.relative_to(package.parent))
        else:
            with tarfile.open(archive, "x:gz") as output:
                output.add(package, arcname=name)
    with archive.open("rb") as file:
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    print(archive)


if __name__ == "__main__":
    main()

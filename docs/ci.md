# GitHub CI und Releases

Die Workflows folgen dem Aufbau von LAML: native Runner, schreibgeschützte
Build-Jobs und eine getrennte Veröffentlichung des vollständigen Paketsets.
Frühere TUI-, Android- und Container-Image-Builds sind ersetzt.

| Plattform | Architektur | GUI | Runner | Separates Server-Paket |
| --- | --- | --- | --- | --- |
| Linux / Ubuntu 24.04 | x86_64, aarch64 | ja | ja | ja |
| macOS | Apple Silicon aarch64 | ja | ja | nein |
| Windows / MSVC | x86_64 | ja | ja | nein |

Insgesamt entstehen zehn Archive mit einzelnen `.sha256`-Dateien. macOS-GUIs
enthalten eine `.app`, Windows verwendet ZIP, Unix-Pakete verwenden `.tar.gz`.
GUI-Pakete enthalten auf jeder Plattform den passenden Runner, `factoryd`
und die verifizierte OpenCode-Laufzeit.
Kein Paket enthält Laufzeitdaten, Tokens oder Journale.

## Workflows

- `.github/workflows/ci.yml`: Push auf `main`, Pull Requests und manueller Start.
- `.github/workflows/build.yml`: gemeinsame native Build-Matrix für CI und Releases;
  Tests für Plattformcode, Protokoll, Runner, GUI und Server.
  GUI-Tests sind zustandslose egui-Tests ohne Benutzerfenster.
  Die gepinnte OpenCode-Laufzeit wird heruntergeladen, per SHA-512 geprüft
  und vor dem Paketieren mit `--version` geprüft. Anschließend werden die fertigen Archive in ein
  temporäres Verzeichnis entpackt und alle enthaltenen Binaries mit `--version`
  außerhalb des Build-Verzeichnisses geprüft, ohne Dienste oder GUI-Fenster.
  Pakete stehen als `packages-OS-ARCH` sieben Tage
  als Actions-Artefakte bereit.
- `.github/workflows/release.yml`: Tag-Push `v*` oder manueller Start mit einem
  vorhandenen Tag. Tag und Workspace-Version müssen übereinstimmen. Der
  Release-Workflow führt die native Build- und Testmatrix selbst aus; ein
  vorheriger `CI`-Lauf auf `main` ist keine Voraussetzung. Alle Release-Builds
  müssen erfolgreich sein, alle zehn Pakete und ihre SHA-256-Prüfsummen müssen
  vorliegen. Erst dann wird das Draft-Release veröffentlicht.

Beispiel bei Workspace-Version `1.2.3`: Nach dem Commit mit der neuen Version
den Tag direkt pushen, ohne auf einen separaten CI-Lauf zu warten:

```sh
git tag -a v1.2.3 <release-commit> -m "Release v1.2.3"
git push origin v1.2.3
```

Tags mit Vorabkennung, z.B. `v1.2.3-rc.1`, werden als GitHub-Prerelease ohne
`latest` veröffentlicht. Ein fehlgeschlagenes Draft-Release kann erneut befüllt
werden; ein bereits veröffentlichtes Release wird nicht überschrieben.
GitHub-Releases brauchen keine zusätzlichen Secrets, nur das automatisch
bereitgestellte `GITHUB_TOKEN` mit `contents: write` im Publish-Job.
Es erfolgt kein Deployment auf Betreiber-Server.

## Lokale Checks

```sh
cargo test --locked -p factory-platform -p factory-opencode -p factory-runner
cargo test --locked -p factory-gui
python3 scripts/test-package-release.py
actionlint
```

`scripts/package-release.py` erstellt Pakete aus bereits gebauten Binaries.
Für ein lokales GUI-Paket die verifizierte Laufzeit zuerst in dasselbe
Binary-Verzeichnis kopieren:

```sh
CARGO_TARGET_DIR="$PWD/target" cargo build --locked --release -p factory-gui -p factoryd -p factory-runner
cargo run --locked -p factory-opencode --example package_distribution -- "$PWD/target/release"
```

`scripts/verify-release.py` weist fehlende, zusätzliche oder beschädigte Pakete
zurück und schreibt `SHA256SUMS`.
Herstellersignaturen und macOS-Notarisierung sind noch nicht eingerichtet.
Container-Containment bleibt Linux-only; siehe [Runner-Installation](runner-installation.md).

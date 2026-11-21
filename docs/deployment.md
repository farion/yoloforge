# YoloForge: Installation, Gerätezugang und Wiederherstellung

Zielrepository: `farion/yoloforge`. Verbindlicher Ablauf: Commit → Tag → Push →
GitHub Release → Ansible des Betreibers lädt das neueste vollständige Release
und installiert es. Der SSH-Alias ist `rs9tm`; der Entwicklungsagent führt dort
keine Deployments aus. Ein Release veröffentlicht lediglich Artefakte.

`yoloforge` ist der neue CLI-Einstieg; die folgenden `factory`-Befehle verwenden
den kompatiblen Alias. Bestehende Datenpfade, Dienstnamen und Protokollkennungen
bleiben kompatibel. Neue Release-Archive heißen `yoloforge-vVERSION-linux-ARCH.tar.gz`.

Implementiert: Factory-Container, Healthcheck, Geräte-/Runner-Credentials,
SSE-Replay, Runner-Long-Poll, systemd-Vorlage, SSH-Launcher und SQLite-Backup/Restore.
Abgenommen: lokales Ubuntu-24.04-Containerimage auf x86_64, echter Containerneustart,
Geräte-Enrollment/Widerruf und Live-WAL-Sicherung. Ein separater Zielserver,
systemd-Autostart am Laptop, ARM-Binaries und veröffentlichte GitHub-Releases
sind noch nicht abgenommen. Gesamtstand: [implementation-status.md](implementation-status.md).

## Versionierter Installationsvertrag für Ansible

Der Betreiber verwaltet Host, SSH, Docker, Benutzer und Installation weiterhin
mit seinem eigenen Ansible. Factory benötigt keinen Docker-Socket.

| Einstellung | Vertrag |
| --- | --- |
| `FACTORY_IMAGE` | konkreter GHCR-Tag plus Digest; lokal `yoloforge-server:development` |
| `FACTORY_PORT` | Host-Loopback-Port, Standard 7432 |
| `FACTORY_BOOTSTRAP_FILE` | absoluter Hostpfad, lesbar durch Container-UID/GID 10001, z.B. Eigentümer 10001 und Modus 0400 |
| Daten | Compose-Volume `factory-data`, im Container `/data`, UID/GID 10001, Verzeichnis 0700 |
| Readiness | `GET /healthz`, 200 nach Initialisierung/Migration und erfolgreichem DB-Lesezugriff |
| Shutdown | SIGTERM; SSE und Long-Polls werden beendet; laufende OpenCode-Agenten bleiben unabhängig |
| Kompatibilität | Factory-Protokoll 1, DB-Schema 6, OpenCode 2.0.27 |

Bootstrap-Datei mit mindestens 32 zufälligen druckbaren ASCII-Zeichen erzeugen,
z.B. mittels Ansible `password`-Lookup oder `secrets.token_hex(32)`. Keine Secrets
in Git, Image-Buildargs oder Kommandozeilenargumenten ablegen.

```sh
docker build -f deploy/Dockerfile -t yoloforge-server:development .
export FACTORY_IMAGE=yoloforge-server:development
export FACTORY_BOOTSTRAP_FILE=/ABSOLUTER/PRIVATER/PFAD/bootstrap.token
docker compose -p yoloforge -f deploy/compose.yaml up -d
curl --fail http://127.0.0.1:7432/healthz
```

Das Image läuft als UID 10001, mit schreibgeschütztem Root-Dateisystem und
beschreibbarem Datenvolume. Nur der Container bindet `0.0.0.0`; Compose veröffentlicht
den Port ausschließlich auf `127.0.0.1`. Außerhalb des Containers bleibt die
Loopback-Bindung Standard; eine andere Bindung verlangt ausdrücklich
`--allow-container-network`. TLS am Factory-Prozess ist nicht implementiert;
Remotezugriff läuft durch SSH.

## Enrollment und Rollen

`FACTORY_TOKEN_FILE` wird von Server, TUI und Runner unterstützt; alternativ
`FACTORY_TOKEN`. Beide gleichzeitig werden abgewiesen. Token-Dateien dürfen am
Ende einen Zeilenumbruch enthalten. Das Bootstrap-Token hat Adminrechte und ist
nicht für den normalen Runnerbetrieb gedacht.

```sh
# Adminzugang zum Server, direkt oder über den SSH-Tunnel:
export FACTORY_TOKEN_FILE=/privat/bootstrap.token
factory device invite --role admin --output /privat/admin.enrollment
factory device enroll --name laptop-admin --code-file /privat/admin.enrollment --output /privat/admin.token
export FACTORY_TOKEN_FILE=/privat/admin.token

factory device invite --role operator --output /privat/tui.enrollment
factory device enroll --name laptop-tui --code-file /privat/tui.enrollment --output /privat/tui.token

# Den Runner vorher mit stabiler UUID benennen, dieselbe ID bei --id verwenden:
factory device invite --role runner --runner RUNNER-UUID --output /privat/runner.enrollment
factory device enroll --name laptop-runner --code-file /privat/runner.enrollment --output /privat/runner.token
factory device list
factory device revoke DEVICE-UUID
```

Codes gelten standardmäßig 600 Sekunden, konfigurierbar von 30 bis 900 Sekunden
über `--ttl-seconds`, und lassen sich genau einmal einlösen. Sie enthalten ca.
244 Zufallsbits. Die DB enthält ausschließlich SHA-256-Prüfwerte von Codes und
Tokens; das Eventlog enthält nur Geräte-Metadaten. CLI-Ausgabedateien werden mit
0600 und `create_new` erstellt. Vorhandene Dateien werden nicht überschrieben.
Bei einem Netzwerkfehler kann eine leere reservierte Ausgabedatei zurückbleiben;
nur diese nach Prüfung entfernen. Nach verlorenem Enrollment-Ergebnis den
gegebenenfalls angelegten Geräteeintrag widerrufen und einen neuen Code erzeugen.

- **Admin:** Verwaltung und Geräteverwaltung.
- **Operator:** Projekte, Tickets, Streams, Fragen und Verlauf; keine Runner-Identität
  und keine Geräteverwaltung.
- **Runner:** ausschließlich Endpunkte seiner gebundenen Runner-ID; zusätzlich
  gelten Registrierung, Command-Zuordnung und Ownership-Epoche.

Widerruf sperrt neue Aufrufe und beendet offene SSE-/Long-Poll-Verbindungen.
Bereits laufende kurze Transaktionen können noch abschließen. Widerruf beendet
keine unabhängigen OpenCode-Prozesse und ist kein Ownership-Transfer.

Nach Enrollment eines Admin-Geräts kann der Server ohne Bootstrap-Umgebungsvariable
gestartet werden. Er verlangt dann mindestens einen aktiven Admin in der DB.
Für Compose den Bootstrap-Environment-/Secret-Eintrag aus der eigenen, von Ansible
verwalteten Konfiguration entfernen, Datenvolume und Compose-Projektname beibehalten.
Bei verlorenem letztem Adminzugang ist ein bewusst eingerichtetes neues
Bootstrap-Token mit Serverneustart der administrative Recovery-Weg.

## SSH und Runner als Dienst

OpenSSH-Hostalias und `known_hosts` werden vom Betreiber eingerichtet. Der Launcher
verwendet bestehende SSH-Konfiguration und Agent, `StrictHostKeyChecking=yes`,
`BatchMode=yes` und `ExitOnForwardFailure=yes`:

```sh
export FACTORY_TOKEN_FILE=/privat/tui.token
python3 scripts/factory-connect.py --host factory-server --local-port 17432 -- factory
```

Der Tunnel ist an den Clientprozess gebunden. Bei Tunnelende wird dieser beendet,
bei Clientende der Tunnel. Entfernte Agenten und deren Workspace-Ownership bleiben
unverändert. Der Launcher installiert keine Schlüssel und umgeht keine Hostprüfung.

`deploy/factory-runner@.service` nach `~/.config/systemd/user/` kopieren, Binaries
nach `~/.local/bin/` installieren und `runner.env.example` als private
`~/.config/yolofactory/runner-laptop.env` mit absoluten Pfaden ausfüllen.
Den tatsächlichen OpenCode-Service-Registrierungspfad aus der eigenen Installation
übernehmen; der Beispielpfad ist kein garantierter OpenCode-Standardpfad.

```sh
systemctl --user daemon-reload
systemctl --user enable --now factory-runner@laptop
systemctl --user status factory-runner@laptop
```

Ein dauerhafter Remote-Runner benötigt einen separat überwachten SSH-Tunnel,
etwa eine vom Betreiber verwaltete systemd-Unit. Bei geschlossenem Laptop kann
der lokale Runner nicht weiterarbeiten. Server-Runner und OpenCode müssen dieselben
absoluten Worktree-Pfade sehen. Journals nicht zwischen Runner-IDs austauschen.

## Backup, Restore und Updates

`scripts/factory-backup.py` verwendet die SQLite-Online-Backup-API einschließlich
WAL, prüft Integrität und Foreign Keys und erstellt ein neues privates Ziel.
Ausgabe: Schema-Version, SHA-256 und Größe. Ein bestehendes Ziel oder vorhandene
WAL-/SHM-Sidecars werden niemals überschrieben.

```sh
python3 scripts/factory-backup.py backup --source /daten/factory.sqlite3 --destination /backup/factory-2026-11-21.sqlite3
python3 scripts/factory-backup.py restore --source /backup/factory-2026-11-21.sqlite3 --destination /neue-daten/factory.sqlite3
```

Im Container ist das Skript als `factory-backup` installiert. Sicherungen aus dem
Volume auf einen getrennten, zugriffsgeschützten Sicherungsort kopieren.
Das Skript sichert **nur die Factory-DB**. Runner-Journals, Repositories samt
Worktrees, OpenCode-Zustand und Credential-Dateien separat konsistent sichern.
Ein DB-Restore ist keine Prozessrücksetzung: während einer Gesamtwiederherstellung
Runner und OpenCode kontrolliert anhalten; unbestätigt laufende Prozesse nicht
durch einen zweiten Schreiber ersetzen.

Updateablauf für Ansible:

1. Version/Digest und Prüfsummen festlegen, aktuellen Zustand sichern.
2. Dienste koordiniert aktualisieren; aktive Agenten und Journals beibehalten.
3. Factory starten, Migration/Healthcheck abwarten, Runner/TUIs aktualisieren.
4. Snapshot, Registrierungen und eine ungefährliche Leseoperation prüfen.

Downgrade auf ein älteres DB-Schema wird abgewiesen. Rollback benötigt die passende
Sicherung plus vorheriges Binary/Image. Restore immer in einen neuen Pfad bzw.
ein neues Volume; erst danach die gestoppte Dienstkonfiguration umschalten.

## Releases und Prüfungen

`.github/workflows/release.yml` baut bei einem zum Cargo-Versionsfeld passenden
`v*`-Tag Linux-x86_64-/ARM64-Binaries, Archive mit Deploymentdateien und SHA-256-
Prüfsummen sowie ein GHCR-Serverimage. Die Serverimage-Pipeline baut zunächst
x86_64. Binaries setzen Ubuntu 24.04 oder neuer voraus. CI testet Rust und die
modellfreien Prozess-/PTY-Smokes. Zielrepository ist `farion/yoloforge`;
es wurde noch kein Tag gepusht und kein öffentliches Release veröffentlicht.

Der Publish-Job wartet auf beide Binary-Architekturen und das Image. Erst nach
Prüfsummenprüfung wird ein vollständiges Release aus dem Draft veröffentlicht.
`server-image.json` enthält den unveränderlichen GHCR-Digest. Ansible löst
`https://api.github.com/repos/farion/yoloforge/releases/latest` einmal pro Lauf
auf, lädt die Assets dieses Tags, prüft die passende Prüfsumme und verwendet
den Image-Digest. So werden während eines Builds keine halbfertigen Releases
als neuestes Release installiert.

Lokale Abnahme:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
python3 -u scripts/smoke-foundation.py
python3 -u scripts/smoke-tui-answers.py
python3 -u scripts/smoke-backup.py
python3 -u scripts/smoke-deployment.py --image yoloforge-server:development
```

Der Container-Smoke erzeugt ausschließlich eigene temporäre Container/Volumes
und entfernt diese anschließend; das authentifizierte OpenCode-Labor bleibt erhalten.

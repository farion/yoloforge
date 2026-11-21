# Dedizierter Container-Freezer (Entwicklung nach v0.1.1)

`factory-runner --containment containment.json` kann einen laufenden
Agentenauftrag durch Docker/cgroup v2 einfrieren und dieselbe Ausführung nach
Resume auftauen. Der Runner läuft **außerhalb** des Containers und bleibt dabei
ansprechbar. Ein dedizierter Runner verwaltet in diesem Modus genau einen
Workstream; Agentenaufträge anderer Streams bleiben ungeklärt und werden nicht
gestartet. Bestehende unverwaltete Ausführungen werden nicht nachträglich adoptiert.

## Einrichtung

Zuerst Server mit Schema 14 aktualisieren, dann Runner. Das Runner-Journal wird
auf Schema 3 angehoben, damit ältere Runner die Laufzeitbindung nicht ignorieren.
Downgrade benötigt die passende Sicherung. Der neue Runner bewirbt
`container-freeze-v1`; ohne Konfiguration bleibt die bisherige native
Interrupt-Semantik bestehen.

1. Projekt/Workstream und dedizierten Runner anlegen, Worktree wie bisher mit
   `yoloforge stream prepare` vorbereiten. Noch keinen Agentenauftrag starten.
2. Eine eigene OpenCode-Instanz im Container starten. Die vorhandene gemeinsame
   Laborinstanz ist dafür ungeeignet. Keine weiteren API-Schreiber oder Sessions
   in diesem Container betreiben.
3. Registrierung auf dessen loopbackgebundenen Port richten; Container-ID und
   Stream-ID explizit konfigurieren, Runner mit dem bisherigen Journal starten.

```json
{
  "stream_id": "STREAM-UUID",
  "container_id": "VOLLSTAENDIGE-64-STELLIGE-CONTAINER-ID",
  "docker_socket": "/var/run/docker.sock"
}
```

`--opencode-registration` bleibt eine private Datei mit `url` und `password`.
Die URL muss exakt `http://127.0.0.1:PORT` auf den veröffentlichten Containerport
`4096/tcp` zeigen. Das Passwort wird wie bisher durch OpenCode HTTP Basic geprüft.

Der Operator stellt den Container mit folgenden Eigenschaften bereit; der Runner
prüft sie bei Bindung und jedem Zustandsabgleich:

- Linux Docker auf lokalem Unix-Socket und cgroup v2, volle immutable Container-ID.
- Labels `io.yoloforge.stream=STREAM-UUID` und `io.yoloforge.runner=RUNNER-UUID`.
- `--read-only --cap-drop ALL --security-opt no-new-privileges`, explizite
  numerische nicht-root UID; keine zusätzlichen Capabilities oder Geräte.
- `--cgroupns private --ipc private`, eigener PID- und Netzwerk-Namespace;
  kein Hostnetz, keine automatische Restart-Policy.
- `-p 127.0.0.1:PORT:4096`; nur dieser Container bedient die registrierte URL.
- Bind-Mount exakt des vorbereiteten Worktrees an denselben absoluten Pfad,
  schreibbar für die Container-UID. Optional Repository-`.git` am gleichen
  absoluten Pfad, **read-only**. Keine anderen Bind-Mounts/VolumesFrom.
- Persistente OpenCode-Daten in einem lokalen Docker-Volume ohne Driver-Optionen,
  mit denselben Stream-/Runner-Labels. Kein anderer Container darf es verwenden.
  Der Operator richtet Eigentümer und OpenCode-HOME passend zur UID ein.
- Optionale tmpfs-Mounts ausschließlich `/tmp` und `/run`.

Das Image muss die benötigte OpenCode-Version und Build-Werkzeuge enthalten.
Git-Metadaten sind im Container bewusst nur lesbar: Dateiänderungen/Tests sind
möglich, Git-Commits dort nicht. Checkpoints benötigen einen späteren separaten
Integrationspfad. Runner und OpenCode sehen identische absolute Worktree-Pfade.
Das Container-Passwort sowie Provider-Zugangsdaten verbleiben in privater
Konfiguration/privatem Volume; Docker-Socket und Host-Credentials werden nicht
in den Container gemountet.

## Ablauf und Fehlerfälle

Vor Sessionanlage speichert der Runner Container-ID, Startzeit, Endpoint und
Workspace. Fortsetzungen verlangen dieselbe Bindung wie der Vorgänger. Vor
Agentenfortschritt, Compaction und Antworten wird die Instanz erneut überprüft.
Entfernte Konfiguration, anderer Endpoint oder Containerneustart blockieren die
Ausführung. Ein neu gestarteter Container wird nicht automatisch als Fortsetzung
der alten Instanz akzeptiert.

Die Pausenverarbeitung läuft vor OpenCode-Aufrufen: Auch dessen HTTP-Service wird
eingefroren. Pro Control-Request wird höchstens ein Docker-Schreibversuch
journalisiert. Absturz nach Intent vor dem Aufruf und verlorene Antworten führen
zum lesenden Abgleich, niemals zum automatischen Wiederholen. Bereits erfüllte
Requests werden ebenfalls gespeichert: Ein extern aufgetauter Container wird
nicht kommentarlos erneut eingefroren. Unklare Zustände reservieren den Stream.
Ein expliziter, revisionsgeschützter neuer Versuch derselben Container-Aktion ist
über [Control-Recovery](control-recovery.md) möglich, sofern bereits ein
Container-Nachweis vorhanden ist. Der alte Request wird nicht erneut ausgeführt.
Nach einem bestätigten Thaw läuft die ursprüngliche Ausführung weiter; es wird
kein neuer Prompt versendet.

Ein Freezer-Bericht beschreibt den **beobachteten** Zustand. Docker-Administratoren
können ihn anschließend ändern; der nächste Abgleich erkennt Abweichungen.
Einfrieren beendet keine Prozesse, annulliert keine schon ausgelösten externen
Dienste und berechtigt nicht zum Ownership-Transfer oder zweiten Schreiber.
Dieser Modus ist kein Schutz gegen einen Host-/Docker-Administrator. Remote-Tools
außerhalb des Containers sind nicht Teil des Nachweises. Aktive Beendigung und
Recovery eines gewechselten Containers bleiben offen.

## Reproduzierbare lokale Abnahme

```sh
docker build -f scripts/fixtures/freezer.Dockerfile \
  -t yoloforge-freezer-test:development scripts/fixtures
cargo test -p factory-runner --locked real_freezer -- --ignored --nocapture
```

Der Test erzeugt nur einen eigenen Container und entfernt ihn anschließend.
Geprüft: gewöhnlicher und doppelt geforkter/`setsid`-Prozess mit ignorierten
Terminationssignalen, ausbleibende Dateischreibvorgänge während Freeze, fortgesetzte
Schreibvorgänge nach Thaw, Journalneustart, verlorener Abschlussabgleich,
persistierter Intent ohne Docker-Aufruf, externe Zustandsänderung, fremder
Instanznachweis, Containerneustart und entfernte Konfiguration. Er braucht keinen
Providerzugang und ruft kein Modell auf. Server-Store-/HTTP-Tests prüfen separat
Control-Transitions, Fencing und Capability-Schutz. Wiederholte Bestätigungen
derselben Freezer-Lage aktualisieren den Beobachtungszeitpunkt dauerhaft, ohne
die Streamrevision zu verändern: Ein Heartbeat macht eine bereits vorbereitete
Resume-Entscheidung nicht allein durch eine neuere Beobachtungszeit ungültig.

## Vollständiger Modelltest — lokal bestanden am 21.11.2026

```sh
cargo build --workspace --locked
docker build -f scripts/fixtures/contained-opencode.Dockerfile \
  -t yoloforge-contained-opencode:2.0.27 scripts/fixtures
python3 -u scripts/smoke-contained-agent.py
```

Der Fixture-Build benötigt das lokale Laborimage `yolofactory-opencode-spike:2.0.27`.
Der Test liest standardmäßig die private Laborregistrierung unter
`spikes/opencode/.runtime/registration.json`; mit `--credential-source DATEI`
kann eine andere explizite Registrierung gewählt werden. Er verlangt einen
mindestens 30 Minuten gültigen OpenAI-OAuth-Access-Token. **Der Refresh-Token wird
nicht übernommen**; der temporäre Dienst kann die Anmeldung der Quellinstanz
damit nicht erneuern/rotieren. Das vorhandene Labor und dessen Datenvolume werden
weder neu gestartet noch verändert. Es wird kein Datenbank-/Sessionklon angelegt.

Getestet mit OpenCode 2.0.27 und `openai/gpt-6-astra`:

- Eigener non-root/read-only Container mit dediziertem Volume und exaktem
  Worktree-Mount; Host-Runner mit eigenem Journal und gescoptem Gerätecredential.
- Ein echter Modellauftrag führt ein wartendes Vordergrundtool aus, das einen
  doppelt abgekoppelten Childprozess startet; beide schreiben Fortschrittsmarker.
- Pause über Factory-CLI friert beide Prozesse ein. Während Factory und Runner
  neu starten, bleiben beide Marker unverändert; der neue Runner bestätigt
  dieselbe Container-Instanz erneut.
- Resume über CLI taut dieselbe Instanz auf. Beide Prozesse arbeiten weiter;
  das Tool wird genau einmal ausgeführt, erstellt/kompiliert seine Ergebnisdatei
  und die ursprüngliche Session schließt erfolgreich ab.
- Der Runner beendet den finalen Archivscan; der revisionsgebundene JSONL-Export
  enthält die projektierten Nachrichten und den eindeutigen Testmarker.

Testcontainer und dessen Volume werden anschließend entfernt. Bei Fehlern bleibt
ein privates Diagnoseverzeichnis unter `/tmp/opencode/contained-agent-*` erhalten;
bei Erfolg wird es ebenfalls entfernt. Der Test ist opt-in und verwendet echte
Modellkapazität. Die Abnahme auf dem Zielhost, automatische Provisionierung,
Recovery nach Containerverlust und Ownership-Transfer bleiben offen.

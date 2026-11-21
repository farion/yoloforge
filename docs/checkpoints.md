# Checkpoints und Runner-Übergabe

API/CLI und Runner unterstützen Git-Checkpoints und Übergaben zwischen zwei
Runnern. Factory speichert das Bundle in SQLite; der Zielrunner lädt es über
seinen authentifizierten Command-Endpunkt. Ein gemeinsames Dateisystem ist
nicht erforderlich.

## Voraussetzungen

- Server, Clients und beide Runner aktualisieren: DB-Schema **21**, Runner-Journal
  **7**, Capability `git-checkpoint-v1`. Erst Server aktualisieren, dann Runner und
  Clients; anschließend neue Funktionen verwenden.
- Beide Runner benötigen eine lokale Repository-Zuordnung für dasselbe Projekt.
- Der Quell-Worktree muss vorbereitet sein. Alle Agenten-Runtimes der aktuellen
  Ownership-Epoche müssen nachweislich entfernt sein. Der Runner prüft den
  gespeicherten Docker-Daemon und die Abwesenheit erneut vor der Aufnahme.
- Ohne vorherige Agentenausführung reicht der vorbereitete Worktree. Ein natives
  Agent-Ergebnis oder ein erfolgreicher Sessionabschluss ist allein **kein**
  Prozessruhe-Nachweis. Aktive Container-Aufträge zuerst gemäß
  [runtime-termination.md](runtime-termination.md) beenden.

## Ablauf

Aktuelle Streamrevision und Aufträge: `yoloforge --snapshot`.

```sh
yoloforge stream checkpoint STREAM_ID --expected-revision REVISION
# Antwort enthält die neue Command-ID; auf succeeded im Snapshot warten.
yoloforge checkpoint CHECKPOINT_COMMAND_ID

yoloforge stream handoff STREAM_ID \
  --expected-revision CURRENT_REVISION \
  --checkpoint CHECKPOINT_COMMAND_ID --runner TARGET_RUNNER_ID \
  --reason 'Auf dem anderen Rechner weiterarbeiten' --request-id REQUEST_UUID

# Neue Revision aus der Handoff-Antwort verwenden:
yoloforge stream prepare STREAM_ID --expected-revision NEW_REVISION
```

Die Checkpoint-Abfrage zeigt das eingefrorene Kontextpaket und die Stilllegungs-
nachweise; das Command-Ergebnis im Snapshot enthält Commit/Tree, Bundle-SHA-256,
Größe und lokale Manifest-/Bundle-Pfade. Erst nach bestätigtem Bundle-Upload
wird der Checkpoint-Auftrag erfolgreich abgeschlossen.

Die Übergabe akzeptiert nur den neuesten, erfolgreichen Checkpoint als letzten
Auftrag des Streams. Jede spätere Ausführung erfordert einen neuen Checkpoint.
Request-Replay ist idempotent; konkurrierende Übergaben sind revisionsgeschützt.
Die Ownership-Epoche wird atomar erhöht. Alte Runner dürfen danach weder neue
Ergebnisse schreiben noch Zielartefakte lesen. Bereits bestätigte Upload-Receipts
können unverändert wieder abgefragt werden.

Der Zielrunner prüft Größe, SHA-256, Bundle-Referenz und Commit. Er importiert in
das zugeordnete Repository und erstellt einen neuen Branch und Worktree:
`factory/STREAM_ID-epoch-N`, `WORKSPACE_ROOT/STREAM_ID-epoch-N`.
So bleibt auch bei einer Rückübergabe der ursprüngliche Worktree erhalten.
Fehlgeschlagene oder unterbrochene Vorbereitung gibt keine Agentenausführung
frei. Erst eine erfolgreiche Vorbereitung derselben Epoche erlaubt den Start.
Die nächste Session ist neu und erhält den historischen Checkpoint-Kontext als
gekennzeichnetes Referenzmaterial; native Session-Fortsetzung wird nicht übertragen.

## Inhalt und Wiederanlauf

- Aufgenommen werden Arbeitsdateien einschließlich neuer, nicht ignorierter
  Dateien, Löschungen, Binärdateien und Symlinks. Ein eigener temporärer Git-Index
  hält den Originalindex, HEAD und Arbeitsbaum unverändert.
- Das selbstständige Git-Bundle enthält den Checkpoint und erreichbare Git-Historie.
  Ignorierte ungetrackte Dateien werden nicht aufgenommen; getrackte Dateien bleiben
  Bestandteil von Git. Submodule/eingebettete Repositories und ungelöste Index-
  Konflikte werden abgewiesen.
- Ein Checkpoint-Plan pinnt Tree, Parent, Commit und Kontext vor dem Veröffentlichen
  der Referenz. Nach Neustart wird derselbe Stand verwendet, auch wenn ein Benutzer
  inzwischen weiter editiert hat. Während der Aufnahme den Quell-Worktree nicht
  manuell bearbeiten; Änderungen zwischen den beiden Scans werden abgewiesen.
- Bundles sind auf **64 MiB** begrenzt. Uploads sind unveränderlich und wiederholbar;
  Download und Import dürfen nach Abbruch erneut gelesen werden. Fremde oder
  veränderte Dateien/Referenzen werden nicht überschrieben.
- Wissenskontext, Profil, Ziel, Ticket-IDs, Quellaufträge und Vorgänger-Checkpoint
  bleiben nachvollziehbar. Native Sessionarchive liegen weiterhin in Factory und
  können nach Beendigung unvollständig sein. Kontextwissen wird nicht als geprüft
  ausgegeben.
- Bundle-Blobs gehören jetzt zum Factory-SQLite-Backup. Lokale zusätzliche
  Artefakte liegen unter `WORKSPACE_ROOT/.checkpoints/`; es gibt noch keine
  automatische Aufbewahrungs-/Löschrichtlinie.

## Abnahme und verbleibende Grenzen

```sh
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke-handoff.py
```

Der Prozess-Smoke prüft zwei unabhängige Runner/Repositories, Änderungen und
Binärdateien, Factory-/Runner-Neustart, Transfer ohne verfügbares Quellbundle,
Rückübergabe und Erhalt des ursprünglichen Arbeitsbaums. Store-Tests prüfen
Stilllegung, Kontextbindung, Blob-Replay, Ownership-Fencing, Revisionskonflikte
und vollständigen Rollback bei Eventfehlern. Git-Tests prüfen Originalindex,
dirty Worktree, ignorierte Dateien, Symlinks und Journal-Replay.

Getrennte Zielhosts und Übergabe nach echtem Modelllauf sind noch nicht abgenommen.
Checkpoint und Handoff sind auch in der TUI-Streamsteuerung verfügbar (siehe
`review-integration.md`). Erfolgreich abgeschlossene Runtimes können inzwischen
unter Erhalt ihres Ergebnisses entfernt werden; der Realmodell-Test prüft danach
die Checkpoint-Erstellung. Native Prozessruhe und sehr große/submodulbasierte
Projekte bleiben Erweiterungen.

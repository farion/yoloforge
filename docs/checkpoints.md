# Checkpoints und Runner-Übergabe

## Automatischer Wechsel

Für Projekte mit Runner-Zuordnung **Automatisch** sichert die Factory bei
scheduler-gesteuerten Aufträgen nach der Workspace-Vorbereitung zunächst den
committeten Stand, wenn der Runner automatische Checkpoints unterstützt.
Der Upload enthält Git-Historie, aber keine uncommitteten Arbeitsdateien.
Wird dieser Runner danach unerreichbar, kann die Factory einen noch nicht
gestarteten Auftrag (auch nach Annahme) auf einen kompatiblen und für die gewählten MCPs
geeigneten Runner umstellen. Der Zielrunner erstellt aus dem bestätigten
Bundle ein neues verwaltetes Repository; der Desktop-Pfad ist dort nicht nötig.

Sobald der committete Stand übertragen ist, bevorzugt **Automatisch** für
einen neuen portablen Auftrag einen verfügbaren und kompatiblen **Server-Runner**,
auch wenn der Laptop noch online ist. Der Desktop-Runner bereitet den Stand vor,
startet aber nicht die schreibende Ausführung, wenn der Server übernehmen kann.
Sobald der Server-Runner den Auftrag ausführt, kann der Laptop ausgeschaltet
bleiben. Der Server-Runner muss mit `--location server` registriert sein; sein
Name allein genügt nicht. Fest zugewiesene Runner und Aufträge, die den lokalen
Checkout oder lokale MCPs benötigen, werden nicht automatisch verschoben.
Vor einer neuen Vorbereitung am Quell-Runner übernimmt der verwaltete
Git-Basisbranch lokal hinzugekommene Commits, solange ein Fast-forward möglich
ist. Bereits vorbereitete Arbeitsbereiche behalten ihren gepinnten Stand.

Vor dem ersten schreibenden OpenCode-Aufruf holt der Runner eine Startfreigabe
ein. Fehlt danach der Heartbeat, behandelt die Factory das nicht als Stopp.
Bei einer nachweislich gebundenen, isolierten Container-Runtime fordert
**Automatisch** bei längerem Kontaktverlust deren kontrolliertes Entfernen an,
sofern ein geeigneter Ersatzrunner und ein bestätigtes Bundle verfügbar sind.
Ausführungen, die auf eine Antwort warten oder ihren Abschluss melden, werden
dabei nicht abgebrochen.
Erst mit einem passenden Stoppnachweis startet der Auftrag auf dem Ersatzrunner
in einer neuen Session vom Checkpoint vor dem ursprünglichen Start neu.
Neuere Änderungen werden dabei nicht übertragen; externe Effekte werden nicht
zurückgesetzt. Bei einem ausgeschalteten Rechner muss der Nachweis ggf.
bis zur Wiederverbindung warten. Bei einem lokalen Linux-Runner mit verwaltetem
OpenCode kann die Factory nach einer erneuten, gerätegebunden authentifizierten
Registrierung auf demselben Rechner einen vollständigen Systemneustart
erkennen. Erst dann darf Auto den nativen Auftrag vom bestätigten Checkpoint
auf einem anderen Runner neu starten. Ein Runner-Neustart im selben Systemboot,
ein anderer Rechner oder ein fehlender Heartbeat reichen nicht aus. Das ist
keine Fortsetzung der nativen Session; seit dem Checkpoint entstandene
Änderungen fehlen. Runner-Identität und Gerätezugang nicht auf andere Rechner
klonen. Ein manuell angeforderter Stopp löst keine automatische Wiederholung
aus.

Ohne bestätigtes Bundle gibt es keinen automatischen Wechsel. Explizit zugewiesene Runner,
lokale Git-Zieloperationen und native Session-Fortsetzungen bleiben gebunden.
Die bestehende Bundle-Grenze von 64 MiB gilt auch hier. Schlägt der Upload
fehl, kann der Auftrag auf dem ursprünglichen Runner starten, ist aber nicht
automatisch übertragbar.

### Neue Aufträge bei ausgeschaltetem Quell-Runner

Hat ein Auto-Projekt einen lokalen Quellpfad, können Orchestrator und neuer
Auftrag auf einem anderen Runner aus einem bestätigten Projekt-Checkpoint
starten. Dafür muss der Checkpoint zum gespeicherten Quell-Checkout und
Basisbranch passen. Wenn ein schreibender Auftrag noch aktiv ist oder die
Factory seit dem Checkpoint ein neueres erfolgreiches Ergebnis kennt, wartet
der neue Auftrag stattdessen auf einen aktuellen Stand. Nicht zur Factory
übertragene lokale Änderungen sind auf dem Ersatzrunner nicht verfügbar.
Die gespeicherte Zuordnung zum Quell-Checkout bleibt unverändert.

### Ergebnisse aus einem übertragenen Arbeitsbereich

Nach erfolgreicher Ausführung in einem importierten Arbeitsbereich exportiert
die Factory den neuen, committeten Stand automatisch als nächsten
Projekt-Checkpoint. Der Runner prüft, dass der Ergebniscommit vom importierten
Stand abstammt und keine nicht ignorierten, uncommitteten Dateien zurückbleiben.
Ein nachweislich neueres oder gleichzeitig laufendes schreibendes Ergebnis
verhindert eine automatische Veröffentlichung konkurrierender Stände.

Folgeaufträge können nach bestätigtem Upload diesen Stand verwenden, auch
wenn der ursprüngliche Runner nicht erreichbar ist. Wenn der ursprüngliche
Runner zurückkommt, überträgt Auto den verifizierten Stand auch in den
ursprünglichen Desktop-Checkout: nur auf dem vorgesehenen Branch,
bei sauberem Arbeitsverzeichnis und nur per Fast-forward. Lokale Änderungen,
abweichende Historie und kollidierende ignorierte Dateien werden nicht
überschrieben. Checkouts mit externen Git-Filtern werden nicht automatisch
aktualisiert. Ein blockierter Rücktransfer bleibt in der Diagnose sichtbar
und kann nach Bereinigung erneut versucht werden. Bis dahin bleibt der
bestätigte Ergebnisstand in der Factory verfügbar.
Nach bestätigter Rückübernahme kann Auto bei erreichbarem Quell-Runner wieder
einen frischen Stand vom Desktop sichern, statt immer die ältere Kopie zu nutzen.

## Manuelle Übergabe

API/CLI und Runner unterstützen Git-Checkpoints und Übergaben zwischen zwei
Runnern. Factory speichert das Bundle in SQLite; der Zielrunner lädt es über
seinen authentifizierten Command-Endpunkt. Ein gemeinsames Dateisystem ist
nicht erforderlich.

## Voraussetzungen

- Server, Clients und beide Runner aktualisieren: DB-Schema **22**, Runner-Journal
  **8**, Capability `git-checkpoint-v1`. Erst Server aktualisieren, dann Runner und
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

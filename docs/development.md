# Factory-Entwicklungsstand starten

## Auftragsdetails und Factory-Verlauf

`5` → Auftrag auswählen → `Enter` öffnet die schreibgeschützte Detailansicht.
`Tab`/`Shift-Tab` bzw. links/rechts wechseln zwischen Übersicht, vollständigem
Prompt, gespeichertem Ausführungsprofil, Factory-Ereignissen und Sessionauszug. Hoch/runter,
Bild auf/ab und Home scrollen; `Esc` kehrt zur Liste zurück.

Die Übersicht zeigt Runner, Stream, Ownership-Epoche, deterministische Session-/
Nachrichten-ID, den zuletzt bestätigten Workspace-Pfad, Fortschritt und Ergebnis.
Profil und Prompt stammen aus dem unveränderlichen Auftrag, nicht aus einem
nachträglich bearbeiteten Projektprofil.

Die Ereignisansicht liest den dauerhaften Factory-Eventstream seitenweise ab
Cursor 0 und folgt neuen Ereignissen im Hintergrund. Sie filtert nach Auftrag
einschließlich seiner Fragen und Pause-/Resume-Kontrollen. Die letzten 256
passenden Ereigniszusammenfassungen bleiben im Speicher (neueste zuerst);
Gesamtzahl und Cursor machen die Begrenzung sichtbar. Zusammenfassungstexte
sind auf 400 Zeichen gekürzt. Das Eventarchiv selbst wird nicht gekürzt.
Netzwerkfehler behalten Anzeige und Cursor; bei Wiederverbindung wird dort
weitergelesen. Schließen beendet nur den lesenden Hintergrundtask.

Die zusätzliche Seite **Sessionauszug** zeigt gespeicherte OpenCode-Nachrichten
und textuelle Toolausgaben. `p`/`n` wechselt die Nachricht; zunächst wird die neueste
angezeigt. Erfassungszeit, Quellrevision und Kürzungsmarker bleiben sichtbar.
Der Runner liest während seiner Verarbeitung die neuesten bis zu 32 Nachrichten,
begrenzt auf 8 KiB Text je Nachricht und 128 KiB insgesamt. Ältere Nachrichten
oder nicht dargestellte Inhalte werden ausdrücklich markiert. Toolname, Status,
Input, Textausgabe und Fehler werden übernommen; Provider-Zustand, binäre Inhalte
und Dateianhänge werden nicht eingebettet. Dies ist ein begrenzter aktueller
Auszug, noch kein vollständiges, historisch wachsendes Chat-/Tool-Archiv.

Der Auszug wird vor der Übertragung im Runner-Journal gespeichert und vor dem
Auftragsabschluss an Factory geliefert. Netzwerkfehler, ungültige/zu große
OpenCode-Antworten oder leere Projektionen überschreiben keinen vorhandenen
Runner-Auszug. Der Zeitstempel kann dann älter als das Auftragsergebnis sein;
er ist kein Vollständigkeitsnachweis. Nach Abschluss werden keine weiteren
OpenCode-Nachrichten automatisch nachgeladen. Factory hält den letzten Auszug
in SQLite (Schema v5), separat vom Eventlog und Hauptsnapshot. Lesen erfordert
keine aktive OpenCode-Verbindung. Vorhandene alte Aufträge werden nicht rückwirkend
importiert. Server und Runner gemeinsam aktualisieren.

## Projekte, Tickets und Streams direkt in Ratatui

| Ansicht | Neue Einträge | Ausgewählten Eintrag öffnen |
| --- | --- | --- |
| `2` Projekte | `n`: Name, Repository und Basisbranch | `e`: Metadaten; `Enter`: Provider, Modell, Toolchain, Start-/Testbefehle |
| `3` Tickets | `n`: Titel, Beschreibung, Abnahmekriterien und Priorität | `Enter`: Felder und Status bearbeiten |
| `4` Streams | `n`: Ziel, Runner und optionale Tickets | `Enter`: Stream-Steuerung |

Links/rechts wählt weiterhin das aktive Projekt für Tickets und Streams;
hoch/runter markiert den obersten sichtbaren Listeneintrag. Im Verwaltungsdialog
wechselt `Tab`/`Shift-Tab` das Feld. Textfelder unterstützen Unicode, Paste,
Cursorpfeile, Home/End und Backspace/Delete; in mehrzeiligen Feldern fügt Enter
einen Zeilenumbruch ein. Hoch/runter und Leertaste bedienen Auswahlfelder;
bei Tickets für einen Stream schaltet Leertaste die jeweilige Zuordnung um.
Der Runner muss ausdrücklich ausgewählt werden. Seine ID, Ausführungsort und
Verbindung werden angezeigt. `Ctrl-R` aktualisiert auch die verfügbaren Runner.

`Ctrl-S` speichert asynchron, `Esc` schließt den Dialog und erhält den Entwurf
im laufenden TUI-Prozess. Nach Erfolg öffnet `n` einen neuen leeren Entwurf.
Bei unklarem HTTP-Ergebnis bleiben Request-ID und Payload eingefroren;
`Ctrl-S` wiederholt genau diesen Request. Bei Revisionskonflikten bleibt der
Entwurf erhalten: `Ctrl-R` lädt den aktuellen Stand, übernimmt fremde Änderungen
an lokal unveränderten Feldern und behält eigene Änderungen. Der geladene
Serverwert geänderter Felder wird zur Prüfung angezeigt. Erst erneutes `Ctrl-S`
sendet mit der neuen Revision. Ticketupdates enthalten nur geänderte Felder.

Die Projektanlage registriert Metadaten; Repository-Klonen und die lokale
Runner-Zuordnung bleiben wie bisher separat. Bestehende Metadaten lassen sich
mit `e` bearbeiten. Sobald das Projekt Streams besitzt, sind Repository und
Basisbranch festgeschrieben; eine Änderung wird als Arbeitskonflikt abgewiesen.
Der Name bleibt bearbeitbar. Das vermeidet das unbeabsichtigte Umleiten bestehender
Streams auf ein anderes Repository oder eine andere Git-Basis.

CLI-Beispiel: `factory project update <ID> --expected-revision <REV> --name "Neuer Name"`.
Optional: `--repository`, `--base-branch`, `--request-id`. Profil und Metadaten
teilen dieselbe Projektrevision; parallele Änderungen werden als Konflikt sichtbar.

## Stream-Steuerung und Pausewunsch

In Ratatui: `4` → Stream auswählen → `Enter`. `Tab`/`Shift-Tab` wählt
Worktree vorbereiten, Agent starten, Pause anfordern oder Scheduling fortsetzen.
Für Agent starten den Prompt eintippen/einfügen; `Ctrl-S` sendet asynchron.
`Esc` kehrt zur Liste zurück. Entwürfe bleiben im laufenden Client erhalten.
Nach unklarem HTTP-Ergebnis wiederholt `Ctrl-S` exakt denselben Request;
nach bestätigter Speicherung oder Konflikt den Dialog neu öffnen.

CLI: `factory stream pause <ID> --expected-revision <REV>` bzw.
`factory stream resume <ID> --expected-revision <REV>`; beide unterstützen
`--request-id` für identische Wiederholung.

SQLite-Schema v4 speichert den Pausewunsch atomar mit Revision und Event.
`requested` bedeutet noch keine Pause. Der Runner prüft den Wunsch vor seinem
nächsten Agent-Schritt und vor der Fragezustellung. Ein bereits laufender Schritt
oder HTTP-Aufruf kann noch fertig werden. Neue Aufträge und neue Frageantworten
werden während des Holds abgewiesen; bereits gespeicherte Antworten bleiben erhalten.

Vor Prompt-Übergabe bestätigt der Runner `held_before_execution` (Stream `paused`).
Nur dieser Hold kann ausdrücklich freigegeben werden. Ohne aktiven Auftrag hält
Pause lediglich das Scheduling an. Bei laufender/unklarer Prompt-Zustellung
journaliert der Runner einmalig die Interrupt-Absicht und versucht OpenCodes
`interrupt?resume=false`. Auch bei HTTP-Erfolg bleibt der Zustand
`interrupt_unverified` (Stream `blocked`): Das ist kein Nachweis, dass alle
Kindprozesse beendet sind. Nach verlorenem Ergebnis oder Runnerneustart wird der
Interrupt nicht blind wiederholt. Solche Streams bleiben reserviert; Resume,
Ownership-Transfer und ein neuer schreibender Auftrag sind gesperrt.

Noch offen sind Prozesscontainment, bestätigte Pause laufender Ausführungen,
Session-Fortsetzung und ein expliziter Recovery-Workflow. Neue Factory-Versionen
auf Server und Runner gemeinsam ausrollen; alte Runner beachten Holds nicht.

F-101 liefert einen ersten ausführbaren Verbindungsdurchstich: `factoryd`, zwei
Runner und mehrere Ratatui-Clients. Der OpenCode-Adapter prüft Version 2.0.27 und
unterstützt inzwischen auch Session-Anlage, Prompt-Zustellung und Wiederabgleich.
F-102 ergänzt persistente Projekte, Tickets, Streams und Aufträge mit SQLite,
Revisionsschutz, Request-Deduplizierung und dauerhaftem Eventlog. Der erste
Runner-Auftrag prüft die OpenCode-Bereitschaft mit lokalem persistentem Journal.
F-105 ergänzt Worktrees und Ausführungsprofile; der erste F-106-Durchstich führt
schreibende Agentenaufträge aus und übernimmt nach Runner-Neustart deren Ergebnis.

## Crates

| Crate | Inhalt |
| --- | --- |
| `factory-core` | Typisierte IDs, Revision, fachliche Zustände und Heartbeat-Grenzen |
| `factory-protocol` | Versionierte JSON-Verträge; optionaler gemeinsamer HTTP-Client |
| `factory-opencode` | HTTP-Adapter für Status, Sessions, Prompt und Verlauf mit nativer Service-Anmeldung |
| `factoryd` | Axum, SQLite-Projekte/Tickets, Eventlog und Live-Runner-Registry |
| `factory-runner` | Heartbeats, Journale, Git-Worktrees und OpenCode-Ausführung/Wiederabgleich |
| `factory-tui` | Binary `factory`: Ratatui-Ansichten und Verwaltungsdialoge, CLI und JSON-Ausgabe |

## Build und Tests

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 -u scripts/smoke-foundation.py
python3 -u scripts/smoke-tui-answers.py
```

Der Prozess-Smoke-Test startet eigene Dienste auf einem freien Loopback-Port und
räumt diese wieder auf. Er prüft zwei Runner, zwei unabhängige CLI-Clients, die
echte Ratatui-Oberfläche über ein Pseudoterminal, das Beenden per `q` sowie die
Runner-Wiederanmeldung nach Server-Neustart. Die HTTP-Integrationstests prüfen
Authentifizierung, Versionskonflikte, JSON-Fehler, konkurrierende Runner-Instanzen
und abgewiesene Heartbeats mit ungültiger Registrierungs-ID.
Zusätzlich werden CLI-Mutationen, Wiederholungen nach Neustart, TUI-Projekt-/
Ticketansichten, konkurrierende SQLite-Verbindungen, Migrationen und atomarer
Rollback bei einem absichtlich fehlschlagenden Event-Insert geprüft. Die
Prozesstests verwenden eine eigene temporäre Datenbank.
Der zusätzliche Antwortdialog-Test verwendet eine echte Ratatui-PTY und echte
Factory-HTTP-/SQLite-Endpunkte mit synthetischen Fragen. Er prüft Unicode/Paste,
Formularantworten, explizite Permission-Auswahl, konkurrierende Antworten und
bedienbare Dialoge bei Serverausfall, ohne Modellaufrufe.

Optional gegen das bereits angemeldete OpenCode-Labor:

```sh
python3 -u scripts/smoke-foundation.py \
  --opencode-registration spikes/opencode/.runtime/registration.json
```

Die Foundation-Tests starten keine Agenten und nutzen keine Modellkontingente.
Der existierende OpenCode-Container wird dabei weder neu gestartet noch verändert.

Echter schreibender Agententest im bereits angemeldeten Labor (zwei Modellaufrufe):

```sh
python3 -u scripts/smoke-agent.py
```

Der Test kopiert die gebauten Factory-Binaries in den bestehenden Ubuntu-24.04-
Container und verwendet zwei Runner, temporäre Repositories und eigene Sessions.
Er prüft parallele Agenten, echte Dateiänderungen, Python-Validierung, Agentenfragen,
Permissions und einen Factory-/Runner-Neustart während laufender Shelltools.
Die Antworten auf Agentenfragen werden bei gestoppten Runnern gespeichert und
erst nach einem weiteren Factory-Neustart zugestellt. Konto und Lab-Volume bleiben
erhalten. Python 3 ist im aktuellen Lab-Dockerfile enthalten; ältere behaltene
Container benötigen das Paket ebenfalls. Der Test ist kein Remote-SSH-Nachweis.

## Manuell ausprobieren

Für diese Entwicklungsstufe verwenden Server, Runner und TUI denselben zufälligen
Token. Folgendes erzeugt einmalig eine nur für den Benutzer lesbare lokale Datei;
eine vorhandene Datei wird nicht überschrieben:

```sh
python3 - <<'PY'
import os, secrets
fd = os.open('.factory-dev.env', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, 'w') as f:
    f.write('export FACTORY_TOKEN=' + secrets.token_hex(32) + '\n')
PY
```

Terminal 1, aus dem Repository:

```sh
source .factory-dev.env
target/debug/factoryd
```

Standard-Datenbank: `.factory-data/factory.sqlite3` (von Git ausgeschlossen).
`--database /pfad/factory.sqlite3` wählt einen anderen Speicherort. Beim Start
werden Migrationen ausgeführt, bevor der Dienst seinen Listener bereit meldet.
Ein neueres, nicht unterstütztes Datenbankschema führt zum Startfehler.

Terminal 2:

```sh
source .factory-dev.env
target/debug/factory-runner \
  --id 78d15fe6-56c2-47bd-b367-14833c384a71 \
  --name Laptop --location local \
  --opencode-registration "$HOME/.local/state/opencode/service.json"
```

Für das isolierte Labor stattdessen dessen `.runtime/registration.json` angeben.
Ohne `--opencode-registration` bleibt der Runner verbunden und meldet OpenCode
als `not_configured`. Eine fehlende, ungültige oder nicht erreichbare Registrierung
führt zu `unavailable`; eine andere OpenCode-Version zu `unsupported`.
Die Datei wird bei jedem Heartbeat neu gelesen, damit geänderte Endpunkte auffallen.

Terminal 3 und beliebig weitere:

```sh
source .factory-dev.env
target/debug/factory
```

`q`, Esc oder Ctrl+C beendet die Oberfläche; die Runner bleiben eigenständige
Prozesse. Ohne interaktive Oberfläche:

```sh
target/debug/factory --snapshot
```

In der TUI wechselt `1` zu Runnern, `2` zu Projekten, `3` zu Tickets. Links/rechts
wählt das Projekt für die Ticketansicht, hoch/runter scrollt die Liste. Mehrere
TUIs sehen Änderungen beim nächsten Snapshot-Abruf (ungefähr eine Sekunde).

## Projekte und Tickets verwalten

In einem Terminal mit geladenem `FACTORY_TOKEN`:

```sh
target/debug/factory project create \
  --name yoloforge --repository git@github.com:farion/yoloforge.git
target/debug/factory project list
```

`project create` speichert Metadaten; es klont noch kein Repository und prüft
keinen Hosting-Zugang. Die zurückgegebene Projekt-ID für Tickets verwenden:

```sh
target/debug/factory ticket create \
  --project <PROJEKT-ID> --title 'SSH-Verbindungsprofil einrichten' \
  --description 'Host, Benutzer und Schlüssel im Client auswählen' \
  --acceptance-criteria 'Die Verbindung über einen vorhandenen SSH-Tunnel funktioniert' \
  --priority 2
target/debug/factory ticket list --project <PROJEKT-ID>
target/debug/factory ticket update <TICKET-ID> --expected-revision 1 --status ready
```

Priorität: 0 niedrig, 1 normal, 2 hoch, 3 dringend. Neue Tickets beginnen in
`backlog`. Statuswerte: `backlog`, `ready`, `in_progress`, `blocked`, `review`,
`done`, `cancelled`. Statusänderungen sind derzeit manuelle Ticketpflege und
starten keine Agenten. Update-Optionen umfassen auch Titel, Beschreibung,
Akzeptanzkriterien und Priorität; ausgelassene Felder bleiben erhalten.

Jede Mutation gibt ihre Request-ID auf stderr aus. Bei unklarer Antwort dieselbe
Operation mit denselben Argumenten und `--request-id <ID>` erneut senden.
Eine erfolgreiche Wiederholung liefert das ursprüngliche Ergebnis zurück und
erzeugt kein zusätzliches Ereignis – auch nach Server-Neustart. Diese Antwort
kann älter als der inzwischen weiterbearbeitete Ticketstand sein. Dieselbe ID
mit anderer Payload führt zu HTTP 409 `request_conflict`.

Ein Update mit veralteter `--expected-revision` führt zu HTTP 409
`revision_conflict` und liefert die aktuelle Revision. Zustand neu laden und
bewusst zusammenführen; die CLI überschreibt den Konflikt nicht automatisch.

```sh
target/debug/factory events --after 0
```

Dies liefert eine JSON-Seite mit höchstens 100 Ereignissen und `next_cursor`.
Mit dem zurückgegebenen Cursor weitere Seiten abrufen. Die HTTP-Route akzeptiert
zusätzlich `limit` von 1 bis 500. Das ist noch kein SSE-Abonnement.

## Streams und Runner-Aufträge

```sh
target/debug/factory stream create --project <PROJEKT-ID> \
  --goal 'Aufgabe umsetzen' --runner <RUNNER-ID> --ticket <TICKET-ID>
target/debug/factory stream list
target/debug/factory stream check <STREAM-ID> --expected-revision 1
target/debug/factory commands --stream <STREAM-ID>
```

`--ticket` kann mehrfach angegeben werden. Tickets müssen zum Projekt gehören
und können derzeit nur einem Stream zugeordnet sein. Zuordnung und Runner sind
in dieser Version unveränderlich. Offline-Runner dürfen als Ziel eingetragen
werden; ihre Aufträge warten persistent auf die Wiederverbindung.

Die TUI zeigt mit `4` Streams und mit `5` Aufträge des ausgewählten Projekts.
`stream prepare` legt den Branch `factory/<STREAM-ID>` und einen Worktree auf
dem zugeordneten Runner an (siehe unten). Eine Agentensession startet es noch nicht.

`stream check` reiht `check_open_code` ein. Pro Stream ist höchstens ein Auftrag
offen. Zustände: `pending` → `accepted` → `succeeded` oder `failed`. Ohne
konfigurierten OpenCode-Dienst schlägt die Prüfung mit `not_configured` fehl.
Der Stream wird bei Erfolg `idle`, andernfalls `blocked`; Tickets werden durch
diese Diagnose nicht automatisch abgeschlossen. Jede Stream-Zustandsänderung
erhöht seine Revision: vor einem weiteren Auftrag den aktuellen Stand laden.

Runner speichern Empfang, stabile Request-IDs und Ergebnisse standardmäßig in
`.factory-data/runner-<RUNNER-ID>.sqlite3`; `--journal <PFAD>` überschreibt dies.
Ein Dateilock verhindert parallele Nutzung desselben Journals. Das Journal ist
an Server-URL und Runner-ID gebunden. Bei Neustarts denselben Pfad verwenden.
Persistierte Ergebnisse werden erneut gemeldet, bis der Server sie bestätigt.
Die schreibgeschützte Diagnose darf nach einem Absturz vor Ergebnis-Commit
wiederholt werden. Das ist keine Exactly-once-Garantie für externe Seiteneffekte;
Agentenaufträge verwenden den unten beschriebenen `unknown`-Abgleich.

Aufträge werden im Heartbeat-Zyklus abgefragt, ein Auftragsschritt pro Zyklus.
Offene Aufträge werden innerhalb der ersten 16 Queue-Einträge reihum verarbeitet,
damit mehrere externe Agenten parallel laufen können. Polling, Ack und Ergebnis benötigen die aktuelle Runner-Registrierung;
zusätzlich prüft die Datenbank Runner-Zuordnung und Ownership-Epoch. Ein
Verbindungsabbruch überträgt keinen Stream an einen anderen Runner.

## Lokale Git-Worktrees pro Stream

Auf jedem Ausführungsort werden vorhandene Git-Repositories explizit den
Factory-Projekten zugeordnet. Eine JSON-Datei, etwa `repositories.json`:

```json
{
  "<PROJEKT-ID>": "/home/USER/src/yoloforge"
}
```

Den Runner mit der bestehenden ID und seinem bestehenden Journal starten:

```sh
target/debug/factory-runner --id <RUNNER-ID> --name laptop \
  --repositories /pfad/repositories.json \
  --workspace-root /home/USER/factory-workspaces
```

Der Runner liest die Zuordnungsdatei bei jeder Vorbereitung neu. Standardmäßig
liegen die Worktrees in `workspaces-<RUNNER-ID>/<STREAM-ID>` neben dem Journal.
Mit `--workspace-root` liegen sie direkt unter `<ROOT>/<STREAM-ID>`. Ein bereits
geplanter Stream behält Repository, Root, Branch und Ausgangs-Commit; geänderte
Zuordnungen führen zum Fehler statt zur stillen Verschiebung.

```sh
target/debug/factory stream list
target/debug/factory stream prepare <STREAM-ID> --expected-revision <AKTUELLE-REVISION>
target/debug/factory commands --stream <STREAM-ID>
```

Der erfolgreiche Auftrag enthält `result.workspace` mit absolutem Runner-Pfad,
Branch und `base_commit`. Das ist der Ausgangs-Commit, kein laufend aktualisierter
HEAD. Auch die Auftragsansicht der TUI zeigt den Pfad. Der lokale Basisbranch
aus den Projektmetadaten muss bereits existieren. Clone/Fetch und Git-Anmeldung
werden derzeit extern erledigt; das Repository-Feld des Projekts wird nicht als
lokaler Pfad interpretiert. Änderungen im ursprünglichen Checkout werden nicht
in den neuen Worktree übernommen oder überschrieben.

Vor Git-Schreiboperationen speichert der Runner einen Plan in Journal-Schema v2.
Die Migration erhält alte Diagnose-Aufträge und unbestätigte Ergebnisse.
Nach Unterbrechung gleicht er Pfad, Git-Repository, Branch und Checkout mit dem
Plan ab. Ein sauber angelegter Worktree kann übernommen werden. Ein bereits
bestätigter Worktree darf Änderungen enthalten; Wiederholung setzt diese nicht
zurück. Bestehende fremde Pfade/Branches, verschwundene bestätigte Worktrees und
uneindeutige teilweise Checkouts führen zu `workspace_error` und Stream `blocked`.
Es gibt kein automatisches `reset`, `clean`, Löschen oder Überschreiben.

Git läuft ohne Shell und ohne Hooks; einzelne Aufrufe haben fünf Sekunden Timeout.
Bei Fehlern bleiben Branch, Pfad und Plan zur Untersuchung erhalten. Nach der
Korrektur einen neuen `stream prepare`-Auftrag mit aktueller Revision senden.
Die Wiederholung derselben Request-ID liefert weiterhin das ursprüngliche
Auftragsergebnis. Große Checkouts sowie projektspezifische Filter/Submodule sind
noch kein verifizierter Ausführungspfad.

## Ausführungsprofile und schreibende Agenten

Das Projektprofil enthält Toolchain-Hinweise, Start-/Testbefehle und das am
Ausführungsort verfügbare Modell. `configure` ersetzt das gesamte Profil mit
Revisionsschutz; ausgelassene Textoptionen werden leer:

```sh
target/debug/factory project configure <PROJEKT-ID> --expected-revision 1 \
  --toolchain 'Rust stable' --test-command 'cargo test --workspace --locked' \
  --start-command 'cargo run -p factoryd' --provider openai --model gpt-6-astra
```

Der Runner benötigt zusätzlich zur Repository-Zuordnung
`--opencode-registration /pfad/service-registration.json`. OpenCode und Runner
müssen denselben absoluten Worktree-Pfad sehen. Ein Host-Runner kann deshalb nicht
einfach den isolierten Lab-Container als Ausführungsdienst für Host-Pfade verwenden.
Die Provider-Anmeldung bleibt beim jeweiligen OpenCode-Service.

Nach erfolgreichem `stream prepare`:

```sh
target/debug/factory stream list
target/debug/factory stream run <STREAM-ID> --expected-revision <AKTUELLE-REVISION> \
  --prompt 'Implementiere die Aufgabe im Worktree, führe die Tests aus und berichte die Ergebnisse.'
target/debug/factory commands --stream <STREAM-ID>
```

Ein Lauf erhält eine eigene OpenCode-Session und einen unveränderlichen Snapshot
des Prompts und Projektprofils. Profile sind Kontext für den Agenten; der Runner
führt deren Shellbefehle nicht separat aus. Ticketinhalte und Projektwissen werden
noch nicht automatisch als Kontext zusammengestellt. OpenCode übernimmt seine
bestehenden Permissions; Factory setzt keine pauschale Freigabe.

Die TUI-Auftragsansicht (`5`) zeigt `starting`, `running`, `waiting_input`,
`unknown` oder das Endergebnis. CLI-JSON enthält Session-ID, Nachrichten-ID und
die Abschlusszusammenfassung. Erfolg setzt den Stream auf `review`; Tickets
werden nicht automatisch auf `done` gesetzt. Ein nächster `stream run` erstellt
eine neue Session im vorhandenen Worktree mit dessen bisherigen Änderungen.

### Wiederanlauf und unklarer Ausgang

- Session- und Nachrichten-ID werden deterministisch aus der Command-ID gebildet.
- Der Runner journalisiert die Absicht **vor** Session-Anlage und Prompt-Zustellung.
- Nach unklarer HTTP-Antwort oder Neustart liest er Session, Inbox, exakte Nachricht,
  offene Eingaben, Aktivität und letzten Assistentenbeitrag. Er prüft dabei auch
  Session-Metadaten, Ort, Modell und Promptinhalt.
- Fehlende oder widersprüchliche Daten führen zu `unknown`. Der Auftrag bleibt
  `accepted` und reserviert den Stream; weder Prompt noch Session werden blind neu
  angelegt. Später sichtbare Ergebnisse können den Zustand wieder auflösen.
- Eine inaktive Session allein gilt nicht als Erfolg. Bestätigter Abschluss
  erfordert Session-Erfolg und einen abgeschlossenen Assistentenbeitrag mit `stop`.
- Unterbrochene Ausführung bleibt `unknown`, weil die Stilllegung aller Prozesse
   damit noch nicht verifiziert ist. Der Factory-Pause-Dialog zeigt dies ausdrücklich
   als unbestätigten Interrupt; nur Pre-Execution-Holds lassen sich freigeben.
- `waiting_input` nennt die Zahl offener Fragen/Permissions und die Session-ID.
  Die Fragen werden zusätzlich mit Schema und Herkunft dauerhaft gespeichert;
  Antworten lassen sich über die Factory-CLI zustellen (siehe unten).

Der Runner beendet beim eigenen Herunterfahren keine OpenCode-Session. Ein neuer
Runner mit demselben Journal kann sie nach gültiger Registrierung weiter beobachten.
Nach verlorenen Daten oder entferntem Service wird kein neuer Lauf gestartet, um
einen möglicherweise noch laufenden alten Auftrag zu ersetzen. `unknown` ist
derzeit bewusst kein automatisch auflösbarer Fehlerstatus.

## Agentenfragen und Permissions beantworten

`6` öffnet die projektbezogene Fragenliste in Ratatui. Hoch/runter wählt die oben
hervorgehobene Frage; `Enter` öffnet den Antwortdialog. Im oberen Bereich stehen
Herkunft und vollständiges Schema, bei Permissions auch `action`, `resources`
und ggf. `source`. `Bild auf/ab` scrollt diese Details.

| Taste | Antwortdialog |
| --- | --- |
| `Tab` / `Shift-Tab` | Nächstes / vorheriges sichtbares Formularfeld |
| Text / Terminal-Paste | Text, Zahlen oder JSON-Mehrfachauswahl bearbeiten |
| Links/rechts, Home/End, Backspace/Delete | Cursor bewegen und Text bearbeiten |
| Enter | Zeilenumbruch in Freitextfeldern; kein Absenden |
| Hoch/runter | Angebotenen Wert eines Text-Auswahlfelds wählen |
| Leertaste | Boolean-Feld umschalten |
| `1` / `2` / `3` im Permission-Dialog | `once` / `always` / `reject` auswählen |
| `Ctrl-S` | Antwort absenden; nach unklarer Zustellung exakt denselben Request wiederholen |
| `Ctrl-R` | Aktuelle Revision ausdrücklich laden; vorhandenen Entwurf übernehmen |
| `Esc` | Zur Fragenliste; Entwurf bleibt während dieses TUI-Prozesses erhalten |
| `Ctrl-C` | TUI beenden |

Text, Boolean, Integer, Number und Multiselect werden unterstützt. Mehrfachauswahl
wird als JSON-Liste eingegeben, etwa `["option1","option2"]`; gültige Werte stehen
im Schema. Defaults, versteckte Felder und `when`-Bedingungen werden berücksichtigt.
Typen, Optionen sowie einfache Größen-/Zahlengrenzen werden lokal geprüft;
Format-/Pattern-Prüfungen und die endgültige Validierung übernimmt OpenCode.
Externe Link-Felder werden angezeigt und nicht automatisch geöffnet oder beantwortet.

Während des HTTP-Aufrufs bleibt die Oberfläche bedienbar. Bei unklarer Antwort
bleiben Payload, Revision und Request-ID unverändert; Bearbeitung ist bis zum
Abgleich gesperrt. Ein Snapshot mit derselben `answer_request` bestätigt die
Speicherung auch bei verlorener HTTP-Antwort. Konflikte überschreiben keine
fremden Antworten: Der Entwurf bleibt erhalten, `Ctrl-R` ist ein bewusster
Neuabgleich. Entwürfe sind derzeit nur im Speicher des TUI-Prozesses, nicht über
einen Client-Neustart hinweg gespeichert. Die eingeblendete Request-ID kann
auch für eine Wiederholung derselben Antwort per CLI verwendet werden.

Alternativ liefert und bearbeitet die CLI dieselben Fragen:

```sh
target/debug/factory question list
target/debug/factory question list --command <COMMAND-ID>
target/debug/factory question answer <FRAGE-ID> --expected-revision 1 \
  --answer-json '{"choice":"meine Antwort"}'
target/debug/factory question answer <PERMISSION-FRAGE-ID> --expected-revision 1 \
  --decision once
```

Die JSON-Schlüssel müssen den `key`-Werten des Formularschemas entsprechen.
Für Permissions sind `once`, `always` und `reject` erlaubt. `always` verwendet
OpenCodes dauerhafte Freigabesemantik; Factory entscheidet dies nicht selbst.
Wie andere Mutationen unterstützen Antworten `--request-id` für Wiederholungen.
Ein konkurrierender Client kann eine bereits eingereihte Antwort nicht unbemerkt
ersetzen; veraltete Revisionen ergeben HTTP 409.

Fragenzustände:

| Zustand | Bedeutung |
| --- | --- |
| `pending` | Offene, beobachtete Frage; kann beantwortet werden |
| `answer_queued` | Antwort dauerhaft gespeichert, noch nicht bestätigt |
| `answered` | OpenCode hat die Antwort bestätigt oder der Formularzustand belegt exakt diese Antwort |
| `rejected` | OpenCode hat die Eingabe mit HTTP 400 zurückgewiesen; korrigierte Antwort mit neuer Request-ID/Revision möglich |
| `unknown` | Frage verschwunden/verändert, Lauf unklar oder Zustellung nicht nachweisbar |

Der Runner prüft vor der Zustellung erneut den exakten Frageinhalt und einen
aktiven OpenCode-Lauf. Er speichert eine Zustellabsicht im lokalen Journal,
bevor er die Antwort sendet. Bei unklarer HTTP-Antwort erfolgt kein zweiter POST.
Ein Formular kann über seinen Detailzustand und die exakt gespeicherte Antwort
nachträglich bestätigt werden. Eine verschwundene Permission beweist keine
erfolgte Freigabe und bleibt bei unbestätigter Zustellung `unknown`.

Ein erfolgreicher leerer Fragenabgleich markiert verschwundene offene Fragen als
`unknown`; Netzwerkfehler gelten nicht als leerer Abgleich. Alte Fragen bleiben
im Snapshot/Eventlog erhalten und werden nicht neu angelegt oder automatisch
wieder freigegeben. Die API synchronisiert höchstens 64 Fragen / 128 KiB pro
Auftrag und akzeptiert Antwortobjekte bis 32 KiB. Android-Antwortdialoge folgen;
TUI und CLI verwenden denselben revisionsgeschützten Antwortvertrag.

## Persistenzvertrag

- SQLite im WAL-Modus mit `synchronous=FULL`, Foreign Keys und Server-Schema-Version 5;
  bestehende v1/v2-Datenbanken werden automatisch migriert. Das Runner-Journal
  bleibt bei Schema v2 und ergänzt Antwortbelege im bestehenden JSON-Datensatz.
- Entity-Änderung, Ereignis und erfolgreiche Request-Antwort werden in einer
  `BEGIN IMMEDIATE`-Transaktion gespeichert. Ein Fehler rollt alles zurück.
- Snapshot-Projekte, Tickets und `event_cursor` stammen aus derselben
  SQLite-Lesetransaktion. Runner-Verbindungsdaten bleiben separate Live-Daten.
- Fehlgeschlagene Requests werden nicht als erfolgreiche Deduplizierungsantwort
  gespeichert. Erfolgreiche Requests und Events werden derzeit nicht gekürzt.
- SQLite-Arbeit läuft über `spawn_blocking`, nicht auf den async Netzwerk-Tasks.
- Für Sicherungen den Dienst beenden und die Datenbank sichern, oder ein
  konsistentes SQLite-Backup verwenden; im laufenden WAL-Betrieb ist das bloße
  Kopieren nur der Hauptdatei kein vollständiges Backup.

Ein zweiter Runner benötigt eine andere `--id`, etwa
`28a12d68-a711-4533-97bc-d7d7c0e96d03`, und kann mit `--location server` beschriftet
werden. Das setzt nur die Ortsmetadaten, es startet keinen Remote-Prozess.

## Verhalten und API

- Standard: `127.0.0.1:7432`; der Foundation-Dienst akzeptiert nur Loopback-Bindung.
- `FACTORY_TOKEN` ist obligatorisch; Header: `Authorization: Bearer …`.
- Alle Routen verlangen `x-factory-protocol: 1`; andere Versionen erhalten HTTP 426.
- `GET /v1/snapshot` liefert Boot-ID, Live-Revision, Runner, Projekte, Tickets und Event-Cursor.
- `POST /v1/projects`, `POST /v1/tickets`, `PATCH /v1/tickets/{id}` speichern Metadaten.
- `GET /v1/events?after=…&limit=…` liefert paginierte dauerhafte Ereignisse als JSON.
- `POST /v1/runners/register` registriert stabile Runner-ID plus Prozess-Instance-ID.
- `POST /v1/runners/{id}/heartbeat` verlangt die aktuelle Registrierungs-ID.
- Zwei lebende Prozesse können nicht gleichzeitig dieselbe Runner-ID beanspruchen.
- Heartbeat alle zehn Sekunden; nach 30 Sekunden `suspect`, nach 60 `unreachable`.
- Server-Neustart leert die Live-Registry. Ein Runner erhält beim nächsten Heartbeat
  HTTP 404 und registriert sich neu. Seine konfigurierte Runner-ID bleibt gleich.
- Eine fremde/veraltete Registrierungs-ID ergibt HTTP 409; der Runner beendet sich,
  statt die Identität eines anderen laufenden Prozesses zu übernehmen.
- Die TUI lädt jede Sekunde einen Snapshot. Bei Verbindungsausfall kennzeichnet sie
  die letzte Ansicht als veraltet; Netzwerkzugriffe blockieren nicht die Eingabe.

Für einen tatsächlichen Remote-Host wird der SSH-Tunnel vorerst extern aufgebaut;
`--server http://127.0.0.1:<lokaler-tunnelport>` wählt ihn im Client/Runner aus.
Es gibt noch kein produktives Container-Deployment für `factoryd`.

## Bewusste Grenze dieses Durchstichs

Die Registry enthält nur Live-Verbindungsdaten im Arbeitsspeicher. Ihre Revision
ist kein dauerhafter Event-Cursor. Registrierung überträgt keinerlei Workspace-
Schreibrecht, auch nicht wenn eine alte Runner-Verbindung abgelaufen ist.

SSE-Abonnement, Android-Client, bestätigte Pause, Ownership-Transfer
und pro Gerät eingeschränkte Credentials folgen. Projektanlage, Profilkonfiguration,
Ticketänderungen und Streamanlage sind auch direkt in Ratatui möglich.
Der gemeinsame Entwicklungs-Token ist kein implementiertes Geräte-Enrollment.
Schreibende Factory-Agenten sind im Integrationslabor geprüft; ein produktiver
Remote-SSH-/Container-Ausführungspfad bleibt offen.

Für einen sofortigen Runner-Prozessneustart denselben Prozess nicht doppelt starten:
die alte Live-Registrierung bleibt bis zum Timeout aktiv. Ein kontrolliertes
Deregistrieren mit Prozess-Reconciliation wird mit dem Runner-Lifecycle ergänzt.

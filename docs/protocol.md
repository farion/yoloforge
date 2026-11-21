# Datenmodell und Runner-Protokoll

## Gespeicherte Sessionauszüge (Schema v5)

- `GET /v1/commands/{id}/excerpt`: letzter `SessionExcerpt` oder `null`;
  unbekannter Auftrag liefert 404. Normale Factory-Authentifizierung ist erforderlich.
- `PUT /v1/runners/{runner}/commands/{command}/excerpt`:
  `{ownership_epoch, excerpt}` mit aktueller Runner-Registrierung.
- `excerpt`: `{session_id, revision, captured_unix_ms, older_omitted, messages}`.
  Nachrichten enthalten `{id, kind, text, truncated}` in Reihenfolge alt → neu.

Runner, Ownership-Epoche, deterministische Session-ID und Auftragsart werden
geprüft. Bis zu 32 eindeutige Nachrichten, 8192 UTF-8-Bytes Text pro Nachricht
und insgesamt 131072 Textbytes sind erlaubt. Identische Wiederholungen sind
idempotent; ältere Revisionen und veränderte Payloads unter derselben Revision
werden abgewiesen. Quellrevision und Sample werden vor Versand journalisiert.
Ein finaler Auszug wird vor dem Ergebnis-Report geliefert und bei Wiederanlauf
aus derselben Outbox erneut angeboten. Das Sample ist kein Abschlussnachweis.

Die Tabelle `excerpts` speichert nur den neuesten begrenzten Auszug je Auftrag;
es entstehen weder große Snapshot-Payloads noch duplizierte Textinhalte im
Factory-Eventlog. Fehlgeschlagene Quellreads ersetzen keine gespeicherten Daten.
OpenCode-Antworten über 8 MiB werden verworfen; Abrufzeit und Kürzungsmarker
machen Staleness beziehungsweise unvollständige Darstellung sichtbar.

## Projekt-Metadaten bearbeiten

`PATCH /v1/projects/{id}` akzeptiert `{request_id, expected_revision, name?,
repository?, base_branch?}`. Mindestens ein Feld ist erforderlich; dieselben
Text-/Repository-URL-Regeln wie bei der Anlage gelten. Profil und Metadaten
verwenden eine gemeinsame Projektrevision. Mutation, `project.updated`-Event
und Request-Deduplizierung werden atomar gespeichert.

Repository und Basisbranch dürfen nur geändert werden, solange keine Streams
zum Projekt existieren. Die Prüfung ist mit der Mutation transaktional serialisiert;
ab der Streamanlage liefert eine Änderung `work_conflict`. Name und Profil bleiben
bearbeitbar. Bereits gespeicherte Aufträge und Workspace-Spezifikationen werden
durch Projektupdates nicht verändert.

## Persistente Stream-Holds (Schema v4)

- `POST /v1/streams/{id}/control`: `{request_id, expected_revision, action}`,
  Aktion `pause` oder `resume`; erfolgreiche Requests werden dedupliziert.
- `GET /v1/runners/{runner}/commands/{command}/control?epoch=1`:
  aktueller Hold oder `null`, mit aktueller Runner-Registrierung und Ownership-Prüfung.
- `POST` auf denselben Runner-Pfad: `{control_request, ownership_epoch, state, summary}`.
  Veraltete Control-IDs und fremde Runner werden abgewiesen. Identische Reports
  ändern weder Revision noch Eventlog.
- Snapshot-Feld `controls` und Event `stream.control` enthalten die Zustände
  `requested`, `held_before_execution`, `interrupt_unverified`, `released`.

Ein Hold blockiert neue Aufträge und neue Frageantworten. `requested` wird als
`pausing`, bestätigter Pre-Execution-Hold als `paused`, unbestätigter Interrupt als
`blocked` angezeigt. Nur `held_before_execution` darf freigegeben werden.
Laufende Netzwerkaufrufe sind nicht atomar mit einem späteren Pausewunsch;
eine angenommene Pause ist deshalb noch kein Stilllegungsnachweis. Der Runner
journaliert Interrupt-Absicht vor dem externen POST, wiederholt ihn bei unklarem
Ausgang nicht und bestätigt damit niemals Prozessruhe. Server und Runner müssen
gemeinsam aktualisiert werden, da ältere Runner den zusätzlichen Vertrag nicht kennen.

Entwurf für die eigene Factory-API, keine Beschreibung bestehender OpenCode-Routen.

## Aktuell implementierter Ausschnitt (F-101 / F-102 / F-105 / Teile F-104 und F-106)

`factory-protocol` implementiert Snapshot, Runner-Registrierung, Heartbeat,
Projekt-/Ticket-Mutationen, Eventseiten und typisierte Fehler. Header
`x-factory-protocol: 1` plus gemeinsamer Entwicklungs-Bearer-Token sind erforderlich.
`/v1/snapshot` enthält Boot-ID, Live-Revision, Runner sowie persistente Projekte,
Tickets und `event_cursor`. Die drei letzten Felder stammen aus einer SQLite-
Lesetransaktion. Runner-Revision und dauerhafter Event-Cursor sind unabhängig.
Die TUI pollt weiterhin; `/v1/events` liefert aktuell paginiertes JSON, kein SSE.

Snapshot enthält außerdem `streams` und `commands` aus derselben dauerhaften
Lesetransaktion. Implementierte Work-Routen:

- `POST /v1/streams`: Projekt, Ziel, Runner-ID und Ticket-IDs; feste Ownership-Epoch 1.
- `POST /v1/streams/{id}/commands`: Request-ID, erwartete Stream-Revision und
  `kind: "check_open_code"`, `"prepare_workspace"` oder `"run_agent"`;
  Letzteres benötigt `prompt` und einen zuvor erfolgreich vorbereiteten Workspace.
- `GET /v1/runners/{id}/commands`: maximal 16 offene Aufträge (`pending`/`accepted`).
- `POST /v1/runners/{runner}/commands/{command}/ack`: Request-ID und Ownership-Epoch.
- `POST /v1/runners/{runner}/commands/{command}/result`: zusätzlich typspezifisches Ergebnis.
- `POST /v1/runners/{runner}/commands/{command}/observe`: Ownership-Epoch und
  `progress` für laufende Agenten; identische Beobachtungen erzeugen kein neues Event.
- `PATCH /v1/projects/{id}/profile`: Request-ID, erwartete Projektrevision und
  vollständiges Ausführungsprofil (Toolchain, Start-/Testbefehl, Provider und Modell).
- `POST /v1/runners/{runner}/commands/{command}/questions`: Ownership-Epoch und
  vollständige beobachtete Fragenliste; liefert die dauerhaften Factory-Fragen
  dieses Commands inklusive eingereihter Antworten zurück.
- `POST /v1/questions/{id}/answer`: Request-ID, erwartete Fragenrevision und
  `answer` (Formular-Feldobjekt oder `{"decision":"once|always|reject"}`).
- `POST /v1/runners/{runner}/questions/{id}/delivery`: Request-ID, Ownership-Epoch,
  `answer_request` und bestätigter/ungeklärter/zurückgewiesener Zustellstatus.

Die Runner-Auftragsrouten benötigen `x-factory-runner-registration` mit aktueller
Registrierungs-ID. Zuordnung und Epoch werden bei Ack/Result geprüft. Dies
ergänzt den gemeinsamen Entwicklungstoken, ersetzt noch keine Geräte-Credentials.
Stream- und Command-Änderungen erzeugen atomare Ereignisse. Diagnose-Erfolg setzt
den Stream auf `idle`, Fehlschlag auf `blocked`; Ticketstatus bleibt unverändert.
Runner journalisieren vor Ack, speichern Ergebnisse vor Übertragung und melden
unbestätigte Ergebnisse erneut. Nur die read-only Diagnose darf nach unklarem
lokalem Ausgang erneut ausgeführt werden. Ownership-Transfer ist noch nicht verfügbar.

`prepare_workspace` erhält beim Einreihen eine unveränderliche `workspace_spec`
mit Projekt-ID, Stream-Branch und Basisbranch. Lokale Repository-Pfade stammen
aus der Runner-Konfiguration. Das Ergebnis ist entweder
`{"workspace":{"path":"/…","branch":"factory/…","base_commit":"…"}}`
oder `{"workspace_error":"…"}`. Der Server prüft Ergebnisart und Branch gegen
den Auftrag. Diagnose-Ergebnisse behalten ihr bisheriges `{"status":"…"}`-Format;
alte persistierte Aufträge ohne `workspace_spec` bleiben lesbar. Alle Prozesse
sollten gemeinsam aktualisiert werden, bevor die neue Auftragsart verwendet wird.

Workspace-Anlage verwendet ein eigenes persistentes Plan-/Abgleichverfahren im
Runner-Journal. Ein wiedergefundener vollständiger Checkout wird übernommen;
mehrdeutige Zustände werden nicht automatisch bereinigt.

Agentenaufträge enthalten zusätzlich `agent_spec` (Prompt und unveränderliches
Projektprofil). `agent_progress` enthält die deterministische Session-/Nachrichten-ID,
Phase und Zusammenfassung. Die Phasen `starting`, `running`, `waiting_input` und
`unknown` belassen den Command auf `accepted`; damit greift weiterhin der eindeutige
Index für einen offenen Auftrag pro Stream. Der Stream zeigt dabei `running`,
`waiting_input` beziehungsweise `blocked`. `unknown` ist keine Freigabe zum erneuten
Starten. Nur `result.agent` mit `succeeded` oder `failed` beendet den Auftrag.
Agentenerfolg setzt den Stream auf `review`, nicht auf `idle` oder das Ticket auf `done`.

Der Runner speichert Session-/Prompt-Absichten vor dem externen API-Aufruf. Nach
Verbindungsverlust liest er die bekannte Session und Nachrichten-ID, statt erneut
zu senden. Fehlende Identitäten, abweichender Prompt oder unbestätigte Inaktivität
bleiben `unknown`. Fortschritte und Ergebnisse werden atomar als Factory-Events
gespeichert. Das ist eine konservative Reconciliation, keine Exactly-once-Garantie
für beliebige Agenten-Tools oder abgekoppelte Prozesse.

Server-Schema v3 ergänzt `questions` mit eindeutiger Kombination aus Command,
Fragenart und externer ID. Snapshot enthält `questions` aus derselben dauerhaften
Lesetransaktion. Schema/Inhalt, Session-ID, Revision, Antwort und deren Request-ID
werden gespeichert. Ein erfolgreicher Abgleich markiert verschwundene oder unter
derselben ID veränderte Fragen als `unknown`; er ersetzt nicht ihre Provenienz.
Unveränderte Abgleiche erzeugen keine weiteren Events. Antworten, Revision und
Request-Antwort werden atomar gespeichert. Pro Frage ist nur eine Antwort gleichzeitig
eingereiht. Eine definitive Eingabeablehnung (`rejected`) erlaubt einen neuen Versuch.

Runner-Zustellabsichten werden vor dem OpenCode-POST gespeichert. Fehlende HTTP-
Bestätigung löst nur lesenden Abgleich aus, niemals automatisches Wiederholen.
Formularantworten können nachträglich durch identisches Schema plus bestätigten
Antwortinhalt belegt werden; entfernte Permissions bleiben ohne positive Bestätigung
ungeklärt. Frage und Antwort bleiben nach Abschluss des Commands für den Audit erhalten.

Alle Projekt-/Ticket-Mutationen benötigen eine Request-ID; Ticket-PATCH außerdem
`expected_revision`. Änderbare Felder sind optional. Erfolgreiche Mutation,
Event und Request-Antwort werden atomar gespeichert. Wiederholungen liefern die
ursprüngliche Antwort. Andere Payload unter derselben ID ergibt `request_conflict`,
veraltete Revision `revision_conflict` mit `current_revision` (jeweils HTTP 409).

Registrierung verwendet eine stabile Runner-ID, eine Prozess-Instance-ID und eine
serverseitige Registrierungs-ID für Heartbeats. Dies ist Verbindungsverwaltung,
kein Workspace-Lease und kein Ersatz für Ownership-Epochen. Zwei lebende Instanzen
derselben ID erhalten einen Konflikt. Nach Server-Neustart registriert sich ein
Runner erneut. Details: [Entwicklungsanleitung](development.md).

## Minimales Datenmodell

Alle veränderlichen Aggregate erhalten ID, Revision und Zeitstempel.

| Entität | Wichtige Felder |
| --- | --- |
| Project | Name, Repository, Basisbranch, Umgebungsprofil, Regeln |
| Ticket | Project-ID, Titel, Beschreibung, Akzeptanzkriterien, Priorität, Status |
| TicketDependency | Ticket-ID, benötigte Ticket-ID |
| Stream | Project-ID, Ziel, Branch, Status, Runner-ID, Ownership-Epoch |
| StreamTicket | Stream-ID, Ticket-ID, Reihenfolge |
| Runner | Name, Ort, Fähigkeiten, Version, letzter Heartbeat, Verbindungszustand |
| Workspace | Stream-ID, Runner-ID, Pfad, HEAD, Dirty-Zustand |
| Session | Stream-ID, Runner-ID, OpenCode-ID, Generation, Zustand |
| Run | Stream-ID, Session-ID, Command-ID, Start, Ende, Ergebnis |
| Question | Session-ID, externe Referenz, Art, Inhalt, Zustand, Antwort, Bearbeiter |
| Checkpoint | Stream-ID, Commit-ID, Kontextpaket, bestätigte Verfügbarkeit |
| TestResult | Stream-ID, Checkpoint/Snapshot-ID, Befehl, Umgebung, Exitcode, Logreferenz |
| Command | ID, Request-ID, Ziel, Epoch, Art, Payload, Ausführungszustand |
| Event | globale Sequenz, Typ, Aggregate-ID, Revision, Payload, Zeitpunkt |
| Device | Name, Token-Hash, Widerrufsstatus |
| DeviceCredential | Device-ID, Rolle, optional Runner-ID, Prüfwert, Ablauf, Widerruf |
| Enrollment | Code-Prüfwert, erlaubte Rolle, Ablauf, Versuche, Einlösestatus |
| Connection | Anbieter, Instanz-URL, Kontoanzeige, Fähigkeiten; keine Secrets |
| ConnectionBinding | Connection-ID, Ausführungsort, externe Credential-Referenz, Prüfstatus |
| ProjectConnection | Project-ID, Connection-ID, Zweck, benötigte Fähigkeiten |

Ein Ticket gehört im MVP höchstens einem aktiven Stream. Abhängigkeiten dürfen
keine Zyklen bilden. OpenCode-IDs werden immer zusammen mit ihrem Runner/Service
gespeichert, nicht als global eindeutige Factory-Identität behandelt.

## Zustände

### Erweiterung: Projektgedächtnis

- `MemorySource`: Projekt, Session/Stream, Quellreferenz, Codezustand und
  inkrementeller Analyse-Cursor.
- `KnowledgeEntry`: Projekt, Art, Aussage, Quellen, Gültigkeitsbereich
  (Projekt/Branch/Commit), Bestätigungsstatus, Revision und ersetzter Eintrag.
- `ContextPackage`: Aufgabe/Run, ausgewählte Wissensrevisionen, Tokenbudget und
  tatsächlich verwendete Quellen. Damit bleibt die Kontextzusammenstellung nachvollziehbar.

Wissensabfragen sind projektbezogen. Runner beziehen den Projektkern und das
aufgabenbezogene Paket über die Factory; zusätzliche Details werden bei Bedarf
abgerufen. Diese API-Erweiterung wird vor Umsetzung gesondert versioniert.

### Ausführungszustände

- Ticket: `backlog`, `ready`, `in_progress`, `blocked`, `review`, `done`, `cancelled`.
- Stream: `idle`, `queued`, `running`, `waiting_input`, `pausing`, `paused`,
  `blocked`, `review`, `completed`, `failed`.
- Runner-Verbindung: `online`, `suspect`, `unreachable`.
- Command: `pending`, `accepted`, `running`, `succeeded`, `failed`, `unknown`.
- Frage: `pending`, `answered`, `cancelled`.
- Fragen-Reconciliation: `current`, `reconciliation_required`, `superseded`;
  unabhängig vom fachlichen Antwortstatus.

Verbindungszustand ist unabhängig vom fachlichen Streamzustand. Ein unterbrochener
Eventstream beweist weder das Ende noch den Erfolg eines Agentenlaufs.

## Client-API (vorgeschlagen)

Präfix `/v1`; JSON-Fehler enthalten `code`, `message`, `request_id` und optional
die aktuelle Revision.

- `GET /snapshot`: konsistenter Projektzustand plus Event-Cursor.
- `GET /events?after=<cursor>`: SSE mit dauerhaftem Factory-Cursor.
- `POST /projects`, `POST /tickets`, `PATCH /tickets/{id}`.
- `POST /streams`, `POST /streams/{id}/commands`.
- `GET /streams/{id}/messages`: normalisierter, paginierter Sessionverlauf.
- `POST /questions/{id}/answer`.

Ändernde Requests enthalten eine eindeutige `request_id`; Änderungen bestehender
Aggregate zusätzlich `expected_revision`. Konflikte liefern HTTP 409, nicht
stillschweigend Last-Write-Wins. Dieselbe Request-ID mit anderer Payload wird
abgelehnt. Mutationen, Befehlsanlage und Events werden atomar gespeichert.

Für initiales Laden liefert der Snapshot den dazugehörigen Cursor. Anschließend
liest der Client Ereignisse ab diesem Cursor. Ein zu alter Cursor fordert einen
neuen Snapshot an. Token-Deltas können flüchtig bleiben; vollständige Nachrichten
und wichtige Statusänderungen müssen rekonstruierbar sein.

## Runner-API (vorgeschlagen)

1. `POST /runners/register`: authentifiziertes Gerät meldet Version und Fähigkeiten.
2. `POST /runners/{id}/heartbeat`: Zustand, aktive Commands, Sessions und Workspaces.
3. `GET /runners/{id}/commands?wait=...`: wartende Befehle per Long-Poll beziehen.
4. `POST /commands/{id}/ack`: lokale dauerhafte Annahme bestätigen.
5. `POST /commands/{id}/result`: Ergebnis oder unklaren Ausgang melden.
6. `POST /runners/{id}/events`: deduplizierbare Ereignis-Batches übertragen.

Jeder Streambefehl enthält `command_id`, `stream_id`, `ownership_epoch`, Art und
Payload. MVP-Arten: `prepare_workspace`, `start_session`, `send_prompt`,
`pause`, `resume`, `answer_question`, `reply_permission`.

Heartbeat-Intervalle und Ausfallgrenzen sind konfigurierbar; erste Testwerte:
10 Sekunden Heartbeat, nach 30 Sekunden `suspect`, nach 60 Sekunden `unreachable`.
Diese Zeitgrenzen erteilen niemals automatisch neue Schreibrechte.

### Zustellung und Wiederanlauf

Transport garantiert höchstens At-least-once-Zustellung. Der Runner führt ein
persistentes lokales Command-Journal und eine Event-Outbox. Wiederholte Befehle
werden anhand ihrer ID erkannt; lokale Events tragen eine stabile ID.

Genau-einmalige Ausführung über die OpenCode-Grenze wird nicht vorausgesetzt:
Stürzt der Runner nach einem externen Aufruf, aber vor dessen Bestätigung ab,
ist der Ausgang zunächst `unknown`. Er gleicht Session-/Inboxzustand ab und
sendet den Prompt nicht blind erneut. Nicht auflösbare Fälle werden angezeigt.

Nach Wiederverbindung: authentifizieren, Journal und aktive Sessions abgleichen,
Outbox übertragen, offene Fragen und Permissions nachladen, erst dann neue Arbeit.

### Schreibzuständigkeit

Der Server vergibt eine monotone Epoch je Stream. Runner lehnen ältere Epochen
ab. Ein Ownership-Wechsel benötigt zusätzlich die bestätigte Stilllegung des
vorherigen Schreibers. Epoch/Fencing alleine stoppt keinen bereits laufenden
Shellprozess auf einem getrennten Laptop.

Kann die Stilllegung nicht bestätigt werden, bleibt der Stream blockiert. Eine
manuelle Wiederherstellung erfolgt in einem getrennten Recovery-Branch und muss
spätere lokale Änderungen explizit zusammenführen.

### Fragen und Permissions

Live-Verifikation mit OpenCode 2.0.27: Eine `build`-Session mit explizit erlaubter
Aktion `question` öffnet ein Formular; dessen API-Antwort setzt den wartenden
Agentenlauf fort. Der Test prüft einen erst nach Eingang der Frage erzeugten Code
in der finalen Modellantwort. Die allgemeine Tool-Sperre `action:* / deny` muss
dabei um die bewusste Freigabe von `question` ergänzt werden. Fachliche Rückfragen
erhalten dadurch keine Freigabe für Shell-, Datei- oder andere Tool-Aktionen.

Antworten verwenden die externe Frage-/Permission-ID, Factory-Revision und
Request-ID. Der erste gültige Abschluss gewinnt; weitere Antworten erhalten den
aktuellen Zustand. Ist die externe Frage inzwischen abgelaufen oder erledigt,
wird der Zustand abgeglichen statt eine erfolgreiche Zustellung vorzutäuschen.

Im isolierten 2.0.27-Labor verschwanden synthetische offene Forms und Permissions
nach Container-Neustart und SIGKILL. Die Factory speichert deshalb vollständige
Frageschemata, Quellreferenzen und Run-Zuordnungen dauerhaft. Eine nach Neustart
fehlende Anfrage ist nicht automatisch beantwortet oder abgebrochen: sie benötigt
Reconciliation mit dem gültigen aktuellen Lauf. Das Neuanlegen einer Form anhand
ihrer ID ist möglich, stellt aber keine verlorene Tool-Continuation wieder her.

### OpenCode-Adapter

Der Adapter kapselt Service-Lifecycle, Authentifizierung, Sessions, Prompts,
Events, Verlauf, Interrupt, Forms und Permissions. Ein Versionscheck beim Start
erkennt nicht unterstützte API-Versionen. Sessionmigration bleibt optional.
Nach Event-Verbindungsabbruch werden aktive Sessions und offene Eingaben abgefragt.

Erster Befund gegen 2.0.27: Lokaler Service-Zugang verwendet HTTP Basic über die
private Service-Registrierung. Das ist unabhängig von Factory-Geräte-Tokens.
Prompts mit expliziter Nachrichten-ID wurden im Spike bei identischer Wiederholung
nicht doppelt eingereiht. HTTP 204 muss als erfolgreicher leerer Body behandelt werden.
`session.interrupt` meldet bei einer über `session.shell` gestarteten Shell
`interrupted:false`; bestätigte Pause erfordert zusätzlich Tool-/Prozessabgleich.
Details und Grenzen: [Spike](../spikes/opencode/README.md).

Beim echten Agenten-Shelltool mit kooperativem Vordergrund-/Kindprozess bestätigte
derselbe Endpoint dagegen `interrupted:true`. Beobachtete Prozesse endeten, die
Schreibdatei blieb stabil, die Session meldete Outcome `interrupted`. Eine neue
Nachricht setzte dieselbe Session fort, ohne den Worker erneut zu starten.
Ein späterer Runner darf deshalb eine Pause erst nach Reconciliation bestätigen:
keine aktive Agentenausführung, keine bekannten schreibenden Tools/Prozesse und
keine konkurrierende Zustellung neuer Arbeit. Unbekannte separate Shell-/PTY-
Läufe oder abgekoppelte Prozesse verhindern eine pauschale Stilllegungszusage.
`resume` bedeutet explizite weitere Arbeit, nicht das Wiederbeleben eines
unterbrochenen Betriebssystemprozesses.

Ergänzung aus dem Docker-Labor: dieselbe Nachrichten-ID mit anderer Payload liefert
HTTP 200 und den ursprünglichen Eintrag. Das Factory-Journal muss Payload-Konflikte
selbst prüfen. `resume:false` verhindert den unmittelbaren Agentenstart, ist aber
kein dauerhafter Pausen-Lock gegenüber späteren Session-Steuerbefehlen.

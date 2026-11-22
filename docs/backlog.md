# Umsetzungsplan

Historischer Entwicklungsplan. Der aktuelle Softwarestand und die verbleibenden
Aufgaben stehen in `implementation-status.md`. Separate Abnahmen sind gestrichen;
historische Testbefunde begründen keine neuen Abnahmeaufgaben.

F-001/F-002 sind teilweise verifiziert, F-101/F-102/F-105 sind umgesetzt,
F-104/F-106 sind teilweise umgesetzt; die übrigen Tickets
sind offen. Reihenfolge folgt Abhängigkeiten; Aufwandsschätzungen
werden nach dem Spike ergänzt. Etappe 0 endet mit einer dokumentierten
Machbarkeitsentscheidung, Etappe 1 mit einem nutzbaren vertikalen Durchstich.

## Etappe 0 – Integrations-Spike

### F-001: OpenCode-Vertrag und Version prüfen

Status: **teilweise verifiziert** gegen OpenCode 2.0.27 mit einem echten
Rust-HTTP-Client. [Code, reproduzierbare Prüfungen und Befunde](../spikes/opencode/README.md).
Auth, Session/Inbox, Forms, Permissions und Event-Reconnect sind lokal geprüft.
Echter Modelllauf und Headless-Anmeldung sind inzwischen im Container geprüft.
Agenten-Rückfrage und Fortsetzung über die Formular-API sind mit `verify-question`
geprüft. `verify-interrupt` bestätigt den Stop eines echten Agenten-Shelltools
mit kooperativem Kindprozess, Dateischreiber und anschließendem explizitem Fortsetzen.
OAuth-Refresh, signalresistente/abgekoppelte Prozesse und Crash während aktiver
Ausführung bleiben offen; der Grundablauf ist damit lokal und im Container belegt.
Neustarts und parallele Prompt-Wiederholungen sind im isolierten Docker-Labor geprüft.

- Exakte V2-Version und OpenAPI-Vertrag festhalten.
- OpenAI-/ChatGPT-Anmeldung lokal und headless auf dem Server prüfen, inklusive
  Persistenz und erneuter Anmeldung; Zugangsdaten nicht zwischen Services kopieren.
- Rust-HTTP-Zugriff mit korrekter Authentifizierung erproben.
- Session erstellen, Prompt senden, Events und Verlauf lesen.
- Form und Permission getrennt beantworten; Interrupt inklusive Tools prüfen.
- Event-Reconnect und Wiederabfrage offener Eingaben testen.
- Ergebnis: ausführbares Experiment und dokumentierte Einschränkungen.

### F-002: Server-Deployment und SSH-Pfad erproben

Status: **Docker-Lifecycle teilweise verifiziert** auf Ubuntu-24.04-Basis.
[Labor und Befunde](../spikes/opencode/container/README.md): authentifizierter
Loopback-Zugang, Persistenz nach Neustart/SIGKILL, Inbox-Deduplizierung. Offene
Forms/Permissions verschwinden; Factory-Reconciliation ist erforderlich.
Headless-Provider-Anmeldung und echte Modellantwort vor/nach Neustart sind geprüft.
Remote-SSH und Factory-Prozesszuschnitt bleiben offen.

- Voraussetzung: verifizierter API-Grundvertrag aus F-001; ein isoliertes Deployment
  ermöglicht anschließend die verbleibenden Login- und Lifecycle-Prüfungen aus F-001.
- Docker-Deployment auf Ubuntu 24.04, persistente Daten und definierter Lifecycle.
- API ausschließlich über Host-Loopback und SSH-Tunnel ansprechen.

### F-003: Lokalen Runner und Checkpoint-Transport erproben

- Voraussetzung: F-001; Transporttest zusätzlich F-002.
- OpenCode lokal auf Ubuntu 26.04 im eigenen Worktree ausführen.
- Rust-Beispiel lokal bauen, testen und starten.
- Internes Bare-Repository und SSH-Git-Transport zum Container prüfen.

### F-004: Spike auswerten und Verträge festziehen

- Voraussetzungen: F-001 bis F-003.
- Containerzuschnitt, API-Version, Rust-Adapter und Git-Transport festlegen.
- Verhalten bei Prompt-Zustellung mit unklarem Ausgang dokumentieren.
- Session-Export/-Import optional separat prüfen; kein Blocker für Etappe 1.

## Etappe 1 – Persönliche Factory mit zwei Runnern

### F-101: Rust-Workspace und gemeinsame Verträge

Status: **umgesetzt**. Sechs Crates, typisierte IDs/Zustände, versioniertes
JSON-Protokoll und ein ausführbarer Server/Runner/TUI-Verbindungsdurchstich.
HTTP-Integrationstests und Prozess-Smoke-Test inklusive echter Ratatui-PTY,
zwei Runnern, mehreren Clients und Wiederanmeldung nach Server-Neustart bestehen.
[Entwicklungsanleitung](development.md). Persistenz ergänzt F-102.

Vorziehen gegenüber F-004: Der API-Grundvertrag ist durch die isolierten Tests
ausreichend belegt. Offene OAuth-/Prozess-Sonderfälle bleiben im Spike dokumentiert;
der Adapter führt im Grundgerüst nur lesende Statusabfragen aus.

- Voraussetzung: F-004.
- Crates: `factory-core`, `factory-protocol`, `factory-opencode`, `factoryd`,
  `factory-runner`, `factory-tui`.
- Typisierte IDs, Revisionen, Zustände, Fehler und API-Versionierung.

### F-102: Persistenz und konkurrierende Client-Befehle

Status: **umgesetzt**. SQLite-Migrationen v1/v2, atomare
Mutationen/Eventlog/Request-Antworten, persistente Deduplizierung und Revisionskonflikte.
CLI zum Anlegen/Bearbeiten, TUI-Ansichten und JSON-Eventseiten sind vorhanden.
Tests prüfen konkurrierende Verbindungen, Replay nach Neustart, veraltete Revisionen
und vollständigen Rollback bei Fehler im Event-Insert. Streams mit exklusiver
Ticketzuordnung und dauerhafte Diagnose-Aufträge sind implementiert. Migration
v1→v2, Registrierungsschutz und Wiederanlauf werden ebenfalls getestet.

- Voraussetzung: F-101.
- SQLite-Migrationen, Projekte, Tickets, Streams, Commands und Eventlog.
- Transaktionale Mutationen, Request-Deduplizierung, Revisionskonflikte.

### F-103: Gerätezugang, Snapshots und Eventstream

- Voraussetzung: F-102.
- Geräte-Tokens, Snapshot mit Cursor, SSE-Replay und Ablauf alter Cursor.
- SSH-Konfiguration und unabhängige Runner-Verbindung.

### F-104: Runner-Journal und Wiederanlauf

Status: **teilweise umgesetzt**. Lokales SQLite-Journal mit Dateilock,
persistierten Empfangs-/Ergebnis-Request-IDs und Ergebnis-Outbox. Read-only
`check_open_code` wird nach Ack ausgeführt; Ergebnisse und Server-Aufträge
überleben Neustarts. Tests prüfen Journal-Wiederöffnung, Payload-Konflikte,
angenommene Aufträge nach Server-Neustart und echte Prozess-Wiederanläufe.
Schreibende Agentenaufträge journalisieren Session-/Prompt-Absichten vor HTTP-
Writes und gleichen unklare Ergebnisse über OpenCode ab. `unknown` reserviert
den Stream weiter. Der echte Zwei-Agenten-Test bestätigt Wiederanlauf während
Shelltools ohne doppelte Prompts oder Toolausführung. Long-Poll ist inzwischen
implementiert. Offen: Bedienabläufe für dauerhaft unauflösbare `unknown`-Zustände.

- Voraussetzungen: F-101, F-102.
- Registrierung, Heartbeats, Long-Poll, Command-Journal und Event-Outbox.

### F-105: Projekt- und Worktree-Verwaltung

Status: **umgesetzt für registrierte lokale Repositories**. `prepare_workspace` über die persistente
Auftragswarteschlange, lokale Projekt→Repository-Zuordnung, eigener Branch/Pfad
pro Stream und vor Git-Operationen gespeicherter Plan. Der Wiederabgleich prüft
Repository, Pfad, Branch und Ausgangs-Commit; vorhandene Dateien werden nicht
zurückgesetzt. Tests prüfen zwei getrennte schreibbare Worktrees, schmutzige
Quell-Checkouts, fremde Pfade/Branches, unterbrochene Anlage, Journal-Migration
und echte Runner-/Server-Neustarts. Versionierte Toolchain-/Start-/Testprofile
werden mit Provider-/Modellauswahl gespeichert und in Agentenaufträge kopiert.
Automatisches Clone/Fetch über verwaltete Hosting-Verbindungen bleibt eine Erweiterung.

- Voraussetzungen: F-102, F-104.
- Repository registrieren, Branch/Worktree je Stream anlegen, Pfade zuordnen.
- Projektbezogene Toolchain- und Start-/Testbefehle erfassen.

### F-106: OpenCode-Ausführung über beide Runner

Status: **schreibender Durchstich umgesetzt**. `run_agent`, neue Session beim
normalen Start und explizite Fortsetzung nach Erfolg in derselben Session,
deterministische IDs, Prompt-Zustellung, Verlauf-/Aktivitätsabgleich,
persistente Fortschritte und Abschlusszusammenfassung. Zwei echte Agenten haben
im Ubuntu-Labor gleichzeitig getrennte Worktrees geändert und Python-Prüfungen
ausgeführt. Factory-/Runner-Neustart während laufender Shelltools erzeugte weder
eine zweite Nachricht noch eine zweite Toolausführung. Die beiden Runner laufen
im selben Labor; reale getrennte Hosts sind noch nicht abgenommen.
Fragen und Permissions werden inzwischen dauerhaft mit Herkunft/Schema gespeichert.
CLI-Antworten sind revisionsgeschützt und werden über den Runner mit lokalem
Zustelljournal weitergegeben. Der echte Test prüft Agentenfragen mit vorher
unbekannten Tokens, gespeicherte Antworten nach Factory-Neustart und Shell-Freigaben.
Tests für konkurrierende Antworten, verschwundene Fragen und unbestätigte
HTTP-Antworten ohne erneuten POST bestehen.
SSE und Session-Kontextfortsetzung nach Erfolg sind implementiert. Offen:
verifizierte Pause/Resume laufender Prozesse und Fortsetzung nach Interrupt.

- Voraussetzungen: F-104, F-105.
- Adapter implementieren: Start, Prompt, Verlauf, Events, Pause, Rückfragen,
  Permissions und Wiederabgleich.

### F-107: Ratatui-MVP

Status: **Ansichten und Antwortdialoge vorhanden**. Runner, Projekte, Tickets,
Streams, Aufträge und persistente Fragen (`6`) sind sichtbar. `Enter` öffnet
Formular-/Permission-Dialoge mit Schemaansicht, Unicode-Eingabe, typisierten
Feldern und asynchronem Versand. Unklare Zustellung verwendet dieselbe Request-ID;
Konflikte erhalten den Entwurf und verlangen explizites Neuladen. Ein echter
PTY-/HTTP-Test prüft Antworten, Permissions, Mehrclient-Konflikt und Serverausfall.
Stream-Steuerung über `4` → `Enter`: Vorbereitung, Agentstart, persistenter
Pausewunsch und Freigabe bestätigter Pre-Execution-Holds. PTY-Test prüft Pause/Resume;
Store-/Runner-Tests prüfen Neustart, veraltete Reports und einmalige Interrupt-Absicht.
Verwaltungsdialoge: Projektanlage und Profilkonfiguration, Ticketanlage und
revisionsgeschützte Bearbeitung sowie Streamanlage mit Runner-/Ticketauswahl.
Unklare Requests werden identisch wiederholt; Konflikt-Rebase behält lokale
Änderungen und übernimmt fremde Änderungen an unveränderten Feldern. Ein echter
PTY-/HTTP-Test deckt den kompletten Verwaltungsablauf einschließlich Konflikt ab.
Projekt-Metadaten sind über `e` und CLI/API revisionsgeschützt bearbeitbar.
Repository/Basisbranch bleiben ab der ersten Streamanlage festgeschrieben;
Metadaten- und Profiländerungen teilen eine Revision. Neustart-/Race-Tests und
PTY-Abnahme bestehen.
Auftragsdetails über `5` → `Enter` zeigen Prompt, unveränderliches Profil,
Workspace, Fortschritt/Ergebnis und einen cursorbasiert nachgeladenen Factory-
Ereignisverlauf. Die Anzeige ist begrenzt und behält ihren Stand bei Netzfehlern;
PTY-Test prüft Inhalte und Offline-Anzeige. Zusätzlich überträgt der Runner einen
dauerhaften Sessionauszug mit bis zu 32 Nachrichten / 128 KiB Text und sichtbaren
Kürzungen. Toolinput/-ausgaben und abschließender Kontext wurden mit zwei echten
Agenten einschließlich Factory-/Runner-Neustart geprüft. Alte/konkurrierende
Quellrevisionen werden abgewiesen; gescheiterte Reads erhalten den letzten Stand.
Das paginierte projizierte Nachrichtenarchiv ist inzwischen implementiert.
Offen: binäre Anhänge, bestätigte Pause laufender Prozesse und Session-Fortsetzung
nach einem Interrupt.

- Voraussetzungen: F-103, F-106.
- Projektwahl, Ticketliste, Streams samt Runner, Sessionverlauf und Eingabe.
- Stream starten, Aufgabe zuordnen, pausieren/fortsetzen, Fragen beantworten.
- Lokalen Workspace-Pfad sowie Start-/Testbefehle anzeigen.
- Verbindung, ausstehende Befehle und Versionskonflikte sichtbar machen.

### F-108: Installationspakete und Dokumentation

- Voraussetzungen: F-107.
- Docker-/Compose-Konfiguration, Volumes, lokale systemd-User-Unit und Anleitung.
- Daten-/Konfigurationssicherung und Wiederherstellung dokumentieren.

### Ergänzende Umsetzungstickets

#### F-109: SSH-Profile und Geräte-Enrollment

- Voraussetzungen: F-103.
- OpenSSH-Konfiguration/Agent, Hostprüfung, Bootstrap und einmaliges Enrollment.
- TUI-/Runner-Rollen trennen, Widerruf einschließlich aktiver API-Verbindungen.
- SSH-Schlüsselfreischaltung bleibt beim Betreiber.

#### F-110: Verbindungsverwaltung und Projektassistent

- Voraussetzungen: F-105, F-106, F-109.
- Verbindungen pro Anbieter und Ausführungsort, native OpenCode-Anmeldeabläufe.
- Bestehende GitHub-/Forgejo-Repositories verbinden; Git- und API-Rechte unterscheiden.
- Vorhandene Verbindungen wiederverwenden, fehlende Rechte gezielt nachfordern.
- Secret-Eingaben aus Logs, Commands und Wissensarchiv ausschließen.

#### F-111: GitHub-Release-Artefakte und Ansible-Vertrag

- Voraussetzungen: F-108, F-109, F-110.
- GHCR-Image, Linux-Binaries, Prüfsummen, Compose-Beispiel und Konfigurationsreferenz.
- Versionen/Architekturen pinnen, Health-/Readiness-Verhalten und Migrationen dokumentieren.

## Folgeetappen

### Projektgedächtnis – Grundlage im MVP, Automatisierung anschließend

- MVP: Quellenarchiv mit Projekt-/Session-/Stream-ID und Codebezug; kuratierter
  Projektkern, lesbar für beide Runner über die Factory-API.
- Danach: automatische Stream-Zusammenfassungen und inkrementelle Extraktion
  strukturierter Erkenntnisse mit Quellen, Status und Gültigkeitsbereich.
- Widersprüche, Ablösung alter Entscheidungen und branchbezogenes Wissen behandeln.
- Budgetierte Kontextauswahl aus Projektkern und relevanten Erkenntnissen;
  gezieltes Nachladen von Details und Originalquellen.
- Korrigieren, Anheften und Löschen ermöglichen; Secrets von der Extraktion ausschließen.

### Etappe 2 – Übergabe und Integration

- Checkpoint-Verwaltung und Kontextpakete.
- Persistente Übergabe in beide Richtungen mit bestätigter Stilllegung.
- Tests für Transferabbrüche, dirty Workspaces und Wiederanlauf.
- Recovery-Branches für nicht bestätigbar gestoppte Runner.
- Review, Konfliktbehandlung und Test gegen aktuellen Zielbranch vor Integration.

### Etappe 3 – Supervisor

Teilstand: [modellbasierter Session-Supervisor](supervisor.md) mit dauerhaftem
Entscheidungsjournal, Bewertungsbudget, automatischer Compaction-Anforderung und
Abschluss bereits erfolgreicher Sessions implementiert. Echte Modellbewertung,
automatischer Abschluss mit Wissensnotiz und Wiederanlauf sind lokal geprüft.
Ticket-Scheduling, automatische Antworten, Kostenmessung und bestätigtes Beenden
laufender Sessions bleiben offen.

- Projektregeln, Entscheidungsjournal und deterministisches Scheduling.
- Ticket-Zuordnung mit Abhängigkeiten und Parallelitätslimits.
- Belegte automatische Antworten; zentrale menschliche Inbox.
- Kosten-/Wiederholungslimits und Eskalation bei fehlendem Fortschritt.
- Autonomes Session-Lifecycle-Management: Der Operator-Agent entscheidet selbst,
  wann er gezielt eine native OpenCode-Compaction anfordert und wann eine Session
  beendet werden soll. Keine grundsätzliche menschliche Einzelbestätigung für
  diese Entscheidungen. Die Factory protokolliert Entscheidung, Begründung,
  betroffene Session und Ausführungsergebnis.
- Compaction anhand Kontextauslastung, wiederholter/überholter Informationen und
  anstehendem Arbeitsschritt entscheiden; relevante Entscheidungen, offene Fragen
  und Arbeitsstand vorher im Projektgedächtnis sichern. Laufende Toolausführung
  nicht durch einen zweiten schreibenden Auftrag überholen.
- Compaction-Absicht vor externem POST journalisieren, stabile Message-ID verwenden
  und Inbox/projizierte Compaction nach unklarem Ausgang lesen statt blind erneut
  anzufordern. Eine angenommene Compaction ist noch keine abgeschlossene Compaction.
- Sessions beenden bei Zielerreichung, obsoleter Aufgabe, Sackgasse oder sinnvoller
  Neuaufteilung. Beenden erhält Archiv/Provenienz; es löscht keine Sessiondaten.
  Ein Interrupt allein bestätigt keine Prozessruhe und gibt keinen Stream frei.
  Kontext-/Checkpoint-Übergabe an eine neue Session bleibt nachvollziehbar.

### Etappe 4 – Android

- Geräteanmeldung, SSH-Verbindung und mobile API-Bedienung.
- Projektstatus, Tickets, Rückfragen und Pause/Fortsetzung.
- Push-Transport wählen, Benachrichtigungen deduplizieren, Deep Links zur Inbox.

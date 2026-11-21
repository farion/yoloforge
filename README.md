# YoloForge

Repository: https://github.com/farion/yoloforge

Der CLI-Einstieg heißt `yoloforge`. `factory` bleibt als kompatibler Alias
verfügbar; interne `factory-*`-Crates, Protokollkennungen und vorhandene Datenpfade
bleiben kompatibel. Veröffentlichung: Commit → Tag → Push → GitHub Release.
Die Installation auf dem Server erfolgt ausschließlich durch das Ansible des
Betreibers. Siehe [Deployment](docs/deployment.md).

Persönliche Entwicklungs-Factory zur Verwaltung paralleler OpenCode-Arbeitsstreams.
Ein zentraler Rust-Dienst verwaltet Projekte und Aufgaben; Agenten arbeiten auf
dem Laptop oder Server. Eine Ratatui-Oberfläche und später eine Android-App
bedienen denselben Zustand.

## Ausprobieren

Version 0.1.0 ist eine frühe nutzbare Grundlage, nicht der Abschluss aller
15 Arbeitspakete. Gerätezugang, SSE, Docker und Backup/Restore sind implementiert.
Supervisor samt autonomer Compaction/Sessionbeendigung, Projektgedächtnis,
Checkpoint-Transfer und Android folgen. Aktueller
[Umsetzungsstand](docs/implementation-status.md).

Server, Runner und Ratatui laufen; Projekte und Tickets werden mit SQLite persistiert:
[Build, Startanleitung und Tests](docs/development.md).

```sh
cargo build --workspace --locked
python3 -u scripts/smoke-foundation.py
```

## Architektur und Planung

![Geplante Architektur](docs/architecture.svg)

[SVG](docs/architecture.svg) · [PNG](docs/architecture.png) ·
[Editierbare LAML-Quelle](docs/architecture.laml)

Blau: MVP-Prozesse · Grün: persistente Daten · Orange: spätere Erweiterungen.
Runner und OpenCode sind zur Übersicht je Ausführungsort zusammengefasst,
bleiben aber getrennte Prozesse. Pro Stream schreibt nur ein Runner.

- [Architektur und MVP](docs/architecture.md)
- [Datenmodell und Runner-Protokoll](docs/protocol.md)
- [Zugang, Verbindungen und Deployment](docs/access-and-deployment.md)
- [Umsetzungstickets und Abnahme](docs/backlog.md)
- [Repository klonen, registrieren und aktualisieren](docs/repositories.md)

Status: Rust-Verbindungsdurchstich (F-101), persistente Projekt-/Ticketverwaltung
(F-102), Streams mit persistenter Auftragswarteschlange und verifizierter
[OpenCode-Spike](spikes/opencode/README.md). Runner journalisieren Aufträge und
Ergebnisse lokal, legen isolierte Git-Worktrees an und führen dort OpenCode-Agenten
mit Projektprofilen aus (F-105 / Teil F-106). Zwei parallele schreibende Agenten
und Wiederanlauf während aktiver Tools sind im Labor geprüft. Agentenfragen und
Permissions können über die CLI oder direkt in Ratatui beantwortet werden:
`6` → Frage auswählen → `Enter` → `Ctrl-S`. In `2`/`3`/`4` legt `n` Projekte,
Tickets und Streams an; `Enter` öffnet Profil-, Ticket- oder Streamdialoge.
`e` in der Projektliste bearbeitet bestehende Metadaten.
`5` → `Enter` zeigt Auftragsdetails, Prompt, Profil, Factory-Ereignisverlauf und
gespeicherte Sessionauszüge mit Text-/Toolausgaben und sichtbaren Kürzungen.
Persistente Pausewünsche sind vorhanden; bestätigte Pause laufender Prozesse folgt.

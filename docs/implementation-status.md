# Softwarestand der 15 Punkte

Stand: 0.1.3-dev, Server-Schema 33.
Abnahmen sind auf Wunsch des Betreibers aus der Aufgabenplanung gestrichen.
Zielhost-, Realmodell-, Geräte- und Hosting-Abnahmen sind keine offenen Aufgaben
und keine Voraussetzung für den Abschluss. Bereits dokumentierte Testergebnisse
in den technischen Detaildokumenten bleiben als historische Befunde erhalten.

| Nr. | Arbeitspaket | Softwarestand |
| --- | --- | --- |
| 1 | Pause und Session-Fortsetzung | Implementiert: native Unterbrechung mit finalem Archiv und expliziter Freigabe; Fortsetzung in derselben Session. Optional Container-Freeze/Thaw. Native Unterbrechung ist kein Nachweis gestoppter Hintergrundprozesse. |
| 2 | Recovery ungeklärter Aufträge | Journalbasierter Wiederanlauf, Antwortverlust-Abgleich und dauerhafte Quarantäne nach Journalverlust implementiert. Weiterer Recovery-Ausbau wird auf Betreiberwunsch nicht verfolgt; ohne Nachweis keine blinde Wiederholung. |
| 3 | Sessionarchiv | Nachrichtenarchiv, Backfill, CLI/TUI/Android-Export, separate SHA256-geprüfte Dateien bis 64 MiB und Rescan erhaltener externer lokaler Quellen implementiert. Verlorene Bytes lassen sich nicht rekonstruieren. |
| 4 | SSE und Runner-Long-Poll | Implementiert. |
| 5 | Deployment, Backup/Restore, Updates | Verpackung, Vorlagen, Sicherungs-/Wiederherstellungswege und Dokumentation vorhanden. Installation bleibt beim Betreiber. |
| 6 | Laptop-/Server-Abnahme | Gestrichen; kein Software-Arbeitspaket. |
| 7 | Gerätezugang, Rollen, Widerruf | Direkt ausgestellte Zugangsschlüssel, Runner-Scope und Widerruf aktiver Verbindungen über API/CLI. |
| 8 | Anbieter-/Hosting-Zugänge | Native OpenCode-Anmeldung, Statusanzeige, SSH-Profile und Bereitschaftshelfer implementiert. |
| 9 | Clone/Fetch und Repository-Einrichtung | Projektassistent, verwaltete Repositories, Clone/Fetch, explizite Modellbindung, MCP-/Harness-Konfiguration und Runner-Pfade implementiert. Weitere Login-/Editor-Komfortfunktionen optional. |
| 10 | Checkpoints und Host-Übergabe | Git-Snapshots, Kontextpakete, Bundle-Transport und revisionsgeschützter Ownership-Wechsel implementiert. |
| 11 | Review und Integration | Review-Kandidaten, Testausführung, Git-CAS-Integration, GitHub-/Forgejo-Publikationshelfer, TUI-Assistent und Projekt-Watcher implementiert. Lokale parallele Ergebnisse werden konfliktfrei dreiseitig kombiniert; echte Konflikte behalten Snapshots und konkrete Diagnose und können durch Nacharbeit in derselben Session gelöst werden. |
| 12 | Projektgedächtnis und Kontextauswahl | Quellen-/Branch-/Ticketscope, Extraktion, Ablösung, Token-/Kostenanzeige, optionale Arbeitsbudgets und Tokenbudget für injiziertes Wissen implementiert. Eigene Gesamtkontextplanung gestrichen; Sessionhistorie und Compaction bleiben bei OpenCode. |
| 13 | Supervisor | Compaction, Abschluss, Runtime-Beendigung, Nacharbeit in derselben Session, Konflikt-Nacharbeit und Neuverteilung nach bestätigtem Stopp implementiert. Fortschritts-/Stallerkennung bleibt offen. |
| 14 | Android und Push | Verschlüsselte Entwürfe, Fragen, Bildaufgaben, Archiv-/Dateiexport, Updates und optionale dauerhafte ntfy-Outbox implementiert. |
| 15 | GitHub-Releases und Ansible-Vertrag | Getrennte Plattformartefakte, Serverimage, Prüfsummen und stabil signiertes APK vorhanden. Stable ist v0.1.2; neue Funktionen liegen auf main in 0.1.3-dev. |

## Priorisierte Softwarearbeit

**Native GUI:** Rust-/egui-Client mit Dark Theme, Dialogen, Aufgabenfreigabe,
Dateiaufgaben, gruppierten Fragen, lesenden Sessiontabs, Instanzpräsenz und
dauerhaften Benachrichtigungen ist vorhanden. Bestehende TUI bleibt nutzbar.
[Bedienung](gui.md), [Funktionszuordnung](gui-parity.md), [Prüfliste](gui-plan.md).

**Festgefahrene Arbeit:** Bei Stillstand anhand belastbarer Signale reagieren,
ohne langsame Tools oder menschliche Rückfragen abzubrechen. Das ist nicht Teil
der Änderung für parallele Ergebniskonflikte.

Gesamtkontextplanung ist gestrichen. Weiterer Recovery-Ausbau wird ignoriert.

Technische Details: `native-pause.md`, `archive-files.md`, `hosting-publication.md`,
`context-planning.md`, `supervisor.md`. Normale Entwicklungschecks bleiben Teil von
Codeänderungen; es gibt keinen separaten Abnahmeprozess.

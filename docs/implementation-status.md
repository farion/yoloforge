# Umsetzung der 15 offenen Punkte

Quelle: Arbeitsauftrag des Betreibers, alle 15 offenen Punkte umzusetzen.
Ein Eintrag gilt erst nach implementierter Funktion und dokumentierter Abnahme
als abgeschlossen. Laborprüfungen ersetzen keine Abnahme auf dem Zielhost.

## Prompt-Aufgaben und Projektpause (Schema 24)

Ergebnisübernahme (Runner-Journal 9): erfolgreiche lokale Aufgaben werden automatisch
in den angegebenen Projektordner übernommen und bilden die Basis für Folgeaufgaben.
Unabhängiger Snapshot-Index, journalisierter Commit, sauberer unveränderter Zielstand,
Fast-forward und CAS schützen Ausführungsindex und Benutzeränderungen. Konflikte
bleiben mit konkreter Ursache sichtbar; siehe `result-publication.md`.

Session-Einstieg: Enter auf einem Arbeitsbereich oder einer gestarteten Aufgabe
öffnet die originale OpenCode-TUI im selben Terminal, gebunden an Service, Session
und Arbeitsordner. Nach Beenden kehrt YoloForge zurück. Native PTY-Abnahme mit der
Minesweeper-Session bestanden; fehlender CLI-Start wird sichtbar abgefangen.
Für entfernte Runner fehlt noch der native Tunnel. Die bisherige synchronisierte
Live-Ansicht bleibt unter `v`, technische Steuerung unter `c` verfügbar; siehe
`session-view.md`.

TUI-Aufgaben erfassen nur Prompt und optionale Bilder (bis vier PNG/JPEG/GIF/WebP,
zusammen 4 MiB). Ein einzelner Prompt-Editor ersetzt die schrittweise Erfassung:
Enter sendet, Shift-/Alt-Enter erzeugt neue Zeilen, Ctrl-V liest Bild/Text aus der
lokalen Zwischenablage (Linux X11/Wayland, Windows); hineingezogene Bilddateien
werden direkt angehängt. Ctrl-D entfernt den letzten Anhang. Die Zwischenablage
gehört zum TUI-Host; SSH stellt nicht automatisch die lokale Desktop-Zwischenablage
bereit. `/v1/tasks` speichert die Bilddaten dauerhaft;
der Runner übergibt sie als Inline-Dateien an OpenCode und prüft beim Wiederanlauf
den Bild-Digest der Prompt-Projektion. Alte Runner dürfen Bildaufträge nicht annehmen
(`task-images-v1`). Binäranhänge aus Agentenantworten/Archiv-Backfill bleiben offen.

Standardplanung entsteht automatisch beim Runner-Heartbeat, einschließlich
verwaltetem Repository für bisher nicht eingerichtete Projekte mit Aufgaben.
Kein Ready-Schritt und keine Aktivierung nötig. Projektpause hält neue Starts an;
angenommene Arbeit läuft weiter. Nach finalem Prompt-Ergebnis kann die nächste
Aufgabe starten, ohne manuelles Done. Store-Integrationstest prüft Prompt/Bilder,
Pause vor Annahme, Neustart, Capability-Fencing und Folgeaufgabe. Der PTY-Test
`scripts/smoke-project-setup.py --empty --tasks` prüft echte TUI/Factory/Runner,
Prompt-/Bildpersistenz und Pause/Fortsetzen bis zur automatischen Einplanung.
Mit `--new-folder --native --tasks` wurde auch ein echter isolierter OpenCode-
Service gestartet und der fehlende Projektordner angelegt. Prompt-Editor, Unicode-Navigation,
Anhangentfernung und unveränderliche Wiederholung sind zusätzlich unit-getestet.
Der PTY-Test fügt eine Bilddatei direkt in den Prompt ein und sendet mit Enter.
`scripts/smoke-clipboard.py` prüft echte X11-Bild-/Text-Zwischenablage im isolierten
Xvfb. Wayland-/Windows-Desktop-Abnahme steht noch aus. `scripts/smoke-prompt-image.py`
prüft mit echtem Modell den gesamten Ablauf vom fehlenden Projektordner über
automatische Einplanung und Inline-Bild bis zum geschriebenen Ergebnis und finalen
Archiv; bestanden (rotes Bild korrekt erkannt).

Fehlende absolute Projektordner werden als neue Projekte angelegt und ohne
Überschreiben veröffentlicht; vorhandene Repositories bleiben Importquellen.
Unter Schema 23 gescheiterte lokale Imports werden nach Registrierung eines
`local-project-create-v1`-Runners einmalig erneut vorbereitet. Wiederaufnahme ist
revisions-/epochengebunden, respektiert Pause und führt die Planung weiter.
Migration, Neustart und Folgeaufgabe sind geprüft. Auch alte automatisch geplante
Tickets brauchen nach einem finalen Ergebnis kein manuelles Done.

| Nr. | Arbeitspaket | Stand / Abnahme |
| --- | --- | --- |
| 1 | Bestätigte Pause und Session-Fortsetzung | Fortsetzung nach Erfolg sowie dediziertes Container-Freeze/Thaw mit echtem Modell lokal geprüft: abgekoppelte Tools, Factory-/Runner-Neustart, gleiche Ausführung, genau ein Toolaufruf und finales Archiv. Zielhost-Abnahme, automatische Provisionierung und Fortsetzung nach nativem Interrupt offen |
| 2 | Recovery ungeklärter Aufträge | Expliziter API/CLI/TUI-Neuversuch unklarer Container-Freeze/Thaw-Steuerungen mit Instanzbindung, Revisionsschutz und Retry-Provenienz implementiert; TUI mit gebundenem Nachweis, unveränderlichem Replay und Offline-Entwurf lokal per PTY geprüft. Recovery ohne Nachweis, bei Container-/Journalverlust und für native Interrupts/Prompts offen |
| 3 | Vollständiges Sessionarchiv | Ungekürzte paginierte Message-Projektionen, CLI-Export und TUI-Archivbrowser mit eingefrorenem Revisionsstand implementiert; Binäranhänge und historischer Backfill offen |
| 4 | SSE und Runner-Long-Poll | Implementiert und lokal getestet; TUI nutzt SSE, Runner Long-Poll |
| 5 | Deployment, Backup/Restore, Updates | Container, Vorlagen und Backup/Restore lokal getestet; Zielinstallation durch Betreiber |
| 6 | Echte Laptop-/Server-Abnahme | Erstinstallation v0.1.1 auf rs9tm lesend geprüft: Binary-Hash, Health/Auth, Portbindung und Persistenz passen. Runner-/Agentenbetrieb, Update und Rollback noch offen |
| 7 | Gerätezugang, Rollen, Widerruf | API/CLI und TUI-Geräteverwaltung implementiert und lokal getestet: Rollen, einmalige Enrollment-Codes, Runner-Scope, Widerruf einschließlich laufender SSE-/Long-Polls |
| 8 | OpenAI-/GitHub-/Forgejo-Zugangsverwaltung | Offen; lokale OpenCode-Anmeldung im Labor vorhanden |
| 9 | Clone/Fetch und Repository-Einrichtung | CLI clone/register/fetch sowie TUI-Projektanlage ohne technische Eingaben: verwaltetes leeres Repository oder optionaler Clone, automatische Runner-/Service-Auswahl und native Standardmodellbindung. Strukturierter MCP-Editor und gemeinsame Projekt-Harness mit Runner-Zuordnungen implementiert. TUI-/Runner-Test mit synthetischem OpenCode-Health, lokale Imports und Neustarttests bestanden. Fetch im Assistenten, erweiterte Editoren und Login-Führung offen (`project-setup.md`) |
| 10 | Checkpoints und Laptop-/Server-Übergabe | API/CLI/TUI/Runner: journalisierte Git-Checkpoints, Kontextpaket, Bundle-Transport über Factory, revisionsgeschützter Ownership-Wechsel und neue Epochen-Worktrees implementiert. Zwei unabhängige lokale Runner mit Neustart und Rückübergabe geprüft; Checkpoint nach Realmodell-Erfolg und Runtime-Cleanup geprüft. Getrennte Zielhosts und Realmodell-Übergabe offen. Siehe `checkpoints.md` |
| 11 | Review und Integration | API/CLI/TUI/Runner: Merge-Kandidat gegen gepinnten Zielbranch, Docker-isolierte Tests, bestätigte Test-Runtime-Entfernung und lokale Git-CAS-Integration implementiert. Neustart ohne zweiten Testlauf, Testfehler und konkurrierende Zielbranchänderung lokal per Prozess-/Docker-Test geprüft. Hosting-Push/PR und Zielhost-Abnahme offen; siehe `review-integration.md` |
| 12 | Projektgedächtnis und Kontextauswahl | Branch-/Ticketscope, budgetierte Prompt-Übernahme und atomare revisionsgeschützte Ablösung mit Provenienz implementiert; Supervisor-Beobachtungen bleiben branchlokal. Automatische semantische Widerspruchsauflösung und modellspezifischer Tokenizer offen |
| 13 | Supervisor | Modellbasierter Session-Supervisor mit Entscheidungsjournal, Bewertungsbudget, Compaction, Sessionabschluss und gebundener Runtime-Beendigung implementiert. Abhängigkeitengesteuerte Ticketplanung mit Parallelitäts-/Startbudget per API/CLI/TUI implementiert und auf Konkurrenz/Neustart sowie Budget-Rebase geprüft (`scheduler.md`). Automatische Antworten und Zielhost-Abnahme offen |
| 14 | Android und Push | Offen; reales Gerät/Push-Zugang für Abnahme nötig |
| 15 | GitHub-Releases und Ansible-Vertrag | v0.1.1 veröffentlicht; Server-/Linux-TUI-Downloads lokal geprüft, Windows nativ in CI geprüft; Android-APK erst mit implementierter App |

Siehe `backlog.md` für Akzeptanzkriterien und `development.md` für bereits
implementierte Bedienabläufe. Zugangsdaten nicht in dieses Dokument eintragen.

Ergänzung zu 2/13: Bestätigte Entfernung einer gebundenen dedizierten Runtime
über API/CLI und Runner implementiert; reale Docker-Abnahme mit abgekoppelten
Writern und Journal-Recovery bestanden. Auch extern entfernte Container sind mit
erhaltener Daemon-Bindung abschließbar. Fehlende Nachweise/Journale, ersetzte
Instanzen und native Recovery bleiben offen. Neue Ausführungen veröffentlichen
ihre Runtime-/Daemon-Bindung vor Agent-Schreibzugriffen zentral; damit ist
Beendigung nach lokalem Journalverlust implementiert und mit Docker geprüft.
Autonome Supervisor-Beendigungsanfragen mit atomarer Wissenssicherung sind
implementiert und mit synthetischen Modellentscheidungen geprüft; Realmodell-
Abnahme bleibt offen. Cleanup nach abgeschlossenem Erfolg bewahrt Ergebnis und
Archiv und ist mit echtem Modell geprüft; Supervisor-Close fordert Cleanup atomar
mit an. Details: `runtime-termination.md` (DB 24, Runner-Journal 9).

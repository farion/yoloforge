# Umsetzung der 15 offenen Punkte

Quelle: Arbeitsauftrag des Betreibers, alle 15 offenen Punkte umzusetzen.
Ein Eintrag gilt erst nach implementierter Funktion und dokumentierter Abnahme
als abgeschlossen. Laborprüfungen ersetzen keine Abnahme auf dem Zielhost.

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
| 9 | Clone/Fetch und Repository-Einrichtung | Lokale CLI clone/register/fetch mit atomarer Runner-Zuordnung implementiert; TUI-/Remote-Aufträge offen |
| 10 | Checkpoints und Laptop-/Server-Übergabe | API/CLI/TUI/Runner: journalisierte Git-Checkpoints, Kontextpaket, Bundle-Transport über Factory, revisionsgeschützter Ownership-Wechsel und neue Epochen-Worktrees implementiert. Zwei unabhängige lokale Runner mit Neustart und Rückübergabe geprüft; Checkpoint nach Realmodell-Erfolg und Runtime-Cleanup geprüft. Getrennte Zielhosts und Realmodell-Übergabe offen. Siehe `checkpoints.md` |
| 11 | Review und Integration | API/CLI/TUI/Runner: Merge-Kandidat gegen gepinnten Zielbranch, Docker-isolierte Tests, bestätigte Test-Runtime-Entfernung und lokale Git-CAS-Integration implementiert. Neustart ohne zweiten Testlauf, Testfehler und konkurrierende Zielbranchänderung lokal per Prozess-/Docker-Test geprüft. Hosting-Push/PR und Zielhost-Abnahme offen; siehe `review-integration.md` |
| 12 | Projektgedächtnis und Kontextauswahl | Branch-/Ticketscope, budgetierte Prompt-Übernahme und atomare revisionsgeschützte Ablösung mit Provenienz implementiert; Supervisor-Beobachtungen bleiben branchlokal. Automatische semantische Widerspruchsauflösung und modellspezifischer Tokenizer offen |
| 13 | Supervisor | Modellbasierter Session-Supervisor mit Entscheidungsjournal, Bewertungsbudget, Compaction, Sessionabschluss und gebundener Runtime-Beendigung implementiert. Abhängigkeitengesteuerte Ticketplanung mit Parallelitäts-/Startbudget per API/CLI implementiert und auf Konkurrenz/Neustart geprüft (`scheduler.md`). Automatische Antworten, Scheduler-TUI und Zielhost-Abnahme offen |
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
mit an. Details: `runtime-termination.md` (DB 20, Runner-Journal 6).

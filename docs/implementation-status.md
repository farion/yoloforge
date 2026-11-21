# Umsetzung der 15 offenen Punkte

Quelle: Arbeitsauftrag des Betreibers, alle 15 offenen Punkte umzusetzen.
Ein Eintrag gilt erst nach implementierter Funktion und dokumentierter Abnahme
als abgeschlossen. Laborprüfungen ersetzen keine Abnahme auf dem Zielhost.

| Nr. | Arbeitspaket | Stand / Abnahme |
| --- | --- | --- |
| 1 | Bestätigte Pause und Session-Fortsetzung | Fortsetzung nach Erfolg in derselben Session über API/CLI/TUI implementiert und mit echtem Agenten geprüft; Prozesscontainment und Fortsetzung nach Interrupt offen |
| 2 | Recovery ungeklärter Aufträge | Offen |
| 3 | Vollständiges Sessionarchiv | Ungekürzte paginierte Message-Projektionen, CLI-Export und TUI-Archivbrowser mit eingefrorenem Revisionsstand implementiert; Binäranhänge und historischer Backfill offen |
| 4 | SSE und Runner-Long-Poll | Implementiert und lokal getestet; TUI nutzt SSE, Runner Long-Poll |
| 5 | Deployment, Backup/Restore, Updates | Container, Vorlagen und Backup/Restore lokal getestet; Zielinstallation durch Betreiber |
| 6 | Echte Laptop-/Server-Abnahme | Erstinstallation v0.1.1 auf rs9tm lesend geprüft: Binary-Hash, Health/Auth, Portbindung und Persistenz passen. Runner-/Agentenbetrieb, Update und Rollback noch offen |
| 7 | Gerätezugang, Rollen, Widerruf | API/CLI und TUI-Geräteverwaltung implementiert und lokal getestet: Rollen, einmalige Enrollment-Codes, Runner-Scope, Widerruf einschließlich laufender SSE-/Long-Polls |
| 8 | OpenAI-/GitHub-/Forgejo-Zugangsverwaltung | Offen; lokale OpenCode-Anmeldung im Labor vorhanden |
| 9 | Clone/Fetch und Repository-Einrichtung | Lokale CLI clone/register/fetch mit atomarer Runner-Zuordnung implementiert; TUI-/Remote-Aufträge offen |
| 10 | Checkpoints und Laptop-/Server-Übergabe | Offen; abhängig von 1 |
| 11 | Review und Integration | Offen; Agent-Erfolg führt bereits zu Review |
| 12 | Projektgedächtnis und Kontextauswahl | Strukturierte Notizen und begrenzte Prompt-Übernahme; Supervisor sichert quellenverknüpfte Modellbeobachtungen vor Aktionen. Widerspruchs-/Branch-Gültigkeit und modellspezifischer Tokenizer offen |
| 13 | Supervisor | Modellbasierter Session-Supervisor mit Entscheidungsjournal, Bewertungsbudget, autonomer Compaction und Abschluss erfolgreicher Sessions implementiert; Ticketplanung, automatische Antworten und bestätigtes Beenden laufender Sessions offen |
| 14 | Android und Push | Offen; reales Gerät/Push-Zugang für Abnahme nötig |
| 15 | GitHub-Releases und Ansible-Vertrag | v0.1.1 veröffentlicht; Server-/Linux-TUI-Downloads lokal geprüft, Windows nativ in CI geprüft; Android-APK erst mit implementierter App |

Siehe `backlog.md` für Akzeptanzkriterien und `development.md` für bereits
implementierte Bedienabläufe. Zugangsdaten nicht in dieses Dokument eintragen.

# Umsetzung der 15 offenen Punkte

Quelle: Arbeitsauftrag des Betreibers, alle 15 offenen Punkte umzusetzen.
Ein Eintrag gilt erst nach implementierter Funktion und dokumentierter Abnahme
als abgeschlossen. Laborprüfungen ersetzen keine Abnahme auf dem Zielhost.

| Nr. | Arbeitspaket | Stand / Abnahme |
| --- | --- | --- |
| 1 | Bestätigte Pause und Session-Fortsetzung | Offen: Prozesscontainment; vorhandener Interrupt bleibt unbestätigt |
| 2 | Recovery ungeklärter Aufträge | Offen |
| 3 | Vollständiges Sessionarchiv | Teilweise: persistente begrenzte Auszüge vorhanden |
| 4 | SSE und Runner-Long-Poll | Implementiert und lokal getestet; TUI nutzt SSE, Runner Long-Poll |
| 5 | Deployment, Backup/Restore, Updates | Container, Vorlagen und Backup/Restore lokal getestet; Zielinstallation durch Betreiber |
| 6 | Echte Laptop-/Server-Abnahme | Alias rs9tm bekannt; erst nach Ansible-Deployment des Betreibers; kein Deployment durch Agent |
| 7 | Gerätezugang, Rollen, Widerruf | API/CLI implementiert und getestet, einschließlich laufender SSE-/Long-Polls; TUI-Dialog offen |
| 8 | OpenAI-/GitHub-/Forgejo-Zugangsverwaltung | Offen; lokale OpenCode-Anmeldung im Labor vorhanden |
| 9 | Clone/Fetch und Repository-Einrichtung | Offen; registrierte lokale Repositories vorhanden |
| 10 | Checkpoints und Laptop-/Server-Übergabe | Offen; abhängig von 1 |
| 11 | Review und Integration | Offen; Agent-Erfolg führt bereits zu Review |
| 12 | Projektgedächtnis und Kontextauswahl | Offen |
| 13 | Supervisor | Offen; abhängig von 12; einschließlich selbst entschiedener Compaction und Sessionbeendigung |
| 14 | Android und Push | Offen; reales Gerät/Push-Zugang für Abnahme nötig |
| 15 | GitHub-Releases und Ansible-Vertrag | Ziel farion/yoloforge; Release-Pipeline vorbereitet; Veröffentlichung/CI-Abnahme ausstehend |

Siehe `backlog.md` für Akzeptanzkriterien und `development.md` für bereits
implementierte Bedienabläufe. Zugangsdaten nicht in dieses Dokument eintragen.

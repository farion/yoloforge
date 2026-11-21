# Server-Erstabnahme rs9tm

Datum: 2026-11-21. Installation von v0.1.1 durch das Ansible des Betreibers.
Die Prüfung durch den Entwicklungsagenten war lesend; kein Deployment, Neustart
oder Konfigurationswechsel wurde ausgeführt.

- Container `yoloforge-factory-1`: healthy, Neustartzähler 0.
- Binary meldet Version 0.1.1; SHA-256 entspricht dem heruntergeladenen
  GitHub-Serverpaket:
  `9b297e605d60e9760b9efa62668d8da0d9f221251ef78da26a4d43dc06173dfc`.
- `/healthz`: 200 ready. Snapshot mit bestehendem Credential und Protokoll 1:
  200. Ohne Credential: 401. Das Credential blieb auf dem Server und wurde
  nicht ausgegeben oder lokal gespeichert.
- Port: ausschließlich `127.0.0.1:7432`.
- Container-UID/GID: 10001:10001, Root-Dateisystem read-only.
- Restart-Policy: unless-stopped.
- Daten: Bind-Mount `/opt/localbox/yoloforge/data` nach `/data`, beschreibbar.
- Bootstrap: read-only Bind-Mount, Eigentümer 10001, Modus 0400.
- SQLite: Schema 6, `PRAGMA quick_check` ergibt `ok`.
- Noch keine Projekte, Runner, Streams oder Aufträge zum Prüfzeitpunkt.

Die Erstinstallation ist damit bestätigt. Noch nicht nachgewiesen: Update und
Rollback durch Ansible, Restore auf dem Zielhost, Runner-Autostart, tatsächliche
Agentenarbeit und Ownership-Transfer zwischen Laptop und Server. Diese Prüfungen
sind nicht Voraussetzung für die weitere lokale Entwicklung.

Künftige Releases werden über GitHub bereitgestellt und durch das Ansible des
Betreibers installiert. Diese Abnahme ändert nicht die Zuständigkeit für Deployments.

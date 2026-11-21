# Projektgedächtnis (Entwicklungsstand nach v0.1.1)

Wissensnotizen sind unveränderliche Aussagen mit Titel, Inhalt, Quelle, optionalem
Quellauftrag, Projekt und optionalem Ticket. Die Factory validiert, dass Ticket
und Quellauftrag zum Projekt gehören. Freie Quellenangaben sind vom Operator
bereitgestellte Referenzen, kein automatisch verifizierter Beweis.

```sh
yoloforge knowledge add --project PROJEKT-UUID --title "Deployment" \
  --content "Installation erfolgt durch das Ansible des Betreibers." \
  --source "Betreiberentscheidung" --priority 9

yoloforge knowledge list --project PROJEKT-UUID
yoloforge knowledge context --project PROJEKT-UUID --ticket TICKET-UUID --budget-bytes 8192
yoloforge knowledge retire NOTIZ-UUID --expected-revision 1
```

`add` akzeptiert optional `--ticket`, `--source-command` und `--request-id`.
`retire` akzeptiert ebenfalls `--request-id`. Ein identischer Request kann sicher
wiederholt werden, ein veränderter Payload mit derselben ID wird abgewiesen.
Retirement erhält Inhalt und Quellen, erhöht die Revision und entfernt die Notiz
aus zukünftiger Kontextauswahl. Korrekturen erfolgen durch neue Notiz plus
Retirement der bisherigen Aussage. Automatische Widerspruchsauflösung fehlt noch.

Neue `run_agent`-Aufträge übernehmen automatisch aktive Projekt-Notizen und die
Notizen ihrer zugewiesenen Tickets. Die Auswahl erfolgt nach Priorität (0–9,
höher zuerst), dann neueste zuerst. Höchstens 256 passende Kandidaten werden
geprüft; vollständige Notizen inklusive Kennung, Revision und Quellenangabe werden
bis zur Grenze von 8192 UTF-8-Bytes aufgenommen. Nicht passende Notizen werden
übersprungen, Kürzungen innerhalb einer Notiz gibt es nicht. Die Zahl ausgelassener
passender Notizen wird angezeigt. Die Kontext-API erlaubt 0–16384 Bytes.

Dies ist eine harte **Bytegrenze für den Wissensblock**, noch kein genauer
modellspezifischer Tokenzähler. Prompt, Profil, Tools und bestehende Sessionhistorie
brauchen zusätzlich Kontext. Semantische Suche, automatische Extraktion und
Gesamtkontextplanung des Supervisors sind noch nicht implementiert.

Der zusammengesetzte Prompt wird innerhalb derselben SQLite-Transaktion wie der
Auftrag gespeichert. Spätere Retirements oder neue Notizen verändern keinen
bereits erteilten Auftrag. Die Ratatui-Auftragsdetails zeigen den erfassten
Wissensblock als Teil des Prompts. Ein eigener Wissenseditor in Ratatui folgt.

`list` zeigt die neuesten 200 Notizen einschließlich stillgelegter Notizen.
Alle erfolgreichen Änderungen erzeugen dauerhafte Ereignisse; die Notizen liegen
in einer eigenen Tabelle und sind nicht Teil der regelmäßig geladenen Snapshots.
Die DB-Migration erhöht das Schema auf Version 7. Vor einem Downgrade auf v0.1.1
ist dessen passende Sicherung nötig.

API:
- `POST /v1/knowledge`: anlegen (`CreateKnowledge`).
- `POST /v1/knowledge/{id}/retire`: revisionsgeschützt stilllegen.
- `GET /v1/projects/{id}/knowledge`: begrenzte Liste.
- `GET /v1/projects/{id}/context?ticket=UUID&budget_bytes=8192`: Auswahlvorschau.

Admin-/Operator-Credentials dürfen diese Endpunkte verwenden; Runner-Credentials
erhalten den erfassten Kontext ausschließlich über ihre zugeordneten Aufträge.

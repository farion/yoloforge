# Projektgedächtnis (Entwicklungsstand nach v0.1.1)

Wissensnotizen sind unveränderliche Aussagen mit Titel, Inhalt, Quelle, optionalem
Quellauftrag, Projekt, optionalem Ticket und optionalem exakten Branchnamen.
Die Factory validiert, dass Ticket und Quellauftrag zum Projekt gehören. Bei
branchgebundenen Notizen muss ein angegebener Quellauftrag ebenfalls von genau
diesem Workstream-Branch stammen. Freie Quellenangaben sind vom Operator
bereitgestellte Referenzen, kein automatisch verifizierter Beweis.

```sh
yoloforge knowledge add --project PROJEKT-UUID --title "Deployment" \
  --content "Installation erfolgt durch das Ansible des Betreibers." \
  --source "Betreiberentscheidung" --priority 9

yoloforge knowledge list --project PROJEKT-UUID
yoloforge knowledge show NOTIZ-UUID
yoloforge knowledge context --project PROJEKT-UUID --ticket TICKET-UUID --budget-bytes 8192
yoloforge knowledge retire NOTIZ-UUID --expected-revision 1

yoloforge knowledge add --project PROJEKT-UUID --branch factory/STREAM-UUID \
  --title 'Lokale Teststrategie' --content 'Tests auf diesem Branch mit ...' \
  --source 'Review des Workstreams'
yoloforge knowledge context --project PROJEKT-UUID --branch factory/STREAM-UUID

yoloforge knowledge add --project PROJEKT-UUID --branch factory/STREAM-UUID \
  --title 'Korrigierte Teststrategie' --content 'Stattdessen ...' \
  --source 'Neuere Review-Entscheidung' \
  --supersedes ALTE-NOTIZ-UUID --supersedes-revision 1
```

`add` akzeptiert optional `--ticket`, `--source-command`, `--branch`,
`--supersedes` mit `--supersedes-revision` und `--request-id`.
`retire` akzeptiert ebenfalls `--request-id`. Ein identischer Request kann sicher
wiederholt werden, ein veränderter Payload mit derselben ID wird abgewiesen.
Retirement erhält Inhalt und Quellen, erhöht die Revision und entfernt die Notiz
aus zukünftiger Kontextauswahl. Mit `--supersedes` erfolgen neue Notiz und
Stilllegung der bisherigen Aussage **atomar**. Projekt, Ticket und Branch-Scope
müssen gleich bleiben. Nur die aktuelle Revision einer aktiven Notiz kann ersetzt
werden; bei konkurrierenden Korrekturen gewinnt genau eine. `supersedes` und
`superseded_by` verknüpfen beide unverändert erhaltenen Inhalte. Automatische
semantische Widerspruchserkennung und -auflösung fehlen noch.

Neue `run_agent`-Aufträge übernehmen automatisch aktive Projekt-Notizen und die
Notizen ihrer zugewiesenen Tickets, jeweils nur projektweit gültige oder exakt
zum Workstream-Branch passende Notizen. Ohne `--branch` enthält die Vorschau
keine branchgebundenen Notizen. Es gibt weder Wildcards noch automatische
Vererbung vom Basisbranch: `main` und `factory/STREAM-UUID` sind verschiedene
Gültigkeitsbereiche. Die Auswahl erfolgt nach Priorität (0–9,
höher zuerst), dann neueste zuerst. Höchstens 256 passende Kandidaten werden
geprüft; vollständige Notizen inklusive Kennung, Revision und Quellenangabe werden
bis zur Grenze von 8192 UTF-8-Bytes aufgenommen. Nicht passende Notizen werden
übersprungen, Kürzungen innerhalb einer Notiz gibt es nicht. Die Zahl ausgelassener
passender Notizen wird angezeigt. Die Kontext-API erlaubt 0–16384 Bytes.

Dies ist eine harte **Bytegrenze für den Wissensblock**, noch kein genauer
modellspezifischer Tokenzähler. Prompt, Profil, Tools und bestehende Sessionhistorie
brauchen zusätzlich Kontext. Der Supervisor übernimmt inzwischen ausdrücklich
unbestätigte Modellbeobachtungen mit Quellreferenz, immer auf den Zielbranch
beschränkt. Seine Bewertung erhält bis zu 4096 Bytes Wissen für den Zielbranch
und dessen Tickets, nicht für den eigenen Supervisor-Worktree. Semantische Suche,
genaues Tokenbudget und umfassende Gesamtkontextplanung sind noch offen.

Der zusammengesetzte Prompt wird innerhalb derselben SQLite-Transaktion wie der
Auftrag gespeichert. Spätere Ablösungen, Retirements oder neue Notizen verändern keinen
bereits erteilten Auftrag. Die Ratatui-Auftragsdetails zeigen den erfassten
Wissensblock als Teil des Prompts. Ein eigener Wissenseditor in Ratatui folgt.

`list` zeigt die neuesten 200 Notizen einschließlich stillgelegter Notizen.
`show` liest eine bestimmte Notiz per ID auch außerhalb dieses Listenfensters;
damit lassen sich die Verweise auf Vorgänger und Nachfolger nachverfolgen.
Alle erfolgreichen Änderungen erzeugen dauerhafte Ereignisse; die Notizen liegen
in einer eigenen Tabelle und sind nicht Teil der regelmäßig geladenen Snapshots.
Branch-Gültigkeit und Ablösung benötigen DB-Schema 12. Alte kuratierte Notizen
ohne Branch bleiben projektweit gültig. Frühere automatische Supervisor-Notizen,
deren Gültigkeit noch nicht auf einen Branch beschränkt war, werden bei der
Migration mit erhöhter Revision stillgelegt: Inhalte, Quellverweise und
Bewertungsprovenienz bleiben erhalten; das Ereignis
`knowledge.retired_for_scope_migration` dokumentiert dies. Bereits gespeicherte
Auftragsprompts bleiben unverändert. Ältere Server verweigern dieses neuere Schema;
ein Downgrade benötigt die passende Sicherung.

API:
- `POST /v1/knowledge`: anlegen (`CreateKnowledge`), optional
  `branch: "factory/..."`, `supersedes: {id, expected_revision}`.
- `GET /v1/knowledge/{id}`: vollständige einzelne Notiz mit Provenienz.
- `POST /v1/knowledge/{id}/retire`: revisionsgeschützt stilllegen.
- `GET /v1/projects/{id}/knowledge`: begrenzte Liste.
- `GET /v1/projects/{id}/context?ticket=UUID&branch=BRANCH&budget_bytes=8192`: Auswahlvorschau.

Admin-/Operator-Credentials dürfen diese Endpunkte verwenden; Runner-Credentials
erhalten den erfassten Kontext ausschließlich über ihre zugeordneten Aufträge.

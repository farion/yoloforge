# Persistentes Nachrichtenarchiv

Entwicklungsstand nach v0.1.1, DB-Schema 8. Zusätzlich zu den begrenzten
TUI-Auszügen archiviert der Runner die vollständigen **projizierten Nachrichten**
des OpenCode-Message-Endpunkts. Es gibt keine 32-Nachrichten- oder 8-KiB-Textkürzung
im Archiv. Text, Tooldaten, Compaction-Nachrichten und weitere vom API gelieferte
JSON-Felder bleiben erhalten.

## Aufnahme und Abschluss

- Der Runner liest ab der ältesten Nachricht in Seiten zu maximal 16 Nachrichten.
  Opaque Cursor werden unverändert übernommen; Folgeaufrufe verwenden kein `order`.
- Jede Seite wird vor Upload im lokalen Journal gespeichert. Nach verlorener
  Bestätigung wird genau derselbe Batch wiederholt, bevor der Cursor weiterläuft.
- Der Server prüft Runner, Ownership-Epoche, Session-ID und lückenlose Batchrevision.
  Geänderte Wiederholungen derselben Revision werden abgewiesen.
- Unveränderte Nachrichten erzeugen keine neue Archivversion. Änderungen und
  spätere Rückkehr zu einem früheren Inhalt erzeugen jeweils neue Versionen.
  Frühere Versionen werden nicht gelöscht.
- Während der Ausführung wird der Verlauf wiederholt von vorne gelesen, sodass
  Änderungen an älteren Nachrichten ebenfalls erfasst werden können.
- Nach lokal festgehaltenem Ausführungsergebnis beginnt ein letzter Durchlauf ab
  der ältesten Nachricht. Erst nach dessen bestätigtem Upload meldet der Runner
  den Auftrag terminal. Bis dahin bleibt der Stream reserviert und zeigt
  gegebenenfalls „final session archive pending“.
- Service-/Netzwerkfehler führen nicht zu einem leeren oder vollständigen Archiv.
  Die nächste Runner-Runde versucht die Lese-/Uploadoperation erneut.

Ein Batch ist auf 8 MiB begrenzt. Eine größere Antwort wird **nicht gekürzt**;
die Archivierung bleibt stattdessen unvollständig. Aktuell gibt es für diesen
Fall keine automatische Aufteilung einzelner übergroßer Nachrichten. Auch ein
verschwundener OpenCode-Service kann deshalb den terminalen Factory-Abschluss
bis zur Wiederherstellung blockieren. Der Runner journalisiert das vorhandene
Ausführungsergebnis bereits vorher; es wird nicht erneut ausgeführt.

## Export

```sh
yoloforge archive AUFTRAG-UUID --output session.jsonl
```

Der Export verwendet eine neue private Datei und überschreibt keine vorhandene.
Die erste Zeile enthält Auftragskennung und Archivstatus, danach folgen gespeicherte
Nachrichtenversionen. Die letzte Zeile enthält `export_complete: true`, Versionszahl
und die festgehaltene Batchrevision. Ohne diesen Footer ist eine abgebrochene
Exportdatei unvollständig. Ein Retry verwendet einen neuen Ausgabepfad.

Die Exportrevision wird beim ersten Lesen eingefroren. Währenddessen neu
eintreffende Nachrichten werden erst vom nächsten Export erfasst. Der API-Cursor
ist die fortlaufende Archivsequenz; die zeitliche Reihenfolge der Nachrichten
selbst steht in deren OpenCode-Feldern. Für die letzte beobachtete Version einer
Nachricht die letzte Exportzeile mit derselben `message.id` verwenden.

API:
- `GET /v1/commands/{id}/archive?after=0&through_revision=N`
- `PUT /v1/runners/{runner}/commands/{id}/archive`
  mit Runner-Registrierung und `{ownership_epoch,batch}`.

## Vollständigkeitsgrenze

`final_scan_complete` bedeutet: Der letzte paginierte Durchlauf nach dem lokal
festgehaltenen Agentenergebnis wurde hochgeladen. Es ist keine Garantie gegen
nachträgliche externe Sessionänderungen oder gegen vom OpenCode-API selbst
weggelassene Daten. Binäre Anhänge und Objekte, auf die Nachrichten nur verweisen,
werden noch nicht separat heruntergeladen. Änderungen zwischen zwei Polls können
unbeobachtet bleiben. Native OpenCode-Datenbank/Dateien sind separat zu sichern.

Archivdaten können sensible Toolausgaben und Provider-Metadaten enthalten. Sie
werden weder in normale Snapshots noch als vollständiger Inhalt ins Eventlog
kopiert. Admin-/Operator-Credentials können exportieren; Runner dürfen nur die
Archive ihrer eigenen Aufträge schreiben.

Für vor Einführung des Archivs bereits abgeschlossene Aufträge erfolgt noch kein
automatischer Backfill. Die Ratatui-Auszugansicht bleibt eine schnelle begrenzte
Vorschau; das komplette projizierte Archiv ist über API und CLI zugänglich.

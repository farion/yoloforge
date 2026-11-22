# Automatische Session-Erkenntnisse

Server-Schema 27 journalisiert eine einmalige Auswertung je erfolgreichem,
schreibendem Agentauftrag mit finalem Archiv. Im Leerlauf verwendet der Scheduler
den gemeinsamen Read-only-Orchestrator mit Rolle `orchestrator-memory`.
Neue Aufgaben, offene Fragen und laufende Schreibaufträge haben Vorrang.

Der Auswerter erhält begrenzte Ausschnitte aus Aufgabenprompt und gemeldetem
Ergebnis, nicht das vollständige Archiv. Er liefert maximal 3500 UTF-8-Bytes
Beobachtungen oder einen leeren String. Ungültige/fehlgeschlagene Ergebnisse
werden protokolliert und nicht endlos erneut angefordert. Gespeicherte Notizen
sind branchlokal, Priorität 1, ausdrücklich modellabgeleitet und verweisen auf
Quellauftrag/Archiv sowie Auswertungsauftrag. Sie ersetzen keine bestehenden
Notizen. Routing berücksichtigt kurze Ausschnitte passenden Wissens.

Neustart, einmalige Speicherung, leere/ungültige Ergebnisse und Branchfilter sind
getestet. Realmodell-Abnahme mit Bildtask, fester Fortsetzung, automatischer
Frageantwort, Veröffentlichung und Wissen bestanden:
`scripts/smoke-prompt-image.py --local-registration PATH --memory`.
Semantische Widerspruchsauflösung, eigener Kostenhaushalt und modellspezifischer
Tokenizer sind weiter offen.

## Historische Archive

Runner laden in freien Poll-Slots unfertige Archive bereits gemeldeter lokaler
Journalaufträge nach. Vorhandene Outbox-Batches behalten ihre Identität; Cursor,
Fehlerzahl und Retry-Zeit werden journalisiert. Fehler führen zu begrenztem
exponentiellem Zeitabstand, nicht zur erneuten Agent-Ausführung. Bereits fertige
Archive bleiben unberührt. Der Test prüft Antwortverlust, Neustart und identisches
Batch-Replay. Nicht mehr erreichbare Sessions, verlorene Journale oder serverseitig
gesperrte/terminierte Quellen können so nicht rekonstruiert werden; Binäranhänge
sind noch nicht separat archiviert.

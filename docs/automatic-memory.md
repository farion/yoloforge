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
Quellauftrag/Archiv sowie Auswertungsauftrag. Routing berücksichtigt kurze
Ausschnitte passenden Wissens.

Bei einer klaren Korrektur/Widerspruch kann die Auswertung eine fruehere
modellabgeleitete Branch-Notiz abloesen. Angeboten werden maximal acht aktive
Notizen derselben Branch ohne Ticketscope, Prioritaet 1, mit nachgewiesener
Memory-Auswertung als Quelle. Die angebotenen IDs/Revisionen werden vor dem
Modellaufruf journalisiert. Das Modell muss eine Korrektur begruenden und weiter
gueltige Beobachtungen erhalten. Die Speicherung prueft Angebot, Revision, Scope
und Aktivitaet erneut und ersetzt atomar mit beidseitiger Provenienz. Eine
konkurrierende menschliche Ablösung/Stilllegung gewinnt; ungeeignete Ergebnisse
werden einmalig verworfen. Manuelle und projektweite Notizen sind nicht angeboten.

Neustart, einmalige Speicherung, leere/ungültige Ergebnisse und Branchfilter sind
getestet. Realmodell-Abnahme mit Bildtask, fester Fortsetzung, automatischer
Frageantwort, Veröffentlichung und Wissen bestanden:
`scripts/smoke-prompt-image.py --local-registration PATH --provider PROVIDER_ID --model MODEL_ID --memory`.
Branchlokale Widerspruchskorrektur samt Restart, unangebotener ID und menschlichem
Revisionsrennen ist synthetisch getestet. Ihre Realmodell-Abnahme, ein eigener
Kostenhaushalt und modellspezifischer Tokenizer sind weiter offen.

## Historische Archive

Runner laden in freien Poll-Slots unfertige Archive bereits gemeldeter lokaler
Journalaufträge nach. Vorhandene Outbox-Batches behalten ihre Identität; Cursor,
Fehlerzahl und Retry-Zeit werden journalisiert. Fehler führen zu begrenztem
exponentiellem Zeitabstand, nicht zur erneuten Agent-Ausführung. Bereits fertige
Archive bleiben unberührt. Der Test prüft Antwortverlust, Neustart und identisches
Batch-Replay. Nicht mehr erreichbare Sessions, verlorene Journale oder serverseitig
gesperrte/terminierte Quellen können so nicht rekonstruiert werden. Begrenzte
Binärsnapshots und Export sind unter [archive-files.md](archive-files.md) beschrieben.
## Realmodell-Korrekturabnahme

`scripts/smoke-prompt-image.py --local-registration PATH --provider PROVIDER_ID
--model MODEL_ID --memory-correction` prueft eine echte Konventionsaenderung:
Proof-Dateien zuerst kleingeschrieben, danach explizit grossgeschrieben. Das Modell
extrahiert die Korrektur, ersetzt die angebotene alte Branch-Notiz und erhaelt
bidirektionale Provenienz. Mit `openai/gpt-6.2-sol` bestanden; lokale Belege:
`/tmp/opencode/factory-image-751slsmk`.

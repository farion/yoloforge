# Bestätigte Runtime-Beendigung

API und CLI können einen aktiven Container-Auftrag beenden. Voraussetzung ist
eine bereits vom Runner gespeicherte Runtime-Bindung (bei älteren Ausführungen
ein Containment-Nachweis). Die Anfrage bindet
Auftrag, Streamrevision, vollständige Container-ID und Startzeit:

```sh
yoloforge terminate COMMAND_ID \
  --expected-stream-revision REVISION \
  --container-id FULL_CONTAINER_ID --started-at STARTED_AT \
  --reason 'Ausführung wird nicht mehr benötigt' --request-id REQUEST_UUID
yoloforge termination COMMAND_ID
```

Bei unklarer HTTP-Zustellung dieselbe Anfrage mit derselben Request-ID wiederholen.
Ein neuer Versuch nach `unknown` braucht eine neue Request-ID, aktuelle Revision
und `--previous-request OLD_REQUEST_UUID`. Ereignisse und Runner-Receipts erhalten
die Historie; die Statusabfrage zeigt den neuesten Versuch.

Vor dem ersten Session-/Prompt-Schreibzugriff veröffentlicht der Runner seine
Runtime-Bindung unveränderlich in Factory. Nach Verlust des lokalen Journals
kann ausschließlich die Beendigung diese zentrale Bindung verwenden; Prompt-
Zustellung und Session-Fortsetzung werden daraus nicht rekonstruiert.

Der Runner prüft seine persistierte Bindung einschließlich Docker-Daemon-ID,
Container-ID, Startzeit, Workspace und Runtime-Konfiguration. Vor `docker rm
--force` schreibt er die Absicht ins Journal. Derselbe Versuch führt den Befehl
niemals erneut aus. Nach Neustart oder verlorener Antwort prüft er ausschließlich,
ob die vollständige ID auf demselben Daemon fehlt. Auch eine bereits extern
entfernte Runtime ist so abschließbar. Volumes und Worktree bleiben erhalten.

Erst nach bestätigter Entfernung werden Auftragsergebnis (`failed`), Freigabe
der Scheduling-Reservierung, offene Fragen und Ereignisse atomar gespeichert.
Der Stream bleibt `blocked`; ein neuer Auftrag braucht eine neu konfigurierte
Runtime. Offene Compactions erhalten `aborted`, ohne einen nativen Erfolg oder
Fehlschlag zu behaupten. Archivexport und TUI-Archivbrowser kennzeichnen die
Beendigung und mögliche Unvollständigkeit. Es gibt noch keinen TUI-Dialog zum
Anfordern dieser Beendigung.

## Grenzen und Versionierung

- Server zuerst aktualisieren: DB-Schema 20, Runner-Journal 6,
  Capability `runtime-termination-v1`.
- Legacy-Bindungen ohne gespeicherte Daemon-ID werden nicht automatisch adoptiert.
- Fehlen sowohl Journal als auch zentrale Bindung, bleibt der Auftrag reserviert.
  Geänderter Daemon oder neu gestarteter Container erlauben keine Freigabe.
  Native Interrupt-/Prompt-Recovery bleibt offen.
- Beendet werden Prozesse der dedizierten Runtime; bereits ausgelöste externe
  Dienste werden dadurch nicht abgebrochen. Checkpoint/Ownership-Handoff bleibt offen.

## Lokale Abnahme

Store-Tests prüfen atomaren Rollback, Request-Replay, Revision/Epoch/Runner-Bindung,
explizite Retry-Provenienz, Archiv-Erhalt und Abschluss nach Serverneustart.
Der reale Docker-Test prüft signalresistente, doppelt geforkte `setsid`-Writer,
Entfernung im eingefrorenen Zustand, Schreibstillstand, Journalneustart,
verlorene Ergebnis-Persistierung und Intent ohne ausgeführten Docker-Aufruf:

```sh
cargo test -p factory-runner --locked real_freezer -- --ignored --nocapture
```

Die Zielhost-Abnahme der Beendigung steht noch aus.
Die Entfernung nach erfolgreichem Realmodell-Lauf und anschließende
Checkpoint-Erstellung sind inzwischen mit `scripts/smoke-contained-agent.py`
lokal geprüft; die autonome Beendigungsentscheidung selbst ist weiterhin nur
mit synthetischen Modellentscheidungen geprüft.

Bereits abgeschlossene letzte Agentenaufträge können ebenfalls bereinigt werden.
Der Runner verarbeitet diese über eine separate Cleanup-Queue; neues Scheduling
bleibt währenddessen gesperrt. Ergebnis und abgeschlossenes Archiv bleiben
unverändert. Eine Fortsetzung der entfernten Runtime wird abgewiesen. Für eine
neue Ausführung muss eine neue Runtime eingerichtet werden.

## Autonomer Supervisor

Der Session-Supervisor kann nun `terminate` entscheiden. Voraussetzung ist eine
bereits bei der Modellbewertung vorhandene zentrale Runtime-Bindung; die Antwort
darf keine Container-ID wählen. Streamrevision und Policyrevision bleiben geprüft.
Unverifizierte Wissensnotiz, Entscheidung und Beendigungsanfrage werden atomar
gespeichert. Ohne passenden Runner-Nachweis bleibt der Auftrag reserviert.
Unklare Beendigungen werden nicht automatisch erneut ausgelöst.
Beim autonomen `close` einer erfolgreichen Session wird eine vorhandene zentral
gebundene Runtime ebenfalls zur Entfernung vorgemerkt. Schließung, Wissensnotiz
und Cleanup-Anforderung sind atomar; der ursprüngliche Erfolg bleibt erhalten.
Store-Tests prüfen modellförmige Entscheidungen und Neustart/Deduplizierung;
eine echte Modellabnahme dieser neuen Entscheidung steht noch aus.

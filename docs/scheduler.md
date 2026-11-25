# Abhängigkeitengesteuerte Ticketplanung

Projekte sind standardmäßig aktiv. In der TUI unter `3` → `n` einen Prompt
eingeben und mit Enter absenden; Shift-/Alt-Enter fügt eine neue Zeile ein.
Der Prompt-Editor hat keine Formularschritte: Ctrl-V fügt ein Bild oder Text aus
der lokalen Zwischenablage ein, alternativ eine Bilddatei ins Terminal ziehen.
Anhänge erscheinen direkt unter dem Text; Ctrl-D entfernt den letzten Anhang.
Ctrl-S bleibt als Sende-Kürzel verfügbar. Nach bestätigtem Versand schließt der
Editor automatisch; bei unklarem Versand bleibt der unveränderliche Entwurf erhalten.
Titel, Priorität, Statuswahl und Scheduler-Aktivierung entfallen.
Neue Projekte erlauben standardmäßig sechs gleichzeitige Arbeitssessions.
Ein kompatibler Runner richtet die Standardplanung ein;
ohne explizite Provider-/Modell-ID wird kein Agent gestartet. Neue Prompts
aktivieren auch eine zuvor konfigurierte Legacy-Planung.
Ein freigegebener Intent weckt den Scheduler direkt nach dem Speichern.
Entwürfe starten keine Planung. Der periodische Lauf bleibt für spätere
Wiederholungen und Zustandswechsel erhalten.
Bei automatischer Runner-Zuordnung kann Factory für neue Intents auf einen
passenden Online-Runner wechseln. Für lokale Projektpfade braucht der Runner
eine Projektbindung oder muss auf demselben Rechner laufen. Bestehende Sessions
und manuelle Runner-Zuordnungen werden nicht verschoben.
Unter `2` → Projekt → Leertaste lässt sich das Projekt pausieren/fortsetzen.
Die Pause verhindert neue Starts einschließlich noch nicht angenommener Aufträge;
bereits angenommene Arbeit läuft weiter. Neue Prompts werden währenddessen gespeichert.

Die folgenden API/CLI-Steuerungen bleiben für bestehende Tickets verfügbar.
Tickets in `backlog` oder `ready`, deren Abhängigkeiten ausdrücklich `done` sind,
werden berücksichtigt. `cancelled`
erfüllt keine Abhängigkeit. Abhängigkeiten müssen im selben Projekt liegen;
Zyklen und nachträgliche Änderungen bereits zugeordneter Tickets werden abgewiesen.

TUI: `2` → Projekt → `s` für Aktivierung, Runner-Auswahl und Parallelität;
`3` → Ticket → `d` für Abhängigkeiten. Bei einem Revisionskonflikt übernimmt
`Ctrl-R` unveränderte Felder aus dem aktuellen Serverstand.
Ein einzelner Projekt-Runner wird automatisch vorbelegt; ohne Projektbindung
gilt das auch für einen einzigen registrierten Runner. Bei mehreren Kandidaten
erfolgt die Auswahl mit Leertaste. Fehler benennen das konkret ungültige Feld.

```sh
yoloforge ticket dependencies TICKET --expected-revision REV \
  --depends-on PREREQUISITE
yoloforge scheduler PROJECT
yoloforge scheduler-set PROJECT --expected-revision 0 --enabled \
  --runner RUNNER_A --runner RUNNER_B --max-parallel 2
```

Zum Deaktivieren dieselbe Konfiguration mit aktueller Revision und ohne
`--enabled` senden. Bereits eingereihte Aufträge werden dadurch nicht abgebrochen.
`--request-id` ermöglicht unverändertes Replay.

Der Scheduler priorisiert dringende Tickets und verteilt sie auf die konfigurierten
Runner mit den wenigsten aktiven Streams. Eine Zuweisung legt atomar einen
Stream und Worktree-Auftrag an und setzt das Ticket auf
`in_progress`. Der gespeicherte Tickettext wird nach erfolgreicher Vorbereitung
genau einmal als Agentenauftrag eingereiht. Serverneustarts und konkurrierende
Ticks erzeugen keine zweite Zuweisung oder einen zweiten Start.

Das Parallelitätslimit zählt auch manuelle Streams mit offenen Tickets und
laufende Aufträge. Bei Prompt-Aufgaben gibt ein finales Ergebnis den Planungsplatz
frei, ohne manuelles Abschließen. Laufende Rückfragen belegen ihn weiterhin.
Das gilt auch für alte automatisch eingeplante Tickets; manuelles Done ist zum
Freigeben des Planungsplatzes nicht erforderlich.
Agentenerfolg ist weiterhin kein Nachweis einer beendeten Runtime. Bei lokalen
Projektordnern uebernimmt der Runner erfolgreiche Ergebnisse automatisch, sofern
der Zielstand unveraendert und sauber ist (`result-publication.md`).
Offline-Runner behalten ihre persistente Queue; es gibt keine automatische
Umverteilung oder Wiederholung fehlgeschlagener Ausführungen. Der Scheduler
provisioniert keine Container und beantwortet keine Rückfragen. Dafür gelten
weiterhin die vorhandenen Runner-Voraussetzungen beziehungsweise offene Pakete.

Lokal geprüft: Abhängigkeitszyklen, Priorisierung, Verteilung, Parallelitätslimit,
konkurrierende Datenbankverbindungen und Neustart zwischen Vorbereitung
und Agentenstart (`cargo test -p factoryd --test scheduler --locked`).

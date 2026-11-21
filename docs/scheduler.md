# Abhängigkeitengesteuerte Ticketplanung

Der Factory-Dienst plant aktivierte Projekte automatisch. Voraussetzung ist ein
konfiguriertes Provider-/Modellprofil. Nur Tickets im Status `ready`, deren
Abhängigkeiten ausdrücklich `done` sind, werden berücksichtigt. `cancelled`
erfüllt keine Abhängigkeit. Abhängigkeiten müssen im selben Projekt liegen;
Zyklen und nachträgliche Änderungen bereits zugeordneter Tickets werden abgewiesen.

```sh
yoloforge ticket dependencies TICKET --expected-revision REV \
  --depends-on PREREQUISITE
yoloforge scheduler PROJECT
yoloforge scheduler-set PROJECT --expected-revision 0 --enabled \
  --runner RUNNER_A --runner RUNNER_B --max-parallel 2 --start-budget 10
```

Zum Deaktivieren dieselbe Konfiguration mit aktueller Revision und ohne
`--enabled` senden. Bereits eingereihte Aufträge werden dadurch nicht abgebrochen.
`--request-id` ermöglicht unverändertes Replay. Eine Neukonfiguration ersetzt das
verbleibende Startbudget; dessen Verbrauch erhöht ebenfalls die Policyrevision.

Der Scheduler priorisiert dringende Tickets und verteilt sie auf die konfigurierten
Runner mit den wenigsten aktiven Streams. Eine Zuweisung verbraucht einen Start,
legt atomar einen Stream und Worktree-Auftrag an und setzt das Ticket auf
`in_progress`. Der gespeicherte Tickettext wird nach erfolgreicher Vorbereitung
genau einmal als Agentenauftrag eingereiht. Serverneustarts und konkurrierende
Ticks erzeugen keine zweite Zuweisung oder einen zweiten Start.

Das Parallelitätslimit zählt auch manuelle Streams mit offenen Tickets und
laufende Aufträge. Fehler, Fragen und Review bleiben belegt, bis der Betreiber
die Tickets abschließt; Agentenerfolg markiert Tickets nicht als erledigt.
Offline-Runner behalten ihre persistente Queue; es gibt keine automatische
Umverteilung oder Wiederholung fehlgeschlagener Ausführungen. Der Scheduler
provisioniert keine Container und beantwortet keine Rückfragen. Dafür gelten
weiterhin die vorhandenen Runner-Voraussetzungen beziehungsweise offene Pakete.

Lokal geprüft: Abhängigkeitszyklen, Priorisierung, Verteilung, Parallelitätslimit,
Budget, konkurrierende Datenbankverbindungen und Neustart zwischen Vorbereitung
und Agentenstart (`cargo test -p factoryd --test scheduler --locked`).

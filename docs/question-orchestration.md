# Rückfragen und Berechtigungen

Formularfragen haben eine feste ID. Der Intent wartet, bis der Nutzer antwortet
und der Runner die Antwort an OpenCode zustellt. Der Orchestrator beantwortet
keine Formularfragen, auch nicht bei bereits laufenden Modellauswertungen.
Ein interner Statuswechsel ändert weder die Frage noch den Antwortentwurf.
Eine bestätigte Antwort bleibt abgeschlossen. Neue Rückfragen brauchen eine
andere ID.

Antworten werden dauerhaft eingereiht. Der Runner vergleicht vor der Zustellung
die Frage und den aktiven Lauf mit OpenCode. Wenn OpenCode die Frage entfernt
oder unter derselben ID verändert, wird nicht auf ein anderes Formular geantwortet.
Eine abgewiesene Eingabe kann für dieselbe Frage korrigiert werden.

## Berechtigungen

Der Scheduler kann Berechtigungsanfragen einmalig und schreibgeschützt auswerten.
Die Auswertung läuft außerhalb des Limits arbeitender Sessions. Projektpause und
Stream-Steuerung werden respektiert.

- Der Orchestrator verwendet nur `once` oder `reject`, nie `always`. Bei
  unzureichendem Kontext überlässt er die Entscheidung dem Nutzer.
- Die Zustellung nutzt denselben Weg wie eine Nutzerentscheidung. Eine bereits
  eingereihte Nutzerentscheidung wird nicht überschrieben.
- Die Diagnose zeigt `orchestrator-question`. Fragen des Orchestrators werden
  nicht rekursiv ausgewertet.

## Supervisor

Der projektbezogene Orchestrator richtet den Session-Supervisor ein, sofern
keine explizite Konfiguration existiert (Standard: 12 Auswertungen, mindestens
5 Minuten Abstand). Der Supervisor greift nicht in eine Ausführung ein, solange
sie auf eine Antwort oder deren Zustellung wartet. Bestehende deaktivierte oder
benutzerkonfigurierte Richtlinien bleiben unverändert.

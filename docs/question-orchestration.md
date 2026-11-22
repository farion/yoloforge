# Rueckfragenorchestrierung (Schema 26)

Der aktive Projekt-Scheduler behandelt jetzt auch offene Fragen aus Arbeitssessions.
Eine Frage blockiert keinen freien Orchestrator-Platz: dessen schreibgeschuetzte
Auswertung laeuft ausserhalb des Limits arbeitender Sessions. Projektpause und
Stream-Steuerung werden respektiert.

Mit dem projektbezogenen Orchestrator wird auch der vorhandene Session-Supervisor
automatisch eingerichtet, sofern noch keine explizite Supervisor-Konfiguration
existiert. Standard: 12 Auswertungen, mindestens 5 Minuten Abstand. Aufgaben und
Rueckfragen haben Vorrang. Erfolgreiche Sessions erhalten mindestens 15 Minuten
seit dem finalen Archiv, bevor automatische Abschlussbewertungen beginnen.
Offene Aufgaben/Fragen verhindern, dass eine bereits laufende Bewertung die
benoetigte Session schliesst oder terminiert. Bestehende deaktivierte oder
benutzerkonfigurierte Supervisor-Policies werden nicht ueberschrieben.

Das Supervisor-Budget ist begrenzt und wird nicht automatisch aufgefuellt; es
begrenzt die Sessionpflege, nicht Aufgabenrouting oder Rueckfragenantworten.

## Ablauf

- Pro Frage wird genau eine Auswertung mit Frage-Revision, Modellauftrag und
  dauerhafter Antwort-Request-ID gespeichert.
- Das Modell erhaelt Arbeitsauftrag, Frage und branch-/aufgabenbezogenes
  Projektwissen. Es entscheidet zwischen einer begruendeten Antwort und der
  Weitergabe an den Benutzer.
- Formantworten werden auf Feldschluessel, Typen, Pflichtfelder, Optionen,
  Zahlen-/Laengen-/Auswahlgrenzen geprueft. Externe Anmeldung, bedingte Felder
  sowie Pattern-/Formatfelder werden derzeit nicht automatisch beantwortet.
- Berechtigungsantworten sind auf `once` oder `reject` begrenzt. Es entsteht keine
  dauerhafte `always`-Freigabe durch den Orchestrator.
- Die Antwort wird ueber dieselbe persistente Zustellstrecke wie eine Benutzerantwort
  eingeplant. Modellabschluss ist keine Zustellbestaetigung.
- Geaenderte Fragen, zwischenzeitliche Benutzerantworten und nicht mehr passende
  Ausfuehrungen verwerfen die Entscheidung. Fehler eskalieren statt Modellschleifen
  oder wiederholter externer Antworten zu erzeugen.

Die Diagnose zeigt `orchestrator-question`; die Fragenliste zeigt die Begruendung
in der Detailspalte. Zu grosse Fragen bleiben direkt beim Benutzer. Fragen des
Orchestrators selbst werden nicht rekursiv orchestriert.

Store-Abnahme: automatische Antwort, menschliche Konkurrenzantwort, Eskalation,
ungueltige Auswahl, Neustart und genau einmal eingeplante Antwort bestanden.
Die bestehenden Runner-Tests pruefen weiterhin die Wiederaufnahme unklarer
externer Antworten ohne zweiten POST. Echte Modellabnahme mit Folgeprompt in
derselben Session, Frage nach einer vorgegebenen Farbe, automatischer Antwort
und veroeffentlichtem Folgeergebnis bestanden. OAuth- und autonome
Compaction-/Termination-Abnahme sind davon nicht abgedeckt.

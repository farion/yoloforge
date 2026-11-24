# Prompt-Zuordnung und Sessionlimit

Implementierung: Server-Schema 25. Neue Prompt-Aufgaben (`SaveTask`) werden
automatisch geroutet; bestehende technische Tickets behalten ihren bisherigen
Scheduler-Ablauf.

## Bedienung

- Im Prompteditor schaltet **Ctrl-G** zwischen „Automatisch durch Orchestrator“
  und den Arbeitssessions des Projekts um.
- Eine ausgewählte Session ist fest gebunden. Belegte Sessions warten; fehlendes
  Abschlussarchiv, geändertes Profil oder eine geschlossene Session werden nicht
  durch eine neue Session umgangen.
- In der Projektliste öffnet **l** das Limit gleichzeitig arbeitender Sessions
  (1–16, Standard 1). Bereits akzeptierte Ausführungen werden durch eine Absenkung
  nicht unterbrochen; weitere Starts warten. Kurze schreibgeschützte
  Orchestrator-/Supervisor-Auswertungen zählen nicht als Arbeitssession.
- Die Diagnose kennzeichnet Modelläufe als `orchestrator` beziehungsweise
  `supervisor`. Routing-Begründungen stehen bei noch nicht zugeordneten Aufgaben.

## Ausführung

### Archivierung technischer Entscheidungssessions

Orchestrator-, Rückfragen-, Memorizer- und Supervisor-Auswertungen verwenden kurze,
eigene OpenCode-Sessions. Nach bestätigter Ergebnisübernahme und vollständiger
Archivübertragung archiviert der Runner diese Sessions auch in OpenCode.
Laufende Auswertungen und Sessions mit offenen Eingaben bleiben unverändert;
Arbeitssessions werden nicht automatisch archiviert. Verläufe werden nicht gelöscht.
Der Leerlauf-Nachlauf holt dies auch für ältere, bereits bestätigte Aufträge aus
dem Runner-Journal nach. Fehler werden mit begrenztem Retry-Abstand erneut versucht;
ein Neustart oder eine verlorene Bestätigung löst keine erneute Agentenausführung aus.

Die GUI bietet getrennte Projektseiten **Orchestrator**, **Supervisor** und
**Memorizer**, jeweils
mit **Offen** und **Archiv**. Die Sessionlisten und Instanzzähler sind nach Rolle
getrennt; Rückfragenauswertungen gehören zum Orchestrator, Wissensextraktion zum Memorizer.
Terminale Auswertungen erscheinen im Archiv, unabhängig davon, ob die native
OpenCode-Archivierung bereits erreichbar war. Auch fehlgeschlagene Auswertungen
bleiben dort lesbar; Suchfeld und Zähler gelten für den jeweiligen Tab.
Verteilung, Sessionlimit, Supervisor-Konfiguration und Supervisorhistorie liegen
unter **Erweiterte Projekteinstellungen**, nicht oberhalb der Sessionlisten.

Ohne feste Auswahl erzeugt der Scheduler einen dauerhaften, schreibgeschützten
OpenCode-Auftrag im projektbezogenen Orchestrator-Arbeitsbereich. Das Modell erhält
Prompt, Bilder und höchstens 16 aktuelle Sessionkandidaten mit Ziel und Zusammenfassung.
Es entscheidet `new`, `session` oder `wait`. Eine Sessionentscheidung muss sowohl im
ursprünglichen Kandidatenangebot als auch im aktuellen Angebot enthalten sein.
Die eigentliche Fortsetzung prüft zusätzlich Besitzer, Epoche, Profil,
Abschlussarchiv und Lifecycle-Sperren.

Auswertung und Aufgabenübergabe werden transaktional gespeichert. Ein Neustart
wiederholt keine bereits gespeicherte Entscheidung. Ungültige/fehlgeschlagene
Auswertungen und `wait` werden frühestens nach 30 Sekunden erneut bewertet;
bei unverändertem Kontext endet die automatische Neubewertung nach 12 Versuchen.
Eine Änderung des Kandidatenkontexts erlaubt weitere Bewertungen. Die konkrete
Ausführung bleibt am Ticket gespeichert, damit Folgeprompts in derselben Session
nicht die Ergebniszuordnung älterer Aufgaben überschreiben.

## Abnahme und verbleibender Umfang

Store-Tests prüfen feste und modellgewählte Fortsetzung, Warten auf Abschlussarchiv,
fehlgeschlagene/ungültige Entscheidungen, Neustart, unveränderte Request-Wiederholung,
Bildweitergabe und persistente Limits. TUI-Tests prüfen eingefrorene Sessionauswahl
bei unklarer Zustellung. Echte Modell-End-to-End-Abnahme mit Bildaufgabe,
Orchestrator-Zuordnung, fester Sessionfortsetzung, automatischer Rueckfrage und
Ergebnisveroeffentlichung bestanden (`smoke-prompt-image.py --local-registration
... --followup`); Nachweise unter `/tmp/opencode/factory-image-4nc2soic`.

Dies ist die Aufgabenrouting-Komponente. Rueckfragenbeantwortung wurde mit Schema 26
ergaenzt (`question-orchestration.md`). Lokaler Anmeldeeinstieg und SSH-Sessions
sind in `project-readiness.md` und `ssh-sessions.md` dokumentiert.
Der Supervisor wird am Orchestrator automatisch eingerichtet, mit begrenztem
Budget und Vorrang fuer Aufgaben/Fragen. Erweiterte Kostenkontrolle,
vollstaendiger Remote-Login, erweiterte Wissensextraktion/Anhangarchivierung,
Hosting-Integration und Android/Push sind dadurch noch nicht abgeschlossen.

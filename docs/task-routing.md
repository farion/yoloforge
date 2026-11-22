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
bei unklarer Zustellung. Eine echte Modell-End-to-End-Abnahme des neuen Routings
steht noch aus.

Dies ist die Aufgabenrouting-Komponente. Automatische Rückfragenbeantwortung,
ereignisgesteuerte Supervisor-Integration, Kostenkontrolle, Provider-/MCP-Login,
nativer Remote-Einstieg, erweiterte Wissensextraktion/Anhangarchivierung,
Hosting-Integration und Android/Push sind dadurch noch nicht abgeschlossen.

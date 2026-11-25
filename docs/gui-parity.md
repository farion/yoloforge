# GUI-Funktionszuordnung

Diese Matrix ordnet die bisherigen TUI-Verwaltungswege dem nativen Client zu.
CLI/TUI bleiben vorhanden. Native OpenCode-Interaktion wird entsprechend der
Anforderung nicht nachgebaut; die neue Sessionansicht ist bewusst schreibgeschützt.

| Bereich / bisheriger Ablauf | GUI-Einstieg |
| --- | --- |
| Serverwahl, Token, Geräteeinladung einlösen | Verbindungsdialog |
| Projekte anlegen und einrichten | Projekt verwalten → Projekt anlegen / Einrichtung |
| Metadaten, Repository, Basisbranch | Projekt verwalten → Metadaten bearbeiten |
| Modell, Provider, Ausführungsprofil | Projekt verwalten → Modell und Ausführungsprofil |
| Runnerbindung, Servicepfade, Harness/MCP | Projekt verwalten → Einrichtung / Runner / Harness; Spezialwerte als JSON |
| Prompt, Bilder, feste Sessionzuordnung | + Aufgabe → Aufgabendialog |
| Lokaler Entwurf / Serverentwurf / Freigabe | Aufgabendialog; Serverentwurf startet nicht |
| Tickets, Kriterien, Priorität, Status | Technisches Ticket anlegen; Aufgabendetails → Ticket bearbeiten |
| Abhängigkeiten | Aufgabendetails → Abhängigkeiten |
| Anzahl gleichzeitig arbeitender Sessions | Projekteinstellungen → Limits |
| Projektpause | Projekt verwalten → Projekt pausieren / fortsetzen |
| Sessionlimit | Scheduler-/Projektkonfiguration, aktuelle Grenze im Orchestrator |
| Streams erstellen und prüfen | Projekt verwalten → Arbeitsbereich anlegen / Arbeitsbereiche und Aufträge |
| PrepareWorkspace / OpenCode-Prüfauftrag | Arbeitsbereichdetails / Verwaltungsmenü |
| Formulare / Permissions | Fragen → gemeinsame Fragegruppe öffnen |
| Fragenhistorie und Eskalationsgrund | Fragen, inklusive erledigter Fragen; Originaldaten im Dialog |
| Sessionauszüge und vollständiges Archiv | Schreibgeschützter OpenCode-Tab, Versionsansicht und Originaldaten |
| Originale OpenCode-TUI öffnen | Bewusst ersetzt durch native Lesetabs; keine Chat-/Terminalsteuerung |
| Archiv-/Dateiexport | OpenCode-Tab → vollständiger Export / Dateiarchiv |
| Pause und Resume | Session-/Auftragsverwaltung → Arbeitsbereich pausieren / fortsetzen |
| Kontrollwiederholung mit Containerbeweis | Verwaltungsmenü, wenn entsprechender bestätigter Beweis vorliegt |
| Fortsetzung nach erfolgreicher/unterbrochener Ausführung | Neue Aufgabe mit fester Sessionbindung; nötige Freigabe über Arbeitsbereichsteuerung |
| Sessionabschluss und Compaction | Session-/Auftragsverwaltung |
| Dedizierte Runtime beenden | Runtime-Identität laden → gezielt bestätigte Runtime beenden |
| Checkpoint, Review, Integration, Übergabe | Session-/Auftragsverwaltung; geeigneten Auftrag im Auswahlfeld wählen |
| Ergebnisveröffentlichung / Hosting | Geprüftes Ergebnis veröffentlichen → lokaler/SSH-Runner-Helfer |
| Repository-Fetch | Auftragsverwaltung → Repository-Refs abrufen |
| Native Provider-/MCP-Readiness | Auftragsverwaltung → Modell-/Provider-Bereitschaft prüfen |
| SSH-Profile | Runner → SSH-Verbindung konfigurieren; gemeinsam genutztes Profilformat mit TUI |
| Supervisor-Konfiguration und Historie | Orchestrator; Konfiguration aus aktueller Policy oder Verwaltungsmenü |
| Wissen anlegen / lesen / ablösen / Kontext | Projektverwaltung → Wissen / Wissenskontext |
| Nutzung, Kosten, Tokenbudget | Projektverwaltung → Nutzung und Kosten / Tokenbudget |
| Geräte, Rollen, Einladungen, Widerruf | Factory-Verwaltung → Geräte und Zugänge / Gerät einladen |
| Auftragsdetails und Factory-Ereignisse | Aufgaben-/Sessiondetails und globale Diagnose |
| Unklare Schreibantworten | Offene Zustellungen; unveränderte Request-ID und Payload |
| Revisionkonflikte / lokale Eingaben erhalten | Verwaltungs- und Fragen-Dialoge; expliziter Vergleich und erneute Freigabe |

## Zusätzliche GUI-Funktionen

- Allgemeine Eingabedateien, nicht nur Bilder, mit Capability-Fencing älterer Runner.
- Frisch bestätigte OpenCode-Servicepräsenz und getrennte logische Sessionzustände.
- Dauerhafte globale Benachrichtigungen mit Lesestatus und Direktlinks.
- Einstellbare Desktop-Meldungen und In-App-Hinweise.
- Start einer separaten lokalen Factory aus der GUI heraus.

## Grenzen der Zuordnung

Die Matrix ist kein Nachweis erfolgreicher externer Provider-Anmeldung oder
Hosting-Veröffentlichung auf einem konkreten Betreiberhost. Die bestehenden
Sicherheitsprüfungen und Voraussetzungen der Runner-Helfer gelten unverändert.
Eine neue OAuth-Verwaltung oder ein Terminaleditor werden nicht vorgespiegelt.
Komplexe Harness-/Spezialwerte sind zunächst strukturierte/JSON-Felder im Dialog,
keine neue grafische Darstellung jeder beliebigen OpenCode-Konfiguration.

# Umsetzungsplan: native YoloForge-GUI

Status: **Implementierung vorhanden, Entwicklungsprüfungen werden erweitert**.
Start und Bedienung: [GUI](gui.md). Zuordnung bestehender Funktionen:
[Funktionsparität](gui-parity.md). Die ursprünglichen Checkboxen unten bleiben
als detaillierte Prüfliste erhalten und sind keine pauschale Fertigmeldung.
Grundlage sind die gemeinsam
geklärten Anforderungen und die visuelle Referenz des Nutzers. Dieser Plan
ergänzt den bestehenden Softwarestand; er ersetzt nicht den historischen Backlog.

## 1. Verbindliches Zielbild

- Native Desktop-Anwendung in Rust mit egui und SDL3, zunächst für Linux.
  SDL3 ersetzt eframe/winit, damit Datei-Drag-and-drop nativ unter Wayland
  einschließlich sichtbarem Drag-Hover funktioniert, ohne XWayland-Zwang.
- Nur Dark Theme, deutsche Bedienoberfläche. Originaltexte von Agenten, Tools
  und technischen Ausgaben bleiben unverändert.
- Funktional vollständiger Ersatz der bisherigen TUI. TUI und CLI bleiben
  kompatibel vorhanden; keine Entfernung bestehender Einstiege.
- Lokaler Komplettstart und Verbindung zu einem vorhandenen lokalen oder
  entfernten Factory-Server. Keine Installation auf Betreiberhosts durch die GUI.
- Der erste Tab ist die Factory-Verwaltung. Weitere Tabs im selben Hauptfenster
  zeigen OpenCode-Sessions schreibgeschützt. Keine externen OpenCode-Fenster,
  eingebetteten Terminals oder Webviews für die Sessionansicht.
- Die OpenCode-Ansicht sendet keine Prompts und enthält keine Steuerung.
  Arbeitsanweisungen kommen über Aufgaben, Antworten über Fragen-Dialoge.
- Das Schließen eines Tabs oder der GUI beendet keine laufende Instanz oder Aufgabe.

### Aufbau

- Sidebar oben: Projektwahl einschließlich Projektverwaltung, großer
  **„+ Aufgabe“**-Button, Aufgaben, Fragen, Sessions und Orchestrator.
- Sidebar unten: klar abgesetzter Factory-Bereich mit Runner und Diagnose.
- Administration und Einstellungen über benannte Menüs/Schaltflächen, ohne
  die vereinbarte Hauptnavigation durch technische Kategorien zu überladen.
- Kopfzeile mit Projekt-/Ansichtskontext und globaler Benachrichtigungsglocke.
- Rechts scrollbar angeordnete Listen/Karten mit Metainformationen. Details
  öffnen eine eigene Detailansicht mit Rücknavigation, nicht einen Ersatz für
  den Entscheidungsdialog.
- Aufgabenkarten zeigen Titel, mehrzeiligen Prompt-Auszug, Bildvorschauen,
  Status und passende Sprungaktionen zu Fragen und OpenCode-Tabs.
- Optik gemäß Referenz: dezente Rahmen, ruhige dunkle Flächen, abgerundete
  Karten, klare Typografie, großzügige Abstände. Keine pixelgenaue Kopie.

### Durchgängige Regeln

- Entscheidungen, Bearbeitung, Konfiguration und Bestätigungen ausschließlich
  in egui-Dialogen; keine Inline-Formulare oder Inline-Entscheidungen in Listen.
  Navigation, Suche, Filter und aufklappbare Lesedetails bleiben direkt bedienbar.
- Alle Inhalte bleiben erreichbar. Auszüge sind als solche erkennbar; vollständige
  Texte, technische Metadaten und Rohdaten sind in Details zugänglich.
- Fehlende, gekürzte, noch nicht geladene oder veraltete Daten ausdrücklich markieren.
- Listen und lange Dialoge sind scrollbar; horizontale Überläufe werden behandelt.
- Zustände immer durch Text, optional zusätzlich durch Farbe/Symbol kennzeichnen.
- Grüne Sidebar-Zahlen zählen aktuell erreichbar bestätigte OpenCode-Instanzen,
  nicht nur beschäftigte Instanzen. Prozess/Service und logische Session getrennt
  darstellen; mehrere Sessions dürfen denselben Prozess nicht mehrfach zählen.
- Fragenbadge rot mit Zahl offener, vom Benutzer zu beantwortender Fragegruppen;
  Teilfragenanzahl zusätzlich in der Fragenansicht. Kein blinkender Alarm nötig.
- Bei Verbindungsausfall bleibt der letzte Stand mit Warnung und Zeitangabe
  sichtbar. Automatisch wiederverbinden, keine neuen Offline-Schreibaktionen.
  Lokale Entwürfe bleiben erhalten.

## 2. Vorhandene Grundlagen und tatsächliche Lücken

Wiederverwenden:

| Grundlage | Bestehender Ort |
| --- | --- |
| Domänenzustände, IDs, Revisionen | `crates/factory-core` |
| HTTP-Client und SSE mit Cursor | `crates/factory-protocol` |
| Persistenz, Gerätezugang, Routing, Supervisor | `crates/factoryd` |
| OpenCode-Anbindung, Laufzeit und Archivübertragung | `crates/factory-runner`, `crates/factory-opencode` |
| Projektassistent, Antwortzustellung, Verwaltungsabläufe | `crates/factory-tui/src` als funktionale Referenz |
| Vollständiges Nachrichtenarchiv und Datei-Download | `docs/session-archive.md`, `docs/archive-files.md` |
| Session-Live-Ansicht über Factory/Runner | `docs/session-view.md` |
| Routing und Orchestrator-Fragen | `docs/task-routing.md`, `docs/question-orchestration.md` |
| Bestehende Push-Outbox | `docs/background-push.md` |

Insbesondere nicht erneut bauen: Archiv, SSE, Routing und revisionsgeschützte
Antwortzustellung. Die begrenzte Excerpt-Ansicht ist aber kein Ersatz für das
vollständige Archiv. Die bestehende ntfy-Outbox ist kein allgemeines
Benachrichtigungszentrum mit gelesen/ungelesen.

Gezielt prüfen/ergänzen: explizite Aufgabenfreigabe, allgemeine Eingabeanhänge
(`SaveTask` kennt derzeit Bilder), bestätigte Instanzpräsenz, Orchestrator-Aktivität,
dauerhafte Benachrichtigungen und die Aktualisierung vollständiger Live-Inhalte.

## 3. Technische Leitplanken

- Neue Crate `crates/factory-gui`, vorgeschlagener Binärname `yoloforge-gui`.
- egui rendert ausschließlich lokalen Zustand. HTTP, Dateizugriffe,
  Bilddekodierung und Prozessverwaltung laufen außerhalb des UI-Threads.
- Tokio-Worker mit typisierten Nachrichten an den UI-Zustand; Tasks abbrechen,
  wenn der zugehörige View/Verbindungskontext nicht mehr gültig ist.
- Gemeinsame fachliche Clientlogik bei Bedarf in eine neue
  `factory-client-core`-Crate extrahieren. Keine Abhängigkeit der GUI von
  Ratatui/Crossterm; bestehende CLI/TUI-Abläufe nicht blind duplizieren.
- Eine zentrale Zustandsquelle für Navigation, Badge-Zahlen und Datenansichten.
  Stabile Projekt-, Aufgaben-, Session- und Instanz-IDs statt Listenindizes.
- Snapshot plus Eventcursor konsistent zusammenführen; Cursorlücken führen zu
  kontrolliertem Neuabgleich. Fallback-Refresh und Präsenzablauf auch ohne Events.
- Schreibaktionen behalten Request-ID und unveränderte Nutzlast bei unklarem
  Ausgang. Kein blindes Wiederholen fachlicher Aktionen.
- Remote-Sessions über Factory/Runner lesen; keine OpenCode-Passwörter an die GUI
  kopieren und keine zusätzlichen öffentlich erreichbaren OpenCode-Ports verlangen.
- Bestehende Clients bleiben kompatibel. Neue Fähigkeiten aushandeln und fehlende
  Serverunterstützung erklären; erforderliche Schema-Migrationen versionieren.

## 4. Aufgabenpakete

Die ursprünglichen Paketcheckboxen sind noch nicht einzeln abgenommen. Der
implementierte Stand und seine Prüfbelege stehen im folgenden Abschnitt.
„Fertig“ bezeichnet überprüfbare Entwicklungsziele,
keinen zusätzlichen Betreiber-, Realmodell- oder Zielhost-Abnahmeprozess.

### Implementierungsstand

- [x] Native GUI-Crate und Linux-Binary mit Dark Theme und deutscher Navigation.
- [x] Aufgaben mit Entwurf/Freigabe, Bildern, allgemeinen Dateien und Vollansicht.
- [x] Gruppierte Fragen-Dialoge und revisionsgeschützte Einzelzustellungen.
- [x] Schreibgeschützte Sessiontabs mit aktuellem Archivfenster und Historienabruf.
- [x] Virtuelle Listen, begrenzte Textabschnitte, Hintergrund-Bilddekodierung.
- [x] Server/Runner-Präsenz, dauerhafte Meldungen und Geräte-Lesestatus.
- [x] Verwaltungsdialoge und mit der TUI geteilte SSH-Helfer.
- [x] Kompakter Desktop-Snapshot/SSE ohne wiederholte binäre Anhangübertragung.
- [x] Workspace-Tests einschließlich TUI sowie nativer API-/Fenster-Smoke bestanden.
- [x] Linux-Paketdefinition, Desktop-Eintrag und CI-Smoke ergänzt.
- [ ] Einzelabnahme aller ursprünglichen Paketkriterien, insbesondere breiter
  Interaktionstests der Verwaltungsdialoge und Paketinstallation auf beiden
  Releasearchitekturen. Native Provider-/Hosting-Voraussetzungen bleiben
  abhängig vom konfigurierten ausführenden Host.

### Etappe A — Inventar und gemeinsamer Client-Unterbau

#### GUI-001 — Funktionsparität inventarisieren
- [ ] Alle aktuellen TUI-Funktionen und unterstützenden CLI-Einstiege erfassen.
- [ ] Je Funktion zukünftige GUI-Ansicht, Dialog, API, Berechtigung und Test zuordnen.
- [ ] Auch seltene Funktionen aus GUI-025 bis GUI-028 berücksichtigen.
- Abhängigkeit: keine.
- Fertig: versionierte Paritätsmatrix; keine bestehende TUI-Funktion ohne
  GUI-Ziel oder ausdrücklich dokumentierte Nichtanwendbarkeit.

#### GUI-002 — API- und Zustandsverträge festlegen
- [ ] Ticket, freigegebene Aufgabe, Auftrag, Workstream, Session und laufende
  Instanz sauber unterscheiden und fachliche Anzeigenamen definieren.
- [ ] Lücken für Entwurf/Freigabe, Anhänge, Instanzpräsenz, Orchestrator und
  Benachrichtigungen anhand des vorhandenen Codes bestimmen.
- [ ] Statusübergänge, Badge-Semantik, Frischegrenzen, Pagination und
  Kompatibilität älterer Server festlegen.
- Abhängigkeit: GUI-001.
- Fertig: konkrete wiederverwendete und neue API-Verträge samt Testfällen.

#### GUI-003 — UI-unabhängige Clientlogik vorbereiten
- [ ] Request-/Konfliktbehandlung, Antwortvalidierung und Statusprojektionen
  wiederverwendbar machen, ohne terminalabhängige Darstellung zu übernehmen.
- [ ] Sichtbares Fehlerformat und revisionsgeschützte Schreibabläufe vereinheitlichen.
- Abhängigkeit: GUI-002.
- Fertig: GUI kann die Fachlogik ohne TUI-Import nutzen; bestehende Tests bleiben grün.

### Etappe B — Native Anwendung und Datenaktualität

#### GUI-004 — Rust-/egui-Anwendung aufsetzen
- [ ] Neue Workspace-Crate, CLI-Startparameter, Linux-Fenster und asynchrone Laufzeit.
- [ ] Benutzerkonfiguration, private Datenpfade, strukturierte redigierte Logs
  sowie sichtbare Start-/Laufzeitfehler vorsehen.
- [ ] Konkrete Bibliotheksversionen und minimale Rust-/Linux-Anforderungen festhalten.
- Abhängigkeit: GUI-003.
- Fertig: startbares natives Dark-Theme-Fenster ohne Serverzwang oder UI-Blockade.

#### GUI-005 — Gestaltung und gemeinsame Widgets
- [ ] Farben, Schriftgrößen, Abstände, Karten, Statuschips, Badges und Buttons definieren.
- [ ] Wiederverwendbare Scrolllisten, Leer-/Lade-/Fehlerzustände und Detailfelder bauen.
- [ ] Icons mit verlässlicher Font-/Asset-Abdeckung; Unicode, Kontrast,
  Tastaturfokus, Skalierung und kleine Fenster berücksichtigen.
- Abhängigkeit: GUI-004.
- Fertig: konsistente Komponenten entsprechen dem Stil der Referenz; kein Light Theme.

#### GUI-006 — Hauptlayout, Tabs und Navigation
- [ ] Sidebar, Projektwahl, Plus-Button, Factory-Bereich und Kopfzeile umsetzen.
- [ ] Ersten Verwaltungstab und zusätzliche schließbare OpenCode-Tabs implementieren.
- [ ] Typisierte Sprungziele projektübergreifend auflösen, vorhandene Tabs wiederverwenden.
- [ ] Filter, Auswahl und Scrollposition je Ansicht/Projekt erhalten; unbekannte
  oder nicht mehr berechtigte Sprungziele verständlich behandeln.
- Abhängigkeit: GUI-005.
- Fertig: durchgängige Navigation ohne doppelte Sessiontabs oder verlorenen Kontext.

#### GUI-007 — Verbindungen und Gerätezugang
- [ ] Verbindungsdialog für lokale/entfernte Server und vorhandene Zugangsdaten.
- [ ] Enrollment, Rollenanzeige, Profilwechsel, Abmeldung und Widerruf behandeln.
- [ ] Geheimnisse sicher speichern, TLS prüfen, keine Tokens in Logs oder URLs.
- Abhängigkeit: GUI-004, GUI-003.
- Fertig: Serverwechsel verwirft alte Berechtigungen/Tasks; abgelaufener oder
  widerrufener Zugang bleibt nicht scheinbar funktionierend sichtbar.

#### GUI-008 — Zentraler Live-Datenzustand
- [ ] Initialsnapshot, SSE-Verarbeitung, Reconnect/Backoff und Cursor-Recovery.
- [ ] Nach Events betroffene Daten nachladen; Polling als Fallback, nicht jeden
  Frame Vollsnapshots oder Archive laden.
- [ ] Serververbindung, Runnerkontakt und Quellaktualität getrennt kennzeichnen;
  Abfragezeit nicht als frischen Runnernachweis ausgeben.
- [ ] Bei Ausfall letzten Stand behalten, Schreibaktionen sperren, Entwürfe erhalten.
- Abhängigkeit: GUI-007, GUI-002.
- Fertig: Ansichten und Zähler widersprechen sich nicht; veraltete Daten sind erkennbar.

#### GUI-009 — Einheitliches Dialog- und Entwurfssystem
- [ ] Modale egui-Dialoge mit Fokusführung, Validierung, Esc-/Enter-Verhalten,
  explizitem Speichern und Warnung vor Entwurfsverlust.
- [ ] Speichern asynchron, Doppelklickschutz, Request-Abgleich, Konfliktdialog
  mit Erhalt eigener Eingaben und verständlichen Teilfehlern.
- [ ] Private lokale Entwurfsspeicherung inklusive Wiederherstellung und
  Bereinigung; gespeicherte Entwürfe niemals automatisch freigeben.
- Abhängigkeit: GUI-005, GUI-008, GUI-003.
- Fertig: derselbe robuste Entscheidungsablauf gilt in sämtlichen Bereichen.

### Etappe C — Backend-Ergänzungen für die zugesagte GUI

#### GUI-010 — Aufgabenentwurf und explizite Freigabe
- [ ] Entwurfs-/Freigabestatus persistent modellieren und API ergänzen.
- [ ] Entwürfe einschließlich Anhängen nie vom Scheduler starten lassen.
- [ ] „Entwurf speichern“ und „Einreihen“ idempotent und revisionsgeschützt anbieten.
- [ ] Verhalten bestehender Clients/Aufgaben bei Migration ausdrücklich erhalten.
- Abhängigkeit: GUI-002.
- Fertig: Neustart oder unklare HTTP-Antwort erzeugt weder Doppelaufgabe noch
  unbeabsichtigte Ausführung eines Entwurfs.

#### GUI-011 — Allgemeine Aufgabenanhänge
- [ ] Vorhandene Bild- und Archivdateimechanismen prüfen; fehlenden Upload-/
  Auftragsübergabepfad für allgemeine Eingabedateien ergänzen.
- [ ] Name, Typ, Größe, Hash, Limits, Uploadfortschritt und Verfügbarkeit führen.
- [ ] Integrität, sichere Pfade, Quoten, Uploadabbrüche und verwaiste Daten behandeln.
- [ ] Unterstützte Typen bis OpenCode durchreichen; nicht unterstützte Dateien
  nicht stillschweigend weglassen, sondern mit Grund ablehnen.
- Abhängigkeit: GUI-002, GUI-010.
- Fertig: freigegebene Aufgabe erreicht den Agenten mit bestätigten Anhängen;
  Bild-/Dateiverlust und überschrittene Grenzen werden sichtbar.

#### GUI-012 — Bestätigte Instanzpräsenz und Aktivität
- [ ] Instanz-ID, Rolle, Runner, Projektzuordnung, zugehörige Sessions,
  letzten bestätigten Kontakt und beobachteten Zustand verfügbar machen.
- [ ] Arbeits-, Orchestrator- und Supervisor-Rollen eindeutig zuordnen; geteilte
  Services nicht doppelt zählen, mehrdeutige Zustände nicht erfinden.
- [ ] Ablaufzeiten und Zustände für erreichbar, unbekannt, veraltet und nicht
  erreichbar sowie beschäftigt, wartend und untätig bereitstellen.
- [ ] Aktuelle Orchestrator-Aktion, zugehörigen Auftrag, letzte Entscheidung
  und Warte-/Blockadegrund lesbar projizieren.
- Abhängigkeit: GUI-002.
- Fertig: kein grüner Badge allein aufgrund eines alten Auftragsstatus;
  Runner- oder Prozessausfall lässt Präsenz rechtzeitig ablaufen.

#### GUI-013 — Vollständigen Live-Verlauf über Factory absichern
- [ ] Bestehendes Archiv, Excerpts und Aktualisierungsereignisse gemeinsam nutzen.
- [ ] Fehlende Synchronisation veränderlicher Nachrichten/Toolausgaben ergänzen,
  Pagination und Zuordnung mehrerer Ausführungen derselben Session sichern.
- [ ] Quellrevision, Synchronisationszeit, Vollständigkeit und nicht verfügbare
  Inhalte explizit liefern; unbekannte Inhaltstypen als Rohdaten erreichbar lassen.
- Abhängigkeit: GUI-002.
- Fertig: lokale und entfernte Sessions zeigen den verfügbaren vollständigen
  Verlauf ohne abgeschnittene Inhalte als vermeintlich vollständige Wahrheit.

#### GUI-014 — Dauerhaftes Benachrichtigungszentrum im Server
- [ ] Projektübergreifende Ereignisse, stabile IDs, Zeit, Schweregrad,
  Projektkontext und typisierte Direktziele speichern.
- [ ] Gelesen/ungelesen je Benutzeridentität persistent führen; bei vorhandenem
  Gerätemodell Geltungsbereich explizit dokumentieren.
- [ ] Pagination, Filter, Ungelesenzahl, Lesebestätigung und SSE-Updates anbieten.
- [ ] Fragen, Fehler, Abschluss und relevante Zustandswechsel sinnvoll bündeln;
  Reconnect/Replays nicht als neue Benachrichtigungen duplizieren.
- [ ] Berechtigungen, Aufbewahrung und Verhalten bei gelöschten Zielen definieren;
  vorhandene ntfy-Outbox kompatibel beibehalten.
- Abhängigkeit: GUI-002.
- Fertig: Verlauf und Lesestatus überleben Client-/Serverneustart und verraten
  keine Projekte außerhalb der Zugriffsrechte.

### Etappe D — Täglicher Arbeitsablauf

#### GUI-015 — Projektwahl und Projektanlage
- [ ] Suchbare Projektwahl und klarer Einstieg in Projektverwaltung/-einstellungen.
- [ ] Projektassistent für verwaltetes/leeres Repository, Clone, Basisbranch,
  Runner, Modell und Bereitschaft umsetzen.
- [ ] Vorhandene Metadaten bearbeiten; gesperrte Repositoryänderungen erklären.
- Abhängigkeit: GUI-006, GUI-009.
- Fertig: erstes Projekt ohne CLI/TUI anlegbar; kein Runner oder fehlender
  Modellzugang führt zu konkreter, behebbarer Diagnose statt leerer Ansicht.

#### GUI-016 — Aufgabenliste und vollständige Details
- [ ] Karten gemäß Referenz mit Prompt-Auszug, Vorschauen und aktuellem Status.
- [ ] Suche, Statusfilter, Sortierung und effizientes Nachladen langer Listen.
- [ ] Details für ganzen Prompt, Anhänge, Routing, Auftrag, Session, Runner,
  Revisionen, Zeiten, Ergebnis und Ereignisverlauf.
- [ ] Direktsprünge zu Fragegruppen und passendem Sessiontab kontextsensitiv anzeigen.
- Abhängigkeit: GUI-006, GUI-008, GUI-010, GUI-012.
- Fertig: alle Aufgabeninformationen erreichbar; Karten bleiben auch mit langen
  Prompts oder fehlenden Bildern übersichtlich und bedienbar.

#### GUI-017 — Aufgabenanlage, Bearbeitung und Anhänge
- [ ] Großer Plus-Button öffnet Promptdialog mit Entwurf/Freigabe und Vorschau.
- [ ] Dateiauswahl, Drag-and-drop für Dateien/Bilder und Bildpaste anbieten.
- [ ] Anhänge entfernen/ansehen, Uploadzustand und Grenzen verständlich anzeigen.
- [ ] Automatische Zuordnung sowie vorhandene feste Sessionzuordnung anbieten;
  unzulässige Bearbeitung bereits gestarteter Aufträge verhindern.
- Abhängigkeit: GUI-009, GUI-010, GUI-011, GUI-016.
- Fertig: Aufgabe mit Dateien anlegen, als Entwurf wiederöffnen und ausdrücklich
  einreihen; unklare Zustellung erhält unveränderte Request-ID und Nutzlast.

#### GUI-018 — Gruppierte Fragen und gemeinsamer Antwortdialog
- [ ] Pro Session eine Karte, mehrere Teilfragen und Projekt-/Aufgabenkontext zeigen.
- [ ] Formulare und Berechtigungen einschließlich Pflichtfeldern, Auswahl,
  Mehrfachauswahl, Freitext und vorhandenen Typ-/Grenzregeln abbilden.
- [ ] Alle Antworten im gemeinsamen Dialog vorbereiten und gemeinsam absenden;
  vorhandene revisionsgeschützte Zustellung je Frage verwenden.
- [ ] Keine falsche Atomarität versprechen: Erfolg, ausstehende Zustellung,
  Konflikt und Fehler je Teilfrage erhalten; nur offene Antworten erneut senden.
- [ ] Orchestrator-Antworten, Eskalationsgründe und konkurrierende Änderungen zeigen.
- Abhängigkeit: GUI-009, GUI-008, GUI-003.
- Fertig: komplette Fragegruppe ohne TUI beantwortbar; veränderte oder bereits
  beantwortete Fragen überschreiben keine fremde Antwort.

#### GUI-019 — Sessionübersicht und Instanzzuordnung
- [ ] Sessions mit aktueller Aufgabe, Ziel, Zustand, Runner, Instanz und
  Aktualisierungszeit einschließlich inaktiver Historie auflisten.
- [ ] Instanzzustand und Sessionzustand getrennt anzeigen; laufende
  OpenCode-Instanzen auch bei untätigen Sessions sichtbar machen.
- [ ] Von Session zu Aufgabe, Fragen, Diagnosen und Live-Tab navigieren.
- Abhängigkeit: GUI-006, GUI-008, GUI-012.
- Fertig: erkennbar, welche Instanz welche Aufgabe bearbeitet und warum sie wartet.

#### GUI-020 — Schreibgeschützte OpenCode-Tabs
- [ ] Rollen, Nachrichtentexte, Markdown/Code, Toolname, Status, vollständige
  Ein-/Ausgaben, Fehler, Zeitangaben, Anhänge und technische Metadaten anzeigen.
- [ ] Lange Verläufe seitenweise laden; neue Daten einfügen, ohne Auswahl und
  Scrollposition zu verlieren. Automatisches Folgen explizit an-/abschaltbar.
- [ ] Ältere Revisionen und Ausführungen erreichbar halten; unbekannte Daten
  lesbar machen und Rohdaten/Archivexport anbieten.
- [ ] Dateien SHA256-geprüft exportieren, Bilder vergrößern; fehlende Bytes und
  Grenzen klar kennzeichnen. Aktive Inhalte nicht automatisch ausführen.
- [ ] Nur Navigation zu Fragen/Aufgaben, keine Antwort-, Prompt- oder Steuerfelder.
- Abhängigkeit: GUI-013, GUI-019, GUI-005.
- Fertig: mehrere lokale/entfernte Sessions gleichzeitig beobachtbar;
  Tab-Schließen hat keine Auswirkung auf deren Laufzeit.

#### GUI-021 — Orchestratoransicht und Steuerdialoge
- [ ] Aktuelle Verteilung/Fragenbearbeitung, Untätigkeit, Wartegründe, letzte
  Entscheidungen und Verweise zu betroffenen Aufgaben/Sessions anzeigen.
- [ ] Zugehörige Instanz/Modellläufe als OpenCode-Tabs öffnen können.
- [ ] Automatische Verteilung pausieren/fortsetzen, Sessionlimit und bestehende
  Scheduler-/Supervisor-Konfiguration über Dialoge bedienen.
- [ ] Projektpause, Verteilungspause und laufende Arbeit fachlich unterscheiden;
  erforderliche neue Steuerung nur gezielt im Backend ergänzen.
- Abhängigkeit: GUI-009, GUI-012, GUI-019.
- Fertig: „tut gerade nichts“ hat einen nachvollziehbaren Zustand; Pause beendet
  nicht implizit bereits laufende Aufgaben.

#### GUI-022 — Runnerübersicht
- [ ] Alle berechtigten Runner mit Host/Ausführungsort, Erreichbarkeit,
  letztem Heartbeat, Fähigkeiten, OpenCode-Bereitschaft und aktiven Aufträgen zeigen.
- [ ] Globale Verbindung und projektspezifische Bereitschaft unterscheiden.
- [ ] Zu aktiven Aufgaben, Instanzen und Fehlerdiagnosen verlinken.
- Abhängigkeit: GUI-008, GUI-012, GUI-006.
- Fertig: Offline-/Stale-Runner werden nicht als bereit angeboten; lange Listen scrollen.

#### GUI-023 — Diagnose und Ereignisverlauf
- [ ] Ereignisse für Orchestrator, Run-Agent, Supervisor, Routing, Runner und
  Kontrollaufträge mit Zeit, Projekt, Ursache und Ergebnis darstellen.
- [ ] Suche/Filter nach Projekt, Rolle, Session, Auftrag und Schweregrad.
- [ ] Dauerhaften Verlauf cursorbasiert nachladen; Details und Kopieren/Export
  anbieten, vertrauliche Daten nicht zusätzlich in Logs streuen.
- Abhängigkeit: GUI-008, GUI-006.
- Fertig: aus einer Aufgabe bis zum konkreten Fehler/Entscheidungspfad navigierbar;
  sichtbare Speicherbegrenzungen bedeuten keinen stillen Archivverlust.

#### GUI-024 — Benachrichtigungsglocke und Desktop-Meldungen
- [ ] Globale Glocke mit ungelesenen Meldungen, separater Kennzeichnung aktuellen
  Handlungsbedarfs, scrollbarer Liste, Projektfilter und Direktlinks.
- [ ] Gelesen/ungelesen und „alle gelesen“ bedienen; Lesen beantwortet keine Frage.
- [ ] Kurze In-App-Hinweise und konfigurierbare Linux-Desktop-Meldungen für Fragen,
  Fehler und Abschlüsse; sensible Inhalte standardmäßig nicht auf dem Sperrbildschirm.
- [ ] Klicks in Desktop-Meldungen zur laufenden App bzw. zum richtigen Ziel führen;
  fehlenden Benachrichtigungsdienst und entzogene Berechtigungen behandeln.
- [ ] Keine Replay-Flut nach Reconnect; keine Hintergrundzustellung bei beendeter
  GUI versprechen. Bestehendes ntfy bleibt ein separater optionaler Kanal.
- Abhängigkeit: GUI-014, GUI-006, GUI-008.
- Fertig: Meldung aus einem anderen Projekt öffnet zuverlässig dessen Ziel,
  auch wenn inzwischen beantwortet oder nicht mehr verfügbar.

### Etappe E — Vollständige Verwaltungsparität

#### GUI-025 — Projekt-, Modell-, Harness- und Zugangseinstellungen
- [ ] Projektprofile, Modell-/Providerbindung, Runnerpfade, MCP/Harness,
  Projektbereitschaft und bestehende Anmelde-/Hosting-Helfer über Dialoge.
- [ ] Lokale SSH-/Serviceprofile und Verbindungsdiagnose dort anbieten, wo
  bestehende Verwaltungsabläufe sie benötigen; kein Terminal als Ersatz für
  eine fehlende GUI-Verwaltungsfunktion.
- [ ] Externe Provider-Anmeldung bei Bedarf sicher im Browser starten und
  Status zurückmelden; keine Zugangsdaten zwischen Services kopieren.
- Abhängigkeit: GUI-001, GUI-009, GUI-015.
- Fertig: Einrichtung/Änderung eines bestehenden Projekts ohne TUI möglich;
  Plattform-/Providergrenzen und notwendige externe Schritte ausdrücklich sichtbar.

#### GUI-026 — Tickets, Workstreams und Laufzeitverwaltung
- [ ] Technische Tickets, Prioritäten, Kriterien, Abhängigkeiten und Workstreams
  zusätzlich zum einfachen Aufgabenablauf über erweiterte Dialoge verwalten.
- [ ] Vorbereiten, Start, Pause/Resume, bestätigte Fortsetzung, vorhandene
  Kontrollwiederholung, Sessionabschluss, Compaction und Runtime-Beendigung abbilden.
- [ ] Bestehende Container-Recovery-/Quarantänezustände anzeigen und nur bereits
  unterstützte sichere Maßnahmen anbieten; keinen neuen Recovery-Ausbau beginnen.
- [ ] Gefährliche Aktionen mit konkretem Ziel und Folgen bestätigen lassen;
  ungeklärten Stopp nicht als erfolgreich behaupten.
- Abhängigkeit: GUI-001, GUI-009, GUI-019.
- Fertig: bisherige technische Verwaltungsfunktionen sind erreichbar, aber nicht
  versehentlich Teil der schreibgeschützten OpenCode-Tabs.

#### GUI-027 — Ergebnisse, Review, Checkpoints und Veröffentlichung
- [ ] Ergebnisse, Reviewkandidaten, Tests und Integrations-/Publikationszustände zeigen.
- [ ] Review/Integration, Checkpoint, Fortsetzung/Übergabe und bestehende
  GitHub-/Forgejo-Veröffentlichungshelfer durch Dialoge bedienen.
- [ ] Branch-/Commitbezug, Konflikte, Nacharbeit und unveröffentlichte Ergebnisse
  unverfälscht anzeigen; auf zugehörige Session/Diagnose verweisen.
- Abhängigkeit: GUI-001, GUI-009, GUI-016, GUI-026.
- Fertig: bestehender Ergebnisworkflow vollständig ohne TUI möglich;
  Konflikte und fehlgeschlagene Publikation werden nicht als Abschluss kaschiert.

#### GUI-028 — Wissen, Nutzung, Geräte und übrige Parität
- [ ] Projektwissen anzeigen, hinzufügen, Quellen/Scope prüfen und ablösen;
  vorhandene Kontextauswahl zugänglich machen.
- [ ] Token-/Kostenanzeige, Budgets, Sessionlimits und Supervisorhistorie abbilden.
- [ ] Geräte auflisten, Einladungen erstellen, Rollen/Scopes anzeigen und widerrufen.
- [ ] Archiv-/Dateiexport sowie alle verbleibenden Punkte aus GUI-001 schließen.
- Abhängigkeit: GUI-001, GUI-009, GUI-025, GUI-026.
- Fertig: Paritätsmatrix vollständig erfüllt; keine TUI-Pflicht für zugesagte Funktionen.

### Etappe F — Lokaler Betrieb, Qualität und Auslieferung

#### GUI-029 — Lokalen Server und Runner komfortabel starten
- [ ] Lokalen Startassistenten mit Datenpfad, Ports, Token und Bereitschaft anbieten.
- [ ] Bereits laufende Prozesse erkennen, Doppelstarts und Portkonflikte vermeiden;
  entfernte/betreiberverwaltete Prozesse niemals als eigene behandeln.
- [ ] Prozesse von der GUI-Lebensdauer entkoppeln und nach Neustart wiederfinden.
- [ ] Explizites Stoppen eigener lokaler Dienste nur mit Folgendialog;
  Konfiguration/Installationsfehler samt Logs zugänglich machen.
- Abhängigkeit: GUI-007, GUI-009, GUI-022.
- Fertig: lokale Umgebung ohne Terminal startbar; GUI-Schließen lässt Arbeit laufen.

#### GUI-030 — Automatisierte Entwicklungsprüfungen
- [ ] Unit-Tests für Zustandsprojektionen, Gruppierung, Zähler, Navigation,
  Dialogvalidierung, Entwurfswiederherstellung und Request-Wiederholung.
- [ ] Backend-/Protokolltests für neue Migrationen, Freigabe, Anhänge, Präsenz,
  Benachrichtigungen, Berechtigungen und ältere Clients.
- [ ] GUI-Interaktionstests mit deterministischen Fixtures: Aufgabe → Freigabe →
  Session → mehrere Fragen → gemeinsame Antworten → Ergebnis/Benachrichtigung.
- [ ] Reconnect, Cursorlücken, Antwortverlust, konkurrierende Änderungen,
  Widerruf und veraltete/verschwundene Sprungziele abdecken.
- [ ] Tests für lange Listen/Prompts/Toolausgaben, große Archive, Bildfehler,
  Skalierung, Tastaturbedienung und Scrollposition; Testdatenbudgets festhalten.
- [ ] Vorhandene Workspace-/TUI-Tests weiter ausführen; Tests pro Paket bereits
  während dessen Implementierung ergänzen, nicht erst am Ende.
- Abhängigkeit: beginnt mit GUI-002; Abschluss nach GUI-028 und GUI-029.
- Fertig: reproduzierbare Tests belegen Kernabläufe und Fehlerfälle ohne
  Betreiberinstallation oder verpflichtende echte Modellaufrufe.

#### GUI-031 — Linux-Paketierung und CI
- [ ] GUI-Build mit Linux-Systemabhängigkeiten und reproduzierbarem Lockfile in CI.
- [ ] Headless UI-Prüfungen soweit möglich, Artefakte, Icon und Desktop-Eintrag.
- [ ] Benötigte lokale Server-/Runner-Binaries und Assetpfade verlässlich paketieren;
  Start aus Desktop-Menü darf nicht vom aktuellen Arbeitsverzeichnis abhängen.
- [ ] Bestehende Release-/Prüfsummenprozesse ergänzen, CLI/TUI-Artefakte erhalten.
- [ ] Skalierung und unterstützte X11-/Wayland-Umgebungen dokumentieren.
- Abhängigkeit: GUI-004; Abschluss nach GUI-029 und GUI-030.
- Fertig: installierbares Linux-Artefakt startet remote und lokal ohne Cargo/Terminal.

#### GUI-032 — Bedienungsdokumentation und Abschluss des Plans
- [ ] README, Entwicklungs-/Installationsanleitung und Softwarestand aktualisieren.
- [ ] Screenshots und kurze Wege für Projektanlage, Aufgabe, Fragegruppe,
  Sessiontab, Orchestrator und Benachrichtigungen dokumentieren.
- [ ] Grenzen, Datenaktualität, Anhänge, Geheimnisse, Prozesslebensdauer und
  Unterschied zu externer OpenCode-Bedienung erklären.
- [ ] Paritätsmatrix und Fertig-Kriterien prüfen; offene Punkte ehrlich ausweisen.
- Abhängigkeit: GUI-028 bis GUI-031.
- Fertig: dokumentierter GUI-Stand stimmt mit implementierter Funktionalität überein.

## 5. Empfohlene Implementierungsreihenfolge

1. **Verträge und Fundament:** GUI-001 bis GUI-009. Zuerst ein echter
   Serverdurchstich mit korrekter Frischeanzeige, nicht nur ein statisches Mockup.
2. **Arbeitsfähiger Kern:** GUI-010 bis GUI-013 sowie GUI-015 bis GUI-020.
   Ergebnis: Aufgabenfreigabe, Anhänge, gruppierte Fragen und beobachtbare Sessions.
3. **Factory-Transparenz:** GUI-014 sowie GUI-021 bis GUI-024.
   Ergebnis: Orchestrator, Runner, Diagnose und projektübergreifende Meldungen.
4. **Vollständiger Ersatz und lokaler Komfort:** GUI-025 bis GUI-029.
   Die vorherigen Etappen sind Zwischenstände, noch kein vollständiger TUI-Ersatz.
5. **Auslieferung:** GUI-030 bis GUI-032 abschließen. Tests, Dokumentation und
   Paketierungsgrundlagen laufen bereits in den jeweiligen vorherigen Etappen mit.

Unabhängige Backend-Pakete GUI-012 bis GUI-014 können nach GUI-002 unabhängig
vom Layout umgesetzt werden. Die Abhängigkeiten je Paket bleiben maßgeblich.

## 6. Definition „GUI umgesetzt“

- [ ] Vereinbartes Dark-Layout und sämtliche Ansichten sind vorhanden und konsistent.
- [ ] Alle Entscheidungen erfolgen in Dialogen, alle langen Inhalte sind erreichbar.
- [ ] Aufgaben starten erst nach Freigabe; Dateien/Bilder kommen nachweisbar mit.
- [ ] Fragegruppen können gemeinsam beantwortet werden, ohne Zustellstatus zu verschleiern.
- [ ] OpenCode-Tabs sind vollständig lesend; Verlauf, Tools und fehlende Daten klar erkennbar.
- [ ] Instanzzahlen beruhen auf bestätigter Präsenz, Ansichten bleiben aktuell.
- [ ] Globale Benachrichtigungen samt Direktlinks und Desktop-Meldungen funktionieren.
- [ ] Verbindungsabbrüche, Entwürfe und unklare Schreibantworten sind sicher behandelt.
- [ ] Bestehende TUI-Funktionen sind laut Paritätsmatrix in der GUI zugänglich.
- [ ] Lokaler und entfernter Betrieb sind paketiert, getestet und dokumentiert.
- [ ] GUI-/Tab-Schließen beendet keine Arbeit; CLI/TUI bleiben kompatibel nutzbar.

Nicht enthalten: Light Theme, Windows/macOS-Auslieferung, interaktive
OpenCode-Chatsteuerung, neue Supervisor-Stallerkennung, zusätzlicher
Recovery-Ausbau oder eine neue eigene Gesamtkontextplanung.

# Verbindliche GUI-Designregeln

Einstellungsformulare folgen „Default-Modelle“: Label direkt über dem Eingabefeld,
8 pt zusätzlicher Abstand zum nächsten Feld über `design::settings_field`.
Bei bedingten Feldern passt sich die Dialoghöhe in beide Richtungen an; bei
Platzmangel scrollt nur der Inhalt. „Speichern“ bleibt unten rechts.
Zwischen einer Trennlinie und den Buttons der Einstellungs-Fußzeile liegen
mindestens 12 pt Abstand; den Platz zentral über `design::settings_footer` reservieren.
Factory-Settings enthalten Model Provider, Default Models, Skills,
MCPs, Geräte/Zugänge und eine lokale Sprachansicht. Keine separaten Einladungs-, Widerruf-, Desktop-Meldungs-
oder Zustellungsbereiche. Geräte zeigen aktive Zugänge ohne technische Rohdaten;
Widerrufen erfolgt am Eintrag mit Bestätigung. Der technische Zustellungsabgleich
bleibt in der Diagnose erreichbar.

Die vom Nutzer festgelegten Regeln gelten für alle neuen und überarbeiteten
Oberflächen dieser Crate. Bestehendes Dark Theme erhalten. Die gesamte GUI
unterstützt English und Deutsch über `src/i18n.rs` und `locales/messages.json`;
neue GUI-Texte mit `crate::tr!` einbinden. Sprachwahl lokal pro Gerät:
Auto (deutsche Systemsprache → Deutsch, sonst English), English, Deutsch.
Übernahme nach bestätigtem Speichern ohne Neustart. Nutzerdaten, Modellantworten,
Logs und Protokollwerte nicht übersetzen. Auch auf Deutsch heißen Aufgaben
Intent/Intents; Factory, Runner, Agent-Namen und Fachbegriffe wie Settings,
Models, Provider, Skills, Sessions und Tools bleiben englisch.

- **Textfelder:** ausgewogener Innenabstand, auch links/rechts. Standard 13 pt
  horizontal und 11 pt vertikal statt des knappen egui-Standards.
  Zentrale Konstruktoren in `src/design.rs` verwenden, einschließlich
  Suche, Zugangsdaten, Prompt, Zahlen-/JSON-Texte und mehrzeilige Eingaben.
  Platzhalter und Beispieltexte in leeren Feldern immer grau und kursiv über
  `design::input_hint` an `.hint_text(...)` übergeben. Tatsächliche Feldwerte
  bleiben normal formatiert.
  Strg+C muss markierten Text auch an andere native Wayland-Anwendungen liefern;
  leere Kopierausgaben dürfen bestehende Zwischenablageinhalte nicht löschen.
- **Listen sind Listen, keine Ansammlung von Buttons.** Gleichrangige Zeilen
  belegen dieselbe volle Breite. Labels linksbündig; Zähler innerhalb der Zeile
  rechts. Keine unterschiedlich langen Buttonflächen abhängig vom Text oder Badge.
  Navigation nutzt ruhige Zeilen mit Hover-/Auswahlmarkierung. Aktionsbuttons
  (z. B. „+ Aufgabe“) bleiben klar unterscheidbar.
- **Checkboxen:** GUI-weit mindestens 12 pt Abstand zwischen Kästchen und
  Beschriftung, zentral über `style.spacing.icon_spacing`.
- **Textkürzungen:** GUI-weit jede gekürzte Anzeige mit Ellipse „…“ kennzeichnen.
  Gilt für Prompt-Zusammenfassungen, Titel, Menüs, IDs und passive Textvorschauen.
  Originaltexte und editierbare Inhalte nicht verändern; lange Volltexte bleiben
  durch Umbruch oder Scrollen erreichbar.
  Titel in Aufgaben-, Fragen-, Session-, Runner- und Diagnoselisten sowie in der
  OpenCode-Sidebar und im OpenCode-Inhaltskopf sind einzeilig mit Ellipse.
  Volltexte bleiben per Tooltip bzw. Anzeige erreichbar; Dialoginhalte und
  OpenCode-Verlauf werden nicht auf eine Zeile beschränkt.
- **Dialoge:** ausreichend Abstand zum Fensterrand (32 pt Zielabstand) und
  umlaufender Innenabstand (24 pt). Gemeinsame Kopfzeile mit Titel und gut
  erreichbarem Schließen-X oben rechts: Die sichtbare Glyphe (nicht ihre
  Klickfläche) schließt rechts mit der Inhaltskante ab, hat nach oben denselben
  Abstand und ist vertikal auf der Titelzeile ohne Unterlängen zentriert.
  Lange Inhalte scrollen innerhalb des
  verfügbaren Raums; Kopfzeile und Schließen bleiben sichtbar. X/Escape darf
  keine Arbeit beenden oder Entwürfe verwerfen.
- **Kurze Bestätigungen:** Für „Intent löschen“ höchstens 440 pt Inhaltsbreite
  (zuzüglich Innenabstand). Umgebrochener Text bestimmt die Höhe; keine feste
  oder minimale Dialoghöhe aus der Fensterhöhe ableiten. Die Aktionszeile
  rechtsbündig mit `design::dialog_footer` direkt hinter Text und Trennlinie
  platzieren, nicht über einen Füllabstand am unteren Fensterrand. Nur bei
  Überlauf den Text vertikal scrollen; Kopf und Aktionen bleiben außerhalb.
  Geometrietests prüfen ab dem ersten sichtbaren Frame die tatsächliche
  Dialoggröße und den Abstand Text–Buttons auf kleinem und großem Fenster,
  nicht nur, ob die Buttons sichtbar sind.
- **Aufgabendialog:** nur ein Scrollbereich für den gesamten Inhalt, kein
  verschachtelter Aufgaben- oder Prompt-Scrollbereich. Historie bestehender
  Aufgaben direkt unter dem Prompt mit Zeitstempeln anzeigen; Fehler rot,
  Warnungen gelb und Erfolg grün. Standardformular bei ausreichender Fensterhöhe
  ohne Scrollen vollständig anzeigen. Prompt im
  OpenCode-Stil: dezente dunkle Fläche, linke blaue 6-pt-Linie, Monospace in
  CLI-Schriftgröße, deutlich ausgegrauter Platzhalter, keine Kontext-/Längenzeile.
  Bilder/Dateien per Drag-and-drop anpinnen und oberhalb des Textfelds als
  kompakte, gleich große Vorschauen nebeneinander mit Umbruch
  anzeigen. Bilder direkt darstellen, Textdateien als begrenzte, passive
  Textvorschau in gleicher Größe; sonst Dateisymbol. Entfernen-X oben rechts
  auf der Vorschau, Dateiname/Typ/Größe nur im Hover-Tooltip.
  Nach dem Anpinnen eines Anhangs erhält das Promptfeld wieder den Fokus.
  Der Dialog wächst mit dem Inhalt bis zur verfügbaren Fensterhöhe, erst dann scrollt der Inhalt.
  Unter der Trennlinie stehen zwei feste Zeilen außerhalb des Scrollbereichs:
  zuerst breite Modellauswahl („Default (<Modell / Provider>)“ ohne Begleittext),
  Zuordnung („Auto Runner“), Phosphor-Dateiauswahl und MCPs. Kein Einfügen-Icon;
  Strg+V bleibt verfügbar. Die erste Zeile ist standardmäßig ausgeblendet.
  Ein kleiner Phosphor-Reglerbutton links in der Aktionszeile toggelt die erste
  Zeile einschließlich der Trennlinie zwischen den Zeilen.
  Die zweite Zeile enthält rechtsbündig genau einen Aktionsbutton, direkt
  links daneben „Nicht schließen“ und „Entwurf“ als dauerhaft gespeicherte
  Checkboxen. Ihr Zustand bestimmt den Buttontext („Einreihen“, „Einreihen & neu“,
  „Entwurf“, „Entwurf & neu“). Offline sind „Entwurf“ und „MCPs“ ausgeblendet;
  „Nicht schließen“ bleibt mit einem Button („Lokal vorbereiten“ oder
  „Vorbereiten & neu“). Modellnamen immer als „Modell / Provider“ anzeigen.
  Der Aktionsbutton bleibt bei allen Textvarianten 200 pt breit; zwischen
  den Checkboxen und zum Button liegen jeweils 24 pt Abstand.
   „Einreihen“ schließt nach Serverbestätigung;
   „Einreihen & neu“ öffnet einen leeren Auftrag im selben Projekt. Die
   Bestätigung erscheint als Toast, nicht im Dialog. Zuordnung
  zeigt nur offene, nicht beendete Sessions mit ihrem OpenCode-Titel statt IDs.
  Während der Übertragung deckt ein Overlay den gesamten Dialog ab und zeigt
  einen mittigen Spinner. Keine Inline-Ladeanzeige.
  Im Promptfeld löst Enter die aktuelle Sendeaktion aus; Shift+Enter fügt
  einen Zeilenumbruch ein. Das gilt auch für die OpenCode-Session-Promptleiste.
  Das geöffnete Zuordnungsmenü nutzt gleich breite, mehrzeilige Auswahlzeilen:
   Titel in normaler Schriftgröße umbrechen; einzeilige Optionen vertikal
   zentrieren. Status als separate farbige Pill anzeigen, bei mehrzeiligen
   Titeln oben rechts.
  Freigabe bleibt ausdrücklich über „Einreihen“.
  Die MCP-Auswahl steht nur hinter dem Button „MCPs“ in der ersten Footer-
  Aktionszeile und öffnet ein Menü mit Checkboxen. Bei neuen Aufgaben sind
  alle vom Projekt freigegebenen MCPs vorausgewählt; Abwahlen bleiben im
  Entwurf erhalten. Die Session-Promptleiste bekommt keine MCP-Auswahl.
- **Icons:** Phosphor über `egui-phosphor` verwenden. Keine Emoji oder zufälligen
  Unicode-Zeichen als Icon-Ersatz. Icon-Aktionen erhalten lokalisierte Tooltips und
  zugängliche Namen. Icons nur einsetzen, wenn sie eine Funktion unterstützen.
- Gemeinsame Abstände und Komponenten zentral ändern und mit Geometrie- und
  Interaktionstests absichern, statt einzelne Ansichten unterschiedlich zu stylen.
- Toasts nutzen die Statusfarben der Pills: Erfolg grün, Fragen und Hinweise
  mit Handlungsbedarf orange, Fehler rot und Informationen blau. Bei zugehörigem
  Projekt mit Logo steht dieses links im Toast; globale Toasts haben kein Logo.
- **Hover und Linien:** Buttons (auch farbige Hauptaktionen), Tabs und Menüs
  verwenden dieselbe Hoverfarbe aus `design::HOVER_FILL`. Buttons haben keine
  Rahmenlinie. Horizontale/vertikale Trennlinien und neutrale Flächenrahmen
  verwenden `design::SEPARATOR_COLOR`: hellerer Ton der Hintergrundfarbe,
  kein neutrales Framework-Grau. Statusfarben und Fokusakzente bleiben erhalten.
- **Ticketnummern:** Serverreferenzen wie `WOAI-1` stehen vor dem Aufgabentext,
  nach Möglichkeit als neutrale Pill. Sie sind keine Statusfarbe und werden
  nicht in Prompttexte geschrieben. Keine clientseitig erfundenen Nummern.
  Buttons und Pulldowns zeigen bei Hover den Hand-Cursor; obere Tabs haben
  genau einen Titel-Tooltip (das Schließen-X hat seinen eigenen Aktionshinweis).
- **OpenCode-Ansicht:** CLI-artiger, durchgehender Monospace-Verlauf über die
  verfügbare Breite mit 14 pt Schrift für Text, Toolausgaben und Patch-Code;
  gemeinsam über `CLI_FONT_SIZE` steuern, ohne die übrige GUI zu verkleinern.
  Markdown und Code rendern, Nutzernachrichten mit linker
  Akzentlinie, kompakte benannte Toolzeilen. Keine schmalen Archivkarten und
  keine Rohdaten-/Exportknöpfe pro Nachricht im normalen Lesefluss. Technische
   Details und Archivoptionen bleiben in Verwaltungsmenüs erreichbar. Native Lesedarstellung,
   kein Terminal und keine direkte Ausführungssteuerung. Offene und normal geschlossene Arbeitssessions
   haben unter dem Verlauf den gemeinsamen Prompt-Editor mit Anhängen und
   „Prompten“: Er erzeugt eine fest zugeordnete Factory-Folgeaufgabe ohne
   Orchestrator, keine direkte OpenCode-Ausführung. Runner/MCP/Modell werden geerbt.
  Gesendete Prompts sofort als Nutzernachricht im Verlauf anzeigen; ihren
  Zustellstatus kenntlich machen und sie durch die echte Archivnachricht ersetzen.
  CLI-Zeilen ohne zusätzlichen vertikalen Widgetabstand rendern. Keine vertikalen
  Pixel-Polster an Nachrichten: zwischen Nachrichten genau eine Leerzeile in
  tatsächlicher Monospace-Zeilenhöhe, sonst normale Folgezeilen. Keine Fußzeilen
  mit Agent oder absoluten Output-Tokenzahlen und keine Nachrichtenmenüs im
  Lesefluss. Unter abgeschlossenen Antworten dezent Modell, Dauer und tok/s
  anzeigen, soweit die Nachricht die Daten enthält.
  Eingaben erhalten oben und unten jeweils eine hinterlegte Leerzeile in echter
  Monospace-Zeilenhöhe. Der blaue Marker ist 6 pt breit und hat exakt dieselbe
  obere und untere Kante wie der Hintergrund. Auch Eingaben haben kein Menü.
   In der OpenCode-Ansicht kein „Ansicht und Archiv“-Button; Archivexport und
   technische Details gehören in die Verwaltungsmenüs.
  Toolblöcke dezent grau hinterlegen, Aufruf und Ausgabe in Grautönen. Keine
  Originaldaten, Eingabe-/Ausgabeüberschriften oder Menüs im
  Toolblock. Zwischen Aufruf und Ausgabe höchstens eine echte Leerzeile.
  Aufeinanderfolgende Tool-Aktivität in standardmäßig eingeklappten Gruppen
  mit Zählern anzeigen (`+` geschlossen, `-` offen). In offenen Gruppen
  steht jeder Tool Call als eigener, nicht einzeln klappbarer Block. Vor dem
  Aufruf steht `$` statt eines Pfeils. Oben/unten jeweils eine hinterlegte
  Leerzeile in Monospace-Zeilenhöhe.
  Tool- und Eingabeblöcke haben exakt dieselbe linke Kante und volle Breite.
  Patches mit sprachabhängigen Syntaxfarben und Zeilennummern rendern;
  Hinzufügungen/Entfernungen mit eigener Zeilenhinterlegung markieren. Textdateien
  schlicht darstellen, aber ihre Änderungen ebenfalls hervorheben.

Tests dürfen keine zuvor gestoppten Factory-Dienste oder Benutzer-GUIs starten.
Für diese Regeln reichen zustandslose egui-Render-/Interaktionstests.

Die native Fensteranbindung nutzt SDL3 statt eframe/winit. Unter Wayland nativ
bleiben, keinen XWayland-Zwang oder stillen X11-Fallback einbauen. Drag-Eintritt
und -Abbruch müssen sichtbar werden; Datei-Drops einmalig in den offenen lokalen
Aufgabenentwurf übernehmen. Native Ereignisübersetzung zustandslos testen.

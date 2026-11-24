# Verbindliche GUI-Designregeln

Die vom Nutzer festgelegten Regeln gelten für alle neuen und überarbeiteten
Oberflächen dieser Crate. Bestehendes Dark Theme und deutsche Bedienung erhalten.

- **Textfelder:** ausgewogener Innenabstand, auch links/rechts. Standard 13 pt
  horizontal und 11 pt vertikal statt des knappen egui-Standards.
  Zentrale Konstruktoren in `src/design.rs` verwenden, einschließlich
  Suche, Zugangsdaten, Prompt, Zahlen-/JSON-Texte und mehrzeilige Eingaben.
  Strg+C muss markierten Text auch an andere native Wayland-Anwendungen liefern;
  leere Kopierausgaben dürfen bestehende Zwischenablageinhalte nicht löschen.
- **Listen sind Listen, keine Ansammlung von Buttons.** Gleichrangige Zeilen
  belegen dieselbe volle Breite. Labels linksbündig; Zähler innerhalb der Zeile
  rechts. Keine unterschiedlich langen Buttonflächen abhängig vom Text oder Badge.
  Navigation nutzt ruhige Zeilen mit Hover-/Auswahlmarkierung. Aktionsbuttons
  (z. B. „+ Aufgabe“) bleiben klar unterscheidbar.
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
  Der Dialog wächst mit dem Inhalt bis zur verfügbaren Fensterhöhe, erst dann scrollt der Inhalt.
  Unter dem Textfeld steht eine einzige feste Aktionszeile außerhalb des
  Scrollbereichs: schmale Zuordnung, Phosphor-Icons für Dateiauswahl und
   Bild-Zwischenablage gemeinsam linksbündig, Speicher-Icon und „Einreihen & neu“
   sowie „Einreihen“ rechts. „Einreihen“ schließt nach Serverbestätigung;
   „Einreihen & neu“ öffnet einen leeren Auftrag im selben Projekt. Die
   Bestätigung erscheint als Toast, nicht im Dialog. Zuordnung
  zeigt nur offene, nicht beendete Sessions mit ihrem OpenCode-Titel statt IDs.
  Das geöffnete Zuordnungsmenü nutzt gleich breite, mehrzeilige Auswahlzeilen:
   Titel in normaler Schriftgröße umbrechen; einzeilige Optionen vertikal
   zentrieren. Status als separate farbige Pill anzeigen, bei mehrzeiligen
   Titeln oben rechts.
  Freigabe bleibt ausdrücklich über „Einreihen“.
- **Icons:** Phosphor über `egui-phosphor` verwenden. Keine Emoji oder zufälligen
  Unicode-Zeichen als Icon-Ersatz. Icon-Aktionen erhalten deutsche Tooltips und
  zugängliche Namen. Icons nur einsetzen, wenn sie eine Funktion unterstützen.
- Gemeinsame Abstände und Komponenten zentral ändern und mit Geometrie- und
  Interaktionstests absichern, statt einzelne Ansichten unterschiedlich zu stylen.
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
  kein Terminal, keine Chat-Eingabe und keine Ausführungssteuerung.
  CLI-Zeilen ohne zusätzlichen vertikalen Widgetabstand rendern. Keine vertikalen
  Pixel-Polster an Nachrichten: zwischen Nachrichten genau eine Leerzeile in
  tatsächlicher Monospace-Zeilenhöhe, sonst normale Folgezeilen. Keine Fußzeilen
  mit Agent/Modell/Laufzeit/Output-Token oder Nachrichtenmenüs im Lesefluss.
  Eingaben erhalten oben und unten jeweils eine hinterlegte Leerzeile in echter
  Monospace-Zeilenhöhe. Der blaue Marker ist 6 pt breit und hat exakt dieselbe
  obere und untere Kante wie der Hintergrund. Auch Eingaben haben kein Menü.
   In der OpenCode-Ansicht kein „Ansicht und Archiv“-Button; Archivexport und
   technische Details gehören in die Verwaltungsmenüs.
  Toolblöcke dezent grau hinterlegen, Aufruf und Ausgabe in Grautönen. Keine
  Originaldaten, Eingabe-/Ausgabeüberschriften oder Menüs im
  Toolblock. Zwischen Aufruf und Ausgabe höchstens eine echte Leerzeile.
  Toolblöcke sind immer offen, nicht klappbar; vor dem Aufruf steht `$` statt
  eines Pfeils. Oben/unten jeweils eine hinterlegte Leerzeile in Monospace-Zeilenhöhe.
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

# Dauerhafte Nutzerregeln

- Intents dürfen nicht still stehen bleiben: Entweder es gibt einen
  tatsächlichen Weg zur Weiterverarbeitung oder die Inbox nennt den Blocker
  und mindestens eine konkrete Lösung. Ein bloßer Wartestatus ist keine Lösung.
- Implementer liefern den Arbeitsstand für den Review. Sie mergen nicht und
  schreiben nicht in den Zielbranch. Ein Merge ist Aufgabe des Mergers.

## Oberflächen

Diese Designregeln des Nutzers gelten bei UI-Arbeiten in diesem Repository:

- Textfelder haben einen ausgewogenen, sichtbar angenehmen Innenabstand auf
  allen Seiten. Links und rechts ähnlich viel Platz wie oben und unten; etwas
  weniger als Buttons, aber nicht der knappe Framework-Standard.
- Platzhalter und Beispieltexte in leeren Textfeldern sind grau und kursiv
  (egui: `design::input_hint`). Eingegebene oder vorausgefüllte Werte behalten
  die normale Textdarstellung.
- Listen als Listen darstellen, nicht als unterschiedlich lange Buttons.
  Gleichrangige Items sind gleich breit; zusätzliche Zähler dürfen die Breite
  nicht verändern. Gilt insbesondere für die Sidebar-Navigation.
- Dialogfenster haben einen angenehmen Abstand zum Fensterrand und ausreichenden
  umlaufenden Innenabstand. Oben rechts steht typischerweise ein Schließen-X;
  beim Scrollen bleibt es erreichbar. Schließen verwirft keine Entwürfe.
- Kurze Bestätigungsdialoge (z. B. „Intent löschen“) bleiben in Breite und
  Höhe kompakt: kein Leerraum bis zum Fensterrand, die Buttons stehen direkt
  unter dem Inhalt. Die Fensterhöhe ist nur eine Obergrenze. Erst bei Überlauf
  scrollt der Inhalt; Titel, Schließen-X und Aktionen bleiben sichtbar. Bei
  neuen oder geänderten Dialogen die tatsächliche Größe in einem kleinen und
  einem großen Fenster per UI-Test prüfen.
- Benötigte Icons mit Phosphor einbinden, nicht durch Emoji oder beliebige
  Unicode-Symbole ersetzen. Icon-Aktionen erhalten verständliche Namen/Tooltips.
- Fehler aus Serveranfragen, Anmeldungen und anderen fehlgeschlagenen Aktionen
  nie inline in Formularen oder Ansichten anzeigen. Stattdessen einen kurzen,
  verständlichen Toast zeigen. Technische HTTP-Details gehören in die Diagnose.
  Lokale Eingabehinweise dürfen direkt am Feld stehen.
- Verbindungswechsel kurz als Toast melden. Der dauerhafte Offline-Status
  steht im Footer und am Serverknoten der Infrastruktur. Ansichten zeigen
  verfügbare Cache-Daten ohne wiederholte Offline-Hinweise.
- Einstellungen mit explizitem Speichern haben einen einheitlichen Button
  „Speichern“, unten rechts außerhalb des scrollbaren Inhalts.
- Einstellungsformulare richten sich nach „Default-Modelle“: Label direkt am
  Eingabefeld, danach ein sichtbarer, einheitlicher Abstand zum nächsten Feld
  (egui: 8 pt zusätzlich über `design::settings_field`). Dialoge wachsen und
  schrumpfen mit den sichtbaren Feldern. Erst bei ausgeschöpfter Fensterhöhe
  scrollt der Inhalt; Kopf und Speichern bleiben erreichbar.

Konkrete egui-Komponenten und Abstände: `crates/factory-gui/AGENTS.md` und
`crates/factory-gui/src/design.rs`. Das vom Nutzer gewünschte Dark Theme behalten.
Zuvor gestoppte Factory-Dienste nicht für Designarbeiten wieder starten.

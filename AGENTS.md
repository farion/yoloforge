# Dauerhafte Nutzerregeln für Oberflächen

Diese Designregeln des Nutzers gelten bei UI-Arbeiten in diesem Repository:

- Textfelder haben einen ausgewogenen, sichtbar angenehmen Innenabstand auf
  allen Seiten. Links und rechts ähnlich viel Platz wie oben und unten; etwas
  weniger als Buttons, aber nicht der knappe Framework-Standard.
- Listen als Listen darstellen, nicht als unterschiedlich lange Buttons.
  Gleichrangige Items sind gleich breit; zusätzliche Zähler dürfen die Breite
  nicht verändern. Gilt insbesondere für die Sidebar-Navigation.
- Dialogfenster haben einen angenehmen Abstand zum Fensterrand und ausreichenden
  umlaufenden Innenabstand. Oben rechts steht typischerweise ein Schließen-X;
  beim Scrollen bleibt es erreichbar. Schließen verwirft keine Entwürfe.
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

Konkrete egui-Komponenten und Abstände: `crates/factory-gui/AGENTS.md` und
`crates/factory-gui/src/design.rs`. Das vom Nutzer gewünschte Dark Theme behalten.
Zuvor gestoppte Factory-Dienste nicht für Designarbeiten wieder starten.

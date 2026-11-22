# OpenCode-Sessions in der TUI

`4` → Arbeitsbereich → Enter öffnet direkt die Live-Unterhaltung der zugehörigen
OpenCode-Session. Auch Enter auf einer bereits eingeplanten Aufgabe führt dorthin.
Die Ansicht zeigt Prompts, Antworten und Werkzeugausgaben; sie folgt automatisch
neuen Nachrichten. Der technische Steuerungsdialog liegt hinter `c`.

- Hoch/runter oder Bild hoch/runter: lesen; Ende: wieder live folgen.
- `f`: offene Frage oder Berechtigung dieser Session beantworten. Esc kehrt zur
  Unterhaltung zurück; Antworten behalten die vorhandene revisionsgeschützte Zustellung.
- `[` / `]`: frühere/spätere Ausführung im Arbeitsbereich; `l`: neueste verfolgen.
- `a`: vollständiges gespeichertes Archiv; Esc: zurück zur Unterhaltung.
- `c`: Ausführungssteuerung; Esc: zurück zur Unterhaltung.
- Esc in der Unterhaltung: zurück zur Liste.

Die Live-Ansicht ist ein YoloForge-Frontend für dieselbe OpenCode-Session, keine
eingebettete Kopie des OpenCode-Terminalprogramms. Sie funktioniert über Factory
und Runner auch für entfernte Sessions, ohne OpenCode-Zugangsdaten auf den TUI-Client
zu kopieren. Der Runner synchronisiert Nachrichten; die TUI lädt den Auszug jede
Sekunde. Es handelt sich nicht um direktes Token-Streaming. Gekürzte Ausgaben und
ausgelassene ältere Nachrichten sind gekennzeichnet und über das Archiv erreichbar.
Bei Verbindungsabbruch bleibt der letzte geladene Stand sichtbar.

Geprüft: Unicode-/Tooldarstellung, Kennzeichnung von Kürzungen, automatischer
Sessionwechsel, festgehaltene Historie und Zuordnung von Rückfragen. Der PTY-Test
`uv run --with pyte python scripts/smoke-session-view.py` öffnet mit den üblichen
Factory-Zugangsdaten eine bestehende Session und prüft Live-Ansicht, Steuerung und
Rückkehr. Er verändert weder Aufgaben noch Sessions.

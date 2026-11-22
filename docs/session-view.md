# OpenCode-Sessions in der TUI

`4` → Arbeitsbereich → Enter öffnet die **originale OpenCode-TUI** auf der bereits
vorhandenen Session. Auch Enter auf einer bereits eingeplanten Aufgabe führt
dorthin. OpenCode übernimmt das aktuelle Terminal; beim Beenden kehrt die Anzeige
zu YoloForge zurück. Es wird kein zusätzlicher Prompt gesendet.

Vor dem Start prüft der Client den OpenCode-Service, die konkrete Session-ID und
den bestätigten Arbeitsordner. Der Aufruf nutzt `opencode --server ... --session ...`;
das Passwort wird ausschließlich im Kindprozess-Environment übergeben. Fehler beim
Start stellen die YoloForge-Oberfläche wieder her und bleiben sichtbar.

Der native Einstieg unterstützt derzeit Services und Arbeitsordner, die auf dem
TUI-Rechner erreichbar sind (einschließlich einer lokalen Service-Registrierung).
Fuer SSH-Runner ist jetzt ein nativer SSH-Einstieg mit lokalen Profilen vorhanden
(siehe `ssh-sessions.md`; separate Host-Abnahme steht aus). Für andere entfernte Runner
und für die Factory-Historie bleibt `v` als synchronisierte Live-Ansicht verfügbar.
`o` öffnet aus dieser Ansicht die jeweils ausgewählte Session im nativen OpenCode.
Der technische Steuerungsdialog liegt hinter `c`.

Tasten in der Factory-Live-Ansicht (`v`):

- Hoch/runter oder Bild hoch/runter: lesen; Ende: wieder live folgen.
- `f`: offene Frage oder Berechtigung dieser Session beantworten. Esc kehrt zur
  Unterhaltung zurück; Antworten behalten die vorhandene revisionsgeschützte Zustellung.
- `[` / `]`: frühere/spätere Ausführung im Arbeitsbereich; `l`: neueste verfolgen.
- `a`: vollständiges gespeichertes Archiv; Esc: zurück zur Unterhaltung.
- `c`: Ausführungssteuerung; Esc: zurück zur Unterhaltung.
- Esc in der Unterhaltung: zurück zur Liste.

Die zusätzliche Live-Ansicht ist ein YoloForge-Frontend für dieselbe OpenCode-Session.
Sie funktioniert über Factory
und Runner auch für entfernte Sessions, ohne OpenCode-Zugangsdaten auf den TUI-Client
zu kopieren. Der Runner synchronisiert Nachrichten; die TUI lädt den Auszug jede
Sekunde. Es handelt sich nicht um direktes Token-Streaming. Gekürzte Ausgaben und
ausgelassene ältere Nachrichten sind gekennzeichnet und über das Archiv erreichbar.
Bei Verbindungsabbruch bleibt der letzte geladene Stand sichtbar.

Geprüft: Unicode-/Tooldarstellung, Kennzeichnung von Kürzungen, automatischer
Sessionwechsel, festgehaltene Historie und Zuordnung von Rückfragen. Der PTY-Test
`uv run --with pyte python scripts/smoke-session-view.py` öffnet mit den üblichen
Factory-Zugangsdaten eine bestehende Session und prüft Live-Ansicht, Steuerung und
Rückkehr. Mit `--native` prüft er zusätzlich die originale OpenCode-TUI und die
Rückkehr zu YoloForge; mit `--missing-cli` die Terminalwiederherstellung bei fehlendem
Executable. Beide Prüfungen sind mit der lokalen Minesweeper-Session bestanden.
Es werden keine Prompts gesendet; OpenCode kann die Session als angesehen markieren.

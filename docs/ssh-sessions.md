# Originale OpenCode-Session auf SSH-Runnern

Die TUI kann eine Session ueber einen vom Benutzer provisionierten SSH-Zugang
oeffnen. Auf dem Zielhost muessen `factory-runner` mit `--open-session` und
`opencode` im PATH liegen. Der laufende Runner wird dabei nicht neu registriert,
nicht gestoppt und sein Journal nur lesend geoeffnet.

## Verbindung zuordnen

In der TUI: **1 (Runner) → Runner auswaehlen → s**. Host/Alias, Factory-URL
aus Sicht des Runners, absoluter Journalpfad und optional Registrierung eintragen.
**Ctrl-S** speichert, **Ctrl-D** entfernt die lokale Zuordnung, **Ctrl-R** laedt
neu (verwirft den Entwurf), **Esc** schliesst. Auch offline nutzbar. Die Runner-ID
bleibt beim Bearbeiten fest; konkurrierende Dateiänderungen werden nicht
ueberschrieben. Schreiben erfolgt atomar unter einer lokalen Dateisperre.

Lokale Datei `$XDG_CONFIG_HOME/yoloforge/ssh.json`, standardmaessig
`~/.config/yoloforge/ssh.json`; alternativ `YOLOFORGE_SSH_CONFIG`:

```json
{
  "00000000-0000-0000-0000-000000000001": {
    "host": "mein-runner-ssh-alias",
    "server": "http://127.0.0.1:7432",
    "journal": "/home/me/yoloforge/runner.sqlite3"
  }
}
```

Der Schluessel ist die echte Runner-ID. `server` muss der im Runner-Journal
gespeicherten Factory-URL entsprechen. `journal` ist der Pfad auf dem Zielhost.
Optional kann `opencode_registration` fuer nicht projektgebundene Alt-Sessions
gesetzt werden. Die Konfiguration enthaelt keine Passwoerter. SSH-Alias,
Schluessel und Host-Key-Pruefung werden von der vorhandenen SSH-Konfiguration
uebernommen; es gibt keine automatische Provisionierung.

Danach oeffnet **Enter auf einer Session** die originale OpenCode-TUI per `ssh -t`.
OpenCode liest seine Service-Zugangsdaten ausschliesslich auf dem Runner-Host.
Der Helfer prueft Journalidentitaet, konkrete Command-ID, aufgezeichneten
OpenCode-Endpunkt, existierende Session und Arbeitsordner. Beim Beenden kehrt
die lokale YoloForge-TUI zurueck. Es wird kein neuer Prompt gesendet.

## Remote-Bereitschaft und Anmeldung

Bei einem Projekt mit vorhandener Agent-Session und SSH-Zuordnung verwendet
**r** in der Projektliste `ssh -T` mit BatchMode und Zeitlimit. Der neue Helfer
`factory-runner --open-session COMMAND_ID --session-readiness` prueft dieselbe
Identitaets-/Workspacebindung und liest Provider-/MCP-Status auf dem Runner.
Er braucht keinen Factory-Token und nimmt keinen Runner-Prozesslock.
Die TUI zeigt das aktuelle Projektmodell separat vom gepinnten Ausfuehrungsmodell.
**o** oeffnet die vorhandene entfernte Session zur nativen OpenCode-Anmeldung.
Vor der ersten Agent-Session ist Remote-Projektbereitschaft noch nicht verfuegbar.

## Abnahme und Grenzen

- Tests fuer lesenden Journalzugriff bei gehaltenem Runner-Lock, falsche
  Journalidentitaet, Shell-Quoting und ungueltige SSH-Ziele bestanden.
- `scripts/smoke-runner-attach.py` hat die bestehende Minesweeper-Session lokal
  ohne Factory-Token mit erfolgreicher Rueckkehr geoeffnet.
- `scripts/smoke-ssh-editor.py` prueft Paste, Speichern, Wiedereroeffnen und
  Entfernen in einer isolierten Konfigurationsdatei per PTY. Konflikt-/Erhaltungstests
  fuer mehrere Runner und begrenzte, bereinigte SSH-Ausgabe sind bestanden.
- Der Bereitschaftshelfer wurde lokal gegen die echte OpenAI-Session ohne
  Factory-Token bei laufendem Runner geprueft.
- Der SSH-Hop auf einen separaten Host und echte Remote-OAuth-Anmeldung sind
  noch nicht abgenommen. Es wurde kein Zielhost aktualisiert.

# Originale OpenCode-Session auf SSH-Runnern

Die TUI kann eine Session ueber einen vom Benutzer provisionierten SSH-Zugang
oeffnen. Auf dem Zielhost muessen `factory-runner` mit `--open-session` und
`opencode` im PATH liegen. Der laufende Runner wird dabei nicht neu registriert,
nicht gestoppt und sein Journal nur lesend geoeffnet.

## Verbindung zuordnen

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

## Abnahme und Grenzen

- Tests fuer lesenden Journalzugriff bei gehaltenem Runner-Lock, falsche
  Journalidentitaet, Shell-Quoting und ungueltige SSH-Ziele bestanden.
- `scripts/smoke-runner-attach.py` hat die bestehende Minesweeper-Session lokal
  ohne Factory-Token mit erfolgreicher Rueckkehr geoeffnet.
- Der SSH-Hop auf einen separaten Host ist noch nicht abgenommen. Es wurde kein
  Zielhost aktualisiert. Die Profilbearbeitung erfolgt bislang ueber diese Datei;
  ein TUI-Verbindungseditor und Remote-Provider-Anmeldung bleiben offen.

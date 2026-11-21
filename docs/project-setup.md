# Projektanlage in der TUI

**Projektkonfiguration** und **Service-Verbindung** sind verschiedene Dinge:

- Das Projekt besitzt Provider/Modell, Toolchain und Start-/Testbefehle sowie
  seine Harness-Konfiguration: Agenten, MCPs, Plugins, Skills und Regeln.
- Jeder zugeordnete Runner hat einen eigenen Repository-Pfad und gegebenenfalls
  einen eigenen Pfad zur OpenCode-Service-Registrierung auf seinem Rechner.
- Jeder Stream bekommt einen eigenen Worktree und eine eigene Session. Der native
  OpenCode-Service kann mehrere Projekte bedienen; damit ist keine separate
  Container-Isolation verbunden. Globale und übergeordnete OpenCode-Konfiguration
  wird weiterhin nach den nativen OpenCode-Regeln berücksichtigt.

## Bedienung

Server zuerst aktualisieren, dann Runner/TUI neu starten (DB **21**, Journal **7**).
Für neue Projekte: `2` → `n`. Für ein vorhandenes Projekt: `2` → Projekt → `p`.

Der Assistent fragt der Reihe nach:

1. Projektname, Repository-Metadaten und Basisbranch.
2. Ausführungsort: einen registrierten Runner mit Leertaste auswählen.
3. Absoluten Pfad des vorhandenen Git-Repositories **auf diesem Runner**.
4. OpenCode-Verbindung: leer lassen für den nativen Service des Runner-Benutzers;
   alternativ den absoluten Pfad seiner privaten Registrierungsdatei eingeben.
5. Provider und Modell-ID.
6. Projekt-Harness: leer lassen, um die mit dem Repository ausgecheckte
   `opencode.json(c)`/`.opencode`-Konfiguration zu verwenden. Alternativ ein
   JSON-Objekt für eine neue `opencode.json` eingeben oder einfügen.
7. Toolchain, Start- und Testbefehl.

`Tab`/`Shift-Tab` wechseln die Felder, `Ctrl-S` reiht die Einrichtung ein.
Der Dialog zeigt Fortschritt und das Ergebnis des Runners automatisch an.
Bei unklarer Zustellung wiederholt `Ctrl-S` denselben Request; `Ctrl-R` ist erst
nach einer eindeutig abgewiesenen Änderung zum bewussten Rebase verfügbar.

Projekt, Profil, Runner-Zuordnung, Einrichtungsstream und Vorbereitungsauftrag
werden atomar gespeichert. Offline-Runner behalten den Auftrag in der Queue.
Der Runner legt den Worktree an, erstellt gegebenenfalls die neue Harness-Datei
und prüft OpenCode 2.0.27. Im automatischen Verbindungsmodus findet er die native
Registrierung unter `$XDG_STATE_HOME/opencode/service.json` beziehungsweise
`~/.local/state/opencode/service.json` und ruft bei Bedarf `opencode service start`
auf. Dafür muss OpenCode auf dem Runner installiert sein.

Eine manuelle `repositories.json` oder ein Runner-Neustart mit projektspezifischen
Argumenten ist für so eingerichtete Projekte nicht mehr nötig. Nach erfolgreicher
Vorbereitung: `Esc` → `4` → angezeigten Stream öffnen → Agent starten.

Weitere Runner können über `p` für dasselbe Projekt eingerichtet werden. Vorhandene
Runner-Pfade bleiben erhalten. Beim Wechsel des Runners im Dialog werden dessen
gespeicherte Pfade geladen; für neue Runner werden diese Felder geleert. Die
Harness bleibt projektweit gemeinsam. Jeder neue Einrichtungsvorgang erstellt
einen separaten Vorbereitungsstream. Alte Aufträge behalten ihre gespeicherte
Konfiguration, und Folgeaufträge eines vorbereiteten Streams behalten dessen
Workspace-Spezifikation.

## Harness-Beispiel

```json
{
  "$schema": "https://opencode.ai/config.json",
  "instructions": ["AGENTS.md"],
  "mcp": {
    "servers": {
      "playwright": {
        "type": "local",
        "command": ["bunx", "@playwright/mcp"]
      }
    }
  }
}
```

Die benötigten Programme müssen auf dem Ausführungsort verfügbar sein. Zugangsdaten
gehören nicht in dieses im Factory-Zustand gespeicherte JSON; native OpenCode-
Credentials bleiben beim Service. Vorhandene abweichende Konfigurationsdateien
werden nicht überschrieben. Im Repository-Modus werden nur ausgecheckte Dateien
verwendet; uncommittete/ignorierte Konfiguration der ursprünglichen Arbeitskopie
wird nicht automatisch kopiert. Die generierte Datei liegt nur im neuen Worktree,
nicht in der ursprünglichen Arbeitskopie.

## Noch offene Assistent-Schritte

Der aktuelle Assistent ist ein geführtes Formular mit Statusanzeige. MCPs/Agenten
werden noch als Projekt-JSON eingegeben; ein eigener strukturierter Editor und
eine Auswahl der tatsächlich verfügbaren Modelle fehlen. OAuth-/Provider- und
MCP-Anmeldung sind noch nicht integriert. **Service erreichbar bedeutet nicht,
dass der Provider angemeldet oder jeder MCP verbunden ist.** Automatisches
Clone/Fetch im Assistenten und Container-Provisionierung bleiben ebenfalls offen.

Geprüft mit `scripts/smoke-project-setup.py`: echte TUI, Factory, Runner und Git-
Worktree; synthetischer OpenCode-Health-Endpunkt, kein Modellaufruf. Zusätzliche
Tests prüfen atomaren Rollback, Request-Replay nach Neustart, alte Runner,
Konfigurationsbindung und Schutz vorhandener Dateien/Symlinks. Der automatische
native Servicestart ist über `scripts/smoke-project-setup.py --auto` mit einem
kontrollierten Service-Executable geprüft, aber noch nicht mit einem echten
OpenCode-Service als eigener Prozessfall abgenommen.

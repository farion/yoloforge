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

Server zuerst aktualisieren, dann Runner/TUI neu starten (DB **22**, Journal **8**).
Für neue Projekte: `2` → `n`. Für ein vorhandenes Projekt: `2` → Projekt → `p`.

**Ohne technische Eingaben:** `2` → `n` → `Ctrl-S`. Ein registrierter Runner
wird vorbelegt (vorzugsweise online), ein leeres Git-Repository mit Initialcommit
und ein Worktree werden automatisch erstellt. Auch der Projektname ist vorbelegt
und kann später geändert werden. Repository, lokale Pfade, Provider/Modell und
Harness-Dateien müssen nicht angegeben werden. Bei mehreren Runnern kann die
automatische Auswahl im Assistenten geändert werden.

Provider und Modell bleiben standardmäßig leer. Vor der ersten Session liest der
Runner OpenCodes Standardmodell für den Worktree und speichert die konkreten IDs
in seinem Journal. Konfigurationsänderungen und Neustarts führen dadurch nicht
zum unbemerkten Modellwechsel. Fortsetzungen übernehmen das Modell ihres
Vorgängers. Fehlt ein nutzbares Standardmodell, wird das vor Session-/Prompt-Anlage
als eigener Status gemeldet; die Provider-Anmeldung ist weiterhin Voraussetzung.

Der Assistent fragt der Reihe nach:

1. Optional Projektname ändern, Repository importieren oder Basisbranch ändern.
2. Optional einen anderen registrierten Runner mit Leertaste auswählen.
3. Optional einen eigenen Checkout **auf diesem Runner** angeben. Leer bedeutet:
   Factory verwaltet den Speicherort. Ohne Repository-Quelle wird ein neues
   Projekt initialisiert, mit Quelle wird ein eigener Bare-Clone angelegt.
4. OpenCode-Verbindung: leer lassen für den nativen Service des Runner-Benutzers;
   alternativ den absoluten Pfad seiner privaten Registrierungsdatei eingeben.
5. Optional Provider und Modell-ID gemeinsam angeben; beide leer nutzt OpenCodes Standard.
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

Verwaltete Repositories liegen unter `WORKSPACE_ROOT/.repositories/PROJECT_ID/`.
Der Runner reserviert für jeden Erstellungsversuch einen neuen Unterordner und
journaliert die Absicht vor Git-Operationen. Erst ein erfolgreicher Initialcommit
beziehungsweise Clone wird als Projekt-Repository veröffentlicht. Bestehende oder
ersetzte Pfade werden nicht überschrieben. Neustarts verwenden die bestätigte
Zuordnung weiter und erhalten schmutzige Worktrees. Fehlgeschlagene/unklare
Versuche bleiben zur Untersuchung erhalten; ein neuer Vorbereitungsauftrag kann
in einem neuen Ordner erneut versuchen. Es wird nichts automatisch gelöscht.
Für optionale Imports gelten die Git-/SSH-Zugänge des Runner-Benutzers; Clone
läuft ohne interaktive Git-Prompts. Fetch und Hosting-Anmeldung bleiben separat.

Die Einrichtung startet noch keine fachliche Aufgabe: dazu einen Agentenauftrag
mit Arbeitsziel starten oder ein Ready-Ticket über den aktivierten Scheduler
planen lassen.

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

## Strukturierter MCP-Editor

Die optionalen Felder 13–17 bearbeiten benannte lokale/entfernte MCPs und ergänzen
Regeldateien, Globs oder URLs. Für lokale MCPs Programm und Argumente getrennt
angeben: **ein Argument je Zeile**, ohne Shell-Quotes. Für entfernte MCPs eine
HTTP(S)-URL angeben. OAuth bleibt standardmäßig aktiviert; der Editor setzt es
nicht stillschweigend auf `false`.

- Bestehenden Namen eingeben, `Ctrl-L` lädt Programm/URL und Argumente.
- Aktion wählen: lokal, entfernt, deaktivieren, aktivieren oder entfernen.
- `Ctrl-A` übernimmt in den lokalen Harness-Entwurf und leert die MCP-Felder für
  den nächsten Server. Es wird noch nichts an Factory gesendet.
- `Ctrl-S` übernimmt die noch offene Bearbeitung und reiht die Einrichtung ein.

Unbeteiligte MCPs, Agenten, Plugins, Header, OAuth-/Umgebungsoptionen und Timeouts
bleiben erhalten. Ein Typwechsel verlangt explizites Entfernen und Neuanlegen.
Fortgeschrittene Optionen sind weiterhin im JSON editierbar. Regeldateien werden
ohne Duplikate ergänzt. Unklare Zustellung friert auch diese Änderungen ein.
Der Editor verwaltet die Factory-Projekt-Harness; er schreibt nicht in eine
vorhandene abweichende Repository-Konfiguration. MCP-Verbindung und Anmeldung
werden durch diesen Konfigurationsschritt noch nicht bestätigt.

## Noch offene Assistent-Schritte

Der aktuelle Assistent ist ein Formular mit automatischen Vorgaben und Statusanzeige.
Strukturierte Agenten- und erweiterte MCP-Optionen sowie eine Modellauswahlliste
fehlen noch. OAuth-/Provider- und MCP-Anmeldung sind noch nicht integriert.
**Service erreichbar bedeutet nicht, dass der Provider angemeldet oder jeder MCP
verbunden ist.** Fetch im Assistenten und Container-Provisionierung bleiben offen.

Geprüft mit `scripts/smoke-project-setup.py`: echte TUI, Factory, Runner und Git-
Worktree; synthetischer OpenCode-Health-Endpunkt, kein Modellaufruf. Zusätzliche
Tests prüfen atomaren Rollback, Request-Replay nach Neustart, alte Runner,
Konfigurationsbindung und Schutz vorhandener Dateien/Symlinks. Der automatische
native Servicestart ist über `scripts/smoke-project-setup.py --auto` mit einem
kontrollierten Service-Executable geprüft, aber noch nicht mit einem echten
OpenCode-Service als eigener Prozessfall abgenommen.
`--empty` prüft die Projektanlage mit ausschließlich `2`, `n`, `Ctrl-S`, ohne
Repository, Pfad, Modell oder Konfiguration. Der normale Test prüft zusätzlich
den strukturierten MCP-Editor. Runner-Tests prüfen lokale Imports, verwaltete leere
Projekte, Pfadkonflikte und die Modellbindung über Neustarts.

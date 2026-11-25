# Native Runner installieren

`factory-runner` läuft nativ auf Linux (x86-64/ARM64), macOS (Intel/Apple Silicon)
und Windows (x86-64). Git muss installiert und im `PATH` erreichbar sein.
Die GUI-Pakete enthalten denselben Runner neben dem GUI-Binary; unter macOS
liegen beide in `YoloForge.app/Contents/MacOS`.

## Start

Ein Zugang mit Rolle `runner` muss für die verwendete Runner-ID registriert sein.
Zugangsdaten gehören in eine private Datei, nicht in Kommandozeilenargumente.
Serverzugriff erfolgt über HTTPS oder einen lokalen SSH-Tunnel.

Linux/macOS:

```sh
export FACTORY_TOKEN_FILE="$HOME/.config/yoloforge/runner.token"
./factory-runner --server https://factory.example \
  --id RUNNER-UUID --name laptop --journal /ABSOLUTER/PFAD/runner.sqlite3 \
  --managed-opencode /ABSOLUTER/PFAD/managed-opencode.json
```

Windows (PowerShell):

```powershell
$env:FACTORY_TOKEN_FILE = "$env:LOCALAPPDATA\YoloForge\runner.token"
.\factory-runner.exe --server https://factory.example `
  --id RUNNER-UUID --name laptop --journal C:\YoloForge\runner.sqlite3 `
  --managed-opencode C:\YoloForge\managed-opencode.json
```

Die JSON-Datei enthält einen festen freien Loopback-Port und den absoluten
Pfad zur privaten Zugangsdaten-Datei:

```json
{"port":17434,"model_key_file":"/ABSOLUTER/PFAD/runner.token"}
```

Unter Windows JSON-Pfade als `C:/YoloForge/runner.token` oder mit doppelten
Backslashes angeben. Dateien mit Geheimnissen dürfen nur dem aktuellen Benutzer
zugänglich sein: Unix-Modus `0600`, unter Windows eine entsprechend eingeschränkte
DACL. Vom Runner erzeugte Runtime-Verzeichnisse erhalten diese Rechte automatisch.
Breit freigegebene bestehende Runtime-Verzeichnisse werden abgewiesen.

Der Runner lädt die plattformspezifische, per SHA-512 gepinnte OpenCode-Version
in seinen privaten Cache. Alternativ `FACTORY_OPENCODE_BIN` auf einen absoluten
Pfad setzen. Die persönliche OpenCode-Konfiguration wird nicht übernommen.

## Betrieb und Grenzen

- Das Linux-Paket enthält eine systemd-Vorlage. macOS und Windows werden zunächst
  direkt oder durch die GUI gestartet; es wird kein Systemdienst installiert.
- GUI-verwaltete Runner laufen nach dem Schließen der GUI weiter. Die GUI nutzt
  einen privaten, authentifizierten Loopback-Steuerkanal zum kontrollierten
  Beenden, statt Prozesse anhand wiederverwendbarer PIDs zu signalisieren.
- Journal, Worktrees und Runtime-Daten bleiben beim Upgrade erhalten und sind
  niemals Bestandteil eines Release-Pakets.
- Container-Containment bleibt an Linux-Runtimes gebunden. Windows führt
  Container-Reviews nicht mit abgeschwächter Isolation aus; dafür einen
  Linux-Review-Runner verwenden. Native Git-/Agent-Arbeit funktioniert unabhängig
  davon. Auf macOS benötigen Container-Reviews einen passenden lokalen
  Docker-Unix-Socket und exakt verfügbare Worktree-Bind-Mounts.
- Die Server-Pakete und die lokale Factory-Einrichtung bleiben Linux-only.
  macOS-/Windows-GUIs verbinden sich mit einer bestehenden Factory und können
  dabei native lokale Runner verwalten.
- Desktop-Pakete sind derzeit nicht mit einer Herstelleridentität signiert
  oder notarisiert. macOS/Gatekeeper und Windows/SmartScreen können warnen.

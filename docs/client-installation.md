# YoloForge TUI

Dieses Paket enthält ausschließlich die Terminaloberfläche `yoloforge` und die
Lizenz. Server und Linux-Runner werden separat installiert. Die Android-App ist
noch in Entwicklung und wird später als eigene APK veröffentlicht.

## Linux

Passendes x86_64-/aarch64-Archiv entpacken, Prüfsumme prüfen und `yoloforge`
beispielsweise nach `~/.local/bin/` kopieren. Voraussetzung: Ubuntu 24.04 oder
neuer. Für lokale Agenten zusätzlich das Runner-Paket und OpenCode installieren.

## Windows

Das x86_64-ZIP entpacken, Prüfsumme prüfen und `yoloforge.exe` in Windows Terminal
oder PowerShell starten. Die TUI benötigt keinen lokalen Linux-Runner; sie kann
die Runner auf dem Server steuern. Lokale Agentenausführung unter Windows ist
noch nicht implementiert. Git for Windows wird nur für die lokalen
`repository clone/register/fetch`-Befehle benötigt.

SSH-Schlüssel und Hostprüfung werden über die vorhandene OpenSSH-Konfiguration
verwaltet. Tunnel in einem separaten Terminal starten:

```powershell
ssh -N -T -o ExitOnForwardFailure=yes -o StrictHostKeyChecking=yes -L 17432:127.0.0.1:7432 rs9tm
$env:FACTORY_TOKEN_FILE = "$env:LOCALAPPDATA\YoloForge\operator.token"
.\yoloforge.exe --server http://127.0.0.1:17432
```

Die Token-Datei liegt in einem privaten Benutzerverzeichnis; unter Windows
erben neu erstellte Dateien die ACL ihres Elternverzeichnisses. Keine gemeinsam
beschreibbaren Verzeichnisse für Enrollment-Codes und Tokens verwenden.
Unter Linux werden neue Credential-Dateien mit Modus 0600 erzeugt.

Das Admin-Gerät erzeugt einen kurzlebigen Enrollment-Code. Auf dem neuen Gerät:

```powershell
.\yoloforge.exe --server http://127.0.0.1:17432 device enroll --name windows-tui --code-file invitation.enrollment --output "$env:LOCALAPPDATA\YoloForge\operator.token"
```

Das Zielverzeichnis muss vorher existieren. Die Enrollment-Ausgabedatei darf
noch nicht existieren. Das Token wird nicht auf dem Bildschirm ausgegeben.

## Bedienung

`1` Runner · `2` Projekte · `3` Tickets · `4` Streams · `5` Aufträge · `6` Fragen.
`n` legt Projekte/Tickets/Streams an; `Enter` öffnet Details oder Dialoge,
`Ctrl-S` sendet Änderungen. `q` beendet die Hauptansicht.

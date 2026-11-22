# Binärdateien im Sessionarchiv

Der Runner sichert explizit als Dateianhang referenzierte lokale `file:`-URIs
innerhalb des Ausführungsworktrees zusätzlich zur unveränderten OpenCode-
Message-Projektion. Binärinhalt, MIME, ursprünglicher Name/URI und SHA256 stehen
unter dem reservierten Message-Feld `__yoloforge_files_v1`.
Das vorhandene atomare Archiv-Outbox-/Revisionsverfahren umfasst diese Bytes;
bei verlorenem Upload-Ergebnis wird derselbe Snapshot erneut zugestellt.

Grenzen: höchstens 16 referenzierte Dateien und insgesamt 4 MiB zusätzliche
Binärdaten je Seite; einschließlich Projektion bleibt die 8-MiB-Batchgrenze bestehen.
Dateien außerhalb des Worktrees, externe HTTP-URIs, fehlende und übergroße Dateien
werden nicht geladen. Erfasste erfolglose Referenzen erhalten einen expliziten
Status. Bei bereits nahezu voller Message-Seite wird keine Zusatzkopie angelegt.
Ein finales Messagearchiv bedeutet daher nicht, dass sämtliche Binärreferenzen
verfügbar sind. Bereits inline vorhandene Base64-Dateien bleiben vollständig in
der ursprünglichen Projektion erhalten.

## Export

```sh
yoloforge --server https://factory.example archive COMMAND_UUID \
  --output session.jsonl --attachments session-files
```

Ausgabedatei und Attachment-Verzeichnis dürfen noch nicht existieren. Die
Dateien werden unter ihrem verifizierten SHA256 gespeichert; fremde Dateinamen
werden niemals zu Dateipfaden. `manifest.jsonl` enthält Ursprungsname, MIME,
Message-ID, Sample-Revision und Status. Inline-Base64-Anhänge können ebenfalls
bis 4 MiB je Datei extrahiert werden. Beschädigte Digest-/Base64-Daten brechen
den Export ab. Der Abschlussbericht nennt extrahierte und nicht verfügbare
Dateien. Unvollständige Exporte bleiben als solche ohne Abschlussbeleg erkennbar.

Abnahme: Escape-Symlinks/HTTP werden nicht gelesen; Byteintegrität geprüft;
`scripts/smoke-archive-files.py` prüft echte Factory-HTTP-Persistenz, identischen
Replay, Serverneustart, Löschung der Ursprungsdatei und CLI-Export. Kein Modellaufruf.
In der TUI exportiert `e` die Anhaenge der geladenen Nachrichtenversion in ein
neues privates `yoloforge-attachments-...`-Verzeichnis im aktuellen Arbeitsordner,
einschliesslich Manifest. Vorhandene Verzeichnisse/Dateien werden nicht ersetzt.
Android bietet inzwischen Archivnavigation und SHA256-geprueften Export ueber
den System-Dateidialog; die Byte-/UI-Tests bestehen, die Dateidialog-Abnahme auf
einem echten Geraet fehlt noch.

## Grosse Dateien und erhaltene externe Quellen (Schema 31)

Zusaetzliche lokale Dateien werden bis 64 MiB je Datei, 16 Dateien und 256 MiB
je Auftrag in private Runner-Spools kopiert. Der Upload erfolgt wiederaufnehmbar
in 1-MiB-Bloecken. Factory veroeffentlicht nur vollstaendige Dateien nach Pruefung
des Gesamtdigests; identische Bloecke sind wiederholbar, abweichende werden abgelehnt.
Die Metadaten enthalten Message-ID, Quellrevision und Erfassungszeitpunkt.
Eine spaeter rekonstruierte Datei ist kein Beweis fuer ihren historischen Inhalt.

`factory-runner --archive-source-root /absolute/directory` erlaubt zusaetzliche
lokale Quellverzeichnisse. Standard ist allein der Ausfuehrungsworktree. HTTP-URIs
werden nicht abgerufen. Geoeffnete Deskriptoren werden auf Rootzugehoerigkeit und
regulaere Dateien geprueft; Symlink-Escapes werden abgewiesen.

Nach Wiederherstellung einer Quelle kann beim gestoppten Runner mit dessen normalen
Identitaets-/Journalargumenten `--rescan-archive-files COMMAND_UUID` ein erneuter
Scan vorgemerkt werden. Beim naechsten Runnerstart liest er die Referenzen aus dem
zentralen Archiv, auch wenn die native Session nicht mehr existiert. Ein erhaltenes
Ausfuehrungsjournal ist erforderlich. Ohne erhaltene Bytes gibt es keine Rekonstruktion.

CLI, TUI (`e`) und Android laden die zusaetzlichen Dateien blockweise und pruefen
SHA256 vor der finalen Bereitstellung. APIs: `GET /v1/commands/{id}/files`,
`GET /v1/commands/{id}/files/{sha256}/{offset}` und registrierungs-/ownershipgebundenes
`PUT /v1/runners/{runner}/commands/{id}/files`. SQLite-Backups enthalten die Bloecke.
Alte Inline-Unverfuegbarkeitsmeldungen bleiben als historische Beobachtung erhalten.

`scripts/smoke-archive-blobs.py` hat grosse Dateien, Server-/Runner-Neustart,
Quellloeschung, externe Rekonstruktion und bytegleichen CLI-Export geprueft
(`/tmp/opencode/yf-archive-blobs-zxnu2z8u`). OpenCode ist dort synthetisch,
Factory/Runner/CLI sind echte Prozesse. Der gleiche Test bestaetigt Quarantaene
nach Verlust eines bereits angenommenen Auftragsjournals ohne Prompt-Replay.

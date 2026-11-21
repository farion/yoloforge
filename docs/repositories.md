# Lokale Repository-Einrichtung

Ab dem Entwicklungsstand nach v0.1.0 unterstützt die CLI die Einrichtung auf
dem Rechner, auf dem sie läuft. Die Projektquelle und Basisbranch werden aus
dem authentifizierten Server-Snapshot gelesen.

```sh
yoloforge repository clone --project PROJEKT-UUID \
  --destination /home/USER/repos/projekt \
  --repositories /home/USER/.config/yolofactory/repositories.json

yoloforge repository register --project PROJEKT-UUID \
  --directory /home/USER/repos/bestehend \
  --repositories /home/USER/.config/yolofactory/repositories.json

yoloforge repository fetch --project PROJEKT-UUID \
  --repositories /home/USER/.config/yolofactory/repositories.json
```

Die Elternverzeichnisse müssen vorhanden sein. `clone` verlangt ein vollständig
neues Zielverzeichnis, klont die Projektquelle mit dem eingestellten Basisbranch
und trägt erst nach erfolgreicher Prüfung den absoluten Pfad in die Zuordnung
ein. Ein vorhandener Pfad wird auch dann nicht benutzt, wenn er leer ist.
`register` verlangt einen Repository-Root mit vorhandenem lokalen Basisbranch.
Eine bestehende Projektzuordnung darf nur auf denselben Pfad bestätigt werden.
Andere Projekte in der JSON-Datei bleiben erhalten.

Die Datei wird unter einer benachbarten Dateisperre atomar ersetzt. Bei Abbruch
bleiben vorhandene Zuordnungen erhalten. Falls Clone erfolgreich war, aber die
Zuordnung fehlgeschlagen ist, kann `register` den erhaltenen Checkout nachtragen.
Unvollständige Clone-Verzeichnisse werden nicht automatisch entfernt. Andere
Programme sollen dieselbe `<repositories.json>.lock`-Sperre verwenden, wenn sie
die Zuordnung schreiben; der Runner liest weiterhin atomare JSON-Snapshots.

`fetch` holt ausschließlich `origin`-Branches nach `refs/remotes/origin/*`.
Es gibt kein Checkout, Merge, Reset, Pruning, Tag-Update oder Submodule-Fetch.
Lokale Branches und laufende Worktrees behalten ihren Stand. Insbesondere wird
der lokale Basisbranch durch Fetch **nicht** automatisch vorgerückt; seine
Integration bleibt ein expliziter Git-/Review-Schritt.

Git verwendet die vorhandene lokale SSH-/Credential-Helper-Konfiguration.
YoloForge transportiert keine Hosting-Zugangsdaten zwischen Rechnern. Git-Ausgaben
bei Fehlern werden nicht ungefiltert übernommen, da Credential-Helper Secrets
ausgeben könnten. Die Einrichtung läuft als ausdrücklicher lokaler CLI-Auftrag,
nicht als Remote-Runner-Command. In der Ratatui-Verwaltung ist sie noch nicht
als Dialog verfügbar.

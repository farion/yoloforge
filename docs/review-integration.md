# Review und Git-Integration

Ein Review erstellt aus einem bestätigten Checkpoint und einem ausdrücklich
angegebenen Zielbranch-Commit einen Merge-Kandidaten. Der Projekt-Testbefehl läuft
in einer eigenen Docker-Runtime. Erst ein erfolgreich getesteter Kandidat mit
bestätigter Runtime-Entfernung kann integriert werden.

## Einrichtung

- Server, Runner und Clients aktualisieren: DB **22**, Runner-Journal **8**,
  Runner-Capability `git-review-v1`.
- Im Projektprofil einen Testbefehl konfigurieren, beispielsweise `cargo test --offline`.
- Runner mit `--review-image IMAGE` starten. Das Image muss lokal installiert sein
  und Toolchain/Abhängigkeiten enthalten; es wird nicht automatisch heruntergeladen.
- Optional `--review-docker-socket /pfad/docker.sock`; Standard ist der lokale
  `/var/run/docker.sock`. Unterstützt wird Linux mit Docker/cgroup v2.
- Für Integration darf der Zielbranch in keinem Worktree ausgecheckt sein.
  Ein als Projekt zugeordnetes Bare-Repository ist dafür geeignet. Ein belegter
  Branch wird abgewiesen; bestehende Benutzer-Worktrees werden nicht verändert.

## CLI

Nach erfolgreicher Runtime-Beendigung und Checkpoint-Erstellung:

```sh
yoloforge stream review STREAM_ID --expected-revision REVISION \
  --checkpoint CHECKPOINT_COMMAND_ID --expected-target-commit TARGET_COMMIT

# Spezifikation ansehen; Fortschritt/Ergebnis stehen im Snapshot und Auftrag:
yoloforge review REVIEW_COMMAND_ID
yoloforge --snapshot

# Erst nach erfolgreichem Review:
yoloforge stream integrate STREAM_ID --expected-revision CURRENT_REVISION \
  --review REVIEW_COMMAND_ID
```

Beide Mutationen unterstützen `--request-id` für unverändertes Replay.
`TARGET_COMMIT` ist der vollständige aktuelle Commit des lokalen Projekt-
Basisbranches auf dem Runner, zum Beispiel aus `git rev-parse main`.

In der **TUI**: `4` → Stream → `Enter`, dann mit `Tab` die Aktionen
**Checkpoint erstellen**, **An anderen Runner übergeben**, **Checkpoint prüfen**
oder **Review integrieren** wählen. Die Quell-Command-ID ist sichtbar und an
die Dialogrevision gebunden. Bei Übergabe steht in der ersten Eingabezeile die
angezeigte Ziel-Runner-ID, darunter die Begründung. Beim Review wird der
Zielbranch-Commit eingegeben. `Ctrl-S` sendet, `Esc` schließt. Unklare Zustellung
friert den Request über Schließen/Öffnen hinweg ein; erneutes `Ctrl-S` wiederholt
denselben Request. Offline wird nicht gesendet.

## Ausführung und Schutz vor konkurrierenden Änderungen

1. Nur der jüngste erfolgreiche Checkpoint des Streams kann geprüft werden.
   Checkpoint, Zielcommit und Projekt-Testbefehl werden serverseitig eingefroren.
2. Der Runner vergleicht den aktuellen Zielbranch mit dem erwarteten Commit.
   `git merge-tree` erzeugt einen Kandidaten ohne Eingriff in vorhandene Worktrees.
   Konflikte werden als fehlgeschlagenes Review gemeldet; sie müssen in einem
   neuen Checkpoint aufgelöst werden.
3. Kandidat und Reviewplan werden vor den weiteren Operationen journalisiert.
   Ein separater detached Worktree liegt unter `WORKSPACE_ROOT/.reviews/COMMAND_ID`.
4. Tests laufen ohne Netzwerk als Nicht-Root mit read-only Root-Dateisystem,
   entfernten Capabilities, `no-new-privileges`, eigenen Namespaces und Ressourcen-
   grenzen. Nur der Review-Worktree ist schreibbar, die Git-Verwaltung read-only.
   Image-ID, Daemon-ID, Container-ID und Startzeit werden festgehalten.
5. Startabsichten werden vor Docker-Aufrufen gespeichert. Ein Neustart beobachtet
   die vorhandene Ausführung; er startet keinen zweiten Test. Nach fünf Minuten
   wird abgebrochen. Nach Exit/Timeout wird die Runtime entfernt und ihre Abwesenheit
   geprüft. Unklare Live-Runtime-Ergebnisse bleiben reserviert und werden nicht
   als erfolgreich oder sicher beendet ausgegeben.
6. Nur Exitcode 0, bestätigte Entfernung und ein unveränderter, sauberer Kandidat
   ergeben ein erfolgreiches Review. Lokale Testlogs liegen als private Dateien
   neben dem Review-Worktree; Factory speichert Pfad und Nachweise, nicht den Logtext.
7. Integration nimmt ausschließlich den jüngsten erfolgreichen Reviewauftrag an.
   Git aktualisiert den Zielbranch atomar mit dem erwarteten alten Commit
   (`update-ref` als Compare-and-Swap). Ein zwischenzeitlich geänderter oder
   ausgecheckter Zielbranch wird nicht überschrieben. Nach unklarer Bestätigung
   wird der bereits gesetzte Kandidatencommit wiedererkannt.

Integration erfolgt **lokal im zugeordneten Repository**. Push/PR/Merge-Request
zum Hosting-Anbieter ist ein eigener noch offener Ablauf. Tickets werden nicht
automatisch als erledigt markiert. Checkpoint-, Review- und Testartefakte bleiben
für die Nachvollziehbarkeit erhalten.

## Abnahme

```sh
cargo build --workspace --locked
python3 -u scripts/smoke-handoff.py --review
```

Der reale Prozess-/Docker-Test prüft Übergabe in beide Richtungen, einen genau
einmal ausgeführten Test über Factory-/Runner-Neustart, Integration des geprüften
Kandidaten, abgewiesene Integration bei Testfehlern und bei zwischenzeitlicher
Zielbranchänderung. Das Testimage ist `yoloforge-freezer-test:development`;
`--review-image` wählt ein anderes lokal verfügbares Image.

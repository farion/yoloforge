# Hosting-Push und Pull Requests

## TUI und automatische Publikation (0.1.3-dev)

In der Auftragsliste oeffnet `h` den Assistenten fuer den ausgewaehlten erfolgreichen
Review. Review-ID und Commit bleiben gepinnt; Journal-, Hosting-Konfigurations-
und Factory-Token-Dateipfad werden explizit eingegeben. `Ctrl-S` publiziert bzw.
gleicht den vorhandenen Beleg ab. SSH-Runner nutzen ihr gespeichertes Profil und
remote vorhandene Dateien. Zugangsdaten werden nicht als CLI-Argumente transportiert.

`--watch-publications PROJECT_UUID --hosting-config /path/hosting.json` startet
statt des Execution-Runners einen eigenen projektspezifischen Helfer mit Operator-
oder Admin-Token. Er publiziert erfolgreiche Reviews aktueller Ownership nur bei
unpausiertem Projekt, verwendet das bestehende Publikationsjournal und wartet nach
Fehlern 120 Sekunden. Das Hosting-Ziel bleibt fuer seine Prozesslaufzeit gepinnt.
Ein unklarer PR-POST wird durch Suche abgeglichen, nicht blind wiederholt.
Quarantaenisierte Review-Journale werden nicht publiziert.

Auswahl-/Pause-/Ownership-, SSH-Quoting- und Antwortverlusttests bestehen. Die
fruehere echte GitHub-Abnahme betrifft den zugrunde liegenden Publikationshelfer;
eine neue End-to-End-Abnahme des Watchers gegen GitHub/Forgejo steht noch aus.

Ein neuer expliziter Runner-Helfer veröffentlicht einen **erfolgreich geprüften
Review-Kandidaten** auf GitHub oder Forgejo. Er liest das Runner-Journal ohne
Runner-Prozesslock und bestätigt Review-Ergebnis, Ownership-Epoche, Commit und
entfernte Test-Runtime gegen die lokale/zentral gespeicherte Evidenz.

```sh
factory-runner --id RUNNER_UUID --name publication \
  --server https://factory.example --journal /path/runner.sqlite3 \
  --publish-review REVIEW_COMMAND_UUID --hosting-config /path/hosting.json
```

Der Helfer braucht einen Operator-/Admin-Factory-Zugang über die üblichen
Token-Umgebungsvariablen. Hosting-Zugang und Git-SSH/HTTPS-Credentials bleiben
auf dem Runner. Beispiel für `hosting.json`:

```json
{
  "provider": "github",
  "api_url": "https://api.github.com",
  "repository": "owner/repo",
  "git_remote": "git@github.com:owner/repo.git",
  "token_file": "/home/operator/.config/yoloforge/github.token"
}
```

Forgejo: `provider: "forgejo"`, `api_url: "https://forge.example/api/v1"`
und passend dazu `git_remote: "git@forge.example:owner/repo.git"`.
API und Git müssen denselben Host und Repositorypfad bezeichnen. Die Token-Datei
enthält nur den Hosting-Token; sie gehört außerhalb des Projektworktrees.
Redirects der Hosting-API werden nicht verfolgt, API-Fehler geben keine
Antwortbodies mit möglichen Zugangsdaten aus.

## Wiederanlauf

- Ein eigener Branch `yoloforge/review-REVIEW_UUID` wird mit Absent-Ref-Lease
  angelegt. Ein vorhandener identischer Commit gilt als bestätigter Replay;
  abweichende Remote-Commits blockieren die Veröffentlichung.
- Zielbranch und Workspace/Index werden nicht verändert; kein automatischer Merge.
- `hosting/REVIEW_UUID.json` neben dem Runner-Journal pinnt Ziel, Kandidat,
  PR-Schreibversuch, bestätigte URL und die zentrale Request-ID. Änderungen am
  Ziel eines begonnenen Vorgangs werden zurückgewiesen.
- Nach unklarer PR-Antwort wird nach Branch, Basis, Commit und Provenienzmarker
  gesucht. Ein weiterer POST erfolgt nicht. Ist kein eindeutiger PR auffindbar,
  bleibt der Vorgang unbestätigt und verlangt Prüfung beim Hosting-Dienst.
- Bestätigte URLs werden idempotent über `/v1/commands/{id}/hosting` zentral
  gespeichert (Schema 28) und im Auftragsverlauf angezeigt. Ein Ausfall zwischen
  Hosting-Erfolg und Factory-Bestätigung kann denselben Beleg erneut zustellen.

Tests: lokales Bare-Git für Push/Replay/konkurrierenden Writer; synthetische
GitHub-/Forgejo-API mit verlorener POST-Antwort; zentrale Erfolgsbindung,
Retarget-Schutz und Replay nach DB-Neustart. Nach ausdruecklicher Freigabe wurde
das private Repository `farion/yoloforge-acceptance` angelegt. Die echte GitHub-
Abnahme mit `python3 scripts/smoke-handoff.py --github-acceptance` besteht:
gepruefter Kandidat gepusht, PR erstellt, identischer Replay ohne zweiten PR,
zentraler Beleg und fremder Publikationsbranch nicht ueberschrieben.
Test-PR `https://github.com/farion/yoloforge-acceptance/pull/1` wurde anschliessend
geschlossen; synthetische Testbranches bleiben zur Nachvollziehbarkeit erhalten.
Echte Forgejo-Abnahme steht noch aus.
Der Helfer ist über CLI nutzbar; ein vollständiger TUI-Hosting-Assistent und
automatische Einplanung dieses Schritts bleiben offen.

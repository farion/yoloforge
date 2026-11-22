# Hosting-Push und Pull Requests

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
Retarget-Schutz und Replay nach DB-Neustart. Reale Hosting-Zugänge und ein
vom Betreiber bestimmtes Testrepository sind noch nicht abgenommen.
Der Helfer ist über CLI nutzbar; ein vollständiger TUI-Hosting-Assistent und
automatische Einplanung dieses Schritts bleiben offen.

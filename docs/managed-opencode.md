# Factory-verwaltetes OpenCode (Vorschau)

Die zentrale OpenCode-Instanz ist Teil jedes `factoryd`-Starts. Persönliche
Dienste, Einstellungen und Logins werden nicht übernommen. Serverdatenbank
vor einem Upgrade auf Schema 35 sichern.

## Betrieb

`factoryd` startet und beaufsichtigt ein eigenes **OpenCode 2.4.1**, bindet
es an Loopback und hält dessen Zustand in einem privaten Verzeichnis neben
der Factory-Datenbank (`factory.sqlite3.opencode/`). Im Container wird das
Binary beim Image-Build installiert. Beim direkten Start lädt `factoryd` es bei
Bedarf in dieses private Verzeichnis, prüft die fest hinterlegte SHA-512-Summe
und verwendet es bei weiteren Starts wieder. Ein absoluter Pfad in
`FACTORY_OPENCODE_BIN` hat Vorrang; persönliche Installationen aus dem `PATH`
werden nicht übernommen. Es gibt keinen Gateway-Schalter.

Der Server erzeugt dort auch `model.key`. Dieser Schlüssel gewährt ausschließlich
Zugriff auf den Modellkatalog und Modellaufrufe, nicht auf Zugangsdaten oder die
Verwaltungs-API. `dev-local.sh runner` verwendet die lokale Datei automatisch.
Für Remote-Runner die Datei geschützt bereitstellen. Freigegebene Runner
können alle zentral freigegebenen Zugänge nutzen.

Den Runner statt mit `--opencode-registration` mit
`--managed-opencode /pfad/runner-opencode.json` starten:

```json
{
  "port": 4098,
  "model_key_file": "/pfad/runner-modellschluessel",
  "config": "/pfad/runner-optionen.jsonc"
}
```

`config` ist optional. Ohne `binary` oder `FACTORY_OPENCODE_BIN` lädt der Runner
OpenCode beim ersten Start in seinen eigenen privaten Cache neben dem Journal.
Der Download ist auf Linux x86-64 und ARM64 verfügbar und braucht `tar` und
`gzip`. Bei jedem Start wird die Version geprüft; ein vorhandener Cache
benötigt keinen Download. Der Runner startet
einen eigenen Loopback-Prozess mit privaten HOME-/XDG-Verzeichnissen neben dem
Journal und ohne geerbte Provider-Umgebungsvariablen. Die Verbindung zur Factory
erfolgt für Modellaufrufe über HTTPS oder einen lokalen Tunnel. Der Katalog
wird regelmäßig aktualisiert. Ist der Gateway nicht erreichbar, gibt es keinen
lokalen Fallback. Der frühere implizite Start des persönlichen
OpenCode-Hintergrunddiensts entfällt auch für alte Runner; explizit
registrierte Alt-Instanzen bleiben kompatibel.

Das Runner-Image wird mit `docker build -f deploy/Dockerfile.runner -t yoloforge-runner:development .`
gebaut und bringt das geprüfte Binary bereits mit. Pro Container eine eigene
Runner-ID und ein eigenes persistentes `/data` für Journal, Worktrees und
OpenCode verwenden. Dem Runner außerdem seine Zugangsdaten und die Datei mit
dem Modellschlüssel bereitstellen. `--server` muss auf die vom Runner erreichbare
Factory-URL zeigen. Ein getrenntes Docker-Netz ersetzt kein HTTPS: Für zwei
Container ist eine erreichbare HTTPS-Adresse (z. B. per Reverse Proxy) oder
ein lokaler Tunnel nötig; `127.0.0.1` allein verweist auf den Runner-Container.

Implementer-, Orchestrator-, Supervisor- und Memorizer-Sessions laufen auf den
Runnern. Das zentrale OpenCode im Server verwaltet Zugangsdaten für die
Modell-Anbieter, führt aber keine Aufgaben aus.

Im verwalteten Modus sind Repo-Optionen erlaubt. Anbieter- oder Modellvorgaben
in den Repo-Konfigurationen, Agenten und Befehlen werden vor dem Prompt
abgewiesen. Repo-Plugins sind in der Vorschau gesperrt. Entdeckte
System-/Eltern-Konfiguration führt zu einer Fehlermeldung, nicht zum Einlesen.
Dies ist Konfigurationsisolation, keine Sandbox für nicht vertrauenswürdige
Projekte. Die native interaktive Runner-TUI ist im verwalteten Modus noch nicht
angebunden.

## Schnittstelle für die Oberfläche

Beim Verbinden mit einer Factory ohne Standardmodell zeigt der Desktop einen
Hinweis. **Zu den Modell-Anbietern** öffnet die passende Seite der
Factory-Einstellungen. Dort oben **OpenAI (ChatGPT)**
auswählen, **Hinzufügen** klicken und den Zugang benennen. Anschließend
die Anmeldung im automatisch geöffneten Browser mit dem angezeigten Code bestätigen. Die Factory
wartet im Anmeldedialog auf die Rückmeldung. Der Zugang erscheint
erst nach erfolgreicher Anmeldung und Modellabfrage in der Liste. Beim ersten
Zugang wählt die Factory automatisch ein Standardmodell aus dem Katalog, wenn
noch keines festgelegt ist. Der Hinweis führt zu **Default-Modelle**, wo sich
das Standardmodell und die optionalen Rollenmodelle ändern lassen. Der
Aufgaben-Dialog zeigt **Default** und das geerbte Implementer-Modell; die
Projekt-Rollen stehen unter **Projekteinstellungen → Modell-Anbieter → Agenten-Overrides**.
Einen abgelaufenen Zugang kann der Admin dort mit der gleichen
ChatGPT-Konto-ID erneut anmelden, ohne die Modellreferenzen umzuschreiben.

Für **Google Gemini**, **OpenRouter**, **Anthropic (Claude)** und **LiteLLM**
unter **Modell-Anbieter → Hinzufügen** einen Namen und den API-Schlüssel
eingeben. Bei **Ollama** kann der Schlüssel leer bleiben. Die Schlüssel
bleiben auf dem Server.

- **Google AI Pro oder Ultra:** In [Google AI Studio](https://aistudio.google.com/)
  das [monatliche API-Guthaben aktivieren](https://aistudio.google.com/docs/google-ai-plans#claim-api-credits),
  die Abrechnung für das Projekt einrichten und den
  [Gemini-API-Schlüssel](https://aistudio.google.com/apikey) in Factory eintragen.
  Das Browser-Kontingent des Abos gilt nicht für externe API-Aufrufe.
- **LiteLLM und Ollama:** Beim Hinzufügen die **Server-URL** eintragen, z. B.
  `http://litellm:4000` oder `http://localhost:11434`. Die Adresse muss vom
  Factory-Server aus erreichbar sein. Öffentliche Adressen benötigen HTTPS.
  Ohne Eingabe kann der Server wie bisher `FACTORY_LITELLM_URL` bzw.
  `FACTORY_OLLAMA_URL` als Vorgabe nutzen. Ollama zeigt nur Modelle mit
  Tool-Unterstützung.

**OpenCode (kostenlos):** Ohne Anmeldung oder API-Schlüssel unter **Modell-Anbieter**
hinzufügen. Factory übernimmt die verfügbaren kostenlosen Chat-Modelle mit
Tool-Unterstützung aus dem nativen OpenCode-Katalog. Verwaltete Runner verwenden
diesen Zugang direkt in ihrem eigenen OpenCode; sie brauchen dafür ausgehenden
Zugriff auf OpenCode. Kein automatischer Wechsel zu kostenpflichtigen Modellen.
Gratisangebote können sich ändern und manche behalten Eingaben zur
Modellverbesserung; siehe [OpenCode-Datenschutzhinweise](https://opencode.ai/docs/console/models#privacy).

**OpenCode Console:** Separater, kostenpflichtiger Zugang. In der
[OpenCode Console](https://opencode.ai/console) anmelden, Guthaben hinterlegen und
den API-Schlüssel in Factory eintragen. Der Schlüssel bleibt auf dem Server.
Factory listet die kostenpflichtigen Chat-Modelle des Console-Workspaces mit
Tool-Unterstützung auf; OpenCode Go ist ein separates Abo und nicht enthalten.

**GitHub Copilot:** Unter **Modell-Anbieter** direkt **GitHub Copilot** auswählen,
einen Zugang benennen und im Browser den angezeigten Gerätecode bestätigen.
Factory nutzt die GitHub.com-Anmeldung von OpenCode. Das GitHub-Konto braucht
Copilot-Zugriff; LiteLLM und ein API-Schlüssel sind nicht nötig. GitHub
Enterprise mit eigenem Anmeldehost ist noch nicht angebunden.

Für die zusätzlichen Anbieter müssen Factory-Server, Desktop und Runner aktualisiert sein.
Der Papierkorb entfernt einen Zugang nach Bestätigung samt gespeicherten
Zugangsdaten und zugehörigen Standard- und Rollen-Einstellungen. Solange ein
offener Intent auf diesen Zugang festgelegt ist, kann er nicht gelöscht werden.

Die Control-API verwendet die normale Factory-Anmeldung. Typen stehen in
`crates/factory-protocol/src/models.rs`.

1. `GET /v1/model-settings` liefert den Katalog und die Factory-Defaults.
2. Als Admin einen neuen Zugang mit
   `POST /v1/model-settings/accesses/{access}/openai/oauth/start` anstoßen.
   `access` ist `factory-openai-<UUID>`, der Body ist `{"name":"OpenAI Arbeit"}`.
   Antwort: `attemptID`, `url`, `instructions`, `mode`. Den Browser-/Device-
   Login anzeigen; diese Werte nicht persistieren oder protokollieren.
   Danach `POST .../openai/oauth/complete` mit `{"attempt_id":"…","code":null}`
   (bei `mode: "code"` den Code einsetzen). Die Factory verknüpft den Login
   dauerhaft mit der Zugangs-ID und lädt den Modellkatalog von OpenCode.
   Für ein zweites Konto eine neue Zugangs-ID anlegen; keine aktive
   Konto-Umschaltung pro Lauf.
3. `GET /v1/model-settings/accesses/{access}/status` zeigt `not_connected`,
   `ready` oder `unavailable`. `ready` bedeutet, dass ein nicht abgelaufener
   Login vorhanden ist, nicht, dass der Provider einen Modellaufruf bestätigt hat.
   Wenn die Factory den ersten Default setzt, liefert die Login-Antwort
   `automatic_default`. Für Änderungen `PUT /v1/model-settings` verwenden.
4. `GET/PUT /v1/projects/{id}/models` liest/ändert die vier optionalen
   Projekt-Rollenmodelle. `POST /v1/tasks/submit-with-model` nimmt
   `{"submission":{…},"implementer":null}` an; `null` ist **Default**.
   Eine konkrete Auswahl ist `{"access_id":"…","model_id":"…"}`.
   `GET /v1/tasks/{id}/models` zeigt die gespeicherte Auflösung.

Reihenfolge: Aufgabenauswahl (Implementer) → Projektrolle → Factory-Rolle →
Factory-Standardmodell. Die vier Ergebnisse werden beim Einreihen gespeichert.
Der Memorizer hat eine eigene Modellauswahl unter **Default-Modelle** und in den
erweiterten Projekteinstellungen. Ältere Aufgaben ohne Memorizer-Modell
übernehmen bei der Auswertung die dann gültige Projekt-/Factory-Auswahl.
Bei einer abweichenden Modellauswahl werden vorhandene Sessions nicht
wiederverwendet. Beim ersten Standardmodell werden bisher nicht zugewiesene
Legacy-Promptaufgaben einmalig daran gebunden, statt sie erneut einzureihen.

## Abnahme und Grenzen

Die Rust-Gateway-Tests verwenden simulierte OAuth-/Responses-Endpunkte.
Ein separater Test startet ein echtes isoliertes OpenCode 2.4.1 ohne
Provider-Logins. Die echte ChatGPT-Abnahme mit zwei Konten und echten
Modellaufrufen steht noch aus; siehe
[Spike](../spikes/opencode/managed_access/README.md).

Die automatische OAuth-Token-Erneuerung für ChatGPT fehlt; abgelaufene Zugänge
blockieren ohne Kontowechsel.
Die zentrale OpenCode-Datenbank bleibt der Credential-Speicher. API-Schlüssel
und OAuth-Tokens werden nicht an Runner übergeben. Der native Gratiszugang
braucht keinen Eintrag im Credential-Speicher.

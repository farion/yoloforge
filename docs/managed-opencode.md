# Factory-verwaltetes OpenCode (Vorschau)

Die zentrale OpenCode-Instanz ist Teil jedes `factoryd`-Starts. Persönliche
Dienste, Einstellungen und Logins werden nicht übernommen. Serverdatenbank
vor einem Upgrade auf Schema 35 sichern.

## Betrieb

`factoryd` startet und beaufsichtigt ein eigenes **OpenCode 2.4.1**, bindet
es an Loopback und hält dessen Zustand in einem privaten Verzeichnis neben
der Factory-Datenbank (`factory.sqlite3.opencode/`). Der Container liefert
das Binary mit; bei direktem Start muss es im `PATH` liegen oder über
`FACTORY_OPENCODE_BIN` angegeben werden. Es gibt keinen Gateway-Schalter.

Der Server erzeugt dort auch `model.key`. Dieser Schlüssel gewährt ausschließlich
Zugriff auf den Modellkatalog und Modellaufrufe, nicht auf Zugangsdaten oder die
Verwaltungs-API. `dev-local.sh runner` verwendet die lokale Datei automatisch.
Für Remote-Runner die Datei geschützt bereitstellen. Freigegebene Runner
können alle zentral freigegebenen Zugänge nutzen.

Den Runner statt mit `--opencode-registration` mit
`--managed-opencode /pfad/runner-opencode.json` starten:

```json
{
  "binary": "/opt/factory/opencode/2.4.1/opencode",
  "port": 4098,
  "model_key_file": "/pfad/runner-modellschluessel",
  "config": "/pfad/runner-optionen.jsonc"
}
```

`config` ist optional. Das angegebene Binary wird auf Version 2.4.1 geprüft;
die Bereitstellung des Binaries übernimmt das Deployment. Der Runner startet
einen eigenen Loopback-Prozess mit privaten HOME-/XDG-Verzeichnissen neben dem
Journal und ohne geerbte Provider-Umgebungsvariablen. Die Verbindung zur Factory
erfolgt für Modellaufrufe über HTTPS oder einen lokalen Tunnel. Der Katalog
wird regelmäßig aktualisiert. Ist der Gateway nicht erreichbar, gibt es keinen
lokalen Fallback. Der frühere implizite Start des persönlichen
OpenCode-Hintergrunddiensts entfällt auch für alte Runner; explizit
registrierte Alt-Instanzen bleiben kompatibel.

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
Projekt-Rollen stehen in **Erweiterte Projekteinstellungen → Modell-Anbieter**.
Einen abgelaufenen Zugang kann der Admin dort mit der gleichen
ChatGPT-Konto-ID erneut anmelden, ohne die Modellreferenzen umzuschreiben.

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

Der Gateway unterstützt in dieser Vorschau **OpenAI per ChatGPT-Login**.
Weitere Anbieter und automatische OAuth-Token-Erneuerung fehlen. Abgelaufene Zugänge blockieren
ohne Kontowechsel. Die zentrale OpenCode-Datenbank bleibt der Credential-Speicher;
es werden weder Access- noch Refresh-Tokens an Runner übergeben.

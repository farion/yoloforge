# OpenCode-V2-Integrations-Spike

Ausführbarer Rust-HTTP-Client für den tatsächlich geprüften Vertrag **2.0.27**.
Das ist kein produktiver Runner. Der Client startet/stoppt keinen OpenCode-Service
und verwaltet keine Provider-Credentials.

## Ausführen

Voraussetzung: laufender OpenCode-Service 2.0.27 und Rust/Cargo.
Die Registrierung ist eine lokale private Datei, **keine Projektkonfiguration**.
Standardpfad laut OpenCode-Dokumentation: `~/.local/state/opencode/service.json`.
Bei abweichendem XDG-Verzeichnis den tatsächlichen Pfad verwenden.

```sh
opencode service status
cargo run -p factory-opencode-spike -- inspect "$HOME/.local/state/opencode/service.json"
cargo run -p factory-opencode-spike -- smoke "$HOME/.local/state/opencode/service.json" "$PWD/spikes/opencode/workspace"
```

`inspect` prüft API-Version, Authentifizierung und angebotene OpenAI-Anmeldemethoden.
`smoke` erzeugt eine eigene Testsession und löscht nur diese wieder, auch bei einem
Prüffehler. Bei einem harten Prozessabbruch kann sie zurückbleiben; ihre ID wird
ausgegeben. Das Testverzeichnis bleibt erhalten und ist in `.gitignore` ausgeschlossen.

Der Smoke-Test führt **keinen LLM-Aufruf** aus. Er reiht einen Prompt mit
`resume:false` ein, prüft Formulare und Permissions über die API und führt am Ende
ein begrenztes `sleep 3` über die Session-Shell aus. Keine Nutzer-Session wird
unterbrochen und kein echter Toolzugriff durch die synthetische Permission freigegeben.
Projekt-/globale OpenCode-Plugins können auf Testsession-Ereignisse reagieren.

Der Client liest URL und Passwort direkt aus der explizit angegebenen Registrierung.
Er verwendet HTTP Basic mit Benutzer `opencode`, ausschließlich über Loopback.
Proxies und Redirects sind deaktiviert. Response-Bodies und Secrets werden nicht
protokolliert. HTTP-Basic ist der empirisch verifizierte **lokale OpenCode-Zugang**,
nicht das entworfene Factory-Geräteprotokoll.

Für einen entfernten Service ist zuerst ein passender SSH-Tunnel erforderlich;
dieser Spike richtet ihn noch nicht ein. Automatische Service-Discovery und andere
OpenCode-Authentifizierungsverfahren gehören in den späteren Adapter.

## Reproduzierbarkeit

- `Cargo.lock` pinnt Rust-Abhängigkeiten.
- `openapi-2.0.27.json` wurde vom laufenden Service über `opencode api get /openapi.json`
  exportiert, nicht aus einer möglicherweise neueren Online-Spezifikation.
- SHA256: `a5021c9056cfed7dd45b963c99a4fbace2198e54982f301b78bf9457be0fa413`.
- Die Spezifikation bezeichnet sich intern als API-Version `0.0.1`; die tatsächliche
  Produktversion wird separat mit `/api/info` geprüft.

Ein [isoliertes Docker-Labor](container/README.md) ergänzt den Rust-Spike um
Container-Neustarts, SIGKILL, parallele Wiederholungen und Pending-Input-Persistenz.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## Beobachtete Ergebnisse

| Prüfung | Ergebnis |
| --- | --- |
| Direkter Rust-HTTP-Zugriff mit Service-Passwort | erfolgreich |
| Unauthentifiziertes `/api/info` | HTTP 401 |
| Service-Passwort als Bearer | HTTP 401; kein Ersatz für Basic |
| OpenAI-Methoden lesen | Key, Env, ChatGPT-Anmeldung, Legacy-Browser, Legacy-Device-Code |
| SSE verbinden | `server.connected` empfangen |
| Session erstellen | `session.created` empfangen |
| Prompt mit `resume:false`, identische ID erneut senden | gleiche ID, genau ein Inbox-Eintrag |
| Formular erzeugen, abfragen und beantworten | erfolgreich |
| Formular mit anderer Antwort erneut abschließen | HTTP 409 |
| Synthetische Permission erzeugen, abfragen, ablehnen | erfolgreich |
| Idle-Session unterbrechen | `interrupted:false` |
| SSE trennen, Formular erzeugen, neu verbinden | Formular durch Snapshot-Abfrage wiedergefunden |
| Session und Nachrichten nach Wiederverbindung abfragen | erfolgreich |
| `session.shell` mit `sleep 3`, dann Session-Interrupt | `interrupted:false`; Shell-Aufruf beendet sich |
| Testsession löschen | erfolgreich, HTTP-No-Content unterstützt |

## Konsequenzen

1. Mutationsergebnisse können HTTP 204 ohne JSON sein. Der Adapter muss das abbilden.
2. OpenAPI hat leere `security`-Einträge, obwohl der Dienst Authentifizierung erzwingt.
   Ein generierter Client allein löst die Service-Anmeldung nicht.
3. Explizite Nachrichten-IDs helfen bei Prompt-Deduplizierung. Das Docker-Labor
   prüft auch gleichzeitige Wiederholungen und Neustarts. Gleiche ID mit anderer
   Payload liefert HTTP 200 mit dem ursprünglichen Eintrag: Payload-Konflikte muss
   die Factory selbst erkennen. Crash-Grenzen während einer Mutation bleiben offen.
4. Reconnect erfordert das Nachladen von Zustand und offenen Eingaben. Kein Replay
   des OpenCode-Eventstreams voraussetzen.
5. Session-Interrupt ist kein genereller Prozessbaum-Kill. Insbesondere darf ein
   `false` bei laufender Session-Shell nicht als bestätigte Workspace-Stilllegung gelten.
6. Der begrenzte SSE-Reader ist Spike-Code, keine fertige Stream-Bibliothek:
   produktiv sind unter anderem Event-Dispatch, Backpressure und dauerhafte
   Ereignisnormalisierung separat umzusetzen.

## Noch offen: F-001 nicht vollständig abgeschlossen

- Echte Modellantworten mit `openai/gpt-6-astra` und die vom Agenten ausgelöste
  Rückfrage mit Fortsetzung sind im Container geprüft (`verify-login`, `verify-question`).
  Wiederaufnahme eines wartenden Toolaufrufs über einen Prozessneustart bleibt offen.
- Interrupt eines echten Agenten-Tools mit kooperativem Vordergrund-/Kindprozess
  und anschließendem expliziten Fortsetzen ist geprüft (`verify-interrupt`).
  Abgekoppelte/signalresistente Prozesse und separate Shell-/PTY-Ausführung bleiben offen.
- OAuth-Refresh nach Tokenablauf. Headless-Login mit Nutzerbestätigung und
  Credential-Persistenz im Server-Container sind inzwischen erfolgreich geprüft.
- Crash-Grenzen während aktiver Ausführung. Neustarts im ruhenden Zustand sind
  inzwischen im Docker-Labor geprüft; offene Forms/Permissions gingen dabei verloren.

Der gemeinsam genutzte Service wurde nicht neu gestartet. Die Lifecycle-Tests
verwenden ausschließlich ein isoliertes, wegwerfbares Docker-Deployment.

## Dokumentationsquellen

- https://opencode.ai/v2/docs/api
- https://opencode.ai/v2/docs/build/client
- https://opencode.ai/v2/docs/cli/providers
- https://opencode.ai/v2/docs/troubleshooting

Laut V2-Dokumentation liegen Provider-Credentials im SQLite-Speicher des OpenCode-
Servers. Dieser Speicher wird nicht direkt bearbeitet oder ins Repository kopiert.

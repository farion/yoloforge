# Isoliertes OpenCode-Labor (F-002)

Docker-Umgebung auf Ubuntu 24.04 für OpenCode **2.0.27**, ohne Mounts des Host-
OpenCode-Verzeichnisses. Das ist ein Integrationslabor, noch kein Factory-Server-
Release. Die Docker-Steuerung übernimmt ein Python-Testharness. Die Rust-Factory
läuft inzwischen ebenfalls im Labor für echte schreibende End-to-End-Tests.

## Voraussetzungen

- Docker-Daemon, Python >= 3.11, Cargo und lokales OpenCode-Binary 2.0.27.
- Aktuell auf Linux x86-64 geprüft; das lokale Binary muss zur Containerplattform passen.
- Internet beim Image-Build für Ubuntu-Pakete und beim ersten Start für den Providerkatalog.

## Automatische Tests

Aus dem Repository:

```sh
python3 spikes/opencode/container/lab.py build
python3 -u spikes/opencode/container/lab.py test
```

Der Build kopiert ausschließlich das ausgewählte OpenCode-Binary und das Dockerfile
in einen temporären Build-Kontext. Kein Repository und keine Host-Credentials
werden ins Image kopiert. `--binary /pfad/zu/opencode` überschreibt das Binary.
Der Basisimage-Digest ist gepinnt; apt-Pakete sind noch nicht als Snapshot fixiert.

`test` erstellt einen eigenen Container und ein eigenes Volume. Der API-Port wird
nur auf `127.0.0.1` veröffentlicht. Ein zufälliges Service-Passwort wird in Dateien
mit Modus 0600 unter `.runtime/` abgelegt, nicht im Git oder in CLI-Argumenten.
Im Container läuft OpenCode als Benutzer `factory`, nicht als root.

Nach dem Test entfernt der Harness ausschließlich seine per Label geprüften
Ressourcen und die lokalen Lab-Credentials. Das gebaute Image bleibt erhalten.
Ein vorhandenes manuelles Login-Labor wird von `test` abgelehnt und nicht gelöscht.

## Verifizierte Ergebnisse

| Szenario | Beobachtung mit 2.0.27 |
| --- | --- |
| Rust-API-Zugriff zum Container | Basic-Auth funktioniert; ohne Auth HTTP 401 |
| Zwei gleichzeitige Wiederholungen einer Nachrichten-ID | gleicher Inbox-Eintrag |
| Gleiche ID, anderer Text | HTTP 200; ursprünglicher Inhalt bleibt erhalten |
| Kontrollierter Container-Neustart | Session, wartender Prompt und Workspace-Datei erhalten |
| SIGKILL und erneuter Start | Session, wartender Prompt und Workspace-Datei erhalten |
| Erneute Zustellung derselben Nachrichten-ID nach Neustart | kein weiterer Inbox-Eintrag |
| Offenes synthetisches Formular nach Neustart | nicht wieder in Pending-Liste vorhanden |
| Offene synthetische Permission nach Neustart | nicht wieder in Pending-Liste vorhanden |
| Formular mit vorher gespeicherter ID und Schema neu anlegen | möglich; Antwort anschließend möglich |
| Headless-Login durch den Nutzer | erfolgreich nach xdg-open-Anpassung |
| Echter Modellaufruf mit `openai/gpt-6-astra` | erfolgreich |
| Kontrollierter Neustart nach Anmeldung | Credential-ID erhalten; erneuter Modellaufruf erfolgreich |
| Agent fordert interaktiv einen unbekannten Prüfcode an | echtes Formular empfangen |
| Antwort über Formular-API | Agent setzt fort und gibt den erst danach erzeugten Prüfcode korrekt zurück |
| Interrupt während echtem Agenten-Shelltool mit Kindprozess | `interrupted:true`; beobachtete Prozesse beendet; Schreibdatei stabil |
| Explizite neue Nachricht nach Interrupt | dieselbe Session setzt fort, ohne Worker-Neustart |
| Zwei Factory-Runner mit echten Agenten | parallele isolierte Worktrees, Dateiänderungen und Python-Validierung |
| Factory-/Runner-Neustart während laufender Agenten-Shelltools | bekannte Sessions übernommen, je ein Prompt und eine Toolausführung |

Vor jeder Neustartvariante werden die Testfragen tatsächlich erneut angelegt.
Die Tests töten einen zuvor bestätigten, ruhenden Zustand. Ein Crash mitten im
Commit oder während einer Modell-/Toolausführung ist damit noch nicht geprüft.
Der zusätzliche Factory-Test startet **Factory-Prozesse** während aktiver Tools
neu; er beweist weiterhin keinen OpenCode-Service-Crash während dieser Tools.

## Schreibender Factory-Test

Mit bereits angemeldetem, behaltenem Labor:

```sh
cargo build --workspace --locked
python3 -u scripts/smoke-agent.py
```

Verwendet zwei echte `openai/gpt-6-astra`-Aufrufe. Der Test kopiert Factory-Binaries
in ein temporäres Containerverzeichnis, startet zwei Runner und einen Factoryd,
legt eigene Git-Repositories/Worktrees an und startet pro Stream eine Agentensession.
Die testbezogenen Sessions dürfen das Fragentool verwenden; das Shelltool benötigt
eine explizite Permission. Beide Arten von Fragen werden über Factory beantwortet,
Formularantworten dabei erst nach einem Neustart aus der persistenten Queue zugestellt. Während
die beiden Tools laufen, werden Factoryd und ein Runner neu gestartet. Verifiziert
werden erwartete Dateien, Python-Kompilation und genau eine Ausführung pro Auftrag.
Nur eigene Prozesse, Sessions und Testverzeichnisse werden anschließend entfernt;
das Konto, der OpenCode-Service und dessen Volume bleiben erhalten.

Das aktuelle Dockerfile enthält Python 3. Im bereits angemeldeten älteren Labor
wurde Python nachinstalliert, ohne den Container oder dessen Anmeldung neu anzulegen.

### Weitere Befunde während der Entwicklung

- `/api/info` kann bereit sein, bevor `/api/integration/openai` verfügbar ist.
  Das Labor wartet zusätzlich auf den Providerkatalog.
- Docker kann bei Neustart einen dynamisch veröffentlichten Port ändern. Der
  Harness liest den Port neu; ein produktives Deployment erhält einen festen Port.
- Ein `session.shell`-Aufruf ließ zuvor wartenden Input aus der Inbox verschwinden,
  auch mit `resume:false` und `delivery:queue`. `resume:false` ist keine dauerhafte
  Pausegarantie. Der Persistenztest schreibt seinen Checkpoint deshalb außerhalb
  OpenCodes per `docker exec`. Die genaue Shell-/Scheduler-Semantik bleibt zu klären.

## Konsequenzen für die Factory

1. Nachrichten-ID plus Payload-Hash im eigenen Command-Journal speichern. OpenCode
   meldet bei anderer Payload unter gleicher ID nicht den erwarteten Konflikt.
2. Fragen vollständig und dauerhaft speichern: ID, Schema, Quelle, Run, Antwortstatus.
3. Nach Neustart fehlende Fragen nicht als beantwortet interpretieren. Als
   `reconciliation_required` kennzeichnen und mit dem Ausführungszustand abgleichen.
4. Neu erzeugte Formulare rekonstruieren nicht automatisch die wartende Tool-
   Continuation eines alten Prozesses. Antworten nur einem gültigen aktuellen Lauf
   zustellen; bei Bedarf Session kontrolliert mit Übergabekontext fortsetzen.
5. Permissions ebenfalls nicht blind neu freigeben: Anfrage und konkrete aktuelle
   Toolausführung müssen wieder zusammenpassen.

## Manuelle OpenAI-Anmeldung im isolierten Labor

Die folgenden Befehle führt der Nutzer in seinem interaktiven Terminal aus:

```sh
python3 spikes/opencode/container/lab.py up
python3 spikes/opencode/container/lab.py login
```

`login` ruft im Container gezielt `opencode auth login openai` mit dem vom Server
angebotenen Verfahren `chatgpt-headless` auf. Browser-/Device-Code-Ausgabe geht
direkt ins Terminal. Der Nutzer bestätigt die Anmeldung. Tokens werden durch
OpenCode in dessen Datenbank im privaten Lab-Volume gespeichert. Das Verfahren
ist als anbieterseitige Legacy-Option dokumentiert; sein erfolgreicher Abschluss
wurde inzwischen durch den Nutzer und anschließende Modellaufrufe bestätigt.
Es werden keine Host-Anmeldungen kopiert.

OpenCode 2.0.27 versucht auch bei diesem Device-Code-Verfahren `xdg-open`
auszuführen. Der headless Container hat keinen Browser. `login` installiert
deshalb einen kleinen `xdg-open`-Adapter, der auf das manuelle Öffnen der bereits
angezeigten URL hinweist. Nur die Login-CLI erhält diesen Adapter im PATH;
Agentenwerkzeuge und der OpenCode-Service bleiben unverändert. Die CLI kann damit
auf die Bestätigung warten, statt am fehlenden Browserprogramm abzubrechen.

Nach einem Fehler `Executable not found ... xdg-open` genügt es, **`login` erneut
auszuführen** und den dann angezeigten neuen Code zu verwenden. Ein Rebuild,
`down` oder erneutes `up` ist nicht erforderlich; das bestehende Volume bleibt erhalten.

### Modellzugriff und Login-Persistenz prüfen

```sh
python3 -u spikes/opencode/container/lab.py verify-login
```

Dieser Befehl verwendet die Anmeldung des bestehenden Labors für **zwei kleine
echte Modellaufrufe** mit `openai/gpt-6-astra`: einmal vor und einmal nach einem
kontrollierten Container-Neustart. Jeder Aufruf muss einen frisch erzeugten Marker
korrekt beantworten. Eigene Testsessions werden entfernt; Konto und Volume bleiben
erhalten. Bei aktiven Sessions wird der Neustarttest abgelehnt. Das Labor während
dieser Prüfung exklusiv verwenden, damit zwischen Prüfung und Neustart kein neuer
Lauf gestartet wird.

`--model <id>` wählt ein anderes vom Server angebotenes OpenAI-Modell. Der Katalog
beweist jedoch keine Subscription-Berechtigung: `gpt-5.5` war gelistet, wurde beim
Aufruf aber mit `provider.invalid-request` / „not supported“ abgewiesen. Mit
`gpt-6-astra` waren beide Aufrufe erfolgreich. Provider-Fehler werden nur als Typ,
Status und grobe Kategorie ausgegeben, nicht mit Response-Body oder Zugangsdaten.

Der Test bestätigt die Nutzbarkeit der gespeicherten Anmeldung nach Neustart,
**nicht** die Erneuerung eines abgelaufenen OAuth-Tokens.

### Echte Agenten-Rückfrage prüfen

```sh
python3 -u spikes/opencode/container/lab.py verify-question
```

Dieser Live-Test verwendet eine eigene Session mit `openai/gpt-6-astra` und dem
`build`-Agenten. Alle Tool-Aktionen sind gesperrt, außer `question`, das ausdrücklich
erlaubt ist. Eine pauschale Sperre blockiert auch das Fragen-Werkzeug; dann kann
der Agent kein interaktives Formular erzeugen.

Der Agent wird angewiesen, einen Prüfcode über das Fragewerkzeug anzufordern.
Erst nachdem das echte Formular über die API empfangen wurde, erzeugt der Harness
einen zufälligen Code und beantwortet das Formular. Erfolgreich ist der Test nur,
wenn der Agent anschließend exakt diesen zuvor unbekannten Code bestätigt,
sein Lauf beendet ist und keine offene Frage übrig bleibt. Beobachteter Feldtyp:
`string`. Damit ist nicht nur die synthetische Formular-API, sondern auch die
Fortsetzung eines tatsächlich wartenden Agentenlaufs geprüft.

Der Test nutzt die Subscription, startet den Container nicht neu und entfernt
ausschließlich seine eigene Testsession. Er belegt keine Wiederaufnahme eines
wartenden Agentenlaufs über einen Prozessneustart hinweg.

### Aktiven Agenten unterbrechen und fortsetzen

```sh
python3 -u spikes/opencode/container/lab.py verify-interrupt
```

Der Harness legt in einem frischen Testverzeichnis `interrupt-worker.sh` ab.
Eine eigene `build`-Session darf ausschließlich das Shelltool nutzen und startet
das Skript im Vordergrund. Ein Kindprozess schreibt alle 250 ms eine Zeile in
eine Heartbeat-Datei. Der Worker ist auf ungefähr 120 Sekunden begrenzt und
räumt bei normalen Terminationssignalen kooperativ auf.

Die Prüfung wartet auf einen laufenden Toolstatus sowie lebenden Eltern- und
Kindprozess. Ihre `/proc`-Startzeiten werden gespeichert, damit wiederverwendete
PIDs nicht als alte Prozesse gelten. Erst nach nachgewiesenem Dateiwachstum wird
`session.interrupt?resume=false` aufgerufen. Die Abnahme verlangt:

- API bestätigt `interrupted:true`;
- die beobachteten Prozessidentitäten sind nicht mehr ausführbar (Zombie zählt
  als beendet, weil er nicht schreiben kann);
- Session ist nicht mehr aktiv und hat Outcome `interrupted`;
- Heartbeat-Datei bleibt weitere zwei Sekunden unverändert;
- eine neue Nachricht führt in derselben Session zu einer korrekten Modellantwort,
  ohne dass die Heartbeat-Datei wieder wächst.

Beobachtet: Erfolg in 2,4 Sekunden inklusive Zwei-Sekunden-Nachprüfung. Testsession,
Testdateien und gegebenenfalls identifizierte Restprozesse werden aufgeräumt.
Anmeldung und Container bleiben erhalten. Der Test verwendet echte Modellaufrufe.

**Geltungsbereich:** kooperativer Vordergrund-Worker innerhalb eines echten
Agenten-Toolaufrufs. Das beweist keinen generellen Stop aller abgekoppelten Daemons,
signalresistenten Prozesse oder separat per `session.shell` gestarteten Kommandos.
Der frühere Befund `session.shell` → `interrupted:false` bleibt gültig.

Der Container und das Volume bleiben bis zum expliziten Aufräumen erhalten:

```sh
python3 spikes/opencode/container/lab.py down
```

**`down` löscht auch das Lab-Volume und eine darin vorgenommene Anmeldung.** Dies
betrifft nur dieses Labor, nicht deinen normalen OpenCode-Service. `.runtime/`
enthält Secrets; nicht veröffentlichen oder in Diagnoseberichte kopieren.

## Noch offen

- Abgekoppelte bzw. signalresistente Prozesse und separate Shell-/PTY-Ausführungen.
- OAuth-Refresh nach Tokenablauf; Credential-Persistenz nach Neustart ist geprüft.
- SSH-Tunnel gegen einen tatsächlichen Remote-Host.
- Crash mitten in aktiver Ausführung und Wiederanlauf einer wartenden Tool-Continuation.
- Factory-Dienst/Runner-Container und endgültiger Compose-/Ansible-Vertrag.

# Skills und MCPs: Ausführungsprofile

## Ergebnis des technischen Spikes

Ein privater OpenCode-2.4.1-Prozess bedient drei parallele Git-Worktrees desselben
Repos. Zwei haben denselben Skill-Namen mit unterschiedlichem Inhalt und jeweils
einen anderen lokalen MCP; der dritte hat keine Integrationen. OpenCode meldet
für jeden Worktree nur dessen eigene Integrationen. Eine globale Sperre greift
auch bei einem lokalen `allow`: Der gesperrte Skill lässt sich nicht aktivieren,
der gesperrte MCP-Prozess startet nicht.

Ein Arbeitsverzeichnis ist dagegen **kein unveränderlicher Session-Snapshot**.
Nach einer Profiländerung und einem Reload liest eine bereits angelegte Session
im selben Verzeichnis die neue Skill-Revision. Der andere Worktree sieht
weiterhin seine eigene Revision. Ob der Reload intern weitere Locations neu
lädt, wurde nicht geprüft.

**Entscheidung:** Ein OpenCode-Dienst pro Runner reicht zunächst für getrennte
Worktrees. YoloForge bindet eine Session an ihr wirksames Integrationsprofil.
Ein abweichendes Profil bekommt eine neue Session mit eigenem Worktree; die
laufende Konfiguration wird nicht an Ort und Stelle umgeschrieben. Diese
Trennung betrifft die Tool- und Skill-Auswahl und ist **keine Sandbox** für
Aufgaben oder lokale MCP-Prozesse.

## Vereinbarte Bedienung

**Umgesetzt:** Unter **Projekteinstellungen → Skills** können Admins
importierte Factory-Skills in einer festen Version und erkannte Repository-Skills
per Häkchen freigeben. Die GUI listet `.opencode/skills/<id>/SKILL.md` aus dem
Basisbranch lokal erreichbarer Git-Repositorys auf.
Das erste Speichern aktiviert die Liste; eine leere Liste gibt nichts frei.
Bereits eingereihte Aufträge behalten ihre Auswahl. Eine inkompatible
Session-Zuordnung wird beim Einreihen abgewiesen. Der native verwaltete Runner
prüft die Skill-Dateien inklusive Begleitdateien gegen den vorbereiteten
Git-Commit und den von OpenCode gemeldeten Pfad. Änderungen und unversionierte
Dateien im Skill-Ordner verhindern den Prompt-Start. Neue Runner melden dafür
`repo-skills-v1`. Bestehende Projekte ohne gespeicherte Liste behalten ihr
bisheriges Verhalten.

Die Freigabe beschränkt OpenCodes reguläres `skill`-Tool; sie ist keine
Dateisystem-Sandbox und gilt nicht als Autorisierung für den experimentellen
direkten OpenCode-Skill-Endpunkt. Der Runner prüft vor dem Prompt, beobachtet
aber keine Änderungen am laufenden Worktree nach dem Start.

**Umgesetzt: Lokale MCPs.** Unter **Factory-Einstellungen → MCPs →
MCP hinzufügen** **Lokal** wählen, absoluten Programmpfad und einzelne Argumente eingeben
und die Runner auswählen, auf denen der Adapter installiert ist. Nur
vertrauenswürdige Programme eintragen und keine Secrets in Argumenten ablegen.
Der Server erlaubt Änderungen am Katalog und an Projektfreigaben nur
Factory-Admins. Der Runner muss auch für die Projektausführung verfügbar sein.

Unter **Projekteinstellungen → MCPs** gibt ein Häkchen am MCP den Server
zur Auswahl im Aufgabendialog frei. Bereits als erforderlich gespeicherte MCPs
werden bis zum Entfernen ihrer Freigabe automatisch verwendet. Die Freigabe gilt für alle
angebotenen Tools, einschließlich schreibender Aktionen, ohne Einzelabfrage.
Die Auswahl wird beim Einreihen vom Server aufgelöst und festgehalten. Eine
explizit ausgewählte inkompatible Session wird abgewiesen; die automatische
Zuordnung verwendet nur passende Sessions bzw. legt einen neuen Worktree an.

Der Scheduler weist einen solchen Auftrag nur einem kürzlich erreichbaren
Runner zu, dem alle angeforderten lokalen MCPs zugeordnet sind. Der Runner
richtet lokale Server vor dem Prompt über die Location-API als stdio-MCPs ein;
es wird keine OpenCode-Konfiguration ins Repo geschrieben. Runtime-Einträge
gehen bei einem OpenCode-Reload verloren und werden vor dem nächsten Prompt
wiederhergestellt. Laufende Prompts werden dabei nicht erneut gesendet.
Fehlt ein Adapter oder verbindet er sich nicht, wird der Prompt nicht
abgeschickt. Die Prüfung beschränkt sich auf MCP-Verbindung und Tool-Katalog;
sie kann nicht generisch feststellen, ob etwa eine offene Blender-Szene
erreichbar ist.

Bei verwalteten Integrationen weist der Runner zusätzliche Skill-Quellen und
MCP-Definitionen in Repo-Konfigurationsdateien vor dem Implementer-Prompt ab.
Bestehende native OpenCode-Konfigurationen ohne verwaltete Auswahl bleiben
unverändert. Blender ist ein normaler MCP-Anwendungsfall und braucht keine
spezielle YoloForge-Unterstützung.

**Umgesetzt: Remote-MCPs ohne Authentifizierung oder mit API-Key.** Unter
**Factory-Einstellungen → MCPs → MCP hinzufügen → Remote** den HTTPS-Endpunkt
eintragen und **Keine**, **Bearer-Token** oder **API-Key-Header** wählen. Ein
bereits gespeicherter Schlüssel wird nicht wieder angezeigt. Zum Ersetzen
einen neuen Wert eingeben; er geht nicht in die lokale Wiederholungswarteschlange.
**Gespeicherte Verbindung testen** initialisiert den MCP und liest die Tool-Liste,
ohne ein Tool auszuführen. Die Projektfreigabe funktioniert wie bei lokalen MCPs;
Remote-MCPs werden keinem einzelnen Runner zugeordnet. Separate Einträge für
unterschiedliche Konten bekommen jeweils eigene IDs und Freigaben.

Der Streamable-HTTP-Proxy unter `/mcp-runtime/` verbindet sich von `factoryd`
aus. Er folgt keinen Redirects, verwendet keine Umgebungsproxies und leitet
keine beliebigen HTTP-Header durch. Die Zugangsdaten werden per AES-256-GCM
verschlüsselt. Die private Schlüsseldatei `<Datenbankdatei>.mcp-key` liegt
neben der Datenbank und muss bei Backups **getrennt
geschützt und mitgesichert** werden. Reverse Proxies müssen `/mcp-runtime/`
einschließlich SSE durchreichen. Runner erhalten ausschließlich einen
auf Runner-Registrierung, Auftrag und MCP begrenzten Proxy-Zugang; der Proxy
prüft bei jedem neuen Request die Projektfreigabe und die Konfigurationsrevision.
Ein neuer Schlüssel oder Endpunkt erzeugt eine neue Revision. Bereits gestartete
Requests werden dadurch nicht rückwirkend unterbrochen.

**OAuth-Erkennung und öffentlicher PKCE-Client.** Unter **Authentifizierung
prüfen** testet die Factory den MCP ohne Zugangsdaten und liest bei Bedarf
die standardisierten Ressourcen- und Anbieter-Metadaten. Ein `401` oder
`Bearer` allein reicht nicht für eine OAuth-Erkennung. Bei Bedarf erscheint
die manuelle Auswahl im Dialog.

Ohne `FACTORY_MCP_PUBLIC_URL` verwendet die Desktop-GUI den lokalen Redirect
`http://127.0.0.1:52818/mcp-oauth/callback`. Browser und Desktop-GUI müssen
dazu auf demselben Rechner laufen. Die GUI leitet den kurzlebigen Code über
die authentifizierte Factory-Verbindung weiter; der Token-Austausch bleibt
auf der Factory. Der Anbieter muss den Loopback-Redirect für einen
öffentlichen PKCE-Client akzeptieren. Wenn dynamische Registrierung nicht
möglich ist, benötigt der Anbieter eine vorregistrierte Client-ID. Die
Desktop-GUI zeigt in diesem Fall nur die fehlgeschlagene Prüfung an.

Optional `FACTORY_MCP_PUBLIC_URL` auf den öffentlichen HTTPS-Origin der Factory
setzen (z. B. `https://factory.example`). Ein Reverse Proxy muss dann
`/mcp-oauth/callback` und `/mcp-oauth/client.json` an `factoryd` durchreichen.
Für den Callback keine Query-Parameter protokollieren. Unterstützt der
Anbieter ein Client-ID-Metadatendokument, verwendet die Factory automatisch
`/mcp-oauth/client.json` als Client-ID. Andernfalls versucht die Factory die
dynamische Registrierung, wenn der Anbieter einen öffentlichen Endpunkt
dafür angibt und einen Client ohne Secret zulässt. Hostübergreifende
Metadaten- und Token-Endpunkte müssen öffentlich erreichbar sein.

Sind bei der OAuth-Prüfung alle Angaben vorhanden, startet die
Browser-Anmeldung. Erst nach erfolgreicher Anmeldung und MCP-Prüfung wird
**Speichern** aktiv. Die Zugangsdaten sind für das Speichern zehn Minuten
lang verschlüsselt im Arbeitsspeicher verfügbar.
Factory prüft den einmaligen OAuth-State,
tauscht den Code per PKCE aus und speichert Access- und Refresh-Token
verschlüsselt. Bei Ablauf wird ein vorhandener Refresh-Token benutzt, ohne
die Auftragskonfiguration zu ändern. Der Verbindungstest prüft danach die
MCP-Tools. Anbieter mit verpflichtendem Client-Secret oder geschützter
dynamischer Registrierung werden nicht automatisch eingerichtet.

**Lokaler Verbindungstest:** Im gespeicherten MCP einen zugeordneten Runner
auswählen und **Gespeicherte Verbindung testen** anklicken. Der Runner startet
den Adapter in einer temporären OpenCode-Location und prüft den Tool-Katalog,
ohne ein Tool auszuführen oder eine Aufgabe anzulegen. Danach entfernt er
den Probe-MCP wieder. Das Ergebnis gilt für den ausgewählten Runner und
sagt nichts darüber aus, ob eine hinter dem Adapter laufende Anwendung
wie Blender erreichbar ist.

**Umgesetzt: versionierter Factory-Skill-Import.** Unter **Factory-Einstellungen → Skills**
entweder `SKILL.md` oder einen Ordner mit `SKILL.md` und Begleitdateien wählen,
die Vorschau prüfen und speichern. Name (zugleich die ID) und Beschreibung
stammen aus dem YAML-Kopf der `SKILL.md`. Dateien müssen
UTF-8 sein; ein Paket enthält höchstens 64 Dateien und 256 KiB Text.
Bei vorhandener ID muss eine neue Version ausdrücklich bestätigt werden.
Projektfreigaben wählen eine konkrete Version; beim Einreihen wird sie
für die Aufgabe festgehalten. Der verwaltete Runner installiert genau diese
Version in seinen privaten OpenCode-Skill-Speicher, nicht ins Repo.

**Weiterhin geplant:** ein expliziter „Neue Session“-Vorschlag bei einer
inkompatiblen festen Zuordnung.

## Test wiederholen

```sh
python3 spikes/opencode/integration_profiles.py /absoluter/pfad/zu/opencode
```

Benötigt Linux, Python, Git und OpenCode 2.4.1. Der Test startet nur einen
isolierten Loopback-Dienst mit temporärem HOME/XDG und synthetischen stdio-MCPs.
Ein lokaler Modell-Dummy prüft, dass nur freigegebene Skills angeboten und
geladen werden, und ruft synthetische MCP-Tools über stdio und Streamable HTTP
auf. Der Test kontrolliert auch die Wiederanbindung nach einem Reload und
die Verwendung eines Proxy-Bearer-Headers. Kein echter Modellaufruf und keine
echten Zugangsdaten.

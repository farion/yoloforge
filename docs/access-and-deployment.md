# Zugang, Verbindungen und Deployment

Status: ursprünglicher Zielentwurf; Gerätezugang, Docker, Backup/Restore und
Release-Workflow sind inzwischen implementiert. Der aktuelle Bedien- und
Installationsvertrag steht in [deployment.md](deployment.md).

Repository: `farion/yoloforge`. Serveralias: `rs9tm`. Veröffentlichung per
Commit/Tag/Push und GitHub Release; ausschließlich das Ansible des Betreibers
installiert auf dem Server. Kein Deployment durch den Entwicklungsagenten.

## Verantwortungsgrenze

Der Betreiber richtet Ubuntu-Host, SSH-Dienst, Benutzer, autorisierte öffentliche
Schlüssel und Container-Laufzeit ein. Sein Ansible installiert eine ausgewählte
Factory-Release-Version. Die Factory verändert weder `authorized_keys` noch
die SSH-Serverkonfiguration automatisch und benötigt keinen Docker-Socket.

Das Projekt wird auf GitHub gehostet. Geplante Release-Artefakte:

- versioniertes Server-Image in GHCR mit festem Tag und Digest;
- Linux-Binaries für TUI und Runner, Prüfsummen und Versionsinformationen;
- Compose-Beispiel, Konfigurationsreferenz und systemd-User-Unit;
- später Android-APK.

CPU-Architekturen und minimale Systembibliotheken müssen vor dem ersten Release
festgelegt und auf Ubuntu 24.04/26.04 geprüft werden. Ein GitHub-Release allein
installiert nichts: Ansible wählt eine Version und übernimmt den Rollout.

## SSH und Factory-Identität

Jede Verbindung benötigt zwei getrennte Nachweise:

1. SSH authentifiziert das Gerät am Host und schützt den Tunnel.
2. Ein Factory-Credential identifiziert den API-Client und dessen Rechte.

Die TUI unterstützt SSH-Hostalias, Benutzer, Port, Schlüsseldatei und ssh-agent.
Sie nutzt vorhandene OpenSSH-Konfiguration. Hostschlüssel werden verifiziert;
ein geänderter Schlüssel wird nicht still akzeptiert. Android verwaltet ein eigenes
Schlüsselpaar und einen bekannten Host-Fingerprint. Der Betreiber installiert
den öffentlichen Schlüssel auf dem Server. Private Schlüssel bleiben am Gerät.

Der SSH-Zugang darf auf die erforderliche Weiterleitung eingeschränkt werden.
Ein eventuell benötigter Git-Transport verwendet separat einen eingeschränkten
Host-Helper; der hierfür notwendige SSH-Befehl ist ausdrücklich zu konfigurieren.

### Erstes Gerät

1. Ansible startet die Factory mit persistentem Zustand und Loopback-Portbindung.
2. Mit dem Bootstrap-Schlüssel oder einem vorhandenen Admin-Zugang stellt
   der Betreiber einen rollenbeschränkten Geräteschlüssel direkt aus.
3. Desktop und Android melden sich mit Serveradresse und Zugangsschlüssel an.
4. Der Server speichert dessen Prüfwert, Rolle und Widerrufsstatus.

Zugangsschlüssel werden kryptographisch zufällig erzeugt und nicht in URLs,
Logs oder Telemetrie übertragen. Siehe [Betriebsanleitung](deployment.md#zugangsschlüssel-und-rollen).

### Weitere Geräte und Runner

Desktop, Android-App, lokaler Runner und Server-Runner können getrennte
Zugangsschlüssel erhalten.
Runner dürfen nur ihre zugewiesenen Aufträge und Zustände verwalten. Der Server-
Runner bekommt seine Dienstidentität bei der administrativen Einrichtung.

Widerruf sperrt neue API-Aufrufe und beendet aktive Streams/Long-Polls dieses
Credentials. SSH-Schlüssel werden zusätzlich durch den Betreiber entfernt;
bereits bestehende SSH-Verbindungen müssen bei Bedarf explizit beendet werden.

## Anbieterzugänge: Metadaten zentral, Secrets am Ausführungsort

Die Factory verwaltet wiederverwendbare Verbindungen, keine Zugangsdatenkopien
pro Projekt. Eine Verbindung beschreibt Anbieter, Konto, Instanz, Fähigkeiten
und Einrichtung am jeweiligen Runner. Secrets werden weder ins Projektgedächtnis
noch in Prompts, allgemeine Events, Git oder Diagnoseprotokolle übernommen.

| Verbindung | Einrichtung und Ablage |
| --- | --- |
| OpenAI lokal | Anmeldung im lokalen OpenCode-Service |
| OpenAI Server | Eigene Anmeldung im serverseitigen OpenCode-Service |
| Git über SSH | Eigener privater Schlüssel je Ausführungsort |
| GitHub-/Forgejo-API | Providerabhängige Anmeldung oder eingeschränkter Token, bevorzugt serverseitig |
| Factory-Gerät | Geschützter lokaler Credential-Speicher; Prüfwert auf dem Server |

OpenCode V2 dokumentiert ChatGPT-Anmeldung für Plus/Pro. Diese Anmeldung ist
nicht mit einem OpenAI-API-Key oder dessen Abrechnung gleichzusetzen. Beide
Ausführungsorte verwenden eigene Anmeldungen desselben Accounts; Limits werden
nicht pro Runner vervielfacht. Refresh-Tokens werden nicht zwischen Services kopiert.

Die TUI startet Anmeldungen gezielt lokal oder remote. Browser-/Device-Flows
werden nur verwendet, wenn die konkrete Integration sie unterstützt. Der Spike
prüft insbesondere die headless Server-Anmeldung und den zugehörigen Credential-
Speicher. Abgelaufene oder widerrufene Zugänge werden als Handlungsbedarf angezeigt.

Git-SSH erlaubt Clone/Fetch/Push, aber nicht automatisch Issues oder Pull Requests.
API-Rechte werden separat geprüft. Forgejo-Verbindungen enthalten ihre Instanz-URL.
Die Unterstützung verschiedener Login-Verfahren wird pro Anbieter ausgewiesen.

Ein Secret, das in der TUI für den Server eingegeben wird, wird gezielt über den
authentifizierten Tunnel zur zuständigen Credential-Verwaltung übertragen und
nicht im allgemeinen Command-Journal persistiert. Erstimplementierung: Zugriff auf
den nativen OpenCode-Credential-Speicher, statt eines selbstgebauten OAuth-Refreshers.
Für andere Secrets sind Speicherformat und Schutzmechanismus vor Umsetzung festzulegen.
Persistente Docker-Volumes alleine bieten keine Verschlüsselung.

## TUI: Verbindungen und neuer Projektassistent

Die Verbindungsübersicht zeigt je Konto und Ausführungsort:
`nicht eingerichtet`, `Anmeldung läuft`, `verbunden`, `erneute Anmeldung nötig`,
`Berechtigung fehlt`, `Prüfung fehlgeschlagen`.
Ein nicht erreichbarer Runner bedeutet nicht automatisch ungültige Credentials.

Der Projektassistent fragt:

1. Bestehendes Repository verbinden; neues Repository bei unterstützter API optional erstellen.
2. Hosting-Anbieter/Forgejo-Instanz und vorhandene Verbindung auswählen.
3. Ausführungsort Laptop, Server oder beide festlegen.
4. LLM-Verbindung und Modell auswählen.
5. Toolchain, Build-, Test- und Startbefehle erfassen.
6. Zugriff pro Ausführungsort prüfen und nur fehlende Zugänge einrichten.

Projekte speichern Referenzen auf Verbindungen, keine Secret-Werte. Die Prüfung
unterscheidet Lesen, Schreiben und Plattform-API. Ein Klon-Test allein beweist
keine Push-Berechtigung. Projekte können bei fehlenden Zugängen als Entwurf bleiben;
nicht verfügbare Ausführungsorte werden sichtbar blockiert.

## Release- und Update-Vertrag

- Ansible pinnt eine Release-Version bzw. einen Image-Digest.
- Factory- und OpenCode-Version sowie unterstützte Protokollversionen sind dokumentiert.
- Readiness wird erst nach erfolgreicher Datenbankmigration und Initialisierung gemeldet.
- Updates berücksichtigen aktive Runs: kontrolliert stilllegen oder Wiederanlauf
  mit Zustandsabgleich; ein Container-Neustart migriert keine laufenden Prozesse.
- Vor Migration konsistente Sicherung von Datenbank, Repositories und OpenCode-Zustand.
- Migrationen sind versioniert; nicht unterstützte Schema-/Client-Versionen werden abgelehnt.
- Rollback benötigt bei inkompatiblem Schema die zugehörige Sicherung, nicht nur ein altes Image.
- Ein Dienst-Neustart erhält Geräte- und Anbieteranmeldungen durch persistente Speicher.

## Integrationsprüfungen

- Erfolgreicher SSH-Zugang ohne Factory-Credential gewährt keinen API-Zugriff.
- Nur Admins dürfen neue Geräteschlüssel ausstellen.
- Widerrufenes Credential verliert auch eine bestehende Ereignisverbindung.
- Zwei Projekte verwenden dieselbe Verbindung ohne erneute Anmeldung.
- Fehlender Server-Git-Zugang blockiert nur den betreffenden Ausführungsort.
- Server-OpenAI-Anmeldung bleibt nach Container-Neustart verfügbar.
- Secret-Eingaben erscheinen weder im Eventlog noch in Wissensextraktionsquellen.

Quelle für OpenCode-Anbieterverfahren: https://opencode.ai/v2/docs/providers

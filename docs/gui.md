# YoloForge Desktop

Native Desktop-Oberfläche für Linux, macOS und Windows in Rust mit egui und SDL3. Dark Theme und wahlweise
englische oder deutsche Bedienung, mit derselben Factory, denselben Runnern und denselben Daten wie die
weiterhin vorhandene CLI/TUI. Die GUI ist kein Webview und startet zum Lesen
von Sessions keine zusätzliche OpenCode-TUI.

Release-Pakete enthalten die GUI und einen nativen Runner. Linux liefert zusätzlich
`factoryd` für die lokale Factory-Einrichtung mit. Unter macOS und Windows mit
einer bestehenden Factory verbinden; lokale Runner können auch dort durch die GUI
verwaltet werden. macOS-Pakete enthalten `YoloForge.app`, Windows-Pakete die `.exe`-Dateien.
Die Desktop-Pakete sind noch nicht herstellersigniert/notarisiert.
Details: [Runner-Installation](runner-installation.md), [CI und Releases](ci.md).

## Projektlogo

Projekte können optional ein eigenes Bild erhalten: beim Erstellen oder unter
**Projekt-Settings → Allgemein**. PNG, JPEG, WebP und GIF bis 16 MiB werden mittig
quadratisch zugeschnitten und als kleines PNG gespeichert. Das Logo erscheint links
neben dem Intent-Button; ohne Logo bleibt der Button über die volle Breite erhalten.
Ersetzen und Entfernen werden erst mit dem Projekt gespeichert. Das Bild wird über
die Factory synchronisiert und bleibt mit den Cache-Daten auch offline sichtbar.

## Sprache und Begriffe

Unter **Factory settings → Language** bzw. **Factory-Settings → Sprache**
stehen **Auto**, **English** und **Deutsch** zur Auswahl. **Auto** ist auch für
bestehende Installationen der Standard: deutsche Systemsprache bedeutet Deutsch,
ansonsten wird Englisch verwendet. Die Wahl wird erst mit **Save / Speichern**
übernommen, lokal pro Gerät gespeichert und benötigt keinen Neustart oder
Serverzugang. Schließen der Settings behält die Auswahl als lokalen GUI-Entwurf.

Die Oberfläche verwendet in beiden Sprachen **Intent / Intents** statt
Aufgabe/Aufgaben. Factory, Runner, Agent-Namen und Fachbegriffe wie Settings,
Models, Provider, Skills, Sessions und Tools bleiben englisch. Nutzereingaben,
vorhandene Daten, Modellantworten und technische Serverausgaben werden nicht
übersetzt. API-, Datenbank- und interne Typnamen bleiben unverändert.

GUI-Texte liegen zentral in `crates/factory-gui/locales/messages.json` als
Paare `[Deutsch, English]`. `crate::tr!` liefert statische Labels oder formatierte
Texte; `build.rs` erzeugt die Makros, sodass Rust Formatargumente in beiden
Sprachen prüft. Die Katalogschlüssel sind stabil und dürfen ältere Begriffe
enthalten; sichtbar sind ausschließlich die jeweiligen Übersetzungen.

## Starten

```sh
cargo build --workspace --locked
cargo run --locked -p factory-gui --bin yoloforge-gui
```

Ohne konfigurierten Zugang erscheint die Verbindungsübersicht. Dort entweder
**Verbinden…** wählen und Serveradresse sowie Zugangstoken angeben oder eine
**Lokale Factory** starten. Bei aktiver Verbindung ist direkt in der Übersicht
nur **Trennen** verfügbar. Für entfernte Verbindungen ist HTTPS oder
ein lokaler SSH-Tunnel erforderlich. TLS-Prüfung und Hostschlüsselprüfung werden
nicht abgeschaltet.

Ein vorhandener Zugang kann wie bei der TUI übergeben werden:

```sh
FACTORY_TOKEN_FILE=/absoluter/pfad/device.token \
  target/debug/yoloforge-gui --server https://factory.example
```

`FACTORY_TOKEN` und `FACTORY_TOKEN_FILE` nicht gleichzeitig setzen. Direkt im
Dialog eingegebene Tokens werden in einer privaten Datei unter
`$XDG_CONFIG_HOME/yoloforge/` (standardmäßig `~/.config/yoloforge/`) gespeichert.
Das Verzeichnis hat Rechte `0700`, die Tokendatei `0600`. Die Desktop-Einstellungen
merken sich Serveradresse und Dateipfad, nicht den Token selbst. Beim nächsten
Start wird die Verbindung automatisch wiederhergestellt.
Beim lokalen Start legt die Anwendung private Tokendateien an und
merkt sich nur deren Pfad. Der nächste Start kann diese Verbindung wiederverwenden.

### Lokale Factory

`factoryd` und `factory-runner` müssen neben `yoloforge-gui` liegen. Das ist nach
dem Workspace-Build und im GUI-Releasepaket der Fall. Der lokale Start verwendet
einen eigenen Datenbestand unter `$XDG_DATA_HOME/yoloforge/local`, standardmäßig
`~/.local/share/yoloforge/local`, sowie `127.0.0.1:17433`.

- Vorhandene Server auf 7432 und die Daten des Entwicklungsskripts werden nicht verändert.
- Token, Runner-ID, SQLite-Daten und Journal bleiben erhalten.
- `factoryd.log` im lokalen Datenverzeichnis und `factory-runner.log` im jeweiligen
  Runner-Verzeichnis erklären Startfehler.
- Bereits laufende, authentifizierbare lokale Dienste werden wiederverwendet.
- Fremde Portbelegungen oder unpassende Zugangsdaten werden nicht übernommen.
- GUI- oder Tab-Schließen beendet keine Dienste, Sessions oder Aufgaben.

### Lokale Runner verwalten

In **Infrastruktur** startet **Lokalen Runner starten** einen zusätzlichen Runner
auf dem Desktop-Rechner und verbindet ihn mit der aktuell ausgewählten Factory.
Zum erstmaligen Einrichten wird ein Factory-Admin-Zugang benötigt. Jeder Runner
bekommt eine eigene dauerhafte ID, ein Journal und einen widerrufbaren
Runner-Zugang; der Admin-Token wird nicht an den Runner weitergegeben.
Auch **Lokale Factory** startet automatisch einen solchen Runner.

Die Runner richten ihren eigenen verwalteten OpenCode-Server automatisch ein.
Sie laden die Factory-Version von OpenCode bei Bedarf herunter, verwenden einen
dauerhaft gespeicherten Loopback-Port und beziehen den Modellkatalog von der
Factory. Anbieter und Standardmodelle werden weiterhin zentral in der Factory
eingerichtet. Die Factory muss den Modellzugang mit Runner-Zugang unterstützen
(GUI, Runner und Factory zusammen aktualisieren).

Die GUI speichert die Zuordnung unter
`~/.config/yoloforge/local-runners.json` bzw. `$XDG_CONFIG_HOME/yoloforge/`.
Private Zugangsdaten, Journal, OpenCode-Laufzeit und Log liegen unter
`$XDG_DATA_HOME/yoloforge/runners/<runner-id>/`; bereits vorhandene lokale
Factory-Runner behalten ihr bisheriges Verzeichnis. Nach erneutem Verbinden
werden vorhandene Prozesse wiederverwendet und fehlende Runner gestartet.
Ein Dateilock verhindert doppelte Verwaltung durch mehrere GUIs.

Nur GUI-verwaltete lokale Runner haben einen runden X-Button auf der rechten
oberen Kachelecke. Er öffnet eine Bestätigung; Entfernen stoppt den identifizierten
Runner mitsamt seinem verwalteten OpenCode, widerruft seinen Zugang und entfernt
ihn aus der lokalen Startliste. Laufende oder ungeklärte Aufträge blockieren das
Entfernen. Journale und Arbeitsdaten werden nicht gelöscht, historische
Serverdaten bleiben bestehen. GUI-Schließen allein beendet keine Runner.

Die Infrastruktur zeigt OpenCode direkt in der Runner-Kachel, ohne separate
Server-/Projektkacheln. Desktop- und Runner-Kacheln zeigen die von der Factory
beobachtete IP; hinter Proxies oder Tunneln kann dies deren IP sein. Bei älteren
Servern ohne IP-Angabe steht „IP unbekannt“. Desktop-IDs stehen nur in den Details.
- Die Desktop-Präsenz verwendet eine dauerhafte lokale ID in `desktop-client-id`
  neben `desktop.json`. Neustarts, Neuverbindungen und lokale Verbindungsprüfungen
  verwenden dieselbe ID, auch bei gleichzeitig gestarteten GUI-Fenstern. Die ID
  ist keine Zugangsdaten-Datei; die serverseitige Trennung nach authentifiziertem
  Gerät bleibt bestehen. Einträge früherer Zufalls-IDs laufen noch nach fünf
  Minuten aus. Eine beschädigte Identitätsdatei wird nicht still ersetzt.
- OpenCode und seine Provider-Anmeldung müssen auf dem ausführenden Host vorhanden
  sein; der vorhandene Projekt-Einrichtungsweg startet dort bei Bedarf den nativen Service.

## Bedienung

### Verbindliche Designregeln

Die Nutzerregeln sind dauerhaft in `crates/factory-gui/AGENTS.md` festgehalten
und als gemeinsame Komponenten in `src/design.rs` umgesetzt: Textfelder mit
13/11 pt Innenabstand, gleich breite flache Navigationszeilen mit rechtsliegenden
Zählern sowie Dialoge mit 24 pt Innenabstand, 32 pt Zielabstand zum Fenster und
Schließen-X oben rechts. Funktionale Icons stammen aus Phosphor. Das Schließen
eines Dialogs erhält Entwürfe und beendet keine laufende Arbeit.
Buttons, Tabs und Menüs teilen sich dieselbe Hoverfarbe; Buttonrahmen entfallen.
Auch farbige Hauptaktionen verwenden beim Hover diese gemeinsame Farbe.
Trennlinien und neutrale Flächenrahmen sind eine hellere Variante des
blau-schwarzen Hintergrundtons, keine neutralgrauen Framework-Linien.

### Navigation

Die Breadcrumb bleibt als Seitenüberschrift erhalten. Die Inhaltsbereiche
wiederholen den Seitentitel nicht; fachliche Unterüberschriften und Dialogtitel
bleiben bestehen.

Im ersten Tab liegt die Verwaltung. Die Sidebar enthält Projektwahl,
**+ Aufgabe**, Aufgaben, Fragen, Sessions, Orchestrator und Supervisor. Overview,
Benachrichtigungen, Infrastructure, Runner und Diagnose bilden darunter den globalen Factory-Bereich.
Das Hauptmenü ist 300 pt breit, seine gleich breiten Zeilen haben passende
Phosphor-Icons und kompakte 4-pt-Abstände. Die Tabs im Gesamtheader zeigen ein
Fabrik-Icon für die Factory und das OpenCode-Terminal-Icon für Sessions.
Der Factory-Tab bleibt fest stehen. Session-Tabs nutzen die gesamte restliche
Breite und scrollen bei Überlauf weich und pixelweise über Pfeile, Mausrad oder
Trackpad. Am Rand angeschnittene Tabs zeigen, dass die Leiste weitergeht; es
werden keine ganzen Tabs seitenweise ausgetauscht.
Das Zahnrad rechts im Footer öffnet **Factory-Einstellungen** als Dialog mit
derselben Kopfzeilenstruktur, Schließen-X und Bereichsnavigation wie die
Projekteinstellungen:
links **Modell-Anbieter**, **Default-Modelle**, **Skills**, **MCPs** sowie
**Geräte und Zugänge**. Die Geräteverwaltung zeigt aktive Zugänge mit Name und
Rolle; ein Zugang wird direkt am Eintrag mit einer kurzen Bestätigung widerrufen.
Widerrufene Zugänge und technische Rohdaten werden nicht mehr aufgelistet.
Separate Bereiche für Einladungen, Widerruf, Desktop-Meldungen und offene
Zustellungen entfallen. Die Funktionsauswahl und Entwürfe
bleiben beim Schließen und Wiederöffnen erhalten.

Direkt neben der Projektwahl öffnet ein Icon das Menü mit **Neues Projekt …**,
**Projekteinstellungen …**, **Projekt pausieren**, **Verbrauch …** und **Projekt löschen …**.
**Verbrauch …** zeigt erfasste Ein- und Ausgabetokens insgesamt, das Datum der
ersten Messung und den Durchschnitt je Kalendertag seitdem. Fehlt das Messdatum,
wird der Durchschnitt ab dem Anlagedatum des Projekts berechnet.
Die Projekteinstellungen öffnen als zweispaltiger Dialog mit dem Projektnamen
im Titel. Änderungen an den Einstellungen bleiben beim Seitenwechsel erhalten.
**Speichern** im gemeinsamen Footer schreibt die Änderungen aus allen Bereichen
und schließt nach Bestätigung, **Abbrechen** oder Schließen fragt bei offenen
Änderungen nach. Projektlöschen verlangt den exakten Namen und die aktuelle
Revision; ausstehende oder angenommene Aufträge verhindern es. Das Projekt
verschwindet aus dem normalen Betrieb, Historie und Repository-Dateien bleiben erhalten.

Unter **Projekteinstellungen → Skills** können Factory-Admins importierte
Factory-Skills per Häkchen freigeben und die Version auswählen. Repository-Skills
aus dem Basisbranch lokal erreichbarer Git-Repositorys erscheinen im separaten
Abschnitt und lassen sich ebenfalls per Häkchen freigeben. Das erste
Speichern aktiviert die Freigabeliste; eine leere Liste gibt keine Skills frei.
Voraussetzung ist ein nativer verwalteter Runner.

Unter **Factory-Einstellungen → MCPs** öffnet **MCP hinzufügen**
einen Dialog für lokale oder entfernte MCPs. Nach dem Speichern erscheint der
Eintrag unter **Vorhandene MCPs**. Dort lässt er sich im selben Formular
bearbeiten oder nach Bestätigung löschen. Projektfreigaben müssen vor dem
Löschen entfernt werden.

Für lokale MCPs: Name, absoluter Programmpfad auf dem Runner,
Argumente einzeln und Runner-Zuordnung. Standardmäßig sind alle unterstützten
Runner aktiviert, auch später hinzugefügte. Abgewählte Runner bleiben als
ausdrückliche Ausnahmen deaktiviert; entfernte lokale Runner erscheinen nicht
mehr in der Auswahl. Bestehende Katalogeinträge erhalten ebenfalls diese
Automatik. Bereits festgeschriebene alte Ausführungsprofile behalten dagegen ihre
ursprüngliche Zuordnung. Die technische ID wird automatisch
erzeugt. In den Projekteinstellungen unter **MCPs**
gilt die Freigabe für den ganzen Server. Ein Häkchen am Eintrag macht ihn im
Aufgabendialog auswählbar. Dort öffnet der Button **MCPs** unten in der
Aktionszeile ein Checkbox-Menü; im Formular selbst steht keine MCP-Liste.
Bei neuen Aufgaben sind standardmäßig alle für das Projekt freigegebenen MCPs
aktiviert, auch wenn das Menü nicht geöffnet wird. Eigene Abwahlen bleiben im
Entwurf erhalten. Bereits als erforderlich gespeicherte MCPs bleiben
automatisch eingebunden, bis ihre Freigabe entfernt wird. Remote-MCPs zeigen
ihren Verbindungsstatus wie im Factory-Katalog an.
[Details und aktueller Umfang](integration-profiles.md).

**Gespeicherte Verbindung testen** startet einen lokalen MCP zur Prüfung auf
dem ausgewählten Runner, ohne ein Tool auszuführen.

Unter **Remote** Name und HTTPS-Endpunkt eingeben und **Authentifizierung
prüfen** wählen. Für einen offenen MCP erscheinen keine Zugangsdatenfelder.
Bei erkanntem OAuth ergänzt die Factory die Anbieter-URLs. Ohne öffentlichen
Factory-Callback meldet sich der Browser lokal bei der Desktop-GUI zurück.
Sind alle OAuth-Angaben verfügbar, öffnet sich der Browser. Unter dem
Prüfbutton zeigt YoloForge das Ergebnis an. Erst nach erfolgreicher
Prüfung kann der MCP gespeichert werden. Nur wenn keine
Methode eindeutig erkannt wird, erscheint das
Pulldown zur manuellen Auswahl, etwa für einen API-Key-Header. Schlüssel
werden maskiert eingegeben und nach dem Speichern nicht mehr angezeigt.
Die [OAuth-Einrichtung](integration-profiles.md) beschreibt die Voraussetzungen.

Die Authentifizierungsprüfung und **Gespeicherte Verbindung testen** führen
keine MCP-Tools aus. Die Freigabe erfolgt in den Projekteinstellungen unter
**MCPs**; eine Runner-Zuordnung ist dafür nicht nötig.

Unter **Factory-Einstellungen → Skills** können Admins eine
`SKILL.md` oder einen Skill-Ordner mit UTF-8-Begleitdateien importieren. Nach
der Auswahl zeigt die Vorschau die enthaltenen Dateien sowie Name und
Beschreibung aus der `SKILL.md`. Eine neue Version überschreibt die
alte nicht. In den **Erweiterten Projekteinstellungen → Skills**
wählt man pro Factory-Skill genau eine Version für den Implementer.

Im Footer öffnet links das Connect-Icon die **Verbindungsübersicht** mit einer
Statusgrafik für Desktop und Factory sowie den direkten Aktionen
**Trennen** bzw. **Verbinden…** und **Lokale Factory**.
Die Desktop-Version steht neben dem Zahnrad unten rechts.

### Factory Overview

**Benachrichtigungen** steht direkt unter **Overview** im Factory-Menü.
Der Zähler zeigt ungelesene Meldungen; der bisherige Meldungsbutton im Kopfbereich entfällt.

**Overview** ist der erste Eintrag unter FACTORY und zeigt den lebenden Aufgabenfluss:
Eingang, Verteilung, Ausführung und Ergebnis. Die Ausführung verzweigt nach den
tatsächlich zugeordneten Runnern und Sessions. Standardmäßig werden alle Projekte
angezeigt; ein Filter beschränkt die Ansicht auf das aktuell ausgewählte Projekt.
Aufgabenkarten heben sich mit blauer Fläche und einem blauen Seitenakzent von
den neutralen Stationsköpfen ab. Sie tragen Phosphor-Icon, blaue Ticket-ID-Pill,
Titel und farbig gefüllte Status-Pill. Das Projekt ergibt sich aus der Ticket-ID;
sein vollständiger Name bleibt im Tooltip verfügbar. Die ID-Farbe kennzeichnet die
Aufgabe, nicht ihren Status: Entwürfe sind violett markiert, eingereihte/aktive
Aufgaben blau, Rückfragen und Wartezustände gelb, Fehler rot und Erfolge grün.
Pro Station bzw. Session sind zunächst drei Karten sichtbar;
weitere Aufgaben bilden einen aufklappbaren Stapel. Rückfragen, Fehler und aktive
Aufgaben stehen dabei vor gewöhnlichen Warteschlangeneinträgen.

Rückfragen bleiben an ihrer Ausführungsstation mit gelbem Rückfrage-Icon und
pulsierendem Rand; Fehler bleiben sichtbar. Bestätigt erfolgreiche Aufgaben
verschwinden nach zwei Minuten aus der Grafik, nicht aus der Factory. Bereits
alte Archivaufgaben werden beim Öffnen nicht erneut eingeblendet. Der
Erfolgszähler bezieht sich auf alle erfolgreichen Aufgaben im gewählten Filter.
Ein Kartenklick öffnet die Aufgabe schreibgeschützt; der Kopf einer
Ausführungsstation öffnet die zugehörige Session.

Lichtimpulse auf aktiven Verbindungen, ein Heartbeat an aktiven Stationen und
kurze Kartenbewegungen zeigen beobachtete Aktivität, keinen geschätzten
Fortschritt. Animation lässt sich abschalten. Offline zeigt die Ansicht
ausdrücklich den letzten Stand und hält Bewegung und Ausblendung an.

### Infrastructure

Die bisherige Overview heißt **Infrastructure**. Die schreibgeschützte Grafik
verknüpft Clients, den verbundenen Server, dessen Runner und gemeldete
OpenCode-Instanzen. Zähler unterscheiden aktive, auf Antworten wartende und
eingereihte Aufträge. Farben und Statusbeschriftungen zeigen Aktivität sowie
fehlende oder veraltete Beobachtungen. Hover zeigt Identität, Version,
Kontaktzeit, Zuordnung und aktuelle Sessions, aber keine Zugangstoken.
Die Legende steht unten. OpenCode-Instanzen desselben Projekts stehen automatisch
nebeneinander im gemeinsamen Graphen, ohne zusätzliche Gruppierungsoption.
Projektübergreifend genutzte Dienste erscheinen nur einmal in einer gemeinsam
beschrifteten Gruppe; Instanzen ohne Zuordnung separat.

Client-Präsenz benötigt aktualisierte Clients und Server: Sie basiert auf
authentifizierten HTTP-Kontakten, ist nach 30 Sekunden veraltet und wird nach
fünf Minuten ausgeblendet. Sie wird nicht über einen Serverneustart hinweg als
live wiederhergestellt. Ältere Clients ohne Präsenzheader sind nicht einzeln
erkennbar. Registrierte Geräte sind nicht automatisch laufende Clients. Es wird
nur der aktuell verbundene Server dargestellt, kein unbekanntes Servernetzwerk.

Aufgaben-, Session- und Benachrichtigungslinks öffnen das konkrete Ziel.
OpenCode-Ansichten sind zusätzliche, wiederverwendete Tabs im Hauptfenster.
Ihre linke Sidebar ist standardmäßig 500 pt breit und bis auf 80 % der
Fensterbreite verstellbar. Sie zeigt ausschließlich Aufgaben und Fragen der
jeweiligen Session, einschließlich Folgeausführungen: älteste Ausführung zuerst, ihre
zugeordneten Aufgaben vor den daraus entstandenen Fragen. Aktualisierte Antworten
verändern diese Reihenfolge nicht. Noch nicht ausgeführte Folgeaufgaben erscheinen
nicht vorzeitig in der Sessionhistorie.
Die ganze Box ist mit Hover und Zeigercursor anklickbar, ohne separaten Button.
Ein Klick zeigt Aufgaben schreibgeschützt beziehungsweise Fragen mit ihrer
Antwort an. Bei offenen oder abgelehnten Fragen öffnet er direkt den
bestehenden Antwortdialog für genau diese Frage. Eingereihte oder unklare
Antwortzustellungen erhalten keine zweite Sendeaktion. Die Session bleibt beim
Öffnen und Schließen dieser Dialoge im Hintergrund erhalten.
Fragenfenster zeigen ausschließlich verständliche **Frage**/**Antwort**-Paare,
bei mehreren Fragen mit Trennlinien. Auswahlantworten erscheinen mit ihrem
Anzeigenamen, Ja/Nein-Antworten und Berechtigungsentscheidungen als Klartext.
JSON, vollständige Metadaten und Exportaktionen gehören nicht in diese Fenster.
Listen, lange Dialoge und Archive scrollen unabhängig. Technische Originaldaten
bleiben in den jeweiligen Details zugänglich.

### Projekte

**Neues Projekt …** und **Projekteinstellungen → Allgemein** zeigen Name,
Projektkürzel und lokalen Pfad. Der Pfad kann
eingetippt oder über den Ordnerdialog gewählt werden.
Bei einem bestehenden Git-Repository wird der ausgecheckte Branch übernommen.
Ist der Ordner noch nicht vorhanden, kann der Runner ihn beim ersten
Arbeitsbereich mit `main` anlegen. Bestehende Ordner ohne Git-Repository werden
nicht verändert. Der ausführende Runner muss den angegebenen Pfad erreichen können.

Die Runner-Zuordnung steht bei neuen Projekten auf **Automatisch** und lässt
sich im Erstellungsdialog oder später unter **Projekteinstellungen →
Runner-Zuordnung** ändern. Änderungen betreffen neue Zuweisungen; bestehende
Arbeitsbereiche behalten ihren Runner. Der Pfad lässt sich
ändern, solange das Projekt noch keine Arbeitsbereiche hat.

### Aufgaben

Schlägt die Orchestrator-Vorbereitung fehl, zeigt **Orchestrator → Offen**
den fehlgeschlagenen Versuch mit Ursache; der erste Fehler eines Arbeitsbereichs
erscheint zusätzlich als Benachrichtigung und Toast. Die Aufgabe bleibt
eingereiht. Die Factory versucht die Vorbereitung nach einer Minute erneut,
auch nach einem Neustart. Nach erfolgreicher Vorbereitung wandert der Fehler
ins Archiv. Fehlgeschlagene Modell-Auswertungen werden mit längeren Abständen
weiter versucht. Es muss kein neuer Auftrag angelegt werden.

**Eingereiht** heißt, dass der Orchestrator diesen Auftrag noch nicht prüft.
Danach zeigt die Aufgabenliste den aktuellen Schritt oder dass die Aufgabe
wartet, z. B. **Wird vorbereitet**, **Orchestrator prüft** oder **Blockiert**
bei fehlendem Projektmodell.

Im Aufgabendialog steht unter dem Prompt der **Verlauf** mit Zeitstempeln.
Fehler sind rot, Warnungen gelb und erfolgreiche Schritte grün. Der Verlauf
scrollt mit dem Dialog; ältere Einträge können nachgeladen werden.

Jedes Projekt hat ein eindeutiges Kürzel, jede serverseitig gespeicherte Aufgabe
eine dauerhafte Ticketnummer, z. B. **TEST-1** oder **WOAI-1**. Das Kürzel wird
beim Einrichten angegeben (leer = aus dem Namen ableiten) und über
**Projekteinstellungen → Allgemein → Projektkürzel** geändert: 1–10 ASCII-Buchstaben/Ziffern,
beginnend mit einem Buchstaben; Kleinbuchstaben werden großgeschrieben.
Ein neuer Präfix ändert bestehende Referenzen, aber nie deren laufende Zahlen.
Frühere Kürzel bleiben für dasselbe Projekt reserviert.

Die Zählung gilt pro Projekt, einschließlich Entwürfen und technischen Tickets.
Bearbeiten, Freigeben und Archivieren vergeben keine neue Nummer. Lokale, noch
nicht gespeicherte Entwürfe bekommen keine erfundene Nummer. Bestehende Aufgaben
werden bei Servermigration 34 einmalig nach Erstellungszeit/Einfügereihenfolge
nummeriert. Nummernvergabe, Aufgabe und Request-Replay liegen in derselben
SQLite-Transaktion; Neustarts und gleichzeitige Clients erzeugen keine Duplikate.
Der Prompttext selbst bleibt unverändert.

Ticketnummern stehen vor dem Aufgabentext, wo möglich als neutrale Pill in Listen,
Fragen-Karten, Benachrichtigungen und Session-Sidebar; in Dialogtiteln und
Kontextzeilen als Präfix. Aufgaben und Fragen sind auch über die Nummer suchbar.
Projektwahl und Projektdialoge zeigen bzw. bearbeiten das Kürzel. Für diese
Funktion müssen Server und GUI aktualisiert werden; ältere Server liefern
keine Ticketnummern.

Die Aufgabenliste besteht aus flachen, rahmenlosen Zeilen mit dezenter Fläche
und nur 2 pt Rundung. Sie zeigt die Prompt-Zusammenfassung in normaler Schrift,
den Status rechts oben als Pill und darunter die Anzahl der Anhänge
(Bilder und Dateien) sowie in einer eigenen Zeile das Erstellungsdatum.
Entwürfe tragen die Pill **Draft**. Rechts unten erscheint **OpenCode anzeigen**,
sobald eine Session nachgewiesen ist; andernfalls **Bearbeiten**, solange die
Factory noch keine Verarbeitung begonnen hat. Details, Prompt-Auszüge und
Anhangvorschauen werden in der Liste nicht zusätzlich eingeblendet.

**Anzeigen** steht immer links neben der jeweiligen Aktion und öffnet denselben
Aufgabendialog schreibgeschützt: vollständiger, kopierbarer Prompt und passive
Anhangvorschauen, aber keine Änderungen, Entfernen-X, Dateiauswahl oder
Speicher-/Einreihen-Aktionen. Das Anzeigen reserviert keine Aufgabe und verändert
keine lokalen Entwürfe oder offenen Zustellungen. Fehlende Anhangbytes werden
getrennt per GET geladen; offline bleibt der bekannte Auftragstext lesbar.

**Bearbeiten** reserviert die Aufgabe atomar auf dem Server und hält sie als
Draft aus der Einplanung heraus. Erst danach öffnet sich der Aufgabendialog mit
vollständigen Anhängen. Eine begonnene Orchestrator-Auswertung, Vorbereitung oder
Sessionzuordnung verhindert Änderungen – auch wenn noch keine OpenCode-Session
angezeigt werden kann. Parallele Scheduler-/Bearbeitungsversuche werden
transaktional aufgelöst. Andere Clients können eine reservierte Aufgabe nicht
versehentlich freigeben. Schließen und Neustart erhalten Entwurf und Reservierung;
**Entwurf speichern** hält sie als Draft, **Einreihen** gibt sie explizit wieder
frei. Unklare Zustellungen werden mit derselben gespeicherten Anfrage abgeglichen.
Die Reservierung erfordert den aktualisierten Server (Schema 33).

Rechts über der Aufgabenliste stehen die Tabs **Queue (Zahl)**, **Draft (Zahl)** und
**Archiv (Zahl)**. Standard ist Queue; die Auswahl hat einen Akzent-Unterstrich.
Die Zahlen zählen alle Aufgaben des gewählten Projekts unabhängig vom Suchtext.
Die bisherige Gesamtzahl am Titel und das Filter-Pulldown entfallen.
Ins Archiv wechseln Aufgaben mit bestätigt erfolgreicher Ausführung und ohne
offene, abgelehnte oder ungeklärt zugestellte Gegenfrage. Fehler, Unterbrechungen,
wartende Arbeit und reine Vorbereitung bleiben aktuell; Entwürfe stehen separat.
Das Archiv ist eine automatische, schreibgeschützte Gruppierung: Daten und
Factory-Abläufe bleiben unverändert.

**+ Aufgabe** öffnet einen Dialog mit Prompt, Zuordnung und Anhängen:

Das Eingabefeld ist wie ein OpenCode-Prompt gestaltet: Monospace, dezente Fläche,
linke blaue Akzentlinie und ausgegrauter Platzhalter, ohne Projekt-/Längenanzeige.
Die Anhänge stehen über dem Textfeld. Bilder und Dateien lassen sich per Drag-and-drop
anpinnen; sie erscheinen als kompakte Vorschauen
nebeneinander mit einem Entfernen-X oben rechts. Textdateien zeigen einen
kleinen Textausschnitt statt einer Grafik, andere Dateien ein Dateisymbol.
Dateiname, Typ und Größe stehen im Hover-Tooltip. HTML/SVG-Code wird nur als
Text angezeigt, niemals ausgeführt oder mit externen Ressourcen nachgeladen.
Der Dialog wächst mit dem Inhalt bis zum verfügbaren Platz; erst dann scrollt
der Inhalt. Unter der Trennlinie steht standardmäßig nur die Aktionszeile.
Ein kleiner Phosphor-Reglerbutton links blendet die erweiterten Optionen samt
zweiter Trennlinie ein und aus, ohne ausgewählte Werte zu verwerfen.
Die erweiterte Zeile enthält die breite
Modellauswahl (etwa **Default (gpt-6.2-sol / OpenAI Arbeit)**), die Zuordnung mit
**Auto Runner**, die Datei-Auswahl und **MCPs**. Modellnamen stehen als
**Modell / Provider**. Eine weitere Trennlinie grenzt die Aktionszeile ab.
Sie enthält rechtsbündig genau einen Button mit fester Breite (200 pt);
direkt links daneben stehen mit 24 pt Abstand zwischen den Bedienelementen
**Nicht schließen** und **Entwurf**. Beide Checkboxen behalten ihren Zustand
auch nach einem Neustart. Der Button heißt entsprechend **Einreihen**,
**Einreihen & neu**, **Entwurf** oder **Entwurf & neu**.
Entwürfe starten keine Arbeit. Mit **Nicht schließen** öffnet sich nach
Bestätigung eine leere Aufgabe im selben Projekt; sonst schließt der Dialog.
Offline sind **Entwurf** und **MCPs** ausgeblendet. **Nicht schließen** bleibt:
Der einzelne Button heißt **Lokal vorbereiten** oder **Vorbereiten & neu**.
Die Bestätigung erscheint als Toast.
Icon-Aktionen haben deutsche Tooltips.
**Strg+V** übernimmt Screenshots aus der nativen
Zwischenablage als Bildanhang. Bei reinem Text fügt **Strg+V** den Text wie gewohnt
ein; **Strg+Umschalt+V** fügt nur Text ein.
Strg+C kopiert markierten Prompttext in die Systemzwischenablage. Unter Wayland
wird er bei vorhandenem Data-Control-Protokoll über eine eigene Verbindung
ausgeliefert, unabhängig vom Rendern oder Fokus des Fensters; andernfalls über
SDL. Leere Kopierausgaben löschen die vorhandene Zwischenablage nicht und
Kopierfehler erscheinen als Meldung.
Die Zuordnung blendet geschlossene und beendete Sessions aus. Wird eine bereits
gewählte Session geschlossen, muss die Zuordnung bewusst korrigiert werden;
der Entwurf wird nicht stillschweigend einer anderen Session zugeordnet.
Im geöffneten Menü stehen die vollständigen Sessiontitel in normaler Schriftgröße.
Lange Titel brechen um, einzeilige Optionen sind vertikal zentriert. Der Status
erscheint separat als farbige Pill.

Sessionnamen in Zuordnung, Sessionliste und Tabs verwenden den von OpenCode
gelieferten Titel. Der Runner überträgt ihn mit dem vorhandenen Leseauszug,
der Server liefert ihn als kleine Snapshot-Metadaten. Neue Sessions erhalten
keinen erzwungenen technischen Factory-Titel mehr. Bis ein brauchbarer Titel
vorliegt, wird das Arbeitsziel angezeigt. Bestehende manuelle OpenCode-Titel
werden nicht überschrieben; es gibt keinen zusätzlichen Modellaufruf zum Benennen.
Für die Titelübertragung sind aktualisierte Server und Runner erforderlich;
der reine Dialogumbau benötigt nur einen GUI-Neustart. Keine Schema-Migration nötig.

Beim Ziehen aus einem Dateimanager (z. B. Thunar) zeigt das Fenster eine blaue
Drop-Markierung. Loslassen bei geöffnetem Aufgabendialog lädt die Anhänge in
den lokalen Entwurf. SDL3 liefert Drag-Eintritt, Abbruch und Datei-Drops auch
unter nativem Wayland. Bei gesetztem `WAYLAND_DISPLAY` verwendet die GUI direkt
Wayland, ohne auf XWayland auszuweichen. Unter einer X11-Sitzung bleibt X11 nutzbar.
Die gebündelten SDL-Quellen erhalten eine gezielte Korrektur der Wayland-
Formatauswahl: Bei gleichzeitigen Datei- und Klartextangeboten haben Dateipfade
Vorrang. Details und festgelegte Quellversion: `vendor/sdl3-sys/YOLOFORGE-PATCH.md`.

- **Entwurf auf Server speichern** startet keine Arbeit.
- **Einreihen** gibt die Aufgabe ausdrücklich zur Bearbeitung frei.
- Automatische Verteilung oder eine konkrete bestehende Arbeitssession auswählen.
- Bilder und Dateien über Dateiauswahl oder Drag-and-drop hinzufügen; Bilder auch
  aus der Zwischenablage. Anhänge können vor dem Senden entfernt werden.
- Grenzen: vier Bilder mit zusammen 4 MiB sowie 16 weitere Dateien mit zusammen
  4 MiB. Factory hat keine separate Längenbegrenzung für den Prompt und kürzt ihn
  nicht. Die Größenlimits für HTTP-Anfragen und das Kontextlimit des Modells gelten weiterhin.
- Vorschauen dekodieren im Hintergrund mit Größen-/Speichergrenzen. Die Anzeige
  ist eine skalierte Vorschau, kein Ersatz für die unveränderten Originalbytes.

Die Liste zeigt Prompt-Auszüge, Bilder, Status und direkte Sprünge. Die
Detailansicht enthält vollständigen Auftrag, Anhänge, Routing, Ausführung,
Ergebnis und technische Metadaten. Bereits eingeplante Aufgaben werden nicht
nachträglich zu anderen Agentenprompts umgeschrieben; dafür Folgeaufgaben verwenden.

Eine deaktivierte Verteilung wird durch neue GUI-Aufgaben nicht unbemerkt wieder
aktiviert. Der alte CLI-/TUI-Aufruf `SaveTask` behält seine bisherige Freigabesemantik.
Alte Clients können neue Datei-Anhänge nicht bearbeiten oder stillschweigend entfernen.

### Fragen

Suche links und die Tabs **Aktuell (Zahl)** / **Archiv (Zahl)** rechts stehen in
derselben Leiste wie bei Aufgaben. Aktuell ist der Standard. Nur bestätigte
Antworten sind archiviert; offene, abgelehnte, eingereihte und ungeklärt
zugestellte Antworten bleiben aktuell. Die Zähler zählen die Fragegruppen pro
Tab im gewählten Projekt unabhängig von der Suche. Die frühere Checkbox entfällt.

Eine Karte bündelt alle offenen und beantworteten Fragen eines ausgeführten
Auftrags. Gemischte Gruppen bleiben unter Aktuell und wandern erst bei vollständig
bestätigter Beantwortung ins Archiv. Oben links steht **Fragen zu: Aufgabentext**,
einzeilig mit Ellipse; rechts oben **Beantwortet**, **Teilweise beantwortet** oder
**Unbeantwortet**. Die zweite Zeile nennt die Anzahl offener und beantworteter
Teilfragen. Konkrete Fragen werden nur im Dialog gezeigt, die Aktionen stehen
rechts unten. Im gemeinsamen Dialog stehen
Formularfelder, Auswahloptionen, Beschreibungen und Berechtigungen. Antworten
werden vorab validiert und anschließend je Frage zugestellt. Das ist keine
atomare externe Sammelantwort: Bestätigung, Fehler und Zustellung bleiben pro
Teilfrage sichtbar. Bereits bestätigte Antworten werden nicht erneut abgeschickt.

Bei geänderter Frage lassen sich der aktuelle Stand und die erhaltenen eigenen
Eingaben ausdrücklich prüfen. Frage- und Aufgabenentwürfe werden privat lokal
gespeichert. Markieren einer Benachrichtigung als gelesen beantwortet keine Frage.

### Sessions und OpenCode

Die gleiche Such-/Tab-Leiste bietet **Offen (Zahl)** / **Geschlossen (Zahl)**,
standardmäßig Offen. Explizit geschlossene Sessions und bestätigte
Runtime-Beendigungen stehen unter Geschlossen. Ein erfolgreicher Auftrag allein
schließt keine wiederverwendbare Session; unbestätigte Beendigungen bleiben
unter Offen sichtbar. Zähler gelten für das Projekt, unabhängig von der Suche.

Die Sessionzeilen nutzen dieselbe flache, rahmenlose Fläche wie Aufgaben:
oben den OpenCode-Titel, darunter „Letzte Aufgabe“ und „Zuletzt aktiv“.
Rechts oben zeigt eine Pill **Geschlossen**, **Offen** oder **Working**;
rechts unten stehen das Phosphor-„…“-Menü und **OpenCode anzeigen**.
Verwaltung, Fragenlinks und technische Metadaten liegen im Menü statt im Lesefluss.
Die letzte Aktivität basiert auf dauerhaften Ausführungs-/Frageänderungen und
zugeordneten Aufgaben, nicht auf Heartbeats oder dem Zeitpunkt des GUI-Abgleichs.
Ältere Server liefern nur die im Snapshot bereits bekannten Aufgaben-/Erstellungszeiten.
Geschlossene oder beendete Sessions werden nicht als verfügbare Ziele neuer
Aufgaben angeboten.

Der Verlauf im OpenCode-Tab ist **schreibgeschützt**:

Die Darstellung folgt der OpenCode-CLI: durchgehender Monospace-Verlauf über die
verfügbare Breite, Markdown mit Hervorhebungen und Syntaxfarben in Codeblöcken,
Nutzernachrichten mit linker Akzentlinie sowie kompakte Toolzeilen mit echtem
Toolnamen und Status. Überlegungen lassen sich aufklappen; Toolblöcke sind immer
offen und beginnen mit `$` statt eines Klapppfeils.
Toolblöcke liegen auf einer dezent grauen Fläche; Aufruf und Ausgabe erscheinen
in Grautönen, ohne Originaldaten-/Menüaktionen. Toolblöcke haben oben/unten
jeweils eine hinterlegte Leerzeile in Monospace-Zeilenhöhe. Ihre linke Kante und
Breite stimmen mit Eingabeblöcken überein. Patches zeigen Zeilennummern,
Syntaxfarben für Code und markierte hinzugefügte/entfernte Zeilen; bei Textdateien
bleibt der Text schlicht, die Änderungsmarkierung bleibt erhalten.
Die Textzeilen haben keinen
zusätzlichen vertikalen Widgetabstand und Nachrichten keine vertikalen Polster:
Zwischen Nachrichten steht genau eine echte Leerzeile. Bei Eingaben liegen oben
und unten je eine hinterlegte Leerzeile; Hintergrund und 6 pt breite blaue Linie
haben exakt dieselben Kanten. Eingaben haben kein Nachrichtenmenü. Fußzeilen mit Agent,
Modell, Zeit und Tokenzahlen entfallen vollständig. Das Menü **Ansicht und Archiv**
entfällt aus der OpenCode-Ansicht. Der Titel bleibt einzeilig mit Ellipse und
vollständigem Tooltip. Vollständiger Archivexport bleibt über das Session-Menü
in der Verwaltung erreichbar. Es wird kein Terminalhintergrund kopiert.

- Nachrichten und vollständige verfügbare Tooldaten aus dem dauerhaften Archiv.
- Automatisches Nachladen und optionales Folgen neuer Ausgaben.
- Zuerst der aktuelle Archivstand, nicht ein minutenlanger Durchlauf ab Nachricht 1.
  Der normale Lesemodus lädt dabei unterschiedliche Nachrichten mit ihrer jeweils
  neuesten Version, statt 32 Zwischenrevisionen desselben Toolaufrufs.
  Ältere Nachrichten werden beim Hochscrollen automatisch nachgeladen.
- Standardmäßig neueste Nachrichtenversion; alle archivierten Revisionen zuschaltbar.
- Aufgaben und Sessionverläufe werden mit variablen Zeilenhöhen virtualisiert;
  große Listen müssen nicht pro Frame vollständig gerendert werden.
- Gemeinsame Historie von Folgeausführungen wird nicht als doppelte Unterhaltung gezeigt.
- Zeit der Quellerfassung, laufende Synchronisierung und fehlende Inhalte sichtbar.
- Sehr lange Texte in verlustfreien, Unicode-sicheren Abschnitten statt stiller Kürzung.
- Vollständiger JSONL-Export unabhängig vom bereits im Fenster geladenen Verlauf.
- Inline-Anhänge und SHA256-geprüfter Export separat gesicherter Archivdateien.

Unter dem Verlauf offener und normal geschlossener Arbeitssessions steht derselbe Prompt-Editor wie bei
**Aufgabe hinzufügen**, einschließlich Bilder-/Datei-Drop, Anhangvorschauen und
Bild-Zwischenablage. **Prompten** erstellt eine dauerhafte Hintergrundaufgabe,
die fest an diese Session gebunden ist. Die Nutzerentscheidung ersetzt die
Orchestrator-Zuweisung; Runner, Modell und MCPs stammen aus der Session und
werden nicht erneut ausgewählt. Laufende Arbeit wird nicht unterbrochen:
Folgeaufgaben warten auf den sicheren Fortsetzungspunkt einschließlich
Archivbestätigung. Bestehende Pausen, Freigaben und Sicherheitsprüfungen gelten
weiterhin. Bei einer normal geschlossenen Session reaktiviert die erfolgreiche
Prompt-Annahme dieselbe Session zusammen mit der neuen Folgeaufgabe in einer
Transaktion. Ungültige Prompts oder die Wiederholung alter Anfragen reaktivieren
nichts. Ausdrücklich beendete Laufzeiten und Controller-Auswertungen haben keinen Editor.

Entwürfe bleiben pro Factory und Session erhalten, auch bei Tabwechsel oder
Neustart. Der Editor leert sich erst nach eindeutiger Serverbestätigung; bei
unklarer Zustellung wird dieselbe gespeicherte Anfrage geprüft statt eine zweite
Aufgabe anzulegen. Endpunkt: `POST /v1/commands/{root}/prompt`.

Die Ansicht ist Runner-/Factory-synchronisiert, **kein direktes Token-Streaming**.
Bereits verlorene, nie erfasste Inhalte kann die GUI nicht rekonstruieren.
Interrupt-Schaltflächen oder direkte Agentsteuerung gibt es im OpenCode-Tab nicht.

### Orchestrator, Runner und Diagnose

Orchestrator und Supervisor haben getrennte Sessionlisten mit Offen-/Archiv-Tabs.
Verteilung, Sessionlimit und Supervisor-Konfiguration samt Historie liegen in den
erweiterten Projekteinstellungen.
Neue Verteilung und bereits laufende Prozesse werden bei Pause nicht gleichgesetzt.

Runner erscheinen in gleich breiten flachen Zeilen wie Aufgaben und Sessions:
Name links, farbige Verbindungspille rechts oben, darunter Standort, offene
Aufträge, letzter Kontakt und ein lesbarer OpenCode-Servicestatus. Laufende
Agentaufträge lassen sich über **OpenCode anzeigen** öffnen. Rohmetadaten,
technische IDs und der SSH-Konfigurationsbutton entfallen aus dieser Liste.
Im Normalbetrieb verbindet sich der Runner zur Factory-API und holt dort seine
Aufträge; weder die GUI noch der Server benötigen dafür SSH zum Runner.
SSH-Profile sind ausschließlich optionale Direktzugänge für native Terminals
oder spezielle Host-Helfer, keine Voraussetzung für die Runner-Steuerung.

Grüne Sidebar-Zahlen basieren auf frischen, vom Runner geprüften OpenCode-Services,
nicht auf alten `accepted`-Aufträgen. Services werden runnerbezogen dedupliziert;
mehrere Sessions können denselben Service nutzen. Nach 30 Sekunden ohne frischen
Nachweis oder bei gestörtem Runner verschwindet der grüne Präsenznachweis. Nach
Serverneustart wird keine persistierte Vermutung als Live-Präsenz wiederhergestellt.

Diagnose zeigt aktuelle Ereignisse automatisch und lädt ältere Historie separat
nach. Projekt-/Textfilter und vollständige Ereignisdaten bleiben verfügbar.
Die Liste verwendet dieselben flachen, gleich breiten Zeilen wie Aufgaben:
Ereignistitel links, Kategorie-Pill rechts oben, Zeitpunkt und Bezug darunter,
**Anzeigen** rechts unten. Die Ereignisdaten öffnen in einem separaten
Detailfenster; das vollständige Original wird über das Drei-Punkte-Menü geladen.

Verwaltungsdialoge decken Profile, Harness-JSON, Scheduler, Tickets/Abhängigkeiten,
Workstreams, Pause/Resume, vorhandene Container-Steuerungswiederholung,
Sessionabschluss, Compaction, Checkpoints, Reviews, Integration, Übergabe, Wissen,
Budgets und Gerätezugang ab. Technische Spezialfelder bleiben als strukturierte
Felder bzw. JSON zugänglich, nicht als Terminal-Ersatz.

Nichtinteraktive Readiness-, Fetch- und Hosting-Helfer laufen bei Bedarf lokal
oder über ein ausdrücklich gewähltes SSH-Profil. Die SSH-Argumentkonstruktion
ist mit der TUI geteilt. Kein stiller Wechsel von SSH zu lokal und keine
Weitergabe von OpenCode-Passwörtern an die GUI. Native Provider-/MCP-Anmeldung
und externe Browserformulare bleiben auf dem jeweiligen OpenCode-Host nötig;
die GUI behauptet keine erfolgreiche Anmeldung, bevor der Readiness-Stand dies zeigt.

### Benachrichtigungen

Die globale Schaltfläche zeigt ungelesene Meldungen und offene Fragegruppen
aller Projekte. Jede Meldung zeigt Projekt und Zeitpunkt auf zwei Zeilen.
Die Zielaktion benennt ihr Ziel, etwa **Zur Aufgabe** für den normalen
Aufgabendialog. Der Haken und das Öffnen des Ziels markieren die Meldung direkt
als gelesen. Lesestatus gilt je Geräteidentität; der Bootstrap-Zugang hat einen
gemeinsamen Lesestatus.

Ein eigener Einstellungsbereich für Desktop-Meldungen wird nicht mehr angeboten.
Die vorhandene ntfy-Outbox bleibt ein separater optionaler Hintergrundkanal.

## Verbindung und unklare Zustellung

Snapshot, SSE und regelmäßiger Abgleich halten die Ansichten aktuell. Dabei
werden binäre Anhangdaten nicht bei jedem Live-Abgleich erneut übertragen:
Desktop-Snapshot und SSE verwenden Metadaten, Originalanhänge werden bei Bedarf
separat geladen. In Diagnoseereignissen ist die ausgelagerte Nutzlast markiert;
das vollständige Originalereignis bleibt abrufbar. Bei Ausfall
wechselt die Oberfläche automatisch in den reduzierten Offline-Modus.
Server-Schreibaktionen sind dann gesperrt. Die App verbindet automatisch neu.

### Aufgaben offline vorbereiten

Ohne Serververbindung bleibt nur **Aufgaben**, **+ Aufgabe** und die gespeicherte
Projektauswahl aktiv. Alle anderen Menüpunkte, das Projektmenü, Session-Tabs und
die Factory-Einstellungen sind ausgegraut. Beim Verbindungsabbruch wird eine
andere Ansicht verlassen; offene Eingaben bleiben gespeichert. Die Verbindung
kann weiterhin über die Verbindungsübersicht im Footer eingerichtet werden.
Mittig unten steht orange:
**Offline-Modus: Aufgaben können nur lokal vorbereitet werden.**

Die reduzierte Liste zeigt lokal vorbereitete Aufgaben des gewählten Projekts.
Text und Anhänge werden dauerhaft lokal gespeichert; Aufgaben lassen sich
bearbeiten oder nach Bestätigung entfernen. Ohne zuvor gespeicherte Projekte
muss zunächst einmal eine Verbindung hergestellt werden.

Nach Wiederverbindung werden vorbereitete Aufgaben automatisch ausschließlich
an ihre ursprüngliche Factory und ihr ursprüngliches Projekt als **Server-Entwürfe**
übertragen. Sie starten keine Ausführung; die Freigabe erfolgt anschließend
explizit über **Einreihen**. Bei unklarer Zustellung wird dieselbe gespeicherte
Anfrage abgeglichen, nicht eine zweite Aufgabe erzeugt. Erst nach bestätigter
Übertragung verschwindet die lokale Kopie. Fehlende Projekte oder abgelehnte
Übertragungen lassen die Aufgabe lokal erhalten; technische Fehlerdetails stehen
in der Diagnose. Noch offene lokale Überarbeitungen werden erst nach erneutem
**Lokal vorbereiten** übertragen.

Schreibabsichten werden **vor HTTP** in einem privaten lokalen Journal gespeichert.
Bei unklarem Ergebnis bleiben Request-ID und Nutzlast unverändert; die Oberfläche
bietet in **Diagnose → Zustellungen prüfen** den identischen idempotenten
Abgleich an. Dieser technische Abgleich ist kein Bereich der Factory-Einstellungen.
Einladungserstellung ist davon ausgenommen: ihr API fehlt eine idempotente
Request-ID, deshalb wird ein unklarer Aufruf nicht einfach wiederholt.

Revisionkonflikte bewahren Eingaben. Vergleich und Übernahme der aktuellen
Revision erfolgen ausdrücklich im Dialog, nicht durch blindes Überschreiben.
Gespeicherte Verwaltungsdialog-Entwürfe sind über Desktop-Einstellungen wieder
zugänglich. Einstellungen/Entwürfe liegen privat unter
`$XDG_DATA_HOME/yoloforge/desktop.json` (Dateimodus 0600, Verzeichnis 0700).

## Server-/Runner-Update und Linux-Paket

Server-Schema 32 ergänzt Benachrichtigungen und Lesebelege; Schema 33 ergänzt
persistente Aufgaben-Bearbeitungsreservierungen. Vor einem Serverupdate wie
üblich sichern; ältere Serverbinaries können Schema 33 nicht öffnen.
Server und Runner für Dateiaufgaben gemeinsam aktualisieren. Die GUI fällt bei
fehlenden neuen Endpunkten nicht unbemerkt auf sofortige Aufgabenfreigabe oder
geschätzte Instanzzahlen zurück.

GUI-Releasepakete enthalten GUI, Server, Runner, Desktop-Eintrag und Icon. Das
Paket in ein Benutzerverzeichnis entpacken und dieses Verzeichnis zum `PATH`
hinzufügen. `yoloforge.desktop` kann nach `~/.local/share/applications/`, das SVG
nach `~/.local/share/icons/hicolor/scalable/apps/yoloforge.svg` kopiert werden.
Die drei Binaries müssen für lokale Starts zusammenbleiben.

Linux benötigt eine X11- oder Wayland-Desktopumgebung und einen funktionierenden
OpenGL-3.3/EGL-Treiber. SDL3 wird aus fest versionierten Quellen statisch mitgebaut;
eine systemweit installierte SDL3-Bibliothek ist nicht nötig. Für den Build nutzt
CI CMake, Wayland-Protokolle und die Entwicklungsdateien für xkbcommon, Wayland,
X11 und OpenGL/EGL (Paketliste in `.github/workflows/ci.yml`). Light Theme und Windows-/macOS-Auslieferung
gehören nicht zu dieser Version.

## Entwicklungsprüfungen

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke-gui.py
# Optional: isolierte API-Fixtures plus Screenshot nur des eigenen nativen Fensters
python3 scripts/smoke-gui.py --screenshots /absoluter/neuer/ausgabepfad
# X11-Fallback in CI, ohne eine vorhandene Wayland-Sitzung anzusprechen:
# env -u WAYLAND_DISPLAY xvfb-run -a python3 scripts/smoke-gui.py --screenshots target/gui-smoke
# Natives Wayland + echte GTK-Datei-Drops, ohne Factory-Dienste:
# benötigt Sway, grim, wtype, C-Compiler, wayland-scanner und GTK3-PyGObject
python3 scripts/smoke-gui-wayland.py
python3 scripts/smoke-gui-wayland.py --uri-only
# Optional zusätzlich: echte Thunar-Instanz mit eigenem Test-DBus/Benutzerverzeichnis
python3 scripts/smoke-gui-wayland.py --thunar
python3 scripts/smoke-gui-wayland.py --covered-clipboard
```

Die Prüfungen starten einen eigenen kurzlebigen Server mit synthetischen
Runner-/Sessiondaten, keine Modelle und keine Produktionsaufträge. Vorhandene
Screenshot-Belege werden nicht überschrieben. Weitere Planung und Zuordnung:
[GUI-Plan](gui-plan.md), [Funktionsparität](gui-parity.md).

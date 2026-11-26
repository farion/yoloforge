# Android-Client

Native Java-App unter `android/`, Android 8/API 26 oder neuer. Enthalten:
Anmeldung mit Factory-Adresse und Zugangsschlüssel wie am Desktop, Projekt-/Aufgabenansicht, Prompt mit
bis zu vier Bildern (zusammen 4 MiB), feste Sessionwahl, Pause/Fortsetzen,
Sessionauszug, paginiertes Sessionarchiv mit Dateiexport und Frage-/Berechtigungsantworten.

Das Flyout gliedert sich wie am Desktop in **Projekt** und **Factory**.
Die Projektauswahl öffnet sich über der Navigation, ohne sie zu verschieben.
Das Drei-Punkte-Menü daneben öffnet das Projektmenü zum Anlegen,
Pausieren/Fortsetzen und Löschen von Projekten. Beim Löschen muss der vollständige
Projektname bestätigt werden; Repository-Dateien und Historie bleiben erhalten.
Unter der Auswahl stehen Projektlogo und **+ Intent**, gefolgt von Intents, Inbox,
Sessions und Projektaktivität. Der Factory-Block enthält Overview, den
Benachrichtigungsverlauf, Infrastructure und Diagnose. Lokale Android-Push-
Einstellungen stehen am Ende des Navigationsmenüs.

Ein bereits gültiger Zugangsschlüssel funktioniert auch auf Android. Für einen
separat widerrufbaren Zugang empfiehlt sich ein eigener
[Operator-Schlüssel](deployment.md#zugangsschlüssel-und-rollen).

## Systemdialog-Abnahme im Emulator

`scripts/smoke-android-device.py --serial emulator-5580` prueft mit dem
installierten APK und einem isolierten echten Rust-Server Anmeldung, Bildauswahl
ueber DocumentsUI, unveraenderten Bild-Upload sowie SAF-Dateiexport mit
bytegenauem Vergleich. Die Abnahme setzt die App-Daten auf dem ausdruecklich
ausgewaehlten Emulator zurueck; nicht fuer ein persoenliches Geraet verwenden.
Bild- und Exportpfad sind bestanden. Dies ersetzt keine reale Geraete-/Doze-Abnahme.

Mit `--upgrade-to 0.1.2` wird zusaetzlich der eingebaute GitHub-Updater aus einer
aelteren installierten Version gestartet. Der Test erlaubt die angebotene
Play-Protect-Pruefung, wartet auf die tatsaechlich installierte neuere Version
und prueft anschliessend den Zugriff mit der erhaltenen Factory-Anmeldung.
Diese Upgrade-Abnahme von `0.1.2-dev` auf das veroeffentlichte `0.1.2` ist
bestanden, einschliesslich Play-Protect-Pruefung und Android-Installer.
Beleg fuer Bildimport, SAF-Export und Upgrade:
`/tmp/opencode/yf-android-device-tb7yossq`.

Zugangsdaten und der einzelne ausstehende Schreibauftrag werden mit einem
Android-Keystore-Schluessel AES-GCM-verschluesselt gespeichert. Ein unsicherer
Schreibauftrag behält Factory-Adresse, Payload und Request-ID auch beim Neustart.
HTTPS ist erforderlich; HTTP ist ausschließlich fuer localhost/127.0.0.1 und
den Emulator-Host 10.0.2.2 freigegeben. Keine Zugangsdaten in URLs.

## Bauen und pruefen

JDK 17, Android SDK Platform 36 und Build Tools 36.0.0 bereitstellen:

```sh
cd android
./gradlew :app:assembleDebug :app:lintDebug
./gradlew :app:connectedDebugAndroidTest
```

Debug-APK: `android/app/build/outputs/apk/debug/app-debug.apk`. CI baut und
prüft dieses APK mit Lint und lädt es als `yoloforge-android-development` hoch.
Für lokal signierte Release-Builds siehe unten.
Gradle Wrapper 9.4.1 ist mit Distributions-SHA256 gepinnt.

Lokal bestanden: Build, Lint und Instrumentierungstests im isolierten
Android-17-Emulator: Keystore/Endpoint-Regeln, identischer Replay nach verlorener
HTTP-Antwort, Verhinderung eines Factory-Wechsels fuer ausstehende Aufträge sowie
Wiederherstellung eines abgelehnten Prompts als verschluesselter Entwurf.
Zusaetzlich bestanden: echte Android-UI gegen einen isolierten Rust-Server:
Anmeldung mit Zugangsschlüssel, Fragenbenachrichtigung, Formularantwort, Promptanlage und Erhalt einer
festen Sessionwahl beim Speichern/Wiedereroeffnen sowie nach Activity-Neustart,
Archivnavigation und Anhangangebot. Weitere Vertragstests pruefen Binärintegrität,
bedingte Mehrfachauswahl, Integer-Typen und Update-Transportregeln. Ausfuehren mit
`python3 scripts/smoke-android.py --serial EMULATOR_SERIAL` nach dem APK-/Test-APK-
Build; nur einen dedizierten Testemulator verwenden (App-Testdaten werden benutzt).
`--sign-in-only` prüft die Anmeldung mit gültigem und ungültigem Schlüssel ohne
Runner- oder Archiv-Testdaten.
`--project-management` prüft die überlagernde Projektauswahl sowie das Anlegen
und Löschen eines Testprojekts.
Der Test startet keinen Runner und fuehrt keine Modellaufrufe aus.
Instrumentierung und API/UI-Test werden lokal im Emulator ausgeführt.

Promptentwuerfe inklusive Bildern und fester Sessionwahl werden beim Verlassen
des Editors bzw. beim Hintergrundwechsel im verschluesselten Vault gespeichert,
getrennt je Factory und Projekt. Nach einer endgueltigen HTTP-Ablehnung wird der
Prompt wieder als Entwurf bereitgestellt. Nicht mehr aufgelistete feste Sessions
werden im Entwurf bewahrt und nicht automatisch umgebunden.
Nach Rotation/Activity-Neustart werden Projekt und geoeffneter Prompteditor
wiederhergestellt. Der Bundle enthaelt nur Navigation; Prompt/Bilder bleiben im
verschluesselten Vault. Antwortentwuerfe sind nach Factory, Frage und Revision
getrennt und werden beim Wiedereroeffnen oder nach Rotation geladen.

Formulare unterstuetzen beschriftete Auswahl mit Beschreibungen, Mehrfachauswahl,
eigene Werte, Boolean, Integer, Number sowie `eq`/`neq`-Bedingungen und versteckte
Felder. Inaktive Felder werden nicht gesendet. Pflichtfelder, Laengen, Wertebereiche
und Auswahlanzahl werden lokal geprueft; die API bleibt massgeblich. Externe
Anmeldefelder werden weiterhin in OpenCode beantwortet.

Der Archivbrowser haelt die Revision beim Blaettern fest. Lokale/inline
Binaersnapshots werden vor dem Export mit SHA256 geprueft; der Android-Dateidialog
bestimmt das Ausgabeziel. Fremde Dateinamen werden nicht als Pfade verwendet.
Ein ausstehender Dateiexport wird verschluesselt gespeichert und uebersteht Rotation.

## Benachrichtigungen und Grenzen

Die Android-App ist bei einer Factory gleichzeitig angemeldet. Die Inbox ist
über das Flyout erreichbar; Antippen einer Meldung öffnet die Rückfrage,
Berechtigungsanfrage oder das Problem direkt. Wegwischen bearbeitet den
Inbox-Eintrag nicht. Automatisch bearbeitete Rückfragen, Starts und erfolgreiche
Abschlüsse lösen keine Android-Push-Meldung aus.

**Google-Push ohne Zusatz-App einrichten:**

1. In der [Firebase-Konsole](https://console.firebase.google.com/) ein Projekt
   erstellen und darin eine Android-App mit Paketname `dev.yoloforge.mobile`
   registrieren. `google-services.json` herunterladen.
2. Die Firebase Cloud Messaging API (HTTP v1) aktivieren. Ein Dienstkonto im
   selben Projekt mit Sendeberechtigung (`cloudmessaging.messages.create`,
   z. B. über `roles/firebasecloudmessaging.admin`) einrichten und einen
   Schlüssel im JSON-Format erstellen.
3. Am Desktop als Factory-Admin **Factory-Einstellungen → Geräte und Zugänge →
   Android-Push** öffnen. Beide JSON-Dateien auswählen oder ihren Inhalt einfügen, den Anzeigenamen der
   Factory festlegen, aktivieren und **Speichern**. Zum Ersetzen des Schlüssels
   nur die neue Dienstkonto-JSON einfügen; leere Felder behalten ihren Wert.
4. Auf einem Android-Gerät mit Google Play-Diensten anmelden und im Flyout
   unter **Benachrichtigungen** den Inbox-Empfang einschalten. Kein Google-Login
   in YoloForge und keine eigene APK pro Factory nötig.

Der private Schlüssel wird nicht an Android ausgeliefert und nicht in der
Desktop-Anfragehistorie abgelegt. Er liegt in der Datei
`<datenbank>.android-push.json` neben der Datenbank (unter Unix mit
Benutzerzugriff, Standardrechte `0600`). Die Datei und Backups privat halten;
den Schlüssel bei einer Rotation auch in Google Cloud widerrufen. Die App lädt
nur die öffentliche Firebase-Konfiguration. Google erhält die Geräteadresse,
den Factory-Anzeigenamen und undurchsichtige IDs, aber keine Prompts,
Fragetexte, Factory-Adresse oder Factory-Zugangsschlüssel. Die eigentlichen
Inbox-Daten lädt YoloForge nach dem Öffnen direkt von der Factory.

Ohne FCM bietet die App weiterhin den direkten Empfang von Rückfragen per
Liveverbindung; für ältere Setups bleibt die optionale [ntfy-Begleitapp](background-push.md).
Der separate, optionale Agentenstatus nutzt eine laufende Factory-Verbindung
und meldet bei Netzverlust eine unbekannte statt einer veralteten Anzahl.
Android begrenzt die Laufzeit dieses `dataSync`-Foreground-Service. Google-Push
ist kein Ersatz für die Inbox; Zustellung nach erzwungenem App-Stopp oder ohne
Google Play-Dienste ist nicht zugesichert.

## APK-Updates

`App-Updates` prueft das neueste stabile GitHub-Release. Nach Auswahl werden
APK und SHA256-Datei nur von den vorgesehenen GitHub-Hosts geladen. Pruefsumme,
Paketname, hoehere Version sowie der gepinnte Release-Schluessel und die Signatur
der installierten App muessen passen. Erst dann startet die Android-Systeminstallation;
ihre Berechtigung und Bestaetigung bleiben beim Nutzer. Kein stilles Installieren.
Factory-Zugangsdaten werden niemals an GitHub weitergegeben. Debug-Installationen
koennen nicht mit dem anders signierten Release-APK aktualisiert werden.
Die reale Updateinstallation wartet auf ein neueres veroeffentlichtes Android-Asset.
Lokal zusaetzlich geprueft: Installation des stabil signierten Release-APK,
erneute Installation mit demselben Schluessel und Aufruf des Fragen-Deep-Links
bis zur Anmeldung im dedizierten Emulator.

## Stabile Release-Signierung

Auf ausdrücklichen Wunsch wurde ein dauerhafter RSA-4096-Release-Schlüssel lokal
angelegt: `$XDG_DATA_HOME/yoloforge/signing/android`, standardmäßig
`~/.local/share/yoloforge/signing/android`. Das private Verzeichnis enthält
`release.p12`, Passwortdatei, `signing.json` und Sicherungshinweise. Für spätere
App-Updates muss dieses gesamte Verzeichnis erhalten und sicher gesichert werden.
Private Schlüssel und Kennwörter gehören nicht ins Repository; dort liegt nur
der öffentliche Zertifikatsfingerprint in `android/release-certificate.sha256`.

```sh
python3 scripts/android-release.py
```

Der Helfer baut mit JDK 17, führt Release-Lint und `apksigner verify` aus und
vergleicht den Signer mit dem gepinnten Fingerprint. Ausgabe:
`dist/yoloforge-android-vVERSION.apk` plus SHA256-Datei. Version und monotoner
Android-VersionCode werden aus der Cargo-Workspace-Version abgeleitet.
Vorhandene abweichende Ausgabeartefakte werden nicht überschrieben.
`--init-signing` ist nur für die einmalige Einrichtung eines bislang nicht
vorhandenen Schlüsselverzeichnisses gedacht; es ersetzt niemals einen Schlüssel.
Der Release-Workflow besitzt einen opt-in Android-Signierjob: Repositoryvariable
`ANDROID_RELEASE_SIGNING=enabled` sowie Secrets `ANDROID_KEYSTORE_BASE64`,
`ANDROID_STORE_PASSWORD` und `ANDROID_KEY_ALIAS` (PKCS12, gleicher Key-/Store-Passwortwert).
Er prueft den gepinnten Zertifikatsfingerprint, laedt ausschliesslich APK/Pruefsumme
als Release-Artefakte hoch und entfernt die temporaeren Schluesseldateien.
Bei aktiviertem Job blockiert ein Signierfehler die Gesamtveroeffentlichung.
Secrets und Variable wurden nach ausdruecklicher Freigabe fuer
`farion/yoloforge` eingerichtet; der lokale Schluessel bleibt fuer Sicherung und
lokale Builds erhalten. Der eigenstaendige CI-Signierlauf
`45036459858` hat den signierten Entwicklungsbuild erfolgreich erzeugt.
Der versionierte Releasepfad wurde mit v0.1.2 geprueft (CI `45042304226`):
veroeffentlichtes APK heruntergeladen, SHA256 mit Release-Pruefsumme verglichen
und Signatur mit `apksigner verify --print-certs` bestaetigt. Der Signer entspricht
weiterhin `android/release-certificate.sha256`.
Ohne Aktivierung bleiben die anderen Release-Ziele nutzbar.

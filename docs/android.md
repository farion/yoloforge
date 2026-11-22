# Android-Client (Entwicklungsstand)

Native Java-App unter `android/`, Android 8/API 26 oder neuer. Enthalten:
Enrollment per einmaligem Operator-Code, Projekt-/Aufgabenansicht, Prompt mit
bis zu vier Bildern (zusammen 4 MiB), feste Sessionwahl, Pause/Fortsetzen,
Sessionauszug, paginiertes Sessionarchiv mit Dateiexport und Frage-/Berechtigungsantworten.

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

Debug-APK: `android/app/build/outputs/apk/debug/app-debug.apk`. CI baut dieses
APK als Entwicklungsartefakt. Für lokal signierte Release-Builds siehe unten.
Gradle Wrapper 9.4.1 ist mit Distributions-SHA256 gepinnt.

Lokal bestanden: Build, Lint und Instrumentierungstests im isolierten
Android-17-Emulator: Keystore/Endpoint-Regeln, identischer Replay nach verlorener
HTTP-Antwort, Verhinderung eines Factory-Wechsels fuer ausstehende Aufträge sowie
Wiederherstellung eines abgelehnten Prompts als verschluesselter Entwurf.
Zusaetzlich bestanden: echte Android-UI gegen einen isolierten Rust-Server:
Enrollment, Fragenbenachrichtigung, Formularantwort, Promptanlage und Erhalt einer
festen Sessionwahl beim Speichern/Wiedereroeffnen sowie nach Activity-Neustart,
Archivnavigation und Anhangangebot. Weitere Vertragstests pruefen Binärintegrität,
bedingte Mehrfachauswahl, Integer-Typen und Update-Transportregeln. Ausfuehren mit
`python3 scripts/smoke-android.py --serial EMULATOR_SERIAL` nach dem APK-/Test-APK-
Build; nur einen dedizierten Testemulator verwenden (App-Testdaten werden benutzt).
Der Test startet keinen Runner und fuehrt keine Modellaufrufe aus. CI fuehrt
Instrumentierung und diesen API/UI-Test ebenfalls auf einem Emulator aus.

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

Explizit einschaltbarer Foreground-Service empfängt Factory-SSE direkt, meldet
offene Fragen per Android-Notification und öffnet die Frage beim Antippen.
Reconnect liest einen Snapshot samt Eventcursor; Frage-ID/Revision deduplizieren
Benachrichtigungen. Texte auf dem Sperrbildschirm enthalten keinen Prompt.
Android begrenzt die Laufzeit des `dataSync`-Foreground-Service; bei Timeout wird
er beendet. Fuer Push bei geschlossener App gibt es jetzt optional eine
**ntfy-Begleitapp** mit dauerhafter Server-Outbox, Wiederholung nach Fehlern und
Deep-Link auf die aktuellen Fragen. Einrichtung: [background-push.md](background-push.md).
YoloForge selbst enthaelt keinen FCM-/UnifiedPush-Connector. Doze, Netzwechsel,
ntfy-Zustellung auf einem echten Geraet und Android-Dateidialog-Abnahme bleiben offen.

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
lokale Builds erhalten. Die CI-Ausfuehrung dieses Signierpfads muss beim naechsten
Release noch bestaetigt werden. Ohne Aktivierung bleiben die anderen Release-Ziele nutzbar.

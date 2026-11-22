# Android-Client (Entwicklungsstand)

Native Java-App unter `android/`, Android 8/API 26 oder neuer. Enthalten:
Enrollment per einmaligem Operator-Code, Projekt-/Aufgabenansicht, Prompt mit
bis zu vier Bildern (zusammen 4 MiB), feste Sessionwahl, Pause/Fortsetzen,
Sessionauszug und einfache Frage-/Berechtigungsantworten.

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

Lokal bestanden: Build, Lint und vier Instrumentierungstests im isolierten
Android-17-Emulator: Keystore/Endpoint-Regeln, identischer Replay nach verlorener
HTTP-Antwort, Verhinderung eines Factory-Wechsels fuer ausstehende Aufträge sowie
Wiederherstellung eines abgelehnten Prompts als verschluesselter Entwurf.
Zusaetzlich bestanden: echte Android-UI gegen einen isolierten Rust-Server:
Enrollment, Fragenbenachrichtigung, Formularantwort, Promptanlage und Erhalt einer
festen Sessionwahl beim Speichern/Wiedereroeffnen eines Entwurfs. Ausfuehren mit
`python3 scripts/smoke-android.py --serial EMULATOR_SERIAL` nach dem APK-/Test-APK-
Build; nur einen dedizierten Testemulator verwenden (App-Testdaten werden benutzt).
Der Test startet keinen Runner und fuehrt keine Modellaufrufe aus. CI fuehrt
Instrumentierung und diesen API/UI-Test ebenfalls auf einem Emulator aus.

Promptentwuerfe inklusive Bildern und fester Sessionwahl werden beim Verlassen
des Editors bzw. beim Hintergrundwechsel im verschluesselten Vault gespeichert,
getrennt je Factory und Projekt. Nach einer endgueltigen HTTP-Ablehnung wird der
Prompt wieder als Entwurf bereitgestellt. Nicht mehr aufgelistete feste Sessions
werden im Entwurf bewahrt und nicht automatisch umgebunden.

## Benachrichtigungen und Grenzen

Explizit einschaltbarer Foreground-Service empfängt Factory-SSE direkt, meldet
offene Fragen per Android-Notification und öffnet die Frage beim Antippen.
Reconnect liest einen Snapshot samt Eventcursor; Frage-ID/Revision deduplizieren
Benachrichtigungen. Texte auf dem Sperrbildschirm enthalten keinen Prompt.
Es gibt keinen FCM-/UnifiedPush-Dienst. Android begrenzt die Laufzeit des
`dataSync`-Foreground-Service; bei Timeout wird er beendet. Das ist **noch kein
dauerhafter Push bei geschlossener App**. Doze, Netzwechsel, Widerruf, Deep-Link-
Rennen und echtes Gerät sind noch abzunehmen.

Erweiterte/bedingte Frageformulare verweisen auf den Desktop. Vollständiger
Archivbrowser, automatische Wiedereroeffnung des Editors nach Rotation und
automatische APK-Updates bleiben offen.

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
CI-Signierung/automatische Veröffentlichung des APK ist noch nicht eingerichtet;
die privaten Schlüsseldaten wurden nicht auf GitHub hochgeladen.

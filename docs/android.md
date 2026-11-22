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

APK: `android/app/build/outputs/apk/debug/app-debug.apk` (Debugsignatur,
kein stabil signiertes Release). CI baut das APK als Entwicklungsartefakt.
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
Archivbrowser, automatische Wiedereroeffnung des Editors nach Rotation, stabile Release-Signierung und
automatische APK-Updates bleiben offen.

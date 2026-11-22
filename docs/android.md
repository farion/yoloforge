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

Lokal bestanden: Build, Lint und drei Instrumentierungstests im isolierten
Android-17-Emulator: Keystore/Endpoint-Regeln, identischer Replay nach verlorener
HTTP-Antwort, Verhinderung eines Factory-Wechsels fuer ausstehende Aufträge.
Rust-Server-Vertrag und vollständige mobile UI-End-to-End-Abnahme stehen noch aus.

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
Archivbrowser, Entwurfserhalt bei Rotation, stabile Release-Signierung und
automatische APK-Updates bleiben offen.

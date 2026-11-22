# Hintergrund-Push mit ntfy

Factory kann offene Fragen ueber einen explizit konfigurierten ntfy-Server
melden. Die ntfy-Android-App empfaengt und speichert die Benachrichtigung;
YoloForge braucht dafuer keinen laufenden SSE-Service. Beim Antippen oeffnet
`yoloforge://questions` die angemeldete App und laedt den aktuellen Factory-Stand.
Push selbst fuehrt keine Antworten oder anderen Schreibaktionen aus.

## Einrichtung

1. Auf einem eigenen oder ausgewaehlten ntfy-Dienst ein privates/unerratbares
   Topic bereitstellen und in der ntfy-App abonnieren. Die YoloForge-Seite
   `Hintergrund-Push (ntfy)` oeffnet die Topic-Adresse in ntfy.
2. Auf dem Factory-Host eine private JSON-Konfiguration ablegen:

```json
{
  "endpoint": "https://notify.example/yoloforge-PRIVATE-TOPIC",
  "token_file": "/etc/yoloforge/ntfy.token"
}
```

`token_file` ist optional; falls gesetzt, enthaelt die Datei nur den ntfy-Bearer-
Token, niemals einen Factory-Token. Der Dienstbenutzer muss die Datei lesen koennen.
HTTPS ist vorgeschrieben, ausser fuer Loopback-Prozesstests. HTTP-Redirects werden
nicht verfolgt. ntfy-URL/Token werden nicht in API-Events oder Fehlermeldungen ausgegeben.

3. Factory mit `--push-config /etc/yoloforge/push.json` starten. Ohne diese Option
   entstehen keine Push-Netzwerkzugriffe. Das Deployment bleibt beim Betreiber.
4. Zum Abschalten die Option entfernen oder das Topic in ntfy abbestellen.

## Dauerhafte Zustellung

Schema 29 speichert Ziel-Fingerprint, bestaetigten Eventcursor, ausstehenden
Cursor, Versuche und naechste Retry-Zeit in SQLite. Mehrere offene Fragen werden
zu einem Hinweis zusammengefasst. Vor dem HTTP-Aufruf wird die Wiederholungsfrist
gespeichert; Absturz/Neustart setzen sie nicht zurueck. Die Wartezeit steigt von
30 Sekunden bis 15 Minuten. Bei einer unklaren Antwort darf ein Hinweis doppelt
erscheinen; es werden keine fachlichen Auftraege wiederholt. Inzwischen beantwortete
Fragen werden nicht weiter gemeldet. Ein geaendertes konfiguriertes Ziel beginnt
einen eigenen Zustellstand; spaete Bestaetigungen des alten Ziels sind wirkungslos.

Payload: nur ein allgemeiner Rueckfragenhinweis mit App-Link. Keine Frage-ID,
Prompts, Antwortwerte, Bilder, Projektpfade oder Factory-Credentials. Abnahme:
`python3 scripts/smoke-push.py` prueft echte HTTP-Ereignisse, verlorene Provider-
Antwort, Serverneustart, erhaltenen Retry-Abstand und anschliessende Bestaetigung.

Eine bestaetigte ntfy-Annahme ist kein Nachweis der Anzeige auf einem konkreten
Telefon. Offline-Aufbewahrung, Doze und Systemenergiesparen haengen von ntfy-
Konfiguration und Geraet ab; diese reale Zielgeraet-Abnahme steht noch aus.
Die ntfy-App verwaltet ihr Topic-Abonnement unabhaengig vom Factory-Geraetetoken.
Ein widerrufener Factory-Zugang kann keine Fragen laden; zum Entzug auch der
allgemeinen Hinweise das Topic-Abonnement bzw. dessen Zugang entfernen/rotieren.

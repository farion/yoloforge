# Recovery einer ungeklärten Container-Steuerung

Entwicklungsstand nach v0.1.1, Server-Schema 14; Runner-Journal weiterhin Schema 3.

`yoloforge stream retry-control` ermöglicht einen **expliziten neuen Versuch**
derselben Freeze-/Thaw-Aktion. Typischer Fall: Ein Auftau-Intent wurde vor einem
Runner-Absturz gespeichert, der Docker-Aufruf kam aber noch nicht zustande.
Der automatische lesende Abgleich sieht weiterhin einen eingefrorenen Container
und kann den alten Request deshalb nicht als ausgeführt bestätigen.

## Voraussetzungen und Wirkung

- Der bisherige Control-Request ist `interrupt_unverified`, gehört zu einem
  weiterhin akzeptierten Agentenauftrag und besitzt einen zuvor vom Runner
  gespeicherten Container-Nachweis.
- Der Operator benennt Streamrevision, vorherige Request-ID, exakt gespeicherten
  Nachweis und eine Begründung. Nachweise aus einer selbst erstellten Datei können
  fehlende Runner-Evidenz nicht ersetzen.
- Der neue Request behält Aktion, Command-ID, Container-Identität und Ownership
  bei. Er wird `requested` bzw. `resume_requested`; er gibt den Stream nicht frei.
- Der Runner prüft weiterhin seine lokale Bindung und aktuelle Container-Startzeit.
  Pro neuer Request-ID darf er höchstens einen Docker-Schreibversuch durchführen.
  Ist die gewünschte Lage bereits eingetreten, genügt lesender Abgleich.
- Alte Requests/Receipts bleiben erhalten. `retry.previous_request` und
  `retry.reason` sowie das Ereignis `stream.control.retry` dokumentieren die
  Entscheidung; wiederholte Recovery-Versuche bilden darüber eine Historie.
  Eine nachfolgende gewöhnliche Pause/Resume beginnt ohne Retry-Verweis.

Dieser Weg ersetzt keinen automatischen Retry: Der Operator entscheidet nach
Prüfung der unklaren Situation über einen neuen Versuch derselben Aktion.
Insbesondere müssen eventuell noch ausstehende Docker-Operationen berücksichtigt
werden; der gespeicherte Nachweis ist eine vergangene Beobachtung. Es werden keine
Prompts erneut gesendet, Sessions neu angelegt, Journalreceipts gelöscht oder
Ownership-Epochen verändert.

## CLI

```sh
umask 077
STREAM=STREAM-UUID
yoloforge --snapshot > snapshot.json
jq --arg id "$STREAM" '.controls[] | select(.stream_id == $id) | .proof' \
  snapshot.json > proof.json
REV=$(jq -r --arg id "$STREAM" '.streams[] | select(.id == $id) | .revision' snapshot.json)
PREVIOUS=$(jq -r --arg id "$STREAM" '.controls[] | select(.stream_id == $id) | .request_id' snapshot.json)

yoloforge stream retry-control "$STREAM" --expected-revision "$REV" \
  --previous-request "$PREVIOUS" --proof-file proof.json \
  --reason 'Dedizierte Laufzeit geprüft; ursprünglichen Auftauvorgang erneut anfordern' \
  --request-id NEUE-REQUEST-UUID
```

Nach einer verlorenen API-Antwort denselben Aufruf mit **identischer Request-ID
und identischem Payload** wiederholen. Er liefert das ursprüngliche Ergebnis und
erzeugt keinen weiteren Versuch. Für den aktuellen Zustand den Snapshot erneut
lesen. Konkurrierende Operatoren können nicht beide dieselbe Revision ablösen;
ein neuerer Report oder anderer Nachweis verhindert eine veraltete Entscheidung.

API: `POST /v1/streams/{id}/control/retry` mit `RetryControl`:
`request_id`, `expected_revision`, `previous_request`, `expected_proof`, `reason`.
Zugriff für Operator/Admin, nicht für Runner-Credentials. Anforderung, neue
Control, Revision, Ereignisse und deduplizierte Antwort werden atomar gespeichert.

Der aktualisierte Runner toleriert einen verspäteten Report für einen inzwischen
abgelösten Request: Er liest die neue Control und verarbeitet sie im nächsten
Durchlauf. Er überschreibt sie nicht mit dem alten Ergebnis und beendet sich
nicht wegen dieses erwartbaren Revisionsrennens.

## Abnahme und offene Recovery-Fälle

- Store-Tests: zwei konkurrierende DB-Verbindungen, Neustart/Replays, veraltete
  Reports, geänderte Evidenz und atomarer Rollback bei fehlgeschriebenem Event.
- HTTP-Test: Authentifizierung, API-Roundtrip und Deduplizierung.
- Runner-Test: Request wird während des Reports ersetzt, kein fataler Abbruch.
- Reales Docker-Labor: persistierter, noch nicht ausgeführter Thaw-Intent bleibt
  zunächst unklar; ein expliziter neuer Request taut dieselbe Instanz auf und
  erhält den alten Receipt. Aufruf siehe [container-runtime.md](container-runtime.md).

Weiterhin offen: Recovery ohne bisherigen Container-Nachweis, verschwundene oder
neu gestartete Container, verlorene Journale, unklare native Interrupts/Prompts,
abschließendes Stilllegen solcher Aufträge und Übertragung an einen anderen
Runner. Die interaktive TUI besitzt noch keinen eigenen Recovery-Dialog.

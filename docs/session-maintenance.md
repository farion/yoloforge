# Autonome Sessionpflege durch den Operator-Agenten

## Implementierter Mechanismus (Entwicklungsstand nach v0.1.1)

Native Compaction ist über API/CLI und den owning Runner ausführbar, mit
persistenten Aufträgen in Schema 9. Die automatische Entscheidung des Supervisors
und Sessionbeendigung sind weiterhin offen.

```sh
yoloforge compact AUFTRAG-UUID --expected-stream-revision REVISION \
  --reason 'Veraltete Toolausgaben verdichten' --request-id REQUEST-UUID
yoloforge compaction AUFTRAG-UUID
```

Nur aktive, bestätigte Agentenaufträge ohne Pause oder bereits fertigen
Archivdurchlauf können neue Anforderungen erhalten. Pro Auftrag darf höchstens
eine Compaction offen sein. Die Request-ID schützt gegen doppelte Anforderungen;
die Streamrevision gegen konkurrierende Änderungen. Der Grund wird unveränderlich
gespeichert. Die Session und ihre Nachrichten werden dabei nicht gelöscht.

Zustände: `requested`, `admitted`, `running`, `succeeded`, `failed`, `rejected`,
`unknown`. Terminal sind `succeeded`, `failed` und `rejected`. Eine erfolgreiche
POST-Antwort bedeutet ausschließlich `admitted`. Erst die projizierte native
Compaction-Nachricht mit Status `completed` bestätigt `succeeded`.

Der Runner speichert die Schreibabsicht vor dem POST. Nach einem unklaren Ausgang
gleicht er ausschließlich Inbox und Nachrichten mit der stabilen Message-ID ab.
Auch nach Neustart wird nicht erneut gepostet. Fehlende Evidenz bleibt `unknown`
und verhindert einen terminalen Factory-Auftragsabschluss. Einen solchen Fall
automatisch aufzulösen oder manuell freizugeben ist noch nicht implementiert.
Vor einem noch nicht versuchten POST prüft der Runner nochmals den Pausenstatus;
ein konkurrierender Interrupt bleibt wie bisher keine bestätigte Prozesspause.

Der abschließende Archivdurchlauf wartet auf die Auflösung der Compaction und
nimmt deren projizierte Nachricht auf. Ein inzwischen beendeter Agent oder eine
Pause kann eine noch nicht versuchte Compaction als `rejected` beenden.
Für die Ausführung muss der zugeordnete Runner ebenfalls diese Version verwenden.

Endpunkte: `GET/POST /v1/commands/{id}/compaction`, registrierungs- und
ownership-geschützte `GET/PUT /v1/runners/{runner}/commands/{id}/compaction`.
Änderungen werden als `session.compaction` im dauerhaften Eventlog veröffentlicht.

Abnahme: Store-Tests prüfen Persistenz, Request-Replay, Runner-/Revision-Fencing
und die Abschlusssperre. Der Runner-Test simuliert einen verlorenen POST-Ausgang,
Journalneustart und spätere Abschlussbeobachtung ohne zweiten POST. Der erweiterte
`scripts/smoke-agent.py` hat mit zwei echten parallelen OpenCode-Agenten eine native
Compaction einschließlich Factory-/Runner-Neustart, Weiterarbeit und archivierter
Abschlussnachricht erfolgreich geprüft. Das ist eine lokale Laborabnahme.

Verbindliche Anforderung des Betreibers: Der Operator-Agent darf und soll selbst
entscheiden, wann er Sessions beendet oder eine explizite native Compaction
anfordert. Das ist Teil des Supervisors, keine ausschließlich manuelle TUI-Funktion.

## Geprüfter OpenCode-2.0.27-Vertrag

Quelle: `spikes/opencode/openapi-2.0.27.json`.

- `POST /api/session/{sessionID}/compact` mit `{id, delivery?}` nimmt eine
  Compaction dauerhaft in die Inbox auf. Standardmäßig wird am nächsten
  Schrittübergang compactet, nicht erst hinter allen wartenden Prompts.
- Die optionale `id` ist eine `Session.Message.ID`. Factory muss eine stabile,
  journalisierte ID verwenden. Nach unklarem Ausgang ist der Inbox-/Message-
  Abgleich erforderlich; HTTP-Annahme beweist keinen erfolgreichen Abschluss.
- `POST /api/session/{sessionID}/interrupt?resume=false` kann aktive Ausführung
  unterbrechen. `interrupted=true` beweist weder vollständige Prozessruhe noch
  einen dauerhaft gesetzten Pausen-Lock. `false` kann ein Idle-No-op sein.

## Ausführungsregeln

1. Operator bewertet Kontextnutzung, Informationsdichte, Fortschritt, offene
   Fragen und nächste Aufgabe; Entscheidung und Begründung werden gespeichert.
2. Relevante Erkenntnisse und offener Arbeitsstand werden mit Quellen gesichert.
3. Factory legt einen revisions-/ownership-geschützten Lifecycle-Auftrag an.
4. Der zuständige Runner journalisiert den externen Write vor dem API-Aufruf.
5. Ergebnis wird durch Lesen abgeglichen; unklare Ausführung bleibt explizit unklar.
6. Nach Compaction arbeitet dieselbe Session weiter, sofern keine Beendigung
   angefordert ist. Bei Beendigung bleiben Archiv und Projektwissen erhalten.
7. Ein neuer Schreiber oder Ownership-Transfer folgt erst nach bestätigtem
   Stillstand. Sessionbeendigung ist kein Löschen der OpenCode-Daten.

Es gibt keine allgemeine menschliche Bestätigungspflicht für diese Supervisor-
Entscheidungen. Selbstständigkeit ersetzt aber nicht den Nachweis der tatsächlichen
Ausführung. Wiederholte Compactions ohne Fortschritt müssen als fehlgeschlagene
Strategie behandelt werden, nicht als unbegrenzte Schleife.

Stand: Compaction-Auftrag und Abschlussabgleich sind implementiert. Der automatische
Entscheider, die vorgeschaltete Wissensextraktion und Sessionbeendigung sind noch
offen; die vorhandene Pause-/Interrupt-Implementierung bleibt konservativ.

# Autonome Sessionpflege durch den Operator-Agenten

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

Stand: Anforderung und API-Vertrag festgehalten. Der automatische Entscheider,
Compaction-Auftrag und Abschlussabgleich sind noch nicht implementiert; die
vorhandene Pause-/Interrupt-Implementierung bleibt konservativ.

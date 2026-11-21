# Autonome Sessionpflege durch den Operator-Agenten

## Implementierter Mechanismus (Entwicklungsstand nach v0.1.1)

Native Compaction ist über API/CLI und den owning Runner ausführbar, mit
persistenten Aufträgen seit Schema 9. Der [Session-Supervisor](supervisor.md)
entscheidet inzwischen selbst zwischen Warten, Compaction und Abschluss bereits
erfolgreicher Sessions. Das bestätigte Stoppen laufender Prozesse bleibt offen.

## Fortsetzung und Abschluss einer erfolgreichen Session

### Vorbereitung: Freezer-Protokoll (Schema 13)

Der Server unterstützt jetzt den persistenten Zustandsvertrag für eine spätere
dedizierte Container-Laufzeit pro Workstream:

- `requested` → `frozen` benötigt einen Runner-Nachweis mit vollständiger
  Container-ID, Startzeit der Container-Instanz und Beobachtungszeitpunkt.
- Resume erzeugt zunächst `resume_requested`. Erst ein bestätigtes Auftauen
  derselben Container-Instanz setzt `released`. Ein Klick allein gibt keine
  laufende Ausführung frei.
- Unklare Ergebnisse bleiben `interrupt_unverified`; vorhandene Identität bleibt
  gespeichert. Lesender Abgleich darf anschließend denselben Vorgang bestätigen.
- Fremde Runner, veraltete Ownership-Epochen/Requests, gewechselte Instanzen und
  rückläufige Beobachtungszeitpunkte werden abgewiesen. Ein verlorener identischer
  Auftau-Report kann auch nach Serverneustart wiederholt werden.
- Solange ein nicht freigegebener Container-Nachweis vorliegt, wird kein
  terminales Auftragsergebnis angenommen. Vor-Ausführung-Holds können einen
  Container-Nachweis nicht ersetzen.
- Die HTTP-Schnittstelle verlangt für neue Freezer-Meldungen die registrierte
  Capability `container-freeze-v1`. Ältere Runner erhalten keine Container-Control
  mit Nachweis zur Verarbeitung.

**Dies ist bisher nur der getestete serverseitige Vertrag.** Der ausgelieferte
Runner bewirbt diese Capability noch nicht und führt kein Docker-Pause/Unpause
aus. Dedizierte Laufzeitbereitstellung, lokales Intent-Journal, Prüfung der
Containment-Grenzen und echte Tests mit abgekoppelten/signalresistenten Prozessen
stehen noch aus. Der Server vertraut einem autorisierten Runner-Bericht; ein
gespeicherter Bericht ist kein aktueller Kernel-Nachweis und keine Erlaubnis für
Ownership-Transfer. Einfrieren beendet außerdem keine Prozesse und stoppt keine
bereits ausgelösten externen Dienste. Die bisherige Interrupt-Funktion bleibt
unverändert unbestätigt.

Store-Tests prüfen Serverneustart, identischen Replay, Instanzwechsel, verspätete
Reports, unklare Auftau-Ergebnisse und blockierten Auftragsabschluss. Schema 13
verhindert ein Öffnen der Datenbank durch ältere Server; Downgrade benötigt die
passende Sicherung.

```sh
yoloforge stream continue STREAM --previous-command LETZTER-AUFTRAG \
  --expected-revision STREAM-REVISION --prompt 'Naechster Arbeitsschritt'
yoloforge close-session LETZTER-AUFTRAG --expected-stream-revision REVISION \
  --reason 'Ziel erreicht'
yoloforge session-closure AUFTRAG
```

In der TUI: Stream öffnen, mit Tab „Session nach Erfolg fortsetzen“ wählen,
Prompt eingeben und Ctrl-S. Die ursprüngliche Session-ID bleibt erhalten, jede
Fortsetzung erhält eine neue Command-/Message-ID und den direkten Vorgänger als
Provenienz. Ein eigener abschließender Archivdurchlauf sichert den neuen Stand.

Fortsetzung setzt den letzten erfolgreichen, archivierten Agentenauftrag desselben
Streams, Runners, Ownership-Epoche und Ausführungsprofils voraus. Der Runner prüft
sein Vorgängerjournal, Endpoint, Worktree und dass keine aktive Ausführung, Inbox
oder offene Rückfrage vorliegt. Eine verschwundene Session wird niemals neu
erstellt. Eine alte Assistant-Antwort darf nicht als Ergebnis des Folgeprompts
gelten. Unklare Promptzustellung wird wie beim ersten Auftrag nur lesend abgeglichen.

Ein Abschluss sperrt die gesamte Sessionlinie für weitere Fortsetzungen; Grund,
Root- und letzter Auftrag bleiben dauerhaft erhalten. Ein neuer normaler Agentstart
verwendet eine neue Session und das Projektgedächtnis. Dies ist ein Scheduling-
Abschluss nach Erfolg, kein Nachweis einer Prozessbaum-Stilllegung nach Interrupt.

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

Stand: Compaction, erfolgreiche Session-Fortsetzung und -Abschluss sowie ein
modellbasierter Session-Supervisor mit quellenverknüpfter Wissensübernahme sind
implementiert. Das Stoppen laufender Sessions, Recovery und vollständige
Supervisor-Ticketplanung bleiben offen; Pause/Interrupt bleibt konservativ.

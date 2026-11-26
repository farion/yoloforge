# Intent-Lebenszyklus

[BPMN als SVG](intent-lifecycle.svg) · [LAML-Quelle](intent-lifecycle.laml)

Das Diagramm zeigt den Ist-Ablauf des aktuellen Arbeitsstands. Ausgänge wie
„Lauf fortsetzen“ oder „Weiter bei …“ sind Querverweise: Dort endet nur der
gezeichnete Ausschnitt, nicht der Intent.

**Warten auf Zuteilung ist dagegen eine echte Schleife:** `factoryd` startet
die Scheduler-Prüfung alle fünf Sekunden oder früher bei einem internen
Wecksignal. Das Signal bedeutet nicht, dass der Intent jetzt ausführbar ist.
Der Scheduler prüft die Voraussetzungen erneut; fehlen sie weiterhin,
bleibt der Intent eingereiht und wartet auf die nächste Prüfung.
Siehe [Scheduler-Schleife in `main.rs`](../crates/factoryd/src/main.rs).

## Wo ändert sich der Status?

| Auslöser | Gespeicherter Status bzw. sichtbare Änderung |
| --- | --- |
| Intent anlegen oder absenden | Entwürfe sind über `draft` gekennzeichnet, nicht über einen eigenen `TicketState`. Beim Absenden fällt das Flag weg; `Backlog` und `Ready` sind für die Zuteilung relevant. Ein noch nicht übernommener Intent kann über „Bearbeiten“ wieder zum Entwurf werden. |
| Zuteilung und Vorbereitung | Projekt-/Verteilungspause, fehlender Runner, nicht erfüllte Abhängigkeiten oder eine belegte Session können den Start verzögern. Anzeigen wie „Wartend“, „Wird vorbereitet“ und „Orchestrator prüft“ sind keine eigenen Ticketstatus. Bei einer nötigen Entscheidung kann die Vorbereitung blockieren und eine Rückfrage stellen. |
| Factory startet den Auftrag | `InProgress`. Das ist nicht dasselbe wie der Runnerstatus: Der Auftrag selbst wechselt von `Pending` über `Accepted` zu `Succeeded` oder `Failed`. |
| Lauf braucht eine Antwort oder Berechtigung | Die UI kann „Braucht Antwort“ anzeigen, ohne den Ticketstatus zu ändern. Erst nach Zustellung geht der Lauf weiter. Für freigegebene Standardfragen und bestimmte Berechtigungen ist eine automatische Entscheidung möglich. Rückfragen von Routing und Review sind davon getrennt und können den Intent auf `Blocked` setzen. |
| Lauf schlägt fehl oder stoppt ohne Ergebnis | „Fehlgeschlagen“ kann bereits aus dem Auftrag abgeleitet werden; `Blocked` ist ein separater Ticketstatus. Je nach Ursache kommen ein geplanter Retry, einmalige Fortsetzung in derselben Session nach abgeschlossenem Sessionarchiv oder Eskalation in die Inbox infrage. Git-, Veröffentlichungs- und Workspace-Probleme haben eigene Recovery-Pfade. |
| Ergebnis liegt zur Prüfung vor | `Review` bedeutet in der UI „Ergebnis vorhanden“; `Validating` ist die aktive Reviewer-Phase. `Review` ist kein Pflichtschritt vor `Validating`. Ein Review-Retry kann in `Validating` bleiben. Bei Nacharbeit: `ReworkQueued` → `InProgress` mit neuem Auftrag → erneute Prüfung. Bei einer Rückfrage: `Blocked` → nach Klärung zurück zur Prüfung. |
| Review gibt frei | Ohne nötige Übernahme kann die Validierung auf `Done` wechseln. Sonst wird der geprüfte Stand zur Veröffentlichung bzw. zum Merger weitergereicht. Der Implementer schreibt nicht in den Zielbranch. Bestätigte Übernahme führt zu `Done`; ein geplanter Integrations-Retry kann zunächst in `InProgress` bleiben. Eskalierte Übernahmeprobleme halten den Intent in der Queue. |
| Inbox löst einen Blocker | Je nach Ursache: Session neu zuordnen, Worktree neu vorbereiten, Lauf fortsetzen, Review wiederholen oder Übernahme erneut versuchen. Eine zulässige manuelle Abschlussaktion nach gescheitertem Merger setzt `Done`, ohne den fehlgeschlagenen Lauf umzuschreiben. |
| Nutzer bricht ab oder löscht | Ein zulässiger Abbruch setzt `Cancelled`, schließt offene Rückfragen und fordert bei Bedarf einen Stopp an. Der Intent bleibt im Archiv. Löschen ist getrennt: Der Tombstone blendet den Intent aus den Listen aus; er landet nicht als neuer Archiveintrag. |
| Nutzer setzt einen beendeten Lauf zurück | Auch aus dem Archiv möglich, sofern der Reset zulässig ist. Zunächst `Blocked`; erst nach Runner-Bestätigung wieder `Backlog` mit `draft = true`. Der Intent bleibt derselbe, das Sessionarchiv bleibt erhalten. |

## Wann erscheint der Intent im Archiv?

Die Desktop-UI berechnet die Zuordnung; **`Archived` ist kein `TicketState`**.
`Cancelled` und manuell abgeschlossene fehlgeschlagene Merger erscheinen im
Archiv. Ohne Auftrag reicht `Done`. Mit Auftrag prüft die UI den Erfolg:
ein erfolgreich beendeter, nicht schreibgeschützter Agentenlauf mit bestätigter
Erfolgsphase oder ein erfolgreicher Target-Git-Auftrag mit Git-Erfolgsnachweis,
jeweils ohne ungelöste Frage. Blockierte oder unbestätigte Veröffentlichung
sowie `Blocked`, `Validating` und `ReworkQueued` halten den Intent grundsätzlich
in der Queue; die genannten Sonderfälle haben Vorrang.

Ein **Sessionarchiv** kann bereits vollständig sein, während der Intent wegen
eines Fehlers noch in der Queue oder Inbox steht. Es archiviert den Lauf, nicht
den Intent.

Code: [TicketState](../crates/factory-core/src/lib.rs),
[UI-Status und Archivregel](../crates/factory-gui/src/model.rs),
[Server-Aktionen](../crates/factoryd/src/store/tasks.rs),
[Validierung](../crates/factoryd/src/store/validation.rs),
[Rückfragen und Recovery](question-orchestration.md).

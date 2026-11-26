# Rückfragen und Berechtigungen

Formularfragen haben eine feste ID. Der Intent wartet, bis eine Antwort
vorliegt und der Runner sie an OpenCode zustellt. Der Orchestrator rät keine
Antworten und beantwortet keine Interviews. Ein interner Statuswechsel
ändert weder die Frage noch den Antwortentwurf.
Eine bestätigte Antwort bleibt abgeschlossen. Neue Rückfragen brauchen eine
andere ID.

Antworten werden dauerhaft eingereiht. Der Runner vergleicht vor der Zustellung
die Frage und den aktiven Lauf mit OpenCode. Wenn OpenCode die Frage entfernt
oder unter derselben ID verändert, wird nicht auf ein anderes Formular geantwortet.
Eine abgewiesene Eingabe kann für dieselbe Frage korrigiert werden.

Für eine risikoarme Standardfrage kann ein Operator über `POST /v1/knowledge`
eine Projekt- oder Intent-Notiz mit `source: "orchestrator-approved-faq"`
und ohne `source_command` freigeben. `content` ist ein JSON-Objekt mit `form`
(dem exakten Formular ohne `id`, `sessionID` und `state`) und `answer`.
Unterstützt ist ein einzelnes String-Auswahlfeld ohne Freitext. Die Antwort
muss eine angebotene Option sein. Bei widersprüchlichen Antworten, geändertem
Formular, abweichendem Branch oder ohne Freigabe bleibt die Frage offen.
Die Einreihung der Antwort nennt die Notiz-ID und Revision im Verlauf.

## Berechtigungen

Der Scheduler kann Berechtigungsanfragen einmalig und schreibgeschützt auswerten.
Die Auswertung läuft außerhalb des Limits arbeitender Sessions. Projektpause und
Stream-Steuerung werden respektiert.

- Der Orchestrator verwendet nur `once` oder `reject`, nie `always`. `once`
  ist auf lesende Aktionen mit expliziten relativen Ressourcen begrenzt.
  Bei unzureichendem Kontext überlässt er die Entscheidung dem Nutzer.
- Die Zustellung nutzt denselben Weg wie eine Nutzerentscheidung. Eine bereits
  eingereihte Nutzerentscheidung wird nicht überschrieben.
- Die Diagnose zeigt `orchestrator-question`. Fragen des Orchestrators werden
  nicht rekursiv ausgewertet.

## Supervisor

Der projektbezogene Orchestrator richtet den Session-Supervisor ein, sofern
keine explizite Konfiguration existiert (Standard: 12 Auswertungen, mindestens
5 Minuten Abstand). Der Supervisor greift nicht in eine Ausführung ein, solange
sie auf eine Antwort oder deren Zustellung wartet. Bestehende deaktivierte oder
benutzerkonfigurierte Richtlinien bleiben unverändert.

## Recovery fehlgeschlagener Intent-Läufe

Bei aktiviertem Scheduler bewertet der Orchestrator fehlgeschlagene,
abschließend archivierte Intent-Läufe. Er sieht den Intent, die
Fehlerzusammenfassung, begrenzte Sessionauszüge und Projektwissen. Die
Bewertung ist schreibgeschützt. Ihre Aktionen sind **Fortsetzen** und
**Eskalieren**. Eine Fortsetzung ist auf den bisherigen Runner, die bisherige
Session und den bisherigen Worktree beschränkt. Sie sendet den Originalprompt
und seine Anhänge nicht erneut. Der Runner prüft vor dem Start Sessionidentität,
Ruhezustand und ausstehende Eingaben. Nach einem erneuten Fehlschlag bleibt der
Intent mit einem konkreten Hinweis blockiert. Bei alten Runnern wartet der
Auftrag auf die Fähigkeit `intent-recovery-v1`.

Git- und Workspace-Recovery, Wiederaufnahme einer gesicherten Veröffentlichung
und der Merger behalten ihre getrennten Prüfungen. Ein Veröffentlichungskonflikt
wird nicht als generischer Agentenfehler wiederholt.

Der Runner vergleicht bei bestätigter Veröffentlichung den Git-Dateibaum des
Ergebnissnapshots mit dessen Basis. Eine unveränderte Version ist ein Nachweis
für den Reviewer, aber kein pauschaler Fehler: Analyse-Intents und reine
Test-Fortsetzungen können zu Recht keine Dateien ändern. Bei angeforderten
Änderungen muss der Reviewer fehlende Arbeit zur Nacharbeit geben oder den
Intent als unklar markieren.

## Inbox und Meldungen

Die Desktop-**Inbox** trennt **Fragen** und **Probleme**. Das Projekt-Badge
zählt nur offene Entscheidungen. Geht eine passende FAQ-Antwort sofort in
die Zustellung, erscheint kein Toast. Berechtigungsfragen bleiben während der
automatischen Prüfung aus der Inbox. Scheitert die Prüfung oder bleiben nach
zwei Minuten Ergebnisse aus, wird die Frage sichtbar. Der Nutzer kann während
der Prüfung weiterhin direkt in der Session antworten.

Ein eskaliertes Problem gehört zum aktuellen Auftrag des Intents. Ein neuer
Auftrag, die Löschung oder der Abschluss entfernt es aus der Inbox. Explizit
blockierte Veröffentlichungen aus älteren Serverständen werden nachgetragen;
ein fehlender Erfolgsnachweis allein erzeugt keinen Eintrag. In der
Inbox kann der Nutzer den Intent ansehen, einen wiederholbaren Fehler erneut
einreihen oder einen inaktiven Intent nach Bestätigung löschen. Schlägt der
Orchestrator eine Fortsetzung in derselben Session oder eine erneute
Worktree-Vorbereitung vor, zeigt die Inbox die Aktion und den Grund. Die
Factory führt erst nach Bestätigung die gespeicherte Aktion aus und prüft
Revision, aktuellen Auftrag und Ausführungsvoraussetzungen erneut. Eine
wiederholte Zustellung derselben Bestätigung startet keinen zweiten Lauf.
Eine fest gewählte, nachweislich nicht mehr nutzbare Session erscheint mit dem
Vorschlag zur Neuzuordnung. Erst nach Zustimmung löst die Factory die Bindung;
eine vom Orchestrator selbst gewählte, nicht mehr nutzbare Session ersetzt er
ohne Rückfrage. Auf eine noch laufende Session wartet die Factory weiterhin.
Statusmeldungen
bleiben im Verlauf. Nur neuer Handlungsbedarf erzeugt einen Toast, keine
Routine-Erfolge oder internen Wiederholungen.

## Zurück zum Entwurf

Nach ausdrücklicher Bestätigung wird eine beendete, vollständig archivierte
Intent-Session sofort für weitere Aufträge gesperrt. Noch nicht gestartete
Folge-Intents werden entkoppelt und dem Orchestrator zur Neuzuordnung
überlassen. Hat noch ein anderer nicht abgeschlossener Lauf Anspruch auf den
Worktree, lehnt die Factory den Vorgang ab.

Der zuständige verwaltete Runner prüft sein Journal und die Identität der
ruhenden OpenCode-Session und des verknüpften Worktrees. Er entfernt nur
diese Session und diesen Worktree. Erst nach Bestätigung beider Schritte
wird der Intent wieder zum Entwurf. Prompt, Anhänge, Factory-Verlauf und
bereits integrierte Ergebnisse bleiben erhalten. Der Vorgang kann nach einem
Neustart fortgesetzt werden; bei fehlenden Nachweisen bleibt er offen.
Laufende Sessions, Container-Sessions, Merger und Target-Git-Aufträge sind
von diesem Weg ausgenommen.

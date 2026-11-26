# Ergebnisse im angegebenen Projektordner

Bei einem lokalen Projektordner (absoluter Pfad als Projektquelle und verwaltete
Ausführungs-Worktrees) übernimmt der Runner erfolgreiche Agentenergebnisse
automatisch dorthin, bevor er den Abschluss meldet. Der Projektordner erhält einen
Git-Commit; der isolierte Arbeitsbereich samt Index bleibt unverändert erhalten.
Eine nachfolgende Aufgabe startet von diesem veröffentlichten Stand.

Im OpenCode-Verlauf der YoloForge-GUI zeigt ein Factory-Hinweis mit Uhrzeit,
ob der ausführende Runner den Stand lokal oder auf dem verwalteten Basisbranch
veröffentlicht hat. Der Hinweis bestätigt keine spätere Spiegelung oder
Checkpoint-Rückübertragung auf andere Geräte.

Die Übernahme erstellt mit einem separaten Git-Index einen unveränderlichen
Snapshot einschließlich neuer, nicht ignorierter Dateien. Build-Ausgaben gemäß
`.gitignore` werden nicht übernommen. Ungelöste Konflikte und Gitlinks werden
abgewiesen. Zielbranch muss übereinstimmen, der Zielordner muss sauber sein.
Ein Fast-forward aktualisiert Dateien und Branch; kein Reset, Clean oder
erzwungenes Überschreiben. Uncommittete Änderungen im Ziel blockieren weiterhin.

## Parallele Ergebnisse

Ist der Zielbranch inzwischen fortgeschritten, kombiniert `git merge-tree` den
unveränderlichen Ergebnissnapshot dreiseitig mit dem aktuellen Zielcommit. Nur
konfliktfreie Ergebnisse werden als neuer Commit mit beiden Eltern veröffentlicht.
Abgezweigte Zielgeschichte wird abgewiesen. Repository-eigene Merge-Programme werden
nicht ausgeführt. Snapshot, kombinierter Commit und Zielbindung werden journalisiert;
Ausführungs-HEAD und -Index bleiben unverändert. Folgeaufträge vergleichen gegen
den letzten eigenen Snapshot, damit sie Änderungen anderer Streams nicht löschen.

Bei echten Konflikten bleibt der Snapshot unter
`refs/yoloforge/publication-snapshots/COMMAND_ID` erhalten. Der Abschluss zeigt
Ziel-/Basis-/Snapshotcommit, betroffene Dateien und die nötige Nacharbeit. Keine
Konfliktmarker oder Indexänderungen werden in den Projektordner geschrieben.
Bei einem echten Konflikt in einem lokalen oder Factory-verwalteten Projekt
startet die Factory einmalig einen **Merger**. Er arbeitet in einem eigenen
Worktree, ausgehend vom eingefrorenen Ergebnissnapshot. Das Modell wird zuerst
in den Projekt- und dann in den Factory-Einstellungen nachgeschlagen; ohne
Rollenmodell gilt das Factory-Standardmodell. Laufende und abgeschlossene
Versuche stehen unter **Merger** und **Archiv**.

Der Merger soll die Auflösung testen und committen. Der Runner prüft, dass
beide ursprünglichen Commits in der Historie liegen, und veröffentlicht nur
gegen den unveränderten Zielstand. Ein schmutziger Zielordner löst keinen
Merger aus. Schlägt die Auflösung oder Veröffentlichung fehl, bleibt der
Intent offen; es startet keine automatische Wiederholung. Neue Intent-Arbeit
nach einer Übernahme beginnt in einer neuen Session vom Zielbranch, nicht im
alten Ausführungs-Worktree.

Die automatische Kombination ist keine erneute Testausführung des Gesamtstands.
Für verpflichtende Tests vor Integration bleibt der getrennte Review-Ablauf vorgesehen.

Das Runner-Journal (Schema 9) hält Snapshot und Zielbindung vor der Änderung fest.
Nach einem Neustart wird derselbe Commit abgeglichen, nicht ein neuer Snapshot
erstellt. Eine bereits abgeschlossene Übernahme überschreibt beim Replay keine
späteren Benutzeränderungen. Die verwaltete Basis wird per Git-CAS fortgeschrieben.
URL-Imports ohne lokalen Zielordner und explizite Repository-Bindings nutzen
weiterhin den separaten Integrationsablauf.

Factory-verwaltete Projekte verwenden denselben unveränderlichen Snapshot- und
CAS-Mechanismus direkt auf ihrem verwalteten Basisbranch. Ihr erfolgreicher
Stand wird anschließend als verifizierter Projekt-Checkpoint exportiert. Damit
können Folgeaufträge und registrierte lokale Spiegelkopien denselben
maßgeblichen Stand beziehen, ohne Git-Hosting oder ein gemeinsames Dateisystem.

Für bereits vor dieser Funktion abgeschlossene Aufgaben existiert die
Wartungsoption `factory-runner --publish-result COMMAND_ID` mit den bisherigen
Runner-/Journal-/Workspace-Argumenten und einem Operator-Token. Der Runner muss
dazu gestoppt sein. Die Option prüft Factory-Erfolg, aktuellen Besitzer/Epoche,
neueste Ausführung und ausstehende Projektarbeit, nutzt denselben journalisierten
Übernahmeablauf und startet keinen Agenten erneut.

Wurde ein altes Ergebnis bereits manuell zusammengeführt, kann der Operator
mit `--publish-result COMMAND_ID --record-integrated-snapshot SNAPSHOT_COMMIT`
genau diesen Intent nachtragen. Erst die fachliche Übernahme prüfen: Die reine
Git-Abstammung belegt nicht, ob die richtigen Konfliktauflösungen gewählt wurden.
Der Runner prüft anschließend, dass der gesicherte Stream-Snapshot
(`refs/yoloforge/consolidation/STREAM_ID`) im konfigurierten Zielbranch enthalten
ist. Nur ein Admin-Zugang kann die Bestätigung an die Factory melden. Der Befehl
schreibt weder in Git noch startet er einen Agenten.

Geprüft: ignorierte Dateien, unveränderter Ausführungsindex/HEAD, Folgeaufgaben,
Fortsetzung, Zieländerungen, idempotentes Replay und Wiederaufnahme nach erfolgtem
Ziel-Fast-forward. Der Realmodell-Test `scripts/smoke-prompt-image.py` prüft zusätzlich,
dass das geschriebene Ergebnis im angegebenen Projektordner ankommt. Dieser
erweiterte Realmodell-Test konnte beim letzten Lauf nicht starten, weil das
authentifizierte Testlabor gestoppt war. Die nachtraegliche Uebernahme des bereits
mit echtem Modell erstellten Minesweeper-Projekts wurde dagegen ausgefuehrt;
`cargo test --offline` im Zielordner bestand mit 15 Tests.

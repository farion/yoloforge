# Ergebnisse im angegebenen Projektordner

Bei einem lokalen Projektordner (absoluter Pfad als Projektquelle und verwaltete
Ausführungs-Worktrees) übernimmt der Runner erfolgreiche Agentenergebnisse
automatisch dorthin, bevor er den Abschluss meldet. Der Projektordner erhält einen
Git-Commit; der isolierte Arbeitsbereich samt Index bleibt unverändert erhalten.
Eine nachfolgende Aufgabe startet von diesem veröffentlichten Stand.

Die Übernahme erstellt mit einem separaten Git-Index einen unveränderlichen
Snapshot einschließlich neuer, nicht ignorierter Dateien. Build-Ausgaben gemäß
`.gitignore` werden nicht übernommen. Ungelöste Konflikte und Gitlinks werden
abgewiesen. Zielbranch und Ausgangscommit müssen übereinstimmen, der Zielordner
muss sauber sein. Ein Fast-forward aktualisiert Dateien und Branch; kein Reset,
Clean oder erzwungenes Überschreiben. Eigene Änderungen im Ziel oder ein inzwischen
veränderter Projektstand blockieren die Übernahme; das Ergebnis bleibt im
Arbeitsbereich erhalten und der Abschluss nennt den Grund.

Das Runner-Journal (Schema 9) hält Snapshot und Zielbindung vor der Änderung fest.
Nach einem Neustart wird derselbe Commit abgeglichen, nicht ein neuer Snapshot
erstellt. Eine bereits abgeschlossene Übernahme überschreibt beim Replay keine
späteren Benutzeränderungen. Die verwaltete Basis wird per Git-CAS fortgeschrieben.
URL-Imports ohne lokalen Zielordner und explizite Repository-Bindings nutzen
weiterhin den separaten Integrationsablauf.

Für bereits vor dieser Funktion abgeschlossene Aufgaben existiert die
Wartungsoption `factory-runner --publish-result COMMAND_ID` mit den bisherigen
Runner-/Journal-/Workspace-Argumenten und einem Operator-Token. Der Runner muss
dazu gestoppt sein. Die Option prüft Factory-Erfolg, aktuellen Besitzer/Epoche,
neueste Ausführung und ausstehende Projektarbeit, nutzt denselben journalisierten
Übernahmeablauf und startet keinen Agenten erneut.

Geprüft: ignorierte Dateien, unveränderter Ausführungsindex/HEAD, Folgeaufgaben,
Fortsetzung, Zieländerungen, idempotentes Replay und Wiederaufnahme nach erfolgtem
Ziel-Fast-forward. Der Realmodell-Test `scripts/smoke-prompt-image.py` prüft zusätzlich,
dass das geschriebene Ergebnis im angegebenen Projektordner ankommt. Dieser
erweiterte Realmodell-Test konnte beim letzten Lauf nicht starten, weil das
authentifizierte Testlabor gestoppt war. Die nachtraegliche Uebernahme des bereits
mit echtem Modell erstellten Minesweeper-Projekts wurde dagegen ausgefuehrt;
`cargo test --offline` im Zielordner bestand mit 15 Tests.

# Native Session unterbrechen und fortsetzen

Im normalen Betrieb nutzt der Runner den vorhandenen OpenCode-Service und dessen
Provider-Anmeldung. Eine zusätzliche Container-Anmeldung ist nicht erforderlich.

1. Im TUI die Streamsteuerung öffnen und **Pause** senden.
2. Der Runner journalisiert den Interrupt vor dem HTTP-Aufruf. Auch bei einer
   verlorenen Antwort oder nach Neustart wird derselbe Interrupt nicht erneut
   gesendet; stattdessen liest er den Sessionzustand.
3. Nur `outcome=interrupted`, eine inaktive Session und eine leere Inbox ergeben
   `session_interrupted`. Der Runner archiviert abschließend und beendet den
   bisherigen Auftrag mit Agentphase `interrupted` (kein erfolgreicher Abschluss).
4. **Resume** gibt die Planung nach dem Auftragsabschluss wieder frei. Anschließend
   einen neuen Fortsetzungsprompt in der Streamsteuerung senden oder beim neuen
   Task dieselbe Session fest auswählen. Der neue Auftrag verwendet denselben
   Sessionverlauf und Arbeitsbereich, aber eine neue Prompt-/Auftrags-ID.

Fortsetzung erfordert das finale Archiv, dieselbe Ownership und dasselbe explizite
Projektmodell/Profil. Ein geändertes Profil führt nicht zu einem Modell-Fallback.
Der Runner prüft außerdem sein Vorgängerjournal, Workspace, Endpoint und offene
Inbox-/Form-/Permission-Einträge vor einem neuen Prompt.

`session_interrupted` bestätigt **keinen Prozessbaum-Stopp**. Separat gestartete
Hintergrunddienste können weiterlaufen. Deshalb entsteht daraus weder ein
Containment-Nachweis noch eine Freigabe für einen konsistenten Checkpoint/Handoff.
Fehlende Journale, nicht passende Sessionidentität, aktive Ausführung oder offene
Inbox bleiben ungeklärt; kein blinder Neuversuch und keine automatische Freigabe.

Ab 0.1.3-dev werden erstmals lokal empfangene, serverseitig bereits angenommene
Schreibauftraege dauerhaft als `recovery_required` gesperrt. Das gilt auch fuer
Workspace-/Review-/Integrationsauftraege und fuer terminale Cleanup-Auftraege.
Eine nach verlorenem Ack erneut zugestellte Arbeit mit vorhandenem Empfangsjournal
ist davon nicht betroffen. Bei Agentenauftraegen prueft der Runner ausschliesslich
lesend die native Session-ID, Herkunft, Modell, Workspacepfad, Prompt/Bilddigest,
Aktivitaet und Inbox. Das Ergebnis erscheint als `unknown` in der bestehenden
Auftragsdiagnose; selbst beobachtetes `succeeded` ersetzt keine verlorene
Schreibabsicht. Keine automatischen Antworten, Compactions, Interrupts oder
Publikationen werden aus diesem Zustand ausgefuehrt. Zur Wiederherstellung ist
das urspruengliche Runner-Journal mit seinen Write-ahead-Belegen erforderlich.

Tests prüfen verlorene Interruptantworten mit Journal-Neustart, aktive Sessions,
offene Inbox, Freigabe erst nach Finalisierung sowie Archiv-/Revisionsschutz und
Fortsetzung unter derselben Session-ID. Die explizite Realmodell-Abnahme lautet:

```sh
python3 scripts/smoke-prompt-image.py --local-registration PATH \
  --provider PROVIDER_ID --model MODEL_ID --native-pause
```

Sie nutzt einen isolierten Factory-Server/Runner und Testarbeitsbereich am
vorhandenen nativen OpenCode-Service. Bestehende Factory-Dienste werden nicht
neu gestartet.

Die lokale Realmodell-Abnahme mit `openai/gpt-6.2-sol` ist bestanden:
`/tmp/opencode/factory-image-ymt1fccx`. Der unterbrochene Shell-Auftrag wurde
archiviert; ein neuer Prompt in derselben Session hat das erhaltene Testprojekt
erfolgreich weiterbearbeitet und veröffentlicht.

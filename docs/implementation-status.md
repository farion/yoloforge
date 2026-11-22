# Umsetzung der 15 offenen Punkte

Quelle: Arbeitsauftrag des Betreibers, alle 15 offenen Punkte umzusetzen.
Ein Eintrag gilt erst nach implementierter Funktion und dokumentierter Abnahme
als abgeschlossen. Laborprüfungen ersetzen keine Abnahme auf dem Zielhost.

## Prompt-Aufgaben und Projektpause (Schema 24)

Erweiterung Schema 25: feste Sessionauswahl im Prompteditor (`Ctrl-G`),
modellgestützte Zuordnung ohne Auswahl (`new`/`session`/`wait`), persistentes
Projektlimit (`l`) und gekennzeichnete Orchestrator-Läufe in der Diagnose.
Fortsetzungen behalten die Sessionwurzel und Aufgaben ihren eigenen
Ausführungsbezug. Implementierungsumfang, Prüfungen und offene Punkte:
[`task-routing.md`](task-routing.md). Die sieben Produktpakete sind noch nicht
vollständig umgesetzt.

Erweiterung Schema 26: einmalige, revisionsgeschuetzte Rueckfragenauswertung mit
Projektwissen, automatischer Antwort oder Eskalation; siehe
[`question-orchestration.md`](question-orchestration.md). Tests fuer Neustart,
menschliche Konkurrenzantwort und ungueltige Modellantworten bestanden.
Lokaler Anmeldeeinstieg (`o`) und Bereitschaftsansicht (`r`) sind in der
Projektliste verfuegbar; siehe [`project-readiness.md`](project-readiness.md).
Native SSH-Sessions haben einen lesenden Runner-Helfer und lokale Profile;
siehe [`ssh-sessions.md`](ssh-sessions.md). Separater SSH-Host und OAuth-Abnahme
stehen noch aus. Echte Modellabnahme fuer Bildrouting, feste Sessionfortsetzung,
automatische Frage und Folgepublikation bestanden
(`smoke-prompt-image.py --local-registration ... --followup`).

Ergebnisübernahme (Runner-Journal 9): erfolgreiche lokale Aufgaben werden automatisch
in den angegebenen Projektordner übernommen und bilden die Basis für Folgeaufgaben.
Unabhängiger Snapshot-Index, journalisierter Commit, sauberer unveränderter Zielstand,
Fast-forward und CAS schützen Ausführungsindex und Benutzeränderungen. Konflikte
bleiben mit konkreter Ursache sichtbar; siehe `result-publication.md`.

Session-Einstieg: Enter auf einem Arbeitsbereich oder einer gestarteten Aufgabe
öffnet die originale OpenCode-TUI im selben Terminal, gebunden an Service, Session
und Arbeitsordner. Nach Beenden kehrt YoloForge zurück. Native PTY-Abnahme mit der
Minesweeper-Session bestanden; fehlender CLI-Start wird sichtbar abgefangen.
Fuer SSH-Runner ist ein profilbasierter nativer Einstieg implementiert (Host-Abnahme offen). Die bisherige synchronisierte
Live-Ansicht bleibt unter `v`, technische Steuerung unter `c` verfügbar; siehe
`session-view.md`.

TUI-Aufgaben erfassen nur Prompt und optionale Bilder (bis vier PNG/JPEG/GIF/WebP,
zusammen 4 MiB). Ein einzelner Prompt-Editor ersetzt die schrittweise Erfassung:
Enter sendet, Shift-/Alt-Enter erzeugt neue Zeilen, Ctrl-V liest Bild/Text aus der
lokalen Zwischenablage (Linux X11/Wayland, Windows); hineingezogene Bilddateien
werden direkt angehängt. Ctrl-D entfernt den letzten Anhang. Die Zwischenablage
gehört zum TUI-Host; SSH stellt nicht automatisch die lokale Desktop-Zwischenablage
bereit. `/v1/tasks` speichert die Bilddaten dauerhaft;
der Runner übergibt sie als Inline-Dateien an OpenCode und prüft beim Wiederanlauf
den Bild-Digest der Prompt-Projektion. Alte Runner dürfen Bildaufträge nicht annehmen
(`task-images-v1`). Binäranhänge aus Agentenantworten/Archiv-Backfill bleiben offen.

Standardplanung entsteht automatisch beim Runner-Heartbeat, einschließlich
verwaltetem Repository für bisher nicht eingerichtete Projekte mit Aufgaben.
Kein Ready-Schritt und keine Aktivierung nötig. Projektpause hält neue Starts an;
angenommene Arbeit läuft weiter. Nach finalem Prompt-Ergebnis kann die nächste
Aufgabe starten, ohne manuelles Done. Store-Integrationstest prüft Prompt/Bilder,
Pause vor Annahme, Neustart, Capability-Fencing und Folgeaufgabe. Der PTY-Test
`scripts/smoke-project-setup.py --empty --tasks` prüft echte TUI/Factory/Runner,
Prompt-/Bildpersistenz und Pause/Fortsetzen bis zur automatischen Einplanung.
Mit `--new-folder --native --tasks` wurde auch ein echter isolierter OpenCode-
Service gestartet und der fehlende Projektordner angelegt. Prompt-Editor, Unicode-Navigation,
Anhangentfernung und unveränderliche Wiederholung sind zusätzlich unit-getestet.
Der PTY-Test fügt eine Bilddatei direkt in den Prompt ein und sendet mit Enter.
`scripts/smoke-clipboard.py` prüft echte X11-Bild-/Text-Zwischenablage im isolierten
Xvfb. Wayland-/Windows-Desktop-Abnahme steht noch aus. `scripts/smoke-prompt-image.py`
prüft mit echtem Modell den gesamten Ablauf vom fehlenden Projektordner über
automatische Einplanung und Inline-Bild bis zum geschriebenen Ergebnis und finalen
Archiv; bestanden (rotes Bild korrekt erkannt).

Fehlende absolute Projektordner werden als neue Projekte angelegt und ohne
Überschreiben veröffentlicht; vorhandene Repositories bleiben Importquellen.
Unter Schema 23 gescheiterte lokale Imports werden nach Registrierung eines
`local-project-create-v1`-Runners einmalig erneut vorbereitet. Wiederaufnahme ist
revisions-/epochengebunden, respektiert Pause und führt die Planung weiter.
Migration, Neustart und Folgeaufgabe sind geprüft. Auch alte automatisch geplante
Tickets brauchen nach einem finalen Ergebnis kein manuelles Done.

| Nr. | Arbeitspaket | Stand / Abnahme |
| --- | --- | --- |
| 1 | Bestätigte Pause und Session-Fortsetzung | Fortsetzung nach Erfolg sowie optionales dediziertes Container-Freeze/Thaw mit echtem Modell lokal geprüft. Native Sessionunterbrechung, finales Archiv und neuer Fortsetzungsauftrag in derselben Session implementiert; Zustand ausdruecklich ohne Prozessstopp-Nachweis (`native-pause.md`). Journal-/Server-Neustart, aktive Session/offene Inbox und Archivschutz getestet; Realmodell-Abnahme mit explizitem `openai/gpt-6.2-sol` bestanden. Normalbetrieb nutzt vorhandene Anmeldung, keine Containerpflicht. Zielhost-Abnahme offen |
| 2 | Recovery ungeklärter Aufträge | Expliziter API/CLI/TUI-Neuversuch unklarer Container-Freeze/Thaw-Steuerungen mit Instanzbindung, Revisionsschutz und Retry-Provenienz implementiert; TUI mit gebundenem Nachweis, unveränderlichem Replay und Offline-Entwurf lokal per PTY geprüft. Native Interrupts nach Antwortverlust/Neustart werden lesend abgeglichen, ohne erneuten POST. Recovery ohne Nachweis, bei Container-/Journalverlust und fuer nicht nachweisbare native Prompts offen |
| 3 | Vollständiges Sessionarchiv | Ungekürzte paginierte Message-Projektionen, CLI-Export und TUI-/Android-Archivbrowser mit eingefrorenem Revisionsstand; historischer Backfill aus erhaltenen Runner-Journalen. Begrenzte lokale Binärsnapshots und SHA256-geprüfter CLI-/TUI-/Android-Dateiexport implementiert (`archive-files.md`). Große/externe Dateien und Rekonstruktion verlorener Quellen offen |
| 4 | SSE und Runner-Long-Poll | Implementiert und lokal getestet; TUI nutzt SSE, Runner Long-Poll |
| 5 | Deployment, Backup/Restore, Updates | Container, Vorlagen und Backup/Restore lokal getestet; Zielinstallation durch Betreiber |
| 6 | Echte Laptop-/Server-Abnahme | Erstinstallation v0.1.1 auf rs9tm lesend geprüft: Binary-Hash, Health/Auth, Portbindung und Persistenz passen. Runner-/Agentenbetrieb, Update und Rollback noch offen |
| 7 | Gerätezugang, Rollen, Widerruf | API/CLI und TUI-Geräteverwaltung implementiert und lokal getestet: Rollen, einmalige Enrollment-Codes, Runner-Scope, Widerruf einschließlich laufender SSE-/Long-Polls |
| 8 | OpenAI-/GitHub-/Forgejo-Zugangsverwaltung | Lokaler OpenCode-Anmeldeeinstieg und Modell-/Integrations-/MCP-Status aus der TUI implementiert. SSH-Profil-Editor per PTY, lesender Remote-Bereitschaftshelfer und echter Loopback-SSH-Hop lokal geprueft. Bereitschaft/Anmeldung jetzt auch anhand bestaetigter Projektvorbereitung vor erster Agent-Session; Diagnose verwendet explizite aktuelle Projektmodell-IDs. Separater SSH-Host, OAuth-Refresh und echte Hosting-Zugaenge offen |
| 9 | Clone/Fetch und Repository-Einrichtung | CLI clone/register/fetch sowie TUI-Projektanlage: verwaltetes leeres Repository oder optionaler Clone, automatische Runner-/Service-Auswahl und ausschliesslich explizite Projektmodellbindung. Strukturierter MCP-Editor, lokale SSH-Verbindungsverwaltung und gemeinsame Projekt-Harness mit Runner-Zuordnungen implementiert. TUI-/Runner-Test mit synthetischem OpenCode-Health, lokale Imports und Neustarttests bestanden. Fetch im Assistenten, erweiterte Editoren und Login-Führung offen (`project-setup.md`) |
| 10 | Checkpoints und Laptop-/Server-Übergabe | API/CLI/TUI/Runner: journalisierte Git-Checkpoints, Kontextpaket, Bundle-Transport über Factory, revisionsgeschützter Ownership-Wechsel und neue Epochen-Worktrees implementiert. Zwei unabhängige lokale Runner mit Neustart und Rückübergabe geprüft; Checkpoint nach Realmodell-Erfolg und Runtime-Cleanup geprüft. Getrennte Zielhosts und Realmodell-Übergabe offen. Siehe `checkpoints.md` |
| 11 | Review und Integration | Merge-Kandidat, isolierte Tests, bestätigte Runtime-Entfernung und lokale Git-CAS-Integration implementiert/geprüft. CLI-Hosting-Helfer für GitHub/Forgejo mit gepinntem Review-Commit, Absent-Ref-Lease, journalisierter PR-Anlage, Antwortverlust-Reconciliation und zentralem URL-Beleg. Echte GitHub-Abnahme im freigegebenen privaten Testrepo bestanden (PR 1, Replay, fremder Branch, CAS-Konflikt). Forgejo-Abnahme, TUI-Assistent/automatische Einplanung und Zielhost offen (`hosting-publication.md`) |
| 12 | Projektgedächtnis und Kontextauswahl | Branch-/Ticketscope, budgetierte Prompt-Übernahme und atomare revisionsgeschützte Ablösung mit Provenienz implementiert; Extraktion und Widerspruchskorrektur mit echtem `openai/gpt-6.2-sol` bestanden. Neustart/unangebotene IDs/menschliche Aenderungen getestet. Archivbasierte Token-/Kostenanzeige nach Modell und Rolle in API/CLI/TUI implementiert; Replay-/Fortsetzungs-Deduplizierung und echte archivierte Verbrauchswerte geprueft (`usage.md`). Eigener Kostenhaushalt und modellspezifischer Tokenizer offen (`automatic-memory.md`) |
| 13 | Supervisor | Modellbasierter Session-Supervisor mit Entscheidungsjournal, Bewertungsbudget, Compaction, Sessionabschluss und gebundener Runtime-Beendigung implementiert. Mit dem Orchestrator automatisch aktiviert (12 Auswertungen, 5 Minuten Abstand, Vorrang fuer Aufgaben/Fragen). Prompt-Routing und einmalige automatische Rueckfragenauswertung mit Projektwissen implementiert; Konkurrenz-/Neustarttests bestanden. Echte autonome Lifecycle-/Zielhost-Abnahme offen |
| 14 | Android und Push | Native App mit verschluesselten Prompt-/Antwortentwuerfen, Rotation-Wiederaufnahme, bedingten Formularen/Mehrfachauswahl und Archivbrowser/Dateiexport. UI gegen echten Rust-Server und Vertragstests lokal und in CI bestanden (Lauf 45036987787). Optionaler ntfy-Begleitapp-Push mit dauerhafter Outbox (Schema 29), Backoff und Neustart/Antwortverlust-Abnahme. Stabile Signatur und signatur-/versionsgepruefter In-App-Updatepfad implementiert. Freigegebene GitHub-Secrets eingerichtet, signierter CI-Build erfolgreich (45036459858). Echtes Geraet/Doze, SAF-Dateidialog, Bildworkflow und reale Updateinstallation offen (`android.md`, `background-push.md`) |
| 15 | GitHub-Releases und Ansible-Vertrag | v0.1.1 veröffentlicht; Server-/Linux-TUI-Downloads lokal geprüft, Windows nativ in CI geprüft; signiertes Android-Entwicklungs-APK lokal verfuegbar, Android-Releasejob mit Secrets aktiviert. Neue Funktionen weiterhin 0.1.2-dev, noch keine neue versionierte Releaseveroeffentlichung |

Siehe `backlog.md` für Akzeptanzkriterien und `development.md` für bereits
implementierte Bedienabläufe. Zugangsdaten nicht in dieses Dokument eintragen.

Aktuelle lokale Abnahme: Workspace-Tests und Clippy bestanden; sieben Android-
Vertragstests sowie API/UI-Test mit Activity-Neustart und Archivnavigation;
signiertes APK installiert/erneut installiert und Fragen-Deep-Link gestartet.
ntfy-HTTP-Prozesstest nach Antwortverlust und Serverneustart bestanden
(`/tmp/opencode/yf-push-rwlj3gg2`), Android-UI-Evidenz unter
`/tmp/opencode/yf-android-api-o97oqq8v`, SSH-Bereitschaft eines vorbereiteten
Projektordners unter `/tmp/opencode/yf-ssh-lab-6d913utm`.

Modellbindung korrigiert: Automatische Legacy-Projekteinrichtung bewahrt das
explizite Projektprofil. Ohne Provider-/Modell-ID werden keine Agentaufträge
gestartet; der Runner verwendet keinen OpenCode-Standard als Ersatz und blockiert
abweichende Journalmodelle. Routing bietet nur profilkompatible Sessions an und
verwirft alte Bewertungen bei Profilwechsel. Explizite Interviews wie „Grill me“
lassen Formularfragen beim Nutzer; Agentanweisungen verlangen das Fragewerkzeug.
Regressionstests für Profilbewahrung, fehlendes Modell und Interviewfragen bestehen.

Ergänzung zu 2/13: Bestätigte Entfernung einer gebundenen dedizierten Runtime
über API/CLI und Runner implementiert; reale Docker-Abnahme mit abgekoppelten
Writern und Journal-Recovery bestanden. Auch extern entfernte Container sind mit
erhaltener Daemon-Bindung abschließbar. Fehlende Nachweise/Journale, ersetzte
Instanzen und native Recovery bleiben offen. Neue Ausführungen veröffentlichen
ihre Runtime-/Daemon-Bindung vor Agent-Schreibzugriffen zentral; damit ist
Beendigung nach lokalem Journalverlust implementiert und mit Docker geprüft.
Autonome Supervisor-Beendigungsanfragen mit atomarer Wissenssicherung sind
implementiert und mit synthetischen Modellentscheidungen geprüft; Realmodell-
Abnahme bleibt offen. Cleanup nach abgeschlossenem Erfolg bewahrt Ergebnis und
Archiv und ist mit echtem Modell geprüft; Supervisor-Close fordert Cleanup atomar
mit an. Details: `runtime-termination.md` (DB 24, Runner-Journal 9).

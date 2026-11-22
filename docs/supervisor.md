# Autonomer Session-Supervisor

## Nacharbeit und Neuverteilung (0.1.3-dev)

`continue` erzeugt fuer konkret unerfuellte Originalanforderungen nach Erfolg
einen neuen Schreibauftrag in derselben Session. Die Entscheidung enthaelt dazu
`followup`; finales Archiv, unveraendertes Profil, Ownership, Projektpause und
Arbeitsbudget werden weiterhin geprueft. Explizite Interviews werden ausgeschlossen.
Auftrag, Wissensnotiz und Entscheidungsbeleg entstehen atomar.

`reallocate` wird nur nach bestaetigter Entfernung der dedizierten Runtime eines
fehlgeschlagenen Auftrags angeboten. Unfixierte zugeordnete Aufgaben werden mit
ihrem Originalprompt/Bildern erneut bereitgestellt. Ticketbezogenes Quellenwissen
verweist auf den vorherigen Auftrag. Der regulaere Orchestrator entscheidet ueber
die neue Session; alte Worktrees bleiben erhalten, ihre Aenderungen werden nicht
automatisch kopiert. Feste Sessionbindungen, Interviews und ungeklaerte native
Unterbrechungen werden nicht neuverteilt. Beendete Runtime-Sessions werden aus dem
Routingkatalog ausgeschlossen.

Store-Tests pruefen Restart/Einmaligkeit, Budget, Interview, fehlenden Stoppnachweis
und konkurrierend gesetzte feste Sessionbindung. Realmodell-Abnahmen dieser beiden
neuen Aktionen stehen noch aus; der bisherige autonome Abschluss wurde real geprueft.

Entwicklungsstand nach v0.1.1, Schema 14. Der Factory-Daemon führt einen
dauerhaften Bewertungszyklus aus. Ein echter OpenCode-Agent bewertet Sessions
mit dem Provider/Modell des Projekts. Er entscheidet selbst zwischen `wait`,
`compact` und `close`; einzelne Entscheidungen benötigen keine menschliche
Bestätigung.

## Einrichtung

Server und Runner müssen diesen Entwicklungsstand unterstützen. Der Runner
meldet hierfür `session-lifecycle-v1` bei der Registrierung. Alte Runner
erhalten weder Fortsetzungs- noch Supervisor-Agentenaufträge; auch ein alter
Ack-Replay kann diese Schranke nicht umgehen. Beim Upgrade zuerst den Server,
dann die Runner aktualisieren.

Ein dedizierter Stream ohne Tickets dient ausschließlich den Supervisor-Bewertungen. Sein Runner
benötigt die übliche Repository-Zuordnung und OpenCode-Registrierung.

```sh
yoloforge stream create --project PROJEKT --runner RUNNER \
  --goal 'Autonome Sessionpflege'
yoloforge supervisor-set PROJEKT --stream SUPERVISOR-STREAM \
  --expected-revision 0 --enabled true --evaluation-budget 10 --interval-seconds 60
yoloforge supervisor PROJEKT
yoloforge supervisor-history PROJEKT
```

Die Factory bereitet dessen Worktree selbst vor. Der Supervisor-Stream ist für
gewöhnliche Agentenaufträge reserviert. Jede Bewertung erhält eine eigene Session
mit allen Toolberechtigungen auf `deny`; die Factory prüft diese Berechtigungen
vor dem Prompt. Er bewertet nur die bereitgestellten Beobachtungen und darf weder
Projektdateien bearbeiten noch Tools oder Unteragenten starten.

Die Erstkonfiguration verwendet Revision 0. Zum Ändern, Auffüllen des Budgets oder
Deaktivieren die aktuelle Policyrevision verwenden. `--enabled false` verwirft
noch nicht angewandte Bewertungen. Bereits angenommene Modell- oder Compaction-
Ausführungen werden damit nicht rückwirkend unterbrochen.

## Auswahl, Entscheidung und Grenzen

- Pro Projekt ist höchstens eine Bewertung offen; bereits bewertete Zielaufträge
  werden nach noch nicht bzw. länger nicht bewerteten Aufträgen berücksichtigt.
- Ziele sind der jeweils letzte Agentenauftrag eines Streams: laufend mit
  bestätigtem `running`-Fortschritt oder erfolgreich mit fertigem Archiv.
  Pausen, ungeklärte Zustände, offene Eingabewartezustände und Supervisoraufträge
  werden nicht automatisch überholt.
- Der Agent erhält Streamziel, Status, einen begrenzten Fortschritts-/Ergebnistext,
  einen begrenzten Archivauszug und bis zu 4096 Bytes Projekt-/Ticketwissen mit
  exakter Gültigkeit für den Zielbranch. Dies ist noch
  keine modellspezifische Messung der Kontextbelegung.
- Antwortformat ist strikt JSON mit `action`, `reason` und `memory`.
  Unbekannte Aktionen, fehlende Felder, Markdown und übergroße Ausgaben werden
  als `invalid_decision` protokolliert, ohne Aktion.
- Vor `compact` oder `close` speichert die Factory die Modellzusammenfassung als
  niedrig priorisierte, ausdrücklich unbestätigte Wissensnotiz mit Quellauftrag
  und Bewertungsreferenz. Diese Beobachtung gilt ausschließlich auf dem Zielbranch;
  sie ist keine unabhängig geprüfte Tatsache.
- Policyänderungen und eine veränderte Zielstreamrevision verwerfen veraltete
  Entscheidungen. Notiz, Entscheidungsjournal und Aktionsauftrag werden in einer
  SQLite-Transaktion gespeichert. Wiederanlauf erzeugt keine zweite Aktion.
- `compact` verwendet den bestehenden stabilen Runner-Auftrag. `applied` im
  Bewertungsjournal bedeutet hier **angefordert**, nicht nativ abgeschlossen.
  Den tatsächlichen Zustand zeigt `yoloforge compaction ZIEL-AUFTRAG`.
- Pro Zielauftrag wird automatisch höchstens eine Compaction angefordert.
  Eine fehlgeschlagene oder ungeklärte Compaction löst keine Wiederholungsschleife
  aus. Ein neuer Fortsetzungsauftrag ist eine neue bewertbare Arbeitseinheit.
- `close` schließt nur bereits erfolgreiche, fertig archivierte Sessions für
  zukünftige Fortsetzungen. Es löscht keine Nachrichten und beendet keinen
  laufenden Prozess. Den Nachweis liefert `yoloforge session-closure AUFTRAG`.

Das Budget zählt Modellbewertungen (maximal 100 je Konfiguration), nicht Tokens
oder Euro. Ein Intervall von 30 bis 86400 Sekunden begrenzt die Startfrequenz;
ohne verbleibendes Budget startet keine neue Bewertung. Journal, Wissensnotizen
und Ereignisse bleiben nach Budgeterschöpfung und Neustart erhalten.

## Noch offen

Ticketplanung mit Abhängigkeiten und Parallelitätslimits, belegte automatische
Antworten, echte Token-/Kostenbudgets, automatische Neuaufteilung und das Beenden
obsoleter **laufender** Sessions mit bestätigter Prozessruhe sind weitere Schritte.
Die automatische Wissensübernahme ist zunächst eine quellenverknüpfte
Modellbeobachtung. Branch-Gültigkeit und explizite revisionsgeschützte Ablösung sind
implementiert; automatische semantische Widerspruchsauflösung bleibt offen.
Der Supervisor wird im Moment per API/CLI eingerichtet, nicht über einen eigenen
Ratatui-Dialog oder die noch ausstehende Android-App.

## Lokale Abnahme

- Store-Tests: autonome Compaction und Abschluss, atomare Wissensübernahme,
  Neustart ohne Doppelaktion, unverbrauchtes Restbudget, deaktivierte Policies,
  veraltete Beobachtungen, ungültige Modellantworten und Rollback bei inzwischen
  fertigem Archiv.
- `scripts/smoke-agent.py`: echter Modell-Supervisor auf einem separaten Worktree,
  Factory-/Runner-Neustart während der Bewertung, selbst entschiedener Abschluss,
  verknüpfte Wissensnotiz und erschöpftes Bewertungsbudget erfolgreich geprüft.
  Die Supervisor-Session hatte ausschließlich `deny`-Toolberechtigungen und keine
  Toolaufrufe. Derselbe Test prüft native Compaction und anschließende Fortsetzung
  einer Arbeitssession mit erhaltenem vorherigem Kontext.
# Realmodell-Abnahme und Bildbelege

Ab Entwicklungsstand **0.1.3-dev** erhaelt der Supervisor auch die bereits
validierten Bilder des bewerteten Auftrags. Ohne diese Vorlage konnte er ein
bildabhaengiges Ziel trotz erfolgreicher Ausfuehrung nicht bestaetigen; drei
Auswertungen blieben nachvollziehbar bei `wait`. Die Bilder bleiben untrusted
Aufgabenbelege und erweitern keine Toolberechtigungen.

Mit explizitem `openai/gpt-6.2-sol` wurde der autonome Abschluss nach der
15-Minuten-Folgepromptfrist und Factory-/Runner-Neustart erfolgreich geprueft:
einmaliger Abschluss, erhaltenes finales Archiv und quellverknuepfte Notiz.
Beleg: `/tmp/opencode/factory-image-2gy6j6xr/supervisor-acceptance.json`.
Regressionstest prueft Bilduebergabe bei Compact/Close/Terminate samt
journalisiertem Budget und einmaliger Anwendung. Reproduzierbarer Vollablauf:
`scripts/smoke-prompt-image.py --local-registration PATH --provider openai --model gpt-6.2-sol --supervisor`.
Die Bildkorrektur ist noch nicht Bestandteil des veroeffentlichten v0.1.2.

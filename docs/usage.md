# Beobachteter Token- und Kostenverbrauch

TUI: in der Projektliste **u** drücken. CLI/API:

```sh
yoloforge usage PROJECT_ID
# GET /v1/projects/PROJECT_ID/usage
```

Die Auswertung liest archivierte OpenCode-Assistant-Nachrichten und gruppiert
nach Rolle (Arbeit, Routing, Fragen, Gedächtnis, Supervisor) und gemeldetem
Provider/Modell. Sie zeigt Input-, Output-, Reasoning- und Cache-Zähler getrennt.
Reasoning-/Cache-Zähler werden nicht blind zu einer Token-Gesamtsumme addiert.

Pro nativer Session und Message-ID zählt nur der neueste archivierte Stand.
Wiederholte Archivübertragung, Message-Versionen und vollständige
Session-Snapshots bei Fortsetzungen erhöhen den Verbrauch nicht erneut.
Die Herkunft bleibt im unveränderten Archiv nachprüfbar.

Fehlende oder ungültige Token-/Kostenangaben werden als fehlende Abdeckung gezählt,
nicht als kostenlos. Angezeigte USD-Werte sind ausschließlich von OpenCode
gemeldete Werte, keine Rechnung und keine Schätzung aus einer Preisliste.
Insbesondere lässt sich daraus keine Abrechnung einer OAuth-Flatrate ableiten.
Unfertige Nachrichten werden gekennzeichnet; nicht archivierte Nachrichten sind
nicht erfasst. Historische Archive funktionieren ohne Backfill oder neue Tabelle.

## Optionale Tokenlimits

TUI: Projektliste **b**, drei Grenzen bearbeiten, **Ctrl-S** speichern;
**u** zeigt Verbrauch und konfigurierte Grenzen. Standard ist jeweils
`unlimited`. `0` sperrt neue Starts dieser Kategorie.

```sh
yoloforge token-budget show PROJECT_ID
yoloforge token-budget set PROJECT_ID --expected-revision 0 \
  --work 1000000 --orchestration 200000 --memory 100000 \
  --request-id REQUEST_UUID
```

Diese Zahlen sind Beispiele. Alle drei Felder sind beim Setzen erforderlich;
`unlimited` entfernt die entsprechende Grenze. Bei unklarer Zustellung denselben
Request mit identischem Inhalt wiederholen. API: `GET/PUT
/v1/projects/PROJECT_ID/token-budget`. Die Budgetrevision ist unabhängig von der
Projektprofilrevision; veraltete Änderungen werden abgewiesen. Server-Schema: **30**.

- **Arbeit**: normale Aufgaben und Fortsetzungen.
- **Orchestrierung**: Routing, Auswertung von Berechtigungsanfragen und Supervisor. Formularfragen warten auf deine Antwort.
- **Gedächtnis**: automatische Extraktion und Widerspruchskorrektur.

Gezählt werden gemeldete **Input + Output** seit Projektbeginn, über alle Modelle
der jeweiligen Kategorie. Reasoning-/Cache-Unterzähler werden nicht zusätzlich
addiert. Das ist ein fest definierter Factory-Zähler, keine Nachbildung des
Subscription-Kontingents eines Providers und keine USD-Kostenbegrenzung.
Grenzen lassen sich erhöhen oder entfernen; es gibt keinen automatischen Reset.

Factory prüft atomar vor dem Einplanen und nochmals vor der Runner-Annahme eines
Auftrags. Neue Kompaktierungen prüfen das Budget ihrer Sessionkategorie ebenfalls.
Bereits angenommene Aufträge laufen weiter; sie können das Limit überschreiten,
auch durch mehrere parallele Sessions. Es handelt sich daher um eine Startsperre
anhand beobachteten Verbrauchs, nicht um eine harte Tokenobergrenze innerhalb
eines Modellaufrufs. Fehlende Tokenangaben oder fehlende finale Archive vorheriger
Aufträge sperren weitere Starts einer begrenzten Kategorie, statt als kostenlos
zu gelten. Andere Kategorien und Projekte behalten ihre eigenen Grenzen.

Die Limits ersetzen keinen modellspezifischen Tokenizer für die Planung von
Kontextfenstern. Modell-/Provider-Fallbacks werden dadurch nicht eingeführt.

Abnahme: `python3 scripts/smoke-token-budgets.py` prüft mit echtem HTTP/CLI und
synthetischen Verbrauchsdaten die Startsperre, erneute Prüfung wartender Aufträge,
Weiterlaufen angenommener Aufträge, den exakten beobachteten Grenzwert und
identischen Replay nach Serverneustart. Kein Modellaufruf erforderlich.

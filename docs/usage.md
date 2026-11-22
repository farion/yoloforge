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

Diese Anzeige setzt noch keine Kostenlimits durch und ersetzt keinen
modellspezifischen Tokenizer für die Planung von Kontext-/Gedächtnisbudgets.

# Geräteverwaltung in der TUI

In der Hauptansicht öffnet `7` die Geräteverwaltung. Ein Admin-Credential oder
das Bootstrap-Credential ist erforderlich; Operator-/Runner-Credentials erhalten
keinen Zugriff auf diese Verwaltungsendpunkte.

- **Liste:** Gerätename, Rolle, aktiver/widerrufener Zugang, Runner-Scope und
  Geräte-ID. Pfeile wählen das Gerät; die Auswahl bleibt bei Aktualisierungen
  an dessen ID gebunden. Die Liste wird einmal pro Sekunde abgeglichen.
- **Enrollment:** `n` öffnet das Formular. Standardrolle ist Operator;
  links/rechts im Rollenfeld wählt Runner oder Admin. Tab wechselt das Feld.
  Runner benötigt eine explizite gültige Runner-UUID. Laufzeit: 30–900 Sekunden,
  standardmäßig 300. Ctrl-S erzeugt genau einen Einmalcode.
- **Code:** Nur in der Ergebnisansicht sichtbar, keine automatisch angelegte
  Datei. Esc blendet ihn aus; nach Ablauf wird er ebenfalls ausgeblendet.
  Eine nach Schließen eintreffende Antwort öffnet die Codeansicht nicht erneut.
  Den Code auf dem Zielgerät über den vorhandenen CLI-Enrollment-Ablauf einlösen:
  `yoloforge device enroll --name NAME --code-file DATEI --output TOKEN-DATEI`.
- **Widerruf:** `r` öffnet den ausgewählten Zugang mit Name, ID, Rolle und Scope.
  Ctrl-S widerruft genau diese ID; Esc bricht ab. Eine parallele Listenänderung
  kann die Aktion nicht auf ein anderes Gerät umlenken.
- **Netzfehler:** Die letzte Liste bleibt als veraltet sichtbar. Ohne bestätigten
  Admin-Zugriff werden keine neuen Aktionen gesendet. Ein unklarer Widerruf kann
  mit Ctrl-S idempotent für dieselbe ID wiederholt werden. Eine unklare
  Enrollment-Erzeugung wird nicht automatisch oder per erneutem Ctrl-S wiederholt;
  ein neuer Code erfordert ausdrücklich einen neuen Dialog über Esc und `n`.

Esc in der Geräteliste kehrt zur Hauptansicht zurück. Das Einlösen auf einem neuen
Gerät und Schreiben seiner Credential-Datei bleibt beim CLI-Befehl; der Dialog
verwaltet die Zugänge der bereits angemeldeten Admin-Instanz.

Lokale Abnahme: Unit-Tests für unklare Zustellung, Ablauf/Ausblenden, unveränderliche
Widerrufsziele und Runner-Validierung. Der echte Ratatui-/HTTP-/SQLite-PTY-Test
prüft Operator- und Runner-Enrollment, einmalige Einlösung, Rollenbeschränkung,
Code-Ausblenden, wirksamen Widerruf und Offline-Anzeige.

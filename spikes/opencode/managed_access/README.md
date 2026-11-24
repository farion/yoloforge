# Spike: Factory-weite Modellzugänge

Der Spike verprobt nur den Modellpfad. Er ändert keine Factory-Dienste, GUI,
Benutzer-Konfiguration, bestehende OpenCode-Anmeldungen oder Aufgabendaten.
Getestet gegen **OpenCode V2 2.4.1** und Python 3.13.

```sh
python3 -B spikes/opencode/managed_access/smoke.py
```

Der Smoke-Test startet zwei normale OpenCode-Prozesse auf Loopback mit eigenen
Homes und XDG-Verzeichnissen. Er verwendet ein vorhandenes Binary (optional
`--binary /pfad/zu/opencode`), startet aber weder dessen normalen Hintergrunddienst
noch übernimmt er dessen Anmeldungen. Der Test verweigert den Start bei einer
fremden System- oder Elternverzeichnis-Konfiguration, statt sie einzulesen.
Die beiden Prozesse und ihre temporären Daten werden beim Beenden entfernt.

Der synthetische Provider prüft:

- dieselbe benannte Verbindung gleichzeitig auf beiden OpenCodes,
- zwei verschieden benannte Verbindungen desselben Anbieters parallel, obwohl
  im synthetischen Store nur eine als aktiv markiert ist,
- Streaming und Ausführung eines Lesetools im Runner-OpenCode inklusive zweitem
  Modellaufruf mit dem Tool-Ergebnis,
- leere Provider-Credential-Stores auf beiden Runnern, wirksame erlaubte
  Repo-Einstellung und keine eingelesene Konfiguration aus `OPENCODE_CONFIG`,
- gesperrte Anfragen bei falschem Runner-Schlüssel, Modell, Zugang oder bei
  abgelaufener Anmeldung; kein Ausweichen auf den anderen Zugang.

## Echte ChatGPT-Abnahme (noch offen)

Im eigenen Terminal starten, zwei **verschiedene** zugängliche ChatGPT-Konten
bereithalten und die angezeigten Browser-/Device-Schritte durchführen. Keine
OAuth-Codes, URLs oder Tokens in Chat oder Logs kopieren.

```sh
python3 -B spikes/opencode/managed_access/smoke.py --chatgpt --model gpt-6.2-sol
```

Bei Bedarf ein anderes in **beiden** Konten verfügbares Modell angeben. Der Test
startet zusätzlich ein leeres zentrales OpenCode, führt zwei native OAuth-Logins
aus und nutzt dafür keine bestehende Benutzeranmeldung. Danach macht er vier
echte Modellaufrufe: zwei über `Arbeit`, dann je einen über `Arbeit` und `Privat`
parallel. Die beiden Zugangsdaten liegen nur im temporären zentralen OpenCode;
beim Beenden wird diese Testinstanz mitsamt ihrem Datenspeicher entfernt.

## Architektur und Grenzen

Jede Modellreferenz auf Runner-Seite zeigt auf den Factory-Relay. Die Factory
ordnet einen benannten Zugang einer festen OpenCode-Credential-ID zu. Der Relay
liest den ausgewählten Zugang ausschließlich über die private zentrale API und
leitet einen Responses-Stream weiter. Er wechselt beim Modellaufruf nie die
aktive OpenCode-Anmeldung. Auf den Runner gelangt nur ein Testschlüssel für den
Relay, kein OAuth-Access- oder Refresh-Token. Der ChatGPT-Adapter verwendet den
in OpenCode 2.4.1 beobachteten ChatGPT-Endpunkt; dessen Kompatibilität mit dem
weitergeleiteten Request muss die echte Abnahme noch bestätigen.

Das ist **kein produktiver Broker**: Nur Loopback, keine getrennten Hosts oder
OS-Benutzer und kein Missbrauchstest gegen ein bösartiges Repo. Automatische
Token-Erneuerung, persistenter Login über Factory-Neustarts, Anbindung an das
Factory-Runner-Protokoll sowie Installation und Lifecycle eines verwalteten
Binaries sind noch nicht implementiert. Ein abgelaufenes Token blockiert den
Testzugang ausdrücklich.

Die Vorprüfung validiert die erprobten JSON/JSONC-Dateien vor Prozessstart und
weist Anbieter- und Modellfelder inklusive Agenten-/Befehlsvorgaben mit Datei
und Feld ab. Produktionsreife erfordert zusätzlich die Validierung aller
Ladepfade, unter anderem Markdown-Frontmatter, und Schutz gegen nachträgliche
Änderungen sowie ausführbare Plugins. Der Spike ist kein Ersatz für diese
Grenze.

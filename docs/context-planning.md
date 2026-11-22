# Tokenisierte Kontextauswahl

Das versionierte Projektprofil kann einen Tokenizer fuer das explizit gewaehlte
Projektmodell binden. Es gibt keine Modell-/Provider-Ausweichwahl und keine
geratene Tokenizer-Zuordnung fuer unbekannte Modellnamen.

```json
"context_planning": {
  "tokenizer": "o200k_base",
  "knowledge_tokens": 2048
}
```

Unterstuetzt: `cl100k_base`, `o200k_base`, `p50k_base`. Die Vokabulare sind im
Server-Binary enthalten; es werden keine Texte an Tokenizer-Dienste uebertragen.
Die Auswahl zaehlt den tatsaechlichen zusammengefuegten UTF-8-Text einschliesslich
Notiz-ID, Revision, Titel und Herkunft. Token-aehnliche Spezialzeichenfolgen im
Notiztext werden als gewoehnlicher Text behandelt. Notizen werden nur vollstaendig
uebernommen; ein zu grosser Eintrag verhindert nicht die Auswahl kleinerer
nachfolgender Eintraege. Prioritaet und Branch-/Ticketscope bleiben erhalten.

CLI: `yoloforge project configure PROJECT --expected-revision REV --provider PROVIDER
--model MODEL --context-tokenizer o200k_base --knowledge-tokens 2048` (Profilersetzung:
auch vorhandene Toolchain-/Start-/Testwerte mitgeben). TUI: optionales Feld
**Kontextplanung JSON** im Profil-/Projekteditor. API: bestehendes revisions-
geschuetztes Projektprofil. `knowledge context` liefert `token_plan` mit Tokenizer,
gemessenen Tokens und Budget. Null Tokens unterdruecken Wissensnotizen.

Ohne Konfiguration gilt unveraendert das bisherige Bytebudget. Zusaetzlich zum
Tokenbudget bleibt die Byteobergrenze bestehen. Maximal 16384 Wissenstokens sind
konfigurierbar. Diese Messung betrifft den injizierten Wissenstext, nicht den
gesamten OpenCode-Verlauf, Toolschemas, Bilder oder die vom Provider erzeugten
Tokens. Session-Compaction und kumulative Verbrauchslimits sind getrennte Regeln.
Runner muessen `context-planning-v1` unterstuetzen; aeltere Runner erhalten solche
Auftraege nicht. Bereits erstellte Auftragsprompts bleiben unveraendert.

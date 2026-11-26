# Projektbereitschaft und Anmeldung

In der Projektliste:

- **r** liest den Status des lokalen OpenCode-Service fuer den bestaetigten
  Projektarbeitsordner: Standard-/Projektmodell, hinterlegte Anmeldungen und
  MCP-Verbindungen. Pfeiltasten scrollen, Esc kehrt zur Projektliste zurueck.
- **o** oeffnet die originale OpenCode-Oberflaeche fuer diesen Arbeitsordner,
  auch wenn noch keine Arbeitssession existiert. Dort sind Provider-/OAuth- und
  MCP-Anmeldedialoge erreichbar (`/mcps` fuer MCP). Anschliessend OpenCode beenden
  und mit **r** den Status erneut lesen.

Die Abfrage sendet keinen Modellprompt und exportiert weder Zugangsdaten noch
OAuth-URLs oder rohe Providerfehler. Ein hinterlegter Zugang wird nicht mit einem
erfolgreichen Modellaufruf gleichgesetzt. `needs_auth` wird ausdruecklich angezeigt.
Antworten sind an den gewaehlten Projektordner gebunden; verspätete Statusantworten
eines zuvor geschlossenen Dialogs werden verworfen.

Der Einstieg ist lokal. Remote-Anmeldung und -Bereitschaft ueber Factory sind noch
nicht implementiert. Native SSH-Arbeitssessions siehe `ssh-sessions.md`.

Abnahme: Adaptertest mit abgelaufenem Zugang und MCP-Anmeldebedarf.
Eine wirkliche OAuth-Anmeldung/Erneuerung wurde dabei nicht ausgefuehrt.

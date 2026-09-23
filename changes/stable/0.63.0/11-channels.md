---
type: added
---
[en]
**A channel is a declaration, and the declaration is the workflow.** Write down its members and it fans out to them; add `expects:` and it waits for a quorum; add `timeout:` and a silent member releases the round instead of holding it forever. `on_complete:` decides what the requester gets back — every answer as it stands, or one folded summary. `flow: sequential` turns the member list into an order, so a channel IS a workflow: same declaration, one step at a time. A channel can also be `public` — a project's front door, readable and addressable across project boundaries.
[de]
**Ein Kanal ist eine Deklaration, und die Deklaration ist der Ablauf.** Schreiben Sie seine Mitglieder hin, und er fächert zu ihnen auf; ergänzen Sie `expects:`, und er wartet auf ein Quorum; ergänzen Sie `timeout:`, und ein verstummtes Mitglied gibt die Runde frei, statt sie ewig zu halten. `on_complete:` entscheidet, was zurückkommt — jede Antwort, wie sie steht, oder eine gefaltete Zusammenfassung. `flow: sequential` macht aus der Mitgliederliste eine Reihenfolge, ein Kanal IST damit ein Ablauf: dieselbe Deklaration, ein Schritt nach dem anderen. Ein Kanal kann außerdem `public` sein — die Vordertür eines Projekts, über Projektgrenzen hinweg lesbar und ansprechbar.

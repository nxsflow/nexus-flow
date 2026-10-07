---
type: changed
facade: changed
---
[en]
Which tickets are ready, in progress, deferred or blocked — and how `next` tiers them — is now
decided by one Rust library over the active tickets and their dependencies and parents
(`nexus_flow_core::graph`). The board selects those tickets with SQL; a server can select them from
its own index and get exactly the same lanes. One rule comes with it: a ticket that is not active —
closed, archived or deleted — counts as closed wherever another ticket depends on it or hangs under
it. Three answers change, all in states only a merge of concurrent edits produces: a dependency on
an archived ticket that was opened again no longer blocks; an open ticket whose only parent was
deleted rests as it would under a closed parent; and a dependency cycle that runs through a closed
ticket no longer blocks anything. `nxf blocked` lists only active blockers, and the "blocks N" count
in `nxs prime` counts only active dependents.
[de]
Welche Tickets bereit, in Arbeit, zurückgestellt oder blockiert sind — und wie `next` sie in Stufen
ordnet —, entscheidet jetzt eine Rust-Bibliothek über die aktiven Tickets mit ihren Abhängigkeiten
und Elternteilen (`nexus_flow_core::graph`). Das Board wählt diese Tickets per SQL aus; ein Server
kann sie aus seinem eigenen Index auswählen und bekommt genau dieselben Spalten. Dazu gehört eine
Regel: Ein Ticket, das nicht aktiv ist — geschlossen, archiviert oder gelöscht —, gilt überall als
geschlossen, wo ein anderes von ihm abhängt oder unter ihm hängt. Drei Antworten ändern sich, alle
in Zuständen, die nur das Zusammenführen nebenläufiger Änderungen erzeugt: Eine Abhängigkeit von
einem archivierten, wieder geöffneten Ticket blockiert nicht mehr; ein offenes Ticket, dessen
einziges Elternteil gelöscht wurde, ruht wie unter einem geschlossenen Elternteil; und ein
Abhängigkeitszyklus über ein geschlossenes Ticket blockiert nichts mehr. `nxf blocked` nennt nur
noch aktive Blocker, und die Zahl „blockiert N“ in `nxs prime` zählt nur noch aktive abhängige
Tickets.

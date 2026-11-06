---
type: fixed
facade: none
---
[en]
Opening a workspace database that has only just been created no longer fails with `failed to set sqlite pragmas: database is locked` when another connection is writing to it at that moment, for example when two `nxf`, `nxm` or `nxc` processes start together on a new workspace, or a watching app re-opens a database that was deleted and recreated. The switch to WAL mode now tries again for up to 5 seconds instead of giving up at once.
[de]
Das Öffnen einer gerade erst angelegten Workspace-Datenbank schlägt nicht mehr mit `failed to set sqlite pragmas: database is locked` fehl, wenn in diesem Moment eine andere Verbindung darauf schreibt, etwa wenn zwei `nxf`-, `nxm`- oder `nxc`-Prozesse gleichzeitig in einem neuen Workspace starten oder eine beobachtende App eine gelöschte und neu angelegte Datenbank wieder öffnet. Die Umstellung auf den WAL-Modus versucht es jetzt bis zu 5 Sekunden lang erneut, statt sofort aufzugeben.

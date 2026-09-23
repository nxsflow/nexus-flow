---
type: added
---
[en]
**Project memory is a file now.** `NEXUS_MEMORY.md` at the workspace root is a generated projection of the `nxm` store, so the context survives for a reader without nexus-flow installed — change it with `nxm remember` / `nxm forget`, never by hand, and `nxm doc --check` reports drift. Memories carry a category, a reach (this workspace or everywhere), the board items they are about and a reading order, and `nxs prime` hands them to a session in that order instead of alphabetically. `nxm migrate` moves a workspace off a hand-maintained `CLAUDE.md`.
[de]
**Projektwissen ist jetzt eine Datei.** `NEXUS_MEMORY.md` in der Workspace-Wurzel ist eine erzeugte Projektion des `nxm`-Speichers, damit der Kontext auch für einen Leser ohne installiertes nexus-flow erhalten bleibt — ändern Sie sie mit `nxm remember` / `nxm forget`, nie von Hand; `nxm doc --check` meldet Abweichungen. Erinnerungen tragen Kategorie, Reichweite (dieser Workspace oder überall), die Board-Items, um die es geht, und eine Lesereihenfolge; `nxs prime` übergibt sie einer Sitzung in dieser Reihenfolge statt alphabetisch. `nxm migrate` löst einen Workspace von einer handgepflegten `CLAUDE.md`.

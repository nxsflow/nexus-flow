---
type: fixed
facade: changed
---
[en]
Safety fix: a persona declared with `tools: []` can no longer run arbitrary shell commands when it
owes a reply. Until now the engine granted every such session the bare, auto-approved `Bash` so that
it could answer, which gave a persona declared to have no tools a full shell (`gh pr merge`,
`git push --force`, a release). It now grants `Bash(nxc reply:*)`: the reply runs, every other
command needs an approval nobody is there to give and is refused. A persona whose `tools:` is absent,
or lists `Bash`, behaves as before; one that declares `Bash(nxc reply:*)` itself now stays that
narrow. For an embedding app's own worker: `RoleSpec::granted_tools` (`grantedTools` in the sidecar
spec) can now carry a scoped permission rule rather than a tool name — put the tool it names into the
SDK's `tools` and the rule itself into `allowedTools`, as the bundled sidecar does.
[de]
Sicherheitskorrektur: Eine Persona mit `tools: []` kann keine beliebigen Shell-Befehle mehr
ausführen, wenn sie eine Antwort schuldet. Bisher gewährte die Engine jeder solchen Sitzung das
nackte, automatisch freigegebene `Bash`, damit sie antworten konnte — eine Persona ohne Werkzeuge
hatte damit eine volle Shell (`gh pr merge`, `git push --force`, ein Release). Jetzt gewährt sie
`Bash(nxc reply:*)`: Die Antwort läuft, jeder andere Befehl braucht eine Freigabe, die niemand
erteilen kann, und wird abgewiesen. Eine Persona ohne `tools:` oder mit `Bash` in der Liste verhält
sich wie bisher; eine, die selbst `Bash(nxc reply:*)` deklariert, bleibt jetzt so eng. Für den
eigenen Worker einer einbettenden App: `RoleSpec::granted_tools` (`grantedTools` in der
Sidecar-Spezifikation) kann jetzt eine eingeschränkte Berechtigungsregel statt eines Werkzeugnamens
tragen — das genannte Werkzeug gehört in `tools` des SDK, die Regel selbst in `allowedTools`, wie es
die mitgelieferte Sidecar tut.

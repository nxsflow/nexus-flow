---
type: added
---
[en]
**One chain per working copy.** A persona or channel that declares `working_tree: exclusive` gets sole use of the repository checkout and build directory for as long as its task runs; a second chain that would collide waits instead of running its build against the same target directory. The protection is derived, not repeated: declare it on the persona that builds, and every channel that persona is a declared member of counts as needing it too.
[de]
**Eine Kette je Arbeitskopie.** Eine Persona oder ein Kanal mit `working_tree: exclusive` bekommt die Arbeitskopie und das Build-Verzeichnis für die Dauer ihrer Aufgabe allein; eine zweite Kette, die kollidieren würde, wartet, statt ihren Build gegen dasselbe Zielverzeichnis laufen zu lassen. Der Schutz wird abgeleitet, nicht wiederholt: deklarieren Sie ihn an der Persona, die baut, und jeder Kanal, in dem diese Persona deklariertes Mitglied ist, gilt ebenfalls als schützenswert.

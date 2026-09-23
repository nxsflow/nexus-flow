---
type: fixed
---
[en]
**An agent session starts, stays single, and does not lose its answer.** On macOS every spawned persona session used to die at authentication; it now starts. One internal session runs exactly one process — a wake arriving while that session is still working is refused and reported instead of starting a second agent in the same working directory, where the two overwrote each other's edits. A session that dies at an error is machine-readably distinguishable from one that answered, and a failed transcript write no longer costs the turn.
[de]
**Eine Agenten-Sitzung startet, bleibt einzeln und verliert ihre Antwort nicht.** Auf macOS starb bisher jede gespawnte Persona-Sitzung an der Authentifizierung; sie startet jetzt. Eine interne Sitzung führt genau einen Prozess — ein Wecken, das eintrifft, während diese Sitzung noch arbeitet, wird abgelehnt und gemeldet, statt einen zweiten Agenten im selben Arbeitsverzeichnis zu starten, wo beide einander die Änderungen überschrieben. Eine Sitzung, die an einem Fehler stirbt, ist maschinell von einer beantworteten unterscheidbar, und ein fehlgeschlagener Verlaufsschreibvorgang kostet nicht mehr den Zug.

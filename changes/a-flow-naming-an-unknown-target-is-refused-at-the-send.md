---
type: fixed
facade: none
---
[en]
A send to a channel on the new engine is now refused when one of its steps would start a persona the declarations do not hold: a step target that is neither a persona nor a channel the flow fans out to, or a member of a nested channel that no persona declaration resolves. The refusal names the step and the target, says `declarations.persona` refused, and nothing is written: no run is opened and no flow is stored. Before, such a flow was accepted and the run could only fail when that step was due to start.
[de]
Eine Sendung an einen Kanal auf der neuen Engine wird jetzt abgelehnt, wenn einer ihrer Schritte eine Persona starten würde, die die Deklarationen nicht kennen: ein Schrittziel, das weder eine Persona noch ein Kanal ist, an den der Ablauf verteilt, oder ein Mitglied eines verschachtelten Kanals, das keine Persona-Deklaration auflöst. Die Ablehnung nennt den Schritt und das Ziel, sagt, dass `declarations.persona` abgelehnt hat, und es wird nichts geschrieben: Kein Lauf wird eröffnet und kein Ablauf gespeichert. Bisher wurde ein solcher Ablauf angenommen, und der Lauf konnte erst scheitern, wenn dieser Schritt starten sollte.

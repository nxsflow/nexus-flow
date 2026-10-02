---
type: changed
facade: changed
---
[en]
`nxf show` now points at an item's parents in a neutral line — `Parents: <ids> (read for full context)` —
instead of "URGENT RECOMMENDATION: … !!!". Agents reading the board had flagged the old line as a
possible prompt injection. The `parents_notice` field in `--json` and on the facade carries the same
new text; its name and when it appears are unchanged.
[de]
`nxf show` verweist auf die Eltern eines Eintrags jetzt in einer neutralen Zeile —
`Parents: <ids> (read for full context)` — statt mit „URGENT RECOMMENDATION: … !!!“. Agenten, die das
Board lesen, hatten die alte Zeile als möglichen Prompt-Injection-Versuch gemeldet. Das Feld
`parents_notice` in `--json` und in der Fassade trägt denselben neuen Text; Name und Bedingung, wann es
erscheint, bleiben gleich.

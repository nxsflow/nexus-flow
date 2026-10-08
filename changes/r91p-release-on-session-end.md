---
type: fixed
facade: changed
---
[en]
A run that claims the working copy now lets it go as soon as its last step's session ends and
the result is delivered. Before, the release was only asked when a step replied — and the last
step's reply comes while its session is still running, so the answer was no — and the run held the
working copy until the latest deadline any of its threads had declared, often hours, with the next
runs queued behind it. If that release cannot be decided, the session-end receipt now says so
(`lease_undecided`, new in `ConsequenceClass` for embedding apps) instead of only printing to the
sidecar's standard error.
[de]
Ein Lauf, der die Arbeitskopie für sich beansprucht, gibt sie jetzt frei, sobald die Session seines
letzten Schritts endet und das Ergebnis zugestellt ist. Bisher wurde die Freigabe nur geprüft, wenn
ein Schritt antwortete — und die Antwort des letzten Schritts kommt, während seine Session noch
läuft, also lautete die Antwort nein. Der Lauf hielt die Arbeitskopie dann bis zur spätesten Frist,
die einer seiner Fäden erklärt hatte, oft Stunden, und die nächsten Läufe warteten dahinter. Lässt sich diese Freigabe nicht entscheiden, sagt es die Quittung des Session-Endes jetzt
(`lease_undecided`, für einbettende Apps neu in `ConsequenceClass`), statt es nur auf die
Standardfehlerausgabe des Sidecars zu schreiben.

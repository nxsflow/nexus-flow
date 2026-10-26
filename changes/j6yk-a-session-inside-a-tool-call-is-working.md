---
type: fixed
facade: changed
---
[en]
A session inside one long tool call is no longer taken for a silent one. On 0.220.0 a step that watched a CI run of about an hour, as the merger, verifier and finisher do, was asked three times whether it was still working and then escalated as unrecoverable, although its process was alive and the command was still running; an ask only reaches a session once its tool call returns, so it could never have answered in time. Now the coordinator reads a tool call that has not returned as work: it asks nothing and escalates nothing while the call runs. A tool call that runs longer than three hours is taken as hung and escalated at once, naming the tool and when it started, with no ask spent on it. A session that is really silent, with no tool call open, is still asked after 20 minutes and escalated after three asks, as before. The sidecar now writes a tool call that is still running after two seconds to the transcript, so the coordinator can see it.

For embedders: `ladder::TOOL_CALL_BOUND_SECS` is the new bound of one tool call.
[de]
Eine Sitzung, die in einem einzigen langen Werkzeugaufruf steckt, gilt nicht mehr als stumm. Unter 0.220.0 wurde ein Schritt, der einen etwa einstündigen CI-Lauf beobachtete, wie es Merger, Verifier und Finisher tun, dreimal gefragt, ob er noch arbeitet, und dann als nicht wiederherstellbar eskaliert, obwohl sein Prozess lebte und der Befehl noch lief; eine Frage erreicht eine Sitzung erst, wenn ihr Werkzeugaufruf zurückkehrt, sie hätte also nie rechtzeitig antworten können. Jetzt liest der Koordinator einen Werkzeugaufruf, der noch nicht zurückgekehrt ist, als Arbeit: Solange der Aufruf läuft, fragt er nicht und eskaliert nicht. Ein Werkzeugaufruf, der länger als drei Stunden läuft, gilt als hängend und wird sofort eskaliert, mit dem Werkzeug und dem Zeitpunkt seines Beginns, ohne dass dafür eine Frage verbraucht wird. Eine Sitzung, die wirklich stumm ist und keinen offenen Werkzeugaufruf hat, wird wie bisher nach 20 Minuten gefragt und nach drei Fragen eskaliert. Der Sidecar schreibt einen Werkzeugaufruf, der nach zwei Sekunden noch läuft, jetzt ins Transkript, damit der Koordinator ihn sieht.

Für Einbettende: `ladder::TOOL_CALL_BOUND_SECS` ist die neue Frist eines einzelnen Werkzeugaufrufs.

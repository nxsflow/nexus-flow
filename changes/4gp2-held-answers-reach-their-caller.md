---
type: fixed
facade: changed
---
[en]
An answer that reaches a caller while it is still finishing its turn is now delivered. On the
command line the background service was handed the delivery as a channel deadline on a session id
("no such thread"), so a held answer was never handed over; and the caller's sidecar, seeing its
sub-round already answered, reminded it and then posted a substitute reply in its name. A sub-round
whose answer is held for its commissioner now counts as still in flight (`waiting_on_sub_round` in
`nxc status --json` and on the facade). Also fixed: a persona that declares no `tools:` and is woken
with an answer it still has to pass on is granted its `nxc reply` (the wake carries the obligation it
still has), and a session spawned by a development build resolves that build's service home, not the
installed one.
[de]
Eine Antwort, die einen Aufrufer erreicht, während er seinen Zug noch beendet, wird jetzt
zugestellt. Auf der Kommandozeile bekam der Hintergrunddienst die Zustellung als Kanal-Frist auf
eine Sitzungs-ID („no such thread“), eine zurückgehaltene Antwort wurde also nie übergeben; und die
Sidecar des Aufrufers sah seine Unterrunde schon beantwortet, erinnerte ihn und postete dann eine
Ersatzantwort in seinem Namen. Eine Unterrunde, deren Antwort für ihren Auftraggeber zurückgehalten
wird, zählt jetzt weiter als unterwegs (`waiting_on_sub_round` in `nxc status --json` und in der
Fassade). Außerdem behoben: Eine Persona ohne `tools:`, die mit einer Antwort geweckt wird, die sie
noch weitergeben muss, darf ihr `nxc reply` ausführen (die Weckung trägt die Pflicht mit, die sie
noch hat), und eine Sitzung, die ein Entwicklungs-Build startet, nutzt dessen Dienst-Verzeichnis,
nicht das installierte.

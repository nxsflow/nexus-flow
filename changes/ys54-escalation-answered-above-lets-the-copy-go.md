---
type: fixed
facade: changed
---
[en]
An operation that had delivered could keep the working copy until its lease ran out, with the next
operation waiting behind it. In the field a verify step escalated. The requester answered on the
thread above it, which is the normal way to answer an escalation, and the run started again in new
step threads and delivered. Nothing was written in the escalating step's thread again, so its
escalation stayed its last reply, and the working copy was never released. Now an escalation counts
as answered once the commissioner of a thread above it has written in that thread and asked it
again, and the copy goes when that new turn is done. The engine moving a register on its own, with
no word from the commissioner, answers nothing. An escalation that nobody answered still holds the
working copy, as before, and a question is still answered only in its own thread. Such an operation
also no longer shows `NEEDS DECISION` once it has delivered. `nxc status` now says why a copy is
held when no thread is open: a line `working copy held: thread <id> …` under the operation that says
where the answer goes, and the new optional field `held_by_hand_back` (`thread` and `kind`) in
`--json` and on `StatusOperation`.
[de]
Ein Vorgang, der schon geliefert hatte, konnte die Arbeitskopie behalten, bis sein Lease ablief,
während der nächste Vorgang dahinter wartete. Im Einsatz eskalierte ein Verify-Schritt. Der
Auftraggeber antwortete im Faden darüber, wie man eine Eskalation üblicherweise beantwortet, und der
Durchlauf startete in neuen Schritt-Fäden neu und lieferte. Im Faden des eskalierenden Schritts
wurde nichts mehr geschrieben, also blieb seine Eskalation die letzte Antwort dort, und die
Arbeitskopie wurde nie freigegeben. Jetzt gilt eine Eskalation als beantwortet, sobald der
Auftraggeber eines Fadens darüber in diesen Faden geschrieben und ihn erneut gefragt hat, und die
Kopie geht, wenn dieser neue Durchgang erledigt ist. Bewegt die Maschine ein Register von sich aus,
ohne ein Wort des Auftraggebers, beantwortet das nichts. Eine Eskalation, die niemand beantwortet
hat, hält die Arbeitskopie wie bisher, und eine Frage wird weiterhin nur in ihrem eigenen Faden
beantwortet. Ein solcher Vorgang zeigt nach dem Liefern auch nicht mehr `NEEDS DECISION`.
`nxc status` sagt jetzt, warum eine Kopie gehalten wird, wenn kein Faden offen ist: eine Zeile
`working copy held: thread <id> …` unter dem Vorgang, die sagt, wohin die Antwort gehört, und das
neue optionale Feld `held_by_hand_back` (`thread` und `kind`) in `--json` und an `StatusOperation`.

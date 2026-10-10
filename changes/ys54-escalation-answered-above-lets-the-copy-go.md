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
as answered once a thread above it has been asked again after it, and the copy goes when that new
turn is done. An escalation that nobody answered still holds the working copy, as before.
`nxc status` now also says why a copy is held when no thread is open: a line
`working copy held: thread <id> handed the task back …` under the operation, and the new optional
field `held_by_hand_back` in `--json` and on `StatusOperation`.
[de]
Ein Vorgang, der schon geliefert hatte, konnte die Arbeitskopie behalten, bis sein Lease ablief,
während der nächste Vorgang dahinter wartete. Im Einsatz eskalierte ein Verify-Schritt. Der
Auftraggeber antwortete im Faden darüber, wie man eine Eskalation üblicherweise beantwortet, und der
Durchlauf startete in neuen Schritt-Fäden neu und lieferte. Im Faden des eskalierenden Schritts
wurde nichts mehr geschrieben, also blieb seine Eskalation die letzte Antwort dort, und die
Arbeitskopie wurde nie freigegeben. Jetzt gilt eine Eskalation als beantwortet, sobald ein Faden
darüber nach ihr erneut gefragt wurde, und die Kopie geht, wenn dieser neue Durchgang erledigt ist.
Eine Eskalation, die niemand beantwortet hat, hält die Arbeitskopie wie bisher. `nxc status` sagt
jetzt auch, warum eine Kopie gehalten wird, wenn kein Faden offen ist: eine Zeile
`working copy held: thread <id> handed the task back …` unter dem Vorgang und das neue optionale
Feld `held_by_hand_back` in `--json` und an `StatusOperation`.

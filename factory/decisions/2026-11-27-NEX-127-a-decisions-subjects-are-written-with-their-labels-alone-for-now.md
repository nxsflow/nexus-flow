# 2026-11-27 · NEX-127 · A decision's subjects are written with their labels alone, for now

- Decided by: coder on NEX-127
- Why: RFC 0003, "decisionsSubject", always writes `label`, and `ticket` or `elsewhere` only when the subject resolves. NEX-127's acceptance asks for the subjects and the range expansion, not for resolution. The labels it takes are ticket ids such as `NEX-22`, which name tickets of the tracker and not keys of `ticketsTicket`. So each subject is written with its label and date only, and the printed subject has `object`, `space` and `status` set to `null`. `--ticket NEX-22` still finds every one of them through `byLabel`. A decision takes no write after its change, so a resolved link can only be added for decisions written later.
- Instead of: resolving each label through `ticketsTicket.byKey` in this ticket, outside its acceptance criteria. pm decides whether that becomes a ticket.
- Topics: decisions, tickets, store, v2

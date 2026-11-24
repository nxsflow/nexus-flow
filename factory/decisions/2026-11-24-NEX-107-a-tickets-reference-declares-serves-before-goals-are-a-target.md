# 2026-11-24 · NEX-107 · A ticket's reference declares the kind serves before goals are a target

- Decided by: coder on NEX-107
- Why: RFC 0002, "References out of the space", gives `ticketsReference.kind` the options `serves` and `cites`, and `target` the collections `ticketsTicket` and `goalsGoal`; `goalsGoal` arrives with NEX-142, and a lock refuses a target that names no collection of the model. So `target` names `ticketsTicket` alone until then, while `kind` holds both options the RFC decided: the store writes no `serves` reference before `nxs ticket ref --serves` exists, and NEX-142 adds `goalsGoal` to `target` and `proposed` to `kind`, both additive.
- Instead of: leaving `serves` out until NEX-142, which would make it a second additive change to the same field for no gain.
- Topics: model, tickets, goals, v2

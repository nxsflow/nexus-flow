# 2026-11-24 · NEX-125 · A decision's subject in another space refers only to tickets until goals and requests exist

- Decided by: coder on NEX-125
- Why: RFC 0003, "decisionsSubject", gives `elsewhere` the targets `ticketsTicket`, `goalsGoal` and `requestsRequest`; a lock refuses a target that names no collection of the model, and `goalsGoal` arrives with NEX-142 (E6.T1) and `requestsRequest` with E8.T2. So `elsewhere` names `ticketsTicket` alone until then, and each of those tickets adds its collection to the field's targets, an additive change. Nothing writes a decision yet, so no subject is lost meanwhile: `label` holds every subject as written.
- Instead of: leaving `elsewhere` out until all three collections exist, which would declare `decisionsSubject` without the field the RFC decided and make a decision about another repository's ticket keep only its label.
- Topics: model, decisions, goals, requests, v2

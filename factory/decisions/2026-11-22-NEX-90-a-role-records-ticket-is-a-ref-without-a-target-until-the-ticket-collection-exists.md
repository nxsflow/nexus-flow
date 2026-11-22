# 2026-11-22 · NEX-90 · A role record's ticket is a ref without a target until the ticket collection exists

- Decided by: coder on NEX-90
- Why: a lock refuses a ref target that names no collection of the model, and the ticket collection arrives with NEX-107. `nc.ref([])` builds, and `ncdb model check` accepts adding `ticketsTicket` as its target later as an additive change (tried with ncdb 0.1.0-beta.1).
- Instead of: leaving `ticket` out until NEX-107, which would add a field to a collection already on v2, or declaring a placeholder ticket collection now, which the model could never remove.
- Topics: model, members, nocentraldb, v2

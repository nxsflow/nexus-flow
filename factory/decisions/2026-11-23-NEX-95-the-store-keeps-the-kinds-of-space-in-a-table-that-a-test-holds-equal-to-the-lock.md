# 2026-11-23 · NEX-95 · The store keeps each collection's kinds of space in a table that a test holds equal to the lock

- Decided by: coder on NEX-95
- Why: the kinds are per collection, not per module: RFC 0009 lets copies of requests into an ad-hoc space, but not a request's comments. So `space_kind::KINDS` pairs each collection name with its kinds, beside `model::COLLECTIONS`, and a test in `model_collections.rs` reads the lock's collections and fails when a collection has no kinds, or the table names one the lock does not. The check is also offered over a caller's table (`check_in`), so the rule and its codes are tested for the module collections the lock does not hold yet.
- Instead of: kinds per module, derived from the prefix, which cannot tell `requestsRequest` from `requestsComment`; or a third field in `model::COLLECTIONS`, which every reader of that table would then carry.
- Topics: store, model, v2

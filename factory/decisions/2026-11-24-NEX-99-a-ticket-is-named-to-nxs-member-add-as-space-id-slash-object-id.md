# 2026-11-24 · NEX-99 · A ticket is named to nxs member add as space id / object id

- Decided by: coder on NEX-99; provisional until the owner approves the PR
- Why: RFC 0001 writes `--ticket <ref>` without saying how a ref is written, and the record's `ticket` is an `nc.ref`, a space id and an object id. RFC 0002 writes a reference's target as `<space id>/<object id>`, so `--ticket` takes that form and `nxs member list` prints it so. A ticket key such as `ka.12` needs the ticket collection (NEX-107) to be resolved, so it is refused (`validation`) until then; resolving a key there is additive.
- Instead of: taking a ticket key now and storing it as text, which the `nc.ref` field cannot hold.
- Topics: members, tickets, nxs

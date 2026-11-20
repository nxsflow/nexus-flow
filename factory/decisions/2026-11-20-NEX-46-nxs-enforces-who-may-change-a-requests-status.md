# 2026-11-20 · NEX-46 · nxs enforces who may change a request's status

- Decided by: Carsten
- Why: agents get one protocol at no cost: the receiving side accepts, declines and marks done, the requesting side reopens, and nxs and the library seam refuse any other transition with `request_transition_not_allowed`. NoCentralDB cannot enforce it (rights apply to a whole space), so a client outside nexus-flow's library is not stopped. Written into RFC 0009, "Status".
- Instead of: no rule at all, with the thread as the only record, which would let a requesting pm mark its own request done.
- Topics: cross-repo, coordination, requests

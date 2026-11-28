# 2026-11-28 · NEX-109 · Minting reads the keys of a tag through byUpdatedAt

- Decided by: coder on NEX-109
- Why: RFC 0002 mints a key as the highest number among the keys with the member's tag in its copy, plus one, but NoCentralDB 0.1.0-beta.1 runs a query only with the index's whole equality prefix fixed, so `byKey` (`equal: 1`) finds one key and cannot list the keys of a tag. `byUpdatedAt` fixes nothing and holds every ticket of the space, `updatedAt` unset included, so minting reads every ticket's key through it and takes the highest number of its tag. No model change, and no command or seam symbol names the index. A deleted ticket is not read, so a key whose ticket was deleted as the tag's highest is minted again.
- Instead of: a new index on `ticketsTicket`, a model change for a read that the RFC's indexes already serve; or probing `byKey` for `ka.1`, `ka.2`, … until the first gap, which mints a key again after any deleted ticket, not only the highest.
- Topics: tickets, model, store

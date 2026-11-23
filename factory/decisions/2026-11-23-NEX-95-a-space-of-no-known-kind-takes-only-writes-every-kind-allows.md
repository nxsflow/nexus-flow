# 2026-11-23 · NEX-95 · A space's kind comes from nexus-flow.toml, and a space of no known kind takes only writes every kind allows

- Decided by: coder on NEX-95
- Why: how the store knows the kind of a space opened by id alone is open (NEX-95, "Refuses or parks safely"). Until it is answered, `Opened` takes the kind from the `nexus-flow.toml` of the repository it opened or of the checkout the caller named: `[space] repository` and `[space] coordination`. Any other space, among them one opened by id without a checkout and every ad-hoc space, has no known kind, and `Opened::check_write` refuses every collection the table does not allow in all three kinds, which today is every collection. Its error carries no code, since no RFC names one.
- Instead of: assuming a space opened by id is a repository space, which would let manufakt.io write a ticket into a coordination space.
- Topics: store, storage, v2

# 2026-11-28 · NEX-242 · A space's marker names its kind over nexus-flow.toml, and markers that disagree name none

- Decided by: coder on NEX-242
- Why: the owner made the `spaceKind` marker the source of a space's kind (NEX-205), and a space created before it has none. So `Opened::kind` takes the marker's kind first, also where `nexus-flow.toml` names the space as another kind; it takes the file's kind only for a space with no marker, which keeps NEX-95's behaviour for those. Two markers that name different kinds, which only a write outside the store can make, leave the space of no known kind, so it takes only the writes every kind allows. The markers are read when the space opens and when sync brings a space the database did not hold.
- Instead of: the file over the marker, which keeps a team's mistaken line in `nexus-flow.toml` deciding what is written into a space; or the first marker found winning, which lets one stray write change a space's kind.
- Topics: spaces, store, model, v2

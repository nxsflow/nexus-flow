# 2026-11-27 · NEX-205 · A space's kind is a marker record written at creation

- Decided by: Carsten
- Why: RFC 0001 has the store refuse a collection written into the wrong kind of space, but a space opened by id alone (manufakt.io through `Engine::open_space`) has no `nexus-flow.toml`, and NoCentralDB spaces carry no type. So the space gets a marker record when it is created: a collection `spaceKind` with one object naming the kind, which every reader can read. It is a new stored format. RFC 0001, "The model", says so.
- Instead of: (b) deriving the kind from the collections the space holds, which leaves an empty space without a kind and lets a mistaken write change it; (c) reading the kind only from `nexus-flow.toml`, which refuses a kind-restricted write to a space opened by id alone.
- Topics: spaces, model, v2

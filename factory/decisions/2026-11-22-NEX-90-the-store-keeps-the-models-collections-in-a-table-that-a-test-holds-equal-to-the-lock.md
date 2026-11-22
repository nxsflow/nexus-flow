# 2026-11-22 · NEX-90 · The store keeps the model's collections in a table that a test holds equal to the lock

- Decided by: coder on NEX-90
- Why: the store needs each collection's owning module (for the naming rule now, and for the kind of space a collection belongs to in NEX-95), which the lock does not hold, and the SDK does not expose the collections of an opened model. `model::COLLECTIONS` pairs each name with its module, and a test reads the committed lock's collection names and fails when the two differ.
- Instead of: reading the lock's JSON at run time, which adds a JSON dependency to the store and still needs the module of each collection from somewhere.
- Topics: model, store, v2

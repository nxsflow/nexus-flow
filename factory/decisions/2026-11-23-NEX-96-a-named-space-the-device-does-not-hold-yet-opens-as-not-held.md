# 2026-11-23 · NEX-96 · A named space the device does not hold yet opens as not held

- Decided by: coder on NEX-96
- Why: RFC 0001, "Reading and writing", answers a read from what the database holds and never fetches, and a device that an admin has added but that has not synced yet holds none of the space. So `Engine::open` and `Engine::open_space` open a space NoCentralDB answers `NotFound` for as not held, and a read of it finds nothing; any other refusal, such as a space of another model, fails the open. `nxs sync` must open the engine on exactly such a device to fetch the space.
- Instead of: failing the open, which would leave a newly added device unable to open the engine it syncs through.
- Topics: storage, seam, v2

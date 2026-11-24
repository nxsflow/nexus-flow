# 2026-11-23 · NEX-98 · A session folder holds its database in db/ and its keys in member.keys

- Decided by: coder on NEX-98
- Why: RFC 0001 says only that `sessions/<session id>/` holds one session member's database and keys. A session agent often runs where no key store exists, such as a cloud container, so its keys are always in a file. The database goes in `db/` and the key file is `member.keys` beside it, so the SDK owns the `db/` folder alone. A session is running while both exist. A `NXS_SESSION` that names any other session fails before anything is created, because opening the database would create a new member. `nxs session start` must create these two paths, and `nxs session end` must remove them.
- Instead of: the database directly in the session folder with the key file inside it, which would mix a file of nexus-flow's into a folder the SDK owns; or keeping session keys in the platform key store, which a cloud session does not have.
- Topics: storage, seam, sessions, v2

# 2026-11-23 · NEX-226 · A session's keys are in a key file beside its member database

- Decided by: Carsten
- Why: NoCentralDB 0.1.0-beta.1 keeps a member's keys either in the platform key store or in a key file the app names, never inside the database, and a cloud session has no key store. So pm's and every role session's keys are in a key file beside the member's database, `sessions/<id>/member.keys` next to `sessions/<id>/db/`, as #201 (NEX-98) builds it. The file lives in the container's data directory: the keys die with the container and never reach the repository, the environment, a ticket or a log. RFC 0001's members table and data layout say so. This also corrects the "Why" of the NEX-224 decision, which said sessions keep their keys inside their member databases. That decision, about the device, stands.
- Instead of: (b) the key store when one exists and a key file otherwise, a second path for the same thing; (c) waiting for NoCentralDB to keep member keys inside the database.
- Topics: nocentraldb, storage, sessions, v2

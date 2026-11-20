# 2026-11-20 · NEX-79 · A moved workspace raises v1's schema header, so every v1 binary refuses it

- Decided by: Carsten
- Why: v1 refuses a database whose header asks for a newer schema, in every release since schema 4; with `.nxs/migrated.toml` the move raises the header of `db.sqlite` to 10 and records the old values, so every installed v1 binary stops writing, and a last v1 release from `main` opens a moved workspace read-only and says why. `nxs migrate --undo` restores the header (RFC 0008, "The marker").
- Instead of: the marker and a last v1 release only, which older binaries ignore; or the raised header only, with no read access and no reason given.
- Topics: migration, cli, storage

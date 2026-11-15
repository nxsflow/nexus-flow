# 2026-11-15 · no ticket · The shipped nxf-relay carries SQLite + Postgres, NOT DynamoDB — and rustls stays pinned to default-features=false + ring, the provider named.

- Decided by: recorded in the project memory (`nxm`, key `relay-ships-sqlite-and-postgres-not-dynamodb`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: relay, release, dependencies

## The record

DECISION (owner, 2026-08-07): the SHIPPED `nxf-relay` binary carries SQLite + Postgres, NOT
DynamoDB. `release.yml` builds all four targets with `--features nxs-server/postgres` (verified
2026-09-05).

MEASURED, not estimated — a cold `--release` build of the relay alone: sqlite 76 crates / 57 s /
3.8 MB; +postgres 153 / 71 s / 5.0 MB; +dynamodb 247 / 3 m 35 s / 22.6 MB. DynamoDB pulls in
`aws-lc-sys` (native C/asm) and needs cmake plus a C toolchain on EVERY runner, to put ~19 MB into
EACH of the four archives — for a capability no archive consumer pulls: manufakt.io builds the
relay from a pinned revision (`apps/web/relay-pin.json`, tag+commit+features) with its own
toolchain and never unpacks ours. DynamoDB remains a SUPPORTED SOURCE BUILD and is documented as
such (spec §5.2, `nxf guide running-a-relay`, and a comment in the `[features]` block) — leaving it
out silently was the original mistake.

CONSEQUENT RULE FOR rustls IN THIS REPO: always depend on rustls with `default-features = false` +
`ring`, and name the provider explicitly via
`builder_with_provider(rustls::crypto::ring::default_provider())` (see
`crates/server/src/tls_pg.rs`). rustls defaults to aws-lc-rs — exactly the cmake dependency this
decision avoids — so it would otherwise come back in through the back door. Naming it is
additionally necessary because a `--features postgres,dynamodb` build compiles TWO providers in,
and `ClientConfig::builder()` then panics at runtime.

Proven: both musl legs cross-compile with postgres+rustls/ring through cargo-zigbuild (x86_64 and
aarch64, statically linked). The Postgres backend speaks TLS (webpki-roots, plus
`NXF_RELAY_PG_CA_FILE` for a private CA); `sslmode` still decides WHETHER TLS is used, so the CI
service container without SSL stays green unchanged.

OPEN AND DELIBERATELY NOT BRIDGED: the relay has NO CLIENT AUTHENTICATION. A managed store changes
nothing about that. That is also why the GHCR container image promised in spec §5.6 is still
waiting on the auth/multi-tenant slice — a one-command way to start the relay would be a
one-command way to put an unauthenticated shared board on the network.

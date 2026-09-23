//! nxs-server — the **dumb durable relay** (spec §2/§3).
//!
//! A neutral op-carrier: it persists the raw op log and serves "ops since a cursor".
//! It does NOT fold, derive, or validate, and it never rejects an op for an unknown
//! shape (server-opaque, §4.1). It depends on the wire types alone — not on core's
//! fold code — so the separation is visible in the dependency graph.

pub mod app;
pub mod presence;
pub mod registry;
pub mod store;

// Durable Postgres backend behind the storage traits — the op log and the registry, which also
// carries the optional presence board (spec §5/§10, 6j6v.f0b5). Feature-gated so
// the default relay and `cargo test` stay SQLite-only; CI enables it against a real DB.
#[cfg(feature = "postgres")]
pub mod registry_pg;
#[cfg(feature = "postgres")]
pub mod store_pg;
// TLS for that backend — what makes a MANAGED Postgres (Supabase, Neon, RDS) reachable at
// all (nexus-flow-6j6v.94gm). Public for ONE reason: the parity suite has to open its own
// admin connection (to create each test's isolated schema), and it must dial exactly the way
// the backend dials. When it built its own `NoTls` connection instead, the suite could not run
// against a TLS-only endpoint at all — so the one database we most want to prove ourselves
// against was the one database it could not reach.
#[cfg(feature = "postgres")]
pub mod tls_pg;
// Reaching a MANAGED Postgres, and saying honestly what went wrong when we cannot
// (nexus-flow-6j6v.da42). Public for the same reason `tls_pg` is: the thing that has to run this
// check is outside the library — the `pg-verdict` example CI runs as its Supabase preflight, and
// the suites' own scratch-schema hygiene. Keeping the classification here rather than in a
// workflow file is what makes it testable, and it is tested: the split between "the provider was
// not there" and "we are misconfigured" is the whole reason the Supabase job may block.
#[cfg(feature = "postgres")]
pub mod managed_pg;

// Durable DynamoDB backend (ddb-opstore epic) behind the same storage traits.
// Feature-gated so the default relay and `cargo test` need no AWS SDK, endpoint, or
// credentials; CI enables it against DynamoDB Local / a real table.
#[cfg(feature = "dynamodb")]
pub mod store_ddb;

//! `nxf-relay` — the dumb durable relay binary. Serves the append/read_since/register/machines
//! HTTP surface on loopback (no auth — a later slice). The storage backend is selected at boot:
//! the default is the file-backed SQLite store; building with `--features postgres` and
//! setting `NXF_RELAY_BACKEND=postgres` swaps in the durable Postgres backend, or building
//! with `--features dynamodb` and setting `NXF_RELAY_BACKEND=dynamodb` swaps in the
//! DynamoDB backend — both behind the same traits, no change to the HTTP surface or the
//! relay's behaviour. Address, store path, Postgres URL, and DynamoDB table/endpoint/region
//! are env-overridable; behaviour is covered by the `app`/store unit tests, the
//! `relay_http` end-to-end test, the Postgres parity suite, and the DynamoDB Local suite.

use std::sync::Arc;

use nxs_server::app::{app, SharedRegistry, SharedStore};
use nxs_server::registry::SqlitePrefixRegistry;
use nxs_server::store::SqliteOpStore;

/// Pick the storage backend from `NXF_RELAY_BACKEND` (default `sqlite`). Both arms produce
/// the same trait objects — the relay above them is identical regardless of backend.
fn backends() -> (SharedStore, SharedRegistry) {
    let backend = std::env::var("NXF_RELAY_BACKEND").unwrap_or_else(|_| "sqlite".to_string());
    match backend.as_str() {
        #[cfg(feature = "postgres")]
        "postgres" => {
            use nxs_server::registry_pg::PostgresPrefixRegistry;
            use nxs_server::store_pg::PostgresOpStore;
            // One URL serves both backends (separate tables in one database), matching the
            // SQLite default where both share one file.
            let url = std::env::var("NXF_RELAY_PG_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect("NXF_RELAY_PG_URL or DATABASE_URL must be set for the postgres backend");
            let ops = Arc::new(PostgresOpStore::connect(&url).expect("connect postgres op store"));
            let registry =
                Arc::new(PostgresPrefixRegistry::connect(&url).expect("connect postgres registry"));
            (ops, registry)
        }
        #[cfg(feature = "dynamodb")]
        "dynamodb" => {
            use nxs_server::store_ddb::{DynamoDbOpStore, DynamoDbPrefixRegistry};
            // Table names default so a deployment only has to set them if it wants
            // non-default names; endpoint/region default to None, which hands the SDK's
            // standard resolution chain (real AWS in production, DynamoDB Local only when
            // NXF_RELAY_DDB_ENDPOINT overrides it).
            let table = std::env::var("NXF_RELAY_DDB_TABLE")
                .unwrap_or_else(|_| "nxf_stream_ops".to_string());
            let registry_table = std::env::var("NXF_RELAY_DDB_REGISTRY_TABLE")
                .unwrap_or_else(|_| "nxf_prefix_claims".to_string());
            let endpoint = std::env::var("NXF_RELAY_DDB_ENDPOINT").ok();
            let region = std::env::var("NXF_RELAY_DDB_REGION").ok();
            // Unlike `PostgresOpStore::connect`, `DynamoDbOpStore::new` returns a bare
            // value, not a `StoreResult<Self>` — deliberately, not an oversight. Postgres's
            // `connect` does real I/O at construction (`init_schema` creates the table if
            // missing), so it has something to fail on. The DynamoDB tables are provisioned
            // externally by the deploying stack — this code never creates or migrates one
            // (their required shape is documented at the top of `store_ddb.rs`) — so
            // construction is pure: it builds a client from config and no request leaves the
            // process. The tables' existence, the region, and the credentials are only ever
            // proven by the first real request, exactly like the SQLite arm opening a file
            // lazily.
            //
            // The cost is that a misconfigured deployment boots clean and only fails later,
            // per request. `store_ddb::ddb_err` logs every such failure so it is at least
            // self-diagnosing rather than a silent 500; an actual `DescribeTable` probe here
            // — which would turn a bad table name or region into a loud boot failure, the way
            // Postgres already gets for free — is a tracked follow-up, deliberately not
            // smuggled in here.
            let ops = Arc::new(DynamoDbOpStore::new(
                table,
                endpoint.clone(),
                region.clone(),
            ));
            let registry = Arc::new(DynamoDbPrefixRegistry::new(
                registry_table,
                endpoint,
                region,
            ));
            (ops, registry)
        }
        "sqlite" => sqlite_backends(),
        other => {
            // `postgres` and `dynamodb` are cargo features, so their arms above only exist when
            // the binary was compiled with them. A message that names all three unconditionally
            // both contradicts itself on a feature-less build ("unknown ... dynamodb (expected
            // ... or dynamodb)") and — since the shipped binary gained postgres (6j6v.7nee) —
            // recommends rebuilding for a feature this build already carries. So the message is
            // assembled from the SAME cfg the arms are: it names what THIS build can do and what
            // it is missing, which is the only thing that distinguishes a typo from a wrong
            // build at runtime (6j6v.0hjs).
            panic!("{}", unknown_backend_message(other))
        }
    }
}

/// The compile-time backend inventory of THIS binary. `sqlite` is always in; `postgres` and
/// `dynamodb` are cargo features, so the two lists are decided by the same `cfg` that decides
/// which match arms above exist — they cannot drift apart.
fn backend_inventory() -> (Vec<&'static str>, Vec<&'static str>) {
    let mut built_in = vec!["sqlite"];
    let mut missing: Vec<&'static str> = Vec::new();
    if cfg!(feature = "postgres") {
        built_in.push("postgres");
    } else {
        missing.push("postgres");
    }
    if cfg!(feature = "dynamodb") {
        built_in.push("dynamodb");
    } else {
        missing.push("dynamodb");
    }
    (built_in, missing)
}

/// The panic text for an unusable `NXF_RELAY_BACKEND`, for THIS build's inventory.
fn unknown_backend_message(requested: &str) -> String {
    let (built_in, missing) = backend_inventory();
    unknown_backend_message_from(requested, &built_in, &missing)
}

/// The message logic proper — pure in its inventory, so every shape it can produce (a build
/// gap, a typo on a build with gaps, a typo on an all-features build) is testable no matter
/// which features the test binary itself was compiled with. Splitting it this way is what
/// keeps the tests from silently going vacuous when the build's own feature set changes; the
/// `cfg!` truth still lives in `backend_inventory`, so the arms and the message cannot drift.
///
/// The operator must be able to tell "I typo'd the name" from "this build cannot do that" by
/// reading the message alone.
fn unknown_backend_message_from(
    requested: &str,
    built_in: &[&'static str],
    missing: &[&'static str],
) -> String {
    let mut msg = format!(
        "unknown NXF_RELAY_BACKEND {requested:?} — this build carries: {}.",
        built_in.join(", ")
    );
    if missing.contains(&requested) {
        // The name IS a real backend; it is this artifact that cannot serve it. Say so, and say
        // what it takes — a source build, not a config change.
        msg.push_str(&format!(
            " `{requested}` is a cargo feature this build was compiled WITHOUT: it needs a source \
             build with `--features {requested}` (see `nxf guide running-a-relay`)."
        ));
    } else if !missing.is_empty() {
        // A genuine typo. Still name the backends that exist but are absent here, so the next
        // attempt does not run into the branch above.
        msg.push_str(&format!(
            " Also available as source builds (`--features <name>`): {}.",
            missing.join(", ")
        ));
    }
    msg
}

fn sqlite_backends() -> (SharedStore, SharedRegistry) {
    let db = std::env::var("NXF_RELAY_DB").unwrap_or_else(|_| "relay.sqlite".to_string());
    // The prefix registry shares the relay's durable file by default (its own table).
    let registry_db = std::env::var("NXF_RELAY_REGISTRY_DB").unwrap_or_else(|_| db.clone());
    let ops = Arc::new(SqliteOpStore::open(&db).expect("open relay op store"));
    let registry =
        Arc::new(SqlitePrefixRegistry::open(&registry_db).expect("open prefix registry"));
    (ops, registry)
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("NXF_RELAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let (ops, registry) = backends();
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind relay address");
    eprintln!("nxf-relay listening on http://{addr}");
    axum::serve(listener, app(ops, registry))
        .await
        .expect("relay server error");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default build (this test's own features are the crate's defaults) must never
    /// recommend a feature it already carries — the regression 6j6v.0hjs describes.
    #[test]
    fn names_what_this_build_carries_and_never_recommends_it() {
        let (built_in, _) = backend_inventory();
        let msg = unknown_backend_message("nonsense");
        assert!(msg.contains("this build carries"), "{msg}");
        for name in &built_in {
            assert!(
                msg.contains(name),
                "message must list built-in {name}: {msg}"
            );
            assert!(
                !msg.contains(&format!("--features {name}")),
                "must not recommend rebuilding for {name}, which this build has: {msg}"
            );
        }
    }

    /// A real backend name that the build lacks: the message must blame the BUILD, not the
    /// spelling, and point at the one thing that fixes it. Driven off an explicit inventory
    /// rather than this binary's own, so it asserts the same thing under every feature set —
    /// the earlier version returned early (asserting nothing) on an all-features build.
    #[test]
    fn a_missing_backend_is_reported_as_a_build_gap() {
        let msg = unknown_backend_message_from("dynamodb", &["sqlite", "postgres"], &["dynamodb"]);
        assert!(
            msg.contains("--features dynamodb"),
            "must name the feature that would provide dynamodb: {msg}"
        );
        assert!(msg.contains("compiled WITHOUT"), "{msg}");
        assert!(
            msg.contains("this build carries: sqlite, postgres"),
            "{msg}"
        );
    }

    /// A typo is not a build gap: it must not be told to rebuild for the name it mistyped.
    #[test]
    fn a_typo_is_not_reported_as_a_build_gap() {
        let msg = unknown_backend_message_from("postgress", &["sqlite", "postgres"], &["dynamodb"]);
        assert!(!msg.contains("--features postgress"), "{msg}");
        assert!(
            msg.contains("unknown NXF_RELAY_BACKEND \"postgress\""),
            "{msg}"
        );
        // The real gap is still worth naming, so the next attempt lands in the branch above.
        assert!(msg.contains("Also available as source builds"), "{msg}");
    }

    /// The shape no feature set of this crate can currently produce on its own: an all-features
    /// build, where a bad name can only ever be a typo. It must not sprout an empty "also
    /// available" tail. Unreachable through `unknown_backend_message`, which is exactly why the
    /// message logic takes its inventory as an argument.
    #[test]
    fn an_all_features_build_offers_no_rebuild_at_all() {
        let msg =
            unknown_backend_message_from("postgress", &["sqlite", "postgres", "dynamodb"], &[]);
        assert_eq!(
            msg,
            "unknown NXF_RELAY_BACKEND \"postgress\" — this build carries: sqlite, postgres, dynamodb."
        );
    }
}

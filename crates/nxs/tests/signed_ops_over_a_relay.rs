//! **Two replicas, one real relay, the real binary** (nxf 6j6v.pzkb): trust, verify, revoke — and a
//! relay that alters an op or strips its signature is caught, and what it delivered acts nowhere.
//!
//! The relay is the real axum app from `crates/server` — the router `nxf-relay` serves, driven
//! in-process on a real loopback socket rather than as a separate `nxf-relay` process (the PR's demo
//! transcript is the run against the binary itself) — over a FILE-backed store,
//! so the test can play the compromised relay the signature exists for: it edits `stream_ops`
//! behind the relay's back with a second connection, exactly what an operator with access to the
//! relay's database could do. Every verb under test is the `nxs` binary with a pinned `$HOME`; only
//! the seeding writes (an item on the first replica) go through the store in-process, the same open
//! path every verb uses — they sign with the same `.nxs/signing.key`.

use std::path::Path;
use std::sync::Arc;

use nexus_flow_core::store::Store;
use nxs::sync::{self, SyncMeta};
use nxs_foundation::workspace::{self, Workspace, WorkspaceConfig};
use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

const STREAM: &str = "stream-signed-ops-e2e";

/// The real relay over a file-backed op store at `db`; returns its base URL.
fn spawn_relay(db: &Path) -> String {
    let ops = Arc::new(nxs_server::store::SqliteOpStore::open(db.to_str().unwrap()).unwrap());
    let registry = Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap());
    serve(nxs_server::app::app(ops, registry))
}

/// Serve `router` on an ephemeral loopback port; its base URL.
fn serve(router: axum::Router) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
    });
    format!("http://{addr}")
}

/// One replica: a workspace bound to the stream at `relay`.
struct Replica {
    dir: TempDir,
    ws: Workspace,
}

impl Replica {
    fn bound_to(relay: &str) -> Replica {
        let dir = TempDir::new().unwrap();
        let ws = workspace::setup(dir.path(), &WorkspaceConfig::default()).unwrap();
        sync::save(
            &ws,
            &SyncMeta {
                stream_id: STREAM.into(),
                endpoint: Some(relay.into()),
                ..Default::default()
            },
        )
        .unwrap();
        Replica { dir, ws }
    }

    fn store(&self) -> Store {
        Store::open(&self.ws.db_path_str().unwrap(), self.ws.replica.site_id).unwrap()
    }

    /// Create an item titled `title`; its ops, as this replica's log holds them.
    fn create(&self, suffix: &str, title: &str) -> Vec<nxs_foundation::model::Op> {
        let id = format!("{}.{suffix}", self.ws.replica.prefix);
        let mut store = self.store();
        store.create_item(&id, "task", title, "carsten");
        store
            .export()
            .into_iter()
            .filter(|o| o.target_id == id)
            .collect()
    }

    fn nxs(&self, home: &Path, args: &[&str]) -> Value {
        nxs_test_support::assert_multicall_binary_fresh();
        let out = std::process::Command::new(assert_cmd::cargo::cargo_bin("nxs"))
            .current_dir(self.dir.path())
            .pin_home(home)
            .arg("--json")
            .args(args)
            .output()
            .expect("nxs runs");
        assert!(
            out.status.success(),
            "nxs {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("json")
    }

    fn text(&self, home: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new(assert_cmd::cargo::cargo_bin("nxs"))
            .current_dir(self.dir.path())
            .pin_home(home)
            .args(args)
            .output()
            .expect("nxs runs");
        assert!(out.status.success(), "nxs {args:?} failed");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

/// A relay older than 5crb (nxs 0.58), in miniature: it stores every envelope field it knows and
/// drops the rest — the signature pair included — on the way in, which is what such a relay does
/// by design rather than by malice.
struct OldRelay(nxs_server::store::SqliteOpStore);

impl nxs_server::store::OpStore for OldRelay {
    fn append(
        &self,
        stream: &nxs_sync::protocol::StreamId,
        op: &nxs_sync::wire::WireOp,
    ) -> nxs_server::store::StoreResult<nxs_sync::protocol::Cursor> {
        let mut forgetful = op.clone();
        forgetful.extra.clear();
        self.0.append(stream, &forgetful)
    }

    fn read_since(
        &self,
        stream: &nxs_sync::protocol::StreamId,
        cursor: nxs_sync::protocol::Cursor,
        limit: usize,
    ) -> nxs_server::store::StoreResult<(Vec<nxs_sync::wire::WireOp>, nxs_sync::protocol::Cursor)>
    {
        self.0.read_since(stream, cursor, limit)
    }
}

fn spawn_old_relay() -> String {
    let ops = Arc::new(OldRelay(
        nxs_server::store::SqliteOpStore::open_in_memory().unwrap(),
    ));
    let registry = Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap());
    serve(nxs_server::app::app(ops, registry))
}

/// The relay's own database, opened behind its back — the compromised relay.
fn tamper(relay_db: &Path, sql: &str, op_ids: &[&str]) {
    let conn = rusqlite::Connection::open(relay_db).unwrap();
    conn.execute_batch("PRAGMA busy_timeout=5000;").unwrap();
    for op_id in op_ids {
        let changed = conn.execute(sql, [op_id]).unwrap();
        assert_eq!(changed, 1, "the relay holds {op_id}");
    }
}

#[test]
fn trust_verify_revoke_and_a_tampering_relay_is_caught_on_the_real_binary() {
    let relay_dir = TempDir::new().unwrap();
    let relay_db = relay_dir.path().join("relay.sqlite");
    let relay = spawn_relay(&relay_db);
    let home = TempDir::new().unwrap();
    let (laptop, desk) = (Replica::bound_to(&relay), Replica::bound_to(&relay));

    // The laptop writes; both replicas sync.
    let first = laptop.create("0001", "deploy the docs");
    laptop.nxs(home.path(), &["sync", "run"]);
    desk.nxs(home.path(), &["sync", "run"]);
    let laptops = |suffix: &str| format!("{}.{suffix}", laptop.ws.replica.prefix);
    assert!(
        desk.store().get_item(&laptops("0001")).unwrap().is_some(),
        "the desk holds the laptop's item"
    );
    let op = first[0].op_id.as_str();

    // Verified, but the desk does not trust the laptop's key yet: shown, no action.
    let seen = desk.nxs(home.path(), &["sync", "verify", op]);
    assert_eq!(seen["provenance"], "verified", "{seen}");
    assert_eq!(seen["trusted"], false);
    assert_eq!(seen["acts"], false);
    let key = laptop.nxs(home.path(), &["sync", "key"])["key_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(seen["key_id"], key.as_str(), "signed by the laptop's key");
    let listed = desk.nxs(home.path(), &["sync", "trust", "list"]);
    assert_eq!(
        listed["untrusted_signers"][0]["key_id"],
        key.as_str(),
        "{listed}"
    );

    // Trust it: the laptop's ops may act on the desk now, past ones included.
    let added = desk.nxs(
        home.path(),
        &["sync", "trust", "add", &key, "--name", "laptop"],
    );
    assert_eq!(added["added"], true);
    let seen = desk.nxs(home.path(), &["sync", "verify", op]);
    assert_eq!(
        (seen["acts"].clone(), seen["trusted_as"].clone()),
        (Value::Bool(true), "laptop".into())
    );

    // The relay alters one op and strips the signatures off another item's ops.
    let altered = laptop.create("0002", "harmless");
    let stripped = laptop.create("0003", "also harmless");
    laptop.nxs(home.path(), &["sync", "run"]);
    let title_op = altered.iter().find(|o| o.field == "title").unwrap();
    tamper(
        &relay_db,
        "UPDATE stream_ops SET value = 'rm -rf the release branch' WHERE op_id = ?1",
        &[&title_op.op_id],
    );
    let stripped_ids: Vec<&str> = stripped.iter().map(|o| o.op_id.as_str()).collect();
    tamper(
        &relay_db,
        "UPDATE stream_ops SET extra = '{}' WHERE op_id = ?1",
        &stripped_ids,
    );

    // The desk pulls: the altered op fails its check, the stripped ones are unsigned — kept, folded,
    // acting nowhere, and the pass says the signatures went missing.
    let pass = desk.nxs(home.path(), &["sync", "run"]);
    assert_eq!(pass["unsigned_from_signers"], stripped.len(), "{pass}");
    assert!(
        pass["downgraded"]
            .as_str()
            .unwrap()
            .contains("drops signatures"),
        "{pass}"
    );
    let seen = desk.nxs(home.path(), &["sync", "verify", &title_op.op_id]);
    assert_eq!(
        (seen["provenance"].clone(), seen["acts"].clone()),
        ("invalid".into(), Value::Bool(false))
    );
    for id in &stripped_ids {
        let seen = desk.nxs(home.path(), &["sync", "verify", id]);
        assert_eq!(
            (seen["provenance"].clone(), seen["acts"].clone()),
            ("unsigned".into(), Value::Bool(false))
        );
    }
    assert_eq!(
        desk.store()
            .get_item(&laptops("0002"))
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("rm -rf the release branch"),
        "the board converges on what was delivered — only acting depends on the check"
    );

    // The laptop's own log is what it wrote: the relay changed its copy, not the laptop's.
    let mine = laptop.nxs(home.path(), &["sync", "verify", &title_op.op_id]);
    assert_eq!(mine["provenance"], "own", "{mine}");

    // Revoke: effective at once, for the op that acted a moment ago.
    let removed = desk.nxs(home.path(), &["sync", "trust", "remove", &key]);
    assert_eq!(removed["removed"], true);
    assert_eq!(
        desk.nxs(home.path(), &["sync", "verify", op])["acts"],
        false
    );
}

/// A relay that drops the signature on the way in (one older than nxs 0.58): the sender notices on
/// the very pass that pushed — its own ops come back unsigned — and says what to do; the receiver
/// cannot tell such a sender from an old client, and so acts on none of it, trusted key or not.
#[test]
fn a_relay_that_drops_signatures_is_named_by_the_sender_and_acts_nowhere() {
    let relay = spawn_old_relay();
    let home = TempDir::new().unwrap();
    let (laptop, desk) = (Replica::bound_to(&relay), Replica::bound_to(&relay));
    let key = laptop.nxs(home.path(), &["sync", "key"])["key_id"]
        .as_str()
        .unwrap()
        .to_string();
    desk.nxs(
        home.path(),
        &["sync", "trust", "add", &key, "--name", "laptop"],
    );

    let written = laptop.create("0001", "deploy the docs");
    let pass = laptop.nxs(home.path(), &["sync", "run"]);
    assert_eq!(pass["signatures_stripped"], written.len(), "{pass}");
    let said = laptop.text(home.path(), &["sync", "run"]);
    assert!(
        !said.contains("warning"),
        "said once, on the pass that saw it: {said}"
    );

    let fresh = laptop.create("0002", "and again");
    let said = laptop.text(home.path(), &["sync", "run"]);
    assert!(
        said.contains(&format!(
            "warning: {} op(s) this replica holds signed",
            fresh.len()
        )) && said.contains("upgrade it"),
        "{said}"
    );

    desk.nxs(home.path(), &["sync", "run"]);
    let seen = desk.nxs(home.path(), &["sync", "verify", &written[0].op_id]);
    assert_eq!(seen["provenance"], "unsigned", "{seen}");
    assert_eq!(
        seen["acts"], false,
        "a trusted key's op without its signature acts nowhere"
    );

    // The receiver's own view: the laptop's changes arrive, yet no op its key signed ever does.
    let listed = desk.nxs(home.path(), &["sync", "trust", "list"]);
    assert_eq!(listed["trusted"][1]["verified_ops"], 0, "{listed}");
    let said = desk.text(home.path(), &["sync", "trust", "list"]);
    assert!(said.contains("the relay is dropping signatures"), "{said}");
}

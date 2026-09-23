//! T14 (`t4ax`) — the end-to-end proof both `n4dn` items exist for: two independent replicas of
//! ONE repo converge with no manual sync command, plus the p5sa 404 regression against the real
//! relay app. Every earlier task in this epic proved one piece in isolation (slug derivation,
//! the scheduler's debounce, the sweep's per-workspace bookkeeping, ...); this file is the only
//! place those pieces are shown to actually solve the problem together.
//!
//! Deliberately in-process, not subprocess: [`nxs::sync::daemon::sweep`] is driven directly
//! (never `nxs sync run`, never the real-clock `serve()` loop) so the test proves the DATA path
//! with no sleeps — the timing behaviour (debounce/backoff/wake) is already proven by the
//! clock-injected `Scheduler`/`DaemonState` tests in `crates/nxs/src/sync/daemon.rs`.
//!
//! Binding here goes through [`nxs::sync::save`] directly, using the exact same derivation
//! [`nxs::sync::bind`]'s default (no `--create`/`--join`) path uses under the hood
//! ([`nxs::sync::slug::origin_remote`] + [`nxs::sync::slug::stream_id_from_remote`]) — NOT
//! through the public `bind()` wrapper itself. `bind()` always drives the REAL registrar
//! (`~/.nexusflow/workspaces.toml`) and a REAL best-effort launchd autostart attempt; the
//! injectable spy seam that lets `crates/nxs/src/sync/mod.rs`'s own tests avoid that (`bind_with`
//! plus the private `Autostart`/`Registrar` traits) is intentionally not `pub`, so it is
//! unreachable from this external test crate. This branch has already polluted a real
//! developer's home directory twice from exactly this mistake — see the task brief — so this
//! file never calls `nxs::sync::bind` at all.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use axum::response::IntoResponse;
use nxs_foundation::workspace::{self, Workspace, WorkspaceConfig};
use nxs_service::ServiceHome;
use nxs_sync::presence::Presence;
use tempfile::TempDir;

use nxs::sync::daemon::{self, RealPass, ServicePass};
use nxs::sync::{self, PresenceNote, SyncMeta};

/// Start the real relay (the axum app from `crates/server`, not a stand-in) on an ephemeral
/// loopback port; return its base URL. Copied from `crates/server/tests/relay_http.rs::spawn_relay`
/// per the task brief — an in-memory `SqliteOpStore`/`SqlitePrefixRegistry` behind the real router,
/// so both tests below exercise the actual HTTP path-building + routing, not a mock.
fn spawn_relay() -> String {
    let ops = std::sync::Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap());
    let registry =
        std::sync::Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap());
    let router = nxs_server::app::app(ops, registry);
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

/// A fresh `.nxs/` workspace inside a real (local-only — nothing here ever touches the network)
/// git repo whose `origin` is `remote`. Mirrors the private helper of the same shape in
/// `crates/nxs/src/sync/mod.rs`'s own test module — duplicated rather than shared, since that one
/// is `#[cfg(test)]`-private to the crate and this file is a separate integration-test crate.
fn workspace_with_remote(remote: &str) -> (TempDir, Workspace) {
    let tmp = TempDir::new().unwrap();
    for args in [vec!["init", "-q"], vec!["remote", "add", "origin", remote]] {
        let ok = std::process::Command::new("git")
            .args(&args)
            .current_dir(tmp.path())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
    let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
    (tmp, ws)
}

/// A workspace with `stream_id` written directly into `.nxs/sync.toml`, bypassing `bind`
/// entirely. Used by the p5sa regression: the bind-time gate (`ensure_valid_stream_id`) would
/// correctly REFUSE an id containing `/`, but one may already sit in a pre-existing `sync.toml`
/// from before that gate shipped — this is exactly that legacy state, which the transport's
/// percent-encoding must still carry correctly.
fn bound_workspace_with_raw_id(raw_id: &str) -> (TempDir, Workspace) {
    let tmp = TempDir::new().unwrap();
    let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
    sync::save(
        &ws,
        &SyncMeta {
            stream_id: raw_id.to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    (tmp, ws)
}

/// Bind `ws` to the stream its `origin` remote derives (spec §4.1) — the DEFAULT `nxs sync bind`
/// path, no `--create`/`--join` anywhere — using the same derivation `bind`'s internal
/// `bind_derived` calls ([`sync::slug::origin_remote`] + [`sync::slug::stream_id_from_remote`],
/// both `pub`), then persists it with [`sync::save`] directly. See the file doc comment for why
/// this does not go through the public `bind()` wrapper itself.
fn bind_derived_with_endpoint(ws: &Workspace, endpoint: &str) {
    let root = ws.dir.parent().unwrap_or(&ws.dir);
    let remote = sync::slug::origin_remote(root).expect("git origin remote is set");
    let stream_id =
        sync::slug::stream_id_from_remote(&remote).expect("a usable remote derives an id");
    sync::save(
        ws,
        &SyncMeta {
            stream_id,
            endpoint: Some(endpoint.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
}

/// The stream a bound workspace derived/joined — `.nxs/sync.toml`'s `stream_id`.
fn stream_of(ws: &Workspace) -> String {
    sync::load(ws).unwrap().unwrap().stream_id
}

/// Create one item directly against `ws`'s store (flow's `nexus_flow_core::store::Store`, the
/// SAME open path `sync::run_pass` itself uses — `Store::open(ws.db_path_str(), ws.replica.site_id)`)
/// with the given `title`, and return its id. In-process, not via a `nxf create` subprocess: this
/// crate already links `nexus-flow-core` (see `crates/nxs/src/sync/mod.rs`'s own `run_pass`), so
/// there is no reason to pay for a second binary invocation just to seed one item.
fn create_item_titled(ws: &Workspace, title: &str) -> String {
    let path = ws.db_path_str().unwrap();
    let mut store = nexus_flow_core::store::Store::open(&path, ws.replica.site_id).unwrap();
    let id = format!("{}.0001", ws.replica.prefix);
    store.create_item(&id, "task", title, "tester");
    id
}

fn create_one_item(ws: &Workspace) -> String {
    create_item_titled(ws, "seed item")
}

/// Every item title materialized in `ws`'s store right now — read fresh off disk (a new `Store`
/// handle), so this only sees what actually landed durably, not anything cached in-process by an
/// earlier open.
fn item_titles(ws: &Workspace) -> Vec<String> {
    let path = ws.db_path_str().unwrap();
    let store = nexus_flow_core::store::Store::open(&path, ws.replica.site_id).unwrap();
    store
        .list_items()
        .unwrap()
        .into_iter()
        .filter_map(|row| row.title)
        .collect()
}

#[test]
fn a_stream_id_that_needs_encoding_round_trips_against_the_real_relay() {
    // The consumer's exact failure (manufakt-io, v0.35.0): a stream_id containing '/' built
    //   GET /streams/nxsflow/manufakt-io/ops -> 404
    // because the path was assembled without percent-encoding. Task 1 fixed the client-side
    // encoding and the bind gate now refuses to MINT such an id going forward — but an id like
    // this may already sit in someone's `.nxs/sync.toml` from before the gate existed, so the
    // transport itself must still carry it correctly. That legacy state is exactly what
    // `bound_workspace_with_raw_id` sets up (bind's own gate would reject this id outright).
    let base = spawn_relay();
    let (_tmp, ws) = bound_workspace_with_raw_id("nxsflow/manufakt-io");
    create_one_item(&ws);

    let outcome = sync::run_pass(&ws, &base).expect("no 404 against the real relay app");

    assert!(
        outcome.pushed > 0,
        "the create-item ops actually made it to the relay, not just a 200 with nothing sent"
    );
}

#[test]
fn two_clones_of_one_repo_converge_without_a_manual_sync() {
    let base = spawn_relay();

    // Two INDEPENDENT working copies of "the same repo" — different remote spellings (scp vs.
    // https) of one github path, so the slug derivation is what has to prove they land on one
    // stream. No stream id is EVER copied between them (no `--join`, no shared variable) — that
    // is the entire point: if this test threaded an id from `a` to `b`, it would prove nothing
    // about the derivation actually working end to end.
    let (_a, a) = workspace_with_remote("git@github.com:nxsflow/convergence-probe.git");
    let (_b, b) = workspace_with_remote("https://github.com/nxsflow/convergence-probe");
    for ws in [&a, &b] {
        bind_derived_with_endpoint(ws, &base);
    }
    assert_eq!(
        stream_of(&a),
        stream_of(&b),
        "the slug derivation put both independently-bound replicas on one stream with no --join"
    );

    create_item_titled(&a, "from replica A");

    // Drive the DAEMON's sweep directly — nobody types `nxs sync run` or any other command here,
    // and there is no sleep waiting on the real-clock `serve()` loop. `RealPass` delegates to the
    // exact same `run_pass` the CLI verb calls, so this is the real push/pull engine, just called
    // the way the daemon calls it rather than the way a human would.
    let pass = RealPass;
    let push = daemon::sweep(std::slice::from_ref(&a), &pass, Some(&base));
    assert_eq!(push.synced.len(), 1, "A's push pass must actually succeed");
    let pull = daemon::sweep(std::slice::from_ref(&b), &pass, Some(&base));
    assert_eq!(pull.synced.len(), 1, "B's pull pass must actually succeed");

    assert!(
        item_titles(&b).contains(&"from replica A".to_string()),
        "an item created on replica A must be readable BY TITLE on replica B, with no manual \
         sync command ever run — that is the DoD this test exists to prove"
    );
}

// ----- machine presence (6j6v.f0b5) ------------------------------------------------------------

/// The real relay app with a clock the test turns by hand: presence is judged by AGE, and the window
/// for the production cadence is eleven minutes nobody waits out in a test.
fn spawn_relay_with_clock() -> (String, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000));
    let clock_now = Arc::clone(&now);
    let ops = Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap());
    let registry = Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap());
    let router = nxs_server::app::app_with_clock(
        ops,
        registry,
        Arc::new(move || clock_now.load(Ordering::SeqCst)),
    );
    (serve(router), now)
}

/// The real relay app with its presence route taken away — at the HTTP level, what a relay built
/// before 6j6v.f0b5 answers (manufakt.io's runs v0.35.0): every other route as before, `404` on
/// `/streams/{id}/machines`.
fn spawn_relay_from_before_presence() -> String {
    let ops = Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap());
    let registry = Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap());
    let router = nxs_server::app::app(ops, registry).layer(axum::middleware::from_fn(
        |req: axum::extract::Request, next: axum::middleware::Next| async move {
            if req.uri().path().ends_with("/machines") {
                axum::http::StatusCode::NOT_FOUND.into_response()
            } else {
                next.run(req).await
            }
        },
    ));
    serve(router)
}

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

/// A machine: its own service home (the unit this slice calls a machine), under a name.
fn machine(name: &str) -> (TempDir, ServiceHome) {
    let dir = TempDir::new().unwrap();
    let home = ServiceHome::at(dir.path().join(".nexusflow"));
    home.rename_machine(name).unwrap();
    (dir, home)
}

/// `(name, online)` for every machine a workspace's relay reports, in the relay's order.
fn seen_from(ws: &Workspace) -> Vec<(String, bool)> {
    match sync::read_machines(ws, None).unwrap().presence {
        Presence::Reported { machines, .. } => {
            machines.into_iter().map(|m| (m.name, m.online)).collect()
        }
        Presence::Unsupported => panic!("this relay records presence"),
    }
}

#[test]
fn two_machines_syncing_one_workspace_see_each_other_and_a_stopped_one_goes_offline_after_its_window(
) {
    // The Definition of Done of 6j6v.f0b5, in process: two machines — two service homes — each run
    // the service's own pass over their clone of one repo against one relay. Both list both. Then
    // one stops; once its window (2 x 300 s + 60 s) has passed on the relay's clock, the other
    // lists it as not online, and itself still as online.
    let (relay, now) = spawn_relay_with_clock();
    let remote = "git@github.com:acme/presence.git";
    let (_ta, laptop_ws) = workspace_with_remote(remote);
    let (_tb, mini_ws) = workspace_with_remote(remote);
    bind_derived_with_endpoint(&laptop_ws, &relay);
    bind_derived_with_endpoint(&mini_ws, &relay);
    let (_ha, laptop) = machine("MacBook");
    let (_hb, mini) = machine("Mac mini");
    let laptop_service = ServicePass::new(laptop.clone(), 300);
    let mini_service = ServicePass::new(mini.clone(), 300);

    let report = daemon::sweep(std::slice::from_ref(&laptop_ws), &laptop_service, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.synced[0].1.presence, Some(PresenceNote::Recorded));
    now.store(1_005, Ordering::SeqCst);
    daemon::sweep(std::slice::from_ref(&mini_ws), &mini_service, None);

    let both = [
        ("Mac mini".to_string(), true),
        ("MacBook".to_string(), true),
    ];
    assert_eq!(seen_from(&laptop_ws), both, "the laptop sees both");
    assert_eq!(seen_from(&mini_ws), both, "the mini sees both");

    // The mini stops. The laptop's service keeps passing; the relay's clock moves on.
    now.store(1_005 + 660, Ordering::SeqCst);
    daemon::sweep(std::slice::from_ref(&laptop_ws), &laptop_service, None);
    assert_eq!(
        seen_from(&laptop_ws),
        [
            ("MacBook".to_string(), true),
            ("Mac mini".to_string(), true)
        ],
        "at exactly its window the mini still counts as online"
    );
    now.store(1_005 + 661, Ordering::SeqCst);
    daemon::sweep(std::slice::from_ref(&laptop_ws), &laptop_service, None);
    assert_eq!(
        seen_from(&laptop_ws),
        [
            ("MacBook".to_string(), true),
            ("Mac mini".to_string(), false)
        ],
        "one second past it, the stopped machine is listed but not online"
    );
}

#[test]
fn a_machine_renamed_between_passes_is_listed_under_its_new_name_without_a_restart() {
    let (relay, _now) = spawn_relay_with_clock();
    let (_t, ws) = workspace_with_remote("git@github.com:acme/rename.git");
    bind_derived_with_endpoint(&ws, &relay);
    let (_h, home) = machine("MacBook");
    let service = ServicePass::new(home.clone(), 300);
    daemon::sweep(std::slice::from_ref(&ws), &service, None);
    home.rename_machine("Work laptop").unwrap();
    daemon::sweep(std::slice::from_ref(&ws), &service, None);
    assert_eq!(seen_from(&ws), [("Work laptop".to_string(), true)]);
}

#[test]
fn a_service_against_a_relay_from_before_presence_syncs_on_and_says_presence_is_unsupported() {
    // New client, old relay (6j6v.f0b5's compatibility clause): the announcement meets a 404, the
    // pass is still a success, the ops still converge, and reading the machines names the gap
    // instead of failing.
    let relay = spawn_relay_from_before_presence();
    let remote = "git@github.com:acme/old-relay.git";
    let (_ta, a) = workspace_with_remote(remote);
    let (_tb, b) = workspace_with_remote(remote);
    bind_derived_with_endpoint(&a, &relay);
    bind_derived_with_endpoint(&b, &relay);
    let (_h, home) = machine("MacBook");
    let service = ServicePass::new(home, 300);
    create_item_titled(&a, "made before the upgrade");

    let report = daemon::sweep(std::slice::from_ref(&a), &service, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.synced[0].1.presence, Some(PresenceNote::Unsupported));
    let report = daemon::sweep(std::slice::from_ref(&b), &service, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(item_titles(&b), ["made before the upgrade"]);

    assert_eq!(
        sync::read_machines(&b, None).unwrap().presence,
        Presence::Unsupported
    );
}

#[test]
fn a_client_from_before_presence_syncs_against_a_relay_that_has_it_and_is_simply_not_listed() {
    // Old client, new relay. An old client speaks exactly register / push / pull, in the bodies this
    // change leaves untouched — which is what `RealPass` (the pass `nxs sync run` makes) speaks too:
    // it announces nothing. So it stands in for the old client on the wire, and the same test pins
    // the second half of the design: a MANUAL sync never makes a machine look online, because only
    // the service keeps the promise "online" makes.
    let (relay, _now) = spawn_relay_with_clock();
    let remote = "git@github.com:acme/old-client.git";
    let (_ta, a) = workspace_with_remote(remote);
    let (_tb, b) = workspace_with_remote(remote);
    bind_derived_with_endpoint(&a, &relay);
    bind_derived_with_endpoint(&b, &relay);
    create_item_titled(&a, "from an old client");

    daemon::sweep(std::slice::from_ref(&a), &RealPass, None);
    let report = daemon::sweep(std::slice::from_ref(&b), &RealPass, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.synced[0].1.presence, None, "nothing was announced");
    assert_eq!(item_titles(&b), ["from an old client"]);
    assert_eq!(seen_from(&a), Vec::<(String, bool)>::new());
}

/// The real relay app, but with `/machines` answering `status` — a relay (or a gateway before it)
/// that fails the announcement while every sync route works.
fn spawn_relay_failing_presence(status: axum::http::StatusCode) -> String {
    let ops = Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap());
    let registry = Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap());
    let router = nxs_server::app::app(ops, registry).layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| async move {
            if req.uri().path().ends_with("/machines") {
                status.into_response()
            } else {
                next.run(req).await
            }
        },
    ));
    serve(router)
}

#[test]
fn a_relay_that_fails_the_announcement_costs_the_announcement_never_the_sync() {
    // "The sync is what the pass is for": a 500 on `/machines` is a `Failed` note on an otherwise
    // successful pass — the ops still move, and nothing lands in `errors`, which is what feeds the
    // service's backoff.
    let relay = spawn_relay_failing_presence(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    let (_t, ws) = workspace_with_remote("git@github.com:acme/failing-presence.git");
    bind_derived_with_endpoint(&ws, &relay);
    create_item_titled(&ws, "still synced");
    let (_h, home) = machine("MacBook");

    let report = daemon::sweep(
        std::slice::from_ref(&ws),
        &ServicePass::new(home, 300),
        None,
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let outcome = &report.synced[0].1;
    assert!(outcome.pushed > 0, "the ops still reached the relay");
    let Some(PresenceNote::Failed(msg)) = &outcome.presence else {
        panic!("expected a failed announcement, got {:?}", outcome.presence);
    };
    assert!(msg.contains("500"), "{msg}");
}

#[test]
fn a_machine_file_that_cannot_be_read_costs_the_announcement_never_the_sync() {
    let (relay, _now) = spawn_relay_with_clock();
    let (_t, ws) = workspace_with_remote("git@github.com:acme/broken-identity.git");
    bind_derived_with_endpoint(&ws, &relay);
    create_item_titled(&ws, "still synced");
    let dir = TempDir::new().unwrap();
    let home = ServiceHome::at(dir.path().join(".nexusflow"));
    std::fs::create_dir_all(home.root()).unwrap();
    std::fs::write(home.machine_file(), "id = [broken").unwrap();

    let report = daemon::sweep(
        std::slice::from_ref(&ws),
        &ServicePass::new(home, 300),
        None,
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let outcome = &report.synced[0].1;
    assert!(outcome.pushed > 0);
    let Some(PresenceNote::Failed(msg)) = &outcome.presence else {
        panic!("expected a failed announcement, got {:?}", outcome.presence);
    };
    assert!(msg.contains("machine.toml"), "names the file: {msg}");
    assert_eq!(
        seen_from(&ws),
        Vec::<(String, bool)>::new(),
        "nothing was announced"
    );
}

#[test]
fn a_pass_that_fails_announces_nothing() {
    // A sighting means "synced just now": a relay that cannot be reached gets no announcement, which
    // is what lets an unreachable machine drop out of everyone's list on its own.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let (_t, ws) = workspace_with_remote("git@github.com:acme/unreachable.git");
    bind_derived_with_endpoint(&ws, &closed);
    let (_h, home) = machine("MacBook");
    let report = daemon::sweep(
        std::slice::from_ref(&ws),
        &ServicePass::new(home, 300),
        None,
    );
    assert_eq!(report.errors.len(), 1, "the pass itself failed");
    assert!(
        report.synced.is_empty(),
        "and so there is no outcome to carry an announcement"
    );
}

#[test]
fn a_legacy_stream_id_that_needs_encoding_reads_its_machines_too() {
    // The p5sa shape (`nxsflow/manufakt-io`, from before the bind gate) must reach `/machines` as
    // one path segment, like every other route.
    let (relay, _now) = spawn_relay_with_clock();
    let (_tmp, ws) = bound_workspace_with_raw_id("nxsflow/manufakt-io");
    let (_h, home) = machine("MacBook");
    let report = daemon::sweep(
        std::slice::from_ref(&ws),
        &ServicePass::new(home, 300),
        Some(relay.as_str()),
    );
    assert_eq!(report.synced[0].1.presence, Some(PresenceNote::Recorded));
    assert_eq!(
        sync::read_machines(&ws, Some(&relay)).unwrap().stream_id,
        "nxsflow/manufakt-io"
    );
    let Presence::Reported { machines, .. } =
        sync::read_machines(&ws, Some(&relay)).unwrap().presence
    else {
        panic!("reported");
    };
    assert_eq!(machines.len(), 1);
}

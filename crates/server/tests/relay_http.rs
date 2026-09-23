//! T3 acceptance: append + read_since end-to-end over HTTP against the SQLite OpStore.
//! The relay is dumb — it appends every well-formed op and never rejects one for shape.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use nxs_server::app::{app, app_with_clock, MAX_PUSH_OPS};
use nxs_server::registry::PrefixRegistry;
use nxs_server::registry::SqlitePrefixRegistry;
use nxs_server::store::SqliteOpStore;
use nxs_server::store::StoreResult;
use nxs_sync::presence::{MAX_INTERVAL_SECS, MAX_LISTED_MACHINES, RETENTION_SECS};
use nxs_sync::protocol::{
    MachineSeen, MachinesResponse, PullResponse, PushResponse, RegisterOutcome, StreamId,
};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

/// Start the relay on an ephemeral loopback port in a background thread; return its URL.
fn spawn_relay() -> String {
    let ops = Arc::new(SqliteOpStore::open_in_memory().unwrap());
    let registry = Arc::new(SqlitePrefixRegistry::open_in_memory().unwrap());
    let router = app(ops, registry);
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

fn wire(op_id: &str, op_type: &str) -> WireOp {
    WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: op_id.into(),
        lamport: 1,
        site: 7,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "ab12.0001".into(),
        field: "title".into(),
        op_type: op_type.into(),
        value: Some("hello".into()),
        author: "alice".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

fn push(base: &str, stream: &str, ops: &[WireOp]) -> PushResponse {
    let body = serde_json::json!({ "ops": ops });
    let resp = ureq::post(&format!("{base}/streams/{stream}/ops"))
        .send_json(body)
        .unwrap();
    resp.into_json().unwrap()
}

fn pull(base: &str, stream: &str, since: i64, limit: usize) -> PullResponse {
    let resp = ureq::get(&format!("{base}/streams/{stream}/ops"))
        .query("since", &since.to_string())
        .query("limit", &limit.to_string())
        .call()
        .unwrap();
    resp.into_json().unwrap()
}

#[test]
fn append_then_read_since_round_trips_over_http() {
    let base = spawn_relay();
    let appended = push(&base, "s1", &[wire("o1", "set"), wire("o2", "set")]);
    assert_eq!(appended.appended, 2);

    let page = pull(&base, "s1", 0, 100);
    assert_eq!(
        page.ops
            .iter()
            .map(|o| o.op_id.as_str())
            .collect::<Vec<_>>(),
        ["o1", "o2"]
    );
    assert_eq!(page.next.0, 2);

    // Reading from the watermark yields nothing new.
    let empty = pull(&base, "s1", 2, 100);
    assert!(empty.ops.is_empty());
    assert_eq!(empty.next.0, 2);
}

#[test]
fn relay_accepts_an_op_with_an_unknown_shape() {
    // Server opacity (§4.1): an op_type the core does not know is still stored & served.
    let base = spawn_relay();
    let mut op = wire("future-1", "future_set");
    op.envelope_version = 999;
    op.target_kind = "gizmo".into();
    let appended = push(&base, "s1", std::slice::from_ref(&op));
    assert_eq!(appended.appended, 1);

    let page = pull(&base, "s1", 0, 100);
    assert_eq!(page.ops.len(), 1);
    assert_eq!(page.ops[0], op, "unknown op stored & served verbatim");
}

#[test]
fn a_newer_clients_signed_op_crosses_an_older_relay_still_signed() {
    // 6j6v.5crb end-to-end, as the mixed version stand it is actually about: a self-hosted relay
    // is normally a version behind at least one of its clients. The newer client pushes an op
    // carrying a field this relay build does not know — `sig`, once 6j6v.6aza lands — and the
    // receiving client must pull it back with the signature intact. When the relay dropped it,
    // the receiver saw a well-formed UNSIGNED op and had no way to tell that from an op nobody
    // ever signed: a silent downgrade on exactly the path signing protects.
    //
    // Deliberately driven as RAW JSON on both ends rather than through `WireOp`, so the assertion
    // holds against the bytes on the wire and cannot be satisfied by the test and the relay
    // sharing a type.
    let base = spawn_relay();
    let mut signed = serde_json::to_value(wire("signed-1", "set")).unwrap();
    signed["sig"] = serde_json::json!("ed25519:deadbeef");
    signed["key_id"] = serde_json::json!("01HRUUIDEXAMPLE0000000000");

    let resp = ureq::post(&format!("{base}/streams/s1/ops"))
        .send_json(serde_json::json!({ "ops": [signed.clone()] }))
        .unwrap();
    assert_eq!(
        resp.into_json::<PushResponse>().unwrap().appended,
        1,
        "the relay accepts an op with fields it does not know"
    );

    let served: serde_json::Value = ureq::get(&format!("{base}/streams/s1/ops"))
        .query("since", "0")
        .query("limit", "100")
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(
        served["ops"][0], signed,
        "the op comes back off the relay exactly as it went in — signature and key id included"
    );
}

#[test]
fn re_pushing_a_batch_over_http_does_not_duplicate() {
    // Idempotency through the HTTP layer (§4.4): a retried push (same op_ids) leaves the
    // authoritative log with one copy of each op, and the cursor stays monotone — it does
    // not advance past already-stored ops.
    let base = spawn_relay();
    let batch = [wire("o1", "set"), wire("o2", "set"), wire("o3", "set")];
    push(&base, "s1", &batch);
    let first = pull(&base, "s1", 0, 100);
    assert_eq!(first.ops.len(), 3);
    assert_eq!(first.next.0, 3);

    // Re-send the identical batch (as a retry would).
    push(&base, "s1", &batch);
    let again = pull(&base, "s1", 0, 100);
    assert_eq!(
        again
            .ops
            .iter()
            .map(|o| o.op_id.as_str())
            .collect::<Vec<_>>(),
        ["o1", "o2", "o3"],
        "each op stored exactly once after a retry"
    );
    assert_eq!(again.next.0, 3, "cursor did not advance past duplicates");
}

#[test]
fn an_oversized_op_batch_is_rejected_with_413() {
    // k64: the relay caps the per-request op count deliberately (413), so a misbehaving or
    // malicious client cannot force unbounded per-request work. Well-behaved clients paginate
    // push below this cap, so they never trip it. The cap is intentional, not axum's implicit
    // byte default.
    let base = spawn_relay();
    let oversized: Vec<WireOp> = (0..=MAX_PUSH_OPS)
        .map(|i| wire(&format!("o{i}"), "set"))
        .collect();
    let body = serde_json::json!({ "ops": oversized });
    let err = ureq::post(&format!("{base}/streams/s1/ops"))
        .send_json(body)
        .expect_err("an over-cap batch must be rejected");
    match err {
        ureq::Error::Status(code, _) => assert_eq!(code, 413, "Payload Too Large"),
        other => panic!("expected an HTTP 413 status, got {other:?}"),
    }
    // The rejected batch was not partially appended — the stream is still empty.
    assert!(
        pull(&base, "s1", 0, 100).ops.is_empty(),
        "nothing was stored"
    );
}

#[test]
fn a_full_page_sized_batch_is_accepted() {
    // The cap must sit ABOVE the client's push page size so a legitimate full page never
    // trips it. A batch of exactly MAX_PUSH_OPS ops is accepted.
    let base = spawn_relay();
    let batch: Vec<WireOp> = (0..MAX_PUSH_OPS)
        .map(|i| wire(&format!("o{i}"), "set"))
        .collect();
    let appended = push(&base, "s1", &batch);
    assert_eq!(appended.appended, MAX_PUSH_OPS, "the cap itself is allowed");
}

#[test]
fn streams_are_isolated() {
    let base = spawn_relay();
    push(&base, "a", &[wire("a1", "set")]);
    push(&base, "b", &[wire("b1", "set"), wire("b2", "set")]);
    assert_eq!(pull(&base, "a", 0, 100).ops.len(), 1);
    assert_eq!(pull(&base, "b", 0, 100).ops.len(), 2);
}

fn register(base: &str, stream: &str, prefix: &str, uuid: &str) -> RegisterOutcome {
    let body = serde_json::json!({ "prefix": prefix, "replica_uuid": uuid });
    let resp = ureq::post(&format!("{base}/streams/{stream}/register"))
        .send_json(body)
        .unwrap();
    resp.into_json().unwrap()
}

#[test]
fn register_round_trips_and_reassigns_a_colliding_prefix_over_http() {
    // bab/§6 end-to-end over HTTP: a free prefix registers; a second replica that minted
    // the SAME prefix offline is reassigned a distinct one — no aliasing at merge time.
    let base = spawn_relay();
    assert_eq!(
        register(&base, "s1", "aaaa", "uuid-A"),
        RegisterOutcome::Registered
    );
    let outcome = register(&base, "s1", "aaaa", "uuid-B");
    let RegisterOutcome::Reassigned { new_prefix } = outcome else {
        panic!("expected Reassigned, got {outcome:?}");
    };
    assert_ne!(new_prefix, "aaaa");
}

// ----- machine presence (6j6v.f0b5) ------------------------------------------------------------

/// The relay with a clock the test turns by hand — presence is judged by AGE, and a test that had
/// to wait eleven minutes for one to grow would not be run.
fn spawn_relay_with_clock() -> (String, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000));
    let clock_now = Arc::clone(&now);
    let ops = Arc::new(SqliteOpStore::open_in_memory().unwrap());
    let registry = Arc::new(SqlitePrefixRegistry::open_in_memory().unwrap());
    let router = app_with_clock(
        ops,
        registry,
        Arc::new(move || clock_now.load(Ordering::SeqCst)),
    );
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
    (format!("http://{addr}"), now)
}

fn announce(base: &str, stream: &str, machine_id: &str, name: &str) -> u16 {
    let body = serde_json::json!({ "machine_id": machine_id, "name": name, "interval_secs": 300 });
    match ureq::post(&format!("{base}/streams/{stream}/machines")).send_json(body) {
        Ok(resp) => resp.status(),
        Err(ureq::Error::Status(status, _)) => status,
        Err(e) => panic!("transport error: {e}"),
    }
}

fn machines(base: &str, stream: &str) -> MachinesResponse {
    ureq::get(&format!("{base}/streams/{stream}/machines"))
        .call()
        .unwrap()
        .into_json()
        .unwrap()
}

#[test]
fn an_announced_machine_is_listed_with_its_age_on_the_relay_s_clock() {
    let (base, now) = spawn_relay_with_clock();
    assert_eq!(announce(&base, "s1", "m1", "MacBook"), 204);
    now.store(1_042, Ordering::SeqCst);
    assert_eq!(
        machines(&base, "s1").machines,
        [MachineSeen {
            machine_id: "m1".into(),
            name: "MacBook".into(),
            last_seen: 1_000,
            age_secs: 42,
            interval_secs: 300,
        }]
    );
}

#[test]
fn a_stream_nobody_announced_for_lists_no_machines_rather_than_failing() {
    let (base, _) = spawn_relay_with_clock();
    assert!(machines(&base, "never-announced").machines.is_empty());
}

#[test]
fn an_announcement_never_enters_the_op_log() {
    // Presence is state, not history (6j6v.f0b5): a heartbeat per machine per pass in the log would
    // grow every stream forever and replay into every replica. Checked on a stream that already
    // carries an op, with the announcement proven accepted — a refused one would prove nothing.
    let (base, _) = spawn_relay_with_clock();
    push(&base, "s1", &[wire("o1", "set")]);
    assert_eq!(announce(&base, "s1", "m1", "MacBook"), 204);
    assert_eq!(machines(&base, "s1").machines.len(), 1);
    let page = pull(&base, "s1", 0, 100);
    assert_eq!(
        page.ops
            .iter()
            .map(|o| o.op_id.as_str())
            .collect::<Vec<_>>(),
        ["o1"],
        "the log holds the op and nothing else"
    );
    assert_eq!(page.next.0, 1, "the stream's cursor did not move past it");
}

fn announce_body(base: &str, stream: &str, body: serde_json::Value) -> u16 {
    match ureq::post(&format!("{base}/streams/{stream}/machines")).send_json(body) {
        Ok(resp) => resp.status(),
        Err(ureq::Error::Status(status, _)) => status,
        Err(e) => panic!("transport error: {e}"),
    }
}

#[test]
fn every_bound_on_an_announcement_is_enforced_at_the_relay() {
    let (base, _) = spawn_relay_with_clock();
    let hello = |id: &str, name: &str, interval: u64| serde_json::json!({ "machine_id": id, "name": name, "interval_secs": interval });
    for (body, why) in [
        (hello("", "MacBook", 300), "empty id"),
        (hello(&"x".repeat(65), "MacBook", 300), "id too long"),
        (hello("has space", "MacBook", 300), "id with a space"),
        (hello("m1", "   ", 300), "blank name"),
        (hello("m1", &"x".repeat(65), 300), "name too long"),
        (
            hello("m1", "Mac\u{202E}mini", 300),
            "direction override in the name",
        ),
        (hello("m1", "MacBook", 0), "interval 0"),
        (
            hello("m1", "MacBook", MAX_INTERVAL_SECS + 1),
            "interval past the cap",
        ),
    ] {
        assert_eq!(announce_body(&base, "s1", body), 400, "{why}");
    }
    assert!(machines(&base, "s1").machines.is_empty(), "nothing stored");
    assert_eq!(
        announce_body(
            &base,
            "s1",
            hello(&"x".repeat(64), "MacBook", MAX_INTERVAL_SECS)
        ),
        204,
        "the edges themselves are accepted"
    );
}

#[test]
fn an_announcement_body_is_capped_far_below_the_push_limit() {
    // A valid hello is under 300 bytes; the route must not buffer megabytes before refusing.
    //
    // Driven through the router in process, not over a socket: over TCP the relay answers 413 and
    // hangs up mid-upload, and whether the client then reads the 413 or a broken pipe depends on the
    // platform's timing — the dispatch run of PR #485 saw the pipe. The limit itself is the router's.
    use tower::ServiceExt;
    let status_for = |padding: usize| {
        let router = app(
            Arc::new(SqliteOpStore::open_in_memory().unwrap()),
            Arc::new(SqlitePrefixRegistry::open_in_memory().unwrap()),
        );
        let body = serde_json::json!({
            "machine_id": "m1", "name": "MacBook", "interval_secs": 300,
            "padding": "x".repeat(padding)
        })
        .to_string();
        let request = axum::http::Request::post("/streams/s1/machines")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(router.oneshot(request))
            .unwrap()
            .status()
    };
    assert_eq!(status_for(64 * 1024), 413, "64 KiB is refused");
    assert_eq!(
        status_for(3 * 1024),
        204,
        "3 KiB with the same unknown field is accepted — it is the limit, not the field"
    );
}

#[test]
fn a_flood_of_machines_is_cut_to_the_most_recent_and_the_answer_says_so() {
    // Nothing stops an anonymous caller inventing machines inside the window (only relay
    // authentication can, 6j6v.6aza). What the relay can do is bound the answer and say it is not
    // the whole list, so a reader never presents a flooded list as complete.
    let (base, now) = spawn_relay_with_clock();
    for i in 0..=MAX_LISTED_MACHINES {
        now.store(1_000 + i as i64, Ordering::SeqCst);
        announce(&base, "s1", &format!("m{i:03}"), "Spam");
    }
    let listed = machines(&base, "s1");
    assert_eq!(listed.machines.len(), MAX_LISTED_MACHINES);
    assert_eq!(
        listed.machines[0].machine_id,
        format!("m{MAX_LISTED_MACHINES:03}"),
        "newest first"
    );
    assert!(listed.truncated, "the relay knows more than it returned");

    let (base, _) = spawn_relay_with_clock();
    announce(&base, "s1", "m1", "MacBook");
    assert!(
        !machines(&base, "s1").truncated,
        "an honest stream is not truncated"
    );
}

#[test]
fn a_machine_nobody_heard_from_for_the_retention_window_is_forgotten() {
    let (base, now) = spawn_relay_with_clock();
    announce(&base, "s1", "ghost", "Reinstalled laptop");
    now.fetch_add(RETENTION_SECS - 1, Ordering::SeqCst);
    assert_eq!(
        machines(&base, "s1").machines.len(),
        1,
        "still inside the window"
    );
    now.fetch_add(2, Ordering::SeqCst);
    assert!(
        machines(&base, "s1").machines.is_empty(),
        "past the window it is not listed"
    );
    announce(&base, "s1", "live", "MacBook");
    let listed = machines(&base, "s1").machines;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].machine_id, "live");
}

/// A prefix registry written the way an embedder wrote one before presence existed: it implements
/// `PrefixRegistry` and nothing else.
struct RegistryFromBeforePresence;

impl PrefixRegistry for RegistryFromBeforePresence {
    fn register(
        &self,
        _stream: &StreamId,
        _prefix: &str,
        _replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome> {
        Ok(RegisterOutcome::Registered)
    }
}

#[test]
fn a_registry_written_before_presence_still_builds_the_relay_which_then_answers_like_an_old_one() {
    // `SharedRegistry` did not change: a registry without a presence board still fits `app()`, and
    // the relay answers the presence route with 404 — what every client reads as "no presence
    // here" — while every other route works as before.
    let ops = Arc::new(SqliteOpStore::open_in_memory().unwrap());
    let router = app(ops, Arc::new(RegistryFromBeforePresence));
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
    let base = format!("http://{addr}");
    assert_eq!(announce(&base, "s1", "m1", "MacBook"), 404);
    match ureq::get(&format!("{base}/streams/s1/machines")).call() {
        Err(ureq::Error::Status(404, _)) => {}
        other => panic!("expected 404, got {other:?}"),
    }
    assert_eq!(
        register(&base, "s1", "aaaa", "uuid-A"),
        RegisterOutcome::Registered
    );
    assert_eq!(push(&base, "s1", &[wire("o1", "set")]).appended, 1);
}

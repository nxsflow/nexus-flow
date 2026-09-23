//! The long-lived engine handle (E5 #9t7.2): in-process reads held across many calls, reading
//! the live db, and safe to share across async tasks (`Clone + Send + Sync`).

use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::read;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use nexus_flow_facade::write::NewItem;
use nexus_flow_facade::{LinkRelation, LinkWeight, ThreadLink};
use std::time::Duration;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";
const ACTOR: &str = "alice";

/// The mandatory-only `create` payload, for the write-path tests.
fn minimal<'a>(description: &'a str, priority: &'a str) -> NewItem<'a> {
    NewItem {
        description,
        priority,
        design: None,
        dod: None,
        due: None,
        defer: None,
        parent: None,
        depends_on: &[],
        custom: &[],
    }
}

/// Delete + recreate the workspace db at the SAME path, seeded with `ids` — a fresh inode at the
/// old path, mirroring `nxf init` over an existing workspace, an E4 sync reset, or rm + recreate.
/// Any store still open on the old file is now pinned to the now-unlinked inode.
fn recreate_db_with(dir: &std::path::Path, ids: &[&str]) {
    let ws = workspace::discover(dir).unwrap();
    let db = ws.db_path();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    let mut store = ws.open_store().unwrap();
    for id in ids {
        // Each `create_item` commits its op (the store wraps every op in BEGIN/COMMIT), so the
        // recreated db and its rows are durably on disk by the time the asserting `engine.list().unwrap()`
        // reads them — the same cross-connection durability `handle_sees_writes_from_another_connection`
        // relies on. No explicit flush is needed.
        store.create_item(id, "task", "x", "t");
    }
}

/// Initialize a fresh workspace and seed two open tasks, returning the temp dir (kept alive for
/// the test's duration so the `.nexusflow` directory survives).
fn seeded_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let mut store = ws.open_store().unwrap();
    store.create_item("ab12.0001", "task", "A", "t");
    store.set_field("ab12.0001", "priority", Some("0".into()), "t");
    store.create_item("ab12.0002", "task", "B", "t");
    store.set_field("ab12.0002", "priority", Some("1".into()), "t");
    tmp
}

#[test]
fn handle_reads_in_process_and_is_reusable_across_calls() {
    let tmp = seeded_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();

    // One handle answers many reads — it is not open-per-call.
    assert_eq!(engine.list(None, None).unwrap().len(), 2);
    assert_eq!(engine.show("ab12.0001").unwrap().item.id, "ab12.0001");
    assert!(
        engine.show("ab12.9999").is_err(),
        "missing item is not_found"
    );
    assert!(engine.prime(NOW, false).unwrap().next_total >= 2);

    // `next` is ranked by the workspace plugin's policy (priority 0 ahead of 1).
    let next: Vec<_> = engine
        .next(NOW)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(next, ["ab12.0001", "ab12.0002"]);

    // The handle exposes the selected plugin config for consumers that render themselves.
    assert_eq!(engine.plugin_config().name, "issue-tracker");
}

#[test]
fn read_methods_reject_a_malformed_now_with_a_validation_error() {
    // The time-dependent reads (`ready`/`next`/`prime`) take `now` as an explicit parameter; an
    // embedding host passing a non-date must fail loudly with a `validation` kind rather than
    // silently mis-deriving the ready/deferred split (nexus-flow-9t7.9). `blocked`/`list` take no
    // `now` and stay infallible; the CLI pre-validates `now`, so its reads are unaffected.
    let tmp = seeded_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();

    for kind in [
        engine.next("garbage").unwrap_err().kind,
        engine.prime("garbage", false).unwrap_err().kind,
        // C3/C6 (#916.4/#916.6): the new time-dependent reads (`deferred`'s defer boundary,
        // `search`'s lane grouping) validate `now` the same way.
        engine.deferred("garbage").unwrap_err().kind,
        engine
            .search("garbage", "x", None, None, false, false)
            .unwrap_err()
            .kind,
    ] {
        assert_eq!(kind, ErrorKind::Validation);
    }

    // A well-formed `now` still computes the same records as before — pinned, not just `is_ok`,
    // so the guard is shown to reject *only* the malformed input (review, Test Quality #3).
    let next: Vec<_> = engine
        .next(NOW)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(next, ["ab12.0001", "ab12.0002"]);
    assert_eq!(engine.prime(NOW, false).unwrap().next_total, 2);
}

#[test]
fn reads_repoint_after_a_same_path_db_recreate() {
    // nexus-flow-2at: a same-path delete+recreate of the workspace db (nxf init over an existing
    // workspace, an E4 sync reset, rm+recreate) leaves the handle's store pinned to the now-unlinked
    // inode, which keeps answering STALE rows WITHOUT erroring. The change watcher (#i8o) already
    // re-arms its poll connection on such a swap; the handle's OWN read store must likewise re-point
    // on the next access and read FRESH data, not the dead file. No watcher/subscribe is involved —
    // re-pointing is on-access, so the test is fully deterministic.
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    {
        let mut store = ws.open_store().unwrap();
        store.create_item("ab12.0001", "task", "A", "t");
    }
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        engine
            .list(None, None)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        ["ab12.0001"],
        "reads the original db"
    );

    // Delete + recreate the db at the SAME path with DIFFERENT content (a fresh inode). The handle's
    // store is now pinned to the unlinked inode; without re-pointing it keeps reading the stale A.
    recreate_db_with(tmp.path(), &["ab12.0002", "ab12.0003"]);

    let ids: Vec<String> = engine
        .list(None, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0003"],
        "the handle re-points to the recreated db and reads fresh data, not the stale A"
    );
}

#[test]
fn subscribed_reads_repoint_after_a_recreate_on_the_watcher_cadence() {
    // nexus-flow-uh3: the per-access identity stat is lifted off the hot read path — while a
    // subscriber keeps the watcher polling, reads reuse the watcher's per-tick file_id instead of
    // stat-ing themselves. This pins that the reset detection from #2at/#i8o is NOT weakened by
    // that shift: once the watcher observes the recreated inode (its normal poll cadence, awaited
    // here through the Change channel), reads through the handle return FRESH data, not the stale
    // pre-recreate rows. The unsubscribed on-access path stays covered by
    // `reads_repoint_after_a_same_path_db_recreate`.
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    {
        let mut store = ws.open_store().unwrap();
        store.create_item("ab12.0001", "task", "A", "t");
    }
    let engine =
        Engine::open_with_poll_interval(None, tmp.path(), Duration::from_millis(40)).unwrap();
    let rx = engine.subscribe().unwrap();
    assert_eq!(
        engine
            .list(None, None)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        ["ab12.0001"],
        "reads the original db"
    );

    // Delete + recreate the db at the SAME path with DIFFERENT content (a fresh inode). The handle's
    // store is now pinned to the unlinked inode; the subscribed read path no longer stats per access,
    // so it leans on the watcher to notice the swap.
    recreate_db_with(tmp.path(), &["ab12.0002", "ab12.0003"]);

    // Await the watcher observing the same-path recreate (its identity-change re-arm fires a
    // Change, #i8o) — by which point its published identity is the new inode.
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the watcher signals the same-path recreate");

    let ids: Vec<String> = engine
        .list(None, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0003"],
        "subscribed reads re-point to the recreated db off the watcher's identity — reset detection intact"
    );
}

#[test]
fn subscribed_writes_repoint_immediately_not_on_the_watcher_cadence() {
    // nexus-flow-uh3 scopes the stat-lifting to READS. A write must never land on the old, unlinked
    // inode (it would commit to a file no one else can see — silently lost), so writes keep stat-ing
    // for an IMMEDIATE re-point even while subscribed; they do NOT defer to the watcher cadence.
    // Pin that with a poll interval so long the watcher cannot tick during the test: a write right
    // after a same-path recreate still lands durably in the NEW db, verified through a fresh
    // connection (not the engine, so engine bookkeeping can't mask it).
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine =
        Engine::open_with_poll_interval(None, tmp.path(), Duration::from_secs(3600)).unwrap();
    let _rx = engine.subscribe().unwrap();
    engine
        .create(NOW, ACTOR, "bug", "A", minimal("why-A", "P1"))
        .unwrap();

    // Recreate the db at the same path (empty + a fresh inode). The watcher will not tick for an
    // hour, so only the write path's own stat can notice the swap.
    recreate_db_with(tmp.path(), &[]);
    let b = engine
        .create(NOW, ACTOR, "bug", "B", minimal("why-B", "P1"))
        .unwrap();

    // Read the NEW db file directly (foreign connection): B is durably there and the pre-recreate A
    // is gone — the write re-pointed immediately rather than committing to the dead inode.
    let fresh = ws.open_store().unwrap();
    let cfg = nexus_flow_facade::plugin::load("issue-tracker").unwrap();
    let ids: Vec<String> = read::list(&cfg, &fresh, None, None, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        [b.id],
        "the post-recreate write landed durably in the new db, not the dead inode"
    );
}

#[test]
fn reads_surface_a_persistent_repoint_failure_instead_of_serving_stale() {
    // Review I&R #3: a same-path recreate whose new file is *persistently* unopenable (here: a
    // directory occupies the db path) must NOT degrade into indefinite silent stale reads. The
    // best-effort reopen fails on every access; a fallible read surfaces it as `io` rather than
    // returning rows from the old, now-unlinked inode forever. The two infallible reads
    // (`blocked`/`list`) stay best-effort by design — they carry no `Result` to surface it; the
    // read-method contract is finalized in #9t7.7.
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    {
        let mut store = ws.open_store().unwrap();
        store.create_item("ab12.0001", "task", "A", "t");
    }
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        engine.list(None, None).unwrap().len(),
        1,
        "reads the original db"
    );

    // Replace the db file with a DIRECTORY at the same path: the file identity changes (so a
    // re-point is attempted) but `open_store` fails *persistently* — a directory is not a valid
    // sqlite file, and it stays there, so every access re-attempts and re-fails.
    let db = ws.db_path();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    std::fs::create_dir(&db).unwrap();

    let err = engine.next(NOW).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Io,
        "a persistent re-point failure surfaces as io, not stale data"
    );

    // #76u.13: `list` is fallible now too, so — like `ready` above — a persistent re-point failure
    // surfaces as `io` rather than serving the last-known (stale) rows. Every Engine read now reports
    // the failure instead of masking it; none silently serves stale data on a broken store.
    assert_eq!(
        engine.list(None, None).unwrap_err().kind,
        ErrorKind::Io,
        "a fallible read surfaces the re-point failure, not stale data"
    );
}

#[test]
fn handle_sees_writes_from_another_connection() {
    // A foreign writer (== another process, to SQLite) commits while the handle is open; the
    // next read through the handle reflects it. The push-based notification of this is #9t7.3.
    let tmp = seeded_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(engine.list(None, None).unwrap().len(), 2);

    let ws = workspace::discover(tmp.path()).unwrap();
    let mut other = ws.open_store().unwrap();
    other.create_item("ab12.0003", "task", "C", "t");
    drop(other);

    assert_eq!(
        engine.list(None, None).unwrap().len(),
        3,
        "handle reads the live db"
    );
}

#[test]
fn handle_writes_in_process_and_reads_them_back() {
    // The write half of the handle (#9t7.5): mutate through the same long-lived handle and read
    // the result back — no `nxf` spawn, same store, same derivation.
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why-A", "P1"))
        .unwrap();
    let b = engine
        .create(NOW, ACTOR, "bug", "B", minimal("why-B", "P2"))
        .unwrap();
    assert_eq!(
        engine.list(None, None).unwrap().len(),
        2,
        "both creates persisted"
    );

    // A depends on B → A is blocked, B is ready.
    engine.dep_add(NOW, ACTOR, &a.id, &b.id).unwrap();
    let ready: Vec<_> = engine
        .next(NOW)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ready, [b.id.as_str()], "A is blocked by B; only B is ready");

    // Claim, note, then close B; the reads through the handle reflect each mutation.
    engine.claim(NOW, ACTOR, &b.id, Some("bob")).unwrap();
    assert_eq!(
        engine.show(&b.id).unwrap().item.status.as_deref(),
        Some("in_progress")
    );
    let note_id = engine.note_add(NOW, ACTOR, &b.id, "worklog").unwrap();
    assert_eq!(
        engine.show(&b.id).unwrap().notes,
        vec![(note_id, "worklog".into())]
    );

    engine.close(NOW, ACTOR, &b.id, Some("done")).unwrap();
    // With B closed, A becomes ready.
    let ready: Vec<_> = engine
        .next(NOW)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ready, [a.id], "closing the blocker unblocks A");
}

#[test]
fn handle_write_stamps_explicit_now_and_actor_into_the_ops() {
    // now → op.wall_clock, actor → op.author, both from the explicit parameters (#9t7.5). Read
    // the raw ops back over a foreign connection to the same db file.
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();

    let ops = ws.open_store().unwrap().export();
    assert!(!ops.is_empty());
    assert!(
        ops.iter().all(|o| o.wall_clock == NOW && o.author == ACTOR),
        "every op the handle wrote carries the explicit now + actor"
    );
}

#[test]
fn handle_write_validation_errors_surface_with_their_kind() {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    // Unknown type → validation; nothing persisted.
    let err = engine
        .create(NOW, ACTOR, "nonsense", "A", minimal("why", "P1"))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        engine.list(None, None).unwrap().is_empty(),
        "rejected create wrote nothing"
    );

    // Cycle rejection at write time.
    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();
    let b = engine
        .create(NOW, ACTOR, "bug", "B", minimal("why", "P1"))
        .unwrap();
    engine.dep_add(NOW, ACTOR, &a.id, &b.id).unwrap();
    let err = engine.dep_add(NOW, ACTOR, &b.id, &a.id).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cycle);

    // Missing item → not_found.
    let err = engine
        .update(NOW, ACTOR, "ab12.9999", &["title=x".into()])
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn writes_recover_after_a_poisoned_lock() {
    // The handle is process-lifetime and shared (`Arc`); a panic-while-locked must not brick it.
    // Reads already recover (the read test); a write on another clone must recover too, and the
    // write must actually land — the store mutation is transactional, so there is no torn state.
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    let poisoner = engine.clone();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Panic while holding the lock during a write attempt.
        poisoner
            .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
            .unwrap();
        panic!("poison the lock after a successful write");
    }));
    std::panic::set_hook(prev_hook);
    assert!(res.is_err(), "the poisoning closure panicked as set up");

    // The lock is poisoned; a write on another clone still works (recovered, not cascaded).
    let b = engine
        .create(NOW, ACTOR, "bug", "B", minimal("why", "P1"))
        .unwrap();
    assert_eq!(engine.show(&b.id).unwrap().item.title.as_deref(), Some("B"));
}

#[test]
fn handle_is_clone_send_sync() {
    fn assert_send_sync<T: Send + Sync + Clone + 'static>() {}
    assert_send_sync::<Engine>();

    let tmp = seeded_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let clone = engine.clone();
    // Move the clone to another thread and read concurrently with this one.
    let handle = std::thread::spawn(move || clone.next(NOW).unwrap().len());
    let here = engine.next(NOW).unwrap().len();
    let there = handle.join().unwrap();
    assert_eq!(here, there);
}

#[test]
fn concurrent_reads_under_a_live_writer_stay_consistent() {
    // Stress the `Mutex<State>`: many reader threads hammer one shared handle while a foreign
    // connection keeps committing new items. Every read must return a coherent snapshot — never
    // a torn or shrinking view, never a panic (#9t7.2 review, Test Quality #1).
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let tmp = seeded_workspace(); // starts with two items
    let engine = Engine::open(None, tmp.path()).unwrap();

    // Background writer on its OWN connection — only ever appends, never deletes.
    let writer_dir = tmp.path().to_path_buf();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_w = Arc::clone(&stop);
    let writer = std::thread::spawn(move || {
        let ws = workspace::discover(&writer_dir).unwrap();
        let mut store = ws.open_store().unwrap();
        let mut n = 0;
        while !stop_w.load(Ordering::SeqCst) && n < 60 {
            store.create_item(&format!("ab12.1{n:03}"), "task", "w", "t");
            n += 1;
        }
    });

    let mut readers = Vec::new();
    for _ in 0..8 {
        let e = engine.clone();
        readers.push(std::thread::spawn(move || {
            let mut prev = 0usize;
            for _ in 0..60 {
                let items = e.list(None, None).unwrap();
                // The writer only appends, so a coherent snapshot never shrinks; and every row
                // is well-formed (a real id) — no torn read leaking through the lock.
                assert!(
                    items.len() >= prev,
                    "list count went backwards: {} < {prev}",
                    items.len()
                );
                prev = items.len();
                assert!(
                    items.iter().all(|i| !i.id.is_empty()),
                    "every row has an id"
                );
                e.next(NOW).unwrap();
            }
        }));
    }
    for r in readers {
        r.join().unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();

    // Everything committed is visible afterward (seed + all appended items).
    assert!(engine.list(None, None).unwrap().len() >= 2);
}

// ---- h89s user-labels through the handle (bug: unreachable on Engine) ------

#[test]
fn handle_adds_reads_and_removes_user_labels() {
    // Labels are the surface app-foundations consumes via the Engine handle. Add, read back sorted,
    // and remove — the same OR-set the CLI/MCP drive, now reachable on `Engine` (mirrors mention/dep).
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();

    engine.label_add(NOW, ACTOR, &a.id, "urgent").unwrap();
    engine.label_add(NOW, ACTOR, &a.id, "backend").unwrap();
    assert_eq!(
        engine.labels(&a.id).unwrap(),
        vec!["backend".to_string(), "urgent".to_string()],
        "labels read back sorted through the handle"
    );

    engine.label_remove(NOW, ACTOR, &a.id, "urgent").unwrap();
    assert_eq!(engine.labels(&a.id).unwrap(), vec!["backend".to_string()]);

    // A label read on a missing item is `not_found`, mirroring `show`.
    assert_eq!(
        engine.labels("ab12.9999").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn handle_read_path_carries_labels_on_records() {
    // Issue-1 #4 pinned as option (b): the label-enriched JSON reads ride through the handle via the
    // SAME *_to_value projections the CLI/MCP use, so an Engine-only consumer sees labels ON records
    // without ItemRow/ShowRecord (the byte-identity contract) having to carry a labels field.
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();
    // An unlabelled sibling B exercises the SPARSE key on a MIXED board: only A's record carries
    // `labels`; B's must omit the key entirely (byte-identical to the pre-labels output).
    let b = engine
        .create(NOW, ACTOR, "bug", "B", minimal("why", "P1"))
        .unwrap();
    engine.label_add(NOW, ACTOR, &a.id, "urgent").unwrap();

    let show = engine.show_value(&a.id).unwrap();
    assert_eq!(
        show["labels"],
        serde_json::json!(["urgent"]),
        "show_value: {show}"
    );
    let show_b = engine.show_value(&b.id).unwrap();
    assert!(
        show_b.get("labels").is_none(),
        "the unlabelled item's show_value omits the sparse key: {show_b}"
    );

    let find = |v: &serde_json::Value, id: &str| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .cloned()
            .unwrap()
    };

    let list = engine.list_value(None, None).unwrap();
    assert_eq!(
        find(&list, &a.id)["labels"],
        serde_json::json!(["urgent"]),
        "list_value labelled: {list}"
    );
    assert!(
        find(&list, &b.id).get("labels").is_none(),
        "list_value: the unlabelled record omits the sparse key: {list}"
    );

    let next = engine.next_value(NOW).unwrap();
    assert_eq!(
        find(&next, &a.id)["labels"],
        serde_json::json!(["urgent"]),
        "next_value labelled: {next}"
    );
    assert!(
        find(&next, &b.id).get("labels").is_none(),
        "next_value: the unlabelled record omits the sparse key: {next}"
    );

    // #5 (nice-to-have): the `with_label` filter reachable through the handle — only the labelled
    // item passes; the unlabelled sibling is filtered out.
    let items = engine.list(None, None).unwrap();
    let filtered = engine.with_label(items, "urgent").unwrap();
    assert_eq!(
        filtered.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        vec![a.id],
        "with_label returns only the labelled item"
    );
}

#[test]
fn handle_label_boundaries_zero_labels_idempotent_add_and_noop_remove() {
    // The SemVer-locked label surface's boundary behaviours (review Test Quality #1-3): a zero-label
    // read, an idempotent OR-set add, and an observed-remove of a never-added label.
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();

    // A never-labelled item reads back an EMPTY set — not an error, not a phantom label.
    assert!(
        engine.labels(&a.id).unwrap().is_empty(),
        "a fresh item has no labels"
    );

    // label_add is idempotent: adding the same label twice leaves exactly ONE (OR-set add).
    engine.label_add(NOW, ACTOR, &a.id, "urgent").unwrap();
    engine.label_add(NOW, ACTOR, &a.id, "urgent").unwrap();
    assert_eq!(
        engine.labels(&a.id).unwrap(),
        vec!["urgent".to_string()],
        "a duplicate add is idempotent"
    );

    // Removing a label that was never added is a documented no-op: Ok, and the present set is
    // untouched (observed-remove of an absent tag removes nothing).
    engine
        .label_remove(NOW, ACTOR, &a.id, "never-added")
        .unwrap();
    assert_eq!(
        engine.labels(&a.id).unwrap(),
        vec!["urgent".to_string()],
        "a no-op remove leaves the set unchanged"
    );
}

#[test]
fn labels_bulk_reads_a_whole_lane_in_one_pass_and_matches_singular() {
    // 1w5v: decorate a whole lane's labels in ONE query instead of N per-item reads (the app-bridge
    // `with_labels` seam). The bulk read must agree with the singular `labels` for every id it
    // carries, and — unlike singular `labels` — must NOT error on an id with no labels or one that
    // does not exist: a lane hands ids it already read, and a miss simply means "no labels".
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();
    let b = engine
        .create(NOW, ACTOR, "bug", "B", minimal("why", "P1"))
        .unwrap();
    let c = engine
        .create(NOW, ACTOR, "bug", "C", minimal("why", "P1"))
        .unwrap(); // stays label-less
    engine.label_add(NOW, ACTOR, &a.id, "urgent").unwrap();
    engine.label_add(NOW, ACTOR, &a.id, "backend").unwrap();
    engine.label_add(NOW, ACTOR, &b.id, "urgent").unwrap();

    let bulk = engine
        .labels_bulk(&[a.id.as_str(), b.id.as_str(), c.id.as_str(), "zz99.9999"])
        .unwrap();

    // Bulk == singular for each labelled id (the property lane decoration relies on), sorted.
    assert_eq!(bulk.get(&a.id), Some(&engine.labels(&a.id).unwrap()));
    assert_eq!(
        bulk.get(&a.id),
        Some(&vec!["backend".to_string(), "urgent".to_string()])
    );
    assert_eq!(bulk.get(&b.id), Some(&engine.labels(&b.id).unwrap()));
    // A label-less item is simply absent from the batch (empty set → no entry), NOT an error.
    assert_eq!(
        bulk.get(&c.id),
        None,
        "a label-less item is absent from the batch"
    );
    // A non-existent id: singular `labels` not_founds, but bulk tolerates it (absent, no error).
    assert!(
        engine.labels("zz99.9999").is_err(),
        "singular not_founds a missing id"
    );
    assert_eq!(
        bulk.get("zz99.9999"),
        None,
        "bulk omits a missing id without erroring"
    );

    // Empty lane → empty map, no query.
    assert!(engine.labels_bulk(&[]).unwrap().is_empty());
}

// ---- custom-carrying lane/bulk value reads (6j6v.1hh7) ----------------------
//
// v0.27.0 added the sparse plugin-`custom` join to the `blocked`/`deferred`/`closed`/`archived`
// lanes ONLY on the CLI/MCP `--json` surface (the `read::*_with_custom` free fns). The embedding
// Engine carried `custom` only on `show_value`/`list_value`/`next_value`; there was no
// `closed_value`/`deferred_value`/`archived_value`/`blocked_value` and no bulk custom read to join
// onto `prime`'s `next`. These tests pin the Engine seam to the SAME sparse-`custom` shape the CLI
// emits, so app-foundations (41j0.5rnj) can mirror the `--json` coverage without an N+1 `show_value`
// on the non-live archived lane. Bundled plugins declare no `[fields]`, so — exactly like the
// read-layer custom tests — this file registers its OWN fields-declaring plugin and selects it in
// the workspace, the only way a custom value can surface through the Engine's workspace-loaded cfg.

use nexus_flow_core::model::{EdgeKind, MergeStrategy};
use nexus_flow_facade::plugin::PluginRegistration;

inventory::submit!(PluginRegistration {
    name: "engine-fields-fixture",
    // A minimal valid plugin that DECLARES a global `uri` text field, so the Engine's read paths
    // have a declared custom field to surface. Its own test binary, so it does not pollute the
    // facade lib tests' "exactly two OSS plugins" assertion (mirrors external-fixture).
    toml: r#"
name = "engine-fields-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3", "P4"]
[types]
list = ["task"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [ { field = "id", dir = "asc" } ]
[presentation.list]
columns = ["id"]
[fields.uri]
type = "text"
label = "URI"
"#,
    order: 101,
});

/// A workspace on the fields-declaring fixture, seeded so every non-`next` lane has exactly one
/// member carrying a declared `custom.uri` value (and, for `blocked`, its open blocker without one):
/// `cf12.0001` closed, `cf12.0002` deferred (future defer), `cf12.0003` archived, `cf12.0004`
/// blocked by the open `cf12.0005`. Returns the temp dir (kept alive for the `.nxs` dir's lifetime).
fn seeded_fields_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "engine-fields-fixture").unwrap();
    let mut store = ws.open_store().unwrap();
    let uri = |s: &mut nexus_flow_core::store::Store, id: &str, v: &str| {
        s.set_custom_field_merge(id, "uri", Some(v.to_string()), "t", MergeStrategy::Lww);
    };

    // Closed lane: status=closed + a closed_at recency key, not archived.
    store.create_item("cf12.0001", "task", "closed one", "t");
    store.set_field("cf12.0001", "status", Some("closed".into()), "t");
    store.set_field(
        "cf12.0001",
        "closed_at",
        Some("2026-06-10T00:00:00Z".into()),
        "t",
    );
    uri(&mut store, "cf12.0001", "u-closed");

    // Deferred lane: open, unblocked, future defer_until (NOW is 2026-06-17).
    store.create_item("cf12.0002", "task", "deferred one", "t");
    store.set_field("cf12.0002", "defer_until", Some("2027-01-01".into()), "t");
    uri(&mut store, "cf12.0002", "u-deferred");

    // Archived lane: any status, `archived` recency key set.
    store.create_item("cf12.0003", "task", "archived one", "t");
    store.set_field(
        "cf12.0003",
        "archived",
        Some("2026-06-12T00:00:00Z".into()),
        "t",
    );
    uri(&mut store, "cf12.0003", "u-archived");

    // Blocked lane: cf12.0004 depends on the open cf12.0005 ⇒ 0004 is blocked, 0005 is its blocker.
    store.create_item("cf12.0004", "task", "blocked one", "t");
    store.create_item("cf12.0005", "task", "the blocker", "t");
    store.add_edge("cf12.0004", "cf12.0005", EdgeKind::Dep, "t");
    uri(&mut store, "cf12.0004", "u-blocked");
    tmp
}

#[test]
fn closed_value_carries_the_sparse_custom_map() {
    let tmp = seeded_fields_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine.closed_value().unwrap();
    let rows = v.as_array().expect("closed_value is a json array");
    assert_eq!(rows.len(), 1, "one closed, non-archived item: {v}");
    assert_eq!(rows[0]["id"], serde_json::json!("cf12.0001"));
    assert_eq!(rows[0]["status"], serde_json::json!("closed"));
    assert_eq!(
        rows[0]["custom"],
        serde_json::json!({ "uri": "u-closed" }),
        "closed_value carries the declared custom map like the CLI --json: {v}"
    );
}

#[test]
fn deferred_value_carries_the_sparse_custom_map() {
    let tmp = seeded_fields_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine.deferred_value(NOW).unwrap();
    let rows = v.as_array().expect("deferred_value is a json array");
    assert_eq!(rows.len(), 1, "one open item with a future defer date: {v}");
    assert_eq!(rows[0]["id"], serde_json::json!("cf12.0002"));
    assert_eq!(
        rows[0]["custom"],
        serde_json::json!({ "uri": "u-deferred" })
    );
}

#[test]
fn archived_value_carries_the_sparse_custom_map() {
    let tmp = seeded_fields_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine.archived_value().unwrap();
    let rows = v.as_array().expect("archived_value is a json array");
    assert_eq!(rows.len(), 1, "one archived item: {v}");
    assert_eq!(rows[0]["id"], serde_json::json!("cf12.0003"));
    assert_eq!(
        rows[0]["custom"],
        serde_json::json!({ "uri": "u-archived" })
    );
}

#[test]
fn blocked_value_carries_both_its_blockers_and_the_sparse_custom_map() {
    let tmp = seeded_fields_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine.blocked_value().unwrap();
    let rows = v.as_array().expect("blocked_value is a json array");
    assert_eq!(rows.len(), 1, "one blocked item: {v}");
    assert_eq!(rows[0]["id"], serde_json::json!("cf12.0004"));
    // The `blockers` join rides the record (the CLI `blocked --json` shape), naming the open blocker.
    assert_eq!(
        rows[0]["blockers"],
        serde_json::json!([{ "id": "cf12.0005", "status": "open" }]),
        "blocked_value names the open blocker: {v}"
    );
    // AND the sparse declared custom map, exactly like the other lanes.
    assert_eq!(rows[0]["custom"], serde_json::json!({ "uri": "u-blocked" }));
}

#[test]
fn custom_fields_bulk_returns_the_sparse_join_for_priming_the_next_list() {
    // The bulk sibling of the lane `_value` reads (the "join custom onto prime's next" option): a
    // single `id → {field: value}` read the consumer joins onto the `prime` next rows (whose
    // `PrimeReport` deliberately carries no custom) — no N+1, sparse (an id with no declared custom
    // value is simply absent), mirroring `declared_custom_by_id` behind the CLI's `prime --json`.
    let tmp = seeded_fields_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let bulk = engine
        .custom_fields_bulk(&["cf12.0001", "cf12.0004", "cf12.0005", "zz99.9999"])
        .unwrap();
    assert_eq!(
        bulk.get("cf12.0001"),
        Some(&serde_json::json!({ "uri": "u-closed" }))
    );
    assert_eq!(
        bulk.get("cf12.0004"),
        Some(&serde_json::json!({ "uri": "u-blocked" }))
    );
    // The bare blocker + a missing id have no declared custom value ⇒ absent (sparse), not an error.
    assert_eq!(
        bulk.get("cf12.0005"),
        None,
        "an id with no custom value is absent"
    );
    assert_eq!(
        bulk.get("zz99.9999"),
        None,
        "a missing id is tolerated, absent"
    );
}

#[test]
fn deferred_value_rejects_a_malformed_now() {
    // Mirrors `deferred`/`next`/`prime`: the defer-boundary read validates `now` at the seam (9t7.9)
    // rather than silently mis-deriving the deferred split.
    let tmp = seeded_fields_workspace();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        engine.deferred_value("garbage").unwrap_err().kind,
        ErrorKind::Validation,
    );
}

// ---- 6j6v.8dbe thread links through the handle (6j6v.72zn) ------------------

#[test]
fn handle_links_an_unknown_thread_and_existence_checks_only_the_item() {
    // The thread↔item edge driven ENTIRELY through the handle — the only door an embedding app has
    // (`Session::with_engine(|e: &Engine|)`); the `write::`/`read::` free functions want a
    // `&mut Store`/`&Store` the private handle never hands out, so they are the CLI/MCP surface,
    // not the embedding one.
    //
    // What this pins is the asymmetry the forwarders must not "improve" away: only the ITEM
    // endpoint is existence-checked. A thread lives in the CHAT store's id space, which flow
    // neither owns nor can read, so a thread no table anywhere knows is a perfectly good endpoint,
    // and a never-linked one simply has no edges instead of being rejected. Checking there would
    // make threads from a foreign workspace unlinkable and take the edge's whole point away.
    //
    // The link types are named from `nexus_flow_facade`, NOT `nexus_flow_core` — which this test
    // binary does not link (it is not a dev-dependency), exactly the position an embedding
    // consumer is in.
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let a = engine
        .create(NOW, ACTOR, "bug", "A", minimal("why", "P1"))
        .unwrap();

    // A thread id no thread table anywhere knows: flow holds the address, it does not adjudicate it.
    const THREAD: &str = "chat-thread-nobody-knows";
    engine
        .thread_link_add(
            NOW,
            ACTOR,
            THREAD,
            &a.id,
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
        )
        .unwrap();

    // Both directions of the edge read back through the handle, attributes intact.
    let edge = ThreadLink {
        thread_id: THREAD.to_string(),
        item_id: a.id.clone(),
        relation: LinkRelation::WorkedOn,
        weight: LinkWeight::Bearing,
    };
    assert_eq!(
        engine.thread_links(&a.id).unwrap(),
        vec![edge.clone()],
        "the item names the thread it is linked to"
    );
    assert_eq!(
        engine.thread_items(THREAD).unwrap(),
        vec![edge],
        "and the thread names the item, though flow knows nothing else about it"
    );

    // A thread nobody ever linked has NO edges — an empty vec, not a rejection.
    assert!(
        engine.thread_items("never-seen-thread").unwrap().is_empty(),
        "an unknown thread simply has no links"
    );

    // The ITEM endpoint, by contrast, IS checked — a typo is loud, not a silent phantom edge.
    assert_eq!(
        engine
            .thread_link_add(
                NOW,
                ACTOR,
                THREAD,
                "zz99.9999",
                LinkRelation::Cited,
                LinkWeight::Passing,
            )
            .unwrap_err()
            .kind,
        ErrorKind::NotFound,
        "only the item endpoint is existence-checked"
    );

    // And the edge comes off again through the handle, leaving both directions empty.
    engine
        .thread_link_remove(NOW, ACTOR, THREAD, &a.id)
        .unwrap();
    assert!(engine.thread_links(&a.id).unwrap().is_empty());
    assert!(engine.thread_items(THREAD).unwrap().is_empty());
}

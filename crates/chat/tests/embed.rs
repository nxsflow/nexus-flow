//! be9y — the embedding acceptance for the nexus-chat app-facade `Engine` handle (mirror of
//! `crates/memory/tests/embed.rs`).
//!
//! A long-lived in-process [`Engine`] — what a Tauri backend (via app-foundations) holds in managed
//! state — opens and OWNS the shared `.nxs/` store for the test's lifetime, reading and writing
//! WITHOUT spawning `nxc`. The reactive half: a genuinely external writer commits to the same
//! workspace and the write is delivered to the handle as a [`Change`] (the app's re-render cue), and
//! the SAME handle reads the new state live. The deeper proof that the handle's writes are
//! byte-identical to the CLI's lives in the `tests/parity.rs` differential.

mod common;

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::orchestration::Caller;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::transcript::TranscriptEntry;
use nexus_chat::watch::Change;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

const NOW: &str = "2026-07-10T10:00:00Z";
/// Short poll interval so the reactive loop closes within a sub-second assertion window — the
/// documented test seam, which is [`EngineConfig::poll_interval`] since nxf 6j6v.yr59 (it was a
/// third constructor of its own before that). Production uses the default cadence.
const FAST: Duration = Duration::from_millis(40);

/// Seed a fresh chat workspace at `dir` with a group channel `c-1` that `o/reader` + `o/a` belong
/// to, via a direct store (channel creation is out of the M1 facade write scope). Returns nothing —
/// the caller opens its own `Engine` over the same workspace.
fn seed_workspace(dir: &Path) {
    setup(dir, &chat_config()).expect("seed chat workspace");
    let ws = Workspace::resolve(None, dir).expect("resolve workspace");
    let mut s = ws.open_chat_store().expect("open store");
    s.set_channel_field("c-1", "kind", "group", "o/a");
    s.set_channel_field("c-1", "name", "general", "o/a");
    s.add_member("c-1", "o/reader", "o/a");
    s.add_member("c-1", "o/a", "o/a");
    // The declaration the handle's ONE remaining opening verb needs (nxf 6j6v.ckeq): `Engine::send`
    // — the raw post into a channel id — is gone, so a first message is `send_to <persona>` and a
    // persona has to be declared for there to be a target at all.
    std::fs::create_dir_all(dir.join(".nxs-personas")).expect("declaration folder");
    std::fs::write(
        dir.join(".nxs-personas/bob.yaml"),
        "handle: bob\nsystem_prompt: You are bob.\n",
    )
    .expect("declare bob");
}

/// The message `c-1`'s read assertions observe. Seeded through the STORE, like the channel and its
/// membership above and for the same reason: `c-1` is a plain substrate channel no declaration
/// names, and since nxf 6j6v.dvyq §3 nothing on the surface can address one — `send_to` refuses it
/// by name. What the handle still does with such a channel is READ it, which is what this fixture
/// is here to let the assertions check.
fn seed_message(dir: &Path) -> String {
    let ws = Workspace::resolve(None, dir).expect("resolve workspace");
    let mut s = ws.open_chat_store().expect("open store");
    s.set_wall_clock(NOW);
    s.post_message(&MessageEnvelope {
        origin: "o".into(),
        channel_id: "c-1".into(),
        sender: "o/a".into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: None,
        refs: Refs::default(),
        body: "hi".into(),
    })
}

/// A caller with no session, which is why it still names its `actor` — the case the session map
/// cannot answer (nxf 6j6v.07me). `origin` is not here and cannot be: the handle answers it
/// (`Engine::origin`), which is why the assertions below ask the handle rather than a literal.
fn caller<'a>(actor: &'a str) -> Caller<'a> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

#[test]
fn engine_is_clone_send_sync() {
    // Must be `Clone + Send + Sync` to live in a Tauri backend's managed state and be shared across
    // async command tasks. The foundation handle's `Mutex` over the non-`Sync` store buys `Sync`.
    fn assert_clone_send_sync<T: Clone + Send + Sync + 'static>() {}
    assert_clone_send_sync::<Engine>();
}

#[test]
fn write_then_read_back_through_the_handle() {
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    seed_message(dir.path());
    // A worker, because the handle's opening verb STARTS something now — `send_to` is not a post,
    // it is a summon (nxf 6j6v.ckeq, owner: "Es startet immer auch die jeweilige Sitzung bzw. weckt
    // sie wieder auf"). The dry one records the trigger and spawns nothing.
    let eng = Engine::open_with(
        None,
        dir.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            ..EngineConfig::default()
        },
    )
    .unwrap();

    // The handle's own write, and the same handle reads it back.
    let r = eng
        .send_to(
            caller("a"),
            SendToRequest {
                machine: None,
                to: "bob",
                body: "hi",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .unwrap();
    assert!(r.message_id.starts_with("m-"));
    let handle = format!("{}/a", eng.origin());
    // Read off the store: `Engine::messages` went with nxf 6j6v.yr59 and the seam's one message
    // reader is keyed by THREAD. What this line is checking is what the WRITE left in the channel,
    // which is why it reads the channel rather than the thread — see `common`'s own note.
    let msgs = common::channel_messages(dir.path(), &r.channel, &handle).unwrap();
    assert_eq!(
        msgs.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["hi"]
    );
    assert_eq!(msgs[0].sender, handle, "sender is origin/actor");
    assert_eq!(msgs[0].created.as_deref(), Some(NOW), "now stamped");

    // Bulk channel lane read: the reader's own channels, with the fields an app renders.
    //
    // It asserted the per-channel UNREAD COUNT here, and then that `mark_read` — the handle's other
    // write — advanced the cursor so the reader's inbox emptied. Both went with the unread
    // apparatus (nxf 6j6v.4d2z): the counts left `ChannelView`, and the ack was the last write on
    // this handle whose fact nothing on the handle could read back.
    let chans = common::channels_of(dir.path(), "o/reader").unwrap();
    assert_eq!(chans.len(), 1);
    assert_eq!(chans[0].channel_id, "c-1");
    assert_eq!(chans[0].members, 2);
}

#[test]
fn rejections_carry_their_kind() {
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    let eng = Engine::open(None, dir.path()).unwrap();

    // open a conversation with a target nothing declares → not_found
    let err = eng
        .send_to(
            caller("a"),
            SendToRequest {
                machine: None,
                to: "nobody-declared-this",
                body: "x",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);

    // read as a non-member → forbidden. Through the seam's own reader since nxf 6j6v.yr59, which
    // makes this a better test than it was: `Engine::thread` is the read an app actually calls, and
    // the refusal it gives is the one an app actually meets.
    seed_board(dir.path());
    assert_eq!(
        eng.thread("m-t1", "o/outsider").unwrap_err().kind,
        ErrorKind::Forbidden
    );
}

/// Open a review board `m-t1` in `c-1` (opener `o/a`, expects `o/b`) with the opener's request
/// message, via a direct store — the app-facade quorum reads then observe it.
fn seed_board(dir: &Path) {
    let ws = Workspace::resolve(None, dir).unwrap();
    let mut s = ws.open_chat_store().unwrap();
    s.open_thread(
        "m-t1",
        &ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/a".into(),
            created: NOW.into(),
            parent: None,
        },
        "o/a",
    );
    s.set_expects_reply_from("m-t1", "[\"o/b\"]", "o/a");
    // A previous session of the opener's role that has ENDED — the watermark a session start's
    // notice is a window over (nxf 6j6v.2hx9). Without one there is no window at all, which is the
    // right answer for a caller that has never run here and the wrong fixture for a test about what
    // the notice CONTAINS.
    s.create_pending_session("s-a", "a").expect("mint");
    s.mark_session_ended("s-a", "2026-07-08T00:00:00Z")
        .expect("end it");
    s.set_wall_clock(NOW);
    s.post_message(&MessageEnvelope {
        origin: "o".into(),
        channel_id: "c-1".into(),
        sender: "o/a".into(),
        kind: MessageKind::Question,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some("m-t1".into()),
        refs: Refs::default(),
        body: "please review".into(),
    });
}

#[test]
fn engine_exposes_the_quorum_surface_and_wakes_on_a_reply() {
    // T4 (be9y, 2nd stage): the app-facade handle exposes the M2 quorum surface — bulk board reads,
    // a single board, and the opener's complete/stale wake — with the SAME derivations as `nxc`, and
    // the reactive loop surfaces a completion the moment a reply folds.
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    seed_board(dir.path());
    let eng = Engine::open_with(
        None,
        dir.path(),
        EngineConfig {
            poll_interval: FAST,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    let rx = eng.subscribe().unwrap();

    // THE SURFACE THIS DRIVES CHANGED IN nxf 6j6v.yr59, and the questions did not. `threads`,
    // `thread_quorums`, `thread_board` and `opener_wake` folded into `status`, `thread` and
    // `prime_as`; what is asserted below is the same four answers off the three verbs that stayed.
    //
    // The bulk read for an EXPLICIT SET of thread ids — the quorum bar's no-N+1 read, which is a
    // scope now. `tests/bulk_quorum.rs` is where the "ONE query" half of it is measured.
    let report = eng
        .status(NOW, nexus_chat::facade::StatusScope::Threads(&["m-t1"]))
        .unwrap();
    assert_eq!(report.operations.len(), 1);
    let board = &report.operations[0].threads[0];
    assert_eq!(board.thread_id, "m-t1");
    assert_eq!(board.expects, vec!["o/b"]);
    assert_eq!(board.outstanding, vec!["o/b"]);
    assert_eq!(board.state, nexus_chat::facade::ThreadState::Open);
    // No working-tree lease was ever taken in this workspace — the ordinary case (nxf 6j6v.qk5b).
    assert_eq!(board.working_tree, None);
    assert_eq!(board.working_tree_queue_position, None);

    // The board's MESSAGES are the other reader — `status` carries state, never text, which is why
    // one message reader had to stay at all.
    let thread = eng.thread("m-t1", "o/a").unwrap();
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "please review");

    // Nothing is complete yet → the opener wake `prime_as` carries is empty.
    assert!(eng
        .prime_as("o/a", None, NOW)
        .unwrap()
        .wake
        .complete
        .is_empty());

    // An EXTERNAL writer posts the awaited reply → the board completes.
    let ws2 = Workspace::resolve(None, dir.path()).unwrap();
    let mut other = ws2.open_chat_store().unwrap();
    other.set_wall_clock(NOW);
    let reply_id = other.post_message(&MessageEnvelope {
        origin: "o".into(),
        channel_id: "c-1".into(),
        sender: "o/b".into(),
        kind: MessageKind::Report,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some("m-t1".into()),
        refs: Refs::default(),
        body: "lgtm".into(),
    });

    // The watcher delivers a Change (the app's cue to re-read the quorum view)…
    assert!(
        rx.recv_timeout(Duration::from_secs(5)).is_ok(),
        "the reply delivers a Change tick"
    );
    // …and the SAME handle now re-reads the board as complete, and the opener is woken.
    let report = eng
        .status(NOW, nexus_chat::facade::StatusScope::Threads(&["m-t1"]))
        .unwrap();
    assert!(report.operations[0].threads[0].outstanding.is_empty());
    assert!(report.operations[0].threads[0].awaiting_human);
    let wake = eng.prime_as("o/a", None, NOW).unwrap().wake;
    assert_eq!(
        wake.complete.len(),
        1,
        "the completed board wakes its opener"
    );
    assert_eq!(wake.complete[0].thread_id, "m-t1");
    assert_eq!(wake.complete[0].completing_message_id, reply_id);
}

#[test]
fn engine_status_reports_working_tree_holding_and_waiting() {
    // The Engine seam for nxf 6j6v.qk5b (`engine-seam-test-rule`): manufakt.io and nexflow.it read
    // the working-tree lease/queue's visibility through the HANDLE, never through `facade::` —
    // that coverage lives in `facade.rs`'s own unit tests, and does not stand in for this. Three
    // threads in one channel — one holds the lease, one waits behind it at queue position 1, one
    // has no relation to it at all — mirrors `verbs.rs`'s identical CLI-level scenario so the two
    // seams cannot disagree.
    //
    // It reads `Engine::status` since nxf 6j6v.yr59, where `threads`/`thread_board` folded into it
    // — which is the same edit as the item's condition for that fold, seen from the test side:
    // `StatusThread` carries the working-tree fields now, so this suite is also what proves the
    // fold left nothing behind.
    use nexus_chat::working_tree::{QueuedTrigger, WorkScope};

    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    let ws = Workspace::resolve(None, dir.path()).unwrap();
    let mut store = ws.open_chat_store().unwrap();
    for id in ["m-holding", "m-waiting", "m-neither"] {
        store.open_thread(
            id,
            &ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/a".into(),
                created: NOW.into(),
                parent: None,
            },
            "o/a",
        );
    }
    assert!(store
        .acquire_working_tree(
            &WorkScope::Thread("m-holding".to_string()),
            NOW,
            "2026-07-10T12:00:00Z",
        )
        .unwrap());
    store
        .enqueue_working_tree(
            &QueuedTrigger {
                id: 0,
                scope_key: WorkScope::Thread("m-waiting".to_string()).key(),
                role: "bob".to_string(),
                session: "s-1".to_string(),
                thread: Some("m-waiting".to_string()),
                message: "review it".to_string(),
                model: None,
                depth: 0,
                priority: Priority::Normal,
                enqueued_at: None,
            },
            NOW,
        )
        .unwrap();
    drop(store);

    let eng = Engine::open(None, dir.path()).unwrap();

    let report = eng
        .status(
            NOW,
            nexus_chat::facade::StatusScope::Threads(&["m-holding", "m-waiting", "m-neither"]),
        )
        .unwrap();
    assert_eq!(report.operations.len(), 3);
    let all: Vec<&nexus_chat::facade::StatusThread> = report
        .operations
        .iter()
        .flat_map(|op| op.threads.iter())
        .collect();
    let board = |id: &str| *all.iter().find(|b| b.thread_id == id).unwrap();
    assert_eq!(
        board("m-holding").working_tree,
        Some(nexus_chat::facade::WorkingTreeStatus::Holding)
    );
    assert_eq!(board("m-holding").working_tree_queue_position, None);
    assert_eq!(
        board("m-waiting").working_tree,
        Some(nexus_chat::facade::WorkingTreeStatus::Waiting)
    );
    assert_eq!(board("m-waiting").working_tree_queue_position, Some(1));
    assert_eq!(board("m-neither").working_tree, None);
    assert_eq!(board("m-neither").working_tree_queue_position, None);
    // `Option::None` alone cannot tell "additive, always renders as null" from "would be silently
    // dropped by a future `skip_serializing_if`" — both look identical at the struct level (PR
    // review of this ticket). Prove the WIRE contract too: the key must genuinely be present, not
    // merely read as `Value::Null` (which `serde_json::Value`'s `Index` also returns for a MISSING
    // key). `facade.rs`'s own unit test verifies this assertion actually goes red under a temporary
    // `skip_serializing_if`; this seam test only needs to repeat the same PRESENCE check, not
    // re-verify that it discriminates.
    // The WIRE contract on `StatusThread` runs the other way round from `ThreadListEntry`'s, and
    // deliberately: these two keys ARE `skip_serializing_if = "Option::is_none"` here, because
    // `status` is the read an app calls on every refresh and its report carries every thread of
    // every operation. So the assertion is that a thread WITH a relation to the tree carries them
    // and a thread without does not — a present-but-null key on every thread in a workspace that
    // has never taken a lease is noise on the one read that pays for it most.
    let holding_json = serde_json::to_value(board("m-holding")).unwrap();
    let holding_obj = holding_json.as_object().unwrap();
    assert_eq!(holding_obj.get("working_tree").unwrap(), "holding");
    let neither_json = serde_json::to_value(board("m-neither")).unwrap();
    let neither_obj = neither_json.as_object().unwrap();
    assert!(!neither_obj.contains_key("working_tree"));
    assert!(!neither_obj.contains_key("working_tree_queue_position"));
}

/// Independent review, Code Quality #1 / Integrity #3, High: `Engine::thread_board` used to
/// hardcode `Visibility::AllMembers` regardless of what a declared channel actually declared,
/// silently never enforcing `requester_only`. Mirrors `ensure_declared_channel`'s exact membership
/// shape (bare declared members alongside the qualified opener) — the same bare-vs-qualified split
/// `filter_board_messages`'s own doc explains.
///
/// **It drives `Engine::thread` since nxf 6j6v.yr59**, which is the whole point of that item's
/// visibility work: `thread_board` was the reader that carried the policy and it is gone from the
/// handle, so the regression this test names would have come STRAIGHT BACK — through the reader
/// that stayed — if the resolution had not moved with it.
#[test]
fn the_one_message_reader_enforces_a_declared_channels_requester_only_visibility() {
    let dir = TempDir::new().unwrap();
    setup(dir.path(), &chat_config()).expect("seed chat workspace");
    std::fs::create_dir_all(dir.path().join(".nxs-personas")).unwrap();
    std::fs::write(
        dir.path().join(".nxs-personas/channels.yaml"),
        "- name: review\n  members: [bob, carol]\n  visibility: requester_only\n",
    )
    .unwrap();

    let ws = Workspace::resolve(None, dir.path()).unwrap();
    let mut s = ws.open_chat_store().unwrap();
    s.set_channel_field("decl:review", "kind", "group", "o/opener");
    s.set_channel_field("decl:review", "name", "review", "o/opener");
    s.add_member("decl:review", "o/opener", "o/opener");
    s.add_member("decl:review", "bob", "o/opener"); // bare, as ensure_declared_channel does
    s.add_member("decl:review", "carol", "o/opener");
    s.set_wall_clock(NOW);
    for (sender, body) in [
        ("o/opener", "please review"),
        ("o/bob", "bob's finding"),
        ("o/carol", "carol's finding"),
    ] {
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "decl:review".into(),
            sender: sender.into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("m-t2".into()),
            refs: Refs::default(),
            body: body.into(),
        });
    }
    drop(s);

    let eng = Engine::open(None, dir.path()).unwrap();

    // bob (bare, the shape a declared member's own membership is recorded in): opening + his own
    // reply — not carol's.
    let board = eng.thread("m-t2", "bob").unwrap();
    assert_eq!(
        board
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        vec!["please review", "bob's finding"],
        "bob must not see carol's reply under requester_only"
    );

    // the opener: sees everything.
    let board = eng.thread("m-t2", "o/opener").unwrap();
    assert_eq!(
        board
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        vec!["please review", "bob's finding", "carol's finding"]
    );
}

#[test]
fn subscribe_fires_on_a_foreign_commit() {
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    let eng = Engine::open_with(
        None,
        dir.path(),
        EngineConfig {
            poll_interval: FAST,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    let rx = eng.subscribe().unwrap();

    // A SEPARATE connection (not the Engine) commits a message to the same workspace.
    let ws2 = Workspace::resolve(None, dir.path()).unwrap();
    let mut other = ws2.open_chat_store().unwrap();
    other.set_wall_clock(NOW);
    other.post_message(&MessageEnvelope {
        origin: "o".into(),
        channel_id: "c-1".into(),
        sender: "o/a".into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: None,
        refs: Refs::default(),
        body: "external".into(),
    });

    // The watcher delivers a Change (the app's re-render cue)…
    let got: Result<Change, _> = rx.recv_timeout(Duration::from_secs(5));
    assert!(got.is_ok(), "a foreign commit delivers a Change tick");

    // …and the SAME handle (never reopened) reads the new state live. Read off the channel: the
    // unread inbox this used to ask went with nxf 6j6v.4d2z, and what is being shown is that the
    // foreign commit is VISIBLE, which any live read says.
    let msgs = common::channel_messages(dir.path(), "c-1", "o/reader").unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].body, "external");
}

#[test]
fn a_host_running_its_own_role_runtime_fills_the_transcript_through_the_handle() {
    // nxf 6j6v.c6e8 — the write half of the transcript, at the seam. It was store-direct from
    // `cli.rs`, so the only ways to record what a session did were to spawn `nxc transcript append`
    // or to reach past the facade into the store. THE HOST THIS IS FOR is app-foundations'
    // `crates/agent-runtime`, which drives the SDK itself: `41j0.9brv` found a bound session whose
    // transcript stayed empty (`real=… (0 entries)`) for exactly this reason. Here the same handle
    // writes and reads, and no process is spawned.
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    let eng = Engine::open(None, dir.path()).unwrap();

    let e =
        |kind: &str, tool_use_id: Option<&str>, parent: Option<&str>, text: &str| TranscriptEntry {
            kind: kind.into(),
            at: Some("2026-07-26T07:42:39.123Z".into()),
            tool_use_id: tool_use_id.map(str::to_string),
            parent_tool_use_id: parent.map(str::to_string),
            subagent_type: parent.map(|_| "code-reviewer".to_string()),
            data: serde_json::json!({ "text": text }),
        };
    let appended = eng
        .transcript_append(
            "s-1",
            &[
                e("tool_use", Some("toolu_task_1"), None, "Task"),
                e("assistant", None, Some("toolu_task_1"), "reading the diff"),
            ],
        )
        .unwrap();
    assert_eq!(appended, 2);
    // A second flush, as a host that batches makes it: one history, not two.
    assert_eq!(
        eng.transcript_append(
            "s-1",
            &[e("assistant", None, None, "the reviewer found nothing")]
        )
        .unwrap(),
        1
    );

    let v = eng.transcript_page("s-1", -1, None).unwrap();
    assert_eq!(
        v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [0, 2],
        "`seq` is continuous across the two flushes — the store assigns it, not the caller"
    );
    assert_eq!(v.entries[0].subagent.len(), 1);
    assert_eq!(v.entries[0].subagent[0].data["text"], "reading the diff");
    assert_eq!(v.entries[1].data["text"], "the reviewer found nothing");

    // What the CLI writes and what the handle writes land in the SAME transcript: a host that
    // spawns the sidecar for one turn and drives the SDK itself for the next gets one history.
    {
        let ws = Workspace::resolve(None, dir.path()).unwrap();
        let mut s = ws.open_chat_store().unwrap();
        s.append_transcript("s-1", &[e("result", None, None, "and out")])
            .unwrap();
    }
    assert_eq!(
        eng.transcript_page("s-1", -1, None)
            .unwrap()
            .entries
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<_>>(),
        [0, 2, 3]
    );

    // The store's one invariant reaches the handle's caller unchanged, naming the batch index.
    let err = eng
        .transcript_append("s-1", &[e("", None, None, "no kind")])
        .expect_err("an empty `kind` is refused at the seam too");
    assert!(err.to_string().contains("transcript entry 0"), "{err}");
}

#[test]
fn transcript_reads_through_the_handle_with_its_subagent_sub_timeline_nested() {
    // The seam surface app-foundations mirrors (nxf epic 6wt2, ticket 5bym): a role session's agent
    // transcript, read through the long-lived handle, with the Task-spawned subagent's entries
    // nested under the `tool_use` that spawned them and the session-map facts resolved. Written by
    // a SEPARATE store (the sidecar is another process, exactly like this) — the handle only reads.
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());
    {
        let ws = Workspace::resolve(None, dir.path()).unwrap();
        let mut s = ws.open_chat_store().unwrap();
        s.create_pending_session("s-1", "coder").unwrap();
        s.bind_session("s-1", "real-abc").unwrap();
        let e = |kind: &str, tool_use_id: Option<&str>, parent: Option<&str>, text: &str| {
            TranscriptEntry {
                kind: kind.into(),
                at: Some("2026-07-26T07:42:39.123Z".into()),
                tool_use_id: tool_use_id.map(str::to_string),
                parent_tool_use_id: parent.map(str::to_string),
                subagent_type: parent.map(|_| "code-reviewer".to_string()),
                data: serde_json::json!({ "text": text }),
            }
        };
        s.append_transcript(
            "s-1",
            &[
                e("tool_use", Some("toolu_task_1"), None, "Task"),
                e("assistant", None, Some("toolu_task_1"), "reading the diff"),
                e("assistant", None, None, "the reviewer found nothing"),
            ],
        )
        .unwrap();
    }

    let eng = Engine::open(None, dir.path()).unwrap();
    // `transcript_page(s, -1, None)` IS the whole session — which is why the unwindowed
    // `Engine::transcript` went in nxf 6j6v.yr59 rather than being kept beside it.
    let v = eng.transcript_page("s-1", -1, None).unwrap();
    assert_eq!(v.session, "s-1");
    assert_eq!(v.role.as_deref(), Some("coder"));
    assert_eq!(v.real_sdk_id.as_deref(), Some("real-abc"));
    assert_eq!(v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [0, 2]);
    assert_eq!(v.entries[0].subagent.len(), 1);
    assert_eq!(v.entries[0].subagent[0].data["text"], "reading the diff");
    // An unknown session reads as an empty transcript through the handle too, never an error.
    assert!(eng
        .transcript_page("never-existed", -1, None)
        .unwrap()
        .entries
        .is_empty());
}

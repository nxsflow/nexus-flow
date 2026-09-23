//! T1 acceptance: `message` ops coexist with `task` ops in one op-log (each reducer folds only its
//! domain; the other is store-don't-fold), and a message op survives an export→apply round-trip
//! and folds on a peer (spec §7).

use nexus_chat::model::*;
use nexus_chat::store::ChatStore;

fn env(channel: &str, body: &str) -> MessageEnvelope {
    MessageEnvelope {
        origin: "o".into(),
        channel_id: channel.into(),
        sender: "o/a".into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: None,
        refs: Refs::default(),
        body: body.into(),
    }
}

#[test]
fn a_message_op_survives_a_round_trip_and_folds_on_a_peer() {
    let mut a = ChatStore::open_in_memory(1);
    let mid = a.post_message(&env("c-1", "hello peer"));
    // Ship the whole log to a fresh peer (the same op path sync uses).
    let ops = a.export();
    let mut b = ChatStore::open_in_memory(2);
    let deferred = b.apply(&ops);
    assert!(
        deferred.is_empty(),
        "the peer folds the message op (domain understood)"
    );
    let body: String = b
        .connection()
        .query_row(
            "SELECT body FROM messages WHERE message_id=?1",
            [&mid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "hello peer");
}

/// A thread root in channel `c-1` opened by `o/a`.
fn root() -> ThreadRoot {
    ThreadRoot {
        origin: "o".into(),
        channel_id: "c-1".into(),
        opener: "o/a".into(),
        created: String::new(),
        parent: None,
    }
}

/// A message from `sender` into thread `tid` (channel `c-1`).
fn reply(tid: &str, sender: &str, body: &str) -> MessageEnvelope {
    MessageEnvelope {
        origin: "o".into(),
        channel_id: "c-1".into(),
        sender: sender.into(),
        kind: MessageKind::Report,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some(tid.into()),
        refs: Refs::default(),
        body: body.into(),
    }
}

#[test]
fn a_deadline_op_folds_on_a_peer_and_the_quorum_re_derives_without_cross_contamination() {
    // M2 wire-compat (spec §7): the new `thread set deadline` op is a chat-domain op. It rides the
    // op path to a fresh peer, folds into `threads.deadline`, and the peer RE-DERIVES the quorum
    // (`stale` past the deadline) — no completion op ever crosses the wire (§6). A foreign `task` op
    // in the same log stays store-don't-fold: no cross-contamination either way.
    let mut a = ChatStore::open_in_memory(1);
    let tid = a.mint_thread_id();
    a.open_thread(&tid, &root(), "o/a");
    a.set_expects_reply_from(&tid, "[\"o/b\"]", "o/a");
    a.set_deadline(&tid, "2026-07-20T00:00:00Z", "o/a");

    // Ship the whole log to a fresh peer (the same op path sync uses).
    let ops = a.export();
    let mut b = ChatStore::open_in_memory(2);
    assert!(
        b.apply(&ops).is_empty(),
        "the peer folds the thread open + expects + deadline ops"
    );
    // The peer is another replica of the same person, so it trusts a's key (nxf 6j6v.pzkb):
    // without that the thread is HELD there — shown, never stale, since an op nobody on the peer
    // can vouch for set its deadline (`store.rs`'s held-thread tests pin that side).
    b.trust_key(a.key_id(), "a", "").unwrap();

    // The deadline register folded, and the peer derives `stale` from its OWN clock read.
    let q = b
        .thread_quorum(&tid, "2026-07-25T00:00:00Z")
        .unwrap()
        .unwrap();
    assert_eq!(q.deadline.as_deref(), Some("2026-07-20T00:00:00Z"));
    assert!(!q.complete, "o/b still outstanding");
    assert!(
        q.stale,
        "now > deadline ∧ outstanding≠∅ → stale, re-derived on the peer"
    );
    // Before the deadline the same peer reads it as NOT stale (the clock read is local, §4.3).
    assert!(
        !b.thread_quorum(&tid, "2026-07-10T00:00:00Z")
            .unwrap()
            .unwrap()
            .stale
    );

    // A flow `task` op in the same log is inert to chat's reducer (store-don't-fold).
    let task_op = nexus_flow_core::model::Op {
        op_id: "flow-x".into(),
        lamport: 99,
        site: 9,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "t.A".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some("A".into()),
        author: "u".into(),
        wall_clock: String::new(),
        key_id: None,
        sig: None,
    };
    assert_eq!(
        b.apply(&[task_op]).len(),
        1,
        "a task op has no chat reducer → deferred, no cross-contamination"
    );
}

#[test]
fn a_thread_without_a_deadline_op_is_byte_identical_under_m2() {
    // §7: a pre-M2 thread (no `set deadline` op — an M1 workspace never authored one) reads exactly
    // as before under M2 — `deadline` NULL, `stale` always false, and `complete` UNCHANGED by the
    // sparse-key minor.
    let mut s = ChatStore::open_in_memory(1);
    let tid = s.mint_thread_id();
    s.open_thread(&tid, &root(), "o/a");
    s.set_expects_reply_from(&tid, "[\"o/b\",\"o/c\"]", "o/a");
    s.post_message(&reply(&tid, "o/b", "lgtm"));
    s.post_message(&reply(&tid, "o/c", "approved"));

    // Even at a far-future clock, a deadline-less thread is never stale; completion is unaffected.
    let q = s
        .thread_quorum(&tid, "2099-01-01T00:00:00Z")
        .unwrap()
        .unwrap();
    assert_eq!(q.deadline, None, "no deadline op → NULL");
    assert!(!q.stale, "no deadline → never stale");
    assert!(
        q.complete,
        "all expected replied → complete, unchanged by the deadline minor"
    );
}

#[test]
fn a_foreign_task_op_is_stored_not_folded_no_cross_contamination() {
    // Build a flow task op via nexus-flow-core, apply it to a ChatStore: it must be DEFERRED
    // (store-don't-fold), never touching chat's views (spec §7).
    //
    // `nexus_flow_core::model::Op` is a literal re-export of `nxs_foundation::model::Op`
    // (see `crates/core/src/model.rs`: "pub use nxs_foundation::model::Op;" — the op shape is
    // domain-agnostic and owned by the foundation). So there is no separate "flow Op" to convert
    // from: the value constructed here already IS the `Op` that `ChatStore::apply` consumes.
    let mut chat = ChatStore::open_in_memory(1);
    let task_op = nexus_flow_core::model::Op {
        op_id: "flow-1".into(),
        lamport: 1,
        site: 9,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "t.A".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some("A".into()),
        author: "u".into(),
        wall_clock: String::new(),
        key_id: None,
        sig: None,
    };
    let deferred = chat.apply(&[task_op]);
    assert_eq!(
        deferred.len(),
        1,
        "a task op has no chat reducer → stored-not-folded"
    );
    let n: i64 = chat
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "no cross-contamination into chat views");
}

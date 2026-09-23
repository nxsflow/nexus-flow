//! T4 acceptance (`6j6v.j25s`) — the nexus-chat M1 gate (spec §6/§7):
//!
//! 1. **Multi-module coexistence**: `task`, `fact` and `message` ops share ONE `.nxs` db; each
//!    reducer folds only its own domain, the others are store-don't-fold (no cross-contamination).
//! 2. **Sync round-trip through the REAL engine**: a `message` op authored on machine A crosses a
//!    relay via the client anti-entropy engine `nxs sync` actually runs (`nxs_sync::engine::sync`,
//!    driven over flow's `Store` as the op-log carrier) and folds into machine B's chat views — the
//!    `domain` (and the folded `origin`) travel with it. This upgrades T1's `export()/apply()`
//!    shortcut (`coexistence.rs`) to the production sync path.
//! 3. **The M1 behavioral invariant**: flow AND memory stay byte-identical — their materialized
//!    views are unchanged whether or not `message`/`fact` ops share the log (the differential oracle
//!    and the flow/memory goldens stay green because `message` ops never touch those reducers).
//! 4. **Global ids (forward-compat, note 2026-07-09 / decision `6j6v.kz8p`)**: the M1 substrate
//!    mints GLOBAL `m-`+ULID message/thread ids and folds `origin` — so the later cross-workspace
//!    bridge needs no data migration. The round-trip proves the id survives the relay unchanged.

use std::cell::RefCell;
use std::collections::HashMap;

use nexus_chat::model::*;
use nexus_chat::store::ChatStore;
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nexus_memory::store::{MemoryQuery, MemoryStore};
use nxs_sync::engine::{sync, Transport, TransportError, Unbounded, Watermarks};
use nxs_sync::protocol::{Cursor, RegisterOutcome, StreamId};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};
use tempfile::TempDir;

/// An in-process stand-in for the relay: an append-only ordered log per stream with the relay's
/// cursor semantics (`next` = position). Faithful to the engine CONTRACT the HTTP relay implements,
/// so the round-trip exercises the real `engine::sync` algorithm without spinning a server. (The
/// genuine server-backed round-trip is verified out-of-band with the `nxf-relay` binary + the `nxc`
/// / `nxs sync` CLIs — this test locks the behavior into CI.)
#[derive(Default)]
struct RelayLog {
    streams: RefCell<HashMap<String, Vec<WireOp>>>,
}

impl RelayLog {
    fn new() -> RelayLog {
        RelayLog::default()
    }
}

impl Transport for RelayLog {
    fn push(&self, stream: &StreamId, ops: &[WireOp]) -> Result<(), TransportError> {
        self.streams
            .borrow_mut()
            .entry(stream.0.clone())
            .or_default()
            .extend_from_slice(ops);
        Ok(())
    }

    fn pull(
        &self,
        stream: &StreamId,
        since: Cursor,
        limit: usize,
    ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
        let map = self.streams.borrow();
        let log = map.get(&stream.0).cloned().unwrap_or_default();
        let start = since.0 as usize;
        let page: Vec<WireOp> = log.iter().skip(start).take(limit).cloned().collect();
        let next = Cursor((start + page.len()) as i64);
        Ok((page, next))
    }

    // Registration is orthogonal to the domain round-trip; accept every prefix unchanged.
    fn register(
        &self,
        _stream: &StreamId,
        _prefix: &str,
        _replica_uuid: &str,
    ) -> Result<RegisterOutcome, TransportError> {
        Ok(RegisterOutcome::Registered)
    }
}

fn path_str(dir: &TempDir, name: &str) -> String {
    dir.path().join(name).to_str().unwrap().to_string()
}

fn info_message(origin: &str, channel: &str, body: &str) -> MessageEnvelope {
    MessageEnvelope {
        origin: origin.into(),
        channel_id: channel.into(),
        sender: format!("{origin}/alice"),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::NextSession,
        thread_id: None,
        refs: Refs::default(),
        body: body.into(),
    }
}

/// One `SELECT domain FROM ops` bag, so a test can prove all three domains coexist in one log.
fn op_domains(conn: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT domain FROM ops ORDER BY domain")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

#[test]
fn three_modules_coexist_in_one_db_each_folds_only_its_domain() {
    // flow (`task`), memory (`fact`) and chat (`message`) write into ONE shared `.nxs` db, each
    // through its own store/connection — the real multi-module layout (spec §7). Writes are
    // sequential (open→write→drop), mirroring separate processes sharing the workspace file.
    let dir = TempDir::new().unwrap();
    let db = path_str(&dir, "shared.sqlite");

    {
        let mut flow = Store::open(&db, 1).unwrap();
        flow.create_item("aaaa.0001", "task", "the task", "alice");
    }
    {
        let mut memory = MemoryStore::open(&db, 1).unwrap();
        memory.remember("k-1", "the fact", "alice");
    }
    let mid = {
        let mut chat = ChatStore::open(&db, 1).unwrap();
        chat.post_message(&info_message("workspace-A", "m-chan", "the message"))
    };

    // All three domains coexist in the SINGLE op-log.
    {
        let probe = Store::open(&db, 1).unwrap();
        assert_eq!(
            op_domains(probe.connection()),
            vec!["fact", "message", "task"],
            "one shared op-log carries all three domains"
        );
    }

    // flow folds ONLY `task` — the fact/message ops are store-don't-fold, invisible to its views.
    {
        let flow = Store::open(&db, 1).unwrap();
        let items = flow.list_items().unwrap();
        assert_eq!(items.len(), 1, "flow sees exactly its own item");
        assert_eq!(items[0].id, "aaaa.0001");
    }

    // memory folds ONLY `fact`.
    {
        let memory = MemoryStore::open(&db, 1).unwrap();
        let facts = memory.memories(&MemoryQuery::default()).unwrap();
        assert_eq!(facts.len(), 1, "memory sees exactly its own fact");
        assert_eq!(facts[0].key, "k-1");
    }

    // chat folds ONLY `message` — and no task/fact leaked into its views (no cross-contamination).
    {
        let chat = ChatStore::open(&db, 1).unwrap();
        let (n, body): (i64, String) = chat
            .connection()
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(body),'') FROM messages",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(n, 1, "chat sees exactly its own message");
        assert_eq!(body, "the message");
        // The minted id is the global forward-compat form.
        assert!(mid.starts_with("m-"), "message id carries the kind prefix");
    }
}

#[test]
fn a_message_survives_the_real_sync_engine_and_folds_on_a_peer() {
    // The M1 sync-acceptance gate: a chat message, authored on machine A, rides the REAL client
    // anti-entropy engine over a relay and folds into machine B's chat views — id + origin intact.
    let dir = TempDir::new().unwrap();
    let db_a = path_str(&dir, "a.sqlite");
    let db_b = path_str(&dir, "b.sqlite");

    // Machine A (site 1): open a thread and post a message in it, so BOTH the global message id and
    // the global thread id are put through the relay (the forward-compat gate, note 2026-07-09).
    let (mid, tid) = {
        let mut chat = ChatStore::open(&db_a, 1).unwrap();
        let tid = chat.mint_thread_id();
        chat.open_thread(
            &tid,
            &ThreadRoot {
                origin: "workspace-A".into(),
                channel_id: "m-chan".into(),
                opener: "workspace-A/alice".into(),
                created: String::new(),
                parent: None,
            },
            "workspace-A/alice",
        );
        let mut env = info_message("workspace-A", "m-chan", "survives the relay");
        env.thread_id = Some(tid.clone());
        let mid = chat.post_message(&env);
        (mid, tid)
    };
    assert!(mid.starts_with("m-") && tid.starts_with("m-"), "global ids");

    let relay = RelayLog::new();
    let stream = StreamId("stream-t4".into());

    // A PUSHES every local op (incl. the message + thread) — driven over flow's Store exactly as
    // `nxs sync` drives it. The message rides the wire with its `domain` (spec §7).
    {
        let mut flow_a = Store::open(&db_a, 1).unwrap();
        let mut marks = Watermarks::default();
        let out = sync(&mut flow_a, 1, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
        assert!(out.pushed >= 2, "A pushed the message + thread ops");
    }

    // Machine B (site 2, FRESH db): PULL over the engine. flow folds no `message` op, so the ops
    // land in B's shared log store-don't-fold — present but not yet in any chat view.
    {
        let mut flow_b = Store::open(&db_b, 2).unwrap();
        let mut marks = Watermarks::default();
        let out = sync(&mut flow_b, 2, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
        assert!(out.pulled >= 2, "B pulled A's ops");
        assert!(
            flow_b.list_items().unwrap().is_empty(),
            "flow folds none of the chat ops it carried"
        );
    }

    // Machine B: open chat over the SAME db → `refold_if_behind(\"chat\")` folds the pulled message.
    let chat_b = ChatStore::open(&db_b, 2).unwrap();
    let (body, origin, got_mid, got_tid): (String, String, String, Option<String>) = chat_b
        .connection()
        .query_row(
            "SELECT body, origin, message_id, thread_id FROM messages",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(body, "survives the relay", "the message folded on the peer");
    assert_eq!(
        origin, "workspace-A",
        "origin traveled with the op — attribution needs no later migration"
    );
    assert_eq!(
        got_mid, mid,
        "the global m-ULID message id is byte-identical on the peer — no re-keying"
    );
    assert_eq!(got_tid.as_deref(), Some(tid.as_str()));

    // The thread root folded too, carrying its own global id + origin.
    let (thread_origin, thread_n): (String, i64) = chat_b
        .connection()
        .query_row(
            "SELECT COALESCE(MAX(origin),''), COUNT(*) FROM threads WHERE thread_id=?1",
            [&tid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(thread_n, 1, "the thread root folded on the peer");
    assert_eq!(thread_origin, "workspace-A", "thread origin traveled");
}

#[test]
fn the_second_turns_obligation_is_derivable_on_a_peer_from_the_synced_ops_alone() {
    // 6j6v.cg8g's fourth acceptance point, proved rather than asserted: the per-TURN discharge is
    // computed in the FOLDED view `threads`, out of material the op log already carries (the
    // `expects_reply_from` register's own LWW position against each message's `(lamport, site)`), so
    // the synced payload needs no new field and no device needs a migration. A peer that has seen
    // NOTHING but the ops — a fresh db, no views of its own — must therefore reach the same answer
    // about the second turn as the machine that wrote them.
    let dir = TempDir::new().unwrap();
    let db_a = path_str(&dir, "turn_a.sqlite");
    let db_b = path_str(&dir, "turn_b.sqlite");

    let tid = {
        let mut chat = ChatStore::open(&db_a, 1).unwrap();
        let tid = chat.mint_thread_id();
        chat.open_thread(
            &tid,
            &ThreadRoot {
                origin: "workspace-A".into(),
                channel_id: "m-chan".into(),
                opener: "workspace-A/alice".into(),
                created: String::new(),
                parent: None,
            },
            "workspace-A/alice",
        );
        // Turn 1: asked, and answered.
        chat.set_expects_reply_from(&tid, "[\"workspace-A/bob\"]", "workspace-A/alice");
        let mut env = info_message("workspace-A", "m-chan", "turn 1: done");
        env.sender = "workspace-A/bob".into();
        env.thread_id = Some(tid.clone());
        // The payload that crosses the wire is the M1/M2 envelope, unchanged: nine keys, none of
        // them a watermark. A field added here would be a synced-format change for every device.
        // (Key order is `serde_json`'s own — a sorted map, not the declaration order.)
        assert_eq!(
            serde_json::to_value(&env)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<String>>(),
            [
                "body",
                "channel_id",
                "disposition",
                "kind",
                "origin",
                "priority",
                "refs",
                "sender",
                "thread_id"
            ]
        );
        chat.post_message(&env);
        // Turn 2: asked again, unanswered.
        chat.set_expects_reply_from(&tid, "[\"workspace-A/bob\"]", "workspace-A/alice");
        let q = chat.thread_quorum(&tid, "2026-07-11T00:00:00Z").unwrap();
        assert_eq!(
            q.unwrap().outstanding,
            ["workspace-A/bob"],
            "on the writer, the second turn is owed"
        );
        tid
    };

    let relay = RelayLog::new();
    let stream = StreamId("stream-cg8g".into());
    {
        let mut flow_a = Store::open(&db_a, 1).unwrap();
        let mut marks = Watermarks::default();
        sync(&mut flow_a, 1, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
    }
    {
        let mut flow_b = Store::open(&db_b, 2).unwrap();
        let mut marks = Watermarks::default();
        let out = sync(&mut flow_b, 2, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
        assert!(
            out.pulled >= 4,
            "B pulled the thread + register + message ops"
        );
    }

    // The peer folds them on open and derives the SAME turn state — no envelope field, no migration.
    let chat_b = ChatStore::open(&db_b, 2).unwrap();
    let q = chat_b
        .thread_quorum(&tid, "2026-07-11T00:00:00Z")
        .unwrap()
        .expect("the thread folded on the peer");
    assert_eq!(
        q.outstanding,
        ["workspace-A/bob"],
        "the peer owes the same second turn: {q:?}"
    );
    assert!(q.replied.is_empty(), "turn 1's answer is turn 1's: {q:?}");
    assert!(!q.complete, "{q:?}");
}

/// A canonical, deterministic dump of ALL of flow's materialized view tables — items PLUS the
/// observable edge relations and notes. The internal provenance columns that are non-deterministic
/// mints (`edge_adds.tag` = an op_id, `notes.id` = a minted note id) are projected AWAY, so the
/// dump captures the derived state a consumer observes, not incidental ids. This is deliberately
/// broader than `list_items()` alone: the invariant is that NO flow view table is perturbed by
/// `message` ops, not just `items`.
fn dump_flow(db: &str) -> String {
    let flow = Store::open(db, 1).unwrap();
    let items = format!("{:?}", flow.list_items().unwrap());
    let conn = flow.connection();
    let mut edges: Vec<String> = conn
        .prepare("SELECT from_id, kind, to_id FROM edge_adds WHERE tag NOT IN (SELECT tag FROM edge_removes)")
        .unwrap()
        .query_map([], |r| {
            Ok(format!(
                "{} -{}-> {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    edges.sort();
    let mut notes: Vec<String> = conn
        .prepare("SELECT item_id, author, body FROM notes WHERE id NOT IN (SELECT note_id FROM note_tombstones)")
        .unwrap()
        .query_map([], |r| {
            Ok(format!(
                "{}|{}|{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    notes.sort();
    format!("items={items}\nedges={edges:?}\nnotes={notes:?}")
}

/// Memory's derived view: the active fact set. Deterministic (explicit keys, empty wall_clock); a
/// forgotten key drops out, so building with a `forget` exercises the tombstone path observably.
fn dump_memory(db: &str) -> String {
    format!(
        "{:?}",
        MemoryStore::open(db, 1)
            .unwrap()
            .memories(&MemoryQuery::default())
            .unwrap()
    )
}

#[test]
fn flow_and_memory_stay_byte_identical_with_chat_ops_in_the_log() {
    // The M1 behavioral invariant (spec §7): sharing the log with `message` ops leaves flow's — and
    // memory's — materialized views byte-identical to a log without them, because a `message` op
    // never reaches those reducers. The AUTHORITATIVE gate is the differential oracle + the
    // flow/memory trycmd goldens (run in CI, unchanged by this milestone); this test is the
    // chat-side end-to-end demonstration over a broad, multi-table derived state — not a one-row
    // smoke-check.
    let dir = TempDir::new().unwrap();

    // A non-trivial flow graph (three items, varied fields incl. a closed one, a dependency edge,
    // and a note) + a non-trivial memory set (three facts, one later forgotten). Identical writes
    // land in a clean single-domain log and in a log ALSO carrying heavy chat noise.
    let build_flow = |db: &str| {
        let mut flow = Store::open(db, 1).unwrap();
        flow.create_item("aaaa.0001", "task", "first task", "alice");
        flow.create_item("aaaa.0002", "task", "second task", "bob");
        flow.create_item("aaaa.0003", "task", "third task", "alice");
        flow.set_field("aaaa.0001", "priority", Some("1".into()), "alice");
        flow.set_field("aaaa.0002", "description", Some("the why".into()), "bob");
        flow.set_field("aaaa.0003", "status", Some("closed".into()), "alice");
        flow.add_edge("aaaa.0001", "aaaa.0002", EdgeKind::Dep, "alice"); // 1 depends on 2
        flow.add_note("aaaa.0001", "a durable note", "alice");
    };
    let build_memory = |db: &str| {
        let mut memory = MemoryStore::open(db, 1).unwrap();
        memory.remember("k-1", "first fact", "alice");
        memory.remember("k-2", "second fact", "bob");
        memory.remember("k-3", "third fact", "alice");
        memory.forget("k-2", "alice"); // exercise the tombstone / active=0 path
    };

    // (a) Clean single-domain logs.
    let flow_only = path_str(&dir, "flow_only.sqlite");
    let mem_only = path_str(&dir, "mem_only.sqlite");
    build_flow(&flow_only);
    build_memory(&mem_only);

    // (b) One mixed log: the same flow + memory writes, PLUS heavy chat noise (two messages, a
    // membership add, and a thread — three message-domain kinds interleaved with the others).
    let mixed = path_str(&dir, "mixed.sqlite");
    build_flow(&mixed);
    build_memory(&mixed);
    {
        let mut chat = ChatStore::open(&mixed, 1).unwrap();
        let cid = "m-noise-chan";
        chat.add_member(cid, "workspace-A/alice", "workspace-A/alice");
        chat.post_message(&info_message(
            "workspace-A",
            cid,
            "noise the reducers ignore",
        ));
        chat.post_message(&info_message("workspace-A", cid, "more noise"));
        let tid = chat.mint_thread_id();
        chat.open_thread(
            &tid,
            &ThreadRoot {
                origin: "workspace-A".into(),
                channel_id: cid.into(),
                opener: "workspace-A/alice".into(),
                created: String::new(),
                parent: None,
            },
            "workspace-A/alice",
        );
        // M2 thread ops in the noise too (spec §7): the `expects_reply_from` + the new sparse
        // `deadline` LWW register must ALSO leave flow's/memory's views byte-identical.
        chat.set_expects_reply_from(&tid, "[\"workspace-A/bob\"]", "workspace-A/alice");
        chat.set_deadline(&tid, "2026-07-20T00:00:00Z", "workspace-A/alice");
    }

    // Every flow view table (items + edges + notes) is byte-identical with vs without the chat noise.
    assert_eq!(
        dump_flow(&flow_only),
        dump_flow(&mixed),
        "flow's views are byte-identical whether or not message/fact ops share the log"
    );

    // memory's derived view is identical with vs without the chat/task noise.
    assert_eq!(
        dump_memory(&mem_only),
        dump_memory(&mixed),
        "memory's view is byte-identical whether or not message/task ops share the log"
    );
}

#[test]
fn minted_message_and_thread_ids_are_global_and_collision_free() {
    // Forward-compat gate (note 2026-07-09): the M1 substrate mints GLOBAL ids, not workspace-local
    // ones — `m-`+ULID (a 128-bit, coordination-free, time-sortable id). Two independent workspaces
    // mint distinct ids for the "same" logical action, so the later bridge relays across origins
    // without re-keying or a data migration.
    let mut a = ChatStore::open_in_memory(1);
    let mut b = ChatStore::open_in_memory(2);

    let ma = a.post_message(&info_message("workspace-A", "m-chan", "hi"));
    let mb = b.post_message(&info_message("workspace-B", "m-chan", "hi"));
    let ta = a.mint_thread_id();
    let tb = b.mint_thread_id();

    for id in [&ma, &mb, &ta, &tb] {
        assert!(id.starts_with("m-"), "kind-prefixed mint: {id}");
        assert_eq!(id.len(), 2 + 26, "m- + 26-char ULID: {id}");
    }
    assert_ne!(ma, mb, "independent workspaces mint distinct message ids");
    assert_ne!(ta, tb, "independent workspaces mint distinct thread ids");
}

#[test]
fn a_malformed_message_op_pulled_through_the_engine_is_stored_not_folded() {
    // Store-don't-fold for the CHAT domain, end-to-end through the REAL engine (spec §3.1): a
    // `message`/`post` op whose `value` is NOT a valid MessageEnvelope must ride the relay, land in
    // the peer's shared log, and — when chat refolds — be DEFERRED (`is_foldable` = false), never
    // folded and never panicking. This combines two guarantees that were previously proven only
    // separately: T1's unit-level malformed-envelope defer (`ChatStore::apply`) and the engine's
    // generic unknown-op defer (which used `domain = "task"`). Here they meet for `message`.
    let dir = TempDir::new().unwrap();
    let db_b = path_str(&dir, "b.sqlite");

    let relay = RelayLog::new();
    let stream = StreamId("stream-bad".into());

    // Seed the relay directly with a `message` op carrying a garbage envelope — a newer or buggy
    // peer (site 9) could author exactly this. The op shape is otherwise well-formed, so the message
    // reducer WILL attempt it and must reject at `is_foldable`, not at (infallible) `fold`.
    let bad = WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: "01KXBAD0000000000000000000".into(),
        lamport: 1,
        site: 9,
        domain: DOMAIN_MESSAGE.into(),
        target_kind: KIND_MESSAGE.into(),
        target_id: "m-not-an-envelope".into(),
        field: FIELD_ENVELOPE.into(),
        op_type: OP_POST.into(),
        value: Some("{ this is not a MessageEnvelope".into()),
        author: "acme/mallory".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    };
    relay.push(&stream, std::slice::from_ref(&bad)).unwrap();

    // Machine B pulls it over the REAL engine into its shared log (flow store-don't-folds it).
    {
        let mut flow_b = Store::open(&db_b, 2).unwrap();
        let mut marks = Watermarks::default();
        let out = sync(&mut flow_b, 2, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
        assert_eq!(out.pulled, 1, "the malformed message op was pulled");
        let n_ops: i64 = flow_b
            .connection()
            .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_ops, 1, "kept in the shared log (not dropped)");
    }

    // Chat opens over the same db and refolds — the bad op must DEFER, not fold, not panic.
    let chat_b = ChatStore::open(&db_b, 2).unwrap();
    let folded: i64 = chat_b
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        folded, 0,
        "a malformed message op never folds into the chat view"
    );
    // Store-not-fold: the op stays in the shared log, so a later build that understands it could
    // still resurface it.
    let logged: i64 = chat_b
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM ops WHERE domain=?1 AND target_id='m-not-an-envelope'",
            [DOMAIN_MESSAGE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(logged, 1, "stored-not-folded: the op remains in the log");
}

#[test]
fn a_foreign_ops_forged_envelope_sender_never_reaches_the_chat_views() {
    // 6j6v.aym3 acceptance, on the path the ticket is about: a FOREIGN op crossing the real sync
    // engine. Locally the two identities cannot diverge — `post_message` stamps `op.author` FROM
    // the envelope — so a unit test can only show the reducer's preference; this shows the actual
    // threat. A peer writes a message whose payload claims `sender: acme/alice` while the op it
    // rides in is authored by `acme/mallory`. `apply` may not reject it (§7) and nothing
    // reconciles the two, so whichever field the reducer folds is what every read surface shows —
    // inbox, unread, expects_reply_from, transcript — and what every future ACL reads.
    //
    // Once 6j6v.6aza signs the op, `op.author` is the AUTHENTICATED identity while the envelope's
    // `sender` stays free text. Folding the envelope would have left the signature guarding a
    // field nobody reads, on exactly the "verify before agent action" path it exists for.
    let dir = TempDir::new().unwrap();
    let db_b = path_str(&dir, "b.sqlite");

    let relay = RelayLog::new();
    let stream = StreamId("stream-forged".into());

    let mut forged_envelope = info_message("acme", "m-chan", "ship the release");
    forged_envelope.sender = "acme/alice".into();
    let forged = WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: "01KXFORGED000000000000000".into(),
        lamport: 1,
        site: 9,
        domain: DOMAIN_MESSAGE.into(),
        target_kind: KIND_MESSAGE.into(),
        target_id: "m-forged".into(),
        field: FIELD_ENVELOPE.into(),
        op_type: OP_POST.into(),
        value: Some(serde_json::to_string(&forged_envelope).unwrap()),
        author: "acme/mallory".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    };
    relay.push(&stream, std::slice::from_ref(&forged)).unwrap();

    {
        let mut flow_b = Store::open(&db_b, 2).unwrap();
        let mut marks = Watermarks::default();
        let out = sync(&mut flow_b, 2, &stream, &mut marks, &relay, 500, &Unbounded).unwrap();
        assert_eq!(out.pulled, 1, "the forged message op was pulled");
    }

    let chat_b = ChatStore::open(&db_b, 2).unwrap();
    let sender: String = chat_b
        .connection()
        .query_row(
            "SELECT sender FROM messages WHERE message_id='m-forged'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        sender, "acme/mallory",
        "the view names the op's author, never the sender the payload claimed"
    );
}

// `a_workflow_run_survives_the_real_sync_engine_and_emits_the_same_events_on_a_peer` stood here —
// a `workflow_start` + two `step_advance` ops driven across the REAL relay, proving a run folded
// and its event stream re-derived identically on a peer. REMOVED with the run record and its event
// stream (6j6v.dvyq §3). What it guarded beyond the run — that chat's ops really cross the engine
// rather than merely being visible to a second handle on the same file — is
// `a_message_survives_the_real_sync_engine_and_folds_on_a_peer` above, which is the sibling it was
// written as.

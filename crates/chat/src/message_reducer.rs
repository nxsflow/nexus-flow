//! nexus-chat's message reducer (spec §3): folds `message`-domain ops into the chat views. The
//! third registered reducer in the platform (after task + fact). Dispatches on `target_kind` then
//! `(op_type, field)` — grow-only messages, per-field LWW channels/profiles, observed-remove OR-set
//! membership, grow-only+LWW threads. Stateless unit struct.

use crate::model::*;
use nxs_foundation::model::Op;
use nxs_foundation::reducer::Reducer;
use rusqlite::{params, Connection};

/// nexus-chat's reducer over the `message` domain.
pub struct MessageReducer;

impl MessageReducer {
    /// Whether the op names who wrote it. The kinds whose views carry an IDENTITY — a message's
    /// `sender`, a thread's `opener` — fold `op.author` (6j6v.aym3), so an op with a blank one has
    /// nothing to put there. A local write cannot produce that (the substrate asserts an
    /// attributable author on every emit, `6j6v.xsf3` invariant 1); `apply` takes a foreign op
    /// verbatim and §7 forbids discarding it, so this is the one place a non-conforming peer's op
    /// is caught. The consequence is store-don't-fold — the op stays in the log, it just does not
    /// materialize.
    ///
    /// The rule itself is `model::is_attributable`, not a local re-spelling: the substrate's assert
    /// and the embedder-facing `validate_author` decide the same question, and a view that folded
    /// an author the write seams reject would be the two definitions drifting apart.
    fn attributable(op: &Op) -> bool {
        nxs_foundation::model::is_attributable(&op.author)
    }

    /// A `message`/`post` op is foldable only if its `value` parses as a complete [`MessageEnvelope`]
    /// (all required fields present, enums in range). This is where ALL message validation lives —
    /// `fold` is infallible, so a bad envelope must be rejected here and deferred store-don't-fold
    /// (spec §3.1). Unknown extra envelope fields are ignored by serde (forward-compat).
    fn valid_envelope(op: &Op) -> bool {
        op.value
            .as_deref()
            .map(|v| serde_json::from_str::<MessageEnvelope>(v).is_ok())
            .unwrap_or(false)
    }

    fn fold_message(conn: &Connection, op: &Op) {
        // is_foldable guaranteed the parse; `message_id`/`created` come from the op, not the payload.
        let env: MessageEnvelope = serde_json::from_str(op.value.as_deref().unwrap()).unwrap();
        let refs = serde_json::to_string(&env.refs).unwrap();
        // `sender` is `op.author`, NOT the envelope's own `sender` (6j6v.aym3). The two agree on
        // every locally written op — `ChatStore::post_message` stamps the author FROM the envelope
        // — but on a foreign op nothing reconciles them, and `apply` may not reject one (§7). Once
        // 6j6v.6aza signs ops, `op.author` is the AUTHENTICATED identity and the envelope's
        // `sender` remains free text; every read surface here (and every ACL over them) has to
        // hang on the former, or signing would protect a field nobody reads.
        //
        // The payload keeps carrying `sender` — an older peer requires it to parse the envelope at
        // all, so dropping it would make new messages invisible across a mixed version stand — it
        // simply has no authority any more.
        //
        // FORWARD-ONLY, deliberately (PR #314 review, Code Quality #1 — tracked as 6j6v.q2vd). A
        // row a PRE-FIX build already folded from a foreign op still holds that op's CLAIMED
        // sender, and nothing here corrects it: `ChatStore::open` only forces a refold on a
        // chat-view schema change, and a refold would not help anyway — `refold` folds over the
        // EXISTING views (it does not clear them), so the `INSERT OR IGNORE` below no-ops on a row
        // that is already there. Correcting those rows needs a genuine rebuild (clear + refold),
        // which is its own decision: it would also re-resolve grow-only "first write wins" from
        // arrival order to `(lamport, site)` order, and that belongs in a change that is about
        // exactly that. What it must NOT become is an upsert here — a second op reusing a
        // message_id would then re-attribute an existing message to whoever wrote it last, which
        // is a re-attribution vector this fold has never had (see the grow-only test
        // `a_distinct_op_reusing_a_message_id_is_ignored_grow_only`).
        conn.execute(
            "INSERT OR IGNORE INTO messages(message_id, origin, channel_id, sender, kind, priority,
                 disposition, thread_id, refs, body, created, lamport, site)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                op.target_id,
                env.origin,
                env.channel_id,
                op.author,
                serde_json::to_value(env.kind).unwrap().as_str().unwrap(),
                serde_json::to_value(env.priority)
                    .unwrap()
                    .as_str()
                    .unwrap(),
                serde_json::to_value(env.disposition)
                    .unwrap()
                    .as_str()
                    .unwrap(),
                env.thread_id,
                refs,
                env.body,
                op.wall_clock,
                op.lamport,
                op.site
            ],
        )
        .unwrap();
    }

    /// One keep-if-beats LWW upsert on `table`, keyed by `id_col`, for the whitelisted field
    /// `op.field`. The whitelist (checked in `is_foldable`) is exactly what makes the
    /// `format!`-interpolated column name injection-safe (mirrors flow's `fold_item_lww`).
    fn fold_lww(conn: &Connection, op: &Op, table: &str, id_col: &str) {
        // Defense-in-depth: fold_lww serves KIND_CHANNEL, KIND_PROFILE and KIND_THREAD, so op.field
        // must be on ONE of those whitelists for the format!-interpolated column name below to be
        // injection-safe. is_foldable already guarantees this per-kind; this is the same
        // belt-and-suspenders check as flow's fold_item_lww (crates/core/src/task_reducer.rs).
        debug_assert!(
            CHANNEL_FIELDS.contains(&op.field.as_str())
                || PROFILE_FIELDS.contains(&op.field.as_str())
                || THREAD_FIELDS.contains(&op.field.as_str()),
            "fold_lww on non-whitelisted field: {}",
            op.field
        );
        let f = &op.field;
        let sql = format!(
            "INSERT INTO {table}({id_col}, {f}, {f}_v, {f}_site) VALUES(?1,?2,?3,?4)
             ON CONFLICT({id_col}) DO UPDATE SET
                 {f}=excluded.{f}, {f}_v=excluded.{f}_v, {f}_site=excluded.{f}_site
             WHERE (excluded.{f}_v, excluded.{f}_site) > ({f}_v, {f}_site)"
        );
        conn.execute(&sql, params![op.target_id, op.value, op.lamport, op.site])
            .unwrap();
    }

    fn fold_membership_add(conn: &Connection, op: &Op) {
        // target_id = channel{SEP}handle; tag = op_id (observed-remove), mirroring flow's edge OR-set.
        let Some((channel_id, handle)) = split2(&op.target_id) else {
            return;
        };
        conn.execute(
            "INSERT OR IGNORE INTO membership_adds(tag, channel_id, handle, lamport, site)
             VALUES(?1,?2,?3,?4,?5)",
            params![op.op_id, channel_id, handle, op.lamport, op.site],
        )
        .unwrap();
    }

    fn fold_membership_remove(conn: &Connection, op: &Op) {
        // value = SEP-joined observed add-tags this remove tombstones (mirrors fold_edge_remove).
        if let Some(tags) = &op.value {
            for tag in tags.split(SEP).filter(|t| !t.is_empty()) {
                conn.execute(
                    "INSERT OR IGNORE INTO membership_removes(tag) VALUES(?1)",
                    [tag],
                )
                .unwrap();
            }
        }
    }

    fn fold_thread_open(conn: &Connection, op: &Op) {
        // Immutable root; upsert so it converges whether or not a `set` already created the row
        // (root values are identical across any duplicate open).
        let Some(root) = op
            .value
            .as_deref()
            .and_then(|v| serde_json::from_str::<ThreadRoot>(v).ok())
        else {
            return;
        };
        // `opener` is `op.author` for the same reason `messages.sender` is (6j6v.aym3), and here
        // it is not even a display field: `facade::set_expects` is opener-only and compares the
        // caller against this column, so a foreign op's self-declared `ThreadRoot.opener` would be
        // an authorization claim anyone could write. `root.opener` stays in the payload for
        // backward-compatible parsing; it just no longer decides anything.
        // `parent` (nxf 6j6v.a71h §3.1) travels with the other immutable root fields — it is written
        // once by the op that opens the thread and never re-set, so it needs no LWW version pair.
        // A root op carries `None`, which lands as NULL: the tree's ROOT is the absence of an edge.
        conn.execute(
            "INSERT INTO threads(thread_id, origin, channel_id, opener, created, parent)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(thread_id) DO UPDATE SET
                 origin=excluded.origin, channel_id=excluded.channel_id,
                 opener=excluded.opener, created=excluded.created, parent=excluded.parent",
            params![
                op.target_id,
                root.origin,
                root.channel_id,
                op.author,
                root.created,
                root.parent
            ],
        )
        .unwrap();
    }
}

impl Reducer for MessageReducer {
    fn domain(&self) -> &'static str {
        DOMAIN_MESSAGE
    }

    fn is_foldable(&self, op: &Op) -> bool {
        match (op.target_kind.as_str(), op.op_type.as_str()) {
            (KIND_MESSAGE, OP_POST) => {
                op.field == FIELD_ENVELOPE && Self::attributable(op) && Self::valid_envelope(op)
            }
            (KIND_CHANNEL, OP_SET) => CHANNEL_FIELDS.contains(&op.field.as_str()),
            (KIND_PROFILE, OP_SET) => PROFILE_FIELDS.contains(&op.field.as_str()),
            (KIND_MEMBERSHIP, OP_ADD) => split2(&op.target_id).is_some(),
            (KIND_MEMBERSHIP, OP_REMOVE) => split2(&op.target_id).is_some(),
            (KIND_THREAD, OP_OPEN) => {
                op.field == FIELD_ROOT
                    && Self::attributable(op)
                    && op
                        .value
                        .as_deref()
                        .map(|v| serde_json::from_str::<ThreadRoot>(v).is_ok())
                        .unwrap_or(false)
            }
            (KIND_THREAD, OP_SET) => THREAD_FIELDS.contains(&op.field.as_str()),
            _ => false,
        }
    }

    fn fold(&self, conn: &Connection, op: &Op) {
        match (op.target_kind.as_str(), op.op_type.as_str()) {
            (KIND_MESSAGE, OP_POST) => Self::fold_message(conn, op),
            (KIND_CHANNEL, OP_SET) => Self::fold_lww(conn, op, "channels", "channel_id"),
            (KIND_PROFILE, OP_SET) => Self::fold_lww(conn, op, "profiles", "handle"),
            (KIND_MEMBERSHIP, OP_ADD) => Self::fold_membership_add(conn, op),
            (KIND_MEMBERSHIP, OP_REMOVE) => Self::fold_membership_remove(conn, op),
            (KIND_THREAD, OP_OPEN) => Self::fold_thread_open(conn, op),
            (KIND_THREAD, OP_SET) => Self::fold_lww(conn, op, "threads", "thread_id"),
            other => unreachable!("non-foldable op reached MessageReducer::fold(): {other:?}"),
        }
    }

    /// The chat views — and ONLY them. The session map, transcripts, leases, the working-tree queue
    /// and the other tables chat keeps in the same database are this machine's own state, not a fold
    /// of the log, and must never reach another replica through a snapshot.
    fn view_tables(&self) -> &'static [&'static str] {
        &[
            "messages",
            "channels",
            "membership_adds",
            "membership_removes",
            "profiles",
            "threads",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;
    use nxs_foundation::model::Op;
    use nxs_foundation::store::Store;

    /// An Op builder (mirrors fact_reducer.rs). op_id is unique per (target,lamport,site).
    pub(super) fn op(
        kind: &str,
        tid: &str,
        field: &str,
        op_type: &str,
        value: Option<&str>,
        lamport: i64,
        site: i64,
    ) -> Op {
        Op {
            // Sanitize SEP out of the op_id: for composite target_ids (membership's
            // `channel{SEP}handle`), op_id becomes the OR-set tag, and a raw SEP byte inside a
            // tag would corrupt fold_membership_remove's `value.split(SEP)` (the tag would get
            // sheared into fragments that never match the stored tag). tid.into() below keeps
            // the real composite id on op.target_id for split2 to consume.
            op_id: format!("op-{}-{field}-{lamport}-{site}", tid.replace(SEP, "_")),
            lamport,
            site,
            domain: DOMAIN_MESSAGE.into(),
            target_kind: kind.into(),
            target_id: tid.into(),
            field: field.into(),
            op_type: op_type.into(),
            value: value.map(str::to_string),
            author: "nxsflow/nexus-flow/PmAgent".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    /// Re-author an [`op`] — the identity E4 authenticates, and (since 6j6v.aym3) the one the
    /// views fold. Every other test wants the default; these want it apart from the payload's.
    fn authored_by(author: &str, op: Op) -> Op {
        Op {
            author: author.into(),
            ..op
        }
    }

    /// A substrate store with the chat views + message reducer wired.
    pub(super) fn store() -> Store {
        let mut s = Store::open_in_memory(1);
        schema::apply_chat_views(s.connection());
        s.register_reducer(Box::new(MessageReducer));
        s
    }

    fn envelope_json(channel: &str, body: &str) -> String {
        envelope_json_from("nxsflow/nexus-flow/PmAgent", channel, body)
    }

    fn envelope_json_from(sender: &str, channel: &str, body: &str) -> String {
        serde_json::to_string(&MessageEnvelope {
            origin: "nxsflow/nexus-flow".into(),
            channel_id: channel.into(),
            sender: sender.into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: body.into(),
        })
        .unwrap()
    }

    fn sender_of(s: &Store, message_id: &str) -> Option<String> {
        s.connection()
            .query_row(
                "SELECT sender FROM messages WHERE message_id=?1",
                [message_id],
                |r| r.get(0),
            )
            .ok()
    }

    /// Whether a table exists at all — the shape a REMOVED view has to be asserted with, since
    /// there is no column left to read and `SELECT` against a missing table is an error rather
    /// than an empty answer.
    fn table_exists(s: &Store, table: &str) -> bool {
        s.connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
    }

    #[test]
    fn a_message_folds_under_its_op_author_not_the_envelopes_claim() {
        // 6j6v.aym3, the case the ticket is named for. LOCALLY the two always agree —
        // `ChatStore::post_message` stamps `op.author = env.sender` — so this can only be shown on
        // a FOREIGN op, which is exactly where they can diverge: `apply` may not reject an op (§7)
        // and nothing reconciles the two. Once 6j6v.6aza signs ops, `op.author` is the
        // AUTHENTICATED identity while `envelope.sender` stays free text, so every read surface
        // (the thread reads, expects_reply_from, transcript) and every future ACL must hang on
        // the former. Folding the latter would have made the signature protect a field nobody
        // reads.
        let mut s = store();
        s.apply(&[authored_by(
            "acme/mallory",
            op(
                KIND_MESSAGE,
                "m-1",
                FIELD_ENVELOPE,
                OP_POST,
                Some(&envelope_json_from(
                    "acme/alice",
                    "c-1",
                    "transfer the funds",
                )),
                1,
                1,
            ),
        )]);
        assert_eq!(
            sender_of(&s, "m-1").as_deref(),
            Some("acme/mallory"),
            "the view shows who wrote the op, not who the payload claims wrote it"
        );
    }

    #[test]
    fn a_message_op_with_no_author_is_store_dont_fold() {
        // The other half of folding `op.author`: it must never be blank, or a read surface would
        // show a message attributable to nobody — matching no member, no expectation, no fan-out
        // target, while still standing in a channel's history. A local write cannot produce this
        // (the substrate asserts a non-blank author on every emit, 6j6v.xsf3 invariant 1), but
        // `apply` takes a foreign op verbatim, so a non-conforming peer can. §7 forbids DISCARDING
        // it — it stays in the log, it just does not materialize (same treatment as a malformed
        // envelope), and a later build that can attribute it may still fold it.
        let r = MessageReducer;
        for blank in ["", "   "] {
            assert!(
                !r.is_foldable(&authored_by(
                    blank,
                    op(
                        KIND_MESSAGE,
                        "m-x",
                        FIELD_ENVELOPE,
                        OP_POST,
                        Some(&envelope_json("c-1", "hi")),
                        1,
                        1
                    )
                )),
                "a message op authored by {blank:?} is not foldable"
            );
        }
        let mut s = store();
        s.apply(&[authored_by(
            "",
            op(
                KIND_MESSAGE,
                "m-x",
                FIELD_ENVELOPE,
                OP_POST,
                Some(&envelope_json("c-1", "hi")),
                1,
                1,
            ),
        )]);
        assert_eq!(sender_of(&s, "m-x"), None, "no row in the view");
        assert_eq!(s.export().len(), 1, "but the op is kept in the log (§7)");
    }

    #[test]
    fn a_read_cursor_op_from_an_older_peer_is_stored_never_folded_and_survives_a_refold() {
        // **The cross-version convergence claim of nxf 6j6v.4d2z, as a run rather than a reading.**
        // That change removed the `read_cursor` kind — its fold, its two `match` arms and the table
        // underneath — and the release note promises in so many words that "an older and a newer
        // peer still converge": a cursor op already in a synced log stays in the log and simply
        // stops being foldable. Structurally that follows from `is_foldable`'s `_ => false` arm and
        // the substrate's is-foldable-then-fold contract, but the property is an OFFLINE-FIRST
        // guarantee and a promise made to a user, so it is pinned here by the literal op kind
        // rather than left to be re-derived from two files (PR #450 review, Test Quality #1,
        // corroborated independently by that review's Integrity pass).
        //
        // Spelled out with a STRING LITERAL, deliberately: `KIND_READ_CURSOR` no longer exists, so
        // a constant would only re-state whatever this build happens to call things. What arrives
        // from a peer is bytes, and `"read_cursor"` is the byte sequence that peer wrote.
        let mut s = store();
        let legacy = op(
            "read_cursor",
            &format!("org/repo/a{SEP}c-1"),
            "seen",
            OP_SET,
            Some("m-9"),
            3,
            1,
        );
        // A message beside it, so "the views are empty" cannot pass for "the fold ran": the log
        // holds one op this build understands and one it does not.
        let message = op(
            KIND_MESSAGE,
            "m-1",
            FIELD_ENVELOPE,
            OP_POST,
            Some(&envelope_json("c-1", "hi")),
            4,
            1,
        );

        assert!(
            !MessageReducer.is_foldable(&legacy),
            "the reducer must not claim a kind it no longer folds — if it does, `fold` is reached \
             and its `unreachable!()` takes the process down on an op a peer is entitled to send"
        );

        s.apply(&[legacy.clone(), message]);
        assert_eq!(
            s.export().len(),
            2,
            "both ops are in the log (§7: never discard)"
        );
        assert_eq!(
            sender_of(&s, "m-1").as_deref(),
            Some("nxsflow/nexus-flow/PmAgent"),
            "the op this build DOES understand folded, so the cursor op was skipped rather than \
             the whole batch being dropped"
        );
        assert!(
            !table_exists(&s, "read_cursors"),
            "and nothing re-created the table the op names"
        );

        // A REFOLD is the sharper half: it replays the whole log through the reducer from scratch,
        // which is the path an existing workspace takes on a view-schema bump. The cursor op is met
        // again, every time, for as long as that log exists.
        s.refold();
        assert_eq!(s.export().len(), 2, "the refold kept both ops");
        assert_eq!(
            sender_of(&s, "m-1").as_deref(),
            Some("nxsflow/nexus-flow/PmAgent"),
            "and rebuilt the view around the op it cannot fold"
        );
        assert!(!table_exists(&s, "read_cursors"));
    }

    #[test]
    fn a_thread_opens_under_its_op_author_not_the_declared_opener() {
        // `threads.opener` is not decoration: `facade::set_expects` is OPENER-ONLY and compares
        // the caller against this column, so a forged `ThreadRoot.opener` on a foreign op is an
        // authorization claim, not a display string. Same fix, same reason as the message sender.
        let mut s = store();
        let root = serde_json::to_string(&ThreadRoot {
            origin: "nxsflow/nexus-flow".into(),
            channel_id: "c-1".into(),
            opener: "acme/alice".into(),
            created: String::new(),
            parent: None,
        })
        .unwrap();
        s.apply(&[authored_by(
            "acme/mallory",
            op(KIND_THREAD, "t-1", FIELD_ROOT, OP_OPEN, Some(&root), 1, 1),
        )]);
        let opener: String = s
            .connection()
            .query_row(
                "SELECT opener FROM threads WHERE thread_id='t-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(opener, "acme/mallory");

        // And a thread nobody can be shown to have opened does not materialize either — it would
        // otherwise land with a blank opener, which `set_expects` compares against.
        assert!(!MessageReducer.is_foldable(&authored_by(
            "  ",
            op(KIND_THREAD, "t-2", FIELD_ROOT, OP_OPEN, Some(&root), 1, 1)
        )));
    }

    #[test]
    fn domain_is_message() {
        assert_eq!(MessageReducer.domain(), "message");
    }

    #[test]
    fn folds_a_valid_message_grow_only_and_idempotent() {
        let mut s = store();
        let o = op(
            KIND_MESSAGE,
            "m-1",
            FIELD_ENVELOPE,
            OP_POST,
            Some(&envelope_json("c-1", "hi")),
            1,
            1,
        );
        s.apply(std::slice::from_ref(&o));
        s.apply(&[o]); // re-delivered op → AlreadySeen, no dup
        let (chan, body): (String, String) = s
            .connection()
            .query_row(
                "SELECT channel_id, body FROM messages WHERE message_id='m-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((chan.as_str(), body.as_str()), ("c-1", "hi"));
        let n: i64 = s
            .connection()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "grow-only, idempotent on message_id");
    }

    #[test]
    fn a_distinct_op_reusing_a_message_id_is_ignored_grow_only() {
        // Unlike re-delivery of the SAME op (proven above and short-circuited at the substrate's
        // op-log dedup before fold() ever runs again), this exercises TWO DISTINCT ops (different
        // op_id/lamport, different envelope) that target the SAME message_id — the case the
        // reducer's own `INSERT OR IGNORE INTO messages` (keyed on message_id) exists for.
        let mut s = store();
        let a = op(
            KIND_MESSAGE,
            "m-1",
            FIELD_ENVELOPE,
            OP_POST,
            Some(&envelope_json("c-1", "hi")),
            1,
            1,
        );
        let b = op(
            KIND_MESSAGE,
            "m-1",
            FIELD_ENVELOPE,
            OP_POST,
            Some(&envelope_json("c-1", "changed")),
            2,
            1,
        );
        assert_ne!(a.op_id, b.op_id, "must be distinct ops, not a re-delivery");
        s.apply(&[a]);
        s.apply(&[b]);
        let n: i64 = s
            .connection()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            n, 1,
            "grow-only, ignores a second op for the same message_id"
        );
        let body: String = s
            .connection()
            .query_row(
                "SELECT body FROM messages WHERE message_id='m-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, "hi", "messages are immutable: first write wins");
    }

    #[test]
    fn malformed_envelope_is_store_dont_fold_not_a_panic() {
        let r = MessageReducer;
        // Not JSON at all, missing required field, and bad enum: none are foldable → deferred (§3.1).
        assert!(!r.is_foldable(&op(
            KIND_MESSAGE,
            "m-x",
            FIELD_ENVELOPE,
            OP_POST,
            Some("{not json"),
            1,
            1
        )));
        assert!(!r.is_foldable(&op(
            KIND_MESSAGE,
            "m-x",
            FIELD_ENVELOPE,
            OP_POST,
            Some("{\"body\":\"x\"}"),
            1,
            1
        )));
        let bad_enum = envelope_json("c-1", "x").replace("\"info\"", "\"gossip\"");
        assert!(!r.is_foldable(&op(
            KIND_MESSAGE,
            "m-x",
            FIELD_ENVELOPE,
            OP_POST,
            Some(&bad_enum),
            1,
            1
        )));
        assert!(!r.is_foldable(&op(
            KIND_MESSAGE,
            "m-x",
            FIELD_ENVELOPE,
            OP_POST,
            None,
            1,
            1
        )));
        // Applying a malformed op through the store must NOT insert and must NOT panic.
        let mut s = store();
        s.apply(&[op(
            KIND_MESSAGE,
            "m-x",
            FIELD_ENVELOPE,
            OP_POST,
            Some("{bad"),
            1,
            1,
        )]);
        let n: i64 = s
            .connection()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "malformed op deferred, no row");
    }

    #[test]
    fn channel_name_is_keep_if_beats_lww() {
        let mut s = store();
        s.apply(&[op(
            KIND_CHANNEL,
            "c-1",
            "name",
            OP_SET,
            Some("review"),
            5,
            1,
        )]);
        // A lower (lamport,site) loses; a higher wins.
        s.apply(&[op(KIND_CHANNEL, "c-1", "name", OP_SET, Some("stale"), 2, 1)]);
        s.apply(&[op(KIND_CHANNEL, "c-1", "kind", OP_SET, Some("group"), 1, 1)]);
        let (name, kind): (String, String) = s
            .connection()
            .query_row(
                "SELECT name, kind FROM channels WHERE channel_id='c-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), kind.as_str()),
            ("review", "group"),
            "higher lamport wins per field"
        );
    }

    #[test]
    fn channel_name_lww_tie_breaks_on_higher_site_at_equal_lamport() {
        // The WHERE clause compares (v, site) pairs: `(excluded.name_v, excluded.name_site) >
        // (name_v, name_site)`. At EQUAL lamport, the higher site must win the tie-break.
        let mut s = store();
        s.apply(&[op(
            KIND_CHANNEL,
            "c-1",
            "name",
            OP_SET,
            Some("review"),
            5,
            1,
        )]);
        s.apply(&[op(KIND_CHANNEL, "c-1", "name", OP_SET, Some("newer"), 5, 2)]);
        let name: String = s
            .connection()
            .query_row(
                "SELECT name FROM channels WHERE channel_id='c-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            name, "newer",
            "equal lamport → higher site wins the tie-break"
        );
        // Order-independence: a later-applied op at the SAME lamport but a LOWER site must not
        // regress the value.
        s.apply(&[op(
            KIND_CHANNEL,
            "c-1",
            "name",
            OP_SET,
            Some("stale-again"),
            5,
            1,
        )]);
        let name2: String = s
            .connection()
            .query_row(
                "SELECT name FROM channels WHERE channel_id='c-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            name2, "newer",
            "equal lamport, lower site loses even applied after"
        );
    }

    #[test]
    fn profile_field_folds_and_foreign_field_defers() {
        let r = MessageReducer;
        assert!(r.is_foldable(&op(
            KIND_PROFILE,
            "org/repo/a",
            "job_title",
            OP_SET,
            Some("QA"),
            1,
            1
        )));
        // A field not on the whitelist can never reach the format!-built SQL.
        assert!(!r.is_foldable(&op(
            KIND_PROFILE,
            "org/repo/a",
            "salary",
            OP_SET,
            Some("1"),
            1,
            1
        )));
        let mut s = store();
        s.apply(&[op(
            KIND_PROFILE,
            "org/repo/a",
            "job_title",
            OP_SET,
            Some("QA reviewer"),
            1,
            1,
        )]);
        let t: String = s
            .connection()
            .query_row(
                "SELECT job_title FROM profiles WHERE handle='org/repo/a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t, "QA reviewer");
    }

    fn membership(
        op_type: &str,
        channel: &str,
        handle: &str,
        value: Option<&str>,
        lamport: i64,
        site: i64,
    ) -> Op {
        op(
            KIND_MEMBERSHIP,
            &format!("{channel}{SEP}{handle}"),
            "member",
            op_type,
            value,
            lamport,
            site,
        )
    }
    fn members(s: &Store, channel: &str) -> Vec<String> {
        let mut st = s
            .connection()
            .prepare(
                "SELECT handle FROM membership_adds WHERE channel_id=?1
               AND tag NOT IN (SELECT tag FROM membership_removes) ORDER BY handle",
            )
            .unwrap();
        let v: Vec<String> = st
            .query_map([channel], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        v
    }

    // `read_cursor_is_a_grow_only_max_register_never_regressing` stood here, with its `cursor` and
    // `seen` fixtures — the proof that the fold kept the GREATER watermark where an LWW-by-op
    // register would have rolled it back. It went with `fold_read_cursor` (nxf 6j6v.4d2z); a test
    // for a fold that no longer exists is a false map, not coverage. The ARGUMENT it stood for
    // outlives it and is kept where the next monotone register will look for it —
    // `docs/specs/nexus-chat-M1.md` §3.4, with its worked regression case.

    #[test]
    fn thread_open_is_grow_only_and_expects_reply_from_is_lww_order_independent() {
        let mut s = store();
        let root = serde_json::to_string(&ThreadRoot {
            origin: "nxsflow/nexus-flow".into(),
            channel_id: "c-1".into(),
            opener: "org/repo/a".into(),
            created: String::new(),
            parent: None,
        })
        .unwrap();
        // set expects_reply_from BEFORE open — must still converge (nullable root cols).
        s.apply(&[op(
            KIND_THREAD,
            "t-1",
            FIELD_EXPECTS_REPLY_FROM,
            OP_SET,
            Some("[\"org/repo/b\"]"),
            4,
            1,
        )]);
        s.apply(&[op(
            KIND_THREAD,
            "t-1",
            FIELD_ROOT,
            OP_OPEN,
            Some(&root),
            1,
            1,
        )]);
        let (chan, erf): (String, String) = s
            .connection()
            .query_row(
                "SELECT channel_id, expects_reply_from FROM threads WHERE thread_id='t-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((chan.as_str(), erf.as_str()), ("c-1", "[\"org/repo/b\"]"));
    }

    #[test]
    fn a_thread_open_folds_its_parent_edge_and_a_rootless_one_stays_null() {
        // nxf 6j6v.a71h §3.1: the parent rides in the `open` op's own `ThreadRoot`, so it lands with
        // the other immutable root columns. Two ops, one with and one without — and BOTH must fold,
        // which is the whole reason the field is `Option` + `#[serde(default)]`: `is_foldable` says
        // yes to an old root, so an already-persisted thread op keeps materializing.
        let mut s = store();
        let root = |parent: Option<&str>| {
            serde_json::to_string(&ThreadRoot {
                origin: "local".into(),
                channel_id: "c-1".into(),
                opener: "local/a".into(),
                created: String::new(),
                parent: parent.map(str::to_string),
            })
            .unwrap()
        };
        s.apply(&[
            op(
                KIND_THREAD,
                "t-root",
                FIELD_ROOT,
                OP_OPEN,
                Some(&root(None)),
                1,
                1,
            ),
            op(
                KIND_THREAD,
                "t-child",
                FIELD_ROOT,
                OP_OPEN,
                Some(&root(Some("t-root"))),
                2,
                1,
            ),
        ]);
        fn parent_of(s: &Store, id: &str) -> Option<String> {
            s.connection()
                .query_row("SELECT parent FROM threads WHERE thread_id=?1", [id], |r| {
                    r.get(0)
                })
                .unwrap()
        }
        assert_eq!(parent_of(&s, "t-root"), None, "no edge → the tree's ROOT");
        assert_eq!(parent_of(&s, "t-child"), Some("t-root".into()));

        // A root op written by a peer that has never heard of the field parses and folds the same
        // way — the literal bytes of a pre-a71h op, not a re-serialization of the struct.
        s.apply(&[op(
            KIND_THREAD,
            "t-old",
            FIELD_ROOT,
            OP_OPEN,
            Some(r#"{"origin":"local","channel_id":"c-1","opener":"local/a","created":""}"#),
            3,
            1,
        )]);
        assert_eq!(parent_of(&s, "t-old"), None);
    }

    #[test]
    fn thread_deadline_is_foldable_and_keep_if_beats_lww() {
        // M2 (spec §3.1): `thread set deadline` folds on the SAME keep-if-beats LWW path as
        // expects_reply_from — a higher (lamport,site) wins per the register, a lower loses.
        let r = MessageReducer;
        assert!(
            r.is_foldable(&op(
                KIND_THREAD,
                "t-1",
                FIELD_DEADLINE,
                OP_SET,
                Some("2026-07-20T00:00:00Z"),
                1,
                1
            )),
            "a thread set deadline op is foldable (M2)"
        );
        let mut s = store();
        s.apply(&[op(
            KIND_THREAD,
            "t-1",
            FIELD_DEADLINE,
            OP_SET,
            Some("2026-07-20T00:00:00Z"),
            5,
            1,
        )]);
        // A lower (lamport,site) loses; row may not exist yet is exercised too (set-before-open).
        s.apply(&[op(
            KIND_THREAD,
            "t-1",
            FIELD_DEADLINE,
            OP_SET,
            Some("2026-07-01T00:00:00Z"),
            2,
            1,
        )]);
        let deadline: String = s
            .connection()
            .query_row(
                "SELECT deadline FROM threads WHERE thread_id='t-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            deadline, "2026-07-20T00:00:00Z",
            "higher lamport wins the deadline register"
        );
    }

    #[test]
    fn thread_name_is_foldable_and_keep_if_beats_lww_like_every_other_thread_register() {
        // nxf 6j6v.e76c. The claim the review of PR #452 asked to see driven directly (Test Quality
        // · Low): `thread set name` reaches the reducer as an ORDINARY thread register — the same
        // whitelist, the same keep-if-beats path as `deadline` above — and an OLD replica that does
        // not list the field meets it as `thread_set_of_an_unknown_field_is_not_foldable` below
        // does, which is store-don't-fold and never a crash.
        let r = MessageReducer;
        assert!(r.is_foldable(&op(
            KIND_THREAD,
            "t-1",
            FIELD_NAME,
            OP_SET,
            Some("Fix the login redirect"),
            1,
            1
        )));
        let mut s = store();
        s.apply(&[op(
            KIND_THREAD,
            "t-1",
            FIELD_NAME,
            OP_SET,
            Some("Fix the login redirect"),
            5,
            1,
        )]);
        // The lower (lamport,site) loses — which is also the WITNESS for the residue
        // `crate::model::FIELD_NAME` documents: "once" is enforced at the write, so two writers
        // that never saw each other converge on one name here rather than diverging.
        s.apply(&[op(
            KIND_THREAD,
            "t-1",
            FIELD_NAME,
            OP_SET,
            Some("Something else entirely"),
            2,
            1,
        )]);
        let name: String = s
            .connection()
            .query_row("SELECT name FROM threads WHERE thread_id='t-1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "Fix the login redirect");
    }

    #[test]
    fn thread_set_of_an_unknown_field_is_not_foldable() {
        // The (KIND_THREAD, OP_SET) dispatch is whitelisted (THREAD_FIELDS): a field outside it can
        // never reach the format!-built column name, mirroring the channel/profile whitelists.
        let r = MessageReducer;
        assert!(!r.is_foldable(&op(
            KIND_THREAD,
            "t-1",
            "priority",
            OP_SET,
            Some("urgent"),
            1,
            1
        )));
    }

    #[test]
    fn observed_remove_keeps_a_concurrent_unseen_add() {
        let mut s = store();
        // add #1 (alice) — its op_id is the tag the remove will observe.
        let a1 = membership(OP_ADD, "c-1", "alice", None, 1, 1);
        let a1_tag = a1.op_id.clone();
        s.apply(&[a1]);
        // A concurrent add of bob the remover never saw.
        s.apply(&[membership(OP_ADD, "c-1", "bob", None, 2, 2)]);
        // remove tombstones ONLY alice's observed tag.
        s.apply(&[membership(OP_REMOVE, "c-1", "alice", Some(&a1_tag), 3, 1)]);
        assert_eq!(
            members(&s, "c-1"),
            vec!["bob".to_string()],
            "alice removed, bob survives (observed-remove)"
        );
        // Rejoin: a fresh add revives alice.
        s.apply(&[membership(OP_ADD, "c-1", "alice", None, 4, 1)]);
        assert_eq!(
            members(&s, "c-1"),
            vec!["alice".to_string(), "bob".to_string()]
        );
    }
}

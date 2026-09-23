//! nexus-chat's materialized views (spec §4). Created over the substrate's `ops` log; the substrate
//! owns `ops` + the schema-version spine, this module owns only the chat-vocabulary DDL. The
//! message reducer folds into these; reads serve from them.

use rusqlite::Connection;

/// Apply the chat views, panicking on failure (in-memory stores / tests).
pub fn apply_chat_views(conn: &Connection) {
    try_apply_chat_views(conn).expect("chat views");
}

/// Apply the chat views (idempotent). `_v`/`_site` columns carry LWW Lamport metadata where the
/// field is an LWW register. `thread` root columns are NULLable so a `set expects_reply_from` that
/// folds BEFORE its `open` still converges (order-independence).
///
/// Returns `true` when it **upgraded** the `threads` table in place — i.e. it added a column a
/// folded root op can carry to a workspace created before that column existed. Two such minors so
/// far: the sparse `deadline` LWW columns (spec §7, M1 → M2) and the `parent` edge (nxf 6j6v.a71h
/// §3.1). A fresh db already has both (they are in the `CREATE TABLE` below), so it returns `false`.
/// The caller uses the flag to force a one-time refold: a plain `refold_if_behind` folds only what
/// the watermark has not seen, and both of these ops sit BELOW it — the `deadline` one because M1
/// store-don't-folded it while advancing the watermark past it, the `parent` one because an `open`
/// op from a newer peer folds perfectly well on an older schema, just with its parent dropped on the
/// floor. Only the view-schema bump resurfaces either (see `ChatStore::open`).
pub fn try_apply_chat_views(conn: &Connection) -> rusqlite::Result<bool> {
    let threads_existed = table_exists(conn, "threads")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages(
             message_id  TEXT PRIMARY KEY,
             origin      TEXT NOT NULL, channel_id TEXT NOT NULL, sender TEXT NOT NULL,
             kind        TEXT NOT NULL, priority TEXT NOT NULL, disposition TEXT NOT NULL,
             thread_id   TEXT, refs TEXT, body TEXT NOT NULL, created TEXT,
             lamport     INTEGER NOT NULL, site INTEGER NOT NULL
         );
         -- Causal-order reads (0b1m): back `WHERE channel_id=? / thread_id=? ORDER BY lamport, site,
         -- message_id` so the deterministic history/thread reads stay an index scan, not a sort. The
         -- old thread_id-only index is subsumed by the thread_causal prefix, so it is dropped (a
         -- one-time no-op on fresh dbs; the DROP IF EXISTS re-runs harmlessly on each open).
         CREATE INDEX IF NOT EXISTS messages_channel_causal ON messages(channel_id, lamport, site, message_id);
         CREATE INDEX IF NOT EXISTS messages_thread_causal  ON messages(thread_id, lamport, site, message_id);
         DROP INDEX IF EXISTS messages_thread;
         -- A third index stood here, `messages_channel(channel_id, message_id)`. Its whole
         -- justification was the inbox cursor's `channel_id=? AND message_id > seen` seek, which
         -- needed `message_id` as the second column; that read went with the unread apparatus
         -- (nxf 6j6v.4d2z), every surviving channel-scoped read is causal, and the causal index
         -- above already serves the `channel_id=?` prefix. An existing workspace sheds it on its
         -- next open, see `try_apply_chat_views`.

         CREATE TABLE IF NOT EXISTS channels(
             channel_id TEXT PRIMARY KEY,
             name   TEXT, name_v   INTEGER DEFAULT 0, name_site   INTEGER DEFAULT 0,
             kind   TEXT, kind_v   INTEGER DEFAULT 0, kind_site   INTEGER DEFAULT 0,
             origin TEXT, origin_v INTEGER DEFAULT 0, origin_site INTEGER DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS membership_adds(
             tag TEXT PRIMARY KEY, channel_id TEXT NOT NULL, handle TEXT NOT NULL,
             lamport INTEGER NOT NULL, site INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS membership_removes(tag TEXT PRIMARY KEY);

         CREATE TABLE IF NOT EXISTS profiles(
             handle TEXT PRIMARY KEY,
             job_title       TEXT, job_title_v       INTEGER DEFAULT 0, job_title_site       INTEGER DEFAULT 0,
             job_description TEXT, job_description_v INTEGER DEFAULT 0, job_description_site INTEGER DEFAULT 0,
             capability_tags TEXT, capability_tags_v INTEGER DEFAULT 0, capability_tags_site INTEGER DEFAULT 0,
             runtime_binding TEXT, runtime_binding_v INTEGER DEFAULT 0, runtime_binding_site INTEGER DEFAULT 0,
             reports_to      TEXT, reports_to_v      INTEGER DEFAULT 0, reports_to_site      INTEGER DEFAULT 0,
             origin          TEXT, origin_v          INTEGER DEFAULT 0, origin_site          INTEGER DEFAULT 0
         );

         -- `parent` (nxf 6j6v.a71h §3.1) sits with the IMMUTABLE root columns, not with the two LWW
         -- registers below it: it arrives in the `open` op's own `ThreadRoot` and never changes
         -- afterwards, so it needs no `_v`/`_site` pair to resolve a concurrent re-set — there is no
         -- such thing. NULL is the tree's ROOT, which is why the root is derived and never stored as
         -- a flag. Deliberately no foreign key to `threads(thread_id)`: ops arrive in relay order, so
         -- a child's `open` can fold BEFORE its parent's (the same order-independence the NULLable
         -- root columns above exist for), and a constraint would reject the child instead of letting
         -- the two converge.
         CREATE TABLE IF NOT EXISTS threads(
             thread_id  TEXT PRIMARY KEY,
             origin     TEXT, channel_id TEXT, opener TEXT, created TEXT, parent TEXT,
             expects_reply_from TEXT,
             expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0,
             deadline TEXT, deadline_v INTEGER DEFAULT 0, deadline_site INTEGER DEFAULT 0,
             -- **What this conversation is CALLED** (nxf 6j6v.e76c) — a third LWW register beside
             -- the two above, folded on the same keep-if-beats path, and NULL for every thread
             -- nobody ever named. It is a DISPLAY name and never a key: the id stays the id, and
             -- nothing in this schema or above it joins on this column.
             name TEXT, name_v INTEGER DEFAULT 0, name_site INTEGER DEFAULT 0,
             -- **Which machine this chat runs on** (nxf 6j6v.1c6k) — a fourth LWW register, a
             -- machine id, NULL for every thread no machine was named for.
             machine TEXT, machine_v INTEGER DEFAULT 0, machine_site INTEGER DEFAULT 0
         );

         -- `read_cursors(consumer, channel_id, seen)` stood here — the synced per-consumer unread
         -- watermark (spec §3.4). REMOVED with the whole unread apparatus (nxf 6j6v.4d2z): its one
         -- consumer, `ChatStore::inbox`, is gone, and the session-start notice that briefly had a
         -- second claim on it is derived from the operation now (nxf 6j6v.2hx9). An existing
         -- workspace sheds it on its next open, see `try_apply_chat_views`.

         -- The role-runtime session map (6j6v.zenf T4): internal (nxc-minted) session id → its role
         -- and its real Claude Agent SDK session id. Plain table, NOT op-folded — it is a private,
         -- device-local binding (the real SDK id is meaningless to a sync peer), not a CRDT view.
         --
         -- `depth` is the spawn chain's hop count, anchored HERE rather than in the caller's own
         -- `NXC_HOP`/`Ctx.hop` (nxf 6j6v.m48m): a triggered session is stamped with its trigger's
         -- depth by the process that spawns it, so a role with Bash cannot hand itself a fresh
         -- budget by rewriting an ambient counter it controls. Monotonic — see
         -- `ChatStore::record_trigger_depth`.
         --
         -- `thread` is WHERE THIS SESSION IS (nxf 6j6v.a71h §3.1): the thread it was last put in
         -- motion on — the one it was triggered into, or the one whose reply resumed it. It is what
         -- makes the parent edge MECHANICAL rather than something an agent has to type: a
         -- `nxc send --to X` issued from inside a session hangs the fresh thread under this one, so
         -- the-thread-that-is-being-sent-FROM needs no argument and cannot be got wrong. Plain and
         -- device-local like the rest of this table, and for the same reason: it is keyed by an
         -- internal session id, which names a live process on THIS machine. What it produces — the
         -- parent on a `ThreadRoot` — IS synced; only the resolution is local.
         CREATE TABLE IF NOT EXISTS session_map(
             internal_id TEXT PRIMARY KEY, role TEXT NOT NULL, real_sdk_id TEXT, created TEXT,
             depth INTEGER NOT NULL DEFAULT 0, thread TEXT, ended TEXT, ended_before TEXT
         );

         -- **The collecting coordinator's pending queue** (nxf 6j6v.gn8b): an answer that arrived
         -- while the caller it belongs to was mid-turn, held until that session settles and then
         -- delivered ONCE with everything else that arrived meanwhile.
         --
         -- Plain and device-local for `session_map`'s reason, one step stronger: it is keyed by an
         -- internal session id, and a sync peer that folded it would be holding a delivery for a
         -- session it cannot start. Nothing here is shared truth — the ANSWER is, and it is in
         -- `messages` where it always was; this table only remembers who has not been told yet.
         --
         -- `UNIQUE(session, message_id)` is what makes the hold idempotent: a re-attempted wake — a
         -- second reply, a sweep, a retried command — must never put the same answer in front of a
         -- caller twice. `escalated` rides along rather than being re-derived from `messages.kind`
         -- at delivery time, so a hand-back is set apart from the batch by the fact recorded when it
         -- arrived.
         CREATE TABLE IF NOT EXISTS pending_wakes(
             id         INTEGER PRIMARY KEY AUTOINCREMENT,
             session    TEXT    NOT NULL,
             thread_id  TEXT    NOT NULL,
             message_id TEXT    NOT NULL,
             sender     TEXT    NOT NULL,
             body       TEXT    NOT NULL,
             escalated  INTEGER NOT NULL DEFAULT 0,
             arrived    TEXT    NOT NULL,
             UNIQUE(session, message_id)
         );
         -- The two reads: one queue in arrival order, and the sweep's list of sessions with
         -- something waiting.
         CREATE INDEX IF NOT EXISTS pending_wakes_by_session ON pending_wakes(session, id);

         -- The agent transcript (nxf epic 6wt2, ticket f8c9): the Claude Agent SDK stream of one
         -- role session — assistant text, extended thinking, tool_use/tool_result, and all
         -- Task-spawned subagent activity — normalized by the sidecar and appended through
         -- `nxc transcript append`. Plain table for the SAME reason `session_map` above is one, and
         -- then some: it describes an SDK session that exists on THIS machine only (meaningless to a
         -- sync peer), there is exactly one writer per session (so there is no concurrent-edit
         -- conflict an LWW/OR-set reducer would resolve), and folding it would balloon the SHARED op
         -- log with device-local noise — a transcript is by far the highest-volume thing chat
         -- records. `data` is the kind-specific payload as opaque JSON text; the consumer owns its
         -- shape, the store never looks inside.
         --
         -- `seq` is assigned HERE, not carried on the wire, so a `resume` that starts a second
         -- sidecar process for the same internal session CONTINUES the transcript instead of
         -- colliding with it. `PRIMARY KEY(internal_session, seq)` is therefore also the read index:
         -- it serves `WHERE internal_session=? ORDER BY seq` whole (gated by
         -- `transcript_reads_are_index_backed_not_a_sort`).
         --
         -- Deliberately NO foreign key to `session_map`: `create_pending_session` and the sidecar's
         -- first flush race, and a transcript for an unmapped session is still worth keeping — it is
         -- the evidence of what happened. The join is a read-side concern.
         CREATE TABLE IF NOT EXISTS agent_transcript(
             internal_session   TEXT    NOT NULL,
             seq                INTEGER NOT NULL,
             kind               TEXT    NOT NULL,
             tool_use_id        TEXT,
             parent_tool_use_id TEXT,
             subagent_type      TEXT,
             at                 TEXT,
             data               TEXT    NOT NULL,
             PRIMARY KEY(internal_session, seq)
         );
         -- Retention's read (nxf 6j6v.t7pa): `SELECT internal_session, MAX(at), COUNT(*) … GROUP BY
         -- internal_session` — one summary row per session, which is how a prune decides what is
         -- stale. The PRIMARY KEY index alone cannot serve it (it does not carry `at`, so every row
         -- would be fetched from the table); this one makes it a COVERING scan that never touches a
         -- `data` payload at all. Two narrow columns against rows that hold whole tool inputs, so
         -- the write amplification is noise next to what it reads past. Gated by
         -- `retention_reads_never_touch_the_payloads`.
         CREATE INDEX IF NOT EXISTS agent_transcript_session_at
             ON agent_transcript(internal_session, at);

         -- Six tables stood here — `workflow_runs`, `workflow_run_tickets`,
         -- `workflow_run_outcomes`, `workflow_run_sessions`, `workflow_run_channel_threads` and
         -- `workflow_step_liveness`. REMOVED with the run record (6j6v.dvyq §3); an existing
         -- workspace sheds them on its next open, see `try_apply_chat_views`.

         -- The per-member channel deadline (nxf 6j6v.nf38): the ONE session standing on a member
         -- thread, the window that thread's channel declared, and when that window currently falls
         -- due. PLAIN and device-local, and here the reason is not merely the same as the tables
         -- around it — it DECIDES the design. What resets this clock is `agent_transcript`, which is
         -- device-local because a transcript is the highest-volume thing chat produces (see its own
         -- DDL comment above); a reset that rode the op log would emit one op per sidecar flush, for
         -- every member of every channel, which is exactly the volume that decision keeps out. And a
         -- sync peer holds no transcript for the session, so it could neither observe the activity
         -- nor act on it. `quorum_sql` LEFT JOINs this table so a member thread's `deadline` — and
         -- therefore its `stale` — is this row when there is one, and the thread's own op-folded
         -- `deadline` register otherwise: one staleness rule, not two. One row per member thread,
         -- replaced in place on the member's next turn. See `member_deadline.rs`.
         CREATE TABLE IF NOT EXISTS channel_member_deadline(
             thread_id   TEXT PRIMARY KEY,
             session     TEXT NOT NULL,
             window_secs INTEGER NOT NULL,
             deadline    TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS channel_member_deadline_session
             ON channel_member_deadline(session);

         -- Who already spawned the synthesizer for a completed declared-channel thread (nxf
         -- 6j6v.296n). PLAIN and device-local for the same reason the three tables above are — it
         -- records that THIS machine started a session. The key is the thread PLUS the LWW version
         -- of the `expects_reply_from` register the two racers both observed, so one completion
         -- round admits exactly one claimant while a legitimately re-declared thread (a new round,
         -- hence a new version) can claim again. See `consolidation_claim.rs` for the CAS itself.
         -- Append-only, one narrow row per synthesis round — bounded by the boards this device has
         -- actually summarized, i.e. far below the message log it rides alongside.
         CREATE TABLE IF NOT EXISTS synthesis_claims(
             thread_id    TEXT    NOT NULL,
             expects_v    INTEGER NOT NULL,
             expects_site INTEGER NOT NULL,
             claimed      TEXT,
             PRIMARY KEY(thread_id, expects_v, expects_site)
         );

         -- The working-tree lease (nxf 6j6v.bqe0): which CHAIN of triggered role sessions currently
         -- has exclusive hands on THIS repo's working copy and build directory. PLAIN and
         -- device-local for the same reason `synthesis_claims` above is one: a lease on files sitting
         -- on this machine's disk means nothing to a synced replica, which has no such files of its
         -- own to hold or release. Exactly one row, under the constant key 'default' — one repo, one
         -- working copy, one lease to contend for, so the primary key needs to be no more elaborate
         -- than a literal. `acquire_working_tree` (working_tree.rs) folds acquire, inherit, and
         -- reclaim-an-expired-holder into one `INSERT ... ON CONFLICT DO UPDATE ... WHERE` statement,
         -- the same reason `synthesis_claims` above needs one: two `nxc` PROCESSES (`nxc send` and
         -- `nxc reply` are separate invocations against the same file) racing to acquire must not be
         -- able to interleave between reading whether it is free and writing that they now hold it.
         -- `expires` is the fallback for a chain that dies hard without releasing — there is no
         -- separate cleanup run; the next acquire attempt reclaims it inline.
         -- `contended_since` (nxf 6j6v.de9s) is NOT a second expiry. It is the instant this device
         -- first saw BOTH halves of contention at once: the holding area has handed its task back
         -- (an escalation is waiting on a human) AND something else is standing in the queue.
         -- NULL means the pair does not currently stand, which is the ordinary state and the
         -- reason parking never becomes the normal path — the owner's rule is that the clock
         -- starts at CONTENTION, not at the escalation, so an escalation nobody is waiting behind
         -- is measured by nothing. It is re-derived and rewritten (or cleared) at every point that
         -- could have changed either half, so it is a memo of a derivable fact rather than a
         -- state machine: the derivation is `orchestration::note_the_contention`.
         CREATE TABLE IF NOT EXISTS working_tree_lease(
             tree            TEXT PRIMARY KEY,
             scope_key       TEXT NOT NULL,
             acquired        TEXT NOT NULL,
             expires         TEXT NOT NULL,
             contended_since TEXT
         );

         -- The working-tree queue (nxf 6j6v.bqe0): triggers that did NOT win the lease above wait
         -- here instead of vanishing. PLAIN and device-local for the same reason `working_tree_lease`
         -- above is one: a queued entry is waiting on exclusive hands on THIS machine's working copy,
         -- and a synced replica has no working copy of its own to wait for — folding a queued entry
         -- into the CRDT op log would hand a remote device a wait-slot for a lease it could never
         -- hold or release, the same corruption `working_tree_lease`'s own rationale rules out for
         -- the lease itself. Only the trigger's INPUTS are stored, never a composed prompt —
         -- `role`/`session`/`thread`/`message`/`model`/`depth` are exactly what `trigger_role` needs
         -- to re-run its own composition later, against the catalogue in force AT THAT TIME (a
         -- frozen prompt captured at enqueue time would go silently stale the moment someone edits
         -- it while the entry waits). Since nxf 6j6v.n92p that catalogue is the OPERATION's own
         -- version (see `declaration_freeze` below) rather than whatever the folder happens to say
         -- at the moment the lease comes free — otherwise waiting in this queue would be the one
         -- way to slip an edited declaration into a chain already under way.
         -- `priority` is the caller's canonical ORDINAL (smaller = more urgent, the
         -- same label-vs-ordinal split `crates/chat/src/model.rs::Priority`'s callers already make at
         -- the surface), never the display label — an ordinal sorts correctly and needs no
         -- label-set lookup to compare. `enqueued_at` is plain TEXT holding the UTC-normalized
         -- RFC3339 form `working_tree.rs::to_utc_rfc3339` produces, for the same byte-sort reason
         -- `working_tree_lease.expires` above needs it: this column is an ORDERING key, and a
         -- mixed-offset or mixed-precision instant would mis-sort it. `working_tree_queue_order`
         -- backs `take`'s `ORDER BY priority, enqueued_at, id` — `id` (AUTOINCREMENT, insertion
         -- order) is the tie-breaker that makes same-instant ordering deterministic rather than
         -- however SQLite happens to walk equal keys.
         CREATE TABLE IF NOT EXISTS working_tree_queue(
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             scope_key   TEXT NOT NULL,
             role        TEXT NOT NULL,
             session     TEXT NOT NULL,
             thread      TEXT,
             message     TEXT NOT NULL,
             model       TEXT,
             depth       INTEGER NOT NULL,
             priority    INTEGER NOT NULL,
             enqueued_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS working_tree_queue_order
             ON working_tree_queue(priority, enqueued_at, id);

         -- The frozen declarations (nxf 6j6v.n92p, answering 6j6v.nby8's question 1): which VERSION
         -- of `.nxs-personas/` a running operation is bound to. PLAIN and device-local for the
         -- reason the two tables above are: `.nxs-personas/` is a folder on THIS machine's disk,
         -- read from THIS machine's filesystem, and a synced replica has its own — folding one
         -- device's reading of its own files into the CRDT log would hand a remote device a version
         -- of a folder it does not have.
         --
         -- TWO tables and not one, which is the answer to nby8's question 1 (a hash, a copy, or a
         -- git ref): a COPY, deduplicated by CONTENT HASH. The copy is what makes a declaration
         -- deleted mid-operation still resolve (nby8's question 2, the case a bare hash cannot
         -- answer); the hash is what stops the copy being paid for per operation, since the
         -- catalogue changes far more rarely than operations begin — N operations under one
         -- unchanged folder share ONE blob. `catalogue` is the JSON projection of `Definitions`
         -- (roles + channels), which is the same serde shape the YAML loader produces and therefore
         -- tolerates a field added or retired by a later version exactly as a `.yaml` written by an
         -- older one does.
         CREATE TABLE IF NOT EXISTS declaration_snapshot(
             hash      TEXT PRIMARY KEY,
             taken     TEXT NOT NULL,
             catalogue TEXT NOT NULL
         );
         -- Which snapshot one OPERATION runs under. The key is the operation ROOT THREAD --
         -- `ChatStore::thread_root`, the boundary that function own doc asks this to take rather
         -- than invent a second one. Written with INSERT-OR-IGNORE exactly once, when the operation
         -- is opened: first write wins, so two `nxc` processes racing at the start of one chain
         -- cannot bind it to two readings of the folder, and every later step reads back what the
         -- first one saw.
         --
         -- IT GROWS WITH EVERY OPERATION and nothing prunes it (PR #391 review, Integrity #4). A
         -- row is a thread id, a hex hash and an instant — about 120 bytes, so ten thousand
         -- operations are a megabyte. Deliberately left for nxf 6j6v.h515, which has to decide the
         -- rule together with nby8's open question of when an operation is provably OVER: deleting
         -- a binding reads as `this thread belongs to no operation` and falls silently back to the
         -- folder, which is the one direction this table exists to prevent.
         CREATE TABLE IF NOT EXISTS declaration_freeze(
             root_thread TEXT PRIMARY KEY,
             hash        TEXT NOT NULL,
             taken       TEXT NOT NULL
         );

         -- Which version of its own declaration a role LAST SPAWNED under (nxf 6j6v.pkw9). PLAIN
         -- and device-local for the two tables above's reason: it is a fact about this machine's
         -- `.nxs-personas/`, and a synced replica has its own folder.
         --
         -- ONE ROW PER ROLE, overwritten at every spawn, and that is the whole storage cost — this
         -- is a pointer to the current version, not a history of them. It is deliberately NOT the
         -- shape `declaration_freeze` has: that table answers `which catalogue is THIS operation
         -- bound to` and therefore needs a row per operation (and, with it, 6j6v.h515's pruning
         -- question); this one answers `did the rules change since the last time this role ran`,
         -- which only ever needs the last answer. The freeze's `declaration_snapshot` is where the
         -- CONTENT behind a hash can be read back; nothing here has to carry it.
         CREATE TABLE IF NOT EXISTS role_declaration_version(
             role TEXT PRIMARY KEY,
             hash TEXT NOT NULL,
             seen TEXT NOT NULL
         );

         -- **Where an operation stood when it FIRST took the working copy** (nxf 6j6v.de9s,
         -- correction 2: 'DIE BASIS IST NICHT `main`'). One row per claim area, written once and
         -- never updated: the point is the branch the interrupted work STARTED from, which is
         -- exactly what a later acquire can no longer see. Without it a park would have to guess
         -- `main`, and an operation begun on a feature branch would be returned to the wrong tree.
         --
         -- PLAIN and device-local for the two tables above's reason, and harder: a branch name and
         -- a commit id describe THIS machine's checkout. A synced replica has a different clone,
         -- possibly with neither ref in it, and folding one device's HEAD into the CRDT log would
         -- hand a remote device a base it cannot check out.
         --
         -- `branch` is empty for a detached HEAD — a real state, kept rather than normalized away,
         -- with `commit` still naming the point exactly.
         CREATE TABLE IF NOT EXISTS operation_base(
             scope_key   TEXT PRIMARY KEY,
             branch      TEXT NOT NULL,
             commit_id   TEXT NOT NULL,
             recorded_at TEXT NOT NULL
         );

         -- **The branches an operation's work has been parked on** (nxf 6j6v.de9s). A LIST and not
         -- a field, because the owner's rule says so and the mechanism means it: one operation can
         -- be parked more than once — escalate, get answered, resume, escalate again — and each
         -- park is its own branch and its own commit.
         --
         -- Device-local for `operation_base`'s reason. `commit_id` is the ANCHOR the movement check
         -- compares against, and `base_branch`/`base_commit` are the second half of that comparison
         -- (correction 4: a branch nobody touched while the BASE ran on is the commoner case, and
         -- the draft's own check could not see it). `resumed_at` is NULL while the work is still
         -- parked; it is stamped rather than the row deleted, because 'this operation was parked
         -- twice and came back twice' is exactly the history somebody debugging a lost branch needs.
         --
         -- `created_branch` and `committed` are kept because they are the two questions somebody
         -- looking for work that seems to be missing actually asks: was this branch invented here
         -- (so it will not be found under a name a person chose), and was there anything to save at
         -- all (a clean tree parks with nothing committed, which is not the same as a park that
         -- lost something).
         CREATE TABLE IF NOT EXISTS parked_work(
             id             INTEGER PRIMARY KEY AUTOINCREMENT,
             scope_key      TEXT NOT NULL,
             branch         TEXT NOT NULL,
             commit_id      TEXT NOT NULL,
             base_branch    TEXT NOT NULL,
             base_commit    TEXT NOT NULL,
             created_branch INTEGER NOT NULL,
             committed      INTEGER NOT NULL,
             parked_at      TEXT NOT NULL,
             resumed_at     TEXT
         );
         CREATE INDEX IF NOT EXISTS parked_work_scope
             ON parked_work(scope_key, id);

         -- **A park that was refused for a reason that can pass, and is being retried** (nxf
         -- 6j6v.8bv9). A tree mid-merge, a git command that failed: the claim stays, the background
         -- service retries on every tick, and `nxc status` shows this row on the holding operation
         -- as `PARK REFUSED`. A permanent refusal never writes one — it hands the copy on instead.
         --
         -- One row per claim (`scope_key`, the lease holder's key), and it never outlives what it
         -- is about: a park that goes through clears it, the lease leaving the holder by any path
         -- clears it (the release and the expired reclaim in their own transaction, a different
         -- scope's acquire in the statement after its compare-and-swap), and so does the OCCASION
         -- named in `occasion` ceasing to stand — the escalation is answered, the hold is taken up.
         -- `first_at` is kept across retries ('retrying since'); `last_at` moves with each one.
         --
         -- `occasion` is why the row is stamped with one at all (fix round 1 of 6j6v.8bv9): several
         -- troubles hand the working copy on and each ends on its own terms, so the tick's park step
         -- clears only the row its own occasion wrote. Without it the choice would be between a mark
         -- that outlives the trouble ('retrying' for a park nobody is retrying) and a blanket clear
         -- that would drop a row whose occasion still stands and reset its 'retrying since'.
         --
         -- Device-local for `operation_base`'s reason, twice over: it describes THIS machine's
         -- checkout, and the lease it is about is itself device-local.
         CREATE TABLE IF NOT EXISTS park_refusals(
             scope_key TEXT PRIMARY KEY,
             refusal   TEXT NOT NULL,
             detail    TEXT NOT NULL,
             occasion  TEXT NOT NULL,
             first_at  TEXT NOT NULL,
             last_at   TEXT NOT NULL
         );

         -- **A holder whose running round was WITHDRAWN and whose work the tick is to park** (nxf
         -- 6j6v.b9nf). `withdraw` on a running round discharges the round's threads and asks the
         -- worker to stop its sessions — and then has to wait: a session's teardown is real work
         -- (it announces `nxc session ended`, a call back into this very store), so the copy cannot
         -- be parked while the process is still there. This row is what carries the intent across
         -- that wait. The tick's park step reads it, parks the holder's work once nothing in its
         -- claim area is running any more, and hands the copy on.
         --
         -- **It is also what keeps every OTHER release from handing the copy on unparked.** The
         -- discharge leaves the claim area owing nothing, which is exactly the state the ordinary
         -- end-of-operation release fires on — a late reply from the stopped session, its own
         -- teardown — and a copy handed on that way would arrive at the next holder with the
         -- withdrawn round's uncommitted work still in the tree. While this row stands for the
         -- holder, that release declines; the tick's withdrawn occasion is the one way such a
         -- holder lets go.
         --
         -- One row per claim (`scope_key`, the lease holder's key), and it never outlives what it
         -- is about: the lease leaving the holder by any path clears it — the release and the two
         -- reclaims in their own transaction, a different scope's acquire in the statement after
         -- its compare-and-swap — exactly `park_refusals`' discipline. `by` is who withdrew it and
         -- `withdrawn_at` when: what the tick's park reason (`why`) names, and — since nxf
         -- 6j6v.b9nf's Integrity #2 fix — what `nxc status` now puts on the holding operation too,
         -- beside the sessions still pinning it, so a reader is not left to infer a withdrawal from
         -- `holds working tree` and a discharged thread alone.
         --
         -- Device-local for `park_refusals`' reason: it is about THIS machine's checkout and the
         -- lease that is itself device-local.
         CREATE TABLE IF NOT EXISTS withdrawn_holders(
             scope_key    TEXT PRIMARY KEY,
             withdrawn_at TEXT NOT NULL,
             by           TEXT NOT NULL
         );

         -- **The sessions a withdrawal STOPPED, and whether the tick has asked them again** (nxf
         -- 6j6v.27b9 — the owner's decision of 2026-09-20: a withdrawn holder that survives its
         -- SIGTERM may get ONE further SIGTERM, from the tick). `withdrawn_holders` says which claim
         -- was taken back; this says which of the sessions pinning it the withdrawal itself asked to
         -- stop — the only sessions a further signal may ever reach.
         --
         -- **Why per session and not a column on the marker.** A human's follow-up into the
         -- withdrawn thread before the tick parked RESUMES the stopped session under the same id,
         -- in a new process, and the marker stays (see `note_withdrawn_holder`). That process is
         -- work somebody just asked for; signalling it would stop exactly that. So a session put
         -- back in motion loses its row at the trigger funnel (`forget_withdrawn_session`), and
         -- what is left is what the withdrawal stopped and nothing has started since.
         --
         -- `stopped_at` is when the withdrawal asked this session to stop — the clock the tick's five
         -- cadences are counted on, per session, so a session withdrawn a second time under a marker
         -- that is older is not signalled a minute after its own stop. `resignalled_at` is NULL until
         -- a tick CLAIMS the further SIGTERM (a compare-and-swap on it, before the signal, so two
         -- overlapping ticks cannot both send one) — at most one per session per withdrawal — and
         -- `resignal_refused` is the worker's own sentence when that attempt did not land, NULL when
         -- it did, so a later tick never calls a signal sent that was not.
         --
         -- The rows go with the marker, in the same statements (`forget_withdrawn_holder`,
         -- `forget_other_withdrawn_holders`), and one goes on its own when its session is put back in
         -- motion (`forget_withdrawn_session`, after a successful spawn). Device-local for the
         -- marker's own reason.
         CREATE TABLE IF NOT EXISTS withdrawn_sessions(
             scope_key        TEXT NOT NULL,
             session          TEXT NOT NULL,
             stopped_at       TEXT NOT NULL,
             resignalled_at   TEXT,
             resignal_refused TEXT,
             PRIMARY KEY (scope_key, session)
         );

         -- **A session that stopped at an availability boundary** (nxf 6j6v.npy3): the quota ran
         -- out, the provider went away, the network broke. One state — the model is not available
         -- to this session for a while, and nobody did anything wrong — and until this table the
         -- engine had no word for it. Measured: a weekly window that demonstrably closed mid-run
         -- left `session state` reporting a clean `ended` and its thread `answered`, with nothing
         -- `escalated` or `substituted`. The fact existed only in the transcript.
         --
         -- APPEND-ONLY, and that is the whole reason it is not a column on `session_map`: a resume
         -- clears `ended` (`reopen_session`), so an interruption kept the same way would be erased
         -- by the very act it exists to explain. `resumed_at` is a stamp, never a delete: on hold
         -- twice and taken up twice is the history somebody debugging a stalled operation needs,
         -- exactly as it is for `parked_work` above.
         --
         -- Plain and device-local for `session_map`'s reason: it is keyed by an internal session
         -- id, which names a process on THIS machine, and a sync peer folding it would be holding a
         -- return for a session it cannot start.
         --
         -- `limit_name` rather than `limit`, which is a reserved word in SQL. `until` is NULLABLE
         -- and a NULL is a real answer: the runtime stated no reset instant, so there is no moment
         -- to arm an automatic return for and the way back is the human's verb.
         CREATE TABLE IF NOT EXISTS session_interruption(
             id         INTEGER PRIMARY KEY AUTOINCREMENT,
             session    TEXT NOT NULL,
             at         TEXT NOT NULL,
             limit_name TEXT NOT NULL,
             until      TEXT,
             detail     TEXT NOT NULL,
             resumed_at TEXT
         );
         CREATE INDEX IF NOT EXISTS session_interruption_open
             ON session_interruption(session, id);
         -- **At most ONE standing hold per session, enforced by the database** (independent review
         -- of PR #476, Integrity & Robustness #1). `record_interruption` was a check-then-insert:
         -- read whether a hold stands, and insert if not. Two callers interleaving between those
         -- two statements — a retried teardown racing the original, two announcements of one
         -- session — would both read no-hold-standing and both insert, then carry two
         -- open holds for one closed window. A PARTIAL unique index makes the second insert a
         -- conflict instead of a duplicate, which is what lets the write be one statement; its
         -- sibling `mark_interruption_resumed` has always been a single conditional UPDATE, and
         -- this is that discipline arriving on the other half.
         --
         -- PARTIAL — `WHERE resumed_at IS NULL` — because the constraint is about holds that STAND.
         -- A session interrupted, taken up, and interrupted again is the case the append-only table
         -- exists for, and a plain unique index would forbid exactly that.
         CREATE UNIQUE INDEX IF NOT EXISTS session_interruption_one_open
             ON session_interruption(session) WHERE resumed_at IS NULL;",
    )?;
    // The M2 sparse-key minor (spec §7): a pre-M2 `threads` table lacks the `deadline` LWW columns.
    // `CREATE TABLE IF NOT EXISTS` above is a no-op for it, so add the columns in place. Idempotent —
    // guarded on column presence, so a fresh db (columns already present) and a re-open both skip it.
    let deadline_added = threads_existed && !column_exists(conn, "threads", "deadline")?;
    if deadline_added {
        conn.execute_batch(
            "ALTER TABLE threads ADD COLUMN deadline TEXT;
             ALTER TABLE threads ADD COLUMN deadline_v INTEGER DEFAULT 0;
             ALTER TABLE threads ADD COLUMN deadline_site INTEGER DEFAULT 0;",
        )?;
    }
    // The parent edge (nxf 6j6v.a71h §3.1): the same in-place, column-presence-guarded add for a
    // workspace whose `threads` table predates it — and, unlike `session_map.depth` and
    // `session_map.depth` below, this one DOES count as a view-schema bump. The difference is
    // whether the op log can already hold the value: a `thread`/`open` op carrying a `parent` folds
    // fine on the old schema (serde ignores the unknown field, so `is_foldable` says yes) and the
    // watermark advances past it with the edge silently dropped. Only a forced refold puts it back,
    // which is exactly what `upgraded` buys.
    let parent_added = threads_existed && !column_exists(conn, "threads", "parent")?;
    if parent_added {
        conn.execute_batch("ALTER TABLE threads ADD COLUMN parent TEXT;")?;
    }
    // The thread NAME (nxf 6j6v.e76c): the same in-place, column-presence-guarded add, and it
    // counts as a view-schema bump for `deadline`'s reason rather than `parent`'s — a
    // `thread`/`set name` op from a newer peer is NOT foldable on the old schema (`THREAD_FIELDS`
    // did not carry the field, so `is_foldable` says no) and is stored-not-folded with the
    // watermark advancing past it. Only a forced refold puts the name back.
    let name_added = threads_existed && !column_exists(conn, "threads", "name")?;
    if name_added {
        conn.execute_batch(
            "ALTER TABLE threads ADD COLUMN name TEXT;
             ALTER TABLE threads ADD COLUMN name_v INTEGER DEFAULT 0;
             ALTER TABLE threads ADD COLUMN name_site INTEGER DEFAULT 0;",
        )?;
    }
    // The chat's MACHINE (nxf 6j6v.1c6k): the same guarded add, and a view-schema bump for the name's
    // reason — a `thread`/`set machine` op from a newer peer is not foldable on the old schema.
    let machine_added = threads_existed && !column_exists(conn, "threads", "machine")?;
    if machine_added {
        conn.execute_batch(
            "ALTER TABLE threads ADD COLUMN machine TEXT;
             ALTER TABLE threads ADD COLUMN machine_v INTEGER DEFAULT 0;
             ALTER TABLE threads ADD COLUMN machine_site INTEGER DEFAULT 0;",
        )?;
    }
    let upgraded = deadline_added || parent_added || name_added || machine_added;
    // Created AFTER the two `threads` minors above, never inside the `CREATE TABLE` batch: on a
    // workspace that predates the `parent` column the index would name a column that does not exist
    // yet and the whole batch would fail. The tree walk (`ChatStore::thread_edges`) reads
    // `(thread_id, parent, channel_id)` whole and orders by `thread_id`, so all three columns are in
    // here: that makes the walk a COVERING scan instead of a table scan paging through every root's
    // origin/opener/expects payload. `channel_id` earns its place because `nxc status --channel`
    // selects its operations from the forest BEFORE reading any quorum (`facade::status`).
    //
    // A NEW NAME, and the superseded one dropped — never a redefinition under the same name (re-review
    // N3). `CREATE INDEX IF NOT EXISTS threads_parent` would be a silent no-op on a workspace built
    // from an intermediate commit of this branch, which carries the earlier two-column
    // `threads_parent`: the covering scan would be lost with nothing to notice it, since the gating
    // test only ever sees a fresh db. This is `messages_thread`'s pattern above, for its reason — a
    // one-time no-op on a fresh db, and the DROP re-runs harmlessly on every open.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS threads_tree ON threads(thread_id, parent, channel_id);
         DROP INDEX IF EXISTS threads_parent;",
    )?;
    // The persisted hop depth (nxf 6j6v.m48m): a workspace created before it has a `session_map`
    // without the column, and `CREATE TABLE IF NOT EXISTS` above is a no-op for it. Same in-place,
    // column-presence-guarded add as the `threads` minor — but deliberately NOT part of `upgraded`:
    // that flag forces a one-time REFOLD of the op log, and `session_map` is a plain device-local
    // table no reducer ever writes, so there is nothing for a refold to resurface. Existing rows
    // default to `0`, which reads as "an unknown, assumed-fresh chain" — the same thing an unset
    // `NXC_HOP` has always meant.
    if !column_exists(conn, "session_map", "depth")? {
        conn.execute_batch("ALTER TABLE session_map ADD COLUMN depth INTEGER NOT NULL DEFAULT 0;")?;
    }
    // The session's current thread (nxf 6j6v.a71h §3.1): same in-place add, and NOT part of
    // `upgraded` for exactly `depth`'s reason — no reducer writes `session_map`, so a refold has
    // nothing to resurface. An existing session reads NULL, which means "no thread known", i.e. what
    // it opens next is a ROOT. That is the fail-safe direction: a wrong root is a readable tree with
    // one operation too many, a wrong parent would splice a chain onto a stranger's.
    if !column_exists(conn, "session_map", "thread")? {
        conn.execute_batch("ALTER TABLE session_map ADD COLUMN thread TEXT;")?;
    }
    // **When the session's own process stopped** (nxf 6j6v.10yb): the instant the sidecar's teardown
    // announced its end through `nxc session ended`, or NULL for a session that has not said so.
    //
    // It exists because "I am finished" is a claim a session makes on a MESSAGE and "my process is
    // over" is a fact about this machine, and a sequential channel was reading the first as if it
    // were the second — the next step started while the previous one's session wrote for another 18
    // minutes, into the same working copy. Process liveness alone cannot answer it at the moment it
    // is asked: the announcement is made BY the ending session, whose process is necessarily still
    // alive while it speaks. So the fact is recorded, and [`crate::worker::Worker::
    // session_is_running`] stays as the backstop for a session that was killed hard and never got
    // to say anything.
    //
    // Same in-place, column-presence-guarded add as `depth` and `thread`, and NOT part of `upgraded`
    // for their reason: no reducer writes `session_map`. An existing row reads NULL, and NULL plus a
    // dead-or-absent process is "not running", which is what every pre-10yb session is.
    if !column_exists(conn, "session_map", "ended")? {
        conn.execute_batch("ALTER TABLE session_map ADD COLUMN ended TEXT;")?;
    }
    // **The end a resume erased** (nxf 6j6v.2hx9): `reopen_session` clears `ended` when a session is
    // put in motion again, which is right for the liveness gate that reads it and wrong for the one
    // fact a session start needs — WHEN THIS CALLER LAST STOPPED. Without somewhere to keep it, a
    // resumed session's own previous end is gone by the time it primes, and the window it should
    // derive its notice from does not exist. Same in-place, column-presence-guarded add as its
    // neighbours, and NOT part of `upgraded` for their reason: no reducer writes `session_map`. An
    // existing row reads NULL, which means "no previous end recorded on this device" — the same
    // answer a first start gives, and the one that yields an empty notice rather than a wrong one.
    if !column_exists(conn, "session_map", "ended_before")? {
        conn.execute_batch("ALTER TABLE session_map ADD COLUMN ended_before TEXT;")?;
    }
    // **The contention clock** (nxf 6j6v.de9s): a workspace created before it has a
    // `working_tree_lease` without the column, and `CREATE TABLE IF NOT EXISTS` above is a no-op for
    // it. Same in-place, column-presence-guarded add as the `session_map` minors above, and NOT part
    // of `upgraded` for their reason — no reducer writes `working_tree_lease`, so a refold has
    // nothing to resurface. An existing row reads NULL, which is "no contention seen yet": the next
    // point that could change either half re-derives it, so a lease held across an upgrade is
    // measured from when this device next looks rather than from a moment it never recorded. That is
    // the benign direction — a park happens later than it might have, never earlier.
    if !column_exists(conn, "working_tree_lease", "contended_since")? {
        conn.execute_batch("ALTER TABLE working_tree_lease ADD COLUMN contended_since TEXT;")?;
    }
    // **Which occasion refused the park** (fix round 1 of nxf 6j6v.8bv9): a store built from an
    // earlier commit of this branch has `park_refusals` WITHOUT it, and `CREATE TABLE IF NOT EXISTS`
    // above is a no-op for it. Same in-place, column-presence-guarded add as the ones above, and NOT
    // part of `upgraded` for their reason — no reducer writes this device-local table.
    //
    // The existing rows go with it, deliberately. A row is retry bookkeeping about the claim this
    // device holds RIGHT NOW, and one with no occasion could never be cleared by the occasion that
    // wrote it; the next tick that meets the same refusal writes it again with one, and the only
    // cost in between is one `PARK REFUSED` line missing for a tick. Idempotent, like its
    // neighbours: the guard skips the whole thing on every open after the first.
    if !column_exists(conn, "park_refusals", "occasion")? {
        conn.execute_batch(
            "ALTER TABLE park_refusals ADD COLUMN occasion TEXT NOT NULL DEFAULT '';
             DELETE FROM park_refusals;",
        )?;
    }
    // **The run record's own tables, dropped rather than left standing** (6j6v.dvyq §3). Nothing
    // writes them any more: `WorkflowRunReducer` is unregistered, so the `workflow`-domain ops still
    // sitting in an existing log fold to nothing, and there is no verb on either surface that could
    // append another. Left in place they would be six tables whose contents can only ever get
    // staler, and a reader finding them would reasonably conclude the concept still exists.
    //
    // Deliberately NOT part of `upgraded`: that flag forces a refold to resurface ops the older view
    // schema store-don't-folded, and there is nothing here to resurface — the opposite, in fact.
    // Idempotent, and re-runs harmlessly on every open, exactly like the `DROP INDEX` above.
    conn.execute_batch(
        "DROP TABLE IF EXISTS workflow_runs;
         DROP TABLE IF EXISTS workflow_run_tickets;
         DROP TABLE IF EXISTS workflow_run_outcomes;
         DROP TABLE IF EXISTS workflow_run_sessions;
         DROP TABLE IF EXISTS workflow_run_channel_threads;
         DROP TABLE IF EXISTS workflow_step_liveness;",
    )?;
    // **The unread apparatus, shed the same way** (nxf 6j6v.4d2z). `read_cursors` was the synced
    // per-consumer watermark and `messages_channel` the index that existed for the one read that
    // seeked on it; both go, for the same reason the six above do — nothing writes them (the
    // reducer no longer claims the `read_cursor` op kind, so the cursor ops still in an existing
    // log fold to nothing) and nothing reads them.
    //
    // Deliberately NOT part of `upgraded`, for that block's reason: the flag forces a refold to
    // resurface ops an older view schema store-don't-folded, and there is nothing here to
    // resurface. Idempotent, and re-runs harmlessly on every open.
    conn.execute_batch(
        "DROP TABLE IF EXISTS read_cursors;
         DROP INDEX IF EXISTS messages_channel;",
    )?;
    Ok(upgraded)
}

/// Whether a table exists (used to tell a fresh db — where `CREATE TABLE` installs the M2 columns —
/// from a pre-M2 db whose existing `threads` table needs the in-place column add).
fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
}

/// Whether `table` has a column named `column` (via `PRAGMA table_info`).
fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut st = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn fresh_db_has_deadline_columns_and_reports_no_upgrade() {
        // A fresh workspace gets the M2 `deadline` LWW columns from the CREATE TABLE, so nothing is
        // upgraded in place — the O(1) reopen path stays (no forced refold).
        let conn = Connection::open_in_memory().unwrap();
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "fresh db: no in-place upgrade"
        );
        assert!(column_exists(&conn, "threads", "deadline").unwrap());
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "re-apply is idempotent: still no upgrade"
        );
    }

    #[test]
    fn causal_message_reads_are_index_backed_not_a_sort() {
        // 0b1m (PR-review Test Quality, Info): the causal channel/thread reads must ride
        // `messages_channel_causal` / `messages_thread_causal` — a SEARCH that ALSO satisfies the
        // `ORDER BY lamport, site, message_id`, not a full SCAN plus a temp-b-tree sort. A future
        // join/index change that regresses either read to a sort reds here instead of only slowing
        // down silently (mirrors the flow-core `labels_of` plan gate).
        let conn = Connection::open_in_memory().unwrap();
        try_apply_chat_views(&conn).unwrap();
        let plan = |sql: &str| -> String {
            let mut st = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            st.query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let p = plan(
            "SELECT message_id FROM messages WHERE channel_id='c-1' \
             ORDER BY lamport, site, message_id",
        );
        assert!(
            p.contains("messages_channel_causal"),
            "channel read plan: {p}"
        );
        assert!(
            !p.contains("TEMP B-TREE"),
            "channel sort must be index-backed: {p}"
        );
        let p = plan(
            "SELECT message_id FROM messages WHERE thread_id='t-1' \
             ORDER BY lamport, site, message_id",
        );
        assert!(
            p.contains("messages_thread_causal"),
            "thread read plan: {p}"
        );
        assert!(
            !p.contains("TEMP B-TREE"),
            "thread sort must be index-backed: {p}"
        );
    }

    #[test]
    fn transcript_reads_are_index_backed_not_a_sort() {
        // Same gate as `causal_message_reads_are_index_backed_not_a_sort`, for `agent_transcript`
        // (nxf epic 6wt2 / ticket f8c9): `transcript_rows`' `WHERE internal_session=? ORDER BY seq`
        // is the ONLY read shape on this table, and `PRIMARY KEY(internal_session, seq)` is meant to
        // serve it whole — a SEARCH on the primary-key index that ALSO satisfies the ORDER BY, not a
        // full SCAN plus a temp-b-tree sort. A transcript is the longest table per session (one row
        // per coalesced block), so a regression here is the one that would actually be felt; it reds
        // this test instead of only slowing down silently.
        let conn = Connection::open_in_memory().unwrap();
        try_apply_chat_views(&conn).unwrap();
        let mut st = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT seq, kind, data FROM agent_transcript \
                 WHERE internal_session='s-1' ORDER BY seq",
            )
            .unwrap();
        let p: String = st
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            p.contains("SEARCH agent_transcript"),
            "transcript plan: {p}"
        );
        assert!(
            !p.contains("TEMP B-TREE"),
            "transcript sort must be index-backed: {p}"
        );
    }

    #[test]
    fn retention_reads_never_touch_the_payloads() {
        // The prune's one query (nxf 6j6v.t7pa) must be served ENTIRELY out of
        // `agent_transcript_session_at` — a COVERING scan, not a plain index scan that then fetches
        // each row. The difference is the whole point of the index: the payloads it would otherwise
        // page through are the unbounded `tool_use` inputs this table exists to hold, and the prune
        // runs on a role session's first flush, i.e. on the write path.
        let conn = Connection::open_in_memory().unwrap();
        try_apply_chat_views(&conn).unwrap();
        let mut st = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT internal_session, MAX(at), COUNT(*) \
                 FROM agent_transcript GROUP BY internal_session",
            )
            .unwrap();
        let p: String = st
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            p.contains("COVERING INDEX agent_transcript_session_at"),
            "retention plan: {p}"
        );
        assert!(
            !p.contains("TEMP B-TREE"),
            "the group-by must be index-backed: {p}"
        );
    }

    #[test]
    fn the_retention_index_is_added_to_a_workspace_that_predates_it() {
        // An existing workspace has `agent_transcript` but no `agent_transcript_session_at`. The
        // index is created in place on the next open — and, like `session_map.depth`, that is NOT a
        // view-schema bump: an index carries no folded state, so there is nothing to resurface and a
        // forced whole-op-log refold would be pure cost.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE agent_transcript(
                 internal_session TEXT NOT NULL, seq INTEGER NOT NULL, kind TEXT NOT NULL,
                 tool_use_id TEXT, parent_tool_use_id TEXT, subagent_type TEXT, at TEXT,
                 data TEXT NOT NULL, PRIMARY KEY(internal_session, seq));
             INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                 VALUES('s-old', 0, 'assistant', '2026-07-01T00:00:00Z', '{}');",
        )
        .unwrap();
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "adding this index is not a view-schema bump: no forced refold"
        );
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='agent_transcript_session_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        // The pre-existing row is indexed by the build, so retention can date it immediately.
        let last: String = conn
            .query_row(
                "SELECT MAX(at) FROM agent_transcript WHERE internal_session='s-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, "2026-07-01T00:00:00Z");
    }

    #[test]
    fn a_pre_m2_threads_table_is_upgraded_in_place_exactly_once() {
        // The sparse-key minor (spec §7): an existing M1 `threads` table (no deadline columns) is
        // migrated in place; the flag is TRUE exactly once, so the caller force-refolds only then.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE threads(
                 thread_id TEXT PRIMARY KEY,
                 origin TEXT, channel_id TEXT, opener TEXT, created TEXT,
                 expects_reply_from TEXT,
                 expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0
             );",
        )
        .unwrap();
        assert!(!column_exists(&conn, "threads", "deadline").unwrap());
        assert!(
            try_apply_chat_views(&conn).unwrap(),
            "pre-M2 threads table → upgraded"
        );
        assert!(column_exists(&conn, "threads", "deadline").unwrap());
        assert!(column_exists(&conn, "threads", "deadline_v").unwrap());
        assert!(column_exists(&conn, "threads", "deadline_site").unwrap());
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "second apply: already migrated, no re-upgrade (O(1) reopen)"
        );
    }

    #[test]
    fn a_threads_table_without_the_parent_edge_is_upgraded_in_place_and_forces_a_refold() {
        // nxf 6j6v.a71h §3.1. An existing M2 workspace has `threads` WITH the deadline columns and
        // WITHOUT `parent`. The column is added in place — and this one reports an upgrade, unlike
        // `session_map.depth`/`workflow_runs.milestone`: a `thread`/`open` op carrying a parent folds
        // fine under the old schema (serde ignores the unknown field), so the watermark can already
        // have moved past a synced child whose edge was dropped. Only a forced refold puts it back.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE threads(
                 thread_id TEXT PRIMARY KEY,
                 origin TEXT, channel_id TEXT, opener TEXT, created TEXT,
                 expects_reply_from TEXT,
                 expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0,
                 deadline TEXT, deadline_v INTEGER DEFAULT 0, deadline_site INTEGER DEFAULT 0
             );
             INSERT INTO threads(thread_id, origin, channel_id, opener)
                 VALUES('t-old', 'local', 'c-1', 'local/a');",
        )
        .unwrap();
        assert!(!column_exists(&conn, "threads", "parent").unwrap());
        assert!(
            try_apply_chat_views(&conn).unwrap(),
            "a threads table without the parent edge → upgraded, so the caller force-refolds"
        );
        assert!(column_exists(&conn, "threads", "parent").unwrap());
        // The pre-existing row reads NULL, which is exactly what it means: nothing recorded a parent
        // for it, so it is a ROOT until a refold says otherwise.
        let parent: Option<String> = conn
            .query_row(
                "SELECT parent FROM threads WHERE thread_id='t-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent, None);
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "second apply: already migrated, no re-upgrade (O(1) reopen)"
        );
    }

    #[test]
    fn a_threads_table_without_the_name_register_is_upgraded_in_place_and_forces_a_refold() {
        // nxf 6j6v.e76c. The third `threads` minor, and it reports an upgrade for `deadline`'s
        // reason rather than `parent`'s: a `thread`/`set name` op arriving from a peer that
        // already has this column is NOT foldable under the old schema — `THREAD_FIELDS` did not
        // list the field, so `is_foldable` answers `false` and the op is stored, not folded, while
        // the watermark moves past it. A forced refold is the only thing that puts the name back.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE threads(
                 thread_id TEXT PRIMARY KEY,
                 origin TEXT, channel_id TEXT, opener TEXT, created TEXT, parent TEXT,
                 expects_reply_from TEXT,
                 expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0,
                 deadline TEXT, deadline_v INTEGER DEFAULT 0, deadline_site INTEGER DEFAULT 0
             );
             INSERT INTO threads(thread_id, origin, channel_id, opener)
                 VALUES('t-old', 'local', 'c-1', 'local/a');",
        )
        .unwrap();
        assert!(!column_exists(&conn, "threads", "name").unwrap());
        assert!(
            try_apply_chat_views(&conn).unwrap(),
            "a threads table without the name register → upgraded, so the caller force-refolds"
        );
        for column in ["name", "name_v", "name_site"] {
            assert!(column_exists(&conn, "threads", column).unwrap(), "{column}");
        }
        // A thread nobody ever named reads NULL, which is what it means — and is exactly the state
        // every surface must render as it did before the name existed.
        let name: Option<String> = conn
            .query_row(
                "SELECT name FROM threads WHERE thread_id='t-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, None);
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "second apply: already migrated, no re-upgrade (O(1) reopen)"
        );
    }

    #[test]
    fn the_tree_walk_is_index_backed_not_a_table_scan() {
        // The sibling of `causal_message_reads_are_index_backed_not_a_sort`, for the parent edge
        // (nxf 6j6v.a71h): `thread_edges`' `SELECT thread_id, parent, channel_id FROM threads ORDER
        // BY thread_id` is what every `nxc status` form walks BEFORE it reads a single quorum, and
        // `threads_tree` exists to serve it WITHOUT touching the table rows (which carry each
        // root's origin/opener/expects payload).
        let conn = Connection::open_in_memory().unwrap();
        try_apply_chat_views(&conn).unwrap();
        let mut st = conn
            .prepare(
                "EXPLAIN QUERY PLAN \
                 SELECT thread_id, parent, channel_id FROM threads ORDER BY thread_id",
            )
            .unwrap();
        let p: String = st
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(p.contains("COVERING INDEX threads_tree"), "tree plan: {p}");
        assert!(
            !p.contains("TEMP B-TREE"),
            "the order must be index-backed: {p}"
        );
    }

    #[test]
    fn a_workspace_carrying_the_superseded_two_column_tree_index_is_moved_onto_the_covering_one() {
        // Re-review N3. `threads_parent` was this branch's first shape of the tree index, before
        // `channel_id` joined it so `nxc status --channel` could select its operations without
        // reading a quorum. Re-CREATEing under the SAME name would have been a silent no-op on any
        // workspace built from an intermediate commit — the covering scan lost, and the gating test
        // (which only ever sees a fresh db) none the wiser. The new name plus a DROP of the old is
        // `messages_thread`'s pattern, and this asserts BOTH halves.
        let conn = Connection::open_in_memory().unwrap();
        try_apply_chat_views(&conn).unwrap();
        conn.execute_batch(
            "DROP INDEX IF EXISTS threads_tree;
             CREATE INDEX threads_parent ON threads(thread_id, parent);",
        )
        .unwrap();
        try_apply_chat_views(&conn).unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'threads_%'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(names, vec!["threads_tree".to_string()], "{names:?}");
    }

    #[test]
    fn a_pre_depth_session_map_gains_the_column_in_place_without_forcing_a_refold() {
        // nxf 6j6v.m48m: an existing workspace's `session_map` predates the persisted hop depth.
        // The column is added in place — and, unlike the `threads` minor above, WITHOUT reporting an
        // upgrade: `session_map` is a plain device-local table no reducer ever writes, so forcing a
        // whole-op-log refold for it would be pure cost with nothing to resurface.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_map(
                 internal_id TEXT PRIMARY KEY, role TEXT NOT NULL, real_sdk_id TEXT, created TEXT
             );
             INSERT INTO session_map VALUES('s-old', 'coding', 'real-1', '2026-07-01T00:00:00Z');",
        )
        .unwrap();
        assert!(!column_exists(&conn, "session_map", "depth").unwrap());
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "adding this column is not a view-schema bump: no forced refold"
        );
        assert!(column_exists(&conn, "session_map", "depth").unwrap());
        // The pre-existing row survives and reads as a fresh chain, exactly as an unset `NXC_HOP`
        // always did — a migration must not strand live sessions above the cap.
        let depth: u32 = conn
            .query_row(
                "SELECT depth FROM session_map WHERE internal_id='s-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(depth, 0);
        try_apply_chat_views(&conn).unwrap(); // second apply must not fail on a duplicate column
    }

    #[test]
    fn a_pre_10yb_session_map_gains_the_ended_column_and_its_rows_read_as_not_running() {
        // nxf 6j6v.10yb: a workspace whose `session_map` predates the announced session end. Same
        // in-place, presence-guarded add as `depth` and `thread`, and NOT an upgrade for their
        // reason. What matters beyond the column existing is what the old rows MEAN: NULL, which the
        // liveness question reads together with a dead-or-absent process as "not running" — so every
        // pre-existing session behaves exactly as it did before this item, rather than pinning a
        // channel open on a session nobody can ask about.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_map(
                 internal_id TEXT PRIMARY KEY, role TEXT NOT NULL, real_sdk_id TEXT, created TEXT,
                 depth INTEGER NOT NULL DEFAULT 0, thread TEXT
             );
             INSERT INTO session_map(internal_id, role, created) VALUES('s-old', 'coding', '2026-07-01T00:00:00Z');",
        )
        .unwrap();
        assert!(!column_exists(&conn, "session_map", "ended").unwrap());
        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "adding this column is not a view-schema bump: no forced refold"
        );
        assert!(column_exists(&conn, "session_map", "ended").unwrap());
        let ended: Option<String> = conn
            .query_row(
                "SELECT ended FROM session_map WHERE internal_id='s-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ended, None);
        try_apply_chat_views(&conn).unwrap(); // second apply must not fail on a duplicate column
    }

    #[test]
    fn a_workspace_that_still_carries_the_unread_apparatus_sheds_it_on_open() {
        // nxf 6j6v.4d2z. A db built before the removal has both `read_cursors` and the
        // `messages_channel` index, and rows in the table — the whole point is that a workspace
        // that USED the cursor is the one that must not keep it. Neither is re-created, and
        // shedding them must not report an upgrade: `upgraded` forces a refold to resurface ops an
        // older view schema store-don't-folded, and there is nothing here to resurface.
        let conn = Connection::open_in_memory().unwrap();
        try_apply_chat_views(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE read_cursors(
                 consumer TEXT NOT NULL, channel_id TEXT NOT NULL, seen TEXT NOT NULL,
                 PRIMARY KEY(consumer, channel_id)
             );
             INSERT INTO read_cursors(consumer, channel_id, seen)
                 VALUES('local/alice', 'c-1', 'm-1');
             CREATE INDEX messages_channel ON messages(channel_id, message_id);",
        )
        .unwrap();
        assert!(table_exists(&conn, "read_cursors").unwrap());

        assert!(
            !try_apply_chat_views(&conn).unwrap(),
            "shedding a retired table is not a view-schema bump: no forced refold"
        );
        assert!(!table_exists(&conn, "read_cursors").unwrap());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='messages_channel'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 0,
            "the index that existed for the cursor seek goes with it"
        );
        try_apply_chat_views(&conn).unwrap(); // and re-running on an already-shed db is harmless
    }

    #[test]
    fn apply_creates_all_chat_views_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply_chat_views(&conn);
        apply_chat_views(&conn); // second apply must not panic
        for t in [
            "messages",
            "channels",
            "membership_adds",
            "membership_removes",
            "profiles",
            "threads",
            "session_map",
            "agent_transcript",
            "channel_member_deadline",
            "working_tree_lease",
            "declaration_snapshot",
            "declaration_freeze",
            "operation_base",
            "parked_work",
            "park_refusals",
            "withdrawn_holders",
            "withdrawn_sessions",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "view {t} exists after apply");
        }
    }
}

//! The role-runtime session map (nxf epic 6j6v.zenf, T4): a plain, device-local table binding an
//! internal (nxc-minted) session id to its role and its real Claude Agent SDK session id.
//! Deliberately NOT op-folded — unlike `messages`/`channels`/etc this is not a CRDT view over the
//! shared log: the real SDK session id is meaningless to a sync peer (it names a live session on
//! THIS machine), so it lives in its own plain table the reducer never touches. T5/T6/T7 build
//! directly on these four methods.
//!
//! Lifecycle: `create_pending_session` mints the row before the real SDK session exists (`role` is
//! known up front, `real_sdk_id` is not yet); `bind_session` fills in the real id once the SDK
//! hands one back. `resolve_real`/`session_role` are the two reads T6/T7 need.

use rusqlite::{params, OptionalExtension};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{NxfError, Result};
use crate::store::ChatStore;

/// The `created` timestamp stamped on a freshly-minted pending session: a pinned `NXC_NOW`
/// (golden/test determinism, mirrors the CLI's `resolve_now` in `cli.rs`), else the system clock.
/// Store-side (not CLI-side) because `session_map` is written directly by the store, not through an
/// op — there is no `emit()`/`wall_clock` to ride, so this is its own reference-time resolution.
///
/// `pub(crate)` for one other caller: transcript retention (nxf 6j6v.t7pa) needs THIS device's
/// clock, and needs it to be the same clock the CLI's own `prune` anchors on — sharing the
/// resolution is what makes the automatic and manual passes one policy rather than two.
pub(crate) fn resolve_now() -> Result<String> {
    match std::env::var("NXC_NOW").ok().filter(|s| !s.is_empty()) {
        Some(s) => Ok(s),
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

/// One row of the session map (nxf 6j6v.h383) — everything this table holds about one session.
///
/// A plain record of what is STORED. What that means for a session's actual state — running, over,
/// or unknowable — is a question about a PROCESS as well as a row, so it is answered one layer up,
/// where a [`crate::worker::Worker`] is in reach ([`crate::orchestration::session_state`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// The internal, nxc-minted session id — this table's key.
    pub internal_id: String,
    /// The declared role this session was minted for.
    pub role: String,
    /// The runtime's own session id, once `bind_session` has been told it.
    pub real_sdk_id: Option<String>,
    /// When the row was minted.
    pub created: Option<String>,
    /// Where this session is: the thread it was last put in motion on (see the DDL in `schema.rs`).
    pub thread: Option<String>,
    /// When the session announced its own end, and `None` while it has not.
    pub ended: Option<String>,
}

/// The one place the six columns are read off a row, so [`ChatStore::session_row`] and
/// [`ChatStore::sessions_in_thread`] cannot drift apart in what they select or in what order.
fn read_session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        internal_id: r.get(0)?,
        role: r.get(1)?,
        real_sdk_id: r.get(2)?,
        created: r.get(3)?,
        thread: r.get(4)?,
        ended: r.get(5)?,
    })
}

impl ChatStore {
    /// Mint the internal↔role binding before the real SDK session id is known. Idempotent by
    /// design (`INSERT OR IGNORE`): re-minting an already-pending (or already-bound) `internal_id`
    /// is a no-op, not an error — a caller retrying a setup step should not clobber a binding a
    /// previous attempt already made.
    pub fn create_pending_session(&mut self, internal_id: &str, role: &str) -> Result<()> {
        let now = resolve_now()?;
        self.connection().execute(
            "INSERT OR IGNORE INTO session_map(internal_id, role, real_sdk_id, created)
             VALUES (?1, ?2, NULL, ?3)",
            params![internal_id, role, now],
        )?;
        Ok(())
    }

    /// Fill in the real Claude Agent SDK session id for a previously-minted `internal_id`. An
    /// unknown `internal_id` (never minted via `create_pending_session`) is the CLI's `not_found` —
    /// binding must not silently succeed with zero rows affected.
    pub fn bind_session(&mut self, internal_id: &str, real_sdk_id: &str) -> Result<()> {
        let n = self.connection().execute(
            "UPDATE session_map SET real_sdk_id = ?1 WHERE internal_id = ?2",
            params![real_sdk_id, internal_id],
        )?;
        if n == 0 {
            return Err(NxfError::not_found(format!(
                "no such internal session: {internal_id}"
            )));
        }
        Ok(())
    }

    /// The bound real SDK session id for `internal_id`, or `None` if the row does not exist OR
    /// exists but is still pending (`real_sdk_id` NULL) — both read as "not yet resolvable to a
    /// real session" to the caller, which is all T6/T7 need (see the module docs).
    pub fn resolve_real(&self, internal_id: &str) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT real_sdk_id FROM session_map WHERE internal_id = ?1",
                [internal_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Stamp `depth` as the spawn-chain depth of `internal_id` — what makes the depth guard
    /// server-side state rather than a number the triggered process reports about itself (nxf
    /// 6j6v.m48m).
    ///
    /// **Monotonic (`MAX`), not assignment.** The depth is a circuit breaker on a chain that costs a
    /// real detached process plus a paid LLM call per hop, so it may only ever be raised. Assignment
    /// would reopen the very hole this closes from a different angle: any caller that presents a
    /// SHALLOWER context — a role that dropped its own ambient session, or a human resuming a
    /// session a deep chain already drove — would hand that session a full fresh budget. The cost of
    /// `MAX` is that a session which once ran deep keeps a shortened budget afterwards; that is the
    /// fail-closed direction, and it is bounded by the session's own lifetime.
    ///
    /// An `internal_id` that was never minted is a silent no-op, deliberately unlike
    /// [`ChatStore::bind_session`]'s `not_found`: this rides along with every trigger rather than
    /// being a caller's explicit act, and there is nothing to protect on a row that does not exist.
    pub fn record_trigger_depth(&mut self, internal_id: &str, depth: u32) -> Result<()> {
        self.connection().execute(
            "UPDATE session_map SET depth = MAX(depth, ?1) WHERE internal_id = ?2",
            params![depth, internal_id],
        )?;
        Ok(())
    }

    /// The recorded spawn-chain depth of `internal_id`, or `None` if it was never minted.
    ///
    /// `None` and `Some(0)` are deliberately distinct: `Some(0)` is a session this workspace minted
    /// at the head of a chain, `None` is an id that names no session of ours at all — which is what
    /// [`crate::orchestration::resolve_hop`] falls back on and must be able to tell apart.
    pub fn session_depth(&self, internal_id: &str) -> Result<Option<u32>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT depth FROM session_map WHERE internal_id = ?1",
                [internal_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Record WHERE this session is: the thread it was just put in motion on (nxf 6j6v.a71h §3.1).
    ///
    /// This is the mechanical half of the parent rule. §3.1 says the parent of a fresh thread is
    /// "the thread it is being sent FROM, not the one on whose behalf you act" — and the item's own
    /// chain makes the difference concrete: the `#coding` supervisor acts on behalf of T2 (its
    /// conversation with the pm) but sends to `#review` while it is reading the coder's answer in
    /// T3, so T4 hangs under T3. A session's CURRENT thread is therefore whatever last put it in
    /// motion: the thread it was triggered into, or the thread of the reply that resumed it. Every
    /// [`crate::orchestration::trigger_role`] call writes it, which is why a
    /// `nxc send --to <target>` needs no `--parent` argument and no cooperation from the agent.
    ///
    /// **Assignment, not `MAX` like [`record_trigger_depth`](ChatStore::record_trigger_depth)** —
    /// the two look alike and mean opposite things. A depth is a budget that may only ever shrink,
    /// so it is monotonic. This is a POSITION, and a session that moves from T2 to T3 and back to T2
    /// must be able to say so; a monotonic rule would freeze it wherever it happened to start.
    ///
    /// `thread: None` clears nothing — a trigger that names no thread (the ephemeral synthesizer, a
    /// bare resume) leaves the session where it was rather than making its next `send` a spurious
    /// root. An `internal_id` that was never minted is a silent no-op, exactly like
    /// `record_trigger_depth`: this rides along with every trigger rather than being a caller's own
    /// act.
    pub fn record_session_thread(&mut self, internal_id: &str, thread: Option<&str>) -> Result<()> {
        let Some(thread) = thread else {
            return Ok(());
        };
        self.connection().execute(
            "UPDATE session_map SET thread = ?1 WHERE internal_id = ?2",
            params![thread, internal_id],
        )?;
        Ok(())
    }

    /// The thread `internal_id` is currently working in, or `None` when it names no session of ours
    /// or that session has never been put in motion on a thread. See
    /// [`record_session_thread`](ChatStore::record_session_thread) for what "currently" means.
    pub fn session_thread(&self, internal_id: &str) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT thread FROM session_map WHERE internal_id = ?1",
                [internal_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// **Record that `internal_id`'s process is over** (nxf 6j6v.10yb) — the sidecar's teardown
    /// announcing its own end through `nxc session ended`, which is the only moment anything knows
    /// it.
    ///
    /// IDEMPOTENT and FIRST-WINS (`WHERE ended IS NULL`): a session announces its end once, and a
    /// second announcement — a retried teardown, a resume that ended again — must not move the
    /// instant, because what reads this reads it as "the process behind this row is gone" and a
    /// later stamp says nothing new. An `internal_id` that was never minted is a silent no-op, the
    /// same contract [`record_session_thread`](ChatStore::record_session_thread) has and for its
    /// reason: this rides along with a session's teardown rather than being a caller's own act.
    pub fn mark_session_ended(&mut self, internal_id: &str, at: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE session_map SET ended = ?1 WHERE internal_id = ?2 AND ended IS NULL",
            params![at, internal_id],
        )?;
        Ok(())
    }

    /// **This session is in MOTION again** (nxf 6j6v.y1t9) — clear the end it once announced.
    ///
    /// [`mark_session_ended`](ChatStore::mark_session_ended) is first-wins, which is right for one
    /// run of a session and wrong across a resume: a session that has ended and is then continued
    /// would carry `ended` for good, and [`unended_sessions_in_thread`](ChatStore::
    /// unended_sessions_in_thread) — which is what the liveness gate of nxf 6j6v.10yb reads — would
    /// never see it again. The gate would then silently stop applying to exactly the sessions a
    /// resume creates: a continued member genuinely writing in an exclusive checkout, invisible to
    /// the check that exists to keep a second session out of it.
    ///
    /// **Written at the one funnel every trigger passes through** (`orchestration::trigger_role`),
    /// beside the thread position and for the same reason: both say where a session IS, and both
    /// are true only of a session actually being put in motion. A freshly minted session has no
    /// `ended` to clear, so this is a no-op for every commission that is not a resume — which is
    /// what makes one unconditional call correct rather than a condition to keep in step.
    ///
    /// **What it does NOT erase since nxf 6j6v.2hx9**: the end itself moves to `ended_before` on the
    /// way out. A session start derives its notice — the commissions that finished while it was away
    /// — from exactly this instant ([`ChatStore::previous_session_end`]), and a resume that dropped
    /// it left the resumed session with no window at all. The two columns answer two different
    /// questions and both are needed: `ended` is "is the process gone RIGHT NOW", `ended_before` is
    /// "when did this caller last stop".
    ///
    /// A no-op for an unminted id, like every other write on this table.
    pub fn reopen_session(&mut self, internal_id: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE session_map
                SET ended_before = COALESCE(ended, ended_before), ended = NULL
              WHERE internal_id = ?1",
            params![internal_id],
        )?;
        Ok(())
    }

    /// The instant `internal_id` announced its end, or `None` for a session that has not — which is
    /// both "still working" and "never minted", exactly as [`session_thread`](ChatStore::
    /// session_thread) collapses its own two absences.
    pub fn session_ended_at(&self, internal_id: &str) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT ended FROM session_map WHERE internal_id = ?1",
                [internal_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// **The sessions standing in `thread` that have not announced an end** (nxf 6j6v.10yb) — the
    /// candidates for "somebody is still writing here", handed to the worker one by one so the
    /// PROCESS question is asked by the thing that owns processes.
    ///
    /// Ordered by `internal_id` so the answer is deterministic; a thread normally holds one such
    /// session, and holds two only while a resume is in flight.
    pub fn unended_sessions_in_thread(&self, thread: &str) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT internal_id FROM session_map
             WHERE thread = ?1 AND ended IS NULL
             ORDER BY internal_id",
        )?;
        let rows = stmt.query_map([thread], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// **Which of `internal_ids` have ANNOUNCED an end** — [`session_ended_at`](ChatStore::
    /// session_ended_at)'s question asked of an explicit SET, in one statement (nxf 6j6v.qmy6).
    ///
    /// It exists because `nxc status` carries a session per thread and now says what became of it.
    /// Asking the single-row reader once per thread would put an N+1 back into the one read this
    /// crate measures for not having one (`tests/bulk_quorum.rs`), and this is the same shape every
    /// other bulk read on that path already has: the caller keeps `internal_ids` under SQLite's
    /// variable ceiling (`facade::MAX_BOUND_IDS`) and falls back to
    /// [`ended_sessions`](ChatStore::ended_sessions) above it.
    ///
    /// **The answer is the POSITIVE set, and that is the whole reason this reads `ended IS NOT
    /// NULL` rather than its complement.** An id absent from the result is a session that has not
    /// ended *or* one this workspace never minted — a thread can name a session another device ran,
    /// because the return address travels with the message — and both of those are questions for
    /// the process side to answer, which is exactly where an absence sends the caller. The
    /// complement would send a foreign session the other way and report it as finished here.
    pub fn ended_sessions_among(
        &self,
        internal_ids: &[&str],
    ) -> Result<std::collections::BTreeSet<String>> {
        if internal_ids.is_empty() {
            return Ok(std::collections::BTreeSet::new());
        }
        let placeholders = vec!["?"; internal_ids.len()].join(",");
        let conn = self.connection();
        let mut stmt = conn.prepare(&format!(
            "SELECT internal_id FROM session_map
             WHERE ended IS NOT NULL AND internal_id IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(internal_ids.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// **Every session this workspace has minted that HAS announced an end** — the whole-table form
    /// of [`ended_sessions_among`](ChatStore::ended_sessions_among), for the caller whose id set has
    /// outgrown the variable ceiling. Same shape as `thread_assignee_sessions` and its `_for` twin:
    /// one projection, two filters.
    pub fn ended_sessions(&self) -> Result<std::collections::BTreeSet<String>> {
        let conn = self.connection();
        let mut stmt =
            conn.prepare("SELECT internal_id FROM session_map WHERE ended IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// **One session's whole row** (nxf 6j6v.h383) — the READ this table never had.
    ///
    /// `bind_session` and [`mark_session_ended`](ChatStore::mark_session_ended) write here, and
    /// until this item the only ways back out were four single-column readers, each carved for the
    /// one caller that needed it. The fact those columns hold — *is this session over?* — is the one
    /// a channel's advance has hung on since nxf 6j6v.10yb, and the only way anybody outside this
    /// crate could ask it was `ps -p $(cat .nxs/agent-logs/<id>.pid)`: a file in a private directory
    /// whose format is promised nowhere, read to answer a question this store already knows.
    ///
    /// `None` is a session this workspace never minted — distinct from a row whose every optional
    /// column is NULL, which is a session that was minted and has done nothing since.
    pub fn session_row(&self, internal_id: &str) -> Result<Option<SessionRow>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT internal_id, role, real_sdk_id, created, thread, ended
                 FROM session_map WHERE internal_id = ?1",
                [internal_id],
                read_session_row,
            )
            .optional()?)
    }

    /// **Every session recorded under `thread`** (nxf 6j6v.h383), ended ones included — which is
    /// what separates this from [`unended_sessions_in_thread`](ChatStore::
    /// unended_sessions_in_thread) beside it: that one collects candidates for "somebody is still
    /// writing here" and an ended session is not one, whereas the question here is "what ran here,
    /// and what became of it" and an ended session is the most interesting answer there is.
    ///
    /// Ordered by `internal_id`, which is a ULID and therefore chronological — the order a reader
    /// wants and a deterministic one.
    ///
    /// A thread nothing ever ran under is an EMPTY list rather than an error, and so is an id that
    /// names no thread at all: this table is keyed by session, not by thread, so it cannot tell the
    /// two apart, and inventing a `not_found` from an empty result would report the wrong one half
    /// the time. `transcript_page` draws the same line for the same reason.
    pub fn sessions_in_thread(&self, thread: &str) -> Result<Vec<SessionRow>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT internal_id, role, real_sdk_id, created, thread, ended
             FROM session_map WHERE thread = ?1 ORDER BY internal_id",
        )?;
        let rows = stmt.query_map([thread], read_session_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// **The NEWEST session of each of `threads`** (fix round 1 of nxf 6j6v.xb24) — one row per
    /// thread, in one statement.
    ///
    /// It is [`sessions_in_thread`](ChatStore::sessions_in_thread) asked of a SET and reduced to the
    /// row its caller wanted. `orchestration::holder_is_provably_dead` asks *did the session that
    /// owes this answer report its end?* of every owing thread in a claim area, and a claim area is
    /// a whole channel's member fan-out. Asking the single-thread reader once per thread was a pass
    /// over this table per thread — it has no index on `thread` — to keep one row of each, which is
    /// the N+1 this crate has written down beside it as the shape not to reintroduce
    /// ([`ended_sessions_among`](ChatStore::ended_sessions_among),
    /// [`interruptions_among`](ChatStore::interruptions_among)).
    ///
    /// **Newest is the greatest `internal_id`**, which is a ULID and therefore chronological — the
    /// same order [`sessions_in_thread`](ChatStore::sessions_in_thread) returns and for the same
    /// reason. It is read with SQLite's documented bare-column rule: in an aggregate query whose
    /// only aggregate is one `MIN`/`MAX`, the other columns come from the row that produced the
    /// extreme value. So one row per thread crosses the boundary instead of every row of every
    /// thread, which is what separates this from "read them all and keep the last".
    ///
    /// **A thread nothing ever ran in is ABSENT** rather than present with nothing in it — the
    /// positive-set contract of `ended_sessions_among` beside it: what comes back is what was found,
    /// and the caller decides what an absence means. For the caller above it means "nothing here
    /// ever died", which is a decline.
    ///
    /// Past SQLite's variable ceiling ([`crate::facade::MAX_BOUND_IDS`]) the same answer comes from
    /// one whole-table pass filtered in memory — `threads_owing_an_answer`'s own strategy, for its
    /// reason: one query either way, and never a bind list that cannot be bound.
    pub(crate) fn newest_sessions_in_threads(
        &self,
        threads: &[String],
    ) -> Result<std::collections::BTreeMap<String, SessionRow>> {
        if threads.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        // The six columns `read_session_row` reads, and the aggregate that picks WHICH row they are
        // read off. It is selected last so the reader's indices are untouched.
        const COLS: &str =
            "internal_id, role, real_sdk_id, created, thread, ended, MAX(internal_id)";
        let conn = self.connection();
        let rows: Vec<SessionRow> = if threads.len() > crate::facade::MAX_BOUND_IDS {
            let wanted: std::collections::BTreeSet<&str> =
                threads.iter().map(String::as_str).collect();
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM session_map WHERE thread IS NOT NULL GROUP BY thread"
            ))?;
            let rows = stmt.query_map([], read_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|r| r.thread.as_deref().is_some_and(|t| wanted.contains(t)))
                .collect()
        } else {
            let placeholders = vec!["?"; threads.len()].join(",");
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM session_map WHERE thread IN ({placeholders}) GROUP BY thread"
            ))?;
            let rows =
                stmt.query_map(rusqlite::params_from_iter(threads.iter()), read_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|r| r.thread.clone().map(|t| (t, r)))
            .collect())
    }

    /// The role an internal session id was minted with, or `None` if it was never minted.
    pub fn session_role(&self, internal_id: &str) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT role FROM session_map WHERE internal_id = ?1",
                [internal_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests that pin `NXC_NOW`, which is process-global. Exactly one takes it
    /// today; the lock is here so that the SECOND one — whenever it arrives — cannot race the
    /// first, rather than being retrofitted after a flake. (Same reason, same shape as
    /// `store.rs`'s `ID_SWITCH_LOCK` and `timer.rs`'s `ENV_LOCK`; a poisoned mutex is recovered
    /// so one failure does not cascade.)
    static NXC_NOW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Hold [`NXC_NOW_LOCK`] and guarantee the clock is unpinned again on the way out — including
    /// through a panicking assertion or a failed `unwrap`, which would otherwise leak a PINNED
    /// CLOCK into every later test of this binary. That leak is the reason this guard exists and
    /// not the second-setter race above: a `set_var`/`remove_var` pair with assertions between
    /// them is safe only while nothing between them can panic, and `unwrap` can. The damage would
    /// also land nowhere near here — every later `create_pending_session` in the binary would stamp
    /// July 19th and fail somewhere else entirely. Mirrors `store.rs`'s `IdSwitchGuard`.
    struct NxcNowGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl NxcNowGuard {
        /// Take the lock FIRST, then pin — the variable is never set outside the lock.
        fn pin(now: &str) -> NxcNowGuard {
            let guard = NxcNowGuard(NXC_NOW_LOCK.lock().unwrap_or_else(|e| e.into_inner()));
            std::env::set_var("NXC_NOW", now);
            guard
        }
    }

    impl Drop for NxcNowGuard {
        fn drop(&mut self) {
            std::env::remove_var("NXC_NOW");
        }
    }

    #[test]
    fn create_pending_session_is_idempotent_and_does_not_clobber_a_bound_session() {
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coding").unwrap();
        s.bind_session("s-1", "real-abc").unwrap();
        // Re-minting the same internal_id must not wipe the binding already in place.
        s.create_pending_session("s-1", "coding").unwrap();
        assert_eq!(s.resolve_real("s-1").unwrap().as_deref(), Some("real-abc"));
    }

    #[test]
    fn the_two_ended_reads_answer_the_same_question_and_an_absence_is_never_an_end() {
        // The bulk pair nxf 6j6v.qmy6 added, and the whole-table half is the one with no other
        // coverage (review of PR #444, Test Quality #4): `facade::session_states_for` falls back to
        // it past `MAX_BOUND_IDS`, so a workspace big enough to take that branch is exactly the one
        // where nobody would notice it answering differently.
        let mut s = ChatStore::open_in_memory(1);
        for id in ["m-a", "m-b", "m-c"] {
            s.create_pending_session(id, "coding").unwrap();
        }
        s.mark_session_ended("m-b", "2026-09-07T04:00:00Z").unwrap();

        let scoped = s.ended_sessions_among(&["m-a", "m-b", "m-c"]).unwrap();
        let whole = s.ended_sessions().unwrap();
        assert_eq!(
            scoped, whole,
            "the two strategies are two spellings of one question, and a caller that crosses the \
             ceiling must not get a different answer for it"
        );
        assert_eq!(
            scoped.iter().map(String::as_str).collect::<Vec<_>>(),
            ["m-b"]
        );

        // **The POSITIVE set, and this is the reason for it.** An id this workspace never minted is
        // absent from the answer exactly as an unended one is — so an absence sends the caller to
        // the process side, which is the truthful place for "this device never ran it". The
        // complement would have reported a foreign session as finished here.
        let foreign = s
            .ended_sessions_among(&["m-a", "m-never-minted-here"])
            .unwrap();
        assert!(
            foreign.is_empty(),
            "neither the unended local one nor the foreign one has announced an end: {foreign:?}"
        );

        // And the scoped form binds nothing for an empty ask rather than building `IN ()`.
        assert!(s.ended_sessions_among(&[]).unwrap().is_empty());
    }

    /// **One read for the newest session of every thread in a set** (fix round 1 of nxf 6j6v.xb24)
    /// — the three properties `orchestration::holder_is_provably_dead` rests on, and the ceiling
    /// its claim area can cross.
    ///
    /// It asks about a CLAIM AREA, which for a channel scope is a whole member fan-out, and it only
    /// wants the last row of each thread. The single-thread reader once per thread was N passes over
    /// a table with no index on `thread`, for N rows — the N+1 this crate has written down next door
    /// as the one not to reintroduce.
    #[test]
    fn the_newest_session_of_each_thread_comes_back_in_one_read() {
        let mut s = ChatStore::open_in_memory(1);
        // `t-resumed` has two: a first session that ended and the one a resume minted after it.
        for (session, thread) in [
            ("m-0001", "t-resumed"),
            ("m-0002", "t-resumed"),
            ("m-0003", "t-quiet"),
        ] {
            s.create_pending_session(session, "coder").unwrap();
            s.record_session_thread(session, Some(thread)).unwrap();
        }
        s.mark_session_ended("m-0001", "2026-09-18T08:00:00Z")
            .unwrap();
        s.mark_session_ended("m-0003", "2026-09-18T08:30:00Z")
            .unwrap();

        let threads: Vec<String> = ["t-resumed", "t-quiet", "t-nothing-ever-ran"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        let newest = s.newest_sessions_in_threads(&threads).unwrap();

        // **The NEWEST row, not any row** — which is the whole point: `t-resumed` has an ended
        // session in its history and a live one now, and reading the history would make it look
        // torn down for ever.
        assert_eq!(newest["t-resumed"].internal_id, "m-0002");
        assert_eq!(newest["t-resumed"].ended, None);
        assert_eq!(newest["t-quiet"].internal_id, "m-0003");
        assert!(newest["t-quiet"].ended.is_some(), "…with its whole row");
        // **A thread nothing ever ran in is ABSENT**, not present-and-empty: what comes back is what
        // was found, exactly as `ended_sessions_among`'s positive set beside it.
        assert!(!newest.contains_key("t-nothing-ever-ran"), "{newest:?}");
        assert_eq!(newest.len(), 2, "{newest:?}");
        // And an empty ask binds nothing rather than building `IN ()`.
        assert!(s.newest_sessions_in_threads(&[]).unwrap().is_empty());
    }

    /// **Past the bound-id ceiling the answer is the same, by the other query** — the branch a
    /// workspace only reaches when it is too big for anybody to notice it answering differently.
    /// `working_tree::threads_owing_an_answer` has this test for its own two strategies, and this
    /// read now sits on the same path with the same ceiling.
    #[test]
    fn the_newest_session_read_answers_the_same_past_the_bound_id_ceiling() {
        let mut s = ChatStore::open_in_memory(1);
        let n = crate::facade::MAX_BOUND_IDS + 1;
        let mut threads: Vec<String> = Vec::with_capacity(n);
        for i in 0..n {
            let thread = format!("t-{i:04}");
            for seq in 0..2 {
                let session = format!("m-{i:04}-{seq}");
                s.create_pending_session(&session, "coder").unwrap();
                s.record_session_thread(&session, Some(&thread)).unwrap();
            }
            s.mark_session_ended(&format!("m-{i:04}-0"), "2026-09-18T08:00:00Z")
                .unwrap();
            threads.push(thread);
        }
        assert!(threads.len() > crate::facade::MAX_BOUND_IDS);

        let whole = s.newest_sessions_in_threads(&threads).unwrap();
        let scoped = s.newest_sessions_in_threads(&threads[..2]).unwrap();

        assert_eq!(whole.len(), n);
        assert_eq!(whole["t-0000"].internal_id, "m-0000-1");
        assert_eq!(
            whole["t-0000"].ended, None,
            "the resume, not the row it replaced"
        );
        assert_eq!(
            scoped["t-0000"], whole["t-0000"],
            "the two strategies are two spellings of one question"
        );
    }

    #[test]
    fn resolve_real_and_session_role_are_none_for_an_unminted_internal_id() {
        let s = ChatStore::open_in_memory(1);
        assert_eq!(s.resolve_real("never-existed").unwrap(), None);
        assert_eq!(s.session_role("never-existed").unwrap(), None);
    }

    // ---- where the session is (nxf 6j6v.a71h §3.1) -------------------------------------------

    #[test]
    fn a_sessions_thread_moves_with_it_and_a_thread_less_trigger_leaves_it_where_it_was() {
        // The mechanical half of the parent rule. A session is wherever it was LAST put in motion:
        // triggered into T-a, resumed on T-b, it is in T-b — which is exactly what makes a71h §3.1's
        // own example come out right (the supervisor sends to #review out of the thread the coder's
        // answer arrived in, not out of the thread it acts on behalf of).
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coding").unwrap();
        assert_eq!(
            s.session_thread("s-1").unwrap(),
            None,
            "a session nobody put in motion opens ROOTS"
        );
        s.record_session_thread("s-1", Some("t-a")).unwrap();
        assert_eq!(s.session_thread("s-1").unwrap().as_deref(), Some("t-a"));
        s.record_session_thread("s-1", Some("t-b")).unwrap();
        assert_eq!(
            s.session_thread("s-1").unwrap().as_deref(),
            Some("t-b"),
            "a position, not a monotonic budget: it moves"
        );
        // …and back, which a `MAX`-style rule (the shape `record_trigger_depth` needs) could not do.
        s.record_session_thread("s-1", Some("t-a")).unwrap();
        assert_eq!(s.session_thread("s-1").unwrap().as_deref(), Some("t-a"));
        // A trigger that names no thread at all leaves it where it was, rather than making the
        // session's next `send --to` a spurious root.
        s.record_session_thread("s-1", None).unwrap();
        assert_eq!(s.session_thread("s-1").unwrap().as_deref(), Some("t-a"));
    }

    #[test]
    fn record_session_thread_for_an_unknown_session_is_a_no_op_not_an_error() {
        // Same contract as `record_trigger_depth`, for the same reason: it rides along with every
        // trigger rather than being a caller's own act, so an id that names no session of ours has
        // nothing to record and nothing to protect.
        let mut s = ChatStore::open_in_memory(1);
        s.record_session_thread("never-existed", Some("t-1"))
            .unwrap();
        assert_eq!(s.session_thread("never-existed").unwrap(), None);
    }

    // ---- the announced session end (nxf 6j6v.10yb) ------------------------------------------

    #[test]
    fn a_session_end_is_recorded_once_and_a_later_announcement_does_not_move_it() {
        // FIRST-WINS, because what reads this reads it as "the process behind this row is gone" and
        // a second stamp says nothing new — a retried teardown, or a resume that ended again, must
        // not rewrite the instant.
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coder").unwrap();
        assert_eq!(s.session_ended_at("s-1").unwrap(), None, "not yet");
        s.mark_session_ended("s-1", "2026-08-24T00:34:27Z").unwrap();
        s.mark_session_ended("s-1", "2026-08-24T09:00:00Z").unwrap();
        assert_eq!(
            s.session_ended_at("s-1").unwrap().as_deref(),
            Some("2026-08-24T00:34:27Z")
        );
    }

    #[test]
    fn a_reopened_session_is_owed_by_its_thread_again_and_can_end_a_second_time() {
        // nxf 6j6v.y1t9, and the seam test the independent review of PR #402 asked for (Code
        // Quality #1 / Test Quality #1): the CONTRACT this method exists for is that an ended
        // session becomes visible to `unended_sessions_in_thread` again — that read is what the
        // liveness gate of nxf 6j6v.10yb consults, and a resume that left the row `ended` would
        // switch the gate off for exactly the sessions a resume creates.
        //
        // The second half is the one a caller would get wrong: `mark_session_ended` is first-wins,
        // so reopening has to make a LATER end land, or a continued session would keep the instant
        // of the turn before it forever.
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coder").unwrap();
        s.record_session_thread("s-1", Some("t-1")).unwrap();
        s.mark_session_ended("s-1", "2026-08-24T00:34:27Z").unwrap();
        assert!(
            s.unended_sessions_in_thread("t-1").unwrap().is_empty(),
            "an ended session is not a candidate for `still writing`"
        );

        s.reopen_session("s-1").unwrap();
        assert_eq!(
            s.unended_sessions_in_thread("t-1").unwrap(),
            vec!["s-1".to_string()],
            "in motion again, so the liveness gate can see it"
        );
        assert_eq!(s.session_ended_at("s-1").unwrap(), None);

        s.mark_session_ended("s-1", "2026-08-24T09:00:00Z").unwrap();
        assert_eq!(
            s.session_ended_at("s-1").unwrap().as_deref(),
            Some("2026-08-24T09:00:00Z"),
            "the SECOND run of this session ends at its own instant, not at the first run's"
        );
    }

    #[test]
    fn reopening_an_unminted_session_is_a_no_op_not_an_error() {
        // The contract every other write on this table keeps, for `record_session_thread`'s reason:
        // this rides along with a trigger rather than being a caller's own act, and there is
        // nothing to protect on a row that does not exist.
        let mut s = ChatStore::open_in_memory(1);
        s.reopen_session("never-existed").unwrap();
        assert_eq!(s.session_ended_at("never-existed").unwrap(), None);
    }

    #[test]
    fn marking_an_unminted_session_ended_is_a_no_op_not_an_error() {
        // `record_session_thread`'s contract, for its reason: this rides along with a teardown
        // rather than being a caller's own act, and there is nothing to protect on a row that does
        // not exist.
        let mut s = ChatStore::open_in_memory(1);
        s.mark_session_ended("never-existed", "2026-08-24T00:34:27Z")
            .unwrap();
        assert_eq!(s.session_ended_at("never-existed").unwrap(), None);
    }

    #[test]
    fn only_the_sessions_that_have_not_announced_an_end_are_listed_for_a_thread() {
        let mut s = ChatStore::open_in_memory(1);
        for (id, thread) in [("s-a", "t-1"), ("s-b", "t-1"), ("s-c", "t-2")] {
            s.create_pending_session(id, "coder").unwrap();
            s.record_session_thread(id, Some(thread)).unwrap();
        }
        assert_eq!(
            s.unended_sessions_in_thread("t-1").unwrap(),
            vec!["s-a".to_string(), "s-b".to_string()],
            "both, in a deterministic order"
        );
        s.mark_session_ended("s-a", "2026-08-24T00:34:27Z").unwrap();
        assert_eq!(
            s.unended_sessions_in_thread("t-1").unwrap(),
            vec!["s-b".to_string()],
            "an ended session is no longer a candidate for `somebody is still writing here`"
        );
        assert_eq!(
            s.unended_sessions_in_thread("t-nobody").unwrap(),
            Vec::<String>::new()
        );
    }

    // ---- the persisted hop depth (nxf 6j6v.m48m) --------------------------------------------

    #[test]
    fn a_freshly_minted_session_starts_at_depth_zero() {
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coding").unwrap();
        assert_eq!(s.session_depth("s-1").unwrap(), Some(0));
    }

    #[test]
    fn session_depth_is_none_for_an_unminted_internal_id() {
        // Distinct from `Some(0)`: "this session is not one we minted" is what the depth guard's
        // fallback branches on, and collapsing it into a real depth of zero would make an unknown
        // id read as a legitimate fresh chain.
        let s = ChatStore::open_in_memory(1);
        assert_eq!(s.session_depth("never-existed").unwrap(), None);
    }

    #[test]
    fn record_trigger_depth_raises_the_depth_but_never_lowers_it() {
        // Monotonic by design (the whole point of the ticket): the depth is a circuit breaker, so a
        // LATER trigger arriving with a smaller claim — a caller that dropped its own ambient
        // context, or a human resuming a session that a deep chain already used — must not hand the
        // chain a fresh budget.
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coding").unwrap();
        s.record_trigger_depth("s-1", 5).unwrap();
        assert_eq!(s.session_depth("s-1").unwrap(), Some(5));
        s.record_trigger_depth("s-1", 2).unwrap();
        assert_eq!(s.session_depth("s-1").unwrap(), Some(5), "never lowered");
        s.record_trigger_depth("s-1", 9).unwrap();
        assert_eq!(s.session_depth("s-1").unwrap(), Some(9), "raised");
    }

    #[test]
    fn record_trigger_depth_for_an_unknown_session_is_a_no_op_not_an_error() {
        // Unlike `bind_session`, this is not a caller's mistake to report: it rides along with every
        // trigger, including the ephemeral synthesizer paths, and a depth recorded against a row
        // that does not exist has nothing to protect. Failing here would turn a bookkeeping miss
        // into a failed trigger.
        let mut s = ChatStore::open_in_memory(1);
        s.record_trigger_depth("never-existed", 3).unwrap();
        assert_eq!(s.session_depth("never-existed").unwrap(), None);
    }

    #[test]
    fn re_minting_a_session_does_not_reset_its_recorded_depth() {
        // `create_pending_session` is `INSERT OR IGNORE`, so a retried setup step must not clobber
        // the depth the same way it must not clobber a binding.
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coding").unwrap();
        s.record_trigger_depth("s-1", 7).unwrap();
        s.create_pending_session("s-1", "coding").unwrap();
        assert_eq!(s.session_depth("s-1").unwrap(), Some(7));
    }

    #[test]
    fn created_is_stamped_from_a_pinned_nxc_now() {
        // The pin is HELD across the assertion and released by `Drop` (see [`NxcNowGuard`]), not
        // removed by hand before it: the two `unwrap`s below USED to sit between a hand-written
        // `set_var`/`remove_var` pair, where either of them panicking would have left `NXC_NOW` set
        // for the rest of this binary.
        let _clock = NxcNowGuard::pin("2026-07-19T00:00:00Z");
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coding").unwrap();
        let created: String = s
            .connection()
            .query_row(
                "SELECT created FROM session_map WHERE internal_id='s-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(created, "2026-07-19T00:00:00Z");
    }
}

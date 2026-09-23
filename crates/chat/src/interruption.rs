//! **A session that stopped because the model went away, and the way back** (nxf 6j6v.npy3).
//!
//! Three causes — an exhausted quota, a provider that cannot be reached, a broken network — and they
//! are one state: *the model is not available to this session for a while, and nobody did anything
//! wrong.* Until this module the engine had no word for it. The classification in the runtime knew
//! two classes (log in / something is broken), so a closed weekly window landed in "something is
//! broken", the teardown posted an escalation in the session's own name saying *"I cannot carry this
//! out"* — which is false — and the operation stopped on NEEDS DECISION with nobody able to restart
//! it.
//!
//! ## The gap this closes is SAYING it, before it is CONTINUING it
//!
//! Measured on 2026-09-12: of thirty sessions in one run, exactly one hit the weekly window. What
//! the engine knew about it afterwards was `state: "ended"` — a clean, ordinary end — and a thread
//! reading `answered`, with `escalated`, `substituted` and `stale` all false. **The fact existed
//! exclusively in the transcript.** The whole channel record of the workspace was searched for a
//! trace of the interruption the owner had watched happen, and there was none.
//!
//! So this table is first and the resume is second. A hold that cannot be read is not a hold.
//!
//! ## Why APPEND-ONLY, and not a column on `session_map`
//!
//! `session_map` carries `ended`, and a resume clears it ([`ChatStore::reopen_session`]). An
//! interruption recorded the same way would be erased by the very act it exists to explain, and
//! "DASS und WANN" — that it happened, and when — would survive exactly until it stopped mattering.
//! A row per interruption keeps the history, and [`ChatStore::current_interruption`] is the one
//! question a resume asks of it. Same shape as [`crate::park::ParkedWork`] next door, and for the
//! same reason: *parked twice and came back twice* is the history somebody debugging needs.
//!
//! ## What it is NOT: a state of the session
//!
//! An interrupted session HAS ended — its process is gone, `nxc session ended` was called, and
//! [`crate::orchestration::SessionState::Ended`] is true of it. The interruption is a fact BESIDE
//! that, exactly as `substituted` and `waiting_on_sub_round` are facts beside a thread's state
//! rather than states of their own. That is why nothing here adds a `SessionState` variant: it would
//! give one word two meanings, and break every host matching on an enum that says what a process is
//! doing.
//!
//! It does carry one consequence that a reader sees immediately, and it is a correction rather than
//! an addition: `nxc status` may not say *"ITS SESSION HAS ENDED, nothing is coming"* over a session
//! that is coming back. That is the same contradiction nxf 6j6v.hw2t had to remove from that line
//! for a waiting caller, arriving from a second direction.

use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::store::ChatStore;

/// **One interruption** — a session stopped at an availability boundary, as the store keeps it.
///
/// Every field but [`resumed_at`](Interruption::resumed_at) is stated by the RUNTIME, through
/// `nxc session interrupted`, and none of it is inferred here: the engine is not in a position to
/// know why a provider refused a session, and a guess would be the thing this record exists to
/// replace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Interruption {
    /// The row id — what [`ChatStore::mark_interruption_resumed`] names.
    pub id: i64,
    /// The internal session that stopped.
    pub session: String,
    /// When it stopped.
    pub at: String,
    /// **Which boundary** — the runtime's own name for the window: `seven_day`, `five_hour`, or the
    /// error class (`rate_limit`, `overloaded`) where it named no window.
    ///
    /// An enumerated value from the runtime rather than free text, which is what lets a reader tell
    /// a five-hour wait from a week-long one without reading a sentence.
    pub limit: String,
    /// **When the boundary falls**, RFC3339, or `None` when the runtime stated no instant.
    ///
    /// `None` is a real answer and is never filled in with a guess: what it costs is the AUTOMATIC
    /// way back — there is no moment to arm one for — and what it leaves is `nxc resume --thread`,
    /// which a human runs when they know better than the record does. Inventing an instant would
    /// start a session at a moment nobody stated.
    pub until: Option<String>,
    /// The runtime's own sentence about it, for a human reading a line — *"You've hit your weekly
    /// limit · resets 2pm (Europe/Berlin)"*. Capped by the runtime before it ever gets here.
    pub detail: String,
    /// When the operation was taken up again, or `None` while it is still on hold. The row STAYS
    /// either way — see the module docs on why this is append-only.
    pub resumed_at: Option<String>,
}

impl Interruption {
    /// **Is this hold still standing?** The one question every reader of the current row asks, so
    /// it is asked in one place rather than spelled as an `is_none` at each of them.
    pub fn on_hold(&self) -> bool {
        self.resumed_at.is_none()
    }

    /// **Has the boundary fallen by `now`?** — what decides whether the way back is open.
    ///
    /// A hold with no stated instant answers `false` FOREVER, and that is the honest direction: an
    /// automatic return may only fire at a moment the runtime named, and nothing here knows when an
    /// unnamed window lifts. The human's verb does not consult this at all — a person resuming by
    /// hand has a reason the record does not (another account, a lifted limit, plain impatience),
    /// and second-guessing them here would make the manual way back useless exactly when it is
    /// wanted.
    ///
    /// String comparison, which is correct for RFC3339 in UTC and is what every other deadline in
    /// this crate compares on ([`crate::working_tree::to_utc_rfc3339`] is what normalises both
    /// sides before they are stored).
    pub fn is_due(&self, now: &str) -> bool {
        self.until.as_deref().is_some_and(|until| until <= now)
    }
}

/// The columns, in the one order [`interruption_from_row`] reads them back in — written once so the
/// three statements that hand it a row cannot drift into three different column orders.
const INTERRUPTION_COLS: &str = "id, session, at, limit_name, until, detail, resumed_at";

fn interruption_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Interruption> {
    Ok(Interruption {
        id: r.get(0)?,
        session: r.get(1)?,
        at: r.get(2)?,
        limit: r.get(3)?,
        until: r.get(4)?,
        detail: r.get(5)?,
        resumed_at: r.get(6)?,
    })
}

impl ChatStore {
    /// **Record that a session stopped at an availability boundary.**
    ///
    /// **Idempotent while the hold stands**, on the session alone: a teardown that is retried, or a
    /// runtime that announces twice, must not leave one interruption looking like two — a reader
    /// counting rows would report an operation as having been knocked over repeatedly when the
    /// model went away once. The FIRST announcement wins and keeps its instant, for exactly
    /// [`ChatStore::mark_session_ended`]'s reason: what a later stamp would say is nothing new.
    ///
    /// **ONE STATEMENT, and the database enforces the rule** (independent review of PR #476,
    /// Integrity & Robustness #1). This was a check-then-insert — read whether a hold stands, insert
    /// if not — and two callers interleaving between those two statements would both read "none"
    /// and both insert. `ON CONFLICT DO NOTHING` against the partial unique index
    /// `session_interruption_one_open` (`schema.rs`) makes the loser's insert a no-op instead of a
    /// duplicate, which is the same shape [`ChatStore::record_operation_base`] next door already
    /// uses and the same discipline [`mark_interruption_resumed`](ChatStore::
    /// mark_interruption_resumed) has always had as a single conditional `UPDATE`.
    ///
    /// A session that was resumed and interrupted AGAIN appends a new row, which is the case the
    /// list exists for: a week-long window can close twice over one piece of work, and the second
    /// time is not the first time. The index is PARTIAL for exactly that reason.
    ///
    /// Returns the row id of the hold now standing — the fresh one, or the one already there. The
    /// read-back is what makes that true for the loser of a race: `last_insert_rowid` after a
    /// do-nothing conflict names some earlier row, so it is never used to answer this.
    pub fn record_interruption(
        &mut self,
        session: &str,
        limit: &str,
        until: Option<&str>,
        detail: &str,
        now: &str,
    ) -> Result<i64> {
        self.connection().execute(
            "INSERT INTO session_interruption(session, at, limit_name, until, detail)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO NOTHING",
            params![session, now, limit, until, detail],
        )?;
        // Read back rather than trusting `last_insert_rowid`: on a conflict nothing was inserted and
        // that function still answers with whatever this connection last wrote, which would be a row
        // belonging to another session entirely.
        self.current_interruption(session)?
            .map(|open| open.id)
            .ok_or_else(|| {
                crate::error::NxfError::io(format!(
                    "the hold for session {session} was neither inserted nor found standing"
                ))
            })
    }

    /// **The hold standing over `session` right now**, or `None` — the read a resume starts from.
    ///
    /// Newest first and unresumed only. `None` covers three things that are one thing to a caller:
    /// a session that was never interrupted, one whose interruptions have all been taken up, and an
    /// id this workspace never minted. All three mean *there is nothing on hold here*, which is
    /// what the caller is asking.
    pub fn current_interruption(&self, session: &str) -> Result<Option<Interruption>> {
        Ok(self
            .connection()
            .query_row(
                &format!(
                    "SELECT {INTERRUPTION_COLS} FROM session_interruption
                      WHERE session = ?1 AND resumed_at IS NULL
                      ORDER BY id DESC LIMIT 1"
                ),
                params![session],
                interruption_from_row,
            )
            .optional()?)
    }

    /// **Every hold standing in this workspace**, oldest first — what the background sweep walks,
    /// and what `nxc status` would need if it ever asked the question workspace-wide.
    ///
    /// Oldest first, deliberately unlike [`current_interruption`](ChatStore::current_interruption)'s
    /// newest-first: this is a WORK LIST, and the session that has been waiting longest is the one
    /// to take up first.
    pub fn standing_interruptions(&self) -> Result<Vec<Interruption>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {INTERRUPTION_COLS} FROM session_interruption
              WHERE resumed_at IS NULL ORDER BY id",
        ))?;
        let rows = st
            .query_map([], interruption_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// **Which THREADS have a session on hold right now** (fix round 1 of nxf 6j6v.xb24) — the
    /// standing list asked the way the working-copy rules ask it, in one statement.
    ///
    /// `orchestration::holder_is_provably_dead` declines on any hold standing anywhere in the claim
    /// area it is judging. It used to ask that by walking [`standing_interruptions`](ChatStore::
    /// standing_interruptions) and resolving each hold's session to its thread one row at a time —
    /// an N+1 over `session_map`, which has no index on the column it was looking at. The join
    /// answers it once, and the caller intersects the answer with its own claim area.
    ///
    /// **The same answer as the walk, including its silent arm.** A hold whose session this
    /// workspace never minted, or whose session has never been put in motion on a thread, names no
    /// thread and so is in nobody's claim area — the `INNER JOIN` plus `thread IS NOT NULL` is
    /// exactly the `continue` the walk did.
    ///
    /// Whole-table and unbounded on purpose, unlike [`interruptions_among`](ChatStore::
    /// interruptions_among) beside it: what bounds this read is how many holds are STANDING, which
    /// is the bound `standing_interruptions` already had, and there is no id list to bind.
    pub(crate) fn interrupted_threads(&self) -> Result<std::collections::BTreeSet<String>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT m.thread FROM session_interruption i
               JOIN session_map m ON m.internal_id = i.session
              WHERE i.resumed_at IS NULL AND m.thread IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// **The holds standing over an explicit SET of sessions** — the bulk form, for the one read
    /// that carries a whole board.
    ///
    /// It exists for [`crate::facade::status`]'s reason and by its rule: a status report names a
    /// session per thread, and asking the single-row reader once per thread would put an N+1 back
    /// into the read this crate measures for not having one. Same contract as
    /// [`ChatStore::ended_sessions_among`] beside it — the caller keeps the id list under SQLite's
    /// variable ceiling and falls back to [`standing_interruptions`](ChatStore::
    /// standing_interruptions) above it.
    pub fn interruptions_among(
        &self,
        sessions: &[&str],
    ) -> Result<std::collections::BTreeMap<String, Interruption>> {
        if sessions.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        let placeholders = vec!["?"; sessions.len()].join(",");
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {INTERRUPTION_COLS} FROM session_interruption
              WHERE resumed_at IS NULL AND session IN ({placeholders})
              ORDER BY id"
        ))?;
        let rows = st
            .query_map(rusqlite::params_from_iter(sessions.iter()), |r| {
                interruption_from_row(r).map(|i| (i.session.clone(), i))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Ascending id, so a session holding two unresumed rows — which `record_interruption`
        // prevents, but a hand-edited db does not — settles on the NEWEST, matching
        // `current_interruption`'s answer for the same session rather than disagreeing with it.
        Ok(rows.into_iter().collect())
    }

    /// Stamp a hold as taken up. The row STAYS — see the module docs.
    ///
    /// Returns whether this call is the one that stamped it, which is what lets two racing ways
    /// back (the background service's and a human's) tell *I took this up* from *somebody else
    /// already had*.
    pub fn mark_interruption_resumed(&mut self, id: i64, now: &str) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE session_interruption SET resumed_at = ?2 WHERE id = ?1 AND resumed_at IS NULL",
            params![id, now],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-09-12T09:53:52Z";

    fn store() -> ChatStore {
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("m-s1", "coder").unwrap();
        s.create_pending_session("m-s2", "reviewer").unwrap();
        s
    }

    #[test]
    fn a_hold_is_recorded_with_everything_the_runtime_stated() {
        let mut s = store();
        let id = s
            .record_interruption(
                "m-s1",
                "seven_day",
                Some("2026-09-14T12:00:00Z"),
                "You've hit your weekly limit",
                NOW,
            )
            .unwrap();
        let open = s.current_interruption("m-s1").unwrap().unwrap();
        assert_eq!(open.id, id);
        assert_eq!(open.session, "m-s1");
        assert_eq!(open.at, NOW);
        assert_eq!(open.limit, "seven_day");
        assert_eq!(open.until.as_deref(), Some("2026-09-14T12:00:00Z"));
        assert_eq!(open.detail, "You've hit your weekly limit");
        assert!(open.on_hold());
    }

    /// **Which THREADS have a session on hold, in one read** (fix round 1 of nxf 6j6v.xb24).
    ///
    /// `orchestration::holder_is_provably_dead` declines on any hold standing anywhere in the claim
    /// area, and it used to ask that by walking the whole standing list and resolving each hold's
    /// session to its thread one row at a time. The join answers it once, and answers it the same
    /// way: a hold whose session this workspace cannot place names no thread and is nobody's.
    #[test]
    fn the_threads_with_a_standing_hold_come_back_in_one_read() {
        let mut s = store();
        s.record_session_thread("m-s1", Some("t-held")).unwrap();
        s.record_session_thread("m-s2", Some("t-resumed")).unwrap();
        // A hold on a session this workspace never minted — it names no thread and is not this
        // area's, which is the arm the per-row walk answered with a `continue`.
        s.record_interruption("m-foreign", "five_hour", None, "elsewhere", NOW)
            .unwrap();
        s.record_interruption("m-s1", "five_hour", Some("2026-09-12T14:00:00Z"), "a", NOW)
            .unwrap();
        let resumed = s
            .record_interruption("m-s2", "seven_day", None, "b", NOW)
            .unwrap();

        let held = s.interrupted_threads().unwrap();
        assert_eq!(
            held.iter().map(String::as_str).collect::<Vec<_>>(),
            ["t-held", "t-resumed"],
            "every thread with a STANDING hold, whatever that hold's window says"
        );

        // …and a hold that has been taken up stops naming its thread, exactly as it stops standing.
        s.mark_interruption_resumed(resumed, "2026-09-12T10:00:00Z")
            .unwrap();
        assert_eq!(
            s.interrupted_threads()
                .unwrap()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["t-held"]
        );
    }

    #[test]
    fn a_session_that_was_never_interrupted_has_no_hold() {
        let s = store();
        assert_eq!(s.current_interruption("m-s1").unwrap(), None);
        // …and neither has an id this workspace never minted, which is the same answer on purpose:
        // both mean "there is nothing on hold here", which is what the caller asked.
        assert_eq!(s.current_interruption("m-nobody").unwrap(), None);
    }

    #[test]
    fn announcing_the_same_hold_twice_records_it_once() {
        // A retried teardown must not make one closed window look like two.
        let mut s = store();
        let first = s
            .record_interruption("m-s1", "seven_day", Some("2026-09-14T12:00:00Z"), "a", NOW)
            .unwrap();
        let again = s
            .record_interruption(
                "m-s1",
                "five_hour",
                Some("2026-09-13T00:00:00Z"),
                "b",
                "2026-09-12T10:00:00Z",
            )
            .unwrap();
        assert_eq!(
            first, again,
            "the second announcement reuses the standing row"
        );
        let open = s.current_interruption("m-s1").unwrap().unwrap();
        assert_eq!(
            open.limit, "seven_day",
            "the FIRST announcement's terms win"
        );
        assert_eq!(open.at, NOW);
        assert_eq!(s.standing_interruptions().unwrap().len(), 1);
    }

    #[test]
    fn a_session_interrupted_again_after_a_resume_gets_its_own_row() {
        // A week-long window can close twice over one piece of work, and the second time is not
        // the first time.
        let mut s = store();
        let first = s
            .record_interruption("m-s1", "seven_day", None, "a", NOW)
            .unwrap();
        assert!(s
            .mark_interruption_resumed(first, "2026-09-14T12:00:00Z")
            .unwrap());
        let second = s
            .record_interruption("m-s1", "five_hour", None, "b", "2026-09-14T13:00:00Z")
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(s.current_interruption("m-s1").unwrap().unwrap().id, second);
        // The history is still there — that is the whole reason this is a table and not a column.
        assert_eq!(s.standing_interruptions().unwrap().len(), 1);
    }

    #[test]
    fn taking_a_hold_up_is_first_wins_so_two_ways_back_cannot_both_claim_it() {
        // The background service and a human can both arrive; exactly one of them started it.
        let mut s = store();
        let id = s
            .record_interruption("m-s1", "seven_day", None, "a", NOW)
            .unwrap();
        assert!(s
            .mark_interruption_resumed(id, "2026-09-14T12:00:00Z")
            .unwrap());
        assert!(!s
            .mark_interruption_resumed(id, "2026-09-14T12:00:05Z")
            .unwrap());
        assert_eq!(s.current_interruption("m-s1").unwrap(), None);
        // The row stays, with the instant the winner stamped.
        assert!(!s
            .standing_interruptions()
            .unwrap()
            .iter()
            .any(|i| i.id == id));
    }

    #[test]
    fn the_work_list_is_oldest_first_and_the_single_read_is_newest_first() {
        let mut s = store();
        s.record_interruption("m-s1", "seven_day", None, "a", NOW)
            .unwrap();
        s.record_interruption("m-s2", "five_hour", None, "b", NOW)
            .unwrap();
        let list = s.standing_interruptions().unwrap();
        assert_eq!(
            list.iter().map(|i| i.session.as_str()).collect::<Vec<_>>(),
            vec!["m-s1", "m-s2"],
            "the session waiting longest is the one to take up first"
        );
    }

    #[test]
    fn the_bulk_read_answers_for_a_set_and_says_nothing_about_the_rest() {
        let mut s = store();
        s.record_interruption("m-s1", "seven_day", None, "a", NOW)
            .unwrap();
        let found = s.interruptions_among(&["m-s1", "m-s2", "m-never"]).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found["m-s1"].limit, "seven_day");
        assert!(s.interruptions_among(&[]).unwrap().is_empty());
    }

    #[test]
    fn the_bulk_read_and_the_single_read_never_disagree_about_one_session() {
        let mut s = store();
        let id = s
            .record_interruption("m-s1", "seven_day", None, "a", NOW)
            .unwrap();
        assert_eq!(
            s.interruptions_among(&["m-s1"]).unwrap()["m-s1"],
            s.current_interruption("m-s1").unwrap().unwrap()
        );
        s.mark_interruption_resumed(id, NOW).unwrap();
        assert!(s.interruptions_among(&["m-s1"]).unwrap().is_empty());
    }

    #[test]
    fn a_hold_with_a_stated_instant_falls_due_at_it() {
        let mut s = store();
        s.record_interruption("m-s1", "seven_day", Some("2026-09-14T12:00:00Z"), "a", NOW)
            .unwrap();
        let open = s.current_interruption("m-s1").unwrap().unwrap();
        assert!(!open.is_due("2026-09-14T11:59:59Z"));
        assert!(open.is_due("2026-09-14T12:00:00Z"), "due AT the instant");
        assert!(open.is_due("2026-09-20T00:00:00Z"));
    }

    #[test]
    fn a_hold_with_no_stated_instant_never_falls_due_on_its_own() {
        // The honest direction: an automatic return may only fire at a moment the runtime named.
        // What is left is the human's verb, which does not consult this at all.
        let mut s = store();
        s.record_interruption("m-s1", "rate_limit", None, "a", NOW)
            .unwrap();
        let open = s.current_interruption("m-s1").unwrap().unwrap();
        assert!(!open.is_due("2030-01-01T00:00:00Z"));
    }
}

//! **The ladder: ask, continue, re-commission, escalate** (nxf 6j6v.558h, cut 1d of epic 6j6v.nmc9,
//! section B).
//!
//! Every coordinated slot (`stranded::coordinator_of`) is taken up by its coordinator — the engine's
//! tick on the thread that opened it — before anybody is asked. The stranded sweep's look decides
//! WHAT it sees ([`crate::stranded`]'s `judge`); this module decides what to DO about it, one rung
//! per look, under a claim:
//!
//! - **A — ask.** A live session with no activity on its slot for [`SILENCE_SECS`] is asked through
//!   the ask door ([`crate::worker::Worker::ask_session`]) with the one text that offers
//!   `nxc reply --progress`.
//! - **B — hold.** A slot whose session is on hold is the resume's; the ladder takes up only the
//!   holds nothing will: a resume the provider refused (→ D), a hold that names no instant and
//!   waits on nobody (→ C one grace after it was taken).
//! - **C — continue.** A session that ended without answering, bound and with work of its own on
//!   the record, is continued in the SAME session with a reminder that names the thread and the
//!   task.
//! - **D — re-commission.** A slot that never got a session, whose start was given up, whose
//!   session ended with nothing written, or whose resume the provider refused, gets a fresh session
//!   with its ORIGINAL body, minted into the same slot.
//! - **E — escalate.** At [`MAX_RECOVERY_ATTEMPTS`], at the wall-clock ceiling, or on a permanent
//!   refusal, the coordinator escalates in the member's name with what it knows — the step, the
//!   cause, every attempt, the last activity, the session and the lease — and the escalation climbs
//!   one level per round through the existing path.
//!
//! **The budget is one counter** — asks, continuations and re-commissions together — reset by a
//! genuine answer (the row goes) and by a `--progress` that was asked for; the resets themselves are
//! bounded by the ceiling, counted from the episode's first attempt.
//!
//! **The claim** makes one action at a time per slot: taken before any side effect, renewed while
//! the trigger runs, released whatever the trigger returned. Inside it, fail-closed: the recheck,
//! then the attempt, the log row, the lease hook — THEN the trigger.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::error::Result;
use crate::orchestration::{ConsequenceClass, Ctx, FailedConsequence};
use crate::store::ChatStore;
use crate::stranded::{
    after, earlier, outstanding_has, reached, CoordinatorRef, Judgement, Observation, OwedStart,
    STRANDED_GRACE_SECS,
};
use crate::worker::{AskOutcome, SessionAsk, TriggerError};

/// **Silence on a slot that earns an ask** — 20 minutes with no transcript activity attributed to
/// the slot, no progress and no ask (owner guideline B of 2026-10-23).
pub const SILENCE_SECS: i64 = 1_200;
/// **How long a claim holds** before another look may take the slot — a rung's trigger is a process
/// spawn, seconds; renewed every half of it while the trigger runs.
pub const CLAIM_SECS: i64 = 60;
/// **How long a rung in flight blocks a second one** — the recheck refuses while the slot's newest
/// log row has no outcome and is younger than this.
pub const TRIGGER_TIMEOUT_SECS: i64 = 600;
/// **The budget** — asks, continuations and re-commissions together.
pub const MAX_RECOVERY_ATTEMPTS: u32 = 3;
/// The wall-clock ceiling's floor: two hours.
pub const RECOVERY_CEILING_FLOOR_SECS: i64 = 7_200;
/// The wall-clock ceiling's cap: six hours.
pub const RECOVERY_CEILING_CAP_SECS: i64 = 21_600;
/// The bound of a child round a session opened with its own `nxc send`: never less than this…
pub const CHILD_WINDOW_FLOOR_SECS: i64 = 1_800;
/// …and never more than this, whatever the parent has left.
pub const CHILD_WINDOW_CAP_SECS: i64 = 7_200;

/// **The ceiling of one episode** — `clamp(3 × declared window, 2 h, 6 h)`.
pub fn recovery_ceiling_secs(window_secs: Option<i64>) -> i64 {
    window_secs.map_or(RECOVERY_CEILING_FLOOR_SECS, |w| {
        w.saturating_mul(3)
            .clamp(RECOVERY_CEILING_FLOOR_SECS, RECOVERY_CEILING_CAP_SECS)
    })
}

/// **The bound a child round inherits** — `clamp(parent_remaining_window, 30 min, 2 h)`.
pub fn child_window_secs(parent_remaining_secs: i64) -> i64 {
    parent_remaining_secs.clamp(CHILD_WINDOW_FLOOR_SECS, CHILD_WINDOW_CAP_SECS)
}

/// **What one rung of the ladder did** — the `action` of its log row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecoveryAction {
    /// Rung A: the running session was asked.
    Asked,
    /// Rung C: the session was continued.
    Continued,
    /// Rung D: a fresh session was started with the original body.
    Recommissioned,
    /// Rung E: the coordinator escalated.
    Escalated,
    /// An asked-for `--progress` reset the budget.
    Progress,
}

impl RecoveryAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RecoveryAction::Asked => "asked",
            RecoveryAction::Continued => "continued",
            RecoveryAction::Recommissioned => "recommissioned",
            RecoveryAction::Escalated => "escalated",
            RecoveryAction::Progress => "progress",
        }
    }

    fn parse(raw: &str) -> Option<RecoveryAction> {
        Some(match raw {
            "asked" => RecoveryAction::Asked,
            "continued" => RecoveryAction::Continued,
            "recommissioned" => RecoveryAction::Recommissioned,
            "escalated" => RecoveryAction::Escalated,
            "progress" => RecoveryAction::Progress,
            _ => return None,
        })
    }
}

/// **One row of the ladder's record.**
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryLogRow {
    pub id: i64,
    pub thread: String,
    pub role: String,
    pub at: String,
    pub action: String,
    pub attempt: u32,
    pub session: Option<String>,
    /// `None` while the rung is in flight.
    pub outcome: Option<String>,
    pub detail: String,
}

/// What taking a slot's claim found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlotClaim {
    /// This caller holds it until the instant named.
    Claimed,
    /// Another look holds it; the earliest moment to try again.
    BeingRecovered { retry_at: String },
    /// The slot has no row: nothing to claim.
    NoRow,
}

const LOG_COLS: &str = "id, thread, role, at, action, attempt, session, outcome, detail";

fn log_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RecoveryLogRow> {
    let attempt: i64 = r.get(5)?;
    Ok(RecoveryLogRow {
        id: r.get(0)?,
        thread: r.get(1)?,
        role: r.get(2)?,
        at: r.get(3)?,
        action: r.get(4)?,
        attempt: u32::try_from(attempt).unwrap_or(0),
        session: r.get(6)?,
        outcome: r.get(7)?,
        detail: r.get(8)?,
    })
}

impl ChatStore {
    /// **Take the slot's claim** — one action at a time per slot, across processes: the read and the
    /// write are one IMMEDIATE transaction, so two looks racing on two connections serialise on the
    /// database's write lock and exactly one of them sees the claim free. Instants are compared in
    /// Rust, never as text.
    pub(crate) fn claim_slot(&mut self, thread: &str, role: &str, now: &str) -> Result<SlotClaim> {
        let until = after(now, CLAIM_SECS, "the slot's claim")?;
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection(),
            TransactionBehavior::Immediate,
        )?;
        let held: Option<Option<String>> = tx
            .query_row(
                "SELECT recovering_until FROM owed_starts WHERE thread = ?1 AND role = ?2",
                params![thread, role],
                |r| r.get(0),
            )
            .optional()?;
        let Some(held) = held else {
            return Ok(SlotClaim::NoRow);
        };
        if let Some(held) = held.filter(|u| !earlier(u, now)) {
            return Ok(SlotClaim::BeingRecovered { retry_at: held });
        }
        tx.execute(
            "UPDATE owed_starts SET recovering_until = ?3 WHERE thread = ?1 AND role = ?2",
            params![thread, role, until],
        )?;
        tx.commit()?;
        Ok(SlotClaim::Claimed)
    }

    /// Release the slot's claim, whatever the rung under it did.
    pub(crate) fn release_slot_claim(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET recovering_until = NULL WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// The claim's renewal horizon when another look holds it at `now` — what `nxc resume` answers
    /// `BeingRecovered` with. `None` when the slot is free or has no row.
    pub(crate) fn slot_claim_standing(
        &self,
        thread: &str,
        role: &str,
        now: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT recovering_until FROM owed_starts WHERE thread = ?1 AND role = ?2",
                params![thread, role],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .filter(|until| !earlier(until, now)))
    }

    /// **Count one attempt** — and start the episode's ceiling clock at its first. Returns the count.
    pub(crate) fn begin_attempt(&mut self, thread: &str, role: &str, now: &str) -> Result<u32> {
        let conn = self.connection();
        conn.execute(
            "UPDATE owed_starts
                SET attempts = attempts + 1,
                    recovery_started_at = COALESCE(recovery_started_at, ?3)
              WHERE thread = ?1 AND role = ?2",
            params![thread, role, now],
        )?;
        let n: i64 = conn.query_row(
            "SELECT attempts FROM owed_starts WHERE thread = ?1 AND role = ?2",
            params![thread, role],
            |r| r.get(0),
        )?;
        Ok(u32::try_from(n).unwrap_or(0))
    }

    /// **Take back an attempt that did nothing** — `AlreadyRunning`, `NoProcess`, `CannotAsk`.
    /// `opened_the_episode` clears the ceiling clock the attempt started.
    pub(crate) fn revert_attempt(
        &mut self,
        thread: &str,
        role: &str,
        opened_the_episode: bool,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts
                SET attempts = MAX(attempts - 1, 0),
                    recovery_started_at = CASE WHEN ?3 THEN NULL ELSE recovery_started_at END
              WHERE thread = ?1 AND role = ?2",
            params![thread, role, opened_the_episode],
        )?;
        Ok(())
    }

    /// Append one row to the ladder's record; `outcome` is NULL until [`settle_rung`].
    ///
    /// [`settle_rung`]: ChatStore::settle_rung
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn log_rung(
        &mut self,
        thread: &str,
        role: &str,
        now: &str,
        action: RecoveryAction,
        attempt: u32,
        session: Option<&str>,
        detail: &str,
    ) -> Result<i64> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO recovery_log(thread, role, at, action, attempt, session, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                thread,
                role,
                now,
                action.as_str(),
                i64::from(attempt),
                session,
                detail
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Write what became of a rung.
    pub(crate) fn settle_rung(&mut self, id: i64, outcome: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE recovery_log SET outcome = ?2 WHERE id = ?1",
            params![id, outcome],
        )?;
        Ok(())
    }

    /// The ladder's record of one slot, oldest first.
    pub(crate) fn slot_recovery_log(
        &self,
        thread: &str,
        role: &str,
    ) -> Result<Vec<RecoveryLogRow>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {LOG_COLS} FROM recovery_log WHERE thread = ?1 AND role = ?2 ORDER BY id"
        ))?;
        let rows = st
            .query_map(params![thread, role], log_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The newest row of every slot among `threads` — `status`'s one read.
    pub(crate) fn newest_recovery_by_threads(
        &self,
        threads: &[&str],
    ) -> Result<std::collections::BTreeMap<(String, String), RecoveryLogRow>> {
        if threads.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        let placeholders = vec!["?"; threads.len()].join(",");
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {LOG_COLS} FROM recovery_log WHERE thread IN ({placeholders}) ORDER BY id"
        ))?;
        let rows = st
            .query_map(rusqlite::params_from_iter(threads.iter()), log_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|r| ((r.thread.clone(), r.role.clone()), r))
            .collect())
    }

    /// **Retention** of the ladder's record: drop the rows older than `cutoff`, compared as
    /// instants; and the obligations kept for queued triggers that are no longer in the queue.
    pub(crate) fn forget_recovery_before(&mut self, cutoff: &str) -> Result<usize> {
        let conn = self.connection();
        let rows: Vec<(i64, String)> = {
            let mut st = conn.prepare("SELECT id, at FROM recovery_log")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut gone = 0;
        for (id, _) in rows.iter().filter(|(_, at)| earlier(at, cutoff)) {
            gone += conn.execute("DELETE FROM recovery_log WHERE id = ?1", [id])?;
        }
        conn.execute(
            "DELETE FROM queued_obligations WHERE NOT EXISTS (
                 SELECT 1 FROM working_tree_queue q
                  WHERE q.session = queued_obligations.session
                    AND q.thread = queued_obligations.thread)",
            [],
        )?;
        Ok(gone)
    }

    /// Mark the slot asked at `now`.
    pub(crate) fn set_asked(&mut self, thread: &str, role: &str, now: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET asked_at = ?3 WHERE thread = ?1 AND role = ?2",
            params![thread, role, now],
        )?;
        Ok(())
    }

    /// The instant the next look is due, for `status`.
    pub(crate) fn set_next_look(&mut self, thread: &str, role: &str, at: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET next_look = ?3 WHERE thread = ?1 AND role = ?2",
            params![thread, role, at],
        )?;
        Ok(())
    }

    /// The next look `status` names, when one was recorded.
    pub(crate) fn next_look(&self, thread: &str, role: &str) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT next_look FROM owed_starts WHERE thread = ?1 AND role = ?2",
                params![thread, role],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// **Take a `--progress` as the sign of life an ask asked for** — when an ask is pending on the
    /// slot (`asked_at` set): the silence clock restarts from `now`, the budget goes back to zero and
    /// the ask is answered. The ceiling clock is left standing. `false` when no ask was pending.
    pub(crate) fn take_progress(&mut self, thread: &str, role: &str, now: &str) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE owed_starts SET last_progress_at = ?3, attempts = 0, asked_at = NULL
              WHERE thread = ?1 AND role = ?2 AND asked_at IS NOT NULL",
            params![thread, role, now],
        )?;
        Ok(n > 0)
    }

    /// **The episode is over** — rung E's record, or a commissioner's reply after it: the budget and
    /// the ceiling clock start again from nothing.
    pub(crate) fn end_recovery_episode(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts
                SET attempts = 0, recovery_started_at = NULL, asked_at = NULL,
                    last_progress_at = NULL, next_look = NULL
              WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// The coordinator's summary on a stranded record, when the ladder wrote one.
    pub(crate) fn stranded_summary(
        &self,
        rec: &crate::stranded::StrandedRecord,
    ) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT summary FROM stranded_sessions
                  WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3",
                params![rec.thread, rec.role, rec.trigger_clock],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    pub(crate) fn set_stranded_summary(
        &mut self,
        rec: &crate::stranded::StrandedRecord,
        summary: &str,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE stranded_sessions SET summary = ?4
              WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3",
            params![rec.thread, rec.role, rec.trigger_clock, summary],
        )?;
        Ok(())
    }

    /// **The cause a session's end announced** (`nxc session ended --cause --attempts`), kept on its
    /// row until the next end says otherwise.
    pub(crate) fn record_session_cause(
        &mut self,
        session: &str,
        cause: Option<&str>,
        attempts: Option<u32>,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE session_map SET last_cause = ?2, last_cause_attempts = ?3 WHERE internal_id = ?1",
            params![session, cause, attempts.map(i64::from)],
        )?;
        Ok(())
    }

    /// The cause the session's last end announced, when it announced one.
    pub fn session_last_cause(&self, session: &str) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT last_cause FROM session_map WHERE internal_id = ?1",
                [session],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// **The work a session wrote** — its transcript entries that are the AGENT's: not the
    /// runtime's own bookkeeping (`session_init`, `result`, `error`) and not the sidecar's
    /// `turn_start` markers. Zero is a session that never got to do anything — a runtime that never
    /// ran it binds a session id and writes its `session_init` and an error `result` all the same,
    /// which is why the rung after its end cannot be chosen by "bound" or by "any entry" (must-fix,
    /// design review round 2 CQ #5 / IR #5 / TQ #6). A kind a newer sidecar writes counts as work:
    /// the slot is then continued, the direction that redoes nothing.
    pub(crate) fn work_entry_count(&self, session: &str) -> Result<i64> {
        Ok(self.connection().query_row(
            "SELECT COUNT(*) FROM agent_transcript
              WHERE internal_session = ?1
                AND kind NOT IN ('turn_start', 'session_init', 'result', 'error')",
            [session],
            |r| r.get(0),
        )?)
    }

    /// **The last activity of `session` attributed to the slot on `thread`** (must-fix, design
    /// review round 3 CQ #6, round 4 CQ #1 / TQ #3 / IR #5): the newest instant among the entries
    /// from the newest `turn_start { thread }` of THAT slot up to the next `turn_start` of any other
    /// slot. `None` when the slot has no turn of its own on record — its silence is then counted from
    /// its trigger.
    pub(crate) fn slot_last_activity(&self, session: &str, thread: &str) -> Result<Option<String>> {
        let conn = self.connection();
        let start: Option<i64> = conn.query_row(
            "SELECT MAX(seq) FROM agent_transcript
              WHERE internal_session = ?1 AND kind = 'turn_start'
                AND json_extract(data, '$.thread') = ?2",
            params![session, thread],
            |r| r.get(0),
        )?;
        let Some(start) = start else {
            return Ok(None);
        };
        let end: Option<i64> = conn.query_row(
            "SELECT MIN(seq) FROM agent_transcript
              WHERE internal_session = ?1 AND kind = 'turn_start' AND seq > ?2
                AND COALESCE(json_extract(data, '$.thread'), '') <> ?3",
            params![session, start, thread],
            |r| r.get(0),
        )?;
        let mut st = conn.prepare(
            "SELECT at FROM agent_transcript
              WHERE internal_session = ?1 AND seq >= ?2 AND (?3 IS NULL OR seq < ?3)
                AND at IS NOT NULL",
        )?;
        let ats = st
            .query_map(params![session, start, end], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(latest(ats.iter().map(String::as_str)).map(str::to_string))
    }

    /// The newest hold of `session`, standing or not, with the outcome its resume recorded.
    pub(crate) fn latest_hold(&self, session: &str) -> Result<Option<HoldOutcome>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT id, at, until, resumed_at, outcome FROM session_interruption
                  WHERE session = ?1 ORDER BY id DESC LIMIT 1",
                [session],
                |r| {
                    Ok(HoldOutcome {
                        id: r.get(0)?,
                        at: r.get(1)?,
                        until: r.get(2)?,
                        resumed_at: r.get(3)?,
                        outcome: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Keep the obligation of a queued trigger with it (field case 4). A trigger with no thread has
    /// no slot to keep one for.
    pub(crate) fn note_queued_obligation(
        &mut self,
        session: &str,
        thread: Option<&str>,
        reply_thread: Option<&str>,
    ) -> Result<()> {
        let Some(thread) = thread else {
            return Ok(());
        };
        self.connection().execute(
            "INSERT INTO queued_obligations(session, thread, reply_thread) VALUES (?1, ?2, ?3)
             ON CONFLICT(session, thread) DO UPDATE SET reply_thread = excluded.reply_thread",
            params![session, thread, reply_thread],
        )?;
        Ok(())
    }

    /// Take the obligation kept with a queued trigger: `Some(reply_thread)` when one was kept (the
    /// inner `None` a trigger that owed nothing), `None` for an entry queued before it was kept.
    pub(crate) fn take_queued_obligation(
        &mut self,
        session: &str,
        thread: Option<&str>,
    ) -> Result<Option<Option<String>>> {
        let Some(thread) = thread else {
            return Ok(None);
        };
        let conn = self.connection();
        let kept: Option<Option<String>> = conn
            .query_row(
                "SELECT reply_thread FROM queued_obligations WHERE session = ?1 AND thread = ?2",
                params![session, thread],
                |r| r.get(0),
            )
            .optional()?;
        if kept.is_some() {
            conn.execute(
                "DELETE FROM queued_obligations WHERE session = ?1 AND thread = ?2",
                params![session, thread],
            )?;
        }
        Ok(kept)
    }

    /// Record what the resume did with a hold (`resumed`, `provider_cannot_resume`,
    /// `tree_moved_asked`, `not_yet`).
    pub(crate) fn set_hold_outcome(&mut self, id: i64, outcome: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE session_interruption SET outcome = ?2 WHERE id = ?1",
            params![id, outcome],
        )?;
        Ok(())
    }
}

/// A hold as the ladder reads it — [`crate::interruption::Interruption`] plus its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HoldOutcome {
    pub id: i64,
    pub at: String,
    pub until: Option<String>,
    pub resumed_at: Option<String>,
    pub outcome: Option<String>,
}

/// The latest of a set of instants, compared as instants.
fn latest<'a>(ats: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<&str> = None;
    for at in ats {
        if best.is_none_or(|b| earlier(b, at)) {
            best = Some(at);
        }
    }
    best
}

/// The bare handle of a qualified role.
fn bare(role: &str) -> &str {
    role.rsplit_once('/').map_or(role, |(_, b)| b)
}

// ---- the texts -----------------------------------------------------------------------------------

/// The first line of `body`, trimmed — what the texts quote of a commission.
fn first_line(body: &str) -> &str {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// **The ask** (rung A) — the one place `nxc reply --progress` is ever offered.
pub fn ask_text(thread: &str) -> String {
    format!(
        "Coordinator of thread {thread}: I have seen no activity from you for {} minutes. If the \
         work is done, send it with `nxc reply --thread {thread} -`. If you are still working, say so \
         this one time with `nxc reply --thread {thread} --progress <one line>` — use it because I \
         asked you, and again only when I ask you again. If you cannot finish, `nxc reply --thread \
         {thread} --escalate -` and say why.",
        SILENCE_SECS / 60
    )
}

/// **The continue** (rung C) — names the thread and quotes the task; offers no `--progress`.
pub fn continue_text(thread: &str, commission: &str) -> String {
    format!(
        "Your turn for thread {thread} ended without a reply; the task was: {}. If the work is done, \
         send it with `nxc reply --thread {thread} -`; if not, continue and reply when it is; if you \
         cannot, `nxc reply --thread {thread} --escalate -` and say why.",
        first_line(commission)
    )
}

/// The slot's commissioning message: the first message on its thread an action may follow.
pub(crate) fn commissioning_body(store: &ChatStore, thread: &str) -> Result<Option<String>> {
    Ok(store
        .acting_messages_in_thread(thread)?
        .into_iter()
        .next()
        .map(|m| m.body))
}

// ---- the rungs -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rung {
    Ask,
    Continue,
    Recommission,
}

impl Rung {
    fn action(self) -> RecoveryAction {
        match self {
            Rung::Ask => RecoveryAction::Asked,
            Rung::Continue => RecoveryAction::Continued,
            Rung::Recommission => RecoveryAction::Recommissioned,
        }
    }
}

/// Why the coordinator acts — what the look saw, carried to the log row and, at the end, to rung E.
struct Occasion {
    observed: Observation,
    judged_from: String,
    detail: String,
}

/// **One look of a coordinated slot's coordinator** — the rung `judgement` calls for.
pub(crate) fn climb(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    judgement: Judgement,
) -> Result<Vec<FailedConsequence>> {
    let mut findings = Vec::new();
    match judgement {
        Judgement::Nothing => {}
        Judgement::Live => live(ctx, store, row, coordinator, &mut findings)?,
        Judgement::Held => held(ctx, store, row, coordinator, &mut findings)?,
        Judgement::Record {
            observed,
            judged_from,
            detail,
        } => {
            let occasion = Occasion {
                observed,
                judged_from,
                detail,
            };
            let rung = rung_after_an_end(store, row, observed)?;
            run(ctx, store, row, coordinator, rung, occasion, &mut findings)?;
        }
    }
    Ok(findings)
}

/// **Which rung an observation without a live process calls for.** Inside the announced-end branch
/// the choice is by (bound, entries): a session with a runtime id to resume and work of its own on
/// the record is continued; anything else — never started, never bound, given up, nothing written —
/// is re-commissioned. A resume the provider refused is never retried: re-commission.
fn rung_after_an_end(store: &ChatStore, row: &OwedStart, observed: Observation) -> Result<Rung> {
    let Some(session) = row.session.as_deref() else {
        return Ok(Rung::Recommission);
    };
    match observed {
        Observation::NeverStarted | Observation::NeverBound | Observation::StartGivenUp => {
            return Ok(Rung::Recommission)
        }
        Observation::EndedWithoutAnswer | Observation::NoLiveProcess | Observation::Silent => {}
    }
    if provider_refused_the_resume(store, row, session)? {
        return Ok(Rung::Recommission);
    }
    let resumable = store.resolve_real(session)?.is_some();
    let worked = store.work_entry_count(session)? > 0;
    Ok(if resumable && worked {
        Rung::Continue
    } else {
        Rung::Recommission
    })
}

/// Whether the session's newest hold was taken up with `provider_cannot_resume` since the slot's
/// clock — the case rung B sends to D, never to C.
fn provider_refused_the_resume(store: &ChatStore, row: &OwedStart, session: &str) -> Result<bool> {
    Ok(store.latest_hold(session)?.is_some_and(|h| {
        h.outcome.as_deref() == Some("provider_cannot_resume")
            && !earlier(&h.at, &row.trigger_clock)
    }))
}

/// **Rung A's look**: a live session. Asked when it has been silent on its slot for
/// [`SILENCE_SECS`]; otherwise the next look is armed for the instant it would be — or one liveness
/// cadence out, whichever is first, so a session killed in between is found at its grace.
fn live(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let session = row.session.as_deref().unwrap_or_default();
    if past_the_ceiling(store, row, ctx.now)? {
        return escalate(
            ctx,
            store,
            row,
            coordinator,
            Occasion {
                observed: Observation::Silent,
                judged_from: row.recovery_started_at.clone().unwrap_or_default(),
                detail: ceiling_words(store, row)?,
            },
            findings,
        );
    }
    let activity = store.slot_last_activity(session, t)?;
    let quiet_since = latest(
        [
            activity.as_deref(),
            row.last_progress_at.as_deref(),
            row.asked_at.as_deref(),
            Some(row.trigger_clock.as_str()),
        ]
        .into_iter()
        .flatten(),
    )
    .unwrap_or(row.trigger_clock.as_str())
    .to_string();
    let due = after(&quiet_since, SILENCE_SECS, "the silence clock")?;
    let recheck = after(
        ctx.now,
        crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
        "the next look",
    )?;
    // An ask this host cannot deliver is not retried every cadence: the slot waits for its end.
    let door_closed = store.slot_recovery_log(t, h)?.last().is_some_and(|r| {
        r.action == RecoveryAction::Asked.as_str()
            && r.outcome.as_deref() == Some("cannot_ask")
            && !reached(
                ctx.now,
                &after(&r.at, SILENCE_SECS, "the closed door").unwrap_or_default(),
            )
    });
    if reached(ctx.now, &due) && !door_closed {
        let occasion = Occasion {
            observed: Observation::Silent,
            judged_from: quiet_since.clone(),
            detail: format!(
                "its process is alive and nothing was heard from it on this slot since {quiet_since}"
            ),
        };
        return run(ctx, store, row, coordinator, Rung::Ask, occasion, findings);
    }
    let next = if earlier(&due, &recheck) && !door_closed {
        due
    } else {
        recheck
    };
    arm(ctx, store, t, h, &next, findings);
    Ok(())
}

/// **Rung B's look**: a hold stands. The resume takes it up; the ladder acts only on a hold that
/// names no instant, waits on nobody and has stood one grace — it is continued.
fn held(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let Some(session) = row.session.as_deref() else {
        return Ok(());
    };
    let Some(hold) = store.latest_hold(session)? else {
        return Ok(());
    };
    let waits_on_nobody = hold.until.is_none()
        && hold.resumed_at.is_none()
        && matches!(hold.outcome.as_deref(), None | Some("not_yet"))
        && !ctx.worker.session_is_running(session);
    if !waits_on_nobody {
        return Ok(());
    }
    let due = after(&hold.at, STRANDED_GRACE_SECS, "the hold's grace")?;
    if !reached(ctx.now, &due) {
        arm(ctx, store, &row.thread, &row.role, &due, findings);
        return Ok(());
    }
    // Taken up by the ladder: the hold no longer stands once the session is continued.
    if store.mark_interruption_resumed(hold.id, ctx.now)? {
        store.set_hold_outcome(hold.id, "resumed")?;
    }
    let occasion = Occasion {
        observed: Observation::EndedWithoutAnswer,
        judged_from: hold.at.clone(),
        detail: format!(
            "its session went on hold at {} with no instant named and nothing took the hold up",
            hold.at
        ),
    };
    let rung = if store.resolve_real(session)?.is_some() {
        Rung::Continue
    } else {
        Rung::Recommission
    };
    run(ctx, store, row, coordinator, rung, occasion, findings)
}

/// The declared window of the slot, when one is armed — what the ceiling multiplies.
fn declared_window(store: &ChatStore, thread: &str) -> Result<Option<i64>> {
    Ok(store.member_deadline(thread)?.map(|d| d.window_secs))
}

fn past_the_ceiling(store: &ChatStore, row: &OwedStart, now: &str) -> Result<bool> {
    let Some(started) = row.recovery_started_at.as_deref() else {
        return Ok(false);
    };
    let ceiling = recovery_ceiling_secs(declared_window(store, &row.thread)?);
    Ok(reached(
        now,
        &after(started, ceiling, "the recovery ceiling")?,
    ))
}

fn ceiling_words(store: &ChatStore, row: &OwedStart) -> Result<String> {
    let ceiling = recovery_ceiling_secs(declared_window(store, &row.thread)?);
    Ok(format!(
        "its recovery began at {} and reached the ceiling of {} h without an answer",
        row.recovery_started_at.as_deref().unwrap_or("?"),
        ceiling / 3600
    ))
}

fn arm(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread: &str,
    role: &str,
    at: &str,
    findings: &mut Vec<FailedConsequence>,
) {
    if let Err(e) = store.set_next_look(thread, role, at) {
        eprintln!(
            "warning: the next look at the slot of {role} on thread {thread} was not noted: {e}"
        );
    }
    findings.extend(crate::stranded::arm_look(ctx, store, thread, role, at));
}

/// **The claim, kept alive while a trigger runs** — renewed every half of [`CLAIM_SECS`] from a
/// connection of its own, so a trigger slower than the claim never lets a second look in. Counted
/// from the look's own instant plus the real time elapsed, so an injected clock and the wall clock
/// agree about how far it reaches.
struct ClaimRenewal {
    stop: Option<std::sync::mpsc::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ClaimRenewal {
    fn start(ctx: &Ctx, thread: &str, role: &str) -> ClaimRenewal {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let db = ctx.db_path.to_string();
        let (thread, role, base) = (thread.to_string(), role.to_string(), ctx.now.to_string());
        let begun = std::time::Instant::now();
        let handle = std::thread::Builder::new()
            .name("nxc-claim-renewal".into())
            .spawn(move || {
                let half = std::time::Duration::from_secs((CLAIM_SECS / 2) as u64);
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = rx.recv_timeout(half) {
                    let elapsed = i64::try_from(begun.elapsed().as_secs()).unwrap_or(i64::MAX);
                    let Ok(until) = after(&base, elapsed.saturating_add(CLAIM_SECS), "renewal")
                    else {
                        return;
                    };
                    let renewed = rusqlite::Connection::open(&db).and_then(|c| {
                        c.busy_timeout(std::time::Duration::from_secs(5))?;
                        c.execute(
                            "UPDATE owed_starts SET recovering_until = ?3
                              WHERE thread = ?1 AND role = ?2 AND recovering_until IS NOT NULL",
                            params![thread, role, until],
                        )
                    });
                    if let Err(e) = renewed {
                        eprintln!(
                            "warning: the claim on the slot of {role} on thread {thread} could not \
                             be renewed: {e}; the recheck still refuses a second rung while this \
                             one is in flight"
                        );
                    }
                }
            })
            .ok();
        ClaimRenewal {
            stop: Some(tx),
            handle,
        }
    }
}

impl Drop for ClaimRenewal {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// What a rung's trigger did, for its log row and for what follows.
enum Done {
    /// It did something; the outcome word for the log.
    Acted(&'static str),
    /// It did nothing and counts nothing.
    Nothing(&'static str),
    /// A refusal no retry will change — rung E names it.
    Refused(String),
    /// It failed this time; the next look tries again.
    Failed(String),
}

/// **Run one rung under the claim** — or rung E, when the budget or the ceiling is spent.
fn run(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    rung: Rung,
    occasion: Occasion,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    if row.attempts >= MAX_RECOVERY_ATTEMPTS || past_the_ceiling(store, row, ctx.now)? {
        return escalate(ctx, store, row, coordinator, occasion, findings);
    }
    match store.claim_slot(t, h, ctx.now)? {
        SlotClaim::Claimed => {}
        SlotClaim::NoRow => return Ok(()),
        SlotClaim::BeingRecovered { retry_at } => {
            arm(ctx, store, t, h, &retry_at, findings);
            return Ok(());
        }
    }
    let done = under_the_claim(ctx, store, row, rung, &occasion, findings);
    if let Err(e) = store.release_slot_claim(t, h) {
        eprintln!("warning: the claim on the slot of {h} on thread {t} could not be released: {e}");
    }
    match done? {
        Some(Done::Refused(why)) => {
            let refused = Occasion {
                detail: format!(
                    "{}; then its {} was refused: {why}",
                    occasion.detail,
                    rung.action().as_str()
                ),
                ..occasion
            };
            let row = store.owed_start(t, h)?.unwrap_or_else(|| row.clone());
            escalate(ctx, store, &row, coordinator, refused, findings)
        }
        _ => Ok(()),
    }
}

/// The part of [`run`] that holds the claim: the recheck, the attempt, the log row, the lease hook,
/// the trigger — in that order, fail-closed. `None` when the recheck stopped it.
fn under_the_claim(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    rung: Rung,
    occasion: &Occasion,
    findings: &mut Vec<FailedConsequence>,
) -> Result<Option<Done>> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    // The recheck: still owed by this role, and no rung of its own still in flight.
    let owed = store
        .thread_quorum(t, ctx.now)?
        .is_some_and(|q| outstanding_has(&q.outstanding, h));
    if !owed {
        return Ok(None);
    }
    if let Some(flying) = store
        .slot_recovery_log(t, h)?
        .into_iter()
        .rev()
        .find(|r| r.outcome.is_none())
    {
        let horizon = after(&flying.at, TRIGGER_TIMEOUT_SECS, "a rung in flight")?;
        if !reached(ctx.now, &horizon) {
            arm(ctx, store, t, h, &horizon, findings);
            return Ok(None);
        }
        store.settle_rung(flying.id, "timed_out")?;
    }
    // The lease hook (cut 1g): the slot's lease still names this operation. Until 1g there is
    // nothing to check and nothing to renew.
    lease_hook(ctx, store, row);
    let opened_the_episode = row.recovery_started_at.is_none();
    let attempt = store.begin_attempt(t, h, ctx.now)?;
    let log = store.log_rung(
        t,
        h,
        ctx.now,
        rung.action(),
        attempt,
        row.session.as_deref(),
        &occasion.detail,
    )?;
    let done = {
        let _renewal = ClaimRenewal::start(ctx, t, h);
        fire_the_rung(ctx, store, row, rung)
    };
    let outcome = match &done {
        Done::Acted(word) | Done::Nothing(word) => (*word).to_string(),
        Done::Refused(why) => format!("refused: {why}"),
        Done::Failed(why) => format!("failed: {why}"),
    };
    store.settle_rung(log, &outcome)?;
    match &done {
        Done::Nothing(_) => store.revert_attempt(t, h, opened_the_episode)?,
        Done::Acted("asked") => {
            store.set_asked(t, h, ctx.now)?;
            let next = after(ctx.now, SILENCE_SECS, "the next ask")?;
            arm(ctx, store, t, h, &next, findings);
        }
        Done::Acted(_) => {
            // The trigger armed the slot's first look one grace out (`stranded::owe`).
            let next = after(ctx.now, STRANDED_GRACE_SECS, "the next look")?;
            store.set_next_look(t, h, &next)?;
        }
        Done::Failed(why) => {
            let detail = format!(
                "the {} of {h} on thread {t} failed: {why}; the next look tries again",
                rung.action().as_str()
            );
            eprintln!("warning: {detail}");
            findings.push(FailedConsequence::stranded(
                ConsequenceClass::RecoveryFailed,
                Some(t),
                row.session.as_deref(),
                detail,
            ));
            let next = after(
                ctx.now,
                crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
                "the next look",
            )?;
            arm(ctx, store, t, h, &next, findings);
        }
        Done::Refused(_) => {}
    }
    if matches!(&done, Done::Nothing(_)) {
        let next = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the next look",
        )?;
        arm(ctx, store, t, h, &next, findings);
    }
    Ok(Some(done))
}

/// **The lease hook every rung calls** (cut 1g, 6j6v.vh0z, fills it): renew the operation's lease
/// to a bound in the future, and say when it is held by another operation. A no-op until then.
fn lease_hook(_ctx: &Ctx, _store: &mut ChatStore, _row: &OwedStart) {}

/// The trigger of one rung.
fn fire_the_rung(ctx: &Ctx, store: &mut ChatStore, row: &OwedStart, rung: Rung) -> Done {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    match rung {
        Rung::Ask => {
            let Some(session) = row.session.as_deref() else {
                return Done::Nothing("no_session");
            };
            let ask = SessionAsk {
                session: session.to_string(),
                thread: t.to_string(),
                role: h.to_string(),
                text: ask_text(t),
                at: ctx.now.to_string(),
            };
            match ctx.worker.ask_session(&ask) {
                AskOutcome::Asked => Done::Acted("asked"),
                AskOutcome::NoProcess => Done::Nothing("no_process"),
                _ => Done::Nothing("cannot_ask"),
            }
        }
        Rung::Continue => {
            let Some(session) = row.session.as_deref() else {
                return recommission(ctx, store, row);
            };
            let commission = commissioning_body(store, t)
                .ok()
                .flatten()
                .unwrap_or_default();
            let text = continue_text(t, &commission);
            match crate::orchestration::ladder_continue(ctx, store, t, h, session, &text) {
                Ok(admission) => Done::Acted(admitted(&admission)),
                // The runtime no longer has the conversation: there is nothing to continue, so the
                // slot is started over with its task, in the same claim.
                Err(TriggerError::SessionGone { .. }) => match recommission(ctx, store, row) {
                    Done::Acted(_) => Done::Acted("session_gone_recommissioned"),
                    other => other,
                },
                Err(e) => classify(e),
            }
        }
        Rung::Recommission => recommission(ctx, store, row),
    }
}

fn recommission(ctx: &Ctx, store: &mut ChatStore, row: &OwedStart) -> Done {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let body = match commissioning_body(store, t) {
        Ok(Some(body)) => body,
        Ok(None) => return Done::Refused("the slot's commissioning message is gone".to_string()),
        Err(e) => return Done::Failed(e.to_string()),
    };
    match crate::orchestration::ladder_recommission(ctx, store, t, h, row.session.as_deref(), &body)
    {
        Ok(admission) => Done::Acted(admitted(&admission)),
        Err(e) => classify(e),
    }
}

fn admitted(admission: &crate::orchestration::TriggerAdmission) -> &'static str {
    match admission {
        crate::orchestration::TriggerAdmission::Queued { .. } => "queued",
        _ => "triggered",
    }
}

fn classify(e: TriggerError) -> Done {
    match e {
        TriggerError::AlreadyRunning { .. } => Done::Nothing("already_running"),
        TriggerError::Failed(inner)
            if matches!(
                inner.kind,
                crate::error::ErrorKind::Forbidden | crate::error::ErrorKind::Validation
            ) =>
        {
            Done::Refused(inner.to_string())
        }
        other => Done::Failed(other.to_string()),
    }
}

// ---- rung E --------------------------------------------------------------------------------------

/// **Rung E**: the coordinator escalates — in the member's name, into the slot's thread, through
/// cut 1a's record and its stamping — with the summary the commissioner needs. The episode ends
/// with it: the budget and the ceiling clock are cleared.
fn escalate(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    occasion: Occasion,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let summary = summary(ctx, store, row, coordinator, &occasion)?;
    let log = store.log_rung(
        t,
        h,
        ctx.now,
        RecoveryAction::Escalated,
        row.attempts,
        row.session.as_deref(),
        &occasion.detail,
    )?;
    store.end_recovery_episode(t, h)?;
    findings.extend(crate::stranded::record(
        ctx,
        store,
        row,
        occasion.observed,
        occasion.judged_from,
        occasion.detail,
        Some(summary),
    )?);
    store.settle_rung(log, "escalated")?;
    Ok(())
}

/// **What rung E tells the commissioner**: the step and its coordinator, the cause, every attempt of
/// the episode with its instant and outcome, the last activity, the session, the lease, and the
/// three ways on.
fn summary(
    ctx: &Ctx,
    store: &ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    occasion: &Occasion,
) -> Result<String> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let step = match store.flow_mark(t).ok().flatten() {
        Some(mark) => mark.step,
        None => bare(h).to_string(),
    };
    let root = store.thread_root(t).unwrap_or_else(|_| t.to_string());
    let since = row.recovery_started_at.as_deref();
    let attempts: Vec<String> = store
        .slot_recovery_log(t, h)?
        .into_iter()
        .filter(|r| r.action != RecoveryAction::Escalated.as_str())
        .filter(|r| since.is_some_and(|s| !earlier(&r.at, s)))
        .map(|r| {
            format!(
                "- attempt {}/{MAX_RECOVERY_ATTEMPTS} at {}: {} — {} ({})",
                r.attempt,
                r.at,
                r.action,
                r.outcome.as_deref().unwrap_or("in flight"),
                r.detail
            )
        })
        .collect();
    let attempts = if attempts.is_empty() {
        "- none: nothing could be tried".to_string()
    } else {
        attempts.join("\n")
    };
    let cause = match row.session.as_deref() {
        Some(s) => match store.session_last_cause(s)? {
            Some(c) => format!("{}; the session's last end said: {c}", occasion.detail),
            None => occasion.detail.clone(),
        },
        None => occasion.detail.clone(),
    };
    let last_activity = match row.session.as_deref() {
        Some(s) => store
            .slot_last_activity(s, t)?
            .or(store.last_transcript_at(s)?)
            .unwrap_or_else(|| "none on record".to_string()),
        None => "none — no session was ever started".to_string(),
    };
    let session = row
        .session
        .as_deref()
        .map_or("none was ever started".to_string(), str::to_string);
    let lease = lease_words(ctx, store, t, row.session.as_deref());
    Ok(format!(
        "ESCALATED BY ITS COORDINATOR — step {step} on thread {t} (coordinated by thread {coord}) \
         could not be recovered.\n\n\
         Cause: {cause}.\n\n\
         What was tried:\n{attempts}\n\n\
         Last activity on this slot: {last_activity}.\n\
         Session: {session}.\n\
         Working copy: {lease}.\n\n\
         The ways on: a reply into thread {root} commissions the round again; `nxc reply --thread \
         {root} --accept` accepts it as it stands; `nxc withdraw --thread {root}` takes it back.",
        coord = coordinator.thread,
    ))
}

/// The lease as the escalation names it.
fn lease_words(ctx: &Ctx, store: &ChatStore, thread: &str, session: Option<&str>) -> String {
    let own = store
        .thread_root(thread)
        .map(|r| crate::working_tree::WorkScope::Thread(r).key())
        .unwrap_or_default();
    match store.working_tree_holder(ctx.now) {
        Ok(Some(holder)) if holder == own => {
            if session.is_some_and(|s| ctx.worker.session_is_running(s)) {
                "held by this operation — process alive".to_string()
            } else {
                "held by this operation — no live process".to_string()
            }
        }
        Ok(Some(holder)) => format!("held by another operation ({holder})"),
        Ok(None) => "not held".to_string(),
        Err(e) => format!("unknown ({e})"),
    }
}

/// Whether the slot is UNDER RECOVERY (epic 6j6v.nmc9 section B, defined once): a row is present,
/// at least one attempt counted, not abandoned, and no rung-E record ends the episode.
pub(crate) fn under_recovery(row: &OwedStart) -> bool {
    row.attempts >= 1 && row.abandoned.is_none()
}

pub(crate) fn action_of(row: &RecoveryLogRow) -> Option<RecoveryAction> {
    RecoveryAction::parse(&row.action)
}

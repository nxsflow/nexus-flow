//! **The ladder: nudge, stop, continue, re-commission, escalate** (nxf 6j6v.558h, cut 1d of epic
//! 6j6v.nmc9, section B; the liveness of 6j6v.1r61, cut 3 of epic 6j6v.pszd).
//!
//! Every coordinated slot (`stranded::coordinator_of`) is taken up by its coordinator — the engine's
//! tick on the thread that opened it — before anybody is asked. The stranded sweep's look decides
//! WHAT it sees ([`crate::stranded`]'s `judge`); this module decides what to DO about it, one rung
//! per look, under a claim:
//!
//! - **A — nudge, and stop.** A live session that showed no life of its own on its slot for
//!   [`SILENCE_SECS`] is nudged through the ask door ([`crate::worker::Worker::ask_session`]) with
//!   the one text that offers `nxc confirm --rest`; nudged and still lifeless after
//!   [`NUDGE_WAIT_SECS`], it is dead and stopped (SIGTERM). The state machine of one look is
//!   `live` below, step by step as design 8nx4 section 4 numbers them.
//! - **B — hold.** A slot whose session is on hold is the resume's; the ladder takes up only the
//!   holds nothing will: a resume the provider refused (→ D), a hold that names no instant and
//!   waits on nobody (→ C one grace after it was taken).
//! - **C — continue.** A session that ended without answering — stopped by A included — bound and
//!   with work of its own on the record, is continued in the SAME session with a reminder that
//!   names the thread, the task and, after a stop, why and what still runs.
//! - **D — re-commission.** A slot that never got a session, whose start was given up, whose
//!   session ended with nothing written, or whose resume the provider refused, gets a fresh session
//!   with its ORIGINAL body, minted into the same slot.
//! - **E — escalate.** At [`MAX_RECOVERY_ATTEMPTS`], at a dead verdict past the resume cap, at the
//!   wall-clock ceiling, or on a permanent refusal, the coordinator escalates in the member's name
//!   with what it knows — the step, the cause, every attempt, the signs of life, the session and
//!   the lease — and the escalation climbs one level per round through the existing path.
//!
//! **The budget is the owed turn's** — continuations and re-commissions; nudges and stops are
//! logged, not counted — reset by an answer that discharges the slot and by rung E. **The episode**
//! is the ceiling's unit: opened by the first nudge, stop or attempt, ended by the session's own
//! life (`recovered`), by an answer, or by rung E.
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

/// **Silence on a slot that earns a nudge** (nxf 6j6v.1r61, design 8nx4 section 2) — ten minutes
/// with no life of the session's own since `max(L, P)`; also the life of a nudge nobody read.
pub const SILENCE_SECS: i64 = 600;
/// **Nudged, and no life since** — five minutes on, the session is dead.
pub const NUDGE_WAIT_SECS: i64 = 300;
/// **An open call's subprocess stops counting as life** this long after its last movement.
pub const SUBPROCESS_IDLE_SECS: i64 = 1_800;
/// **The look after a rest** comes at the rest's end plus this.
pub const CONFIRM_GRACE_SECS: i64 = 300;
/// **How many rests one episode may confirm** (`nxc confirm --rest`).
pub const CONFIRMATIONS_MAX: u32 = 3;
/// **The sum of the rests one episode may be granted**, in seconds — ninety minutes.
pub const CONFIRM_TOTAL_CAP_SECS: i64 = 5_400;
/// **The bound of one `--rest`**, in minutes.
pub const REST_MAX_MINUTES: i64 = 60;
/// **A stop's grace** — the one further SIGTERM at this, "would not stop" at twice this.
pub const STOP_GRACE_SECS: i64 = 300;
/// **The resumes an owed turn is allowed before its next dead verdict escalates** — policy B, the
/// default. Read through [`dead_resumes`] only; cut 4 of 6j6v.pszd replaces that one read with the
/// workspace's configured policy.
pub const DEFAULT_DEAD_RESUMES: u32 = 1;

/// **The resume cap in force** — the ONE read site of the policy (cut 4 reads the workspace's).
pub(crate) fn dead_resumes(_ctx: &Ctx) -> u32 {
    DEFAULT_DEAD_RESUMES
}
/// **The bound of one tool call** (nxf 6j6v.j6yk) — how long a live session may sit inside a single
/// tool call before the call counts as hung. Generous on purpose: a step that watches CI waits on a
/// run of 60–80 minutes, and the quality gates alone may take 150.
pub const TOOL_CALL_BOUND_SECS: i64 = 10_800;
/// **How long a claim holds** before another look may take the slot — a rung's trigger is a process
/// spawn, seconds; renewed every half of it while the trigger runs.
pub const CLAIM_SECS: i64 = 60;
/// **How long a rung in flight blocks a second one** — the recheck refuses while the slot's newest
/// log row has no outcome and is younger than this.
pub const TRIGGER_TIMEOUT_SECS: i64 = 600;
/// **The budget** — resumes and re-commissions; nudges and stops are logged, not counted. It is the
/// OWED TURN's budget: reset only by an answer that discharges the slot, or by rung E. A turn that
/// already used a recovery attempt — an ordinary continue or re-commission included — escalates
/// under policy B at its FIRST dead verdict.
pub const MAX_RECOVERY_ATTEMPTS: u32 = 3;
/// **How often one rung's failed start is tried again on the same rung** (round 2 TQ #6): one
/// re-commission or continue with its retries is ONE attempt and ONE log row — three starts in all,
/// as a direct send gets (`orchestration::MAX_START_ATTEMPTS`). Past that, the next try is a new
/// rung and a new attempt, so a start that never succeeds still reaches [`MAX_RECOVERY_ATTEMPTS`]
/// within minutes rather than at the thread's deadline.
pub const RUNG_START_RETRIES: u32 = 2;
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
    /// An asked-for `--progress` reset the budget. Written by versions before 6j6v.1r61 retired
    /// `nxc reply --progress`, never written now; kept so an old row still reads.
    Progress,
    /// The coordinator stopped a session it found dead (SIGTERM).
    Stopped,
    /// The session confirmed a rest (`nxc confirm --rest`).
    Confirmed,
    /// The episode ended by the session's own life.
    Recovered,
}

impl RecoveryAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RecoveryAction::Asked => "asked",
            RecoveryAction::Continued => "continued",
            RecoveryAction::Recommissioned => "recommissioned",
            RecoveryAction::Escalated => "escalated",
            RecoveryAction::Progress => "progress",
            RecoveryAction::Stopped => "stopped",
            RecoveryAction::Confirmed => "confirmed",
            RecoveryAction::Recovered => "recovered",
        }
    }

    fn parse(raw: &str) -> Option<RecoveryAction> {
        Some(match raw {
            "asked" => RecoveryAction::Asked,
            "continued" => RecoveryAction::Continued,
            "recommissioned" => RecoveryAction::Recommissioned,
            "escalated" => RecoveryAction::Escalated,
            "progress" => RecoveryAction::Progress,
            "stopped" => RecoveryAction::Stopped,
            "confirmed" => RecoveryAction::Confirmed,
            "recovered" => RecoveryAction::Recovered,
            _ => return None,
        })
    }
}

/// **What became of one rung** — the `outcome` of its log row, in the one spelling producer and
/// reader share. A refusal and a failure carry their error beside the word (`refused: <error>`,
/// `failed: <error>`), which is why they are not variants here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RungOutcome {
    /// The ask reached the session.
    Asked,
    /// The trigger started the session.
    Triggered,
    /// The trigger waits in the working-copy queue.
    Queued,
    /// The runtime had lost the conversation; the slot was started afresh instead.
    SessionGoneRecommissioned,
    /// The slot had no session to ask.
    NoSession,
    /// No live process held the session.
    NoProcess,
    /// A process held it and could not be asked.
    CannotAsk,
    /// A process for the session was already running; nothing was started.
    AlreadyRunning,
    /// The rung was in flight past the trigger timeout; a later look took the slot up again.
    TimedOut,
    /// The coordinator escalated.
    Escalated,
    /// An asked-for `--progress` reset the budget. Written by versions before 6j6v.1r61, never
    /// written now; kept so an old row still reads.
    Reset,
    /// A nudge the session's own life overtook, withdrawn unread.
    Overtaken,
    /// The SIGTERM was delivered.
    StopRequested,
    /// There was no process left to signal.
    NothingToStop,
    /// A rest was granted.
    Granted,
    /// The episode ended by the session's own life.
    Life,
}

impl RungOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            RungOutcome::Asked => "asked",
            RungOutcome::Triggered => "triggered",
            RungOutcome::Queued => "queued",
            RungOutcome::SessionGoneRecommissioned => "session_gone_recommissioned",
            RungOutcome::NoSession => "no_session",
            RungOutcome::NoProcess => "no_process",
            RungOutcome::CannotAsk => "cannot_ask",
            RungOutcome::AlreadyRunning => "already_running",
            RungOutcome::TimedOut => "timed_out",
            RungOutcome::Escalated => "escalated",
            RungOutcome::Reset => "reset",
            RungOutcome::Overtaken => "overtaken",
            RungOutcome::StopRequested => "stop_requested",
            RungOutcome::NothingToStop => "nothing_to_stop",
            RungOutcome::Granted => "granted",
            RungOutcome::Life => "life",
        }
    }

    pub fn parse(raw: &str) -> Option<RungOutcome> {
        Some(match raw {
            "asked" => RungOutcome::Asked,
            "triggered" => RungOutcome::Triggered,
            "queued" => RungOutcome::Queued,
            "session_gone_recommissioned" => RungOutcome::SessionGoneRecommissioned,
            "no_session" => RungOutcome::NoSession,
            "no_process" => RungOutcome::NoProcess,
            "cannot_ask" => RungOutcome::CannotAsk,
            "already_running" => RungOutcome::AlreadyRunning,
            "timed_out" => RungOutcome::TimedOut,
            "escalated" => RungOutcome::Escalated,
            "reset" => RungOutcome::Reset,
            "overtaken" => RungOutcome::Overtaken,
            "stop_requested" => RungOutcome::StopRequested,
            "nothing_to_stop" => RungOutcome::NothingToStop,
            "granted" => RungOutcome::Granted,
            "life" => RungOutcome::Life,
            _ => return None,
        })
    }

    /// The outcome as a person reads it — what the escalation shows.
    pub fn words(self) -> &'static str {
        match self {
            RungOutcome::Asked => "the session was asked",
            RungOutcome::Triggered => "the session was started",
            RungOutcome::Queued => "the start waits for the working copy",
            RungOutcome::SessionGoneRecommissioned => {
                "the runtime had lost the conversation, so the slot was started afresh"
            }
            RungOutcome::NoSession => "there was no session to ask",
            RungOutcome::NoProcess => "no process held the session, so nothing was asked",
            RungOutcome::CannotAsk => "the session's process could not be asked",
            RungOutcome::AlreadyRunning => "a process was already running it, so nothing started",
            RungOutcome::TimedOut => "it did not finish within the trigger timeout",
            RungOutcome::Escalated => "escalated",
            RungOutcome::Reset => "the budget was reset",
            RungOutcome::Overtaken => "its own life overtook the nudge, which was withdrawn",
            RungOutcome::StopRequested => "the stop was requested",
            RungOutcome::NothingToStop => "there was no process left to stop",
            RungOutcome::Granted => "the rest was granted",
            RungOutcome::Life => "it showed life of its own",
        }
    }
}

/// A log row's outcome as a person reads it: the typed word in words, and anything else — a
/// refusal or a failure with its error ([`failed_outcome`]), a word this build does not know —
/// verbatim, which already reads as words.
fn outcome_words(raw: Option<&str>) -> String {
    match raw {
        None => "still in flight".to_string(),
        Some(raw) => match RungOutcome::parse(raw) {
            Some(o) => o.words().to_string(),
            None => raw.to_string(),
        },
    }
}

/// The marker a failed rung's outcome carries before its error.
const FAILED: &str = "failed: ";

/// **The outcome of a rung whose trigger failed** — `failed: <error>`, read back by [`failure_of`].
fn failed_outcome(why: &str) -> String {
    format!("{FAILED}{why}")
}

/// The error of a failed rung's outcome, `None` for any other outcome.
fn failure_of(outcome: &str) -> Option<&str> {
    outcome.strip_prefix(FAILED)
}

/// **What a resume did with a hold** — `session_interruption.outcome`, written by
/// `resume_interrupted` and by the ladder's rung B, read by rung B and rung D.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HoldOutcome {
    /// The session was continued.
    Resumed,
    /// The provider cannot continue it — rung D, never rung C.
    ProviderCannotResume,
    /// The tree moved and the root was asked — once.
    TreeMovedAsked,
    /// The window had not lifted yet.
    NotYet,
}

impl HoldOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            HoldOutcome::Resumed => "resumed",
            HoldOutcome::ProviderCannotResume => "provider_cannot_resume",
            HoldOutcome::TreeMovedAsked => "tree_moved_asked",
            HoldOutcome::NotYet => "not_yet",
        }
    }

    pub fn parse(raw: &str) -> Option<HoldOutcome> {
        Some(match raw {
            "resumed" => HoldOutcome::Resumed,
            "provider_cannot_resume" => HoldOutcome::ProviderCannotResume,
            "tree_moved_asked" => HoldOutcome::TreeMovedAsked,
            "not_yet" => HoldOutcome::NotYet,
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
    /// How often this rung's start was tried again after it failed — on the same row and the same
    /// attempt (round 2 TQ #6).
    pub retries: u32,
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

const LOG_COLS: &str = "id, thread, role, at, action, attempt, session, outcome, detail, retries";

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
        retries: u32::try_from(r.get::<_, i64>(9)?).unwrap_or(0),
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

    /// **Take a failed rung up again** for one more start: its outcome goes back to in flight and its
    /// retry count goes up — the same row, the same attempt.
    pub(crate) fn retry_rung(&mut self, id: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE recovery_log SET outcome = NULL, retries = retries + 1 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Write what became of a rung — one of the typed words.
    pub(crate) fn settle_rung_as(&mut self, id: i64, outcome: RungOutcome) -> Result<()> {
        self.settle_rung(id, outcome.as_str())
    }

    /// Write what became of a rung, verbatim — the typed words through [`settle_rung_as`], and a
    /// refusal or failure as `refused: <error>` / `failed: <error>`.
    ///
    /// [`settle_rung_as`]: ChatStore::settle_rung_as
    fn settle_rung(&mut self, id: i64, outcome: &str) -> Result<()> {
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

    /// The slot's newest row of `action`, `None` before its first.
    pub(crate) fn newest_rung_of(
        &self,
        thread: &str,
        role: &str,
        action: RecoveryAction,
    ) -> Result<Option<RecoveryLogRow>> {
        Ok(self
            .connection()
            .query_row(
                &format!(
                    "SELECT {LOG_COLS} FROM recovery_log WHERE thread = ?1 AND role = ?2
                       AND action = ?3 ORDER BY id DESC LIMIT 1"
                ),
                params![thread, role, action.as_str()],
                log_from_row,
            )
            .optional()?)
    }

    /// Write what became of a rung and say it again in its detail.
    pub(crate) fn settle_rung_with_detail(
        &mut self,
        id: i64,
        outcome: &str,
        detail: &str,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE recovery_log SET outcome = ?2, detail = ?3 WHERE id = ?1",
            params![id, outcome, detail],
        )?;
        Ok(())
    }

    /// The slot's newest rung, `None` before its first.
    pub(crate) fn newest_rung(&self, thread: &str, role: &str) -> Result<Option<RecoveryLogRow>> {
        Ok(self
            .connection()
            .query_row(
                &format!(
                    "SELECT {LOG_COLS} FROM recovery_log WHERE thread = ?1 AND role = ?2
                      ORDER BY id DESC LIMIT 1"
                ),
                params![thread, role],
                log_from_row,
            )
            .optional()?)
    }

    /// The newest row of every slot among `threads` — `status`'s one read.
    pub(crate) fn newest_recovery_by_threads(
        &self,
        threads: &[&str],
    ) -> Result<std::collections::BTreeMap<(String, String), RecoveryLogRow>> {
        let mut out = std::collections::BTreeMap::new();
        let conn = self.connection();
        // In chunks well under SQLite's bound-variable limit, so no report is too wide to read.
        for chunk in threads.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let mut st = conn.prepare(&format!(
                "SELECT {LOG_COLS} FROM recovery_log WHERE thread IN ({placeholders}) ORDER BY id"
            ))?;
            let rows = st
                .query_map(rusqlite::params_from_iter(chunk.iter()), log_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.extend(
                rows.into_iter()
                    .map(|r| ((r.thread.clone(), r.role.clone()), r)),
            );
        }
        Ok(out)
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

    /// Mark the slot nudged at `now` — and open the episode, when none is open. No attempt is
    /// counted: a nudge is logged, not counted.
    pub(crate) fn set_asked(&mut self, thread: &str, role: &str, now: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET asked_at = ?3,
                    recovery_started_at = COALESCE(recovery_started_at, ?3)
              WHERE thread = ?1 AND role = ?2",
            params![thread, role, now],
        )?;
        Ok(())
    }

    /// Every OTHER slot `session` owes forgets the nudge it was waiting on — its file was withdrawn
    /// with the session's (see [`withdraw_the_nudges`]).
    pub(crate) fn forget_the_other_nudges(&mut self, session: &str, thread: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET asked_at = NULL
              WHERE session = ?1 AND thread <> ?2 AND asked_at IS NOT NULL",
            params![session, thread],
        )?;
        Ok(())
    }

    /// The nudge is no longer pending — overtaken by life.
    pub(crate) fn set_asked_cleared(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET asked_at = NULL WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// **The ladder stopped the slot's process at `now`** — the stop in flight, its first signal,
    /// and the episode opened when none is open.
    pub(crate) fn note_stop_requested(
        &mut self,
        thread: &str,
        role: &str,
        now: &str,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET stop_requested_at = ?3, stop_signals = 1,
                    recovery_started_at = COALESCE(recovery_started_at, ?3)
              WHERE thread = ?1 AND role = ?2",
            params![thread, role, now],
        )?;
        Ok(())
    }

    /// The one further SIGTERM was sent.
    pub(crate) fn note_second_signal(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET stop_signals = 2 WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// The rest no longer stands.
    pub(crate) fn clear_rest(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET confirmed_until = NULL WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// **The episode ended by life** (step 5): its clock, its confirmations and its rests go;
    /// `attempts` — the owed turn's budget — stays.
    pub(crate) fn end_episode_by_life(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET recovery_started_at = NULL, confirmations = 0,
                    rest_total_secs = 0, confirmed_until = NULL
              WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// **Store one sample of the open call's subprocesses** and answer `M`: the instant the sample
    /// last moved. Any change of `(count, cpu)` is a movement, and so is the first sample with a
    /// process in it; a call that never had one under it has no `M` (design 8nx4 section 1 (b)).
    /// The sample is the CALL's (`call_since`, the call's open instant): a sample stored under an
    /// earlier call is no prior sample, so a call with nothing under it never inherits the `M` of
    /// the call before it (review of pass 1, Integrity #1).
    pub(crate) fn store_subprocess_sample(
        &mut self,
        thread: &str,
        role: &str,
        row: &OwedStart,
        call_since: &str,
        sample: nxs_service::process::SubprocessSample,
        now: &str,
    ) -> Result<Option<String>> {
        let same_call = row.subprocess_since.as_deref() == Some(call_since);
        let (before, prior_moved_at) = if same_call {
            (
                (row.subprocess_count, row.subprocess_cpu_units),
                row.subprocess_moved_at.clone(),
            )
        } else {
            ((None, None), None)
        };
        let after = (Some(sample.count), Some(sample.cpu_units));
        let had_a_process = before.0.is_some_and(|c| c > 0);
        let moved = before != after && (sample.count > 0 || had_a_process);
        let moved_at = if moved {
            Some(now.to_string())
        } else {
            prior_moved_at
        };
        self.connection().execute(
            "UPDATE owed_starts SET subprocess_count = ?3, subprocess_cpu_units = ?4,
                    subprocess_moved_at = ?5, subprocess_since = ?6
              WHERE thread = ?1 AND role = ?2",
            params![
                thread,
                role,
                i64::from(sample.count),
                i64::try_from(sample.cpu_units).unwrap_or(i64::MAX),
                moved_at,
                call_since
            ],
        )?;
        Ok(moved_at)
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

    /// **A rest confirmed** (nxf 6j6v.1r61, design 8nx4 section 5) — ONE update, accepted only while
    /// a nudge is pending and no stop is in flight: the rest's end, one more confirmation, the rest
    /// added to the episode's total, and the nudge answered. `false` when the row did not qualify
    /// at the moment of the write; nothing was recorded then.
    pub(crate) fn confirm_rest_on_row(
        &mut self,
        thread: &str,
        role: &str,
        until: &str,
        granted_secs: i64,
    ) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE owed_starts SET confirmed_until = ?3, confirmations = confirmations + 1,
                    rest_total_secs = rest_total_secs + ?4, asked_at = NULL
              WHERE thread = ?1 AND role = ?2 AND asked_at IS NOT NULL
                AND stop_requested_at IS NULL",
            params![thread, role, until, granted_secs],
        )?;
        Ok(n > 0)
    }

    /// **The episode is over** — rung E's record, or a commissioner's reply after it: the budget and
    /// the ceiling clock start again from nothing, and every pending nudge, rest and stop goes with
    /// them (round 2 CQ #1) — the final stop is not supervised further.
    pub(crate) fn end_recovery_episode(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts
                SET attempts = 0, recovery_started_at = NULL, asked_at = NULL,
                    last_progress_at = NULL, next_look = NULL, stop_requested_at = NULL,
                    stop_signals = 0, confirmed_until = NULL, confirmations = 0,
                    rest_total_secs = 0
              WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// **The owed turn was answered** (nxf 6j6v.1r61): its budget and every field of its episode go
    /// in ONE update — the reply discharged the slot, so whatever is owed on it next is a new turn.
    pub(crate) fn end_the_owed_turn(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts
                SET attempts = 0, recovery_started_at = NULL, asked_at = NULL,
                    confirmed_until = NULL, confirmations = 0, rest_total_secs = 0,
                    stop_requested_at = NULL, stop_signals = 0
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
        let Some((start, end)) = self.slot_turn(session, thread)? else {
            return Ok(None);
        };
        let conn = self.connection();
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

    /// **The session's OWN life on the slot** (nxf 6j6v.1r61, design 8nx4 section 1, `L(a)`): the
    /// newest instant in the slot's current span among the entries the MODEL produced — `assistant`,
    /// `thinking`, `tool_use`, and a `tool_result` only when its `tool_use` lies in the same span.
    /// The sidecar's `turn_start` (written for every opener, a continuation and an ask included),
    /// the runtime's `session_init`, `result` and `error` are bookkeeping, never life — which is
    /// what keeps a resumed session from reading "alive" before its model has said a word. The
    /// unfiltered reading stays [`slot_last_activity`](ChatStore::slot_last_activity), which `nxc
    /// status` and rung E's summary show.
    pub(crate) fn slot_last_own_output(
        &self,
        session: &str,
        thread: &str,
    ) -> Result<Option<String>> {
        self.own_output(session, thread, true, None)
    }

    /// [`slot_last_own_output`](ChatStore::slot_last_own_output) as a look at `now` reads it: the
    /// newest model output at or before `now` — an entry dated later is no evidence of life yet
    /// (review of pass 1, Integrity #4).
    pub(crate) fn slot_last_own_output_by(
        &self,
        session: &str,
        thread: &str,
        now: &str,
    ) -> Result<Option<String>> {
        self.own_output(session, thread, true, Some(now))
    }

    /// [`slot_last_own_output`](ChatStore::slot_last_own_output) without the `tool_result`s — the
    /// MODEL's output only, what "life since a confirmation" counts (round 3 IR #4): a tool's
    /// result arriving says the tool ended, not that the model is alive.
    pub(crate) fn slot_last_model_output(
        &self,
        session: &str,
        thread: &str,
        now: &str,
    ) -> Result<Option<String>> {
        self.own_output(session, thread, false, Some(now))
    }

    fn own_output(
        &self,
        session: &str,
        thread: &str,
        results: bool,
        by: Option<&str>,
    ) -> Result<Option<String>> {
        let Some((start, end)) = self.slot_turn(session, thread)? else {
            return Ok(None);
        };
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT e.at FROM agent_transcript e
              WHERE e.internal_session = ?1 AND e.seq >= ?2 AND (?3 IS NULL OR e.seq < ?3)
                AND e.at IS NOT NULL
                AND ( e.kind IN ('assistant', 'thinking', 'tool_use')
                   OR ( ?4 AND e.kind = 'tool_result' AND e.tool_use_id IS NOT NULL AND EXISTS (
                          SELECT 1 FROM agent_transcript u
                           WHERE u.internal_session = ?1 AND u.kind = 'tool_use'
                             AND u.tool_use_id = e.tool_use_id
                             AND u.seq >= ?2 AND (?3 IS NULL OR u.seq < ?3))))",
        )?;
        let ats = st
            .query_map(params![session, start, end, results], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(latest(
            ats.iter()
                .map(String::as_str)
                .filter(|at| by.is_none_or(|now| !earlier(now, at))),
        )
        .map(str::to_string))
    }

    /// **The span of the slot's own turn** in `session`'s transcript: from the newest `turn_start {
    /// thread }` of THAT slot (inclusive) up to the next `turn_start` of any other slot (exclusive,
    /// `None` while there is none). `None` when the slot has no turn of its own on record.
    fn slot_turn(&self, session: &str, thread: &str) -> Result<Option<(i64, Option<i64>)>> {
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
        Ok(Some((start, end)))
    }

    /// **The tool call the slot's turn is inside** (nxf 6j6v.j6yk), when there is one: the oldest
    /// `tool_use` of the main conversation in the slot's turn that has no `tool_result` on record
    /// and that the model has not spoken past. A model message after a call means its result came
    /// back — the model only speaks again once it has it — so a call whose result the transcript
    /// lost is not held open for ever by that loss. A subagent's own calls are not the session's: the
    /// `Task` call that spawned it is, and is open while the subagent works.
    pub(crate) fn slot_open_tool_call(
        &self,
        session: &str,
        thread: &str,
    ) -> Result<Option<OpenToolCall>> {
        let Some((start, end)) = self.slot_turn(session, thread)? else {
            return Ok(None);
        };
        Ok(self
            .connection()
            .query_row(
                "SELECT u.at, COALESCE(json_extract(u.data, '$.name'), '') FROM agent_transcript u
                  WHERE u.internal_session = ?1 AND u.seq >= ?2 AND (?3 IS NULL OR u.seq < ?3)
                    AND u.kind = 'tool_use' AND u.parent_tool_use_id IS NULL
                    AND u.tool_use_id IS NOT NULL AND u.at IS NOT NULL
                    AND NOT EXISTS (SELECT 1 FROM agent_transcript r
                                     WHERE r.internal_session = ?1 AND r.kind = 'tool_result'
                                       AND r.tool_use_id = u.tool_use_id)
                    AND NOT EXISTS (SELECT 1 FROM agent_transcript m
                                     WHERE m.internal_session = ?1 AND m.seq > u.seq
                                       AND m.parent_tool_use_id IS NULL
                                       AND m.kind IN ('assistant', 'thinking', 'result', 'error',
                                                      'turn_start'))
                  ORDER BY u.seq LIMIT 1",
                params![session, start, end],
                |r| {
                    Ok(OpenToolCall {
                        since: r.get(0)?,
                        tool: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// The newest hold of `session`, standing or not, with the outcome its resume recorded.
    pub(crate) fn latest_hold(&self, session: &str) -> Result<Option<HoldRow>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT id, at, until, resumed_at, outcome FROM session_interruption
                  WHERE session = ?1 ORDER BY id DESC LIMIT 1",
                [session],
                |r| {
                    let outcome: Option<String> = r.get(4)?;
                    Ok(HoldRow {
                        id: r.get(0)?,
                        at: r.get(1)?,
                        until: r.get(2)?,
                        resumed_at: r.get(3)?,
                        outcome: outcome.map(|o| HoldOutcome::parse(&o).ok_or(o)),
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

    /// Record what the resume did with a hold.
    pub(crate) fn set_hold_outcome(&mut self, id: i64, outcome: HoldOutcome) -> Result<()> {
        self.connection().execute(
            "UPDATE session_interruption SET outcome = ?2 WHERE id = ?1",
            params![id, outcome.as_str()],
        )?;
        Ok(())
    }
}

/// **A tool call still running** — when it began and which tool it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenToolCall {
    pub since: String,
    pub tool: String,
}

/// A hold as the ladder reads it — [`crate::interruption::Interruption`] plus its outcome. A word
/// a newer build wrote reads as `Some(Err(word))`: decided, by somebody this build does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HoldRow {
    pub id: i64,
    pub at: String,
    pub until: Option<String>,
    pub resumed_at: Option<String>,
    pub outcome: Option<std::result::Result<HoldOutcome, String>>,
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

/// **The nudge** (design 8nx4 section 3) — the one place `nxc confirm` is ever named. `silent` is
/// the minutes since the session's last life, `rest_left` the minutes of rest its episode has left.
pub fn ask_text(thread: &str, silent_minutes: i64, rest_left_minutes: i64) -> String {
    format!(
        "Coordinator of thread {thread}: no sign of life from you for {silent_minutes} minutes. If \
         you have already replied on thread {thread}, do nothing and end your turn. If the work is \
         done: `nxc reply --thread {thread} -`. If you are still working and need more time: `nxc \
         confirm --thread {thread} --rest <minutes>` (at most {REST_MAX_MINUTES}; \
         {rest_left_minutes} minutes of rest remain). If you cannot finish: `nxc reply --thread \
         {thread} --escalate -`."
    )
}

/// **The continue** (rung C) — names the thread and quotes the task; offers no `nxc confirm`.
pub fn continue_text(thread: &str, commission: &str) -> String {
    format!(
        "Your turn for thread {thread} ended without a reply; the task was: {}. If the work is done, \
         send it with `nxc reply --thread {thread} -`; if not, continue and reply when it is; if you \
         cannot, `nxc reply --thread {thread} --escalate -` and say why.",
        first_line(commission)
    )
}

/// **Why the session is being continued, when the ladder stopped it** (design 8nx4 section 4, step
/// 6): the cause — no sign of life for N minutes after it was nudged, or since its start — and
/// every descendant of the stopped process that is still running, read again by pid and start.
/// A survivor is named, never signalled (htdw): one holding a lock or a port is the first thing a
/// resumed session needs to know. `None` when no stop of the ladder ended the process.
fn stop_cause(ctx: &Ctx, store: &ChatStore, row: &OwedStart) -> Option<String> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let stop = store
        .newest_rung_of(t, h, RecoveryAction::Stopped)
        .ok()
        .flatten()
        .filter(|s| !earlier(&s.at, &row.trigger_clock))?;
    let (since, what) = match row.asked_at.as_deref() {
        Some(asked) => (asked, "after it was asked"),
        None => (row.trigger_clock.as_str(), "after its process started"),
    };
    let minutes = seconds_between(since, &stop.at).unwrap_or(0).max(0) / 60;
    let mut cause = format!(
        "Your session showed no sign of life for {minutes} minutes {what} and was stopped at {}.",
        stop.at
    );
    let survivors: Vec<String> = descendants_of(&stop.detail)
        .into_iter()
        .filter(|m| ctx.worker.process_survives(m))
        .map(|m| {
            format!(
                "pid {} (started {})",
                m.pid,
                m.started.as_deref().unwrap_or("at an unknown instant")
            )
        })
        .collect();
    if !survivors.is_empty() {
        cause.push_str(&format!(
            " Still running from the stopped process, and not signalled: {} — it may hold a lock, \
             a port or a file you need.",
            survivors.join(", ")
        ));
    }
    Some(cause)
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
    Continue,
    Recommission,
}

impl Rung {
    fn action(self) -> RecoveryAction {
        match self {
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
        // **The ceiling, whatever the look judged** (nxf 6j6v.yts5): a slot waiting out a grace is
        // still in its episode.
        Judgement::Pending => {
            if let Some(ceiling) = ceiling_reached(store, row, ctx.now)? {
                escalate(ctx, store, row, coordinator, ceiling, &mut findings)?;
            }
        }
        // ... and a slot whose answer is still coming by another route: that route's clock may fire
        // past the ceiling, so the slot's look is armed AT the ceiling — the one look the sweep arms
        // for a slot it otherwise leaves alone, and only while an episode is under way.
        Judgement::Coming => match ceiling_reached(store, row, ctx.now)? {
            Some(ceiling) => escalate(ctx, store, row, coordinator, ceiling, &mut findings)?,
            None => {
                if let Some(at) = ceiling_instant(store, row)? {
                    arm(ctx, store, &row.thread, &row.role, &at, &mut findings);
                }
            }
        },
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
        h.outcome == Some(Ok(HoldOutcome::ProviderCannotResume))
            && !earlier(&h.at, &row.trigger_clock)
    }))
}

// ---- the look at a live session: the state machine of design 8nx4 section 4 (nxf 6j6v.1r61) ------

/// **Where a sign of the session's own life came from** — what a `recovered` row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    /// A model entry of the slot's span ([`ChatStore::slot_last_own_output`]).
    Entry,
    /// The sidecar's pulse ([`crate::worker::Worker::session_pulse`]).
    Pulse,
    /// The open call's subprocesses moved ([`crate::worker::Worker::subprocess_sample`]).
    Subprocess,
}

impl Signal {
    fn word(self) -> &'static str {
        match self {
            Signal::Entry => "entry",
            Signal::Pulse => "pulse",
            Signal::Subprocess => "subprocess",
        }
    }
}

/// **What one look knows about a live session's life** (design 8nx4 sections 1 and 2).
///
/// `L` ([`Life::own`]) is the session's OWN life: a model entry of the slot's current span, the
/// pulse, a movement of the open call's subprocesses — never `trigger_clock`, never a `turn_start`.
/// `L(a)` is per current process span by design, and `S = max(L, P)` floors `due` at `P + 600`, so
/// no nudge comes earlier than ten minutes after the process's start (round 3 CQ #5). `P`
/// ([`Life::process_start`]) is the row's `trigger_clock`, which only a process start moves.
struct Life {
    /// `L` and where it came from — `M`, the last movement of the open call's subprocesses, among
    /// its candidates (present only while an open call stands, the host samples it and a sample
    /// with a process under the call has been stored).
    own: Option<(String, Signal)>,
    /// `P`.
    process_start: String,
    /// `max(S + SILENCE_SECS, M + SUBPROCESS_IDLE_SECS)`; on a host that cannot sample an open
    /// call, the call's own bound (today's rule: no nudge while the call stands).
    due: String,
    pulse: crate::worker::Pulse,
    /// The host answered `None` for the open call — the signal the escalation names.
    blind_call: bool,
    /// `S = max(L, P)`.
    silent_since: String,
}

impl Life {
    /// **Observe the session at this look** — and STORE the open call's subprocess sample on the
    /// way, since `M` is the instant the stored sample last moved: an observation, not a query.
    fn observe(
        ctx: &Ctx,
        store: &mut ChatStore,
        row: &OwedStart,
        session: &str,
        call: Option<&OpenToolCall>,
    ) -> Result<Life> {
        let (t, h) = (row.thread.as_str(), row.role.as_str());
        let entry = store.slot_last_own_output_by(session, t, ctx.now)?;
        // **The pulse only for a session whose process runs** (review of d1ey, CQ #2 / IR #3): the
        // file outlives its process, and its `At` would read as life of a process that is gone.
        // The judge said `Live` a moment ago; asked again here so the order holds whoever calls.
        let pulse = if ctx.worker.session_is_running(session) {
            ctx.worker.session_pulse(session)
        } else {
            crate::worker::Pulse::Unreported
        };
        let mut moved_at = None;
        let mut blind_call = false;
        if let Some(call) = call {
            match ctx.worker.subprocess_sample(session, &call.since) {
                Some(sample) => {
                    let moved =
                        store.store_subprocess_sample(t, h, row, &call.since, sample, ctx.now)?;
                    moved_at = moved.filter(|m| !earlier(m, &call.since));
                }
                None => blind_call = true,
            }
        }
        let mut own: Option<(String, Signal)> = None;
        let candidates = [
            entry.map(|e| (e, Signal::Entry)),
            match &pulse {
                crate::worker::Pulse::At(at) => Some((at.clone(), Signal::Pulse)),
                _ => None,
            },
            moved_at.clone().map(|m| (m, Signal::Subprocess)),
        ];
        // An instant later than this look is no evidence of life yet — a skewed clock or a forged
        // file would otherwise push the silence out to the ceiling (review of pass 1, IR #4).
        for (at, signal) in candidates
            .into_iter()
            .flatten()
            .filter(|(at, _)| !earlier(ctx.now, at))
        {
            if own.as_ref().is_none_or(|(best, _)| earlier(best, &at)) {
                own = Some((at, signal));
            }
        }
        let process_start = row.trigger_clock.clone();
        let silent_since = match &own {
            Some((l, _)) if earlier(&process_start, l) => l.clone(),
            _ => process_start.clone(),
        };
        let due = match call {
            Some(call) if blind_call => {
                after(&call.since, TOOL_CALL_BOUND_SECS, "the tool call's bound")?
            }
            _ => {
                let silence = after(&silent_since, SILENCE_SECS, "the silence clock")?;
                match &moved_at {
                    Some(m) => {
                        let idle = after(m, SUBPROCESS_IDLE_SECS, "the subprocess clock")?;
                        if earlier(&silence, &idle) {
                            idle
                        } else {
                            silence
                        }
                    }
                    None => silence,
                }
            }
        };
        Ok(Life {
            own,
            process_start,
            due,
            pulse,
            blind_call,
            silent_since,
        })
    }

    /// `L ≥ instant` — life of its own at or after `instant`.
    fn since(&self, instant: &str) -> bool {
        self.own.as_ref().is_some_and(|(l, _)| !earlier(l, instant))
    }

    /// `L ≥ P` — the session has shown life of its own since its process started.
    fn since_its_start(&self) -> bool {
        self.since(&self.process_start)
    }
}

/// **Take back every nudge waiting for `session`** — the worker unlinks ALL of its ask files, so
/// every other slot of the session that was waiting on one is told so: its `asked_at` goes, and
/// its next look nudges it again instead of finding it dead for not reading a nudge that is gone
/// (review of pass 1, Integrity #2). `thread` is the slot the withdrawal is for, which the caller
/// clears itself. The rows are cleared even when the unlink failed: the sidecar's own drain may
/// still deliver a file, and a fresh nudge after a stale one is the harmless direction.
pub(crate) fn withdraw_the_nudges(
    ctx: &Ctx,
    store: &mut ChatStore,
    session: &str,
    thread: &str,
) -> std::result::Result<(), String> {
    let withdrawn = ctx.worker.withdraw_asks(session);
    if let Err(e) = store.forget_the_other_nudges(session, thread) {
        eprintln!(
            "warning: the nudges of session {session} were withdrawn, but its other slots could \
             not be told: {e}; one may wait on a nudge that will not come"
        );
    }
    withdrawn
}

/// The earlier of `at` and `recheck` — no look is armed later than one cadence out — and never an
/// instant already past.
fn soonest(ctx: &Ctx, at: &str, recheck: &str) -> String {
    if earlier(at, recheck) && earlier(ctx.now, at) {
        at.to_string()
    } else {
        recheck.to_string()
    }
}

/// **The look at a live session** (design 8nx4 section 4, steps 0–6) — for a coordinated slot whose
/// session holds a live process. Each step is its own function below; this one decides which.
fn live(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let t = row.thread.as_str();
    let session = row.session.as_deref().unwrap_or_default();
    let recheck = after(
        ctx.now,
        crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
        "the next look",
    )?;
    let call = store.slot_open_tool_call(session, t)?;
    // Step 0 — the caps, unchanged: a call past its own bound is hung; the ceiling.
    if let Some(call) = &call {
        if reached(
            ctx.now,
            &after(&call.since, TOOL_CALL_BOUND_SECS, "the tool call's bound")?,
        ) {
            return escalate(ctx, store, row, coordinator, hung_call(call), findings);
        }
    }
    if let Some(ceiling) = ceiling_reached(store, row, ctx.now)? {
        return escalate(ctx, store, row, coordinator, ceiling, findings);
    }
    let life = Life::observe(ctx, store, row, session, call.as_ref())?;
    if row.stop_requested_at.is_some() {
        return step_stop_in_flight(ctx, store, row, coordinator, &recheck, findings);
    }
    if row.confirmed_until.is_some() {
        return step_rest(ctx, store, row, coordinator, &life, &recheck, findings);
    }
    if row.asked_at.is_some() {
        return step_nudge_pending(ctx, store, row, coordinator, &life, &recheck, findings);
    }
    if reached(ctx.now, &life.due) {
        return step_due(ctx, store, row, coordinator, &life, &recheck, findings);
    }
    step_alive(ctx, store, row, &life, &recheck, findings)
}

/// The occasion of a tool call past its own bound.
fn hung_call(call: &OpenToolCall) -> Occasion {
    Occasion {
        observed: Observation::Silent,
        judged_from: call.since.clone(),
        detail: format!(
            "its process is alive but has been inside one tool call ({}) since {} — past the bound \
             of {} h for a single tool call, so the call is taken as hung; a nudge would only reach \
             the session once the call returns",
            call.tool,
            call.since,
            TOOL_CALL_BOUND_SECS / 3600
        ),
    }
}

/// **Step 1 — a stop is in flight** and the process still runs: the one further SIGTERM at the
/// grace, the escalation at twice the grace ("would not stop"). Before everything else, so no
/// confirmation and no rest can cancel a stop. No SIGKILL (htdw).
fn step_stop_in_flight(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let session = row.session.as_deref().unwrap_or_default();
    let stop = row.stop_requested_at.as_deref().unwrap_or(ctx.now);
    let again = after(stop, STOP_GRACE_SECS, "the stop's grace")?;
    let give_up = after(stop, 2 * STOP_GRACE_SECS, "the stop's second grace")?;
    if reached(ctx.now, &give_up) {
        let pid = store
            .newest_rung_of(t, h, RecoveryAction::Stopped)?
            .and_then(|r| process_of(&r.detail))
            .map_or_else(|| "its pid unknown".to_string(), |p| format!("pid {p}"));
        let occasion = Occasion {
            observed: Observation::Silent,
            judged_from: stop.to_string(),
            detail: format!(
                "it was stopped at {stop} and its process would not stop ({pid}); it was signalled \
                 twice and is left running — no further signal is sent"
            ),
        };
        return escalate(ctx, store, row, coordinator, occasion, findings);
    }
    if reached(ctx.now, &again) && row.stop_signals < 2 {
        let outcome = stop_outcome(ctx.worker.stop_session(session));
        let id = store.log_rung(
            t,
            h,
            ctx.now,
            RecoveryAction::Stopped,
            row.attempts,
            Some(session),
            &format!("the one further SIGTERM: the process still ran {STOP_GRACE_SECS} s after the stop of {stop}"),
        )?;
        store.settle_rung(id, &outcome)?;
        store.note_second_signal(t, h)?;
        arm(ctx, store, t, h, &soonest(ctx, &give_up, recheck), findings);
        return Ok(());
    }
    let next = if reached(ctx.now, &again) {
        give_up
    } else {
        again
    };
    arm(ctx, store, t, h, &soonest(ctx, &next, recheck), findings);
    Ok(())
}

/// **Step 2 — a rest stands** (`nxc confirm --rest`): nothing before its end plus the grace. Then
/// the rest is cleared; life since the confirmation — model output or a pulse, never a
/// `tool_result` (round 3 IR #4) — is step 5; otherwise the confirmations or the rests are spent
/// (stuck), or the session is nudged again.
fn step_rest(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    life: &Life,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let session = row.session.as_deref().unwrap_or_default();
    let until = row.confirmed_until.as_deref().unwrap_or(ctx.now);
    let look = after(until, CONFIRM_GRACE_SECS, "the rest's grace")?;
    if !reached(ctx.now, &look) {
        arm(ctx, store, t, h, &soonest(ctx, &look, recheck), findings);
        return Ok(());
    }
    store.clear_rest(t, h)?;
    let confirmed_at = store
        .newest_rung_of(t, h, RecoveryAction::Confirmed)?
        .map_or_else(|| until.to_string(), |r| r.at);
    let model_since = store
        .slot_last_model_output(session, t, ctx.now)?
        .is_some_and(|m| earlier(&confirmed_at, &m));
    let pulse_since =
        matches!(&life.pulse, crate::worker::Pulse::At(p) if earlier(&confirmed_at, p));
    let row = store.owed_start(t, h)?.unwrap_or_else(|| row.clone());
    if model_since || pulse_since {
        return step_alive(ctx, store, &row, life, recheck, findings);
    }
    if row.confirmations >= CONFIRMATIONS_MAX || row.rest_total_secs >= CONFIRM_TOTAL_CAP_SECS {
        let rests: Vec<String> = store
            .slot_recovery_log(t, h)?
            .into_iter()
            .filter(|r| r.action == RecoveryAction::Confirmed.as_str())
            .filter(|r| {
                row.recovery_started_at
                    .as_deref()
                    .is_some_and(|s| !earlier(&r.at, s))
            })
            .map(|r| format!("{} at {}", r.detail, r.at))
            .collect();
        let occasion = Occasion {
            observed: Observation::Silent,
            judged_from: life.silent_since.clone(),
            detail: format!(
                "stuck: it confirmed {} times ({}) for {} min of rest in all, and showed no life \
                 of its own after its last rest ended at {until}",
                row.confirmations,
                rests.join("; "),
                row.rest_total_secs / 60
            ),
        };
        return escalate(ctx, store, &row, coordinator, occasion, findings);
    }
    nudge(ctx, store, &row, coordinator, life, recheck, findings)
}

/// **Step 3 — a nudge is pending.** No life since it: dead at the nudge's wait. Life since it: the
/// session is working and has not read it — withdrawn once it is as old as a silence (overtaken),
/// and the look goes on to step 5.
fn step_nudge_pending(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    life: &Life,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let session = row.session.as_deref().unwrap_or_default();
    let asked = row.asked_at.as_deref().unwrap_or(ctx.now);
    if !life.since(asked) {
        let wait = after(asked, NUDGE_WAIT_SECS, "the nudge's wait")?;
        if reached(ctx.now, &wait) {
            return step_dead(ctx, store, row, coordinator, life, recheck, findings);
        }
        arm(ctx, store, t, h, &soonest(ctx, &wait, recheck), findings);
        return Ok(());
    }
    let overtaken = after(asked, SILENCE_SECS, "the nudge's life")?;
    if !reached(ctx.now, &overtaken) {
        arm(
            ctx,
            store,
            t,
            h,
            &soonest(ctx, &overtaken, recheck),
            findings,
        );
        return Ok(());
    }
    if let Err(e) = withdraw_the_nudges(ctx, store, session, t) {
        eprintln!(
            "warning: the overtaken nudge of session {session} on thread {t} could not be \
             withdrawn: {e}; the sidecar drops it at its next yield only if it is gone"
        );
    }
    store.set_asked_cleared(t, h)?;
    if let Some(asked_row) = store.newest_rung_of(t, h, RecoveryAction::Asked)? {
        let (l, signal) = own_or_unknown(life);
        store.settle_rung_with_detail(
            asked_row.id,
            RungOutcome::Overtaken.as_str(),
            &format!(
                "{}; overtaken by its own life at {l} ({})",
                asked_row.detail,
                signal.word()
            ),
        )?;
    }
    let row = store.owed_start(t, h)?.unwrap_or_else(|| row.clone());
    step_alive(ctx, store, &row, life, recheck, findings)
}

/// `L` and its signal, as a log row names them — never absent where it is asked for, but said
/// plainly if it were.
fn own_or_unknown(life: &Life) -> (String, Signal) {
    life.own
        .clone()
        .unwrap_or_else(|| ("an unknown instant".to_string(), Signal::Entry))
}

/// **Step 4 — no nudge pending and the silence is due.** A session with life of its own since its
/// start — or on a host whose sidecar reports no pulse, or whose pulse predates the start (round 3
/// TQ #2 / IR #1, the generous side) — is nudged; one whose fresh process has produced nothing at
/// all (pulse `NoneYet`) is dead without a nudge, since a nudge is read only at a turn's `result`.
fn step_due(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    life: &Life,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    if life.pulse == crate::worker::Pulse::NoneYet && !life.since_its_start() {
        return step_dead(ctx, store, row, coordinator, life, recheck, findings);
    }
    nudge(ctx, store, row, coordinator, life, recheck, findings)
}

/// **The nudge** (step 4a): through the ask door, with the one text that offers `nxc confirm`.
/// `Asked` opens the episode if none is open and counts no attempt; a door that stays closed past
/// the nudge's wait is dead. A `cannot_ask` row opens no episode (round 3 CQ #4 / IR #5).
fn nudge(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    life: &Life,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let Some(session) = row.session.as_deref() else {
        arm(ctx, store, t, h, recheck, findings);
        return Ok(());
    };
    let silent_minutes = seconds_between(&life.silent_since, ctx.now).unwrap_or(SILENCE_SECS) / 60;
    let rest_left = (CONFIRM_TOTAL_CAP_SECS - row.rest_total_secs).max(0) / 60;
    let ask = SessionAsk {
        session: session.to_string(),
        thread: t.to_string(),
        role: h.to_string(),
        text: ask_text(t, silent_minutes, rest_left),
        at: ctx.now.to_string(),
    };
    let detail = format!(
        "its process is alive and showed no life of its own on this slot since {}",
        life.silent_since
    );
    match ctx.worker.ask_session(&ask) {
        AskOutcome::Asked => {
            let id = store.log_rung(
                t,
                h,
                ctx.now,
                RecoveryAction::Asked,
                row.attempts,
                Some(session),
                &detail,
            )?;
            store.settle_rung_as(id, RungOutcome::Asked)?;
            store.set_asked(t, h, ctx.now)?;
            let wait = after(ctx.now, NUDGE_WAIT_SECS, "the nudge's wait")?;
            arm(ctx, store, t, h, &soonest(ctx, &wait, recheck), findings);
            Ok(())
        }
        AskOutcome::NoProcess => {
            // The judge finds no process at the next look.
            arm(ctx, store, t, h, recheck, findings);
            Ok(())
        }
        _ => {
            let closed_past = after(&life.due, NUDGE_WAIT_SECS, "the closed door's wait")?;
            if reached(ctx.now, &closed_past) {
                return step_dead(ctx, store, row, coordinator, life, recheck, findings);
            }
            // One row per closed door, not one per cadence.
            let logged = store.newest_rung(t, h)?.is_some_and(|r| {
                r.action == RecoveryAction::Asked.as_str()
                    && r.outcome.as_deref() == Some(RungOutcome::CannotAsk.as_str())
                    && !earlier(&r.at, &life.due)
            });
            if !logged {
                let id = store.log_rung(
                    t,
                    h,
                    ctx.now,
                    RecoveryAction::Asked,
                    row.attempts,
                    Some(session),
                    &detail,
                )?;
                store.settle_rung_as(id, RungOutcome::CannotAsk)?;
            }
            arm(ctx, store, t, h, recheck, findings);
            Ok(())
        }
    }
}

/// **Step 5 — alive.** With an episode open, nothing pending and life of its own since its start,
/// the episode ends by life: the `recovered` row, alone in its look, and the episode's clears —
/// never `attempts`, the owed turn's budget.
fn step_alive(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    life: &Life,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let nothing_stands =
        row.asked_at.is_none() && row.confirmed_until.is_none() && row.stop_requested_at.is_none();
    if row.recovery_started_at.is_some() && nothing_stands && life.since_its_start() {
        let (l, signal) = own_or_unknown(life);
        let id = store.log_rung(
            t,
            h,
            ctx.now,
            RecoveryAction::Recovered,
            row.attempts,
            row.session.as_deref(),
            &format!("its own life at {l} ({})", signal.word()),
        )?;
        store.settle_rung_as(id, RungOutcome::Life)?;
        store.end_episode_by_life(t, h)?;
    }
    let due = if reached(ctx.now, &life.due) {
        recheck.to_string()
    } else {
        life.due.clone()
    };
    arm(ctx, store, t, h, &soonest(ctx, &due, recheck), findings);
    Ok(())
}

/// **Step 6 — dead.** The process is stopped (SIGTERM), the stop is logged with the descendants
/// sampled for the continue text, and THEN the cap: an owed turn that already used its resumes
/// escalates at this look — never through [`run`]'s pre-check, which would escalate without
/// stopping (round 2 TQ #5). Otherwise the judge finds no process next, and the rung after an end
/// continues or re-commissions it.
fn step_dead(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    coordinator: &CoordinatorRef,
    life: &Life,
    recheck: &str,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let session = row.session.as_deref().unwrap_or_default();
    let tree = ctx.worker.session_process_tree(session);
    let outcome = stop_outcome(ctx.worker.stop_session(session));
    let why = match &row.asked_at {
        Some(asked) => format!(
            "no life of its own since it was nudged at {asked} (silent since {})",
            life.silent_since
        ),
        None => format!(
            "its process started at {} and produced nothing of its own since",
            life.process_start
        ),
    };
    let id = store.log_rung(
        t,
        h,
        ctx.now,
        RecoveryAction::Stopped,
        row.attempts,
        Some(session),
        &stop_detail(&why, tree.as_ref(), life),
    )?;
    store.settle_rung(id, &outcome)?;
    store.note_stop_requested(t, h, ctx.now)?;
    let allowed = dead_resumes(ctx);
    if row.attempts >= allowed {
        let row = store.owed_start(t, h)?.unwrap_or_else(|| row.clone());
        let occasion = Occasion {
            observed: Observation::Silent,
            judged_from: life.silent_since.clone(),
            detail: format!(
                "{why}; it was stopped at {} — dead after {} recovery attempts on this turn; this \
                 workspace allows {allowed}",
                ctx.now, row.attempts
            ),
        };
        return escalate(ctx, store, &row, coordinator, occasion, findings);
    }
    arm(ctx, store, t, h, recheck, findings);
    Ok(())
}

/// A stop's answer as its log row's outcome.
fn stop_outcome(stop: std::result::Result<crate::worker::SessionStop, String>) -> String {
    match stop {
        Ok(crate::worker::SessionStop::Requested) => {
            RungOutcome::StopRequested.as_str().to_string()
        }
        Ok(crate::worker::SessionStop::NothingToStop(_)) => {
            RungOutcome::NothingToStop.as_str().to_string()
        }
        Err(e) => failed_outcome(&e),
    }
}

/// The marker before the descendants a `stopped` row names — with its leading separator, so no
/// reason text can contain it (review of pass 1, Integrity #3).
const DESCENDANTS: &str = "; descendants at the stop: ";
/// The marker before the stopped process's own pid, likewise: a reason may say "its process
/// started at …", never "; stopped pid ".
const PROCESS: &str = "; stopped pid ";

/// **A `stopped` row's detail** — why, the process and the descendants sampled at the stop, each
/// as `pid@start`, which the continue text re-reads to name a survivor ([`process_of`],
/// [`descendants_of`]); and "this host reports no …" for each signal the host could not read.
fn stop_detail(why: &str, tree: Option<&crate::worker::SessionProcessTree>, life: &Life) -> String {
    let mut out = describe_the_stop(why, tree);
    if life.pulse == crate::worker::Pulse::Unreported {
        out.push_str("; this host reports no pulse");
    }
    if life.blind_call {
        out.push_str("; this host reports no subprocess signals");
    }
    out
}

/// The reason, the stopped process and its descendants, as a `stopped` row's detail carries them.
fn describe_the_stop(why: &str, tree: Option<&crate::worker::SessionProcessTree>) -> String {
    let mut out = why.to_string();
    match tree {
        Some(tree) => {
            out.push_str(&format!("{PROCESS}{}", tree.pid));
            let marks: Vec<String> = tree
                .descendants
                .iter()
                .map(|d| format!("{}@{}", d.pid, d.started.as_deref().unwrap_or("?")))
                .collect();
            out.push_str(&format!(
                "{DESCENDANTS}{}",
                if marks.is_empty() {
                    "none".to_string()
                } else {
                    marks.join(" ")
                }
            ));
        }
        None => out.push_str("; this host reports no process tree"),
    }
    out
}

/// The stopped process's pid a `stopped` row named.
fn process_of(detail: &str) -> Option<u32> {
    let rest = &detail[detail.find(PROCESS)? + PROCESS.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// The descendants a `stopped` row named.
fn descendants_of(detail: &str) -> Vec<crate::worker::ProcessMark> {
    let Some(at) = detail.find(DESCENDANTS) else {
        return Vec::new();
    };
    let rest = &detail[at + DESCENDANTS.len()..];
    let rest = rest.split(';').next().unwrap_or("");
    rest.split_whitespace()
        .filter_map(|mark| {
            let (pid, started) = mark.split_once('@')?;
            Some(crate::worker::ProcessMark {
                pid: pid.parse().ok()?,
                started: (started != "?").then(|| started.to_string()),
            })
        })
        .collect()
}

/// Whole seconds from `a` to `b`, when both parse.
fn seconds_between(a: &str, b: &str) -> Option<i64> {
    use time::format_description::well_known::Rfc3339;
    let a = time::OffsetDateTime::parse(a, &Rfc3339).ok()?;
    let b = time::OffsetDateTime::parse(b, &Rfc3339).ok()?;
    Some((b - a).whole_seconds())
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
    let hold = match row.session.as_deref() {
        Some(session) => store.latest_hold(session)?,
        None => None,
    };
    // **The ceiling holds over a hold too** (nxf 6j6v.yts5): one that names its instant is the
    // resume's to take up, and the ceiling still fires at the ceiling — so its look is armed there,
    // and its escalation says the hold still stands.
    if let Some(mut ceiling) = ceiling_reached(store, row, ctx.now)? {
        if let Some(until) = hold
            .as_ref()
            .filter(|h| h.resumed_at.is_none())
            .and_then(|h| h.until.as_deref())
        {
            ceiling.detail = format!(
                "{}; ceiling reached while on hold until {until}; a resume may still answer",
                ceiling.detail
            );
        }
        return escalate(ctx, store, row, coordinator, ceiling, findings);
    }
    let (Some(session), Some(hold)) = (row.session.as_deref(), hold) else {
        return Ok(());
    };
    let waits_on_nobody = hold.until.is_none()
        && hold.resumed_at.is_none()
        && matches!(hold.outcome, None | Some(Ok(HoldOutcome::NotYet)))
        && !ctx.worker.session_is_running(session);
    if !waits_on_nobody {
        if let Some(ceiling) = ceiling_instant(store, row)? {
            arm(ctx, store, &row.thread, &row.role, &ceiling, findings);
        }
        return Ok(());
    }
    let due = after(&hold.at, STRANDED_GRACE_SECS, "the hold's grace")?;
    if !reached(ctx.now, &due) {
        arm(ctx, store, &row.thread, &row.role, &due, findings);
        return Ok(());
    }
    // Taken up by the ladder: the hold no longer stands once the session is continued.
    if store.mark_interruption_resumed(hold.id, ctx.now)? {
        store.set_hold_outcome(hold.id, HoldOutcome::Resumed)?;
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

/// **The ceiling, checked in one place**: the occasion of rung E when the episode's wall clock from
/// its first attempt has reached `clamp(3 × window, 2 h, 6 h)`, `None` before that or with no
/// episode under way.
fn ceiling_reached(store: &ChatStore, row: &OwedStart, now: &str) -> Result<Option<Occasion>> {
    let (Some(started), Some(at)) = (
        row.recovery_started_at.as_deref(),
        ceiling_instant(store, row)?,
    ) else {
        return Ok(None);
    };
    if !reached(now, &at) {
        return Ok(None);
    }
    let ceiling = recovery_ceiling_secs(declared_window(store, &row.thread)?);
    Ok(Some(Occasion {
        observed: Observation::Silent,
        judged_from: started.to_string(),
        detail: format!(
            "its recovery began at {started} and reached the ceiling of {} h without an answer",
            ceiling / 3600
        ),
    }))
}

/// **The instant the episode under way reaches its ceiling** — `None` with no episode under way.
/// What [`ceiling_reached`] compares against, and what no armed look may reach beyond
/// ([`crate::stranded::arm_look`]).
pub(crate) fn ceiling_instant(store: &ChatStore, row: &OwedStart) -> Result<Option<String>> {
    let Some(started) = row.recovery_started_at.as_deref() else {
        return Ok(None);
    };
    let ceiling = recovery_ceiling_secs(declared_window(store, &row.thread)?);
    after(started, ceiling, "the recovery ceiling").map(Some)
}

/// **The retry of a rung whose trigger failed** (nxf 6j6v.yts5) — the judgement of the slot's next
/// look when its newest rung, run on this clock or after it, failed: the start is known not to
/// have happened, so the look acts at once rather than waiting a grace and two looks for a process
/// that was never started. A failed continue asks for the rung after an end again; a failed
/// re-commission is a slot that never started. `None` when the newest rung did not fail.
pub(crate) fn retry_of_a_failed_rung(
    store: &ChatStore,
    row: &OwedStart,
) -> Result<Option<Judgement>> {
    let Some(last) = store.newest_rung(&row.thread, &row.role)? else {
        return Ok(None);
    };
    let Some(why) = last.outcome.as_deref().and_then(failure_of) else {
        return Ok(None);
    };
    if earlier(&last.at, &row.trigger_clock) {
        return Ok(None);
    }
    let observed = match RecoveryAction::parse(&last.action) {
        // A failed continue: the rung after an end is asked for again, and finds the continue.
        Some(RecoveryAction::Continued) => Observation::EndedWithoutAnswer,
        // A failed re-commission: no session was started for the slot.
        Some(RecoveryAction::Recommissioned) => Observation::NeverStarted,
        // Never settled failed: an ask settles asked, no process or cannot ask (`fire_the_rung`),
        // and an escalation or a progress row starts nothing. Read as a slot that never started, so
        // a row this build did not write still ends in a re-commission rather than in nothing.
        Some(
            RecoveryAction::Asked
            | RecoveryAction::Escalated
            | RecoveryAction::Progress
            | RecoveryAction::Stopped
            | RecoveryAction::Confirmed
            | RecoveryAction::Recovered,
        )
        | None => Observation::NeverStarted,
    };
    Ok(Some(Judgement::Record {
        observed,
        judged_from: last.at.clone(),
        detail: format!(
            "its {} at {} could not be started ({why}), so the next look tries again",
            last.action, last.at
        ),
    }))
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

/// How often a claim is renewed while its trigger runs, in milliseconds — half of [`CLAIM_SECS`].
static RENEW_EVERY_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(CLAIM_SECS as u64 * 500);

/// **Renew claims more often than every half claim** — for a host whose triggers it wants to watch
/// closely, and for the tests that run a trigger slower than the claim without waiting for one.
/// Each renewal still moves the claim's horizon by half a claim, so a shorter interval makes the
/// claim reach further sooner, never less far. Process-wide; the default is `CLAIM_SECS / 2`.
#[doc(hidden)]
pub fn renew_claims_every(every: std::time::Duration) {
    let ms = u64::try_from(every.as_millis()).unwrap_or(u64::MAX).max(1);
    RENEW_EVERY_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

/// **The claim, kept alive while a trigger runs** — renewed at every interval from a connection of
/// its own, so a trigger slower than the claim never lets a second look in. Each renewal moves the
/// horizon half a claim on from the look's own instant: `base + n × CLAIM_SECS / 2 + CLAIM_SECS`
/// after the `n`-th, which at the default interval is the wall clock's own reading, and which an
/// injected clock agrees with.
pub(crate) struct ClaimRenewal {
    stop: Option<std::sync::mpsc::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
    failures: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ClaimRenewal {
    fn start(ctx: &Ctx, thread: &str, role: &str) -> ClaimRenewal {
        ClaimRenewal::start_on(ctx.db_path, thread, role, ctx.now)
    }

    pub(crate) fn start_on(db: &str, thread: &str, role: &str, base: &str) -> ClaimRenewal {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (db, thread, role, base) = (
            db.to_string(),
            thread.to_string(),
            role.to_string(),
            base.to_string(),
        );
        let failures = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failed = failures.clone();
        let every = std::time::Duration::from_millis(
            RENEW_EVERY_MS.load(std::sync::atomic::Ordering::Relaxed),
        );
        let handle = std::thread::Builder::new()
            .name("nxc-claim-renewal".into())
            .spawn(move || {
                let mut renewals: i64 = 0;
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = rx.recv_timeout(every) {
                    renewals += 1;
                    let reach = renewals
                        .saturating_mul(CLAIM_SECS / 2)
                        .saturating_add(CLAIM_SECS);
                    let renewed = after(&base, reach, "the claim's renewal")
                        .map_err(|e| e.to_string())
                        .and_then(|until| {
                            renew_claim(&db, &thread, &role, &until).map_err(|e| e.to_string())
                        });
                    if let Err(e) = renewed {
                        failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            failures,
        }
    }

    /// How many renewals failed so far.
    #[cfg(test)]
    pub(crate) fn failures(&self) -> usize {
        self.failures.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// One renewal, on a connection of its own — only of a claim still standing (a released one is not
/// taken again).
fn renew_claim(db: &str, thread: &str, role: &str, until: &str) -> rusqlite::Result<usize> {
    let c = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.execute(
        "UPDATE owed_starts SET recovering_until = ?3
          WHERE thread = ?1 AND role = ?2 AND recovering_until IS NOT NULL",
        params![thread, role, until],
    )
}

impl Drop for ClaimRenewal {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let failed = self.failures.load(std::sync::atomic::Ordering::Relaxed);
        if failed > 0 {
            eprintln!(
                "warning: {failed} renewal(s) of a claim failed while its trigger ran; the rung's \
                 own log row is what kept a second rung out"
            );
        }
    }
}

/// What a rung's trigger did, for its log row and for what follows.
enum Done {
    /// It did something.
    Acted(RungOutcome),
    /// It did nothing and counts nothing.
    Nothing(RungOutcome),
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
    if let Some(ceiling) = ceiling_reached(store, row, ctx.now)? {
        return escalate(ctx, store, row, coordinator, ceiling, findings);
    }
    // A retry of this rung's failed start spends no new attempt, so the budget does not stop it.
    if row.attempts >= MAX_RECOVERY_ATTEMPTS && rung_to_retry(store, row, rung)?.is_none() {
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

/// **The rung whose failed start this one retries** — the slot's newest row, when it is this
/// rung's, failed, on the current clock, and has retries left ([`RUNG_START_RETRIES`]).
fn rung_to_retry(store: &ChatStore, row: &OwedStart, rung: Rung) -> Result<Option<RecoveryLogRow>> {
    Ok(store.newest_rung(&row.thread, &row.role)?.filter(|last| {
        last.action == rung.action().as_str()
            && last.outcome.as_deref().and_then(failure_of).is_some()
            && !earlier(&last.at, &row.trigger_clock)
            && last.retries < RUNG_START_RETRIES
    }))
}

/// The part of [`run`] that holds the claim: the recheck, the attempt, the log row, the lease hook,
/// the trigger, what follows — in that order, fail-closed. `None` when the recheck stopped it.
fn under_the_claim(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    rung: Rung,
    occasion: &Occasion,
    findings: &mut Vec<FailedConsequence>,
) -> Result<Option<Done>> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    if !recheck(ctx, store, row, findings)? {
        return Ok(None);
    }
    // The lease hook (cut 1g): the slot's lease still names this operation. Until 1g there is
    // nothing to check and nothing to renew.
    lease_hook(ctx, store, row);
    // **A failed start of this same rung is retried on its own row and attempt** (round 2 TQ #6),
    // up to [`RUNG_START_RETRIES`]; past that, a new rung counts a new attempt.
    let retried = rung_to_retry(store, row, rung)?;
    let counted = retried.is_none();
    let opened_the_episode = counted && row.recovery_started_at.is_none();
    let log = match retried {
        Some(last) => {
            store.retry_rung(last.id)?;
            last.id
        }
        None => {
            let attempt = store.begin_attempt(t, h, ctx.now)?;
            store.log_rung(
                t,
                h,
                ctx.now,
                rung.action(),
                attempt,
                row.session.as_deref(),
                &occasion.detail,
            )?
        }
    };
    let done = {
        let _renewal = ClaimRenewal::start(ctx, t, h);
        fire_the_rung(ctx, store, row, rung)
    };
    match &done {
        Done::Acted(o) | Done::Nothing(o) => store.settle_rung_as(log, *o)?,
        Done::Refused(why) => store.settle_rung(log, &format!("refused: {why}"))?,
        Done::Failed(why) => store.settle_rung(log, &failed_outcome(why))?,
    }
    after_the_rung(
        ctx,
        store,
        row,
        rung,
        &done,
        counted,
        opened_the_episode,
        findings,
    )?;
    Ok(Some(done))
}

/// **The recheck inside the claim**: the slot is still owed by this role, and no rung of its own is
/// still in flight — a log row with no outcome younger than [`TRIGGER_TIMEOUT_SECS`] refuses, an
/// older one is settled `timed_out`. `false` stops the rung.
fn recheck(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    findings: &mut Vec<FailedConsequence>,
) -> Result<bool> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let owed = store
        .thread_quorum(t, ctx.now)?
        .is_some_and(|q| outstanding_has(&q.outstanding, h));
    if !owed {
        return Ok(false);
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
            return Ok(false);
        }
        store.settle_rung_as(flying.id, RungOutcome::TimedOut)?;
    }
    Ok(true)
}

/// **What follows a rung**: an attempt that did nothing is taken back (a retry counted none), a
/// failure is a finding — and the next look is armed once, for when this outcome wants it.
#[allow(clippy::too_many_arguments)]
fn after_the_rung(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    rung: Rung,
    done: &Done,
    counted: bool,
    opened_the_episode: bool,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let recheck = crate::orchestration::SESSION_LIVENESS_RECHECK_SECS;
    let next = match done {
        Done::Nothing(_) => {
            if counted {
                store.revert_attempt(t, h, opened_the_episode)?;
            }
            Some(after(ctx.now, recheck, "the next look")?)
        }
        Done::Acted(_) => {
            // The trigger armed the slot's first look one grace out (`stranded::owe`); noted only.
            let next = after(ctx.now, STRANDED_GRACE_SECS, "the next look")?;
            store.set_next_look(t, h, &next)?;
            None
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
            Some(after(ctx.now, recheck, "the next look")?)
        }
        Done::Refused(_) => None,
    };
    if let Some(next) = next {
        arm(ctx, store, t, h, &next, findings);
    }
    Ok(())
}

/// **The lease hook every rung calls** (cut 1g, 6j6v.vh0z, fills it): renew the operation's lease
/// to a bound in the future, and say when it is held by another operation. A no-op until then.
fn lease_hook(_ctx: &Ctx, _store: &mut ChatStore, _row: &OwedStart) {}

/// The trigger of one rung.
fn fire_the_rung(ctx: &Ctx, store: &mut ChatStore, row: &OwedStart, rung: Rung) -> Done {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    match rung {
        Rung::Continue => {
            let Some(session) = row.session.as_deref() else {
                return recommission(ctx, store, row);
            };
            let commission = commissioning_body(store, t)
                .ok()
                .flatten()
                .unwrap_or_default();
            let mut text = continue_text(t, &commission);
            if let Some(cause) = stop_cause(ctx, store, row) {
                text = format!("{text}\n\n{cause}");
            }
            match crate::orchestration::ladder_continue(ctx, store, t, h, session, &text) {
                Ok(admission) => Done::Acted(admitted(&admission)),
                // The runtime no longer has the conversation: there is nothing to continue, so the
                // slot is started over with its task, in the same claim.
                Err(TriggerError::SessionGone { .. }) => match recommission(ctx, store, row) {
                    Done::Acted(_) => Done::Acted(RungOutcome::SessionGoneRecommissioned),
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

fn admitted(admission: &crate::orchestration::TriggerAdmission) -> RungOutcome {
    match admission {
        crate::orchestration::TriggerAdmission::Queued { .. } => RungOutcome::Queued,
        _ => RungOutcome::Triggered,
    }
}

fn classify(e: TriggerError) -> Done {
    match e {
        TriggerError::AlreadyRunning { .. } => Done::Nothing(RungOutcome::AlreadyRunning),
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
    findings.extend(crate::stranded::record(
        ctx,
        store,
        row,
        occasion.observed,
        occasion.judged_from,
        occasion.detail,
        Some(summary),
    )?);
    // Only once the record stands does the episode end: a record that could not be written leaves
    // the budget and the ceiling clock as they were, and the next look escalates again.
    store.end_recovery_episode(t, h)?;
    store.settle_rung_as(log, RungOutcome::Escalated)?;
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
                outcome_words(r.outcome.as_deref()),
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
    let signs = signs_of_life(ctx, store, row, &last_activity)?;
    Ok(format!(
        "ESCALATED BY ITS COORDINATOR — step {step} on thread {t} (coordinated by thread {coord}) \
         could not be recovered.\n\n\
         Cause: {cause}.\n\n\
         What was tried:\n{attempts}\n\n\
         Signs of life:\n{signs}\n\n\
         Last activity on this slot: {last_activity}.\n\
         Session: {session}.\n\
         Working copy: {lease}.\n\n\
         The ways on: a reply into thread {root} commissions the round again; `nxc reply --thread \
         {root} --accept` accepts it as it stands; `nxc withdraw --thread {root}` takes it back.",
        coord = coordinator.thread,
    ))
}

/// **Rung E's "Signs of life" block** (nxf 6j6v.1r61, design 8nx4 section 6): built from the row and
/// the slot's WHOLE log, not only the episode's rows — the last transcript entry, the last pulse,
/// the last movement of an open call's subprocesses, every nudge with its outcome, every rest
/// confirmed, every stop with its outcome; "this host reports no …" for a signal the worker cannot
/// read; and a stop still in flight named as UNCONFIRMED, since rung E ends its supervision.
fn signs_of_life(
    ctx: &Ctx,
    store: &ChatStore,
    row: &OwedStart,
    last_activity: &str,
) -> Result<String> {
    let (t, h) = (row.thread.as_str(), row.role.as_str());
    let mut lines = vec![format!("- last transcript entry: {last_activity}")];
    let running = row
        .session
        .as_deref()
        .is_some_and(|s| ctx.worker.session_is_running(s));
    lines.push(match row.session.as_deref() {
        Some(s) if running => match ctx.worker.session_pulse(s) {
            crate::worker::Pulse::At(at) => format!("- last pulse: the model's output at {at}"),
            crate::worker::Pulse::NoneYet => {
                "- last pulse: none yet — its process has produced no output".to_string()
            }
            _ => "- last pulse: this host reports no pulse".to_string(),
        },
        _ => "- last pulse: not read — no process runs".to_string(),
    });
    let call = match row.session.as_deref() {
        Some(s) => store.slot_open_tool_call(s, t)?,
        None => None,
    };
    let blind = match (row.session.as_deref(), &call) {
        (Some(s), Some(call)) if running => ctx.worker.subprocess_sample(s, &call.since).is_none(),
        _ => false,
    };
    lines.push(
        match (
            &row.subprocess_moved_at,
            row.subprocess_count,
            row.subprocess_cpu_units,
        ) {
            _ if blind => "- subprocesses: this host reports no subprocess signals".to_string(),
            (Some(m), Some(n), Some(cpu)) => format!(
            "- last subprocess movement: at {m} — {n} process(es) under the open call, the CPU \
             counter standing at {cpu}"
        ),
            (None, Some(n), Some(cpu)) => {
                format!("- subprocesses: {n} under the open call, never moving (CPU counter {cpu})")
            }
            _ => "- subprocesses: none sampled".to_string(),
        },
    );
    for r in store.slot_recovery_log(t, h)? {
        let outcome = r.outcome.as_deref().unwrap_or("in flight");
        match action_of(&r) {
            Some(RecoveryAction::Asked) => {
                lines.push(format!("- nudge at {}: {outcome}", r.at));
            }
            Some(RecoveryAction::Confirmed) => {
                lines.push(format!("- confirmation at {}: {}", r.at, r.detail));
            }
            Some(RecoveryAction::Stopped) => {
                let pid = process_of(&r.detail)
                    .map_or_else(|| "its pid unknown".to_string(), |p| format!("pid {p}"));
                let survivors: Vec<String> = descendants_of(&r.detail)
                    .into_iter()
                    .filter(|m| ctx.worker.process_survives(m))
                    .map(|m| format!("pid {}", m.pid))
                    .collect();
                let survivors = if survivors.is_empty() {
                    String::new()
                } else {
                    format!("; still running from it: {}", survivors.join(", "))
                };
                lines.push(format!("- stop at {} ({pid}): {outcome}{survivors}", r.at));
            }
            Some(RecoveryAction::Recovered) => {
                lines.push(format!("- recovered at {}: {}", r.at, r.detail));
            }
            _ => {}
        }
    }
    if let Some(stop) = row.stop_requested_at.as_deref() {
        let pid = store
            .newest_rung_of(t, h, RecoveryAction::Stopped)?
            .and_then(|r| process_of(&r.detail))
            .map_or_else(|| "its pid unknown".to_string(), |p| format!("pid {p}"));
        lines.push(format!(
            "- the SIGTERM of {stop} to {pid} is UNCONFIRMED: this escalation ends its \
             supervision, and nothing checks whether that process went"
        ));
    }
    Ok(lines.join("\n"))
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
    (row.attempts >= 1 || row.recovery_started_at.is_some()) && row.abandoned.is_none()
}

pub(crate) fn action_of(row: &RecoveryLogRow) -> Option<RecoveryAction> {
    RecoveryAction::parse(&row.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "2026-10-25T10:00:00Z";

    fn at(secs: i64) -> String {
        after(BASE, secs, "a test instant").unwrap()
    }

    /// A store on a real file (the renewal opens a connection of its own), with one slot claimed at
    /// [`BASE`].
    fn claimed() -> (tempfile::TempDir, String, ChatStore) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        let mut store = ChatStore::open(&path, 1).unwrap();
        store.open_slot("t-1", "o/coder", "t-1", BASE).unwrap();
        assert_eq!(
            store.claim_slot("t-1", "o/coder", BASE).unwrap(),
            SlotClaim::Claimed
        );
        (tmp, path, store)
    }

    fn until(store: &ChatStore) -> Option<String> {
        store
            .connection()
            .query_row(
                "SELECT recovering_until FROM owed_starts WHERE thread = 't-1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn wait_for(what: &str, ok: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} never happened"
            );
            std::thread::yield_now();
        }
    }

    /// **The claim is renewed while its trigger runs** (must-fix, design review round 3 TQ #4): each
    /// renewal moves the horizon half a claim further from the look's own instant, so a second look
    /// past the first claim finds it still held — and once the renewal stops, a look past the last
    /// horizon finds it free.
    #[test]
    fn a_running_renewal_moves_the_claim_on_by_half_a_claim_per_renewal() {
        renew_claims_every(std::time::Duration::from_millis(5));
        let (_tmp, path, mut store) = claimed();
        assert_eq!(until(&store), Some(at(CLAIM_SECS)));
        let renewal = ClaimRenewal::start_on(&path, "t-1", "o/coder", BASE);
        wait_for("three renewals", || {
            until(&store).is_some_and(|u| !earlier(&u, &at(CLAIM_SECS + 3 * CLAIM_SECS / 2)))
        });
        // Past the first claim, and past the in-flight guard's ten minutes: still held.
        wait_for("the claim to reach past the trigger timeout", || {
            until(&store).is_some_and(|u| earlier(&at(TRIGGER_TIMEOUT_SECS + 1), &u))
        });
        assert!(matches!(
            store
                .claim_slot("t-1", "o/coder", &at(TRIGGER_TIMEOUT_SECS + 1))
                .unwrap(),
            SlotClaim::BeingRecovered { .. }
        ));
        assert_eq!(renewal.failures(), 0);
        drop(renewal);
        let last = until(&store).unwrap();
        assert!(
            (0..CLAIM_SECS * 10_000).step_by(30).any(|n| at(n) == last),
            "the horizon moves in half-claim steps: {last}"
        );
        // Stopped: a look past the last horizon takes the slot.
        let past = after(&last, 1, "past the horizon").unwrap();
        assert_eq!(
            store.claim_slot("t-1", "o/coder", &past).unwrap(),
            SlotClaim::Claimed
        );
    }

    /// A released claim is not taken again by a renewal that is still winding down.
    #[test]
    fn a_renewal_never_takes_a_released_claim_again() {
        renew_claims_every(std::time::Duration::from_millis(5));
        let (_tmp, path, mut store) = claimed();
        store.release_slot_claim("t-1", "o/coder").unwrap();
        let renewal = ClaimRenewal::start_on(&path, "t-1", "o/coder", BASE);
        // Give it renewals to run: wait until its interval has passed a few times over, by its own
        // count of attempts made (a successful no-op UPDATE counts nothing, so poll the clock).
        let begun = std::time::Instant::now();
        wait_for("several intervals", || {
            begun.elapsed() > std::time::Duration::from_millis(50)
        });
        drop(renewal);
        assert_eq!(until(&store), None);
    }

    /// **A renewal that cannot reach the store** says so, counts it, and neither panics nor creates
    /// a database where there was none; the rung's own in-flight log row is what then keeps a
    /// second rung out.
    #[test]
    fn a_renewal_that_cannot_reach_the_store_counts_its_failures_and_creates_nothing() {
        renew_claims_every(std::time::Duration::from_millis(5));
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-such-dir").join("db.sqlite");
        let renewal = ClaimRenewal::start_on(missing.to_str().unwrap(), "t-1", "o/coder", BASE);
        wait_for("a failed renewal", || renewal.failures() > 0);
        drop(renewal);
        assert!(!missing.exists(), "nothing was created");
    }

    fn entry(store: &ChatStore, kind: &str, at: &str, tool_use_id: Option<&str>, data: &str) {
        store
            .connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, tool_use_id, at, data)
                 VALUES ('s-1', (SELECT COALESCE(MAX(seq), -1) + 1 FROM agent_transcript
                                  WHERE internal_session = 's-1'), ?1, ?2, ?3, ?4)",
                params![kind, tool_use_id, at, data],
            )
            .unwrap();
    }

    /// **The session's own life is model output only** (nxf 6j6v.1r61, design 8nx4 section 1, round
    /// 3 TQ #4 / TQ #6): the newest `assistant`, `thinking` or `tool_use` of the slot's span, and a
    /// `tool_result` only when its `tool_use` lies in the same span. The sidecar's `turn_start`,
    /// the runtime's `session_init`, `result` and `error` are not life — and neither is a result
    /// whose call was made before the span began (a resumed session's first entry, often).
    #[test]
    fn slot_last_own_output_reads_model_output_and_nothing_the_runtime_writes() {
        let store = ChatStore::open_in_memory(1);
        // An earlier turn of the slot: a call made there.
        entry(&store, "turn_start", &at(0), None, r#"{"thread":"t-1"}"#);
        entry(
            &store,
            "tool_use",
            &at(10),
            Some("old"),
            r#"{"name":"Bash"}"#,
        );
        // The current span.
        entry(&store, "turn_start", &at(100), None, r#"{"thread":"t-1"}"#);
        entry(&store, "session_init", &at(101), None, "{}");
        assert_eq!(
            store.slot_last_own_output("s-1", "t-1").unwrap(),
            None,
            "the marker and the runtime's init are not life"
        );
        entry(&store, "tool_result", &at(102), Some("old"), "{}");
        assert_eq!(
            store.slot_last_own_output("s-1", "t-1").unwrap(),
            None,
            "a result whose call lies before the span is not life"
        );
        entry(&store, "thinking", &at(110), None, "{}");
        assert_eq!(
            store.slot_last_own_output("s-1", "t-1").unwrap(),
            Some(at(110))
        );
        entry(
            &store,
            "tool_use",
            &at(120),
            Some("new"),
            r#"{"name":"Bash"}"#,
        );
        entry(&store, "tool_result", &at(130), Some("new"), "{}");
        assert_eq!(
            store.slot_last_own_output("s-1", "t-1").unwrap(),
            Some(at(130)),
            "a result whose call lies in the span is"
        );
        entry(&store, "assistant", &at(140), None, "{}");
        entry(&store, "result", &at(150), None, "{}");
        entry(&store, "error", &at(160), None, "{}");
        assert_eq!(
            store.slot_last_own_output("s-1", "t-1").unwrap(),
            Some(at(140)),
            "result and error are the runtime's"
        );
        assert_eq!(
            store.slot_last_activity("s-1", "t-1").unwrap(),
            Some(at(160)),
            "the unfiltered reader is unchanged"
        );
    }

    /// **A stop's detail names its pid however its reason is worded** (review of pass 1, Integrity
    /// #3): the reason of a stop without a nudge begins "its process started at …", and the pid
    /// marker must not be found inside it.
    #[test]
    fn a_stop_detail_names_its_pid_and_descendants_whatever_its_reason_says() {
        let tree = crate::worker::SessionProcessTree {
            pid: 4242,
            descendants: vec![crate::worker::ProcessMark {
                pid: 77,
                started: Some(BASE.to_string()),
            }],
        };
        let detail = describe_the_stop(
            "its process started at 2026-10-25T10:00:00Z and produced nothing of its own since",
            Some(&tree),
        );
        assert_eq!(process_of(&detail), Some(4242), "{detail}");
        assert_eq!(descendants_of(&detail), tree.descendants, "{detail}");
    }

    /// Every typed outcome reads back as itself — the log's words and the hold's.
    #[test]
    fn every_outcome_word_parses_back_to_its_variant() {
        for o in [
            RungOutcome::Asked,
            RungOutcome::Triggered,
            RungOutcome::Queued,
            RungOutcome::SessionGoneRecommissioned,
            RungOutcome::NoSession,
            RungOutcome::NoProcess,
            RungOutcome::CannotAsk,
            RungOutcome::AlreadyRunning,
            RungOutcome::TimedOut,
            RungOutcome::Escalated,
            RungOutcome::Reset,
            RungOutcome::Overtaken,
            RungOutcome::StopRequested,
            RungOutcome::NothingToStop,
            RungOutcome::Granted,
            RungOutcome::Life,
        ] {
            assert_eq!(RungOutcome::parse(o.as_str()), Some(o));
            assert!(!o.words().contains('_'), "{:?} reads as a sentence", o);
        }
        for o in [
            HoldOutcome::Resumed,
            HoldOutcome::ProviderCannotResume,
            HoldOutcome::TreeMovedAsked,
            HoldOutcome::NotYet,
        ] {
            assert_eq!(HoldOutcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(outcome_words(Some("refused: no")), "refused: no");
        assert_eq!(outcome_words(None), "still in flight");
    }
}

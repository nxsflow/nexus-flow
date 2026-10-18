//! **A member session that will not answer is RECORDED as stranded** (nxf 6j6v.ccvy, cut 1a of epic
//! 6j6v.nmc9) — the store half of the reconciliation primitive.
//!
//! A run's liveness used to be derived from five independently written facts (the hold table, the
//! session's end plus the worker's pid claim, the lease, the queue plus the failed starts, the
//! thread register), and nothing asked the one question that matters: *this thread still expects
//! role X — will X's session produce an answer?* A session that never bound, that was killed, or
//! that ended without answering matched none of them; its window kept counting and the flow was
//! then advanced past it as if it had lapsed. This module keeps the three device-local tables the
//! answer is built from; the sweep that asks the question lives beside it in
//! [`crate::orchestration`].
//!
//! ## The three tables
//!
//! - **`owed_starts`** — the OBLIGATION record: thread T owes an answer from role H, and session S
//!   is the one that owes it. Written where a session is minted for an obligation and where the
//!   trigger funnel puts it in motion, from the CALLER's own reply thread only. It is read instead
//!   of `session_map.thread`, which is a moving POSITION (a woken session is repointed) rather than
//!   an obligation. A slot with no row is outside this rule entirely and keeps the behaviour it had
//!   before this cut — which is also the cutover: every slot opened before this version has no row.
//! - **`stranded_suspects`** — the first of two looks at a session with no end: what the evidence
//!   was (bound or not, transcript entries) and when it was first seen. The second look records only
//!   if the evidence is unchanged.
//! - **`stranded_sessions`** — the RECORD: one row per stranding of one obligation, keyed by the
//!   trigger clock it was judged against, with the escalation's outcome stamped on it.
//!
//! ## What a stranded record is NOT
//!
//! It is not a hold. A row in `session_interruption` means *the model went away and nobody did
//! anything wrong*, renders as `interrupted`, and is taken up by `resume`. A stranded session is a
//! `needs_decision`: the round was escalated to whoever commissioned it, and nothing here restarts
//! anything (that is 6j6v.21xy, a later cut).
//!
//! ## Device-local, and a replica records nothing
//!
//! Every table here names a session by its internal id — a process on THIS machine — exactly as
//! `session_map` does, so none of them is a view and none reaches a sync peer. A replica has no
//! `owed_starts` row for a session another device started, so it never records one; the deciding
//! device is the one that ran the trigger.

use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::orchestration::{ConsequenceClass, Ctx, FailedConsequence};
use crate::store::ChatStore;

/// The DDL of this module's three tables — applied with the chat views, device-local like
/// `session_map`.
pub(crate) const STRANDED_DDL: &str = "
    -- The obligation record (nxf 6j6v.ccvy): thread T owes an answer from role H (the qualified
    -- handle, as the thread register names it), and S is the session that owes it. ONE clock
    -- column: the mint instant until the first trigger, the trigger instant from then on, moved by
    -- every re-trigger and by a resume; `triggered` says which.
    CREATE TABLE IF NOT EXISTS owed_starts(
        thread        TEXT    NOT NULL,
        role          TEXT    NOT NULL,
        session       TEXT    NOT NULL,
        trigger_clock TEXT    NOT NULL,
        triggered     INTEGER NOT NULL,
        PRIMARY KEY(thread, role)
    );
    CREATE INDEX IF NOT EXISTS owed_starts_by_session ON owed_starts(session);
    -- The first look at a session with no end (nxf 6j6v.ccvy): what the evidence was. Keyed by the
    -- OBLIGATION rather than by the session, because one session may owe two threads.
    CREATE TABLE IF NOT EXISTS stranded_suspects(
        thread             TEXT    NOT NULL,
        role               TEXT    NOT NULL,
        session            TEXT    NOT NULL,
        first_seen         TEXT    NOT NULL,
        bound              INTEGER NOT NULL,
        transcript_entries INTEGER NOT NULL,
        PRIMARY KEY(thread, role)
    );
    -- The record (nxf 6j6v.ccvy): one row per stranding of one obligation. CURRENT while
    -- `superseded_at` is NULL — a later trigger of the same obligation stamps it — so the record
    -- outlives the owed row the escalation discharges.
    CREATE TABLE IF NOT EXISTS stranded_sessions(
        thread              TEXT    NOT NULL,
        role                TEXT    NOT NULL,
        session             TEXT    NOT NULL,
        trigger_clock       TEXT    NOT NULL,
        judged_from         TEXT    NOT NULL,
        recorded_at         TEXT    NOT NULL,
        observed            INTEGER NOT NULL,
        detail              TEXT    NOT NULL,
        escalation          TEXT,
        escalation_attempts INTEGER NOT NULL DEFAULT 0,
        escalated_at        TEXT,
        escalation_error    TEXT,
        superseded_at       TEXT,
        PRIMARY KEY(thread, role, trigger_clock)
    );
";

/// How long an owed session may stand with no live claim, no hold and no end before the sweep first
/// LOOKS at it — two liveness cadences. The ONE grace constant of the stranded rule.
///
/// A session with no end needs a SECOND look one cadence later with unchanged evidence, so the
/// earliest record of such a session is its clock + 180 s; a session that announced its end, or
/// whose start was given up, is recorded at the first look.
///
/// **What the grace protects is NOT the bind.** `SessionLock::acquire` takes the pid claim inside
/// the trigger, so a started session holds a claim from the trigger on. What it protects is the
/// trigger funnel's OWN work between the owed row and the claim — the lease gate, the prime block
/// (`nxf prime` has a 30 s wall-clock timeout) — and a hand-over that failed. What would falsify it:
/// an owed-row → claim latency past the second look.
pub const STRANDED_GRACE_SECS: i64 = 2 * crate::orchestration::SESSION_LIVENESS_RECHECK_SECS;

/// How long a stranded record is kept once written — the retention bound of `stranded_sessions`.
pub const STRANDED_RETENTION_DAYS: i64 = 30;

/// **The obligation record** — thread `thread` owes an answer from `role`, and `session` owes it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OwedStart {
    pub thread: String,
    /// The qualified handle, as the thread register names it.
    pub role: String,
    pub session: String,
    /// The instant the session is judged from when it has no end: the mint instant until the first
    /// trigger, the trigger instant from then on.
    pub trigger_clock: String,
    /// Whether any trigger ever ran for this obligation. `false` is a slot that was opened and whose
    /// start never happened.
    pub triggered: bool,
}

/// **What the sweep saw** — the five observations a stranded record carries, numbered as the design
/// numbers them (and as the record's `observed` column stores them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Observation {
    /// 1 — a slot was opened for it and no start ever ran.
    NeverStarted,
    /// 2 — no live process across two looks, and it never bound.
    NeverBound,
    /// 3 — its process ended without answering, and no substitute was posted for it.
    EndedWithoutAnswer,
    /// 4 — it bound and wrote, and has held no live process across two looks.
    NoLiveProcess,
    /// 5 — its start failed and was given up.
    StartGivenUp,
}

impl Observation {
    /// The number the design and the stored column use.
    pub fn number(self) -> u8 {
        match self {
            Observation::NeverStarted => 1,
            Observation::NeverBound => 2,
            Observation::EndedWithoutAnswer => 3,
            Observation::NoLiveProcess => 4,
            Observation::StartGivenUp => 5,
        }
    }

    fn from_number(n: i64) -> Option<Observation> {
        Some(match n {
            1 => Observation::NeverStarted,
            2 => Observation::NeverBound,
            3 => Observation::EndedWithoutAnswer,
            4 => Observation::NoLiveProcess,
            5 => Observation::StartGivenUp,
            _ => return None,
        })
    }
}

/// **What became of a record's escalation.** `None` on the record (the column is NULL) is "not
/// posted yet" — the sweep tries again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Escalation {
    /// The substituted escalation in the member's name is on the thread.
    Posted,
    /// Nothing was posted by the sweep, and the slot is no longer owed: somebody else — a late
    /// reply, a human — discharged it.
    AlreadyAnswered,
    /// The post was refused for good, or a transient error outlasted the bounded retry. A human
    /// withdraws the round.
    GivenUp,
}

impl Escalation {
    fn as_str(self) -> &'static str {
        match self {
            Escalation::Posted => "posted",
            Escalation::AlreadyAnswered => "already_answered",
            Escalation::GivenUp => "given_up",
        }
    }

    fn parse(raw: &str) -> Option<Escalation> {
        Some(match raw {
            "posted" => Escalation::Posted,
            "already_answered" => Escalation::AlreadyAnswered,
            "given_up" => Escalation::GivenUp,
            _ => return None,
        })
    }
}

/// **One stranded record**, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StrandedRecord {
    pub thread: String,
    pub role: String,
    pub session: String,
    /// The owed row's clock when it was recorded — the key that makes a later trigger a NEW
    /// stranding rather than this one.
    pub trigger_clock: String,
    /// The instant the sweep judged from: the clock, the session's end, or the last failed attempt.
    pub judged_from: String,
    pub recorded_at: String,
    pub observed: Observation,
    /// The observation in words, with its instants — what the escalation and `status` say.
    pub detail: String,
    pub escalation: Option<Escalation>,
    pub escalation_attempts: u32,
    pub escalated_at: Option<String>,
    /// The last error a post attempt met, when one did.
    pub escalation_error: Option<String>,
    /// Stamped when a later trigger of the same obligation ran; the record is history from then on.
    pub superseded_at: Option<String>,
}

/// The first look's evidence about a session with no end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Suspect {
    pub session: String,
    pub first_seen: String,
    pub bound: bool,
    pub transcript_entries: i64,
}

const RECORD_COLS: &str = "thread, role, session, trigger_clock, judged_from, recorded_at, \
     observed, detail, escalation, escalation_attempts, escalated_at, escalation_error, \
     superseded_at";

fn record_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StrandedRecord> {
    let observed: i64 = r.get(6)?;
    let escalation: Option<String> = r.get(8)?;
    let attempts: i64 = r.get(9)?;
    Ok(StrandedRecord {
        thread: r.get(0)?,
        role: r.get(1)?,
        session: r.get(2)?,
        trigger_clock: r.get(3)?,
        judged_from: r.get(4)?,
        recorded_at: r.get(5)?,
        // A number this build does not know is a record a newer build wrote; reading it as the
        // nearest thing it could mean would put words in its mouth, and dropping the row would hide
        // it. "No live process" is the most general of the five and is never wrong about a stranded
        // session.
        observed: Observation::from_number(observed).unwrap_or(Observation::NoLiveProcess),
        detail: r.get(7)?,
        escalation: escalation.as_deref().and_then(Escalation::parse),
        escalation_attempts: u32::try_from(attempts).unwrap_or(u32::MAX),
        escalated_at: r.get(10)?,
        escalation_error: r.get(11)?,
        superseded_at: r.get(12)?,
    })
}

fn owed_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OwedStart> {
    let triggered: i64 = r.get(4)?;
    Ok(OwedStart {
        thread: r.get(0)?,
        role: r.get(1)?,
        session: r.get(2)?,
        trigger_clock: r.get(3)?,
        triggered: triggered != 0,
    })
}

impl ChatStore {
    /// **Record (or re-record) the obligation `thread` owes `role` on `session`.**
    ///
    /// An upsert by `(thread, role)`: a new session for the same obligation replaces the row whole,
    /// and the same session re-triggered moves only the clock. Every stranded record of the same
    /// obligation judged against an OLDER clock is stamped superseded in the same call — a later
    /// trigger is what ends a record's currency, and it is the one event that does.
    pub(crate) fn record_owed_start(
        &mut self,
        thread: &str,
        role: &str,
        session: &str,
        clock: &str,
        triggered: bool,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO owed_starts(thread, role, session, trigger_clock, triggered)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(thread, role) DO UPDATE SET
                 session = excluded.session,
                 trigger_clock = excluded.trigger_clock,
                 triggered = excluded.triggered",
            params![thread, role, session, clock, i64::from(triggered)],
        )?;
        supersede_older(conn, thread, role, clock)?;
        Ok(())
    }

    /// **Move the clock of one obligation, only while `session` is the one that owes it** — the
    /// resume and the way-back wake. Keyed by `(thread, role, session)` so a resume for one thread
    /// never turns another thread's current record into history. Returns whether a row moved.
    pub(crate) fn touch_owed_start(
        &mut self,
        thread: &str,
        role: &str,
        session: &str,
        now: &str,
    ) -> Result<bool> {
        let conn = self.connection();
        let n = conn.execute(
            "UPDATE owed_starts SET trigger_clock = ?4, triggered = 1
              WHERE thread = ?1 AND role = ?2 AND session = ?3",
            params![thread, role, session, now],
        )?;
        if n > 0 {
            supersede_older(conn, thread, role, now)?;
        }
        Ok(n > 0)
    }

    /// The obligation record for `(thread, role)`, if one was written.
    pub fn owed_start(&self, thread: &str, role: &str) -> Result<Option<OwedStart>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT thread, role, session, trigger_clock, triggered FROM owed_starts
                  WHERE thread = ?1 AND role = ?2",
                params![thread, role],
                owed_from_row,
            )
            .optional()?)
    }

    /// Every obligation record, in `(thread, role)` order — the sweep's work list. Bounded by what is
    /// open: the sweep deletes a row one look after its thread stops owing the role.
    pub fn owed_starts(&self) -> Result<Vec<OwedStart>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT thread, role, session, trigger_clock, triggered FROM owed_starts
              ORDER BY thread, role",
        )?;
        let rows = st
            .query_map([], owed_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The obligation records `session` owes — what a session's end looks at.
    pub(crate) fn owed_starts_of_session(&self, session: &str) -> Result<Vec<OwedStart>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT thread, role, session, trigger_clock, triggered FROM owed_starts
              WHERE session = ?1 ORDER BY thread, role",
        )?;
        let rows = st
            .query_map([session], owed_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Forget the obligation record and any first look at it — the thread no longer owes the role.
    pub(crate) fn forget_owed_start(&mut self, thread: &str, role: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM owed_starts WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        conn.execute(
            "DELETE FROM stranded_suspects WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// The first look at `(thread, role)`, if one was written.
    pub(crate) fn stranded_suspect(&self, thread: &str, role: &str) -> Result<Option<Suspect>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT session, first_seen, bound, transcript_entries FROM stranded_suspects
                  WHERE thread = ?1 AND role = ?2",
                params![thread, role],
                |r| {
                    let bound: i64 = r.get(2)?;
                    Ok(Suspect {
                        session: r.get(0)?,
                        first_seen: r.get(1)?,
                        bound: bound != 0,
                        transcript_entries: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Write — or RESET — the first look at `(thread, role)`: a changed tuple starts the two looks
    /// over from `suspect.first_seen`.
    pub(crate) fn put_stranded_suspect(
        &mut self,
        thread: &str,
        role: &str,
        suspect: &Suspect,
    ) -> Result<()> {
        self.connection().execute(
            "INSERT INTO stranded_suspects(thread, role, session, first_seen, bound,
                 transcript_entries)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(thread, role) DO UPDATE SET
                 session = excluded.session, first_seen = excluded.first_seen,
                 bound = excluded.bound, transcript_entries = excluded.transcript_entries",
            params![
                thread,
                role,
                suspect.session,
                suspect.first_seen,
                i64::from(suspect.bound),
                suspect.transcript_entries
            ],
        )?;
        Ok(())
    }

    /// Drop the first look at `(thread, role)` — the evidence no longer holds.
    pub(crate) fn drop_stranded_suspect(&mut self, thread: &str, role: &str) -> Result<()> {
        self.connection().execute(
            "DELETE FROM stranded_suspects WHERE thread = ?1 AND role = ?2",
            params![thread, role],
        )?;
        Ok(())
    }

    /// **Write the record.** Idempotent on its key: a second record of the same stranding (two
    /// sweeps racing) is a no-op and answers `false`.
    pub(crate) fn record_stranded(&mut self, rec: &StrandedRecord) -> Result<bool> {
        let n = self.connection().execute(
            "INSERT INTO stranded_sessions(thread, role, session, trigger_clock, judged_from,
                 recorded_at, observed, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT DO NOTHING",
            params![
                rec.thread,
                rec.role,
                rec.session,
                rec.trigger_clock,
                rec.judged_from,
                rec.recorded_at,
                i64::from(rec.observed.number()),
                rec.detail
            ],
        )?;
        Ok(n > 0)
    }

    /// The records whose escalation is still to be settled — NULL and not superseded. The stamp
    /// retry iterates THIS, never the sweep's candidates (which need the role still outstanding,
    /// and a posted escalation is exactly what discharges it).
    pub(crate) fn stranded_awaiting_escalation(&self) -> Result<Vec<StrandedRecord>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM stranded_sessions
              WHERE escalation IS NULL AND superseded_at IS NULL
              ORDER BY recorded_at, thread, role"
        ))?;
        let rows = st
            .query_map([], record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Stamp what became of a record's escalation.
    pub(crate) fn stamp_stranded_escalation(
        &mut self,
        rec: &StrandedRecord,
        escalation: Escalation,
        at: Option<&str>,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE stranded_sessions SET escalation = ?4, escalated_at = ?5
              WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3",
            params![
                rec.thread,
                rec.role,
                rec.trigger_clock,
                escalation.as_str(),
                at
            ],
        )?;
        Ok(())
    }

    /// Count one more failed attempt at the escalation, with the error it met. Returns the count.
    pub(crate) fn note_stranded_attempt(
        &mut self,
        rec: &StrandedRecord,
        error: &str,
    ) -> Result<u32> {
        let conn = self.connection();
        conn.execute(
            "UPDATE stranded_sessions
                SET escalation_attempts = escalation_attempts + 1, escalation_error = ?4
              WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3",
            params![rec.thread, rec.role, rec.trigger_clock, error],
        )?;
        let n: i64 = conn.query_row(
            "SELECT escalation_attempts FROM stranded_sessions
              WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3",
            params![rec.thread, rec.role, rec.trigger_clock],
            |r| r.get(0),
        )?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// **The CURRENT record of each of `threads`**, by thread — the newest unsuperseded row. Read in
    /// one statement, for `status`'s bulk read.
    pub(crate) fn current_stranded_by_threads(
        &self,
        threads: &[&str],
    ) -> Result<std::collections::BTreeMap<String, StrandedRecord>> {
        if threads.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        let placeholders = vec!["?"; threads.len()].join(",");
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM stranded_sessions
              WHERE superseded_at IS NULL AND thread IN ({placeholders})
              ORDER BY recorded_at"
        ))?;
        let rows = st
            .query_map(rusqlite::params_from_iter(threads.iter()), record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Ascending, so the newest of a thread's current rows wins the map — one obligation per
        // slot, so there is normally exactly one.
        Ok(rows.into_iter().map(|r| (r.thread.clone(), r)).collect())
    }

    /// **Every CURRENT record**, by thread — [`current_stranded_by_threads`](Self::current_stranded_by_threads)
    /// for a report too wide to bind its ids.
    pub(crate) fn current_stranded_all(
        &self,
    ) -> Result<std::collections::BTreeMap<String, StrandedRecord>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM stranded_sessions
              WHERE superseded_at IS NULL ORDER BY recorded_at"
        ))?;
        let rows = st
            .query_map([], record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|r| (r.thread.clone(), r)).collect())
    }

    /// Every stranded record, oldest first — for the unit tests.
    #[cfg(test)]
    pub fn stranded_records(&self) -> Result<Vec<StrandedRecord>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM stranded_sessions ORDER BY recorded_at, thread, role"
        ))?;
        let rows = st
            .query_map([], record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// **Retention**: drop the records written before `cutoff`. Returns how many went.
    pub(crate) fn forget_stranded_before(&mut self, cutoff: &str) -> Result<usize> {
        Ok(self.connection().execute(
            "DELETE FROM stranded_sessions WHERE recorded_at < ?1",
            params![cutoff],
        )?)
    }

    /// How many transcript entries `session` has written — half of the first look's evidence.
    pub(crate) fn transcript_entry_count(&self, session: &str) -> Result<i64> {
        Ok(self.connection().query_row(
            "SELECT COUNT(*) FROM agent_transcript WHERE internal_session = ?1",
            [session],
            |r| r.get(0),
        )?)
    }

    /// Whether `session` ever bound to a runtime session id — the other half.
    pub(crate) fn session_is_bound(&self, session: &str) -> Result<bool> {
        Ok(self
            .connection()
            .query_row(
                "SELECT real_sdk_id IS NOT NULL FROM session_map WHERE internal_id = ?1",
                [session],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Whether anything is held for `session` to be handed on when its turn ends.
    pub(crate) fn has_pending_wake(&self, session: &str) -> Result<bool> {
        Ok(self.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_wakes WHERE session = ?1)",
            [session],
            |r| r.get(0),
        )?)
    }
}

/// Stamp every current record of `(thread, role)` judged against a clock OLDER than `clock` as
/// superseded — a later trigger of the obligation ended them.
fn supersede_older(
    conn: &rusqlite::Connection,
    thread: &str,
    role: &str,
    clock: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE stranded_sessions SET superseded_at = ?3
          WHERE thread = ?1 AND role = ?2 AND superseded_at IS NULL AND trigger_clock < ?3",
        params![thread, role, clock],
    )?;
    Ok(())
}

// ---- the sweep ---------------------------------------------------------------------------------

/// `at` plus `secs`, in the one canonical form every stored instant uses.
fn after(at: &str, secs: i64, what: &str) -> Result<String> {
    crate::facade::instant_after(at, time::Duration::seconds(secs), what)
}

/// Whether `now` is at or past `at`. Compared as INSTANTS, never as strings: a fractional second and
/// a whole one do not sort as text the way they do as time. An instant that does not parse has not
/// been reached — the fail-safe direction here is "do not record".
fn reached(now: &str, at: &str) -> bool {
    use time::format_description::well_known::Rfc3339;
    match (
        time::OffsetDateTime::parse(now, &Rfc3339),
        time::OffsetDateTime::parse(at, &Rfc3339),
    ) {
        (Ok(now), Ok(at)) => now >= at,
        _ => false,
    }
}

/// **The thread a look at an obligation of `thread` ticks** — a supervised slot's CHANNEL thread,
/// and the thread itself for anything else (a direct commission, a served chat, a border order).
///
/// Never a supervised slot: `tick` reads a slot as a thread with no members, and a stale one would
/// be consolidated by its own tick. The channel thread's tick changes nothing but through the sweep
/// until the round itself is due.
pub(crate) fn look_thread(store: &ChatStore, origin: &str, thread: &str) -> Result<String> {
    let supervisor = crate::orchestration::supervisor_qualified(origin);
    let opened_by_the_supervisor = store
        .thread_quorum(thread, "1970-01-01T00:00:00Z")?
        .is_some_and(|q| q.opener.as_deref() == Some(supervisor.as_str()));
    if opened_by_the_supervisor {
        if let Some(parent) = store.thread_parent(thread)? {
            return Ok(parent);
        }
    }
    Ok(thread.to_string())
}

/// **Arm a look at `session`, owing on `thread`, for `at`** — best-effort, and REPORTED: a look that
/// cannot be armed is a finding on the receipt of whoever tried, and the sweep is workspace-wide, so
/// the next tick anybody causes looks anyway.
pub(crate) fn arm_look(
    ctx: &Ctx,
    store: &ChatStore,
    session: &str,
    thread: &str,
    at: &str,
) -> Option<FailedConsequence> {
    let target = match look_thread(store, ctx.origin, thread) {
        Ok(t) => t,
        Err(e) => return Some(look_unarmed(thread, session, at, &e.to_string())),
    };
    match ctx.timer.schedule_stranded_look(session, &target, at) {
        Ok(_) => None,
        Err(e) => Some(look_unarmed(thread, session, at, &e.to_string())),
    }
}

fn look_unarmed(thread: &str, session: &str, at: &str, error: &str) -> FailedConsequence {
    let detail = format!(
        "the look at session {session} (owing on thread {thread}) could not be armed for {at}: \
         {error} — the next tick of any thread in this workspace looks instead; `nxc tick --thread \
         {thread}` looks now"
    );
    eprintln!("warning: {detail}");
    FailedConsequence::stranded(
        ConsequenceClass::TickUnscheduled,
        Some(thread),
        Some(session),
        detail,
    )
}

/// **Arm the first look for an obligation just written** — `clock + STRANDED_GRACE_SECS`.
pub(crate) fn arm_first_look(
    ctx: &Ctx,
    store: &ChatStore,
    session: &str,
    thread: &str,
    clock: &str,
) -> Option<FailedConsequence> {
    match after(clock, STRANDED_GRACE_SECS, "the first stranded look") {
        Ok(at) => arm_look(ctx, store, session, thread, &at),
        Err(e) => Some(look_unarmed(thread, session, clock, &e.to_string())),
    }
}

/// **Write the obligation `thread` owes `role` on `session` and arm its first look** — the write
/// every mint and trigger site makes. Best-effort: a store error is a finding, never a failed
/// commission — the row only adds a judgement on top of what the commission already did, and a
/// slot without one keeps today's behaviour.
pub(crate) fn owe(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread: &str,
    role: &str,
    session: &str,
    triggered: bool,
) -> Vec<FailedConsequence> {
    // An engine identity (the synthesizer) answers for the engine; rule 1 excludes it anyway, and
    // keeping it out of the table keeps the work list to what can strand.
    if crate::orchestration::is_engine_identity(role) {
        return Vec::new();
    }
    if let Err(e) = store.record_owed_start(thread, role, session, ctx.now, triggered) {
        let detail = format!(
            "the obligation of {role} on thread {thread} (session {session}) could not be \
             recorded: {e} — a session that never answers there is not noticed by the stranded \
             sweep, and its window lapses as before"
        );
        eprintln!("warning: {detail}");
        return vec![FailedConsequence::stranded(
            ConsequenceClass::StepNotRunning,
            Some(thread),
            Some(session),
            detail,
        )];
    }
    arm_first_look(ctx, store, session, thread, ctx.now)
        .into_iter()
        .collect()
}

/// **Move the clock of the obligation `session` owes on `thread`** — a resume, or a wake on the
/// way back. Only when `session` is the one that owes it (keyed by `(thread, role, session)`, so a
/// woken session never re-owns another session's slot), and the look moves with the clock.
pub(crate) fn touch(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread: &str,
    role: &str,
    session: &str,
) -> Vec<FailedConsequence> {
    match store.touch_owed_start(thread, role, session, ctx.now) {
        Ok(true) => arm_first_look(ctx, store, session, thread, ctx.now)
            .into_iter()
            .collect(),
        Ok(false) => Vec::new(),
        Err(e) => {
            eprintln!(
                "warning: the clock of session {session}'s obligation on thread {thread} could not \
                 be moved: {e}; the stranded sweep may judge it from its previous trigger"
            );
            Vec::new()
        }
    }
}

/// **A session announced its end** — arm a look at `ended + STRANDED_GRACE_SECS` for every
/// obligation it still owes. One that answered owes nothing and arms nothing.
pub(crate) fn arm_after_an_end(
    ctx: &Ctx,
    store: &ChatStore,
    session: &str,
) -> Vec<FailedConsequence> {
    let Ok(owed) = store.owed_starts_of_session(session) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for row in owed {
        let still_owed = store
            .thread_quorum(&row.thread, ctx.now)
            .ok()
            .flatten()
            .is_some_and(|q| outstanding_has(&q.outstanding, &row.role));
        if still_owed {
            findings.extend(arm_first_look(ctx, store, session, &row.thread, ctx.now));
        }
    }
    findings
}

fn outstanding_has(outstanding: &[String], role: &str) -> bool {
    let bare = role.rsplit_once('/').map_or(role, |(_, b)| b);
    outstanding.iter().any(|o| {
        crate::facade::same_identity(Some(o), role) || crate::facade::same_identity(Some(o), bare)
    })
}

/// What the sweep decided about one candidate.
enum Judgement {
    /// Not stranded, or not yet — nothing to record.
    Nothing,
    /// Stranded: record it with this observation, judged from this instant.
    Record {
        observed: Observation,
        judged_from: String,
        detail: String,
    },
}

/// **The sweep** — the sixth workspace-wide step of every tick (nxf 6j6v.ccvy). Records every owed
/// session that will not answer, escalates each record in the member's name, and keeps both tables
/// bounded. It STARTS NOTHING: no hold, no resume, no trigger, no park — the one wake it can cause is
/// the commissioner's, by the escalation's own consolidation.
///
/// Best-effort in every part; what it could not do is a finding on the tick's receipt.
pub(crate) fn reconcile_stranded_sessions(
    ctx: &Ctx,
    store: &mut ChatStore,
) -> Vec<FailedConsequence> {
    let mut findings = Vec::new();
    // FIRST the records whose escalation is still to be settled — the stamp retry iterates THEM,
    // never the candidates below (a posted escalation is exactly what discharges the candidate),
    // and BEFORE retention, which would otherwise take the owed row they are judged against.
    match store.stranded_awaiting_escalation() {
        Ok(rows) => {
            for rec in rows {
                findings.extend(settle_escalation(ctx, store, &rec));
            }
        }
        Err(e) => findings.push(FailedConsequence::stranded(
            ConsequenceClass::StrandedNotEscalated,
            None,
            None,
            format!("the stranded records could not be read to settle their escalations: {e}"),
        )),
    }
    match judge_candidates(ctx, store, &mut findings) {
        Ok(()) => {}
        Err(e) => findings.push(FailedConsequence::stranded(
            ConsequenceClass::StepNotRunning,
            None,
            None,
            format!(
                "the stranded sweep could not read the owed sessions: {e}; the next tick looks \
                 again"
            ),
        )),
    }
    // Retention of the records: a month. The owed rows are kept bounded inside the candidate walk.
    if let Ok(cutoff) = after(
        ctx.now,
        -STRANDED_RETENTION_DAYS * 24 * 60 * 60,
        "the stranded-record retention cutoff",
    ) {
        if let Err(e) = store.forget_stranded_before(&cutoff) {
            eprintln!("warning: old stranded records could not be dropped: {e}");
        }
    }
    findings
}

fn judge_candidates(
    ctx: &Ctx,
    store: &mut ChatStore,
    findings: &mut Vec<FailedConsequence>,
) -> Result<()> {
    let owed = store.owed_starts()?;
    if owed.is_empty() {
        return Ok(());
    }
    let threads: Vec<&str> = owed.iter().map(|o| o.thread.as_str()).collect();
    let quorums: std::collections::BTreeMap<String, crate::store::ThreadQuorum> = store
        .thread_quorums(&threads, ctx.now)?
        .into_iter()
        .map(|q| (q.thread_id.clone(), q))
        .collect();
    // Rule 1, and RETENTION of the owed rows: a row whose thread no longer owes the role is done —
    // answered, escalated, withdrawn — and goes, with any first look at it.
    let mut live = Vec::new();
    for row in owed {
        let owes = quorums
            .get(&row.thread)
            .is_some_and(|q| outstanding_has(&q.outstanding, &row.role));
        if owes {
            live.push(row);
        } else {
            store.forget_owed_start(&row.thread, &row.role)?;
        }
    }
    if live.is_empty() {
        return Ok(());
    }
    // Rule 5's first half, asked once: a worker that cannot look never records anything.
    if !ctx.worker.answers_liveness() {
        findings.push(FailedConsequence::stranded(
            ConsequenceClass::LivenessUnknown,
            None,
            None,
            format!(
                "{} owed session(s) could not be judged: this worker cannot tell a live session \
                 from a dead one, so a member that never answers is settled by its window as \
                 before",
                live.len()
            ),
        ));
        return Ok(());
    }
    let sessions: Vec<&str> = live.iter().map(|o| o.session.as_str()).collect();
    let holds = store.interruptions_among(&sessions)?;
    let failed: std::collections::BTreeMap<String, crate::working_tree::FailedStart> = store
        .failed_starts()?
        .into_iter()
        .map(|f| (f.session.clone(), f))
        .collect();
    let queued: std::collections::HashSet<String> = store
        .list_working_tree_queue()?
        .into_iter()
        .filter_map(|q| q.thread)
        .collect();
    let held_answers = store.threads_holding_wakes()?;
    for row in live {
        let Some(q) = quorums.get(&row.thread) else {
            continue;
        };
        match judge(
            ctx,
            store,
            &row,
            q,
            &holds,
            &failed,
            &queued,
            &held_answers,
            findings,
        )? {
            Judgement::Nothing => {}
            Judgement::Record {
                observed,
                judged_from,
                detail,
            } => findings.extend(record(ctx, store, &row, observed, judged_from, detail)?),
        }
    }
    Ok(())
}

/// Rules 1-6 for one obligation. Drops the first look whenever the evidence no longer holds.
#[allow(clippy::too_many_arguments)]
fn judge(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    q: &crate::store::ThreadQuorum,
    holds: &std::collections::BTreeMap<String, crate::interruption::Interruption>,
    failed: &std::collections::BTreeMap<String, crate::working_tree::FailedStart>,
    queued: &std::collections::HashSet<String>,
    held_answers: &std::collections::HashSet<String>,
    findings: &mut Vec<FailedConsequence>,
) -> Result<Judgement> {
    let (t, h, s) = (row.thread.as_str(), row.role.as_str(), row.session.as_str());
    let not_now = |store: &mut ChatStore| -> Result<Judgement> {
        store.drop_stranded_suspect(t, h)?;
        Ok(Judgement::Nothing)
    };
    // Rule 1: open, not held, the role a REAL one, and not a slot of a round the supervisor is
    // finished with.
    if q.held || crate::orchestration::is_engine_identity(h) {
        return not_now(store);
    }
    if store
        .round_is_finished_with(t, ctx.origin, ctx.now)?
        .is_some()
    {
        return not_now(store);
    }
    // Rule 2: any standing hold excludes the session — it is paused, not stranded.
    if holds.contains_key(s) {
        return not_now(store);
    }
    // Rule 3: a start still to come — a retry due, or the working-copy queue.
    let failed_start = failed.get(s);
    if failed_start.is_some_and(|f| f.retry_at.is_some()) || queued.contains(t) {
        return not_now(store);
    }
    // Rule 4: its own sub-round in flight, or a delivery held for it.
    let children = store.thread_children(t)?;
    let child_ids: Vec<&str> = children.iter().map(String::as_str).collect();
    let kids = store.thread_quorums(&child_ids, ctx.now)?;
    if !crate::awaiting::own_sub_round_in_flight(q, kids.iter(), held_answers).is_empty()
        || store.has_pending_wake(s)?
    {
        return not_now(store);
    }
    // Rule 5: no claim a session still holds. While it holds one, the look comes back one cadence
    // out — a session killed after a healthy look needs a further look.
    if ctx.worker.session_is_running(s) {
        store.drop_stranded_suspect(t, h)?;
        if let Ok(at) = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the next stranded look",
        ) {
            findings.extend(arm_look(ctx, store, s, t, &at));
        }
        return Ok(Judgement::Nothing);
    }
    // Rule 6: the grace on the judged instant.
    let grace = STRANDED_GRACE_SECS;
    // A start that failed and was given up: one look at its last attempt + grace.
    if let Some(f) = failed_start {
        let due = after(&f.last_failed_at, grace, "the stranded grace")?;
        if !reached(ctx.now, &due) {
            findings.extend(arm_look(ctx, store, s, t, &due));
            return Ok(Judgement::Nothing);
        }
        return Ok(Judgement::Record {
            observed: Observation::StartGivenUp,
            judged_from: f.last_failed_at.clone(),
            detail: format!(
                "its start failed {} times and was given up at {}",
                f.attempts, f.last_failed_at
            ),
        });
    }
    // An announced end: one look at the end + grace.
    if let Some(ended) = store.session_ended_at(s)? {
        let due = after(&ended, grace, "the stranded grace")?;
        if !reached(ctx.now, &due) {
            findings.extend(arm_look(ctx, store, s, t, &due));
            return Ok(Judgement::Nothing);
        }
        store.drop_stranded_suspect(t, h)?;
        return Ok(Judgement::Record {
            observed: Observation::EndedWithoutAnswer,
            judged_from: ended.clone(),
            detail: format!(
                "its process ended at {ended} without answering, and no substitute was posted for \
                 it"
            ),
        });
    }
    // No end: two looks on unchanged evidence, from the clock.
    let first_due = after(&row.trigger_clock, grace, "the stranded grace")?;
    if !reached(ctx.now, &first_due) {
        return Ok(Judgement::Nothing);
    }
    let bound = store.session_is_bound(s)?;
    let entries = store.transcript_entry_count(s)?;
    let second_look = |store: &ChatStore, findings: &mut Vec<FailedConsequence>| {
        if let Ok(at) = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the second stranded look",
        ) {
            findings.extend(arm_look(ctx, store, s, t, &at));
        }
    };
    let fresh = Suspect {
        session: s.to_string(),
        first_seen: ctx.now.to_string(),
        bound,
        transcript_entries: entries,
    };
    let Some(suspect) = store.stranded_suspect(t, h)? else {
        store.put_stranded_suspect(t, h, &fresh)?;
        second_look(store, findings);
        return Ok(Judgement::Nothing);
    };
    // The tuple is load-bearing: the session, its bind and its transcript must be what the first
    // look saw. Anything moved — RESET, and look again one cadence out.
    if suspect.session != s || suspect.bound != bound || suspect.transcript_entries != entries {
        store.put_stranded_suspect(t, h, &fresh)?;
        second_look(store, findings);
        return Ok(Judgement::Nothing);
    }
    let second_due = after(
        &suspect.first_seen,
        crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
        "the second stranded look",
    )?;
    if !reached(ctx.now, &second_due) {
        return Ok(Judgement::Nothing);
    }
    store.drop_stranded_suspect(t, h)?;
    let (observed, detail) = if !row.triggered {
        (
            Observation::NeverStarted,
            format!(
                "a slot was opened for it at {} and no start ever ran",
                row.trigger_clock
            ),
        )
    } else if bound || entries > 0 {
        (
            Observation::NoLiveProcess,
            format!(
                "it bound, wrote {entries} transcript entries, and has held no live process \
                 across two looks since {}",
                suspect.first_seen
            ),
        )
    } else {
        (
            Observation::NeverBound,
            format!(
                "no live process across two looks since {}; it never bound",
                suspect.first_seen
            ),
        )
    };
    Ok(Judgement::Record {
        observed,
        judged_from: row.trigger_clock.clone(),
        detail,
    })
}

/// **Recording**, in its order: the row, the escalation in the member's name, the stamp from the
/// evidence, and the service-log finding.
fn record(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    observed: Observation,
    judged_from: String,
    detail: String,
) -> Result<Vec<FailedConsequence>> {
    let rec = StrandedRecord {
        thread: row.thread.clone(),
        role: row.role.clone(),
        session: row.session.clone(),
        trigger_clock: row.trigger_clock.clone(),
        judged_from,
        recorded_at: ctx.now.to_string(),
        observed,
        detail,
        escalation: None,
        escalation_attempts: 0,
        escalated_at: None,
        escalation_error: None,
        superseded_at: None,
    };
    let mut findings = Vec::new();
    if !store.record_stranded(&rec)? {
        // Already recorded for this clock — another sweep got here first; its escalation is that
        // sweep's to settle.
        return Ok(findings);
    }
    let step = step_name(store, &rec);
    let finding = FailedConsequence::stranded(
        ConsequenceClass::SessionStranded,
        Some(&rec.thread),
        Some(&rec.session),
        format!(
            "step {step} on thread {} is stranded: session {} will not answer — observation {}: \
             {}; recorded at {}, escalated to the commissioner",
            rec.thread,
            rec.session,
            rec.observed.number(),
            rec.detail,
            rec.recorded_at
        ),
    );
    eprintln!("warning: {finding}");
    findings.push(finding);
    findings.extend(settle_escalation(ctx, store, &rec));
    Ok(findings)
}

/// The step a slot serves, as a reader names it: the flow mark when the thread carries one, else the
/// role's bare handle.
fn step_name(store: &ChatStore, rec: &StrandedRecord) -> String {
    match store.flow_mark(&rec.thread).ok().flatten() {
        Some(mark) => mark.step,
        None => rec
            .role
            .rsplit_once('/')
            .map_or(rec.role.as_str(), |(_, b)| b)
            .to_string(),
    }
}

/// The substituted escalation sent as the member on `rec.thread` since the record was written — the
/// EVIDENCE a stamp is decided from. `Some(instant)` of the newest such message.
fn posted_evidence(store: &ChatStore, rec: &StrandedRecord) -> Option<String> {
    let rows = store.acting_messages_in_thread(&rec.thread).ok()?;
    rows.into_iter()
        .rev()
        .filter(|m| m.kind == "escalation")
        .filter(|m| {
            crate::facade::same_identity(Some(&m.sender), &rec.role) || m.sender == rec.role
        })
        .filter(|m| {
            m.refs
                .as_deref()
                .and_then(|r| serde_json::from_str::<crate::model::Refs>(r).ok())
                .is_some_and(|r| r.substituted)
        })
        .find(|m| {
            m.created
                .as_deref()
                .is_some_and(|c| reached(c, &rec.recorded_at))
        })
        .map(|m| m.created.unwrap_or_default())
}

/// Whether `rec.thread` still owes `rec.role` an answer.
fn still_owed(ctx: &Ctx, store: &ChatStore, rec: &StrandedRecord) -> Result<bool> {
    Ok(store
        .thread_quorum(&rec.thread, ctx.now)?
        .is_some_and(|q| outstanding_has(&q.outstanding, &rec.role)))
}

/// **Post the escalation if it is still owed, and stamp what happened from the evidence.** Never from
/// the receipt alone: a post can land and its receipt be lost, and a post can be refused while
/// somebody else answers. Bounded: a permanent refusal gives up at once, a transient error after
/// [`crate::orchestration::MAX_START_ATTEMPTS`] sweeps.
fn settle_escalation(
    ctx: &Ctx,
    store: &mut ChatStore,
    rec: &StrandedRecord,
) -> Vec<FailedConsequence> {
    let mut findings = Vec::new();
    // The evidence FIRST: a post an earlier sweep made, whose stamp did not land, is not posted
    // again.
    if let Some(at) = posted_evidence(store, rec) {
        stamp(store, rec, Escalation::Posted, Some(&at), &mut findings);
        return findings;
    }
    match still_owed(ctx, store, rec) {
        Ok(false) => {
            skipped(store, rec, &mut findings);
            return findings;
        }
        Ok(true) => {}
        Err(e) => {
            not_escalated(ctx, store, rec, &e.to_string(), false, &mut findings);
            return findings;
        }
    }
    let body = escalation_body(store, rec);
    let bare = rec
        .role
        .rsplit_once('/')
        .map_or(rec.role.as_str(), |(_, b)| b)
        .to_string();
    let as_member = Ctx {
        actor: &bare,
        session: Some(&rec.session),
        ..*ctx
    };
    match crate::surface::reply_as_the_runtime_for(&as_member, store, &rec.thread, &body, true) {
        Ok(receipt) if receipt.posted && receipt.message_id.is_some() => {
            stamp(store, rec, Escalation::Posted, Some(ctx.now), &mut findings);
        }
        outcome => {
            // Not posted, or an error: the evidence decides — and only after an error is it read
            // again, since a post that DID land is the one case a receipt can get wrong.
            if let Some(at) = posted_evidence(store, rec) {
                stamp(store, rec, Escalation::Posted, Some(&at), &mut findings);
                return findings;
            }
            match (still_owed(ctx, store, rec), outcome) {
                (Ok(false), _) => skipped(store, rec, &mut findings),
                (_, Ok(_)) => not_escalated(
                    ctx,
                    store,
                    rec,
                    "the post found nothing owed but the slot still reads owed",
                    false,
                    &mut findings,
                ),
                (_, Err(e)) => {
                    let permanent = matches!(
                        e.kind,
                        crate::error::ErrorKind::Forbidden
                            | crate::error::ErrorKind::Validation
                            | crate::error::ErrorKind::NotFound
                    );
                    not_escalated(ctx, store, rec, &e.to_string(), permanent, &mut findings);
                }
            }
        }
    }
    findings
}

fn stamp(
    store: &mut ChatStore,
    rec: &StrandedRecord,
    escalation: Escalation,
    at: Option<&str>,
    findings: &mut Vec<FailedConsequence>,
) {
    if let Err(e) = store.stamp_stranded_escalation(rec, escalation, at) {
        let finding = FailedConsequence::stranded(
            ConsequenceClass::StrandedNotEscalated,
            Some(&rec.thread),
            Some(&rec.session),
            format!(
                "the escalation of stranded session {} on thread {} could not be stamped: {e}; \
                 the next sweep reads the thread and stamps it then",
                rec.session, rec.thread
            ),
        );
        eprintln!("warning: {finding}");
        findings.push(finding);
    }
}

fn skipped(store: &mut ChatStore, rec: &StrandedRecord, findings: &mut Vec<FailedConsequence>) {
    stamp(store, rec, Escalation::AlreadyAnswered, None, findings);
    findings.push(FailedConsequence::stranded(
        ConsequenceClass::StrandedEscalationSkipped,
        Some(&rec.thread),
        Some(&rec.session),
        format!(
            "stranded session {} on thread {} was not escalated: the slot was answered meanwhile",
            rec.session, rec.thread
        ),
    ));
}

/// The post did not land. Count it; give up at once on a permanent refusal and after the bound on a
/// transient one, and otherwise arm a further look so a quiet workspace tries again (build item 5).
fn not_escalated(
    ctx: &Ctx,
    store: &mut ChatStore,
    rec: &StrandedRecord,
    error: &str,
    permanent: bool,
    findings: &mut Vec<FailedConsequence>,
) {
    let attempts = store.note_stranded_attempt(rec, error).unwrap_or(u32::MAX);
    let give_up = permanent || attempts >= crate::orchestration::MAX_START_ATTEMPTS;
    if give_up {
        stamp(store, rec, Escalation::GivenUp, None, findings);
    }
    let detail = if give_up {
        format!(
            "the escalation of stranded session {} on thread {} could not be posted and is given \
             up after {attempts} attempt(s): {error} — withdraw the round with `nxc withdraw \
             --thread {}`",
            rec.session, rec.thread, rec.thread
        )
    } else {
        format!(
            "the escalation of stranded session {} on thread {} could not be posted yet \
             (attempt {attempts}): {error}; a further look is armed",
            rec.session, rec.thread
        )
    };
    eprintln!("warning: {detail}");
    findings.push(FailedConsequence::stranded(
        ConsequenceClass::StrandedNotEscalated,
        Some(&rec.thread),
        Some(&rec.session),
        detail,
    ));
    if !give_up {
        if let Ok(at) = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the next escalation attempt",
        ) {
            findings.extend(arm_look(ctx, store, &rec.session, &rec.thread, &at));
        }
    }
}

/// The escalation the commissioner reads.
fn escalation_body(store: &ChatStore, rec: &StrandedRecord) -> String {
    let step = step_name(store, rec);
    let root = store
        .thread_root(&rec.thread)
        .unwrap_or_else(|_| rec.thread.clone());
    format!(
        "STRANDED — step {step} on thread {thread} will not be answered: session {session} is not \
         running and nothing is coming from it.\n\n\
         Observation {n}: {detail}. Judged from {judged} with a grace of {grace} s; recorded at \
         {at}.\n\n\
         Nothing was restarted. A reply into thread {root} commissions the round again; `nxc \
         withdraw --thread {root}` takes it back.",
        thread = rec.thread,
        session = rec.session,
        n = rec.observed.number(),
        detail = rec.detail,
        judged = rec.judged_from,
        grace = STRANDED_GRACE_SECS,
        at = rec.recorded_at,
    )
}

impl ChatStore {
    /// **Is `thread` a slot of a round the supervisor is finished with?** — the parent thread when it
    /// is, `None` otherwise.
    ///
    /// The discriminator `surface::refuse_a_reply_the_supervisor_is_finished_with` refuses a reply
    /// on, lifted here so the stranded sweep asks exactly the same question (rule 1): a slot the
    /// supervisor opened for ONE real party, whose parent no longer expects the supervisor — that
    /// set was consolidated and handed on, and nothing would be woken by an answer.
    pub(crate) fn round_is_finished_with(
        &self,
        thread: &str,
        origin: &str,
        now: &str,
    ) -> Result<Option<String>> {
        let Some(q) = self.thread_quorum(thread, now)? else {
            return Ok(None);
        };
        let supervisor = crate::orchestration::supervisor_qualified(origin);
        if q.opener.as_deref() != Some(supervisor.as_str())
            || q.expects.is_empty()
            || q.expects
                .iter()
                .any(|h| crate::orchestration::is_engine_identity(h))
        {
            return Ok(None);
        }
        let Some(parent) = self.thread_parent(thread)? else {
            return Ok(None);
        };
        let Some(parent_q) = self.thread_quorum(&parent, now)? else {
            return Ok(None);
        };
        if parent_q.expects == [supervisor] {
            return Ok(None);
        }
        Ok(Some(parent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-10-18T10:00:00Z";
    const T1: &str = "2026-10-18T10:03:00Z";
    const T2: &str = "2026-10-18T10:10:00Z";

    fn record(clock: &str) -> StrandedRecord {
        StrandedRecord {
            thread: "m-t".into(),
            role: "ws/verifier".into(),
            session: "m-s".into(),
            trigger_clock: clock.into(),
            judged_from: clock.into(),
            recorded_at: T1.into(),
            observed: Observation::NeverBound,
            detail: "no live process".into(),
            escalation: None,
            escalation_attempts: 0,
            escalated_at: None,
            escalation_error: None,
            superseded_at: None,
        }
    }

    #[test]
    fn a_new_session_for_the_same_obligation_replaces_the_row_whole() {
        let mut s = ChatStore::open_in_memory(1);
        s.record_owed_start("m-t", "ws/verifier", "m-s1", T0, false)
            .unwrap();
        s.record_owed_start("m-t", "ws/verifier", "m-s2", T1, true)
            .unwrap();
        let row = s.owed_start("m-t", "ws/verifier").unwrap().unwrap();
        assert_eq!(
            row,
            OwedStart {
                thread: "m-t".into(),
                role: "ws/verifier".into(),
                session: "m-s2".into(),
                trigger_clock: T1.into(),
                triggered: true,
            }
        );
        assert_eq!(s.owed_starts().unwrap().len(), 1);
    }

    #[test]
    fn a_touch_moves_only_the_obligation_the_session_owes() {
        let mut s = ChatStore::open_in_memory(1);
        s.record_owed_start("m-r1", "ws/pm", "m-p1", T0, true)
            .unwrap();
        s.record_owed_start("m-r2", "ws/pm", "m-p2", T0, true)
            .unwrap();
        assert!(!s.touch_owed_start("m-r1", "ws/pm", "m-p2", T2).unwrap());
        assert!(s.touch_owed_start("m-r2", "ws/pm", "m-p2", T2).unwrap());
        assert_eq!(
            s.owed_start("m-r1", "ws/pm")
                .unwrap()
                .unwrap()
                .trigger_clock,
            T0
        );
        assert_eq!(
            s.owed_start("m-r2", "ws/pm")
                .unwrap()
                .unwrap()
                .trigger_clock,
            T2
        );
    }

    /// Must-fix 1 of the build brief: the record survives the owed row, and only a LATER trigger of
    /// the same obligation makes it history.
    #[test]
    fn a_record_stays_current_after_its_owed_row_goes_and_a_later_trigger_supersedes_it() {
        let mut s = ChatStore::open_in_memory(1);
        s.record_owed_start("m-t", "ws/verifier", "m-s", T0, true)
            .unwrap();
        assert!(s.record_stranded(&record(T0)).unwrap());
        assert!(
            !s.record_stranded(&record(T0)).unwrap(),
            "idempotent on its key"
        );
        s.forget_owed_start("m-t", "ws/verifier").unwrap();
        let current = s.current_stranded_by_threads(&["m-t"]).unwrap();
        assert_eq!(
            current.get("m-t").map(|r| r.trigger_clock.as_str()),
            Some(T0)
        );

        s.record_owed_start("m-t", "ws/verifier", "m-s", T2, true)
            .unwrap();
        assert!(s.current_stranded_by_threads(&["m-t"]).unwrap().is_empty());
        assert_eq!(
            s.stranded_records().unwrap()[0].superseded_at.as_deref(),
            Some(T2)
        );
    }

    #[test]
    fn the_escalation_stamp_and_the_attempts_are_read_back() {
        let mut s = ChatStore::open_in_memory(1);
        let rec = record(T0);
        s.record_stranded(&rec).unwrap();
        assert_eq!(s.stranded_awaiting_escalation().unwrap().len(), 1);
        assert_eq!(s.note_stranded_attempt(&rec, "locked").unwrap(), 1);
        assert_eq!(s.note_stranded_attempt(&rec, "locked again").unwrap(), 2);
        s.stamp_stranded_escalation(&rec, Escalation::Posted, Some(T1))
            .unwrap();
        assert!(s.stranded_awaiting_escalation().unwrap().is_empty());
        let back = &s.stranded_records().unwrap()[0];
        assert_eq!(back.escalation, Some(Escalation::Posted));
        assert_eq!(back.escalated_at.as_deref(), Some(T1));
        assert_eq!(back.escalation_attempts, 2);
        assert_eq!(back.escalation_error.as_deref(), Some("locked again"));
    }

    #[test]
    fn retention_drops_only_what_is_older_than_the_cutoff() {
        let mut s = ChatStore::open_in_memory(1);
        s.record_stranded(&record(T0)).unwrap();
        assert_eq!(s.forget_stranded_before(T0).unwrap(), 0);
        assert_eq!(s.forget_stranded_before(T2).unwrap(), 1);
        assert!(s.stranded_records().unwrap().is_empty());
    }

    #[test]
    fn a_suspect_is_reset_in_place_and_goes_with_its_obligation() {
        let mut s = ChatStore::open_in_memory(1);
        let first = Suspect {
            session: "m-s".into(),
            first_seen: T0.into(),
            bound: false,
            transcript_entries: 0,
        };
        s.put_stranded_suspect("m-t", "ws/verifier", &first)
            .unwrap();
        let reset = Suspect {
            first_seen: T1.into(),
            transcript_entries: 3,
            ..first
        };
        s.put_stranded_suspect("m-t", "ws/verifier", &reset)
            .unwrap();
        assert_eq!(
            s.stranded_suspect("m-t", "ws/verifier").unwrap(),
            Some(reset)
        );
        s.record_owed_start("m-t", "ws/verifier", "m-s", T0, true)
            .unwrap();
        s.forget_owed_start("m-t", "ws/verifier").unwrap();
        assert_eq!(s.stranded_suspect("m-t", "ws/verifier").unwrap(), None);
    }
}

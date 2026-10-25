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
    -- The obligation record (nxf 6j6v.ccvy; the SLOT since 6j6v.kgfn): thread T owes an answer
    -- from role H (the qualified handle, as the thread register names it). Written when the slot is
    -- OPENED, before any session exists — `session` is NULL until the mint fills it. ONE clock
    -- column: the open instant, then the mint's, then every trigger's, moved by every re-trigger
    -- and by a resume; `triggered` says whether any trigger ran. `coordinator` is the thread whose
    -- tick coordinates the slot (`coordinator_of`). `recovering_until`, `attempts`,
    -- `recovery_started_at`, `asked_at` and `last_progress_at` are the ladder's (6j6v.558h) and
    -- stay at their defaults until it is built. `backfilled` marks a row the one-time backfill
    -- wrote for a slot opened before this cut; `abandoned` is the reason such a slot is recorded as
    -- given up (never nudged), with `abandoned_at`.
    CREATE TABLE IF NOT EXISTS owed_starts(
        thread              TEXT    NOT NULL,
        role                TEXT    NOT NULL,
        session             TEXT,
        trigger_clock       TEXT    NOT NULL,
        triggered           INTEGER NOT NULL,
        coordinator         TEXT,
        recovering_until    TEXT,
        attempts            INTEGER NOT NULL DEFAULT 0,
        recovery_started_at TEXT,
        backfilled          INTEGER NOT NULL DEFAULT 0,
        asked_at            TEXT,
        last_progress_at    TEXT,
        abandoned           TEXT,
        abandoned_at        TEXT,
        next_look           TEXT,
        PRIMARY KEY(thread, role)
    );
    CREATE INDEX IF NOT EXISTS owed_starts_by_session ON owed_starts(session);
    -- The one-time backfill's marker (nxf 6j6v.kgfn): one row, written in the SAME transaction as
    -- the rows the backfill wrote, so a crash between them leaves neither.
    CREATE TABLE IF NOT EXISTS owed_backfill_done(
        done_at TEXT NOT NULL
    );
    -- The first look at a session with no end (nxf 6j6v.ccvy): what the evidence was. Keyed by the
    -- OBLIGATION rather than by the session, because one session may owe two threads. `session` is
    -- NULL for a slot that never got one (6j6v.kgfn).
    CREATE TABLE IF NOT EXISTS stranded_suspects(
        thread             TEXT    NOT NULL,
        role               TEXT    NOT NULL,
        session            TEXT,
        first_seen         TEXT    NOT NULL,
        bound              INTEGER NOT NULL,
        transcript_entries INTEGER NOT NULL,
        PRIMARY KEY(thread, role)
    );
    -- The record (nxf 6j6v.ccvy): one row per stranding of one obligation. CURRENT while
    -- `superseded_at` is NULL — a later trigger of the same obligation stamps it — so the record
    -- outlives the owed row the escalation discharges. `session` is NULL for a slot that never got
    -- one (6j6v.kgfn).
    CREATE TABLE IF NOT EXISTS stranded_sessions(
        thread              TEXT    NOT NULL,
        role                TEXT    NOT NULL,
        session             TEXT,
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
        summary             TEXT,
        PRIMARY KEY(thread, role, trigger_clock)
    );
    -- The ladder's record (nxf 6j6v.558h): one row per rung a coordinator ran on one slot — an
    -- ask, a continuation, a re-commission, a progress taken, the escalation — with what became of
    -- it. APPEND-ONLY and device-local: it is what `status` and rung E's escalation read, and a row
    -- whose `outcome` is still NULL is a rung in flight (the claim's recheck refuses a second one
    -- while it is younger than the trigger timeout).
    CREATE TABLE IF NOT EXISTS recovery_log(
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        thread  TEXT    NOT NULL,
        role    TEXT    NOT NULL,
        at      TEXT    NOT NULL,
        action  TEXT    NOT NULL,
        attempt INTEGER NOT NULL,
        session TEXT,
        outcome TEXT,
        detail  TEXT    NOT NULL
    );
    CREATE INDEX IF NOT EXISTS recovery_log_by_slot ON recovery_log(thread, role, id);
    -- The obligation a queued trigger carried when it met the working-copy queue (nxf 6j6v.558h,
    -- field case 4): its reply thread, or NULL for a trigger that owed nothing. Written at the
    -- enqueue, taken at the fire — so the release fires the entry with the obligation it was
    -- queued with instead of deriving one again.
    CREATE TABLE IF NOT EXISTS queued_obligations(
        session      TEXT NOT NULL,
        thread       TEXT NOT NULL,
        reply_thread TEXT,
        PRIMARY KEY(session, thread)
    );
";

/// **The ladder's columns on tables that exist before it** (nxf 6j6v.558h) — added in place,
/// column-presence-guarded, as `schema.rs` adds every device-local minor; a fresh store creates
/// them with its tables, so concurrent first opens of one file never race on an `ALTER`. They are:
/// the coordinator's summary
/// on a stranded record (what rung E posts), the next look a status row names, a session's last
/// cause and attempt count (what `nxc session ended --cause --attempts` says), and the outcome a
/// resume recorded on its hold.
pub(crate) fn add_ladder_columns(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    for (table, column, ddl) in [
        (
            "stranded_sessions",
            "summary",
            "ALTER TABLE stranded_sessions ADD COLUMN summary TEXT;",
        ),
        (
            "owed_starts",
            "next_look",
            "ALTER TABLE owed_starts ADD COLUMN next_look TEXT;",
        ),
        (
            "session_map",
            "last_cause",
            "ALTER TABLE session_map ADD COLUMN last_cause TEXT;",
        ),
        (
            "session_map",
            "last_cause_attempts",
            "ALTER TABLE session_map ADD COLUMN last_cause_attempts INTEGER;",
        ),
        (
            "session_interruption",
            "outcome",
            "ALTER TABLE session_interruption ADD COLUMN outcome TEXT;",
        ),
    ] {
        if table_exists(conn, table)? && !column_exists(conn, table, column)? {
            conn.execute_batch(ddl)?;
        }
    }
    Ok(())
}

/// **The three tables as cut 1a created them, rebuilt in place** (nxf 6j6v.kgfn) — `CREATE TABLE IF
/// NOT EXISTS` is a no-op for a table that exists, and SQLite cannot drop a `NOT NULL` or add a
/// primary-key-compatible shape with `ALTER TABLE`. So a table of the old shape is renamed aside,
/// created anew from [`STRANDED_DDL`], its rows copied over and the old one dropped — inside one
/// savepoint, so an interrupted rebuild leaves the old table standing. Idempotent: a table of the new
/// shape is left alone, so every open after the first does nothing.
pub(crate) fn rebuild_pre_slot_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let owed_is_old =
        table_exists(conn, "owed_starts")? && !column_exists(conn, "owed_starts", "coordinator")?;
    let suspects_is_old = session_is_not_null(conn, "stranded_suspects")?;
    let records_is_old = session_is_not_null(conn, "stranded_sessions")?;
    if !(owed_is_old || suspects_is_old || records_is_old) {
        return Ok(());
    }
    conn.execute_batch("SAVEPOINT pre_slot_rebuild;")?;
    let rebuilt = (|| -> rusqlite::Result<()> {
        if owed_is_old {
            conn.execute_batch(
                "DROP INDEX IF EXISTS owed_starts_by_session;
                 ALTER TABLE owed_starts RENAME TO owed_starts_pre_slot;",
            )?;
        }
        if suspects_is_old {
            conn.execute_batch(
                "ALTER TABLE stranded_suspects RENAME TO stranded_suspects_pre_slot;",
            )?;
        }
        if records_is_old {
            conn.execute_batch(
                "ALTER TABLE stranded_sessions RENAME TO stranded_sessions_pre_slot;",
            )?;
        }
        conn.execute_batch(STRANDED_DDL)?;
        if owed_is_old {
            conn.execute_batch(
                "INSERT INTO owed_starts(thread, role, session, trigger_clock, triggered)
                     SELECT thread, role, session, trigger_clock, triggered FROM owed_starts_pre_slot;
                 DROP TABLE owed_starts_pre_slot;",
            )?;
        }
        if suspects_is_old {
            conn.execute_batch(
                "INSERT INTO stranded_suspects(thread, role, session, first_seen, bound,
                         transcript_entries)
                     SELECT thread, role, session, first_seen, bound, transcript_entries
                       FROM stranded_suspects_pre_slot;
                 DROP TABLE stranded_suspects_pre_slot;",
            )?;
        }
        if records_is_old {
            conn.execute_batch(&format!(
                "INSERT INTO stranded_sessions({RECORD_COLS})
                     SELECT {RECORD_COLS} FROM stranded_sessions_pre_slot;
                 DROP TABLE stranded_sessions_pre_slot;"
            ))?;
        }
        Ok(())
    })();
    match rebuilt {
        Ok(()) => conn.execute_batch("RELEASE pre_slot_rebuild;"),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO pre_slot_rebuild; RELEASE pre_slot_rebuild;");
            Err(e)
        }
    }
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
}

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |r| r.get(0),
    )
}

/// Whether `table` exists with its `session` column declared `NOT NULL` — cut 1a's shape.
fn session_is_not_null(conn: &rusqlite::Connection, table: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info(?1) WHERE name = 'session'",
            [table],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        == Some(1))
}

/// **The key a look at one slot is filed under** (nxf 6j6v.kgfn): the thread and the role's bare
/// handle — a slot, not a session, because a slot is owed before any session exists, and one look
/// per slot is what the service book keeps (`stranded-look-<key>`).
pub(crate) fn look_key(thread: &str, role: &str) -> String {
    let bare = role.rsplit_once('/').map_or(role, |(_, b)| b);
    format!("{thread}-{bare}")
}

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

/// **A backfilled slot whose newest evidence is older than this is abandoned** (nxf 6j6v.kgfn; epic
/// 6j6v.nmc9 section A) — one day. The evidence is the session's end, else its last transcript
/// entry, else the slot's open. Strictly older: a slot exactly this old is still taken up.
pub const ABANDONED_AFTER_SECS: i64 = 86_400;

/// How long a stranded record is kept once written — the retention bound of `stranded_sessions`.
pub const STRANDED_RETENTION_DAYS: i64 = 30;

/// **The obligation record** — thread `thread` owes an answer from `role`; the SLOT (nxf
/// 6j6v.kgfn), owed from the instant it is opened, and `session` is the one that owes it once a
/// session exists.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct OwedStart {
    pub thread: String,
    /// The qualified handle, as the thread register names it.
    pub role: String,
    /// `None` until the mint fills it — a slot whose session was never minted stays `None`.
    pub session: Option<String>,
    /// The instant the slot is judged from when its session has no end: the open instant, then the
    /// mint's, then every trigger's.
    pub trigger_clock: String,
    /// Whether any trigger ever ran for this obligation. `false` is a slot that was opened and whose
    /// start never happened.
    pub triggered: bool,
    /// The thread whose tick coordinates this slot, as [`coordinator_of`] named it when the row was
    /// written. `None` on a row cut 1a wrote before this column existed.
    pub coordinator: Option<String>,
    /// Written by the one-time backfill for a slot opened before this cut.
    pub backfilled: bool,
    /// The ladder's budget (6j6v.558h) — `0` until it is built.
    pub attempts: u32,
    /// The ladder's claim (6j6v.558h).
    pub recovering_until: Option<String>,
    /// The ladder's ceiling anchor (6j6v.558h).
    pub recovery_started_at: Option<String>,
    /// The ladder's last ask (6j6v.558h).
    pub asked_at: Option<String>,
    /// The ladder's last sign of life (6j6v.558h).
    pub last_progress_at: Option<String>,
    /// Why the slot was given up, when it was: `delivered` / `withdrawn` (the root no longer
    /// waits), `aged` (its newest evidence is older than [`ABANDONED_AFTER_SECS`]), `consolidated`
    /// (its commissioning message is gone). Never nudged; `withdraw` is its exit.
    pub abandoned: Option<String>,
    pub abandoned_at: Option<String>,
}

/// **One row the one-time backfill writes** (nxf 6j6v.kgfn) — an open coordinated slot opened before
/// this cut, which therefore has no row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Backfill {
    pub thread: String,
    pub role: String,
    /// The session of that role standing on the thread, when one does (`session_map`).
    pub session: Option<String>,
    pub coordinator: String,
    pub clock: String,
    /// The reason it is abandoned, when the rule says so.
    pub abandoned: Option<String>,
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
    /// 6 — its process stayed alive and silent on its slot through every ask its coordinator put
    /// to it (nxf 6j6v.558h, rung A); recorded only by the ladder's escalation.
    Silent,
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
            Observation::Silent => 6,
        }
    }

    fn from_number(n: i64) -> Option<Observation> {
        Some(match n {
            1 => Observation::NeverStarted,
            2 => Observation::NeverBound,
            3 => Observation::EndedWithoutAnswer,
            4 => Observation::NoLiveProcess,
            5 => Observation::StartGivenUp,
            6 => Observation::Silent,
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
    /// `None` for a slot that never got a session (nxf 6j6v.kgfn) — observation 1.
    pub session: Option<String>,
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
    pub session: Option<String>,
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
        // A number this build does not know is a record a newer build wrote. Dropping the row would
        // hide it; so it reads as "no live process" — the most general of the five, never wrong about
        // a stranded session — and its detail SAYS which number it carried, so nothing shows a
        // newer observation as if it were that one.
        observed: Observation::from_number(observed).unwrap_or(Observation::NoLiveProcess),
        detail: match Observation::from_number(observed) {
            Some(_) => r.get(7)?,
            None => format!(
                "{} (recorded as observation {observed}, which this build does not know)",
                r.get::<_, String>(7)?
            ),
        },
        escalation: escalation.as_deref().and_then(Escalation::parse),
        escalation_attempts: u32::try_from(attempts).unwrap_or(u32::MAX),
        escalated_at: r.get(10)?,
        escalation_error: r.get(11)?,
        superseded_at: r.get(12)?,
    })
}

const OWED_COLS: &str = "thread, role, session, trigger_clock, triggered, coordinator, \
     backfilled, attempts, recovering_until, recovery_started_at, asked_at, last_progress_at, \
     abandoned, abandoned_at";

fn owed_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OwedStart> {
    let triggered: i64 = r.get(4)?;
    let backfilled: i64 = r.get(6)?;
    let attempts: i64 = r.get(7)?;
    Ok(OwedStart {
        thread: r.get(0)?,
        role: r.get(1)?,
        session: r.get(2)?,
        trigger_clock: r.get(3)?,
        triggered: triggered != 0,
        coordinator: r.get(5)?,
        backfilled: backfilled != 0,
        attempts: u32::try_from(attempts).unwrap_or(0),
        recovering_until: r.get(8)?,
        recovery_started_at: r.get(9)?,
        asked_at: r.get(10)?,
        last_progress_at: r.get(11)?,
        abandoned: r.get(12)?,
        abandoned_at: r.get(13)?,
    })
}

impl ChatStore {
    /// **The slot is OPENED: `thread` owes `role` from `clock`, and no session exists yet** (nxf
    /// 6j6v.kgfn). The row the obligation is from now on — written before any mint, so a slot whose
    /// session is never minted still has a clock and a look. Re-opening the same `(thread, role)`
    /// starts it over: no session, not triggered, not abandoned.
    pub(crate) fn open_slot(
        &mut self,
        thread: &str,
        role: &str,
        coordinator: &str,
        clock: &str,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO owed_starts(thread, role, session, trigger_clock, triggered, coordinator)
             VALUES (?1, ?2, NULL, ?3, 0, ?4)
             ON CONFLICT(thread, role) DO UPDATE SET
                 session = NULL, trigger_clock = excluded.trigger_clock, triggered = 0,
                 coordinator = excluded.coordinator, abandoned = NULL, abandoned_at = NULL",
            params![thread, role, clock, coordinator],
        )?;
        supersede_older(conn, thread, role, clock)?;
        Ok(())
    }

    /// **The mint fills the slot's session and moves its clock** (nxf 6j6v.kgfn). Not a trigger:
    /// `triggered` is left as it is. Answers whether a row moved — a slot opened without a row
    /// (no coordinator) has none to fill.
    pub(crate) fn bind_owed_session(
        &mut self,
        thread: &str,
        role: &str,
        session: &str,
        clock: &str,
    ) -> Result<bool> {
        let conn = self.connection();
        let n = conn.execute(
            "UPDATE owed_starts SET session = ?3, trigger_clock = ?4
              WHERE thread = ?1 AND role = ?2",
            params![thread, role, session, clock],
        )?;
        if n > 0 {
            supersede_older(conn, thread, role, clock)?;
        }
        Ok(n > 0)
    }

    /// **Record (or re-record) the obligation `thread` owes `role` on `session`** — the funnel's
    /// write. `coordinator` is the slot's coordinating thread when it has one; `None` is a slot
    /// nobody coordinates, which keeps cut 1a's row exactly as it was (a `coordinator` an earlier
    /// write named is left as it is — the next look decides whether it still holds).
    ///
    /// An upsert by `(thread, role)`: a new session for the same obligation replaces the session,
    /// and the same session re-triggered moves only the clock. What the open wrote about the slot —
    /// and what the ladder will write about its recovery — stays. Every stranded record of the same
    /// obligation judged against an OLDER clock is stamped superseded in the same call — a later
    /// trigger is what ends a record's currency, and it is the one event that does.
    pub(crate) fn record_owed_start(
        &mut self,
        thread: &str,
        role: &str,
        session: &str,
        clock: &str,
        triggered: bool,
        coordinator: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO owed_starts(thread, role, session, trigger_clock, triggered, coordinator)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(thread, role) DO UPDATE SET
                 session = excluded.session,
                 trigger_clock = excluded.trigger_clock,
                 triggered = excluded.triggered,
                 coordinator = COALESCE(excluded.coordinator, owed_starts.coordinator)",
            params![
                thread,
                role,
                session,
                clock,
                i64::from(triggered),
                coordinator
            ],
        )?;
        supersede_older(conn, thread, role, clock)?;
        Ok(())
    }

    /// Name the thread that coordinates a slot whose row did not say (a row cut 1a wrote), or no
    /// longer said the right one — what a look found `coordinator_of` answering.
    pub(crate) fn set_slot_coordinator(
        &mut self,
        thread: &str,
        role: &str,
        coordinator: &str,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE owed_starts SET coordinator = ?3 WHERE thread = ?1 AND role = ?2",
            params![thread, role, coordinator],
        )?;
        Ok(())
    }

    /// Whether the one-time backfill has run on this device.
    pub(crate) fn owed_backfill_done(&self) -> Result<bool> {
        Ok(self.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM owed_backfill_done)",
            [],
            |r| r.get(0),
        )?)
    }

    /// **The one-time backfill: `rows` and the marker, in ONE transaction** (nxf 6j6v.kgfn; must-fix
    /// of the design review, round 2 IR #9). `Ok(false)` when the marker already stands — nothing is
    /// written. A failure anywhere rolls back every row with the marker, so a crash between them
    /// leaves neither and the next sweep backfills again. A row that already exists is left alone:
    /// the backfill fills slots that HAVE no row.
    pub(crate) fn backfill_owed_starts(&mut self, rows: &[Backfill], now: &str) -> Result<bool> {
        let tx = self.connection().unchecked_transaction()?;
        let done: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM owed_backfill_done)", [], |r| {
                r.get(0)
            })?;
        if done {
            return Ok(false);
        }
        for row in rows {
            tx.execute(
                "INSERT INTO owed_starts(thread, role, session, trigger_clock, triggered,
                     coordinator, backfilled, abandoned, abandoned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, CASE WHEN ?7 IS NULL THEN NULL ELSE ?8 END)
                 ON CONFLICT(thread, role) DO NOTHING",
                params![
                    row.thread,
                    row.role,
                    row.session,
                    row.clock,
                    i64::from(row.session.is_some()),
                    row.coordinator,
                    row.abandoned,
                    now
                ],
            )?;
        }
        tx.execute("INSERT INTO owed_backfill_done(done_at) VALUES (?1)", [now])?;
        tx.commit()?;
        Ok(true)
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
    pub(crate) fn owed_start(&self, thread: &str, role: &str) -> Result<Option<OwedStart>> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {OWED_COLS} FROM owed_starts WHERE thread = ?1 AND role = ?2"),
                params![thread, role],
                owed_from_row,
            )
            .optional()?)
    }

    /// Every obligation record, in `(thread, role)` order — the sweep's work list. Bounded by what is
    /// open: the sweep deletes a row one look after its thread stops owing the role.
    pub(crate) fn owed_starts(&self) -> Result<Vec<OwedStart>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {OWED_COLS} FROM owed_starts ORDER BY thread, role"
        ))?;
        let rows = st
            .query_map([], owed_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The obligation records `session` owes — what a session's end looks at.
    pub(crate) fn owed_starts_of_session(&self, session: &str) -> Result<Vec<OwedStart>> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {OWED_COLS} FROM owed_starts WHERE session = ?1 ORDER BY thread, role"
        ))?;
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

    /// Whether `(thread, role)` was already recorded against `trigger_clock`.
    pub(crate) fn stranded_recorded(
        &self,
        thread: &str,
        role: &str,
        trigger_clock: &str,
    ) -> Result<bool> {
        Ok(self.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM stranded_sessions
                WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3)",
            params![thread, role, trigger_clock],
            |r| r.get(0),
        )?)
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

    /// **Retention**: drop the records written before `cutoff`. Returns how many went. Compared as
    /// instants, for [`supersede_older`]'s reason; the table holds what was stranded in a month.
    pub(crate) fn forget_stranded_before(&mut self, cutoff: &str) -> Result<usize> {
        let conn = self.connection();
        let stamps: Vec<String> = {
            let mut st = conn.prepare("SELECT DISTINCT recorded_at FROM stranded_sessions")?;
            let rows = st.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut gone = 0;
        for stamp in stamps.iter().filter(|s| earlier(s, cutoff)) {
            gone += conn.execute(
                "DELETE FROM stranded_sessions WHERE recorded_at = ?1",
                params![stamp],
            )?;
        }
        Ok(gone)
    }

    /// How many transcript entries `session` has written — half of the first look's evidence.
    pub(crate) fn transcript_entry_count(&self, session: &str) -> Result<i64> {
        Ok(self.connection().query_row(
            "SELECT COUNT(*) FROM agent_transcript WHERE internal_session = ?1",
            [session],
            |r| r.get(0),
        )?)
    }

    /// The instant of `session`'s newest transcript entry, when it wrote any — the "last transcript
    /// activity" `border.rs` reads the same way.
    pub(crate) fn last_transcript_at(&self, session: &str) -> Result<Option<String>> {
        Ok(self.connection().query_row(
            "SELECT MAX(at) FROM agent_transcript WHERE internal_session = ?1",
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
///
/// Compared as INSTANTS in Rust, never as text in SQL: a fractional second and a whole one, or two
/// offsets, do not sort as strings the way they do as time. The rows are one obligation's records,
/// a handful at most.
fn supersede_older(
    conn: &rusqlite::Connection,
    thread: &str,
    role: &str,
    clock: &str,
) -> Result<()> {
    let clocks: Vec<String> = {
        let mut st = conn.prepare(
            "SELECT trigger_clock FROM stranded_sessions
              WHERE thread = ?1 AND role = ?2 AND superseded_at IS NULL",
        )?;
        let rows = st.query_map(params![thread, role], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for older in clocks.iter().filter(|c| earlier(c, clock)) {
        conn.execute(
            "UPDATE stranded_sessions SET superseded_at = ?4
              WHERE thread = ?1 AND role = ?2 AND trigger_clock = ?3",
            params![thread, role, older, clock],
        )?;
    }
    Ok(())
}

/// Whether instant `a` is strictly before instant `b`. An instant that does not parse is never
/// earlier — the fail-safe direction for both callers is "leave the row as it is".
pub(crate) fn earlier(a: &str, b: &str) -> bool {
    use time::format_description::well_known::Rfc3339;
    match (
        time::OffsetDateTime::parse(a, &Rfc3339),
        time::OffsetDateTime::parse(b, &Rfc3339),
    ) {
        (Ok(a), Ok(b)) => a < b,
        _ => false,
    }
}

// ---- who coordinates a slot (nxf 6j6v.kgfn) -----------------------------------------------------

/// **Which kind of coordinator a slot has** — the rows of epic 6j6v.nmc9 section A's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CoordinatorKind {
    /// A flow step or a parallel member: the channel thread's tick.
    Channel,
    /// A direct commission (`send --to <persona>`) by a person: the root thread's tick.
    Root,
    /// A direct commission by a SESSION's own `nxc send`: the sending session's thread.
    Session,
    /// A slot that stands for a nested round (a step whose answer is a channel's discharge): the
    /// parent channel thread, coordinated while any open slot inside the round is.
    NestedRound,
}

/// **The software coordinator of one slot** — the thread whose tick coordinates it, and which kind
/// of coordinator that is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CoordinatorRef {
    pub thread: String,
    pub kind: CoordinatorKind,
}

/// What [`coordinator_of`]'s clauses found — the one evaluation the predicate, the open, the funnel
/// and the backfill share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Assessment {
    /// Every clause holds.
    Coordinated(CoordinatorRef),
    /// Clauses 1-4 and 6 hold, and the slot's commissioning message is gone (clause 5): a backfilled
    /// slot of this shape is abandoned with reason `consolidated`.
    Consolidated(CoordinatorRef),
    /// No coordinator: the slot keeps today's behaviour in full.
    Uncoordinated,
    /// Clauses 1, 2, 4-6 hold and clause 3 cannot be decided here: the thread's `machine` register
    /// names a machine, and this device cannot say which machine it is (no machine read, or it
    /// failed). A row that already names a coordinator KEEPS it then (nxf 6j6v.ndbr) — the sidecar
    /// gave up its substitute for that slot, so forgetting the row would leave it with neither.
    MachineUnknown(CoordinatorRef),
}

/// **`coordinator_of(slot)`** (nxf 6j6v.kgfn; epic 6j6v.nmc9 section A) — `Some` iff ALL six
/// clauses hold for the slot `(thread, role)`, evaluated afresh at every call (nothing is cached,
/// so a hand-over moves ownership with the register and a worker that stops answering takes it
/// back):
///
/// 1. `role` is a persona declared in THIS workspace — not a human, not an engine identity, not a
///    foreign or border handle;
/// 2. the thread is not an inbound border thread, and the slot is OPEN for `role` (neither
///    answered, escalated nor awaiting a human — all three discharge the outstanding handle), and
///    the register that opened it may carry an action here;
/// 3. THIS DEVICE EXECUTES THE SLOT: the thread's `machine` register names this machine, or it is
///    unset and this site wrote the expectation;
/// 4. the worker on this device answers liveness, stops sessions AND asks them;
/// 5. the slot's commissioning message exists on its thread — whoever posted it: the channel tick,
///    a person's `send --to`, a session's own `nxc send`;
/// 6. a slot that stands for a nested round (the outstanding handle is the channel supervisor): iff
///    any open slot of that round is coordinated.
///
/// A slot with no coordinator keeps today's behaviour in full: the sidecar's substitute, the
/// staleness lapse, the `LivenessUnknown` finding.
pub fn coordinator_of(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
    role: &str,
) -> Result<Option<CoordinatorRef>> {
    Ok(match assess(ctx, store, thread, role)? {
        Assessment::Coordinated(c) => Some(c),
        Assessment::Consolidated(_) | Assessment::Uncoordinated | Assessment::MachineUnknown(_) => {
            None
        }
    })
}

/// The six clauses, in one place — see [`coordinator_of`].
pub(crate) fn assess(ctx: &Ctx, store: &ChatStore, thread: &str, role: &str) -> Result<Assessment> {
    let Some(q) = store.thread_quorum(thread, ctx.now)? else {
        return Ok(Assessment::Uncoordinated);
    };
    // Clause 2's second half, asked first because every other clause is about an OPEN slot: the
    // role still owes this thread an answer, and nothing that opened the obligation is unvouched.
    if q.held || !outstanding_has(&q.outstanding, role) {
        return Ok(Assessment::Uncoordinated);
    }
    let supervisor = crate::orchestration::supervisor_qualified(ctx.origin);
    if crate::facade::same_identity(Some(role), &supervisor) {
        return nested_round(ctx, store, thread);
    }
    // Clause 1.
    if crate::orchestration::is_engine_identity(role) || !is_local_persona(ctx, role) {
        return Ok(Assessment::Uncoordinated);
    }
    // Clause 2: an inbound border thread is coordinated on the side that commissioned it.
    if matches!(crate::border::row(store, thread)?, Some(r) if r.direction == crate::border::Direction::Inbound)
    {
        return Ok(Assessment::Uncoordinated);
    }
    // Clause 4, before the reads clause 3 makes.
    if !worker_can_coordinate(ctx.worker) {
        return Ok(Assessment::Uncoordinated);
    }
    // Clause 3.
    match executes_here_known(ctx, store, thread)? {
        Some(true) => {}
        Some(false) => return Ok(Assessment::Uncoordinated),
        None => {
            return Ok(Assessment::MachineUnknown(coordinator_ref(
                ctx,
                store,
                thread,
                q.opener.as_deref(),
            )?))
        }
    }
    let coordinator = coordinator_ref(ctx, store, thread, q.opener.as_deref())?;
    // Clause 5.
    if store.acting_messages_in_thread(thread)?.is_empty() {
        return Ok(Assessment::Consolidated(coordinator));
    }
    Ok(Assessment::Coordinated(coordinator))
}

/// **[`coordinator_of`] for a slot this call is ABOUT TO OPEN** (nxf 6j6v.kgfn) — asked by
/// `coordinator_commission_in` before the expectation exists, so that a coordinated slot can be owed
/// before its session is minted. The same clauses, with the two that read the expectation answered
/// for the one this site is about to write: the slot will be open for `role` (clause 2), and when no
/// `machine` register names a machine, this site writes the expectation (clause 3). The
/// commissioning message is already posted (clause 5).
pub(crate) fn assess_at_open(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
    role: &str,
) -> Result<Option<CoordinatorRef>> {
    if crate::orchestration::is_engine_identity(role) || !is_local_persona(ctx, role) {
        return Ok(None);
    }
    if matches!(crate::border::row(store, thread)?, Some(r) if r.direction == crate::border::Direction::Inbound)
    {
        return Ok(None);
    }
    if !worker_can_coordinate(ctx.worker) {
        return Ok(None);
    }
    let executes_here = match store.thread_machine(thread)? {
        Some(_) => executes_here(ctx, store, thread)?,
        None => true,
    };
    if !executes_here || store.acting_messages_in_thread(thread)?.is_empty() {
        return Ok(None);
    }
    let opener = store.thread_quorum(thread, ctx.now)?.and_then(|q| q.opener);
    Ok(Some(coordinator_ref(
        ctx,
        store,
        thread,
        opener.as_deref(),
    )?))
}

/// Clause 4: the three capabilities a software coordinator needs of the worker on this device.
pub(crate) fn worker_can_coordinate(worker: &dyn crate::worker::Worker) -> bool {
    worker.answers_liveness() && worker.stops_sessions() && worker.asks_sessions()
}

/// Clause 1's test: `role` names a persona declared in THIS workspace's catalogue, qualified with
/// this workspace's origin.
fn is_local_persona(ctx: &Ctx, role: &str) -> bool {
    let Some((prefix, bare)) = role.rsplit_once('/') else {
        return false;
    };
    prefix == ctx.origin && ctx.defs.role(bare).is_ok()
}

/// Clause 3: whether THIS device executes the slot on `thread`. Re-read at every call — the
/// register is the one replicated fact a hand-over moves (`handed_to` is a receipt field only).
fn executes_here(ctx: &Ctx, store: &ChatStore, thread: &str) -> Result<bool> {
    Ok(match store.thread_machine(thread)? {
        Some(m) => {
            m.acts
                && ctx
                    .machines
                    .and_then(|ms| ms.here())
                    .is_some_and(|here| here.machine_id == m.machine_id)
        }
        None => store.expects_reply_from_site(thread)? == Some(store.site()),
    })
}

/// Clause 3 with its third answer (nxf 6j6v.ndbr): `None` when the register names a machine this
/// device cannot place — no machine read, or one that does not say which machine this is.
fn executes_here_known(ctx: &Ctx, store: &ChatStore, thread: &str) -> Result<Option<bool>> {
    Ok(match store.thread_machine(thread)? {
        Some(m) if !m.acts => Some(false),
        Some(m) => ctx
            .machines
            .and_then(|ms| ms.here())
            .map(|here| here.machine_id == m.machine_id),
        None => Some(store.expects_reply_from_site(thread)? == Some(store.site())),
    })
}

/// Which thread coordinates the slot on `thread`, opened by `opener` — section A's table.
fn coordinator_ref(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
    opener: Option<&str>,
) -> Result<CoordinatorRef> {
    let supervisor = crate::orchestration::supervisor_qualified(ctx.origin);
    let parent = store.thread_parent(thread)?;
    let opened_by =
        |handle: &str| opener.is_some_and(|o| crate::facade::same_identity(Some(o), handle));
    Ok(if opened_by(&supervisor) {
        CoordinatorRef {
            thread: parent.unwrap_or_else(|| thread.to_string()),
            kind: CoordinatorKind::Channel,
        }
    } else if opener.is_some_and(|o| is_local_persona(ctx, o)) && parent.is_some() {
        CoordinatorRef {
            thread: parent.unwrap_or_default(),
            kind: CoordinatorKind::Session,
        }
    } else {
        CoordinatorRef {
            thread: store.thread_root(thread)?,
            kind: CoordinatorKind::Root,
        }
    })
}

/// Clause 6: the slot on `thread` stands for a nested round — coordinated iff any open slot of that
/// round (a member thread under it, opened by the supervisor) is.
fn nested_round(ctx: &Ctx, store: &ChatStore, thread: &str) -> Result<Assessment> {
    let supervisor = crate::orchestration::supervisor_qualified(ctx.origin);
    for child in store.thread_children(thread)? {
        let Some(member) = store.thread_quorum(&child, ctx.now)? else {
            continue;
        };
        if !member
            .opener
            .as_deref()
            .is_some_and(|o| crate::facade::same_identity(Some(o), &supervisor))
        {
            continue;
        }
        for handle in &member.outstanding {
            if let Assessment::Coordinated(_) = assess(ctx, store, &child, handle)? {
                return Ok(Assessment::Coordinated(CoordinatorRef {
                    thread: store
                        .thread_parent(thread)?
                        .unwrap_or_else(|| thread.to_string()),
                    kind: CoordinatorKind::NestedRound,
                }));
            }
        }
    }
    Ok(Assessment::Uncoordinated)
}

// ---- what is still coming ------------------------------------------------------------------

/// **"This member's answer is still coming"** — the one predicate over a member's OWED session
/// (nxf 6j6v.ccvy). Each reason pauses the member's window, unbounded in this cut by decision: each
/// is recorded and visible, and each has its own exit (6j6v.21xy owns what a due hold nothing takes
/// up should become).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StillComing {
    /// In the working-copy queue (nxf 6j6v.1wep).
    Queued,
    /// A failed start whose retry is due (nxf 6j6v.br25).
    RetryDue,
    /// The owed session has a standing hold — the model went away (5dzm (a), 0990). `until` is the
    /// instant the runtime named, when it named one.
    OnHold {
        limit: String,
        until: Option<String>,
    },
    /// The member opened a sub-round of its own and that round is still in flight (its own window,
    /// its own slots — if IT strands, the sweep escalates it there).
    SubRoundInFlight,
    /// An answer is held for the owed session, to be handed over when its turn ends — from a thread
    /// that is not a child of this slot (a consultation the session opened from where it stood
    /// before, since a session's position moves). A held answer to a sub-round of THIS slot is
    /// [`SubRoundInFlight`](StillComing::SubRoundInFlight) already (build item 8).
    HeldDelivery,
}

impl StillComing {
    /// The reason in words, for the receipt.
    pub(crate) fn says(&self) -> String {
        match self {
            StillComing::Queued => "its start still waits for the working copy".to_string(),
            StillComing::RetryDue => "its start failed and a retry is due".to_string(),
            StillComing::OnHold { limit, until } => match until {
                Some(until) => format!("its session is on hold ({limit}) until {until}"),
                None => format!("its session is on hold ({limit}) with no instant named"),
            },
            StillComing::SubRoundInFlight => {
                "it is waiting for a round it commissioned itself".to_string()
            }
            StillComing::HeldDelivery => {
                "an answer is held for its session, to be handed over when its turn ends"
                    .to_string()
            }
        }
    }
}

/// **What the "still coming" predicate reads in bulk** — taken once per call that walks a set, so
/// both readers ([`crate::orchestration`]'s member set and this module's sweep) hand the same facts
/// to the same predicate.
pub(crate) struct PauseFacts {
    /// Threads whose trigger waits in the working-copy queue.
    queued: std::collections::HashSet<String>,
    /// Every failed start on record.
    failed: Vec<crate::working_tree::FailedStart>,
    /// Threads whose answer is held for a commissioner still mid-turn.
    held_answers: std::collections::HashSet<String>,
}

impl PauseFacts {
    pub(crate) fn read(store: &ChatStore) -> Result<PauseFacts> {
        Ok(PauseFacts {
            queued: store
                .list_working_tree_queue()?
                .into_iter()
                .filter_map(|q| q.thread)
                .collect(),
            failed: store.failed_starts()?,
            held_answers: store.threads_holding_wakes()?,
        })
    }

    /// The failed start that belongs to this slot: recorded against its THREAD (the queue's own
    /// key, and the only one a slot without an obligation record has) or against its OWED SESSION.
    /// One keying for both readers.
    fn failed_start(
        &self,
        thread: &str,
        owed: Option<&str>,
    ) -> Option<&crate::working_tree::FailedStart> {
        self.failed
            .iter()
            .find(|f| f.thread.as_deref() == Some(thread) || owed.is_some_and(|s| f.session == s))
    }

    /// A start of this slot that failed and will NOT be retried — the sweep's observation 5.
    pub(crate) fn given_up(
        &self,
        thread: &str,
        owed: Option<&str>,
    ) -> Option<&crate::working_tree::FailedStart> {
        self.failed_start(thread, owed)
            .filter(|f| f.retry_at.is_none())
    }
}

/// **"This member's answer is still coming"** — THE predicate (nxf 6j6v.ccvy, A), asked by the
/// member-set read that decides settledness and by the stranded sweep's rules 2-4 alike, so what
/// "paused" means is written once. `owed` is the slot's owed session (`None` for a slot with no
/// obligation record, where only the queue and a retry can say anything). `None` when its answer is
/// not coming by any of the five routes.
pub(crate) fn member_answer_is_still_coming(
    now: &str,
    store: &ChatStore,
    member: &crate::store::ThreadQuorum,
    owed: Option<&str>,
    facts: &PauseFacts,
) -> Result<Option<StillComing>> {
    if facts.queued.contains(&member.thread_id) {
        return Ok(Some(StillComing::Queued));
    }
    if facts
        .failed_start(&member.thread_id, owed)
        .is_some_and(|f| f.retry_at.is_some())
    {
        return Ok(Some(StillComing::RetryDue));
    }
    let Some(session) = owed else {
        return Ok(None);
    };
    if let Some(hold) = store.current_interruption(session)? {
        return Ok(Some(StillComing::OnHold {
            limit: hold.limit,
            until: hold.until,
        }));
    }
    let children = store.thread_children(&member.thread_id)?;
    let child_ids: Vec<&str> = children.iter().map(String::as_str).collect();
    let kids = store.thread_quorums(&child_ids, now)?;
    if !crate::awaiting::own_sub_round_in_flight(member, kids.iter(), &facts.held_answers)
        .is_empty()
    {
        return Ok(Some(StillComing::SubRoundInFlight));
    }
    if store.has_pending_wake(session)? {
        return Ok(Some(StillComing::HeldDelivery));
    }
    Ok(None)
}

// ---- the sweep ---------------------------------------------------------------------------------

/// `at` plus `secs`, in the one canonical form every stored instant uses.
pub(crate) fn after(at: &str, secs: i64, what: &str) -> Result<String> {
    crate::facade::instant_after(at, time::Duration::seconds(secs), what)
}

/// Whether `now` is at or past `at`. Compared as INSTANTS, never as strings: a fractional second and
/// a whole one do not sort as text the way they do as time. An instant that does not parse has not
/// been reached — the fail-safe direction here is "do not record".
pub(crate) fn reached(now: &str, at: &str) -> bool {
    use time::format_description::well_known::Rfc3339;
    match (
        time::OffsetDateTime::parse(now, &Rfc3339),
        time::OffsetDateTime::parse(at, &Rfc3339),
    ) {
        (Ok(now), Ok(at)) => now >= at,
        _ => false,
    }
}

/// The instant handed to a quorum read that only wants a thread's OPENER, which no clock changes.
const ANY_INSTANT: &str = "1970-01-01T00:00:00Z";

/// **The thread a look at an obligation of `thread` ticks** — a supervised slot's CHANNEL thread,
/// and the thread itself for anything else (a direct commission, a served chat, a border order).
///
/// Never a supervised slot: `tick` reads a slot as a thread with no members, and a stale one would
/// be consolidated by its own tick. The channel thread's tick changes nothing but through the sweep
/// until the round itself is due.
pub(crate) fn look_thread(store: &ChatStore, origin: &str, thread: &str) -> Result<String> {
    let supervisor = crate::orchestration::supervisor_qualified(origin);
    let opened_by_the_supervisor = store
        .thread_quorum(thread, ANY_INSTANT)?
        .is_some_and(|q| q.opener.as_deref() == Some(supervisor.as_str()));
    if opened_by_the_supervisor {
        if let Some(parent) = store.thread_parent(thread)? {
            return Ok(parent);
        }
    }
    Ok(thread.to_string())
}

/// **Arm a look at the slot `(thread, role)` for `at`** — best-effort, and REPORTED: a look that
/// cannot be armed is a finding on the receipt of whoever tried, and the sweep is workspace-wide, so
/// the next tick anybody causes looks anyway. Filed under the SLOT ([`look_key`], nxf 6j6v.kgfn): a
/// slot is owed before any session exists.
pub(crate) fn arm_look(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
    role: &str,
    at: &str,
) -> Option<FailedConsequence> {
    let slot = look_key(thread, role);
    // The finding names the slot's session when it has one, so the receipt of whoever started it
    // can tell its own look from another's (`open_flow_step` keeps exactly those).
    let unarmed = |error: &str| {
        let session = store
            .owed_start(thread, role)
            .ok()
            .flatten()
            .and_then(|r| r.session);
        look_unarmed(thread, &slot, session.as_deref(), at, error)
    };
    let target = match look_thread(store, ctx.origin, thread) {
        Ok(t) => t,
        Err(e) => return Some(unarmed(&e.to_string())),
    };
    match ctx.timer.schedule_stranded_look(&slot, &target, at) {
        Ok(_) => None,
        // **A service that is down is said once, by the call, not once per look** — the service
        // backend writes the look into the book and THEN refuses, so the look is durable and will
        // be honoured when a service starts; the `service_not_running` finding the outermost verbs
        // add already says that nothing attends the workspace. A breadcrumb stays.
        Err(e) if ctx.timer.service_fault().is_some() => {
            eprintln!(
                "warning: the look at slot {slot} is booked for {at}, and nothing attends this \
                 workspace to honour it yet: {e}"
            );
            None
        }
        Err(e) => Some(unarmed(&e.to_string())),
    }
}

fn look_unarmed(
    thread: &str,
    slot: &str,
    session: Option<&str>,
    at: &str,
    error: &str,
) -> FailedConsequence {
    let detail = format!(
        "the look at slot {slot} (owing on thread {thread}) could not be armed for {at}: {error} \
         — the next tick of any thread in this workspace looks instead; `nxc tick --thread \
         {thread}` looks now"
    );
    eprintln!("warning: {detail}");
    FailedConsequence::stranded(
        ConsequenceClass::TickUnscheduled,
        Some(thread),
        session,
        detail,
    )
}

/// **Arm the first look for an obligation just written** — `clock + STRANDED_GRACE_SECS`.
pub(crate) fn arm_first_look(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
    role: &str,
    clock: &str,
) -> Option<FailedConsequence> {
    match after(clock, STRANDED_GRACE_SECS, "the first stranded look") {
        Ok(at) => arm_look(ctx, store, thread, role, &at),
        Err(e) => Some(look_unarmed(
            thread,
            &look_key(thread, role),
            None,
            clock,
            &e.to_string(),
        )),
    }
}

/// The finding for an obligation row that could not be written. Best-effort, everywhere: a store
/// error is a finding, never a failed commission — the row only adds a judgement on top of what the
/// commission already did, and a slot without one keeps today's behaviour.
fn not_owed(
    thread: &str,
    role: &str,
    session: Option<&str>,
    e: &dyn std::fmt::Display,
) -> Vec<FailedConsequence> {
    let detail = format!(
        "the obligation of {role} on thread {thread} could not be recorded: {e} — a slot that is \
         never answered there is not noticed by the stranded sweep, and its window lapses as before"
    );
    eprintln!("warning: {detail}");
    vec![FailedConsequence::stranded(
        ConsequenceClass::StrandedNotJudged,
        Some(thread),
        session,
        detail,
    )]
}

/// **The slot is opened: write its row with no session, and arm its first look** (nxf 6j6v.kgfn) —
/// before any mint, at the two places a coordinated slot is opened (`coordinator_commission_in`,
/// which every flow step, parallel member and direct commission passes through). `coordinator` is
/// [`coordinator_of`]'s thread.
pub(crate) fn owe_the_slot(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread: &str,
    role: &str,
    coordinator: &str,
) -> Vec<FailedConsequence> {
    if let Err(e) = store.open_slot(thread, role, coordinator, ctx.now) {
        return not_owed(thread, role, None, &e);
    }
    arm_first_look(ctx, store, thread, role, ctx.now)
        .into_iter()
        .collect()
}

/// **The mint fills the slot's session** (nxf 6j6v.kgfn) and moves its clock; the look moves with
/// it. Nothing for a slot that has no row.
pub(crate) fn bind(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread: &str,
    role: &str,
    session: &str,
) -> Vec<FailedConsequence> {
    match store.bind_owed_session(thread, role, session, ctx.now) {
        Ok(true) => arm_first_look(ctx, store, thread, role, ctx.now)
            .into_iter()
            .collect(),
        Ok(false) => Vec::new(),
        Err(e) => not_owed(thread, role, Some(session), &e),
    }
}

/// **Write the obligation `thread` owes `role` on `session` and arm its first look** — the write the
/// trigger funnel makes, and the mint of a slot nobody coordinates (cut 1a's row, kept exactly as it
/// was: a slot with no coordinator keeps today's behaviour). `coordinator` is the slot's
/// coordinating thread when it has one.
pub(crate) fn owe(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread: &str,
    role: &str,
    session: &str,
    triggered: bool,
    coordinator: Option<&str>,
) -> Vec<FailedConsequence> {
    // An engine identity (the synthesizer) answers for the engine; rule 1 excludes it anyway, and
    // keeping it out of the table keeps the work list to what can strand.
    if crate::orchestration::is_engine_identity(role) {
        return Vec::new();
    }
    if let Err(e) = store.record_owed_start(thread, role, session, ctx.now, triggered, coordinator)
    {
        return not_owed(thread, role, Some(session), &e);
    }
    arm_first_look(ctx, store, thread, role, ctx.now)
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
        Ok(true) => arm_first_look(ctx, store, thread, role, ctx.now)
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
            findings.extend(arm_first_look(ctx, store, &row.thread, &row.role, ctx.now));
        }
    }
    findings
}

pub(crate) fn outstanding_has(outstanding: &[String], role: &str) -> bool {
    let bare = role.rsplit_once('/').map_or(role, |(_, b)| b);
    outstanding.iter().any(|o| {
        crate::facade::same_identity(Some(o), role) || crate::facade::same_identity(Some(o), bare)
    })
}

/// What the sweep decided about one candidate.
pub(crate) enum Judgement {
    /// Not stranded, or not yet — nothing to record.
    Nothing,
    /// Stranded: record it with this observation, judged from this instant. For a COORDINATED slot
    /// (nxf 6j6v.558h) this is the occasion of a rung instead of a record.
    Record {
        observed: Observation,
        judged_from: String,
        detail: String,
    },
    /// A coordinated slot whose session holds a live process — the ladder's silence clock decides
    /// (rung A). Never answered for a slot nobody coordinates: rule 5 arms its recheck instead.
    Live,
    /// A coordinated slot whose owed session has a standing hold — the ladder's rung B decides.
    Held,
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
    // The one-time backfill (nxf 6j6v.kgfn), before anything reads the rows it writes.
    findings.extend(backfill_once(ctx, store));
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
            ConsequenceClass::StrandedNotJudged,
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
        // The ladder's record, for the same month (nxf 6j6v.558h), and the obligations kept for
        // queued triggers that are no longer queued.
        if let Err(e) = store.forget_recovery_before(&cutoff) {
            eprintln!("warning: old recovery records could not be dropped: {e}");
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
    let facts = PauseFacts::read(store)?;
    for row in live {
        let Some(q) = quorums.get(&row.thread) else {
            continue;
        };
        // **Who coordinates the slot is asked again at every look** (nxf 6j6v.kgfn): a hand-over
        // moves ownership with the `machine` register, and a device whose row named a coordinator
        // but which no longer executes the slot forgets its row rather than acting on it — the
        // device that now executes it has its own. A row that never named one (a slot nobody
        // coordinates, or a row cut 1a wrote) is judged as cut 1a judges it, and learns its
        // coordinator when it has one now. An abandoned row stays — it is the record of why —
        // until the slot stops being owed.
        let mut coordinated = None;
        if row.abandoned.is_none() {
            match assess(ctx, store, &row.thread, &row.role) {
                Ok(Assessment::Coordinated(c)) => {
                    if row.coordinator.as_deref() != Some(c.thread.as_str()) {
                        store.set_slot_coordinator(&row.thread, &row.role, &c.thread)?;
                    }
                    coordinated = Some(c);
                }
                // **Not knowing which machine this is decides nothing** (nxf 6j6v.ndbr): a row
                // that named a coordinator keeps it, and the slot stays its coordinator's.
                Ok(Assessment::MachineUnknown(c)) if row.coordinator.is_some() => {
                    coordinated = Some(CoordinatorRef {
                        thread: row.coordinator.clone().unwrap_or(c.thread),
                        kind: c.kind,
                    });
                }
                Ok(_) if row.coordinator.is_some() => {
                    // Known to run elsewhere now, or no longer coordinated for a reason this
                    // device can see — forgotten, and SAID: the sidecar posted no substitute for
                    // this slot, so the device that coordinates it now is the one to take it up.
                    let finding = FailedConsequence::stranded(
                        ConsequenceClass::StrandedNotJudged,
                        Some(&row.thread),
                        row.session.as_deref(),
                        format!(
                            "the slot of {} on thread {} is open and no longer coordinated on \
                             this device, so its row here is forgotten; it was coordinated by \
                             thread {}",
                            row.role,
                            row.thread,
                            row.coordinator.as_deref().unwrap_or("?")
                        ),
                    );
                    eprintln!("warning: {finding}");
                    findings.push(finding);
                    store.forget_owed_start(&row.thread, &row.role)?;
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    findings.push(FailedConsequence::stranded(
                        ConsequenceClass::StrandedNotJudged,
                        Some(&row.thread),
                        row.session.as_deref(),
                        format!(
                            "who coordinates the slot of {} on thread {} could not be read: {e}; \
                             the next tick looks again",
                            row.role, row.thread
                        ),
                    ));
                    continue;
                }
            }
        }
        // A failure on ONE obligation is a finding about that obligation; the walk goes on, so one
        // bad row cannot stop every later one from being judged on every tick.
        let judged = judge(ctx, store, &row, q, &facts, findings, coordinated.is_some()).and_then(
            |j| match (j, &coordinated) {
                (Judgement::Nothing, _) => Ok(Vec::new()),
                // **A coordinated slot is never merely recorded** (nxf 6j6v.558h): its coordinator
                // runs one rung of the ladder on what the look saw, and escalates only last.
                (j, Some(c)) => crate::ladder::climb(ctx, store, &row, c, j),
                (
                    Judgement::Record {
                        observed,
                        judged_from,
                        detail,
                    },
                    None,
                ) => record(ctx, store, &row, observed, judged_from, detail, None),
                (Judgement::Live | Judgement::Held, None) => Ok(Vec::new()),
            },
        );
        match judged {
            Ok(more) => findings.extend(more),
            Err(e) => findings.push(FailedConsequence::stranded(
                ConsequenceClass::StrandedNotJudged,
                Some(&row.thread),
                row.session.as_deref(),
                format!(
                    "the slot of {} on thread {} could not be judged: {e}; the next tick looks \
                     again",
                    row.role, row.thread
                ),
            )),
        }
    }
    Ok(())
}

// ---- the one-time backfill, and abandoned (nxf 6j6v.kgfn) ----------------------------------------

/// **Backfill once** — at the first sweep after the upgrade, every open coordinated slot that has no
/// row gets one, written with the marker in ONE transaction. Only a device whose worker can
/// coordinate backfills: under any other worker every slot reads uncoordinated, and burning the
/// marker on zero rows would leave the slots opened before this cut unowned for good.
///
/// A backfilled slot gets `trigger_clock = now`, `backfilled = 1`, and the session of its role
/// standing on the thread when there is one (`session_map`). It is ABANDONED — recorded with its
/// reason, never nudged, never escalated, one [`ConsequenceClass::SlotAbandoned`] finding — when its
/// operation's root no longer waits (delivered, withdrawn, accepted), when its newest evidence is
/// older than [`ABANDONED_AFTER_SECS`], or when its run has no commissioning message (clause 5:
/// `consolidated`). A replica writes none for a slot another site opened: clause 3 says so.
fn backfill_once(ctx: &Ctx, store: &mut ChatStore) -> Vec<FailedConsequence> {
    let not_backfilled = |e: &dyn std::fmt::Display| {
        let detail = format!(
            "the slots opened before this version could not be backfilled: {e}; the next tick \
             tries again"
        );
        eprintln!("warning: {detail}");
        vec![FailedConsequence::stranded(
            ConsequenceClass::StrandedNotJudged,
            None,
            None,
            detail,
        )]
    };
    if !worker_can_coordinate(ctx.worker) {
        return Vec::new();
    }
    match store.owed_backfill_done() {
        Ok(true) => return Vec::new(),
        Ok(false) => {}
        Err(e) => return not_backfilled(&e),
    }
    let rows = match backfill_rows(ctx, store) {
        Ok(rows) => rows,
        Err(e) => return not_backfilled(&e),
    };
    match store.backfill_owed_starts(&rows, ctx.now) {
        Ok(true) => {}
        Ok(false) => return Vec::new(),
        Err(e) => return not_backfilled(&e),
    }
    let mut findings = Vec::new();
    for row in &rows {
        match &row.abandoned {
            Some(reason) => {
                let finding = FailedConsequence::stranded(
                    ConsequenceClass::SlotAbandoned,
                    Some(&row.thread),
                    row.session.as_deref(),
                    format!(
                        "the slot of {} on thread {} was opened before this version and is \
                         abandoned: {} — nothing will take it up; `nxc withdraw --thread {}` \
                         takes it back",
                        row.role,
                        row.thread,
                        abandoned_words(reason),
                        row.thread
                    ),
                );
                eprintln!("warning: {finding}");
                findings.push(finding);
            }
            None => findings.extend(arm_first_look(ctx, store, &row.thread, &row.role, ctx.now)),
        }
    }
    findings
}

/// The rows the backfill writes: every open slot of a persona that [`assess`] coordinates (or finds
/// consolidated) and that has no row yet.
fn backfill_rows(ctx: &Ctx, store: &ChatStore) -> Result<Vec<Backfill>> {
    let mut rows = Vec::new();
    for q in store.thread_quorums_all(ctx.now)? {
        for role in &q.outstanding {
            if crate::orchestration::is_engine_identity(role)
                || store.owed_start(&q.thread_id, role)?.is_some()
            {
                continue;
            }
            let (coordinator, consolidated) = match assess(ctx, store, &q.thread_id, role)? {
                Assessment::Coordinated(c) => (c, false),
                Assessment::Consolidated(c) => (c, true),
                Assessment::Uncoordinated | Assessment::MachineUnknown(_) => continue,
            };
            let session = session_of_role_on(store, &q.thread_id, role)?;
            let abandoned = if consolidated {
                Some("consolidated".to_string())
            } else {
                abandoned_reason(ctx, store, &q.thread_id, session.as_deref())?
            };
            rows.push(Backfill {
                thread: q.thread_id.clone(),
                role: role.clone(),
                session,
                coordinator: coordinator.thread,
                clock: ctx.now.to_string(),
                abandoned,
            });
        }
    }
    Ok(rows)
}

/// The newest session of `role` standing on `thread` in this device's `session_map`.
fn session_of_role_on(store: &ChatStore, thread: &str, role: &str) -> Result<Option<String>> {
    let bare = role.rsplit_once('/').map_or(role, |(_, b)| b);
    Ok(store
        .sessions_in_thread(thread)?
        .into_iter()
        .rev()
        .find(|s| s.role == bare)
        .map(|s| s.internal_id))
}

/// Section A's abandoned rule for a backfilled slot that still has its commissioning message: the
/// root no longer waits, or the newest evidence is older than [`ABANDONED_AFTER_SECS`].
fn abandoned_reason(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
    session: Option<&str>,
) -> Result<Option<String>> {
    let root = store.thread_root(thread)?;
    if let Some(rq) = store.thread_quorum(&root, ctx.now)? {
        let waits = rq.outstanding.iter().any(|h| {
            let bare = h.rsplit_once('/').map_or(h.as_str(), |(_, b)| b);
            bare != crate::channel::DELIVERED_HANDLE && bare != crate::channel::WITHDRAWN_HANDLE
        });
        if !waits {
            let marker = |m: &str| {
                rq.expects
                    .iter()
                    .any(|h| h.rsplit_once('/').map_or(h.as_str(), |(_, b)| b) == m)
            };
            let why = if marker(crate::channel::WITHDRAWN_HANDLE) {
                "withdrawn"
            } else {
                "delivered"
            };
            return Ok(Some(why.to_string()));
        }
    }
    let mut evidence = store
        .acting_messages_in_thread(thread)?
        .into_iter()
        .find_map(|m| m.created);
    if let Some(s) = session {
        for at in [store.session_ended_at(s)?, store.last_transcript_at(s)?]
            .into_iter()
            .flatten()
        {
            if evidence.as_deref().is_none_or(|e| earlier(e, &at)) {
                evidence = Some(at);
            }
        }
    }
    let Some(evidence) = evidence else {
        return Ok(None);
    };
    let horizon = after(&evidence, ABANDONED_AFTER_SECS, "the abandoned horizon")?;
    Ok(earlier(&horizon, ctx.now).then(|| "aged".to_string()))
}

/// An abandoned slot's reason, in words — what `status` and the finding say.
pub(crate) fn abandoned_words(reason: &str) -> String {
    match reason {
        "delivered" => "its operation was already delivered".to_string(),
        "withdrawn" => "its operation was withdrawn".to_string(),
        "aged" => format!(
            "nothing has been heard from it for more than {} h",
            ABANDONED_AFTER_SECS / 3600
        ),
        "consolidated" => "its run no longer has the message that commissioned it".to_string(),
        other => other.to_string(),
    }
}

/// Rules 1-6 for one obligation. Drops the first look whenever the evidence no longer holds.
fn judge(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    q: &crate::store::ThreadQuorum,
    facts: &PauseFacts,
    findings: &mut Vec<FailedConsequence>,
    coordinated: bool,
) -> Result<Judgement> {
    let (t, h, s) = (
        row.thread.as_str(),
        row.role.as_str(),
        row.session.as_deref(),
    );
    let not_now = |store: &mut ChatStore| -> Result<Judgement> {
        store.drop_stranded_suspect(t, h)?;
        Ok(Judgement::Nothing)
    };
    // Rule 1: open, not held, the role a REAL one, and not a slot of a round the supervisor is
    // finished with.
    if q.held || crate::orchestration::is_engine_identity(h) {
        return not_now(store);
    }
    // Already recorded against THIS clock: the record and its escalation are the sweep's answer
    // until a later trigger moves the clock. Judging it again would re-suspect it and re-arm a look
    // on every tick — for ever, after an escalation that was given up.
    if store.stranded_recorded(t, h, &row.trigger_clock)? {
        return not_now(store);
    }
    if store
        .round_is_finished_with(t, ctx.origin, ctx.now)?
        .is_some()
    {
        return not_now(store);
    }
    // Rules 2-4: its answer is still coming — a standing hold, a start still to come (the queue, a
    // retry due), its own sub-round in flight, or a delivery held for it. The ONE predicate the
    // member-set read uses too, so "paused" cannot mean two things.
    match member_answer_is_still_coming(ctx.now, store, q, s, facts)? {
        // **A coordinated slot's hold is the ladder's rung B** (nxf 6j6v.558h): the resume takes
        // most holds up, and the ladder takes up the ones nothing will.
        Some(StillComing::OnHold { .. }) if coordinated => {
            store.drop_stranded_suspect(t, h)?;
            return Ok(Judgement::Held);
        }
        Some(_) => return not_now(store),
        None => {}
    }
    // **An abandoned slot is never nudged** (nxf 6j6v.kgfn): its record is the row's reason, and
    // `withdraw` is its exit.
    if row.abandoned.is_some() {
        return not_now(store);
    }
    let failed_start = facts.given_up(t, s);
    // Rule 5: no claim a session still holds. While it holds one, the look comes back one cadence
    // out — a session killed after a healthy look needs a further look.
    if s.is_some_and(|s| ctx.worker.session_is_running(s)) {
        store.drop_stranded_suspect(t, h)?;
        if coordinated {
            return Ok(Judgement::Live);
        }
        if let Ok(at) = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the next stranded look",
        ) {
            findings.extend(arm_look(ctx, store, t, h, &at));
        }
        return Ok(Judgement::Nothing);
    }
    // Rule 6: the grace on the judged instant.
    let grace = STRANDED_GRACE_SECS;
    // A start that failed and was given up: one look at its last attempt + grace.
    if let Some(f) = failed_start {
        let due = after(&f.last_failed_at, grace, "the stranded grace")?;
        if !reached(ctx.now, &due) {
            findings.extend(arm_look(ctx, store, t, h, &due));
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
    let ended = match s {
        Some(s) => store.session_ended_at(s)?,
        None => None,
    };
    if let Some(ended) = ended {
        let due = after(&ended, grace, "the stranded grace")?;
        if !reached(ctx.now, &due) {
            findings.extend(arm_look(ctx, store, t, h, &due));
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
    // A slot whose session was never minted has neither (nxf 6j6v.kgfn).
    let (bound, entries) = match s {
        Some(s) => (store.session_is_bound(s)?, store.transcript_entry_count(s)?),
        None => (false, 0),
    };
    let second_look = |store: &ChatStore, findings: &mut Vec<FailedConsequence>| {
        if let Ok(at) = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the second stranded look",
        ) {
            findings.extend(arm_look(ctx, store, t, h, &at));
        }
    };
    let fresh = Suspect {
        session: s.map(str::to_string),
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
    if suspect.session.as_deref() != s
        || suspect.bound != bound
        || suspect.transcript_entries != entries
    {
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
pub(crate) fn record(
    ctx: &Ctx,
    store: &mut ChatStore,
    row: &OwedStart,
    observed: Observation,
    judged_from: String,
    detail: String,
    summary: Option<String>,
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
    if let Some(summary) = &summary {
        store.set_stranded_summary(&rec, summary)?;
    }
    let step = step_name(store, &rec);
    let finding = FailedConsequence::stranded(
        ConsequenceClass::SessionStranded,
        Some(&rec.thread),
        rec.session.as_deref(),
        format!(
            "step {step} on thread {} is stranded: {} will not answer — observation {}: {}; \
             recorded at {}, escalated to the commissioner",
            rec.thread,
            session_words(&rec),
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
pub(crate) fn step_name(store: &ChatStore, rec: &StrandedRecord) -> String {
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
        .filter(|m| crate::facade::same_identity(Some(&m.sender), &rec.role))
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
    settle_escalation_with(ctx, store, rec, &mut post_as_the_member)
}

/// The escalation's one way out: posted AS the member, by the runtime, through the door the
/// sidecar's teardown uses (`surface::settle_if_unanswered`'s body). `Some(message id)` when it was
/// posted, `None` when nothing was owed any more.
fn post_as_the_member(
    ctx: &Ctx,
    store: &mut ChatStore,
    rec: &StrandedRecord,
    body: &str,
) -> Result<Option<String>> {
    let bare = rec
        .role
        .rsplit_once('/')
        .map_or(rec.role.as_str(), |(_, b)| b)
        .to_string();
    let as_member = Ctx {
        actor: &bare,
        session: rec.session.as_deref(),
        ..*ctx
    };
    let receipt =
        crate::surface::reply_as_the_runtime_for(&as_member, store, &rec.thread, body, true)?;
    Ok(receipt.message_id.filter(|_| receipt.posted))
}

/// How [`settle_escalation_with`] posts — the seam its tests inject a failing post through.
type Poster<'p> =
    dyn FnMut(&Ctx, &mut ChatStore, &StrandedRecord, &str) -> Result<Option<String>> + 'p;

fn settle_escalation_with(
    ctx: &Ctx,
    store: &mut ChatStore,
    rec: &StrandedRecord,
    post: &mut Poster<'_>,
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
    match post(ctx, store, rec, &body) {
        // Posted, and the receipt names the message: that IS the evidence (build item 15).
        Ok(Some(_message)) => {
            stamp(store, rec, Escalation::Posted, Some(ctx.now), &mut findings);
        }
        outcome => {
            // Not posted, or an error: the evidence decides — read again only now, since a post
            // that DID land is the one case a receipt can get wrong.
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
            rec.session.as_deref(),
            format!(
                "the escalation of stranded {} on thread {} could not be stamped: {e}; the next \
                 sweep reads the thread and stamps it then",
                session_words(rec),
                rec.thread
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
        rec.session.as_deref(),
        format!(
            "stranded {} on thread {} was not escalated: the slot was answered meanwhile",
            session_words(rec),
            rec.thread
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
    let who = session_words(rec);
    let detail = if give_up {
        format!(
            "the escalation of stranded {who} on thread {} could not be posted and is given up \
             after {attempts} attempt(s): {error} — withdraw the round with `nxc withdraw \
             --thread {}`",
            rec.thread, rec.thread
        )
    } else {
        format!(
            "the escalation of stranded {who} on thread {} could not be posted yet (attempt \
             {attempts}): {error}; a further look is armed",
            rec.thread
        )
    };
    eprintln!("warning: {detail}");
    findings.push(FailedConsequence::stranded(
        ConsequenceClass::StrandedNotEscalated,
        Some(&rec.thread),
        rec.session.as_deref(),
        detail,
    ));
    if !give_up {
        if let Ok(at) = after(
            ctx.now,
            crate::orchestration::SESSION_LIVENESS_RECHECK_SECS,
            "the next escalation attempt",
        ) {
            findings.extend(arm_look(ctx, store, &rec.thread, &rec.role, &at));
        }
    }
}

/// The session a record is about, in words — `session <id>`, or what a slot without one is.
pub(crate) fn session_words(rec: &StrandedRecord) -> String {
    match &rec.session {
        Some(s) => format!("session {s}"),
        None => format!("the slot of {} (no session was ever started)", rec.role),
    }
}

/// The escalation the commissioner reads — the coordinator's summary when the ladder escalated
/// (nxf 6j6v.558h), cut 1a's text for a slot nobody coordinates.
fn escalation_body(store: &ChatStore, rec: &StrandedRecord) -> String {
    if let Ok(Some(summary)) = store.stranded_summary(rec) {
        return summary;
    }
    let step = step_name(store, rec);
    let root = store
        .thread_root(&rec.thread)
        .unwrap_or_else(|_| rec.thread.clone());
    let session = match &rec.session {
        Some(s) => format!("session {s} is not running and nothing is coming from it"),
        None => "no session was ever started for it".to_string(),
    };
    format!(
        "STRANDED — step {step} on thread {thread} will not be answered: {session}.\n\n\
         Observation {n}: {detail}. Judged from {judged} with a grace of {grace} s; recorded at \
         {at}.\n\n\
         Nothing was restarted. A reply into thread {root} commissions the round again; `nxc \
         withdraw --thread {root}` takes it back.",
        thread = rec.thread,
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
            session: Some("m-s".into()),
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

    // ---- the obligation is the slot (nxf 6j6v.kgfn) ---------------------------------------------

    /// The row exists from the OPEN, before any session; the mint fills the session and moves the
    /// clock, the funnel marks it triggered, and what the open wrote about the coordinator stays.
    #[test]
    fn a_slot_is_owed_before_any_session_exists() {
        let mut s = ChatStore::open_in_memory(1);
        s.open_slot("m-t", "ws/verifier", "m-chan", T0).unwrap();
        let row = s.owed_start("m-t", "ws/verifier").unwrap().unwrap();
        assert_eq!(row.session, None);
        assert_eq!(row.trigger_clock, T0);
        assert!(!row.triggered);
        assert_eq!(row.coordinator.as_deref(), Some("m-chan"));
        assert!(!row.backfilled);
        assert_eq!(row.attempts, 0);
        assert_eq!(row.abandoned, None);

        s.bind_owed_session("m-t", "ws/verifier", "m-s1", T1)
            .unwrap();
        let row = s.owed_start("m-t", "ws/verifier").unwrap().unwrap();
        assert_eq!(row.session.as_deref(), Some("m-s1"));
        assert_eq!(row.trigger_clock, T1);
        assert!(!row.triggered, "a mint is not a trigger");

        s.record_owed_start("m-t", "ws/verifier", "m-s1", T2, true, Some("m-chan"))
            .unwrap();
        let row = s.owed_start("m-t", "ws/verifier").unwrap().unwrap();
        assert!(row.triggered);
        assert_eq!(row.trigger_clock, T2);
        assert_eq!(row.coordinator.as_deref(), Some("m-chan"));
    }

    /// A store from before this cut carries the old `owed_starts` (session NOT NULL, five columns)
    /// and the two record tables with a NOT NULL session. Opening it rebuilds all three in place and
    /// keeps every row.
    #[test]
    fn a_pre_slot_store_is_rebuilt_in_place_and_keeps_its_rows() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE owed_starts(
                 thread TEXT NOT NULL, role TEXT NOT NULL, session TEXT NOT NULL,
                 trigger_clock TEXT NOT NULL, triggered INTEGER NOT NULL,
                 PRIMARY KEY(thread, role));
             CREATE INDEX owed_starts_by_session ON owed_starts(session);
             CREATE TABLE stranded_suspects(
                 thread TEXT NOT NULL, role TEXT NOT NULL, session TEXT NOT NULL,
                 first_seen TEXT NOT NULL, bound INTEGER NOT NULL,
                 transcript_entries INTEGER NOT NULL, PRIMARY KEY(thread, role));
             CREATE TABLE stranded_sessions(
                 thread TEXT NOT NULL, role TEXT NOT NULL, session TEXT NOT NULL,
                 trigger_clock TEXT NOT NULL, judged_from TEXT NOT NULL,
                 recorded_at TEXT NOT NULL, observed INTEGER NOT NULL, detail TEXT NOT NULL,
                 escalation TEXT, escalation_attempts INTEGER NOT NULL DEFAULT 0,
                 escalated_at TEXT, escalation_error TEXT, superseded_at TEXT,
                 PRIMARY KEY(thread, role, trigger_clock));
             INSERT INTO owed_starts VALUES ('m-t', 'ws/verifier', 'm-s', '2026-10-18T10:00:00Z', 1);
             INSERT INTO stranded_suspects VALUES ('m-t', 'ws/verifier', 'm-s', 'x', 0, 0);
             INSERT INTO stranded_sessions(thread, role, session, trigger_clock, judged_from,
                 recorded_at, observed, detail, escalation)
                 VALUES ('m-t', 'ws/verifier', 'm-s', 'c', 'c', 'r', 2, 'd', 'posted');",
        )
        .unwrap();
        crate::schema::apply_chat_views(&conn);
        crate::schema::apply_chat_views(&conn); // and again: idempotent
        for table in ["owed_starts", "stranded_suspects", "stranded_sessions"] {
            let notnull: i64 = conn
                .query_row(
                    &format!("SELECT \"notnull\" FROM pragma_table_info('{table}') WHERE name = 'session'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(notnull, 0, "{table}.session is nullable after the rebuild");
            let rows: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(rows, 1, "{table} kept its row");
        }
        let (session, clock, triggered, attempts, backfilled): (String, String, i64, i64, i64) =
            conn.query_row(
                "SELECT session, trigger_clock, triggered, attempts, backfilled FROM owed_starts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            (
                session.as_str(),
                clock.as_str(),
                triggered,
                attempts,
                backfilled
            ),
            ("m-s", "2026-10-18T10:00:00Z", 1, 0, 0)
        );
        let escalation: String = conn
            .query_row("SELECT escalation FROM stranded_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(escalation, "posted");
    }

    fn backfill_row(thread: &str, abandoned: Option<&str>) -> Backfill {
        Backfill {
            thread: thread.into(),
            role: "ws/verifier".into(),
            session: None,
            coordinator: "m-chan".into(),
            clock: T0.into(),
            abandoned: abandoned.map(str::to_string),
        }
    }

    /// The marker and the rows are ONE transaction: written together, and a second backfill finds
    /// the marker and writes nothing.
    #[test]
    fn the_backfill_writes_its_rows_and_its_marker_together_and_only_once() {
        let mut s = ChatStore::open_in_memory(1);
        assert!(!s.owed_backfill_done().unwrap());
        assert!(s
            .backfill_owed_starts(
                &[
                    backfill_row("m-a", None),
                    backfill_row("m-b", Some("consolidated"))
                ],
                T0
            )
            .unwrap());
        assert!(s.owed_backfill_done().unwrap());
        let rows = s.owed_starts().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.backfilled && r.session.is_none()));
        assert_eq!(rows[1].abandoned.as_deref(), Some("consolidated"));
        assert_eq!(rows[1].abandoned_at.as_deref(), Some(T0));
        assert!(
            !s.backfill_owed_starts(&[backfill_row("m-c", None)], T1)
                .unwrap(),
            "the marker stands: nothing more is backfilled"
        );
        assert_eq!(s.owed_starts().unwrap().len(), 2);
    }

    /// **A crash between the rows and the marker leaves neither** — the marker write is made to fail
    /// inside the store itself, and the rows written before it in the same call are gone with it.
    #[test]
    fn a_backfill_whose_marker_cannot_be_written_leaves_no_rows_behind() {
        let mut s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch(
                "CREATE TRIGGER no_marker BEFORE INSERT ON owed_backfill_done
                 BEGIN SELECT RAISE(ABORT, 'the marker cannot be written (injected)'); END;",
            )
            .unwrap();
        assert!(s
            .backfill_owed_starts(&[backfill_row("m-a", None)], T0)
            .is_err());
        assert!(
            s.owed_starts().unwrap().is_empty(),
            "no row without its marker"
        );
        assert!(!s.owed_backfill_done().unwrap());
    }

    #[test]
    fn a_look_is_keyed_by_its_slot() {
        assert_eq!(look_key("m-t", "ws/verifier"), "m-t-verifier");
        assert_eq!(look_key("m-t", "verifier"), "m-t-verifier");
    }

    #[test]
    fn a_new_session_for_the_same_obligation_replaces_the_row_whole() {
        let mut s = ChatStore::open_in_memory(1);
        s.record_owed_start("m-t", "ws/verifier", "m-s1", T0, false, Some("m-chan"))
            .unwrap();
        s.record_owed_start("m-t", "ws/verifier", "m-s2", T1, true, Some("m-chan"))
            .unwrap();
        let row = s.owed_start("m-t", "ws/verifier").unwrap().unwrap();
        assert_eq!(row.session.as_deref(), Some("m-s2"));
        assert_eq!(row.trigger_clock, T1);
        assert!(row.triggered);
        assert_eq!(s.owed_starts().unwrap().len(), 1);
    }

    #[test]
    fn a_touch_moves_only_the_obligation_the_session_owes() {
        let mut s = ChatStore::open_in_memory(1);
        s.record_owed_start("m-r1", "ws/pm", "m-p1", T0, true, Some("m-chan"))
            .unwrap();
        s.record_owed_start("m-r2", "ws/pm", "m-p2", T0, true, Some("m-chan"))
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
        s.record_owed_start("m-t", "ws/verifier", "m-s", T0, true, Some("m-chan"))
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

        s.record_owed_start("m-t", "ws/verifier", "m-s", T2, true, Some("m-chan"))
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
            session: Some("m-s".into()),
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
        s.record_owed_start("m-t", "ws/verifier", "m-s", T0, true, Some("m-chan"))
            .unwrap();
        s.forget_owed_start("m-t", "ws/verifier").unwrap();
        assert_eq!(s.stranded_suspect("m-t", "ws/verifier").unwrap(), None);
    }

    // ---- the escalation's bounded retry, through an injected post (AC 6 c, d) -------------------

    static DRY: crate::worker::DryWorker = crate::worker::DryWorker { log: None };

    /// A timer that counts the looks it was asked to arm.
    #[derive(Default)]
    struct Looks(std::sync::Mutex<Vec<String>>);
    impl crate::timer::Timer for Looks {
        fn schedule(
            &self,
            thread: &str,
            _deadline: &str,
            _command: &str,
        ) -> Result<crate::timer::TimerHandle> {
            Ok(crate::timer::TimerHandle(thread.to_string()))
        }
        fn schedule_stranded_look(
            &self,
            slot: &str,
            _thread: &str,
            deadline: &str,
        ) -> Result<crate::timer::TimerHandle> {
            self.0.lock().unwrap().push(format!("{slot}@{deadline}"));
            Ok(crate::timer::TimerHandle(slot.to_string()))
        }
        fn cancel(&self, _handle: &crate::timer::TimerHandle) -> Result<()> {
            Ok(())
        }
    }

    struct Scene {
        _tmp: tempfile::TempDir,
        store: ChatStore,
        defs: crate::definitions::Definitions,
        origin: String,
        db: String,
        looks: Looks,
        rec: StrandedRecord,
    }

    impl Scene {
        fn ctx<'a>(&'a self, now: &'a str, actor: &'a str, session: Option<&'a str>) -> Ctx<'a> {
            Ctx {
                now,
                origin: &self.origin,
                actor,
                session,
                hop: 0,
                defs: &self.defs,
                // The crate's own dry worker: it accepts every start and runs nothing, which is all
                // these tests need — they drive the escalation, not the liveness question.
                worker: &DRY,
                timer: &self.looks,
                namer: &crate::naming::DryNamer,
                db_path: &self.db,
                project_claude_md: None,
                module_primes: None,
                machines: None,
                peers: None,
            }
        }

        fn escalations(&self) -> usize {
            self.store
                .acting_messages_in_thread(&self.rec.thread)
                .unwrap()
                .iter()
                .filter(|m| m.kind == "escalation")
                .count()
        }

        fn row(&self) -> StrandedRecord {
            self.store.stranded_records().unwrap().remove(0)
        }
    }

    /// A human commissions `verifier` directly; its session never starts, and the record is written.
    fn scene() -> Scene {
        use crate::workspace::ChatWorkspaceExt;
        let tmp = tempfile::TempDir::new().unwrap();
        crate::workspace::setup(tmp.path(), &crate::workspace::chat_config()).unwrap();
        let ws = crate::workspace::Workspace::resolve(None, tmp.path()).unwrap();
        let origin = crate::workspace::origin_of(&ws).to_string();
        let db = ws.db_path_str().unwrap();
        let store = ws.open_chat_store().unwrap();
        let role = |h: &str| -> crate::role::RoleDecl {
            serde_yaml::from_str(&format!("handle: {h}\nsystem_prompt: You are {h}.\n")).unwrap()
        };
        let defs = crate::definitions::Definitions::new(vec![role("pm"), role("verifier")], vec![])
            .unwrap();
        let mut scene = Scene {
            _tmp: tmp,
            store,
            defs,
            origin,
            db,
            looks: Looks::default(),
            rec: record(T0),
        };
        let sent = {
            let mut store = std::mem::replace(&mut scene.store, ChatStore::open_in_memory(1));
            let ctx = scene.ctx(T0, "pm", None);
            let sent = crate::surface::send_to(
                &ctx,
                &mut store,
                crate::surface::SendToRequest {
                    machine: None,
                    to: "verifier",
                    body: "verify",
                    refs: crate::surface::SendToRefs::ExplicitlyNone,
                },
            )
            .unwrap();
            scene.store = store;
            sent
        };
        scene.rec = StrandedRecord {
            thread: sent.thread_id.clone(),
            role: format!("{}/verifier", scene.origin),
            session: sent.session.clone(),
            ..record(T0)
        };
        assert!(scene.store.record_stranded(&scene.rec).unwrap());
        scene
    }

    fn settle(scene: &mut Scene, now: &str, post: &mut Poster<'_>) -> Vec<FailedConsequence> {
        let mut store = std::mem::replace(&mut scene.store, ChatStore::open_in_memory(1));
        let rec = scene.rec.clone();
        let findings = {
            let ctx = scene.ctx(now, "service", None);
            settle_escalation_with(&ctx, &mut store, &rec, post)
        };
        scene.store = store;
        findings
    }

    fn io(_: &Ctx, _: &mut ChatStore, _: &StrandedRecord, _: &str) -> Result<Option<String>> {
        Err(crate::error::NxfError::io("database is locked (injected)"))
    }

    /// **AC 6 (d), build item 5**: a transient error is retried on at most three sweeps, a further
    /// look is armed after each failed attempt, and then the escalation is given up — and nothing is
    /// armed after that.
    #[test]
    fn a_transient_post_failure_is_retried_three_times_with_a_look_each_then_given_up() {
        let mut scene = scene();
        let looks_before = scene.looks.0.lock().unwrap().len();
        for (n, now) in [(1, "2026-10-18T10:03:00Z"), (2, "2026-10-18T10:04:00Z")] {
            let findings = settle(&mut scene, now, &mut io);
            assert!(findings
                .iter()
                .any(|f| f.class == ConsequenceClass::StrandedNotEscalated));
            let row = scene.row();
            assert_eq!(row.escalation, None);
            assert_eq!(row.escalation_attempts, n);
            assert_eq!(
                scene.looks.0.lock().unwrap().len(),
                looks_before + n as usize,
                "a look after each failed attempt"
            );
        }
        let findings = settle(&mut scene, "2026-10-18T10:05:00Z", &mut io);
        let last = findings
            .iter()
            .find(|f| f.class == ConsequenceClass::StrandedNotEscalated)
            .expect("the final finding");
        assert!(last.detail.contains("given up"), "{}", last.detail);
        assert_eq!(scene.row().escalation, Some(Escalation::GivenUp));
        assert_eq!(
            scene.looks.0.lock().unwrap().len(),
            looks_before + 2,
            "nothing armed after giving up"
        );
        assert_eq!(scene.escalations(), 0);
    }

    /// **AC 6 (c)**: the post failed, and then the member answered for real. The next sweep reads
    /// the slot as answered and does NOT post: `already_answered`, with its finding, and zero
    /// escalations on the thread.
    #[test]
    fn a_failed_post_followed_by_a_real_answer_is_already_answered_and_posts_nothing() {
        let mut scene = scene();
        settle(&mut scene, "2026-10-18T10:03:00Z", &mut io);
        assert_eq!(scene.row().escalation, None);
        {
            let rec = scene.rec.clone();
            let mut store = std::mem::replace(&mut scene.store, ChatStore::open_in_memory(1));
            let ctx = scene.ctx("2026-10-18T10:03:30Z", "verifier", rec.session.as_deref());
            crate::surface::reply_in_thread(
                &ctx,
                &mut store,
                crate::surface::ReplyThreadRequest {
                    machine: None,
                    thread: &rec.thread,
                    body: "verified after all",
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .unwrap();
            scene.store = store;
        }
        let mut must_not_post = |_: &Ctx, _: &mut ChatStore, _: &StrandedRecord, _: &str| {
            panic!("a slot that was answered is not escalated")
        };
        let findings = settle(&mut scene, "2026-10-18T10:04:00Z", &mut must_not_post);
        assert!(findings
            .iter()
            .any(|f| f.class == ConsequenceClass::StrandedEscalationSkipped));
        assert_eq!(scene.row().escalation, Some(Escalation::AlreadyAnswered));
        assert_eq!(scene.escalations(), 0);
    }

    /// **AC 6 (d)'s other half**: a PERMANENT refusal gives up on the first attempt.
    #[test]
    fn a_permanent_refusal_gives_up_at_once() {
        let mut scene = scene();
        let mut refused = |_: &Ctx, _: &mut ChatStore, _: &StrandedRecord, _: &str| {
            Err(crate::error::NxfError::forbidden("refused (injected)"))
        };
        settle(&mut scene, "2026-10-18T10:03:00Z", &mut refused);
        let row = scene.row();
        assert_eq!(row.escalation, Some(Escalation::GivenUp));
        assert_eq!(row.escalation_attempts, 1);
    }

    /// **AC 1 (h)**: the synthesizer answers for the engine, so no obligation is recorded for it —
    /// rule 1 would exclude it anyway, and keeping it out of the table keeps the work list to what
    /// can strand.
    #[test]
    fn an_engine_identity_is_never_recorded_as_owing() {
        let mut scene = scene();
        let mut store = std::mem::replace(&mut scene.store, ChatStore::open_in_memory(1));
        let synth = format!("{}/{}", scene.origin, crate::channel::SYNTHESIS_HANDLE);
        let findings = {
            let ctx = scene.ctx(T0, "service", None);
            owe(
                &ctx,
                &mut store,
                "m-board",
                &synth,
                "m-synth-session",
                true,
                None,
            )
        };
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!(store.owed_start("m-board", &synth).unwrap(), None);
    }
    /// **IR #1**: supersession and retention compare instants, not text. A clock with fractional
    /// seconds or another offset sorts as text the wrong way round.
    #[test]
    fn supersession_and_retention_compare_instants_not_text() {
        let mut s = ChatStore::open_in_memory(1);
        // As text "…10:00:00.5Z" < "…10:00:00Z"; as time it is later.
        let older = "2026-10-18T10:00:00Z";
        let later_fractional = "2026-10-18T10:00:00.5Z";
        s.record_stranded(&record(older)).unwrap();
        s.record_owed_start(
            "m-t",
            "ws/verifier",
            "m-s",
            later_fractional,
            true,
            Some("m-chan"),
        )
        .unwrap();
        assert!(
            s.current_stranded_by_threads(&["m-t"]).unwrap().is_empty(),
            "a later fractional clock supersedes"
        );
        let mut s = ChatStore::open_in_memory(1);
        // An offset: 12:00+02:00 is 10:00Z — not later than 10:00:00Z.
        s.record_stranded(&record(older)).unwrap();
        s.record_owed_start(
            "m-t",
            "ws/verifier",
            "m-s",
            "2026-10-18T12:00:00+02:00",
            true,
            Some("m-chan"),
        )
        .unwrap();
        assert_eq!(s.current_stranded_by_threads(&["m-t"]).unwrap().len(), 1);
        // Retention: recorded at 10:03Z; a cutoff of 11:00+02:00 (= 09:00Z) keeps it.
        assert_eq!(
            s.forget_stranded_before("2026-10-18T11:00:00+02:00")
                .unwrap(),
            0
        );
        assert_eq!(
            s.forget_stranded_before("2026-10-18T13:00:00+02:00")
                .unwrap(),
            1
        );
    }

    #[test]
    fn an_observation_this_build_does_not_know_says_which_number_it_was() {
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute(
                "INSERT INTO stranded_sessions(thread, role, session, trigger_clock, judged_from,
                     recorded_at, observed, detail)
                 VALUES ('m-t', 'ws/verifier', 'm-s', 'c', 'c', '2026-10-18T10:03:00Z', 9, 'x')",
                [],
            )
            .unwrap();
        let rec = s.stranded_records().unwrap().remove(0);
        assert_eq!(rec.observed, Observation::NoLiveProcess);
        assert!(rec.detail.contains("observation 9"), "{}", rec.detail);
    }
}

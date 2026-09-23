//! THE ANTI-N+1 PROPERTY, PROVED RATHER THAN ASSERTED (nexus-flow-6j6v.yr59).
//!
//! `Engine::thread_quorums` was the bulk quorum read: "the same question as `threads`, but for
//! THIS explicit set of thread ids", and its whole value was that the set costs ONE query rather
//! than one per thread (`ChatStore::thread_quorums`: one prepared statement, one `IN (?,?,…)`
//! filter, one `query_map`). yr59 removes it as a VERB — the item's words: *"Die
//! Anti-N+1-Eigenschaft muss bleiben, der zweite Name nicht"* — and folds the explicit set into its
//! neighbour as a parameter: [`StatusScope::Threads`].
//!
//! # Why this file counts statements instead of trusting the shape
//!
//! Nothing about an N+1 regression is visible from the outside. The values come back identical, the
//! types are the same, every existing test stays green, and the only symptom is a read that gets
//! slower in proportion to how much work a workspace has going on — which is exactly the workspace
//! where somebody would notice last. `facade::threads`' own doc has said "no N+1" in prose since
//! T4; prose is not a gate.
//!
//! So the gate is a MEASUREMENT. SQLite calls an authorizer callback once per statement while that
//! statement is being PREPARED, so `AuthAction::Select` counts the SELECTs one read compiles —
//! independently of how many rows come back and of how many `?` placeholders are bound. The claim
//! "N threads are read in ONE query" is therefore checkable as: **the count does not move when N
//! does.** A read that prepared a statement per thread would show a count that grows with N; this
//! one shows the same number for two threads and for forty.
//!
//! The counting runs against the compute layer (`facade::status`) because that is where the
//! statements are prepared and the connection is reachable; the seam half is asserted beside it —
//! `Engine::status` over the same workspace answers byte for byte, so the property the count
//! proves is the property an app gets.

mod common;

use std::panic::RefUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use nexus_chat::engine::Engine;
use nexus_chat::facade::{self, AskRequest, StatusScope};
use nexus_chat::model::{
    Disposition, MessageEnvelope, MessageKind, Priority, Refs, CHANNEL_KIND_GROUP,
};
use nexus_chat::orchestration::SessionState;
use nexus_chat::store::ChatStore;
use nexus_chat::worker::{DryWorker, TriggerOutcome, TriggerRequest, TriggerResult, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use tempfile::TempDir;

const NOW: &str = "2026-08-21T09:00:00Z";
const ORIGIN: &str = "local";
const CHANNEL: &str = "c-boards";
const OPENER: &str = "local/a";
const EXPECTED: &str = "local/b";

/// How many boards the fixture opens. Chosen so neither sample below is DEGENERATE: `facade`'s
/// `quorums_for` switches to the whole-workspace read when the requested set IS the workspace
/// (`ids.len() == total`) or exceeds SQLite's bind ceiling, and either branch would compare two
/// different statements instead of the same one over different N.
const BOARDS: usize = 60;

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// `BOARDS` quorum boards in one channel, opened through the real `ask` write. Returns their
/// thread ids in the order they were minted.
/// The two return addresses every board's answer carries — see `seed`. Shaped like real minted
/// ids, because `facade::session_states_for` refuses to ask its worker about anything else.
const SESSIONS: &[&str] = &[
    "m-01M1SESSIONAAAAAAAAAAAAAAA",
    "m-01M1SESSIONBBBBBBBBBBBBBBB",
];

fn seed(dir: &Path) -> Vec<String> {
    let mut s = Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap();
    s.set_wall_clock(NOW);
    s.set_channel_field(CHANNEL, "kind", CHANNEL_KIND_GROUP, OPENER);
    s.set_channel_field(CHANNEL, "name", "boards", OPENER);
    s.add_member(CHANNEL, OPENER, OPENER);
    s.add_member(CHANNEL, EXPECTED, OPENER);
    (0..BOARDS)
        .map(|i| {
            facade::ask(
                &mut s,
                AskRequest {
                    now: NOW,
                    origin: ORIGIN,
                    actor: "a",
                    channel: CHANNEL,
                    body: &format!("board {i}"),
                    expect: &[EXPECTED.to_string()],
                    deadline: None,
                    kind: MessageKind::Question,
                    priority: Priority::Normal,
                    refs: Refs::default(),
                },
            )
            .expect("open a board")
            .thread_id
        })
        .collect::<Vec<String>>()
        .into_iter()
        .enumerate()
        .map(|(i, thread)| {
            // **Every board is ANSWERED, and the answer carries a return address** (review of PR
            // #444, Test Quality #1). Without this the boards have no assignee session at all, so
            // `facade::session_states_for` takes its `distinct.is_empty()` exit and the whole read
            // this file exists to measure never reaches the statement — or the worker — that nxf
            // 6j6v.qmy6 added. Two ids for however many boards, because the property under test is
            // that the cost follows the number of DISTINCT sessions and not the number of threads.
            s.set_wall_clock(NOW);
            s.post_message(&MessageEnvelope {
                origin: ORIGIN.into(),
                channel_id: CHANNEL.into(),
                sender: EXPECTED.into(),
                kind: MessageKind::Report,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                thread_id: Some(thread.clone()),
                refs: Refs {
                    session_id: Some(SESSIONS[i % SESSIONS.len()].to_string()),
                    ..Refs::default()
                },
                body: "answered".into(),
            });
            thread
        })
        .collect()
}

/// Run `read` with SQLite counting the SELECT statements it PREPARES, and hand back both.
///
/// The authorizer is removed again afterwards: it is installed on the connection, not on the call,
/// and a counter left behind would silently keep counting somebody else's read.
fn selects_prepared<T>(store: &ChatStore, read: impl FnOnce(&ChatStore) -> T) -> (T, usize) {
    let counter = Arc::new(AtomicUsize::new(0));
    let ticker = Arc::clone(&counter);
    store
        .connection()
        .authorizer(Some(move |ctx: AuthContext<'_>| {
            if matches!(ctx.action, AuthAction::Select) {
                ticker.fetch_add(1, Ordering::SeqCst);
            }
            Authorization::Allow
        }));
    let out = read(store);
    store
        .connection()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    (out, counter.load(Ordering::SeqCst))
}

/// `selects_prepared`'s bound demands this; stated once here rather than at each call.
const _: fn() = || {
    fn assert_unwind_safe<T: RefUnwindSafe + Send + 'static>() {}
    assert_unwind_safe::<Arc<AtomicUsize>>();
};

#[test]
fn a_set_of_threads_costs_the_same_number_of_queries_however_many_are_in_it() {
    let tmp = workspace();
    let ids = seed(tmp.path());
    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();

    let two: Vec<&str> = ids[..2].iter().map(String::as_str).collect();
    let forty: Vec<&str> = ids[..40].iter().map(String::as_str).collect();

    let (small, small_queries) = selects_prepared(&store, |s| {
        facade::status(s, &DryWorker { log: None }, NOW, StatusScope::Threads(&two))
            .expect("status reads two threads")
    });
    let (large, large_queries) = selects_prepared(&store, |s| {
        facade::status(
            s,
            &DryWorker { log: None },
            NOW,
            StatusScope::Threads(&forty),
        )
        .expect("status reads forty threads")
    });

    // The reads really did differ in size — otherwise "the cost did not move" would be a statement
    // about two identical calls.
    //
    // **The quorum check below reads `expects` + a discharged `outstanding` rather than an
    // outstanding `EXPECTED`**, because since the review of PR #444 `seed` ANSWERS every board: an
    // unanswered board leaves no return address, and without one `facade::session_states_for` takes
    // its empty exit and the statement this test counts is never prepared. The claim is unchanged —
    // the quorum was really derived for every board — and it now carries the session with it, which
    // is what makes this fixture cover the read it is measuring.
    assert_eq!(small.operations.len(), 2);
    assert_eq!(large.operations.len(), 40);
    assert!(
        large.operations.iter().all(|op| op.threads.len() == 1
            && op.threads[0].expects == [EXPECTED]
            && op.threads[0].outstanding.is_empty()
            && op.threads[0].session.is_some()),
        "every board really carries its derived quorum: {large:?}"
    );

    assert_eq!(
        small_queries, large_queries,
        "\n\nTHE BULK QUORUM READ HAS BECOME AN N+1 (nexus-flow-6j6v.yr59).\n\n  \
         Two threads cost {small_queries} prepared SELECT statements and forty cost \
         {large_queries}.\n  \
         The whole value of reading an explicit SET of threads is that the set is ONE query:\n  \
         `ChatStore::thread_quorums` binds one placeholder per id into a single statement, and\n  \
         `facade::status` calls it once for the whole selection. A count that follows N means\n  \
         somebody put a per-thread read back on this path.\n"
    );
    // …and constant at a SMALL number, so "constant" cannot quietly mean "constant and enormous":
    // the selection, the quorums, the assignee sessions, the orphan discriminator and the
    // working-tree visibility, each once, plus the sub-selects inside them.
    //
    // The bound moved from 40 to 64 with nxf 6j6v.pzkb, and the reason is a fixed cost, not a
    // per-thread one: every decision read now asks whether an agent action may follow the rows it
    // counts (`store::acts` — a sub-select over the `acting_ops` view, which carries one of its
    // own), spliced once per STATEMENT. It measured 49 for two threads and for forty alike; the
    // equality above is what would catch it following N.
    assert!(
        large_queries < 64,
        "forty threads must not cost anything like forty statements: {large_queries}"
    );
}

/// A worker that COUNTS the process questions put to it — the other half of this file's bound
/// (review of PR #444, Test Quality #1).
///
/// `selects_prepared` next door counts SQL statements, and until nxf 6j6v.qmy6 that was the whole
/// cost of a status read. It is not any more: the read now leaves the database and asks an
/// operating system whether a session's process is alive. A count that stayed flat in statements
/// while growing in `kill(2)` calls would satisfy the old measurement and miss the new N+1
/// entirely.
#[derive(Default)]
struct CountingWorker {
    asked: AtomicUsize,
}

impl Worker for CountingWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, _internal_session: &str) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        false
    }

    fn answers_liveness(&self) -> bool {
        true
    }
}

#[test]
fn the_process_question_is_asked_once_per_session_however_many_threads_name_it() {
    // The claim nxf 6j6v.qmy6 makes in prose and this makes measurable: the worker side of the
    // read follows the number of DISTINCT sessions, never the number of threads. `seed` answers
    // every board with one of two return addresses, so forty boards name the same two sessions as
    // two boards do.
    let tmp = workspace();
    let ids = seed(tmp.path());
    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();

    let two: Vec<&str> = ids[..2].iter().map(String::as_str).collect();
    let forty: Vec<&str> = ids[..40].iter().map(String::as_str).collect();

    let small_worker = CountingWorker::default();
    let (_small, small_queries) = selects_prepared(&store, |s| {
        facade::status(s, &small_worker, NOW, StatusScope::Threads(&two))
            .expect("status reads two threads")
    });
    let large_worker = CountingWorker::default();
    let (large, large_queries) = selects_prepared(&store, |s| {
        facade::status(s, &large_worker, NOW, StatusScope::Threads(&forty))
            .expect("status reads forty threads")
    });

    // The premise: the reads really did carry sessions, or "flat" would be a statement about two
    // calls that both did nothing. This is the exact hole the review of PR #444 found — the old
    // fixture gave no board an assignee session at all, so `session_states_for` took its empty
    // exit and neither the statement nor the worker call below ever happened.
    assert!(
        large
            .operations
            .iter()
            .flat_map(|o| &o.threads)
            .filter(|t| t.session.is_some())
            .count()
            >= 40,
        "every board carries the return address its answer left: {large:?}"
    );
    assert_eq!(
        large.operations[0].threads[0].session_state,
        Some(SessionState::Unknown),
        "…and the state travels with it: {large:?}"
    );

    let asked_small = small_worker.asked.load(Ordering::SeqCst);
    let asked_large = large_worker.asked.load(Ordering::SeqCst);
    assert_eq!(
        asked_small, asked_large,
        "\n\nTHE LIVENESS PROBE HAS BECOME AN N+1 (nxf 6j6v.qmy6).\n\n  \
         Two threads cost {asked_small} process questions and forty cost {asked_large}.\n  \
         `facade::session_states_for` asks once per DISTINCT un-ended session, so a fixture whose\n  \
         forty boards share two return addresses must ask exactly as often as one whose two do.\n  \
         A count that follows the thread count means somebody moved the probe inside the loop.\n"
    );
    assert_eq!(
        asked_large,
        SESSIONS.len(),
        "…and it is the number of distinct sessions, not merely a number that happens to match: \
         {asked_large}"
    );
    assert_eq!(
        small_queries, large_queries,
        "the SQL side stays flat with the sessions on the boards too: {small_queries} vs \
         {large_queries}"
    );
}

#[test]
fn the_seam_serves_the_read_the_query_count_was_measured_on() {
    // The measurement above runs on `facade::status` because that is where the statements are
    // prepared and where a connection can be reached at all. This is the half that makes it a
    // statement about the SEAM: the handle answers the identical value over the same workspace, so
    // an app calling `Engine::status` with an explicit set gets the read that was measured — not a
    // second implementation of it.
    let tmp = workspace();
    let ids = seed(tmp.path());
    let picked: Vec<&str> = ids[..7].iter().map(String::as_str).collect();

    let direct = {
        let store = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        facade::status(
            &store,
            &DryWorker { log: None },
            NOW,
            StatusScope::Threads(&picked),
        )
        .expect("the compute layer reads")
    };
    // The SAME worker on both sides. Since nxf 6j6v.qmy6 the report carries what became of each
    // thread's session, so the two halves of this differential are only comparable when the same
    // runtime answered the process question — `Engine::open` would resolve whatever this machine
    // has installed, and the direct call above passes a dry one.
    let engine = Engine::open_with(
        None,
        tmp.path(),
        nexus_chat::engine::EngineConfig {
            worker: nexus_chat::worker::WorkerConfig::Dry { log: None },
            ..nexus_chat::engine::EngineConfig::default()
        },
    )
    .expect("open engine");
    let through_the_seam = engine
        .status(NOW, StatusScope::Threads(&picked))
        .expect("the seam reads");

    assert_eq!(through_the_seam, direct);
    assert_eq!(through_the_seam.operations.len(), 7);
}

#[test]
fn an_explicit_set_naming_a_thread_that_does_not_exist_is_not_found_rather_than_a_short_answer() {
    // The behaviour `StatusScope::Thread` had for one id, kept for the set: a caller that asks
    // about something that is not there is told so. Silently returning the ones that DID resolve
    // would make a typo look like a finished operation.
    let tmp = workspace();
    let ids = seed(tmp.path());
    let store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let asked = [ids[0].as_str(), "m-nothing-here"];
    let err = facade::status(
        &store,
        &DryWorker { log: None },
        NOW,
        StatusScope::Threads(&asked),
    )
    .unwrap_err();
    assert_eq!(err.kind, nexus_chat::error::ErrorKind::NotFound);
    assert!(err.to_string().contains("m-nothing-here"), "{err}");
}

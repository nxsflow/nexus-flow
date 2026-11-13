//! The crash test over the REAL chat store (§12.4, round 3 TQ #5), because the layering gate keeps the real port
//! out of the crash box: post, die before the `done`, restart, find; open the same thread id twice and read one
//! thread.
//!
//! The port is `ports_v1::V1Ports` (6j6v.b8tj), the one the host is given: its key rides in `refs.run_key`, so
//! what this proves about a post that survives a killed process, and is found by that key, is what the host's
//! find-or-post relies on (§8.3).

use std::path::Path;
use std::process::{Command, Stdio};

use nexus_chat::definitions::Definitions;
use nexus_chat::ports_v1::V1Ports;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nexus_run_contracts::MessageId;
use nexus_run_host::ports::{self, MessagePort};

const DB_VAR: &str = "NXR_PORT_CRASH_DB";
const THREAD_VAR: &str = "NXR_PORT_CRASH_THREAD";
const KEY: &str = "result/r-01JAAAAAAAAAAAAAAAAAAAAAAA";
const CHANNEL: &str = "coding-v2";
const OPENER: &str = "__channel__";

fn port(db: &str) -> V1Ports {
    V1Ports::with_definitions(db, "local", Definitions::default())
}

/// A second reader of the same file: what a human's `nxc threads show` would see.
fn chat(db: &str) -> ChatStore {
    Workspace::resolve(Some(db), Path::new("."))
        .unwrap()
        .open_chat_store()
        .unwrap()
}

fn workspace() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path(), &chat_config()).expect("seed chat workspace");
    let db = dir.path().join(".nxs").join("db.sqlite");
    let db = db.to_str().unwrap().to_owned();
    (dir, db)
}

/// The host's find-or-post, as `Deliver` runs it (§8.3).
fn find_or_post(port: &V1Ports, thread: &str) -> MessageId {
    match port.find(thread, KEY).expect("find answers") {
        Some(found) => found,
        None => port
            .post(thread, OPENER, "the result", &ports::Refs::default(), KEY)
            .expect("the post goes through"),
    }
}

/// The child of the test below: it posts and dies before it can record `done`, with no unwinding and no flush.
fn die_after_the_post(db: &str, thread: &str) -> ! {
    let port = port(db);
    port.open_thread(thread, None, CHANNEL, OPENER).unwrap();
    find_or_post(&port, thread);
    std::process::abort();
}

#[test]
fn a_post_survives_a_kill_before_done_and_is_found_again_never_posted_twice() {
    if let (Ok(db), Ok(thread)) = (std::env::var(DB_VAR), std::env::var(THREAD_VAR)) {
        die_after_the_post(&db, &thread);
    }
    let (_dir, db) = workspace();
    let thread = chat(&db).mint_thread_id();

    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_post_survives_a_kill_before_done_and_is_found_again_never_posted_twice",
            "--nocapture",
        ])
        .env(DB_VAR, &db)
        .env(THREAD_VAR, &thread)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !status.success() && status.code().is_none(),
        "the child aborted: {status:?}"
    );

    // The restart: `pending` is reconciled by `find` first.
    let port = port(&db);
    let found = port.find(&thread, KEY).unwrap();
    let found = found.expect("the post survived the kill, and its key with it");
    let posts = || chat(&db).messages_in_thread(&thread).unwrap().len();
    assert_eq!(posts(), 1);
    assert_eq!(
        find_or_post(&port, &thread),
        found,
        "found, not posted again"
    );
    assert_eq!(posts(), 1);
    let read = port.read(&found).unwrap().expect("the message reads by id");
    assert_eq!(read.body, "the result");
    assert_eq!(read.sender, OPENER);
    assert_eq!(port.find(&thread, "another/key").unwrap(), None);
}

/// A thread is found by its id: a second open of the same id, across a restart as well, is the same thread.
#[test]
fn opening_the_same_thread_id_twice_holds_one_thread() {
    let (_dir, db) = workspace();
    let thread = chat(&db).mint_thread_id();
    let first = port(&db);
    first.open_thread(&thread, None, CHANNEL, OPENER).unwrap();
    first.open_thread(&thread, None, CHANNEL, OPENER).unwrap();
    drop(first);

    port(&db)
        .open_thread(&thread, None, CHANNEL, OPENER)
        .unwrap();
    let chat = chat(&db);
    assert_eq!(
        chat.threads_opened_by(OPENER).unwrap(),
        std::slice::from_ref(&thread),
        "one thread, opened three times"
    );
    assert_eq!(chat.thread_channel(&thread).as_deref(), Some(CHANNEL));
}

/// What the run reads back by id carries the pointers it was posted with, and nothing the old engine keeps
/// beside them.
#[test]
fn a_message_reads_back_with_its_board_pointers() {
    let (_dir, db) = workspace();
    let thread = chat(&db).mint_thread_id();
    let port = port(&db);
    port.open_thread(&thread, None, CHANNEL, "local/owner")
        .unwrap();
    let refs = ports::Refs {
        nxf_ids: vec!["6j6v.b8tj".to_owned()],
    };
    let posted = port
        .post(&thread, "local/owner", "build it", &refs, "root/r-1")
        .unwrap();
    let read = port.read(&posted).unwrap().unwrap();
    assert_eq!(read.refs, refs);
    assert_eq!(read.sender, "local/owner");
    assert_eq!(port.read("m-01JNOSUCHMESSAGE000000000").unwrap(), None);
}

/// A row whose `refs` do not decode is an error, never "not posted": find-or-post on that answer would post a
/// second copy (6j6v.b8tj review, IR #2). A read says so too, rather than handing the message on without
/// its pointers.
#[test]
fn a_row_whose_refs_do_not_decode_is_an_error_for_find_and_read() {
    let (dir, db) = workspace();
    let thread = chat(&db).mint_thread_id();
    let port = port(&db);
    port.open_thread(&thread, None, CHANNEL, OPENER).unwrap();
    let posted = port
        .post(&thread, OPENER, "the result", &ports::Refs::default(), KEY)
        .unwrap();
    let conn = rusqlite::Connection::open(dir.path().join(".nxs").join("db.sqlite")).unwrap();
    let damaged = conn
        .execute(
            "UPDATE messages SET refs = '{not json' WHERE message_id = ?1",
            [&posted],
        )
        .unwrap();
    assert_eq!(damaged, 1, "the row was damaged");
    let port = self::port(&db);
    assert!(
        port.find(&thread, KEY).is_err(),
        "find must not answer 'not posted'"
    );
    assert!(port.read(&posted).is_err());
}

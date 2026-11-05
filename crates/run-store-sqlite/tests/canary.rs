//! The value-free log (§7 privacy, §12.3, M8): a marker put into every free-text field reaches no event
//! body, and leaves `run.sqlite` with the append that folds `root_opened` or `finished`.
//!
//! Every event below is what the host builds from a text holding the marker: a reply, an answer, a raw
//! SDK end message, a refused command and git's stderr each become ids and codes. The bodies live in
//! `pending/<run>` and `tree_refusal/<run>` only, and the scan reads every column of every table.

mod fixtures;

use std::path::Path;

use nexus_run_contracts::{
    Cas, Decision, Disposition, EndCause, EventBody, GitReason, Origin, Outcome, RunId, RunStore,
    SessionFact, TreeRefusal,
};
use nexus_run_store_sqlite::SqliteRunStore;

const MARKER: &str = "CANARY-a7f3-do-not-log";
const MESSAGE: &str = "m-01JDDDDDDDDDDDDDDDDDDDDDDD";

fn commission_body() -> Vec<u8> {
    format!(
        r#"{{"root_thread":"{}","channel":"coding","sender":"requester","body":"{MARKER} in the commission text","refs":{{"nxf_ids":"{MARKER}"}}}}"#,
        fixtures::ROOT_THREAD
    )
    .into_bytes()
}

fn refusal_text() -> Vec<u8> {
    format!(r#"{{"stderr":"fatal: {MARKER} from git's stderr"}}"#).into_bytes()
}

fn set(key: String, to: &[u8]) -> Cas {
    Cas {
        key,
        from: None,
        to: Some(to.to_vec()),
    }
}

fn delete(key: String, from: &[u8]) -> Cas {
    Cas {
        key,
        from: Some(from.to_vec()),
        to: None,
    }
}

/// Every `table.column = rowid` in the file holding the marker, read through a connection of its own.
fn rows_holding_the_marker(path: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap();
    let tables: Vec<String> = tables
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut found = Vec::new();
    for table in tables {
        let mut columns = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns: Vec<String> = columns
            .query_map([], |row| row.get(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for column in columns {
            let sql =
                format!("SELECT count(*) FROM {table} WHERE instr(CAST({column} AS TEXT), ?1) > 0");
            let hits: i64 = conn.query_row(&sql, [MARKER], |row| row.get(0)).unwrap();
            if hits > 0 {
                found.push(format!("{table}.{column}"));
            }
        }
    }
    found
}

/// A commission as `run_host::commission` writes it: the body in `pending/<run>`, beside the inbox row.
fn commission(store: &SqliteRunStore, run: &RunId) -> u64 {
    store
        .enqueue(
            run,
            &fixtures::commissioned(run),
            &[set(format!("pending/{run}"), &commission_body())],
        )
        .unwrap();
    let lease = store
        .acquire_or_renew(run, "host-a", fixtures::t0())
        .unwrap();
    let input = store.next_input(run).unwrap().unwrap();
    store
        .append(
            run,
            0,
            lease.epoch,
            std::slice::from_ref(&input),
            std::slice::from_ref(&input.key),
            &[],
        )
        .unwrap();
    lease.epoch
}

/// What a whole run logs of the texts that held the marker.
fn events_from_marked_texts(run: &RunId) -> Vec<nexus_run_contracts::Event> {
    let session_key = format!("{run}/build/v1/coder/a1");
    let fact = |key: &str, fact: SessionFact| {
        fixtures::event(
            run,
            key,
            EventBody::SessionFact {
                session_key: session_key.clone(),
                fact,
            },
        )
    };
    vec![
        // A reply body and an answer body are chat messages: the log has their ids.
        fixtures::event(
            run,
            "reply/1",
            EventBody::Reply {
                session_key: session_key.clone(),
                message: MESSAGE.into(),
                escalate: false,
                needs_rework: false,
            },
        ),
        fixtures::event(
            run,
            "answer/1",
            EventBody::Answer {
                question: "decision/3/0".into(),
                disposition: Disposition::Continue,
                message: MESSAGE.into(),
            },
        ),
        // A refused command: its rule and a fail-closed program name, no argument.
        fact(
            "fact/refused",
            SessionFact::PermissionRefused {
                rule: "Bash".into(),
                program: "curl".into(),
            },
        ),
        // A raw SDK end message: the mapped cause only.
        fact(
            "fact/ended",
            SessionFact::Ended {
                cause: EndCause::ProviderUnavailable,
            },
        ),
        // Git's stderr: a closed reason only.
        fixtures::event(
            run,
            format!("tree_refused/{run}/1").as_str(),
            EventBody::TreeRefused {
                reason: TreeRefusal::Git {
                    reason: GitReason::BoundExceeded,
                },
            },
        ),
    ]
}

#[test]
fn after_root_opened_no_row_holds_the_commission_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let store = fixtures::open(&path);
    let run = fixtures::run_id();

    store
        .enqueue(
            &run,
            &fixtures::commissioned(&run),
            &[set(format!("pending/{run}"), &commission_body())],
        )
        .unwrap();
    assert_eq!(
        rows_holding_the_marker(&path),
        ["kv.value"],
        "the scan sees it, and only where it waits"
    );

    let lease = store
        .acquire_or_renew(&run, "host-a", fixtures::t0())
        .unwrap();
    let input = store.next_input(&run).unwrap().unwrap();
    store
        .append(
            &run,
            0,
            lease.epoch,
            std::slice::from_ref(&input),
            std::slice::from_ref(&input.key),
            &[],
        )
        .unwrap();
    assert_eq!(rows_holding_the_marker(&path), ["kv.value"], "not yet");

    let root_opened = fixtures::event(
        &run,
        "root_opened",
        EventBody::RootOpened {
            thread: fixtures::ROOT_THREAD.into(),
            message: MESSAGE.into(),
        },
    );
    store
        .append(
            &run,
            1,
            lease.epoch,
            &[root_opened],
            &[],
            &[delete(format!("pending/{run}"), &commission_body())],
        )
        .unwrap();
    assert_eq!(store.get(&format!("pending/{run}")).unwrap(), None);
    assert!(rows_holding_the_marker(&path).is_empty());
}

#[test]
fn after_finished_no_row_holds_git_s_stderr_or_any_other_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let store = fixtures::open(&path);
    let run = fixtures::run_id();
    let fence = commission(&store, &run);

    let root_opened = fixtures::event(
        &run,
        "root_opened",
        EventBody::RootOpened {
            thread: fixtures::ROOT_THREAD.into(),
            message: MESSAGE.into(),
        },
    );
    store
        .append(
            &run,
            1,
            fence,
            &[root_opened],
            &[],
            &[delete(format!("pending/{run}"), &commission_body())],
        )
        .unwrap();

    // A refusal inside the window is a kv row (§6.6), and it stays there until `finished`.
    let refusal_key = format!("tree_refusal/{run}");
    store
        .compare_and_set(&refusal_key, None, Some(&refusal_text()))
        .unwrap();
    let events = events_from_marked_texts(&run);
    let count = events.len() as u64;
    store.append(&run, 2, fence, &events, &[], &[]).unwrap();
    assert_eq!(
        rows_holding_the_marker(&path),
        ["kv.value"],
        "none of the events carried it"
    );
    assert_eq!(store.get(&refusal_key).unwrap(), Some(refusal_text()));

    store
        .append(
            &run,
            2 + count,
            fence,
            &[fixtures::finished(&run, "end")],
            &[],
            &[delete(refusal_key.clone(), &refusal_text())],
        )
        .unwrap();
    assert_eq!(
        store.get(&refusal_key).unwrap(),
        None,
        "gone with `finished`"
    );
    assert!(rows_holding_the_marker(&path).is_empty());
}

#[test]
fn a_run_withdrawn_before_its_root_opened_deletes_the_commission_text_on_finished() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let store = fixtures::open(&path);
    let run = fixtures::run_id();
    let fence = commission(&store, &run);
    assert_eq!(rows_holding_the_marker(&path), ["kv.value"]);

    let withdrawn = nexus_run_contracts::Event::new(
        run.clone(),
        0,
        fixtures::t0(),
        "end",
        Origin::Reducer,
        EventBody::Decision(Decision::Finished {
            outcome: Outcome::Withdrawn,
            reason: None,
        }),
    );
    store
        .append(
            &run,
            1,
            fence,
            &[withdrawn],
            &[],
            &[delete(format!("pending/{run}"), &commission_body())],
        )
        .unwrap();
    assert!(rows_holding_the_marker(&path).is_empty());
}

#[test]
fn the_scan_finds_a_marker_in_an_event_body() {
    // A check on the scan itself: an empty result above means nothing unless it can find one in the log.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let store = fixtures::open(&path);
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    let mut marked = fixtures::input(&run, "marked");
    marked
        .extra
        .insert("from_a_newer_writer".into(), MARKER.into());
    store.append(&run, 1, fence, &[marked], &[], &[]).unwrap();
    assert_eq!(rows_holding_the_marker(&path), ["events.body"]);
}

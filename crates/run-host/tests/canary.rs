//! The value-free log and the findable keys (§12.3 M8, PV1, manufakt H5). A marker goes into every free-text field
//! the host can see: commission text, refs, a reply body, an answer body, a refusal's text, the turn. It must
//! appear in no host log line, in no event body and, once the run has finished, in no kv row. And every kv row of
//! a finished run must be reachable by the derivation the contract's tombstone rule states.

mod common;

use std::sync::{Arc, Mutex};

use nexus_run_contracts::{GitReason, RunStore, TreeRefusal};
use nexus_run_host::bridge::{self, ReplyFlags};
use nexus_run_host::ports::Refs;
use nexus_run_host::test_support::scripted_backend::{pass, Cue};
use nexus_run_host::test_support::scripted_tree::TreeStep;
use nexus_run_host::test_support::{store_path, Rig, CLOCK_START, WORKSPACE};
use nexus_run_host::TreeResult;

const MARKER: &str = "CANARY-7f3a";

fn with_marker(text: &str) -> String {
    format!("{text} {MARKER}")
}

/// A run that carries the marker through everything, escalation and rework included.
fn run_with_markers(
    dir: &std::path::Path,
) -> (
    Rig,
    nexus_run_contracts::RunId,
    String,
    Arc<Mutex<Vec<String>>>,
) {
    let mut rig = Rig::scripted(
        dir,
        CLOCK_START,
        Box::new(|info| {
            if info.step == "verify" && info.attempt == 1 {
                Cue::Reply {
                    text: with_marker("a question"),
                    flags: ReplyFlags {
                        escalate: true,
                        needs_rework: false,
                    },
                }
            } else if info.step == "judge" && info.visit == 1 {
                Cue::Reply {
                    text: with_marker("a verdict"),
                    flags: ReplyFlags::default(),
                }
            } else {
                pass()
            }
        }),
    );
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = lines.clone();
    rig.host.set_log_sink(Arc::new(move |line| {
        sink.lock().unwrap().push(line.to_owned())
    }));
    // git's stderr, in the only place it may be: the refusal's kv row.
    rig.tree.script(vec![TreeStep::Answer(TreeResult::Refused {
        reason: TreeRefusal::Git {
            reason: GitReason::FetchFailed,
        },
        text: with_marker("fatal: unable to access https://user:token@example.invalid/"),
    })]);
    let refs = Refs {
        nxf_ids: vec![with_marker("ref")],
    };
    let receipt = rig.commission_refs(&with_marker("the commission"), &refs);
    let check = receipt.run.clone();
    rig.run_until(600, move |rig| {
        rig.state(&check).open_questions().count() == 1
    });
    bridge::answer(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        &with_marker("the answer"),
        false,
        rig.now(),
    )
    .unwrap();
    rig.run_to_finished(&receipt.run);
    (rig, receipt.run, receipt.root_thread, lines)
}

#[test]
fn the_marker_reaches_no_event_no_host_log_line_and_no_kv_row_after_finished() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run, _, lines) = run_with_markers(dir.path());

    // The marker did travel: the chat store holds it, so the assertions below are about something.
    assert!(rig
        .ports
        .lines()
        .iter()
        .any(|line| format!("{line:?}").contains(MARKER)));

    for event in rig.events(&run) {
        let wire = serde_json::to_string(&event).unwrap();
        assert!(!wire.contains(MARKER), "in the log: {wire}");
        assert!(!wire.contains("token"), "a URL's secret in the log: {wire}");
    }
    let lines = lines.lock().unwrap();
    for line in lines.iter() {
        assert!(!line.contains(MARKER), "in a host log line: {line}");
    }

    let conn = rusqlite::Connection::open(store_path(dir.path())).unwrap();
    let mut rows = conn.prepare("SELECT key, value FROM kv").unwrap();
    let rows: Vec<(String, Vec<u8>)> = rows
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!rows.is_empty());
    for (key, value) in &rows {
        let value = String::from_utf8_lossy(value);
        assert!(
            !value.contains(MARKER),
            "kv row {key} holds the marker: {value}"
        );
        assert!(!key.contains(MARKER), "a kv key holds the marker: {key}");
    }
    assert_eq!(rig.store.get(&format!("pending/{run}")).unwrap(), None);
    assert_eq!(rig.store.get(&format!("tree_refusal/{run}")).unwrap(), None);
}

/// The commission body is in the store only until the root is open, and the refusal's text is scrubbed there.
#[test]
fn the_body_leaves_the_store_with_root_opened_and_a_urls_secret_never_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(|_| Cue::Silent));
    rig.tree.script(vec![TreeStep::Answer(TreeResult::Refused {
        reason: TreeRefusal::Git {
            reason: GitReason::FetchFailed,
        },
        text: "fatal: https://user:token@example.invalid/x".to_owned(),
    })]);
    let run = rig.commission(&with_marker("the commission")).run;
    let pending = format!("pending/{run}");
    assert!(rig.store.get(&pending).unwrap().is_some());
    let check = run.clone();
    rig.run_until(50, move |rig| rig.has(&check, "root_opened"));
    assert_eq!(
        rig.store.get(&pending).unwrap(),
        None,
        "gone with root_opened"
    );

    let key = format!("tree_refusal/{run}");
    let probe = key.clone();
    rig.run_until(50, move |rig| rig.store.get(&probe).unwrap().is_some());
    let text = String::from_utf8(rig.store.get(&key).unwrap().unwrap()).unwrap();
    assert!(!text.contains("token"), "{text}");
    assert!(text.contains("example.invalid"), "{text}");
}

/// The derivation of the contract's tombstone rule, as manufakt's H5 disposition states it: by run prefix, or
/// `thread/<id>` from `commissioned.root_thread` plus the `effect/<run>/thread/*` values. `flow/`, `claim/` and
/// `host-seen/` hold nothing of a run.
#[test]
fn every_kv_row_of_a_finished_run_is_reachable_by_the_stated_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run, root, _) = run_with_markers(dir.path());
    let conn = rusqlite::Connection::open(store_path(dir.path())).unwrap();
    let mut rows = conn.prepare("SELECT key FROM kv").unwrap();
    let keys: Vec<String> = rows
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        keys.iter().any(|key| key.starts_with("effect/")),
        "{keys:?}"
    );
    assert!(
        keys.iter().any(|key| key.starts_with("thread/")),
        "{keys:?}"
    );

    // Everything the derivation can reach, from what is written.
    let mut threads = vec![format!("thread/{root}")];
    for key in keys
        .iter()
        .filter(|key| key.starts_with(&format!("effect/{run}/thread/")))
    {
        let value = rig.store.get(key).unwrap().unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&value).unwrap();
        let state = entry["state"].as_str().unwrap();
        let id = state
            .strip_prefix("done:")
            .expect("a finished run's thread is done");
        threads.push(format!("thread/{id}"));
    }
    let by_prefix = [
        format!("owner/{run}"),
        format!("effect/{run}/"),
        format!("persona_session/{run}/"),
        format!("pending/{run}"),
        format!("tree_refusal/{run}"),
        format!("poison/{run}/"),
        format!("session/{run}/"),
    ];
    for key in &keys {
        let reachable = by_prefix.iter().any(|prefix| key.starts_with(prefix))
            || threads.contains(key)
            || key.starts_with("flow/")
            || key == &format!("claim/{WORKSPACE}")
            || key == &format!("host-seen/{WORKSPACE}");
        assert!(reachable, "{key} cannot be found from the run");
    }
}

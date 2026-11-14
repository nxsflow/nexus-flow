//! A session key names a directory under `<agent-logs>/v2/`, and nothing else (tv58 review, Low, raised for
//! 6j6v.pn9c): the key is joined onto the ledger root, so an absolute key or one that climbs with `..` would put
//! the ledger, and every file the adapter writes or reads for that session, outside it.

mod common;

use common::*;
use nexus_run_contracts::SessionBackend;
use serde_json::json;

/// Keys that would leave `<agent-logs>/v2/` if they were joined as they are.
const ESCAPING: [&str; 7] = [
    "/tmp/elsewhere",
    "../escaped",
    "run1/../../escaped",
    "run1/./a1",
    "",
    "C:",
    "C:/elsewhere",
];

#[test]
fn a_key_that_would_leave_the_ledger_root_is_refused_by_every_verb() {
    let bench = Bench::new();
    let backend = bench.backend();
    for key in ESCAPING {
        let refused = |what: &str, error: Option<String>| {
            let error = error.unwrap_or_else(|| panic!("{what} accepted the key {key:?}"));
            assert!(
                error.contains("session key"),
                "{what} on {key:?} names the key as the reason: {error}"
            );
        };
        refused("alive", backend.alive(key).err().map(|e| e.what));
        refused("stop", backend.stop(key, false).err().map(|e| e.what));
        refused("nudge", backend.nudge(key, "hello").err().map(|e| e.what));
        refused(
            "drain_facts",
            backend.drain_facts(key, 0).err().map(|e| e.what),
        );
        refused("sample_now", backend.sample_now(key).err().map(|e| e.what));
        refused(
            "start",
            backend
                .start(&bench.spec(key, json!({ "steps": [] })))
                .err()
                .map(|e| e.what),
        );
        assert!(
            backend.findings(key).is_empty(),
            "findings reads nothing for {key:?}"
        );
    }
    // Nothing was created beside the ledger root, nor above it.
    let mut left: Vec<_> = std::fs::read_dir(&bench.logs)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    left.sort();
    assert!(
        left.is_empty() || left == ["v2"],
        "nothing was written outside <logs>/v2: {left:?}"
    );
    assert!(!bench.logs.parent().unwrap().join("escaped").exists());
}

#[test]
fn an_ordinary_key_with_several_components_is_accepted() {
    let bench = Bench::new();
    let backend = bench.backend();
    // No child recorded and no spawn: the honest answer for a key nobody started is dead, not a refusal.
    assert!(backend.alive(KEY).is_ok());
    assert!(backend.drain_facts(KEY, 0).is_ok());
}

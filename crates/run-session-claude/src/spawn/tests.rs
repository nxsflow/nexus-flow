//! The ask files a nudge writes.

use super::*;

/// tv58 review, Low, raised for 6j6v.pn9c: two nudges in one millisecond named the same file, and the second
/// rename replaced the first ask before the sidecar drained it.
#[test]
fn two_asks_written_in_one_millisecond_are_both_kept_in_the_order_written() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = SessionLedger::new(temp.path().join("key"));
    ledger.create().unwrap();
    write_ask(&ledger, "first", 1_700_000_000_000).unwrap();
    write_ask(&ledger, "second", 1_700_000_000_000).unwrap();
    // The sidecar's own reader: names starting `ask.` and ending `.json`, sorted (agent-sidecar facts.mjs).
    let mut names: Vec<String> = std::fs::read_dir(ledger.dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ask.") && name.ends_with(".json"))
        .collect();
    names.sort();
    let texts: Vec<String> = names
        .iter()
        .map(|name| {
            let body: serde_json::Value =
                serde_json::from_slice(&std::fs::read(ledger.dir().join(name)).unwrap()).unwrap();
            body["text"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(texts, ["first", "second"]);
}

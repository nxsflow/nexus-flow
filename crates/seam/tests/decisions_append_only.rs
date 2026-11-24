//! RFC 0003, "Append-only": the seam has no symbol that changes or deletes an object of the four
//! decision collections. Its public functions are read from its source: none of them names a
//! decision collection's object together with a verb that changes or deletes, and the seam
//! re-exports nothing of the store's write path, through which a caller could try one.

use std::fs;
use std::path::{Path, PathBuf};

/// What a decision collection's object is called in a symbol's name.
const DECISION_OBJECTS: &[&str] = &["decision", "topic", "subject", "supersession"];

/// The verbs of a symbol that would change or delete an object.
const CHANGES: &[&str] = &[
    "set", "update", "edit", "change", "rewrite", "amend", "redact", "delete", "remove", "restore",
    "undelete", "erase", "withdraw",
];

/// The store's modules that write: re-exporting them would hand a caller the write path.
const WRITE_PATH: &[&str] = &["nexus_flow_store::write", "nexus_flow_store::write_once"];

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// The public symbols that would change or delete a decision object, in `source`.
fn violations(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines().map(str::trim) {
        if line.starts_with("pub use") || line.starts_with("pub(crate) use") {
            if line.starts_with("pub use") && WRITE_PATH.iter().any(|m| line.contains(m)) {
                found.push(line.to_owned());
            }
            continue;
        }
        let Some(rest) = line
            .strip_prefix("pub async fn ")
            .or_else(|| line.strip_prefix("pub fn "))
        else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let words: Vec<&str> = name.split('_').collect();
        let object = words.iter().any(|w| {
            DECISION_OBJECTS
                .iter()
                .any(|o| w.trim_end_matches('s') == *o)
        });
        let change = words.iter().any(|w| CHANGES.contains(w));
        if object && change {
            found.push(name);
        }
    }
    found
}

#[test]
fn the_seam_has_no_symbol_that_changes_or_deletes_a_decision() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty());
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        assert_eq!(
            violations(&source),
            Vec::<String>::new(),
            "{} has a public symbol that changes or deletes a decision",
            file.display()
        );
    }
}

/// The check finds what it looks for, so its silence on the seam means something.
#[test]
fn the_check_finds_a_symbol_that_changes_or_deletes_a_decision() {
    let planted = "
        impl Engine {
            pub async fn edit_decision(&self) {}
            pub fn delete_topics(&self) {}
            pub fn decision_restore(&self) {}
            pub fn add_decision(&self) {}
            pub fn set_relay(&self) {}
        }
        pub use nexus_flow_store::write::{Write, WriteError};
        pub use nexus_flow_store::sync::SyncStatus;
    ";
    assert_eq!(
        violations(planted),
        [
            "edit_decision",
            "delete_topics",
            "decision_restore",
            "pub use nexus_flow_store::write::{Write, WriteError};",
        ]
    );
}

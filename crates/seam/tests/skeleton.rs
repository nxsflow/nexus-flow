//! NEX-212: the skeleton's crate doc names its RFC, and its dependency edges hold: it reaches
//! NoCentralDB only through `store`, and links only the workspace crates RFC 0001 allows it.

use std::path::Path;

const RFC: &str = "RFC 0001";
const WORKSPACE_DEPS: &[&str] = &["nexus-flow-store"];

fn read(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The keys of the manifest's dependency tables, and which of them are workspace crates (`path`).
fn dependencies() -> Vec<(String, bool)> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in read("Cargo.toml").lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.ends_with("dependencies]");
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split(['=', '.']).next().unwrap().trim().to_string();
        deps.push((key, line.contains("path =")));
    }
    deps
}

#[test]
fn the_crate_doc_is_one_line_naming_its_rfc() {
    let source = read("src/lib.rs");
    let doc: Vec<&str> = source
        .lines()
        .take_while(|l| l.starts_with("//!"))
        .collect();
    assert_eq!(doc.len(), 1, "a one-line crate doc, got {doc:?}");
    assert!(
        doc[0].contains(RFC),
        "the crate doc names {RFC}: {}",
        doc[0]
    );
}

#[test]
fn no_dependency_on_nocentraldb_but_through_store() {
    let deps = dependencies();
    assert!(
        deps.iter().all(|(name, _)| name != "nocentraldb"),
        "only nexus-flow-store depends on the SDK: {deps:?}"
    );
}

#[test]
fn links_only_the_workspace_crates_it_may() {
    // A crate named in two tables, such as a dev-dependency that only enables a feature, is
    // linked once.
    let mut workspace: Vec<String> = dependencies()
        .into_iter()
        .filter(|(_, p)| *p)
        .map(|(n, _)| n)
        .collect();
    workspace.sort_unstable();
    workspace.dedup();
    assert_eq!(workspace, WORKSPACE_DEPS, "workspace dependencies");
}

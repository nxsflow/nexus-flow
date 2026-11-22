//! NEX-212: the skeleton's crate doc names its RFC, and its dependency edges hold: it reaches
//! NoCentralDB only through `store`, and links only the workspace crates RFC 0001 allows it.
//! NEX-180: its tests alone may link v1's `nxs-foundation`, to check that v1 refuses a moved
//! workspace, and the crate itself depends on no v1 crate.

use std::path::Path;

const RFC: &str = "RFC 0008";
const WORKSPACE_DEPS: &[&str] = &[];
/// The workspace crates the tests may link besides [`WORKSPACE_DEPS`]: v1's foundation, whose
/// `assess_open` is what refuses a moved workspace.
const WORKSPACE_DEV_DEPS: &[&str] = &["nxs-foundation"];
/// v1's crates on the base branch. The move reads v1's data by its format, never through them.
const V1_CRATES: &[&str] = &[
    "nexus-chat",
    "nexus-flow-cli",
    "nexus-flow-core",
    "nexus-flow-facade",
    "nexus-memory",
    "nxs",
    "nxs-fold-ddb",
    "nxs-foundation",
    "nxs-guide",
    "nxs-init",
    "nxs-server",
    "nxs-service",
    "nxs-sync",
    "nxs-test-support",
    "nxs-ui",
    "plugin-probe",
];

fn read(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// One entry of the manifest's dependency tables.
#[derive(Debug)]
struct Dep {
    name: String,
    /// A workspace crate, named by `path`.
    path: bool,
    /// Declared under `[dev-dependencies]`, so linked by the tests only.
    dev: bool,
}

/// The entries of the manifest's dependency tables.
fn dependencies() -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut table: Option<bool> = None;
    for line in read("Cargo.toml").lines() {
        let line = line.trim();
        if line.starts_with('[') {
            table = line
                .ends_with("dependencies]")
                .then(|| line.ends_with("dev-dependencies]"));
            continue;
        }
        let Some(dev) = table else { continue };
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split(['=', '.']).next().unwrap().trim().to_string();
        deps.push(Dep {
            name,
            path: line.contains("path ="),
            dev,
        });
    }
    deps
}

fn workspace_deps(dev: bool) -> Vec<String> {
    dependencies()
        .into_iter()
        .filter(|d| d.path && d.dev == dev)
        .map(|d| d.name)
        .collect()
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
        deps.iter().all(|d| d.name != "nocentraldb"),
        "only nexus-flow-store depends on the SDK: {deps:?}"
    );
}

#[test]
fn links_only_the_workspace_crates_it_may() {
    assert_eq!(
        workspace_deps(false),
        WORKSPACE_DEPS,
        "workspace dependencies"
    );
}

#[test]
fn its_tests_link_only_the_workspace_crates_they_may() {
    assert_eq!(
        workspace_deps(true),
        WORKSPACE_DEV_DEPS,
        "workspace dev-dependencies"
    );
}

#[test]
fn the_crate_depends_on_no_v1_crate() {
    let v1: Vec<Dep> = dependencies()
        .into_iter()
        .filter(|d| !d.dev && V1_CRATES.contains(&d.name.as_str()))
        .collect();
    assert!(v1.is_empty(), "only the tests may link a v1 crate: {v1:?}");
}

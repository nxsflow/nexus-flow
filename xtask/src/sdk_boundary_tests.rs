use std::path::Path;

use serde_json::{json, Value};

use super::*;
use crate::layering::read_metadata;

fn dep(name: &str, kind: Option<&str>) -> Value {
    json!({ "name": name, "kind": kind, "rename": null, "features": [] })
}

fn pkg(name: &str, deps: Vec<Value>) -> Value {
    json!({ "name": name, "dependencies": deps, "features": {} })
}

fn metadata(packages: Vec<Value>) -> Metadata {
    serde_json::from_value(json!({ "packages": packages })).unwrap()
}

/// The storage crate on the SDK, and a crate beside it that reaches NoCentralDB through it.
fn legal() -> Vec<Value> {
    vec![
        pkg("nexus-flow-store", vec![dep("nocentraldb", None)]),
        pkg(
            "nexus-flow-core",
            vec![dep("nexus-flow-store", None), dep("serde", None)],
        ),
    ]
}

#[test]
fn passes_when_only_the_storage_crate_depends_on_the_sdk() {
    check(&metadata(legal())).unwrap();
}

#[test]
fn fails_when_another_crate_lists_the_sdk_in_any_dependency_kind() {
    for kind in [None, Some("dev"), Some("build")] {
        let mut packages = legal();
        packages.push(pkg("nexus-flow-cli", vec![dep("nocentraldb", kind)]));
        let err = check(&metadata(packages)).unwrap_err().to_string();
        assert!(err.contains("nexus-flow-cli"), "{kind:?}: {err}");
        assert!(err.contains("nocentraldb"), "{kind:?}: {err}");
    }
}

#[test]
fn fails_on_a_renamed_sdk_dependency() {
    let mut packages = legal();
    packages.push(pkg(
        "nexus-flow-cli",
        vec![json!({ "name": "nocentraldb", "kind": null, "rename": "ncdb", "features": [] })],
    ));
    assert!(check(&metadata(packages)).is_err());
}

#[test]
fn fails_on_another_nocentraldb_crate() {
    let mut packages = legal();
    packages.push(pkg("nexus-flow-cli", vec![dep("ncdb-core", None)]));
    let err = check(&metadata(packages)).unwrap_err().to_string();
    assert!(err.contains("ncdb-core"), "{err}");
}

#[test]
fn names_every_crate_that_crosses_the_boundary() {
    let mut packages = legal();
    packages.push(pkg("nexus-flow-cli", vec![dep("nocentraldb", None)]));
    packages.push(pkg("xtask", vec![dep("nocentraldb", Some("dev"))]));
    let err = check(&metadata(packages)).unwrap_err().to_string();
    assert!(err.contains("nexus-flow-cli") && err.contains("xtask"), "{err}");
}

#[test]
fn the_workspace_keeps_the_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    check(&read_metadata(root).unwrap()).unwrap();
}

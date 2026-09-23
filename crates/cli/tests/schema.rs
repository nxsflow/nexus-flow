//! `nxf schema --json` (8qv.5): a machine-readable description of the active plugin's full field
//! model — vocabulary (types/statuses/priorities), and per field its plugin label, whether it is
//! required on create / settable on update, how it is set (flag/alias), and its kind. This is the
//! agent-ergonomic answer to "what fields does create/update take here", introspectable at runtime
//! (clap `--help` is static at compile time and cannot carry the active plugin's vocabulary).

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

fn init(dir: &Path, plugin: &str) {
    nxf()
        .args(["init", "--plugin", plugin])
        .current_dir(dir)
        .assert()
        .success();
}

fn schema(dir: &Path) -> serde_json::Value {
    let out = nxf()
        .args(["schema", "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("schema json")
}

/// Find a field entry by its canonical `field` name.
fn field<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("field {name} present in schema"))
}

#[test]
fn issue_tracker_schema_describes_vocabulary_and_field_model() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let v = schema(tmp.path());

    assert_eq!(v["plugin"], serde_json::json!("issue-tracker"));
    // `priorities` is a keyed {ordinal: label} map, structurally parallel to `types`
    // (nexus-flow-3r6): one resolution rule `schema.<field>[item.<field>]` covers both
    // `type` and `priority`. The stored item value stays the ordinal (8qv.2 intact).
    assert_eq!(
        v["priorities"],
        serde_json::json!({"0": "P0", "1": "P1", "2": "P2", "3": "P3", "4": "P4"})
    );
    // The same keyed-lookup shape as `types`: resolve an item's ordinal to its label.
    assert_eq!(v["priorities"]["0"], serde_json::json!("P0"));
    assert_eq!(
        v["statuses"]["in_progress"],
        serde_json::json!("in progress")
    );
    // `types` is the declared set keyed {type: label}; a type with no override labels itself.
    assert_eq!(v["types"]["epic"], serde_json::json!("epic"));
    assert_eq!(v["types"]["bug"], serde_json::json!("bug"));
    assert_eq!(v["types"]["decision"], serde_json::json!("decision"));

    // Mandatory create fields.
    for name in ["title", "description", "priority"] {
        assert_eq!(
            field(&v, name)["required"],
            serde_json::json!(true),
            "{name} required"
        );
    }
    // Optional + labelled.
    assert_eq!(
        field(&v, "description")["label"],
        serde_json::json!("Description")
    );
    assert_eq!(field(&v, "priority")["kind"], serde_json::json!("priority"));
    assert_eq!(field(&v, "design")["required"], serde_json::json!(false));
    // The DoD field: plugin label + the `--dod` create flag.
    let dod = field(&v, "completion_criterion");
    assert_eq!(dod["label"], serde_json::json!("Definition of Done"));
    assert_eq!(dod["create_flag"], serde_json::json!("--dod"));
    // parent alias + flag.
    let parent = field(&v, "belongs_to");
    assert_eq!(parent["create_flag"], serde_json::json!("--parent"));
    assert_eq!(parent["update_alias"], serde_json::json!("parent"));
    // Settable-on-update fields carry settable: true; there is no longer a `body` field (#25i).
    assert_eq!(field(&v, "status")["settable"], serde_json::json!(true));
    assert!(
        v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["field"] != serde_json::json!("body")),
        "the removed body field must not appear in the schema"
    );

    // Dependencies are reachable at create.
    assert_eq!(
        v["dependencies"]["create_flag"],
        serde_json::json!("--depends-on")
    );
    assert_eq!(v["dependencies"]["repeatable"], serde_json::json!(true));
}

#[test]
fn personal_todo_schema_speaks_its_own_vocabulary() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "personal-todo");
    let v = schema(tmp.path());

    assert_eq!(v["plugin"], serde_json::json!("personal-todo"));
    // Keyed {ordinal: label} map, same shape as `types` (nexus-flow-3r6).
    assert_eq!(
        v["priorities"],
        serde_json::json!({"0": "now", "1": "soon", "2": "later", "3": "someday", "4": "icebox"})
    );
    assert_eq!(v["priorities"]["4"], serde_json::json!("icebox"));
    assert_eq!(v["types"]["todo"], serde_json::json!("todo"));
    assert_eq!(v["types"]["project"], serde_json::json!("project"));
    // The same core fields, named in this plugin's voice (the field-label seam).
    assert_eq!(field(&v, "description")["label"], serde_json::json!("Why"));
    assert_eq!(
        field(&v, "completion_criterion")["label"],
        serde_json::json!("Done when")
    );
    assert_eq!(field(&v, "design")["label"], serde_json::json!("Plan"));
}

#[test]
fn schema_exposes_the_type_hierarchy_and_containers() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let v = schema(tmp.path());
    // y5j8: the additive hierarchy block names containers + the depth limit.
    assert_eq!(v["hierarchy"]["max_depth"], serde_json::json!(2));
    assert_eq!(
        v["hierarchy"]["types"]["epic"]["container"],
        serde_json::json!(true)
    );
    assert_eq!(
        v["hierarchy"]["types"]["bug"]["container"],
        serde_json::json!(false)
    );
    assert_eq!(
        v["hierarchy"]["types"]["bug"]["parents"],
        serde_json::json!(["epic"])
    );
    assert_eq!(
        v["hierarchy"]["types"]["bug"]["cardinality"],
        serde_json::json!("single")
    );

    // The human view names the containers.
    let out = nxf()
        .args(["schema"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(out).unwrap();
    assert!(human.contains("containers"), "{human}");
    assert!(human.contains("epic"), "{human}");
}

#[test]
fn human_schema_lists_required_fields_and_priorities() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path(), "issue-tracker");
    let out = nxf()
        .args(["schema"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(out).unwrap();
    // The human view names the required create fields and the priority variants.
    assert!(human.contains("required"), "{human}");
    assert!(human.contains("P0") && human.contains("P4"), "{human}");
    assert!(human.contains("Definition of Done"), "{human}");
}

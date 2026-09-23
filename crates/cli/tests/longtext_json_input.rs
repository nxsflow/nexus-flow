//! Epic 95d.3 — JSON payload over STDIN: `<json> | nxf create --json -` and
//! `<json> | nxf update <id> --json -`. The whole write is one piped object — no escaping, no
//! flag combinatorics — and it feeds the SAME facade validation path as the flag forms (no
//! second validation logic). The `-` positional triggers reading STDIN as a JSON object; the
//! global `--json` independently selects JSON output.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

fn init(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

/// Long-text values carrying backticks, quotes, a bang and an embedded `=` — the JSON string
/// escaping is serde's problem, not the shell's.
const DESC: &str = "why this exists\nwith `backticks`, \"quotes\", a bang! and x = y";
const DESIGN: &str = "the approach:\n- step `one`\n- step \"two\"";

fn create_via_json(dir: &Path, payload: &str) -> assert_cmd::assert::Assert {
    nxf()
        .args(["create", "--json", "-"])
        .current_dir(dir)
        .write_stdin(payload.to_owned())
        .assert()
}

fn new_task(dir: &Path) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "seed",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn create_full_object_from_json_stdin() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let payload = serde_json::json!({
        "type": "bug",
        "title": "From JSON",
        "description": DESC,
        "priority": "P2",
        "design": DESIGN,
        "dod": "done when the tests pass",
    })
    .to_string();
    let out = create_via_json(tmp.path(), &payload)
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["type"], serde_json::json!("bug"));
    assert_eq!(v["title"], serde_json::json!("From JSON"));
    assert_eq!(v["description"], serde_json::json!(DESC));
    assert_eq!(v["design"], serde_json::json!(DESIGN));
    assert_eq!(
        v["completion_criterion"],
        serde_json::json!("done when the tests pass")
    );
    // Priority label resolved to its canonical ordinal by the SAME facade path.
    assert_eq!(v["priority"], serde_json::json!("2"));
}

#[test]
fn update_multi_field_from_json_stdin() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let payload = serde_json::json!({ "description": DESC, "design": DESIGN }).to_string();
    let out = nxf()
        .args(["update", &id, "--json", "-"])
        .current_dir(tmp.path())
        .write_stdin(payload)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["description"], serde_json::json!(DESC));
    assert_eq!(v["design"], serde_json::json!(DESIGN));
}

#[test]
fn create_json_missing_required_field_is_error_and_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // No "priority".
    let payload =
        serde_json::json!({ "type": "bug", "title": "X", "description": "d" }).to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &create_via_json(tmp.path(), &payload)
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    // Nothing created.
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn create_json_unknown_key_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let payload = serde_json::json!({
        "type": "bug", "title": "X", "description": "d", "priority": "P1",
        "bogus": "nope"
    })
    .to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &create_via_json(tmp.path(), &payload)
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
}

#[test]
fn invalid_json_is_error_and_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v: serde_json::Value = serde_json::from_slice(
        &create_via_json(tmp.path(), "{not valid json")
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn json_stdin_with_a_field_flag_is_an_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let payload =
        serde_json::json!({ "type": "bug", "title": "X", "description": "d", "priority": "P1" })
            .to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &nxf()
            .args(["create", "--json", "-", "--design", "extra"])
            .current_dir(tmp.path())
            .write_stdin(payload)
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
}

#[test]
fn update_json_non_string_value_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = new_task(tmp.path());
    let payload = serde_json::json!({ "description": 42 }).to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &nxf()
            .args(["update", &id, "--json", "-"])
            .current_dir(tmp.path())
            .write_stdin(payload)
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    // Writes nothing on error — symmetric with the create-side tests: the item is unchanged
    // (description still the seed "d", not the rejected 42).
    let list = nxf()
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let items: serde_json::Value = serde_json::from_slice(&list).unwrap();
    assert_eq!(items[0]["id"], serde_json::json!(id));
    assert_eq!(items[0]["description"], serde_json::json!("d"));
}

#[test]
fn create_json_uses_same_validation_as_flags() {
    // An unknown priority label must fail the SAME way as the flag path — proving the JSON path
    // reuses the facade validation rather than a second code path.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let payload = serde_json::json!({
        "type": "bug", "title": "X", "description": "d", "priority": "P9"
    })
    .to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &create_via_json(tmp.path(), &payload)
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"].as_str().unwrap_or_default();
    assert!(
        msg.contains("priority"),
        "expected priority error, got: {msg}"
    );
}

#[test]
fn create_json_with_dependencies() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let dep = new_task(tmp.path());
    let payload = serde_json::json!({
        "type": "bug", "title": "Dependent", "description": "d", "priority": "P1",
        "depends_on": [dep],
    })
    .to_string();
    let out = create_via_json(tmp.path(), &payload)
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let id = v["id"].as_str().unwrap();
    // The dependency edge is real: the new item is blocked by its open dependency.
    let blocked = nxf()
        .args(["blocked", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let blocked: serde_json::Value = serde_json::from_slice(&blocked).unwrap();
    assert!(
        blocked.as_array().unwrap().iter().any(|i| i["id"] == id),
        "new item should be blocked by its dependency"
    );
}

#[test]
fn non_dash_positional_is_a_clear_error() {
    // The JSON-stdin positional accepts only `-`; anything else is a usage error (not a silent
    // no-op), and writes nothing.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v: serde_json::Value = serde_json::from_slice(
        &nxf()
            .args(["create", "--json", "nope"])
            .current_dir(tmp.path())
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"].as_str().unwrap_or_default();
    assert!(msg.contains("nope") && msg.contains('-'), "got: {msg}");
}

// ---- 3bw6: `schema` advertises the exact create --json keys + value forms ---------------------

fn schema_json(dir: &Path) -> serde_json::Value {
    let out = nxf()
        .args(["schema", "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("schema --json is valid JSON")
}

fn schema_field<'a>(schema: &'a serde_json::Value, field: &str) -> &'a serde_json::Value {
    schema["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == serde_json::json!(field))
        .unwrap_or_else(|| panic!("field '{field}' present in schema"))
}

#[test]
fn schema_json_advertises_the_create_payload_keys_and_priority_value_form() {
    // 3bw6: an agent building a `create --json` payload from `schema` must find the EXACT key + value
    // form per field, so it never has to trial-and-error against `deny_unknown_fields`. The three
    // documented divergences (canonical field column vs create-payload key) are named explicitly…
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let schema = schema_json(tmp.path());
    assert_eq!(
        schema_field(&schema, "completion_criterion")["create_json_key"],
        serde_json::json!("dod")
    );
    assert_eq!(
        schema_field(&schema, "defer_until")["create_json_key"],
        serde_json::json!("defer")
    );
    assert_eq!(
        schema_field(&schema, "belongs_to")["create_json_key"],
        serde_json::json!("parent")
    );
    // …priority's create VALUE is the label set (not the read-form ordinal `show --json` emits)…
    assert_eq!(
        schema_field(&schema, "priority")["create_json_values"],
        serde_json::json!(["P0", "P1", "P2", "P3", "P4"])
    );
    // …a non-divergent field keeps its own name, and a non-createable field advertises no key.
    assert_eq!(
        schema_field(&schema, "title")["create_json_key"],
        serde_json::json!("title")
    );
    assert!(schema_field(&schema, "status")
        .get("create_json_key")
        .is_none());
}

#[test]
fn create_json_with_the_canonical_dod_key_points_at_the_payload_key() {
    // 3bw6: a payload keyed with the read/`schema`/`update --set` name `completion_criterion` gets a
    // targeted pointer at `dod`, not the bare `deny_unknown_fields` list that names every key but the
    // right one — so the first mis-keyed attempt already carries the fix.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let payload = serde_json::json!({
        "type": "bug", "title": "X", "description": "d", "priority": "P2",
        "completion_criterion": "done when the tests pass",
    })
    .to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &create_via_json(tmp.path(), &payload)
            .failure()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(v["error"]["kind"], serde_json::json!("validation"));
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(
        msg.contains("dod"),
        "points at the create-payload key `dod`: {msg}"
    );
    assert!(
        msg.contains("completion_criterion"),
        "names the mis-used canonical key: {msg}"
    );
}

#[test]
fn create_json_accepts_the_read_form_ordinal_priority_key() {
    // ee2h (reverses the earlier 3bw6 label-only rule): `"2"` is the read-form ordinal that
    // `show`/`list`/`next --json` emit for `priority`. The create payload now ACCEPTS it — so
    // machine output is valid machine input — additively to the `P2` label. It is stored as the
    // ordinal, exactly as the label form is.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let payload = serde_json::json!({
        "type": "bug", "title": "X", "description": "d", "priority": "2",
    })
    .to_string();
    let v: serde_json::Value = serde_json::from_slice(
        &create_via_json(tmp.path(), &payload)
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(
        v["priority"],
        serde_json::json!("2"),
        "the read-form ordinal key round-trips and is stored as the ordinal"
    );
}

#[test]
fn a_payload_built_from_the_schema_advertised_keys_creates_on_the_first_try() {
    // 3bw6 (direction B): the whole point — a payload assembled from EXACTLY what `schema` advertises
    // (the divergent `dod` key + a `P2` label value) succeeds immediately, no translation round-trip.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let schema = schema_json(tmp.path());
    let dod_key = schema_field(&schema, "completion_criterion")["create_json_key"]
        .as_str()
        .unwrap()
        .to_string();
    let prio = schema_field(&schema, "priority")["create_json_values"][2]
        .as_str()
        .unwrap()
        .to_string();
    let payload = serde_json::json!({
        "type": "bug", "title": "X", "description": "d",
        "priority": prio,
        dod_key: "done when the tests pass",
    })
    .to_string();
    let out = create_via_json(tmp.path(), &payload)
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v["completion_criterion"],
        serde_json::json!("done when the tests pass")
    );
    // The `P2` label is stored as its canonical ordinal — the read/write value forms are distinct.
    assert_eq!(v["priority"], serde_json::json!("2"));
}

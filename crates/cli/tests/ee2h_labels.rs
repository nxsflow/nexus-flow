//! ee2h: additive `priority_label`/`type_label` in `show`/`list`/`next --json`, and `create`/
//! `update` accepting the canonical ordinal KEY as well as the label — so machine output is valid
//! machine input (the round-trip was broken: reads emit `"priority":"2"`, writes rejected it).
//!
//! The canonical `priority`/`type` keys and the human views are unchanged; only the `--json` reads
//! gain the additive label fields, and the write surface additionally accepts the key form.

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_NOW", "2026-06-23T00:00:00Z")
        .current_dir(dir);
    c
}

fn json_out(cmd: &mut Command) -> Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).expect("valid json")
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    tmp
}

#[test]
fn create_and_update_accept_the_canonical_ordinal_key_and_the_label() {
    let tmp = workspace();
    let dir = tmp.path();

    // --priority as the canonical ordinal KEY (the form `--json` emits) — previously rejected.
    let by_key = json_out(nxf(dir).args([
        "create",
        "--type",
        "feature",
        "--title",
        "A",
        "--description",
        "d",
        "--priority",
        "2",
        "--json",
    ]));
    assert_eq!(by_key["priority"], "2", "ordinal key stored as the ordinal");

    // --priority as the named LABEL still works (regression).
    let by_label = json_out(nxf(dir).args([
        "create",
        "--type",
        "bug",
        "--title",
        "B",
        "--description",
        "d",
        "--priority",
        "P1",
        "--json",
    ]));
    assert_eq!(by_label["priority"], "1", "label resolves to its ordinal");

    // update --set priority=<key> also accepts the ordinal key.
    let id = by_key["id"].as_str().unwrap().to_string();
    let updated = json_out(nxf(dir).args(["update", &id, "--set", "priority=0", "--json"]));
    assert_eq!(updated["priority"], "0");

    // A create JSON payload carrying the ordinal key round-trips (machine output → machine input).
    let round = json_out(
        nxf(dir)
            .args(["create", "--json", "-"])
            .write_stdin(r#"{"type":"feature","title":"rt","description":"d","priority":"3"}"#),
    );
    assert_eq!(
        round["priority"], "3",
        "ordinal key accepted in the JSON create payload"
    );

    // An out-of-range ordinal and an unknown label are still rejected.
    nxf(dir)
        .args([
            "create",
            "--type",
            "feature",
            "--title",
            "X",
            "--description",
            "d",
            "--priority",
            "9",
            "--json",
        ])
        .assert()
        .failure();
}

#[test]
fn read_json_carries_additive_priority_and_type_labels() {
    let tmp = workspace();
    let dir = tmp.path();
    let created = json_out(nxf(dir).args([
        "create",
        "--type",
        "chore",
        "--title",
        "C",
        "--description",
        "d",
        "--priority",
        "P3",
        "--json",
    ]));
    let id = created["id"].as_str().unwrap().to_string();

    // show --json: the record is nested under `item`; labels sit alongside the canonical fields.
    let show = json_out(nxf(dir).args(["show", &id, "--json"]));
    let item = &show["item"];
    assert_eq!(item["priority"], "3", "canonical ordinal unchanged");
    assert_eq!(
        item["priority_label"], "P3",
        "additive label resolved via the plugin"
    );
    assert_eq!(item["type"], "chore", "canonical type key unchanged");
    assert_eq!(item["type_label"], "chore");
    assert!(
        show.get("priority_label").is_none(),
        "no stray label at the show wrapper's top level"
    );

    // list --json: each element is a flat record carrying both label fields.
    let list = json_out(nxf(dir).args(["list", "--json"]));
    let row = &list.as_array().unwrap()[0];
    assert_eq!(row["priority"], "3");
    assert_eq!(row["priority_label"], "P3");
    assert_eq!(row["type_label"], "chore");

    // next --json: same additive shape on the ranked read.
    let next = json_out(nxf(dir).args(["next", "--json"]));
    let row = &next.as_array().unwrap()[0];
    assert_eq!(row["priority_label"], "P3");
    assert_eq!(row["type_label"], "chore");
}

#[test]
fn blocked_and_search_json_carry_the_additive_labels() {
    // yfwt: the additive priority_label/type_label, ee2h scoped to show/list/next, now also decorate
    // `blocked --json` and `search --json` — the remaining item-emitting reads — for cross-verb
    // agent-ergonomics consistency. Still CLI-only (stripped at the MCP/handle seam by parity).
    let tmp = workspace();
    let dir = tmp.path();
    let a = json_out(nxf(dir).args([
        "create",
        "--type",
        "feature",
        "--title",
        "A",
        "--description",
        "d",
        "--priority",
        "P1",
        "--json",
    ]));
    let a_id = a["id"].as_str().unwrap().to_string();
    json_out(nxf(dir).args([
        "create",
        "--type",
        "chore",
        "--title",
        "B",
        "--description",
        "d",
        "--priority",
        "P2",
        "--depends-on",
        &a_id,
        "--json",
    ]));

    // B depends on the open A ⇒ B is blocked. Its `blocked --json` record now carries the labels.
    let blocked = json_out(nxf(dir).args(["blocked", "--json"]));
    let row = &blocked.as_array().unwrap()[0];
    assert_eq!(row["title"], "B", "B is the blocked item");
    assert_eq!(row["priority"], "2", "canonical ordinal unchanged");
    assert_eq!(
        row["priority_label"], "P2",
        "blocked --json carries priority_label"
    );
    assert_eq!(
        row["type_label"], "chore",
        "blocked --json carries type_label"
    );

    // `search --json` carries them too (A matches the query).
    let search = json_out(nxf(dir).args(["search", "A", "--json"]));
    let hit = search
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "A")
        .expect("A is in the search results");
    assert_eq!(
        hit["priority_label"], "P1",
        "search --json carries priority_label"
    );
    assert_eq!(
        hit["type_label"], "feature",
        "search --json carries type_label"
    );
}

#[test]
fn next_json_parent_join_carries_type_label_only() {
    // yfwt: the nested `parent` reference join on `next --json` gains its own `type_label`. The join
    // is a lightweight {id,title,type} (no priority), so it carries type_label ONLY — never a null
    // priority_label.
    let tmp = workspace();
    let dir = tmp.path();
    let epic = json_out(nxf(dir).args([
        "create",
        "--type",
        "epic",
        "--title",
        "E",
        "--description",
        "d",
        "--priority",
        "P1",
        "--json",
    ]));
    let epic_id = epic["id"].as_str().unwrap().to_string();
    // A child that belongs to the epic and is itself ready ⇒ appears in `next` with a parent join.
    json_out(nxf(dir).args([
        "create",
        "--type",
        "feature",
        "--title",
        "child",
        "--description",
        "d",
        "--priority",
        "P2",
        "--parent",
        &epic_id,
        "--json",
    ]));

    let next = json_out(nxf(dir).args(["next", "--json"]));
    let child = next
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "child")
        .expect("the child is a ready item in next");
    let parent = &child["parent"];
    assert_eq!(parent["type"], "epic", "canonical parent type unchanged");
    assert_eq!(
        parent["type_label"], "epic",
        "parent join carries type_label"
    );
    assert!(
        parent.get("priority_label").is_none(),
        "the parent reference join carries type_label only — no null priority_label: {parent}"
    );
}

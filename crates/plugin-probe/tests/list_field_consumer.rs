//! 4g6r (`6j6v.4g6r`) acceptance — a plugin LIST field through a real consumer plugin: the
//! `file-fixture` declares `[fields.mirrors] type = "text" list = true` (where else a linked file
//! lives, each mirror optionally labelled). Proven in-process, because a subprocess `nxf` cannot
//! see a test-only plugin: the facade read/write layer over an in-memory `Store`, as
//! `ky26_file_consumer.rs` does, and the real `nxf` command tree (`nexus_flow_cli::run_from`) over
//! a workspace on disk whose plugin is the fixture.

use nexus_flow_core::store::Store;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::read;
use nexus_flow_facade::write::{self, NewItem};
use nexus_flow_facade::FieldEntry;
use serde_json::json;
use std::process::ExitCode;

// Link the `plugin-probe` crate so its `file-fixture` is in the facade registry.
extern crate plugin_probe as _;

const NOW: &str = "2026-10-12T09:00:00Z";
const ACTOR: &str = "alice";

fn cfg() -> PluginConfig {
    plugin::load("file-fixture").expect("the file consumer plugin is registered and loads")
}

fn create_file(s: &mut Store, cfg: &PluginConfig, custom: &[String]) -> String {
    write::create(
        s,
        cfg,
        "ab12",
        NOW,
        ACTOR,
        "file",
        "report.md",
        NewItem {
            description: "A generated report.",
            priority: "P1",
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: &[],
            custom,
        },
    )
    .unwrap()
    .id
}

fn uri() -> Vec<String> {
    vec!["uri=.nxs/files/report.md".to_string()]
}

#[test]
fn schema_json_shows_the_list_field() {
    let v = read::schema(&cfg()).to_value();
    let mirrors = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == "mirrors")
        .expect("mirrors appears in schema --json");
    assert_eq!(mirrors["kind"], "text");
    assert_eq!(mirrors["list"], json!(true));
    assert_eq!(mirrors["custom"], json!(true));
    assert_eq!(mirrors["on"], json!(["file"]));
    assert_eq!(mirrors["label"], "Mirrors");
    let uri = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == "uri")
        .unwrap();
    assert!(
        uri.get("list").is_none(),
        "a single-value field carries no `list` key"
    );
}

#[test]
fn entries_are_added_relabelled_removed_and_read_back_on_every_surface() {
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    let id = create_file(&mut s, &cfg, &uri());
    write::entry_add(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        "mirrors",
        "s3://b/r.md",
        Some("backup"),
    )
    .unwrap();
    write::entry_add(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        "mirrors",
        "https://cdn/r.md",
        None,
    )
    .unwrap();
    write::entry_add(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        "mirrors",
        "s3://b/r.md",
        Some("cold"),
    )
    .unwrap();
    let expected = json!([
        { "value": "s3://b/r.md", "label": "cold" },
        { "value": "https://cdn/r.md" }
    ]);
    assert_eq!(
        read::field_entries(&cfg, &s, &id, "mirrors").unwrap(),
        vec![
            FieldEntry {
                value: "s3://b/r.md".into(),
                label: Some("cold".into())
            },
            FieldEntry {
                value: "https://cdn/r.md".into(),
                label: None
            },
        ]
    );
    // show, list, next — the sparse `custom` map, scalar and list side by side.
    let shown = read::show_value_with_custom(&cfg, &s, &id).unwrap();
    assert_eq!(shown["custom"]["mirrors"], expected);
    assert_eq!(shown["custom"]["uri"], ".nxs/files/report.md");
    let items = read::list(&cfg, &s, None, None, None).unwrap();
    let listed = read::list_to_value_with_custom(&cfg, &s, &items).unwrap();
    assert_eq!(listed[0]["custom"]["mirrors"], expected);
    let next = read::next(&cfg, &s, NOW, None).unwrap();
    let next = read::next_to_value_with_custom(&cfg, &s, &next).unwrap();
    assert_eq!(next[0]["custom"]["mirrors"], expected);
    assert_eq!(
        read::declared_custom_by_id(&cfg, &s, &[id.as_str()]).unwrap()[&id]["mirrors"],
        expected
    );

    write::entry_remove(&mut s, &cfg, NOW, ACTOR, &id, "mirrors", "https://cdn/r.md").unwrap();
    assert_eq!(
        read::show_value_with_custom(&cfg, &s, &id).unwrap()["custom"]["mirrors"],
        json!([{ "value": "s3://b/r.md", "label": "cold" }])
    );
}

#[test]
fn set_only_clears_a_list_field_and_entry_writes_validate() {
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    let id = create_file(&mut s, &cfg, &uri());
    write::entry_add(&mut s, &cfg, NOW, ACTOR, &id, "mirrors", "a", None).unwrap();
    let err = write::update(&mut s, &cfg, NOW, ACTOR, &id, &["mirrors=b".into()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("nxf entry add"), "{}", err.msg);
    write::update(&mut s, &cfg, NOW, ACTOR, &id, &["mirrors=".into()]).unwrap();
    assert!(read::field_entries(&cfg, &s, &id, "mirrors")
        .unwrap()
        .is_empty());

    // A value at create is refused too: entries are added once the item exists.
    let mut sets = uri();
    sets.push("mirrors=x".into());
    let err = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "file",
        "x",
        NewItem {
            description: "d",
            priority: "P1",
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: &[],
            custom: &sets,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);

    for (field, value, label, why) in [
        ("mirrors", "   ", None, "a blank value"),
        ("mirrors", "a\u{1f}b", None, "a separator in the value"),
        ("mirrors", "a", Some(" "), "a blank label"),
        ("uri", "a", None, "a single-value field"),
        ("nope", "a", None, "an undeclared field"),
    ] {
        let err =
            write::entry_add(&mut s, &cfg, NOW, ACTOR, &id, field, value, label).expect_err(why);
        assert_eq!(err.kind, ErrorKind::Validation, "{why}");
    }
    let err =
        write::entry_add(&mut s, &cfg, NOW, ACTOR, "ab12.none", "mirrors", "a", None).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn a_sync_round_trip_merges_concurrent_adds_and_removes() {
    // Two replicas of one workspace: A removes a mirror while B, concurrently, re-adds it with a
    // new label and adds another. After the exchange both hold the same list, and B's re-add —
    // which A never saw — survives A's remove.
    let cfg = cfg();
    let mut a = Store::open_in_memory(1);
    let id = create_file(&mut a, &cfg, &uri());
    write::entry_add(&mut a, &cfg, NOW, ACTOR, &id, "mirrors", "m1", None).unwrap();
    let mut b = Store::open_in_memory(2);
    b.apply(&a.export());

    write::entry_remove(&mut a, &cfg, NOW, ACTOR, &id, "mirrors", "m1").unwrap();
    write::entry_add(&mut b, &cfg, NOW, "bob", &id, "mirrors", "m1", Some("hot")).unwrap();
    write::entry_add(&mut b, &cfg, NOW, "bob", &id, "mirrors", "m2", None).unwrap();
    let (from_a, from_b) = (a.export(), b.export());
    a.apply(&from_b);
    b.apply(&from_a);
    let on_a = read::show_value_with_custom(&cfg, &a, &id).unwrap()["custom"]["mirrors"].clone();
    let on_b = read::show_value_with_custom(&cfg, &b, &id).unwrap()["custom"]["mirrors"].clone();
    assert_eq!(on_a, on_b);
    assert_eq!(
        on_a,
        json!([{ "value": "m1", "label": "hot" }, { "value": "m2" }])
    );
}

/// Run the real `nxf` command tree in-process against the workspace whose db is `db`.
fn nxf(db: &str, args: &[&str]) -> ExitCode {
    let mut argv = vec!["nxf", "--db", db];
    argv.extend_from_slice(args);
    nexus_flow_cli::run_from(argv)
}

#[test]
fn the_nxf_entry_verbs_write_through_the_command_line() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = nxs_foundation::workspace::setup(
        tmp.path(),
        &nexus_flow_facade::workspace::flow_config("file-fixture"),
    )
    .unwrap();
    let db = ws.db_path().to_string_lossy().into_owned();
    let ok = ExitCode::SUCCESS;
    assert_eq!(
        nxf(
            &db,
            &[
                "create",
                "--type",
                "file",
                "--title",
                "report.md",
                "--description",
                "d",
                "--priority",
                "P1",
                "--set",
                "uri=.nxs/files/report.md",
            ],
        ),
        ok
    );
    let id = {
        let store = Store::open(&db, 1).unwrap();
        store.list_items().unwrap()[0].id.clone()
    };
    assert_eq!(
        nxf(
            &db,
            &[
                "entry",
                "add",
                &id,
                "mirrors",
                "s3://b/r.md",
                "--label",
                "backup"
            ]
        ),
        ok
    );
    assert_eq!(
        nxf(&db, &["entry", "add", &id, "mirrors", "https://cdn/r.md"]),
        ok
    );
    assert_eq!(nxf(&db, &["--json", "entry", "list", &id, "mirrors"]), ok);
    assert_eq!(
        nxf(&db, &["entry", "remove", &id, "mirrors", "s3://b/r.md"]),
        ok
    );
    assert_ne!(
        nxf(&db, &["update", &id, "--set", "mirrors=x"]),
        ok,
        "--set refuses a value on a list field"
    );
    assert_ne!(
        nxf(&db, &["entry", "add", &id, "uri", "x"]),
        ok,
        "entry add refuses a single-value field"
    );
    let cfg = cfg();
    let store = Store::open(&db, 1).unwrap();
    assert_eq!(
        read::show_value_with_custom(&cfg, &store, &id).unwrap()["custom"]["mirrors"],
        json!([{ "value": "https://cdn/r.md" }])
    );
    drop(store);
    assert_eq!(nxf(&db, &["update", &id, "--set", "mirrors="]), ok);
    let store = Store::open(&db, 1).unwrap();
    assert!(read::field_entries(&cfg, &store, &id, "mirrors")
        .unwrap()
        .is_empty());
}

/// Run `probe-nxf` — the real `nxf` command tree with the fixture linked — as a subprocess in
/// `dir`, `$HOME` pinned inside it, against the workspace whose db is `db`: (success, stdout).
fn probe_nxf(dir: &std::path::Path, db: &str, args: &[&str]) -> (bool, String) {
    use nxs_test_support::PinHome;
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_probe-nxf"))
        .current_dir(dir)
        .pin_home(&home)
        .arg("--db")
        .arg(db)
        .args(args)
        .output()
        .expect("probe-nxf runs");
    (
        out.status.success(),
        String::from_utf8(out.stdout).expect("utf-8 output"),
    )
}

#[test]
fn nxf_entry_list_prints_the_entries_in_order_human_and_json() {
    // Review of PR #63, Test Quality #5: what a successful `nxf entry list` prints, byte for byte,
    // and the receipts of `entry add`/`remove` — through the real command tree in a subprocess, so
    // the assertion is on stdout and not on a function the verb happens to call.
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = nxs_foundation::workspace::setup(
        tmp.path(),
        &nexus_flow_facade::workspace::flow_config("file-fixture"),
    )
    .unwrap();
    let db = ws.db_path().to_string_lossy().into_owned();
    let run = |args: &[&str]| probe_nxf(tmp.path(), &db, args);
    let (ok, _) = run(&[
        "create",
        "--type",
        "file",
        "--title",
        "report.md",
        "--description",
        "d",
        "--priority",
        "P1",
        "--set",
        "uri=.nxs/files/report.md",
    ]);
    assert!(ok, "create");
    let id = {
        let store = Store::open(&db, 1).unwrap();
        store.list_items().unwrap()[0].id.clone()
    };

    // An empty list is `[]` under --json.
    assert_eq!(
        run(&["--json", "entry", "list", &id, "mirrors"]),
        (true, "[]\n".to_string())
    );

    // The receipts carry the entry as stored: trimmed, the label after it.
    assert_eq!(
        run(&[
            "--json",
            "entry",
            "add",
            &id,
            "mirrors",
            "  s3://b/r.md ",
            "--label",
            " backup "
        ]),
        (
            true,
            format!(
                "{}\n",
                json!({ "ok": true, "msg": format!("{id} mirrors + s3://b/r.md (backup)") })
            )
        )
    );
    assert!(run(&["entry", "add", &id, "mirrors", "https://cdn/r.md"]).0);
    assert!(
        run(&[
            "entry",
            "add",
            &id,
            "mirrors",
            "ftp://old/r.md",
            "--label",
            "old"
        ])
        .0
    );
    assert_eq!(
        run(&[
            "--json",
            "entry",
            "remove",
            &id,
            "mirrors",
            " ftp://old/r.md "
        ]),
        (
            true,
            format!(
                "{}\n",
                json!({ "ok": true, "msg": format!("{id} mirrors - ftp://old/r.md") })
            )
        )
    );

    // --json: the `[{value, label?}]` array, in entry order.
    let (ok, out) = run(&["--json", "entry", "list", &id, "mirrors"]);
    assert!(ok);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).unwrap(),
        json!([
            { "value": "s3://b/r.md", "label": "backup" },
            { "value": "https://cdn/r.md" }
        ])
    );
    assert_eq!(
        out.lines().count(),
        1,
        "one line of JSON, unframed: {out:?}"
    );
    // Human: one entry per line, its label after a tab, framed by a blank line before and after.
    assert_eq!(
        run(&["entry", "list", &id, "mirrors"]),
        (
            true,
            "\ns3://b/r.md\tbackup\nhttps://cdn/r.md\n\n".to_string()
        )
    );
}

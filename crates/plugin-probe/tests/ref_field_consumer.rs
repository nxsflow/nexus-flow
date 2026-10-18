//! 9tx3 (`6j6v.9tx3`) acceptance — a plugin REF field through a real consumer plugin: the
//! `file-fixture` declares `[fields.source] type = "ref" targets = ["file"]` (what a linked file
//! was derived from) and `[fields.related] type = "ref" targets = ["file"] list = true`. Proven
//! through the real `nxf` command tree as the `probe-nxf` subprocess (a subprocess `nxf` cannot see
//! a test-only plugin), its `$HOME` pinned, over a workspace on disk whose plugin is the fixture —
//! so every assertion is on what a verb prints or stores, as `list_field_consumer.rs` does.

use nexus_flow_core::store::Store;
use serde_json::{json, Value};

// Link the `plugin-probe` crate so its `file-fixture` is in the facade registry.
extern crate plugin_probe as _;

/// Run `probe-nxf` in `dir`, `$HOME` pinned inside it, against the db `db`: (success, stdout,
/// stderr).
fn probe_nxf(dir: &std::path::Path, db: &str, args: &[&str]) -> (bool, String, String) {
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
        String::from_utf8(out.stderr).expect("utf-8 output"),
    )
}

#[test]
fn a_ref_field_is_set_resolved_validated_shown_and_reported_through_the_command_line() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = nxs_foundation::workspace::setup(
        tmp.path(),
        &nexus_flow_facade::workspace::flow_config("file-fixture"),
    )
    .unwrap();
    let db = ws.db_path().to_string_lossy().into_owned();
    let run = |args: &[&str]| probe_nxf(tmp.path(), &db, args);
    let create = |title: &str| {
        let (ok, out, err) = run(&[
            "create",
            "--type",
            "file",
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            "P1",
            "--set",
            &format!("uri=.nxs/files/{title}"),
            "-q",
        ]);
        assert!(ok, "create {title}: {err}");
        out.trim().to_string()
    };
    let a = create("a.md");
    let b = create("b.md");
    // `-q` prints the id as the CLI shows ids: the bare local suffix.
    let full = |id: &str| -> String {
        let store = Store::open(&db, 1).unwrap();
        store
            .list_items()
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .find(|full| full == id || full.ends_with(&format!(".{id}")))
            .unwrap()
    };
    let (fa, fb) = (full(&a), full(&b));
    let bare_b = fb.split_once('.').unwrap().1;

    // `--set` with a bare local suffix stores the full id: the CLI resolved it before the facade.
    let (ok, _, err) = run(&["update", &a, "--set", &format!("source={bare_b}")]);
    assert!(ok, "{err}");
    let custom = Store::open(&db, 1).unwrap().custom_fields_of(&fa).unwrap();
    assert_eq!(custom["source"], fb);

    // `entry add` with a bare suffix: the receipt carries the stored, full target id.
    let (ok, out, err) = run(&["entry", "add", &a, "related", bare_b, "--label", "mirror"]);
    assert!(ok, "{err}");
    assert_eq!(
        out.trim(),
        format!("{} related + {fb} (mirror)", fa.split_once('.').unwrap().1),
        "the human receipt, inside the CLI's blank-line framing"
    );

    // A self-reference is refused with its message, and writes nothing.
    let (ok, _, err) = run(&["update", &a, "--set", &format!("source={fa}")]);
    assert!(!ok);
    assert!(
        err.contains("custom field 'source': an item cannot reference itself"),
        "{err}"
    );
    // A target that is not on the board is not_found.
    let (ok, _, err) = run(&["update", &a, "--set", "source=zz99.none"]);
    assert!(!ok);
    assert!(err.contains("no item 'zz99.none'"), "{err}");
    assert_eq!(
        Store::open(&db, 1).unwrap().custom_fields_of(&fa).unwrap()["source"],
        fb
    );

    // Human `show`: the field's heading, then the target's id as the CLI shows ids and its title.
    let (ok, out, err) = run(&["show", &a]);
    assert!(ok, "{err}");
    assert!(
        out.contains(&format!("\nSOURCE\n------\n\n{bare_b}  b.md\n")),
        "{out}"
    );
    assert!(
        out.contains(&format!("\nRELATED\n-------\n\n{bare_b}  b.md (mirror)\n")),
        "{out}"
    );

    // `show --json`: the sparse `refs` key resolves each target.
    let (ok, out, err) = run(&["--json", "show", &a]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["refs"]["source"],
        json!([{ "id": fb, "found": true, "type": "file", "title": "b.md", "status": "open" }])
    );
    assert_eq!(v["refs"]["related"][0]["found"], json!(true));
    assert_eq!(v["custom"]["source"], json!(fb));

    // `schema --json` names the kind and the targets.
    let (ok, out, err) = run(&["--json", "schema"]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let field = |name: &str| -> Value {
        v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["field"] == name)
            .cloned()
            .unwrap()
    };
    assert_eq!(field("source")["kind"], "ref");
    assert_eq!(field("source")["targets"], json!(["file"]));
    assert_eq!(field("related")["list"], json!(true));
    assert_eq!(field("related")["targets"], json!(["file"]));

    // `entry remove` takes the bare suffix too.
    let (ok, _, err) = run(&["entry", "remove", &a, "related", bare_b]);
    assert!(ok, "{err}");
    assert!(Store::open(&db, 1)
        .unwrap()
        .field_entries_of(&fa)
        .unwrap()
        .get("related")
        .is_none());

    // A target deleted afterwards (by a peer — `nxf` has no delete verb) shows as not on this
    // board, in both forms.
    Store::open(&db, 1).unwrap().delete_item(&fb, "t");
    let (_, out, _) = run(&["show", &a]);
    assert!(
        out.contains(&format!("{fb}  (not on this board)\n")),
        "{out}"
    );
    let (_, out, _) = run(&["--json", "show", &a]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["refs"]["source"], json!([{ "id": fb, "found": false }]));
}

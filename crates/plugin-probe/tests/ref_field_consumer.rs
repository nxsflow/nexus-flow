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
    assert!(!Store::open(&db, 1)
        .unwrap()
        .field_entries_of(&fa)
        .unwrap()
        .contains_key("related"));

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

#[test]
fn list_ref_lists_the_referrers_and_resolves_a_bare_id() {
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
    let (a, b, c, d) = (
        create("a.md"),
        create("b.md"),
        create("c.md"),
        create("d.md"),
    );
    let (ok, _, err) = run(&["update", &a, "--set", &format!("source={b}")]);
    assert!(ok, "{err}");
    let (ok, _, err) = run(&["entry", "add", &d, "related", &c]);
    assert!(ok, "{err}");
    let listed = |args: &[&str]| -> Vec<String> {
        let mut argv = vec!["list", "--json"];
        argv.extend_from_slice(args);
        let (ok, out, err) = run(&argv);
        assert!(ok, "{argv:?}: {err}");
        let v: Value = serde_json::from_str(&out).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let id = r["id"].as_str().unwrap();
                id.split_once('.').unwrap().1.to_string()
            })
            .collect()
    };

    // A bare local suffix is resolved to the full id; the referrer is listed, its target not.
    let source_b = format!("source={b}");
    assert_eq!(listed(&["--ref", &source_b]), std::slice::from_ref(&a));
    // Two refs are alternatives: the union of both lookups.
    let related_c = format!("related={c}");
    let mut union = vec![a.clone(), d.clone()];
    union.sort();
    assert_eq!(listed(&["--ref", &source_b, "--ref", &related_c]), union);
    // Composed with `--limit`: the envelope of the filtered list.
    let (ok, out, err) = run(&[
        "list", "--json", "--ref", &source_b, "--ref", &related_c, "--limit", "1",
    ]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["total"], json!(2));
    // An id nothing holds is an empty list.
    assert!(listed(&["--ref", "source=zz99.none"]).is_empty());

    let refused = |args: &[&str]| -> String {
        let mut argv = vec!["list"];
        argv.extend_from_slice(args);
        let (ok, _, err) = run(&argv);
        assert!(!ok, "{argv:?} must be refused");
        err
    };
    let err = refused(&["--ref", &format!("uri={b}")]);
    assert!(
        err.contains(
            "`--ref` names 'uri', which is not a ref field of this plugin; ref fields: related, \
             source"
        ),
        "{err}"
    );
    for malformed in ["novalue", "source=", "=x"] {
        let err = refused(&["--ref", malformed]);
        assert!(err.contains("`--ref` takes FIELD=ID"), "{malformed}: {err}");
    }
    // A bare value is resolved to a local id first; one with whitespace is no id even then.
    let err = refused(&["--ref", "source=two words"]);
    assert!(
        err.contains("`--ref source`: a ref is a full item id '<prefix>.<suffix>' (got"),
        "{err}"
    );
    let nine: Vec<String> = (0..9).map(|i| format!("source=zz99.n{i}")).collect();
    let mut argv: Vec<&str> = Vec::new();
    for r in &nine {
        argv.extend(["--ref", r.as_str()]);
    }
    let err = refused(&argv);
    assert!(err.contains("`--ref` takes at most 8 filters"), "{err}");
    let eight = &argv[..16];
    assert!(listed(eight).is_empty(), "exactly eight refs are accepted");
}

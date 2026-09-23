//! ky26 (`6j6v.ky26`) acceptance — the FIRST real user of the plugin custom-fields epic (rvay,
//! T1-T4): the `file` consumer plugin declares a `file` type with a required custom `uri` field over
//! the `.nxs/files/` convention, proven end-to-end ("erste echte Plugin-Custom-Field-Nutzung").
//!
//! In-process, driving the REAL facade code paths — `plugin::load` (registry, the same path the
//! CLI/MCP use), `write::create` (validation + the folded `field set` op), `read::schema`, and the
//! sparse `custom` read-layer join (`show`/`list`/`next --json`) — over an in-memory `Store`, mirror-
//! ing `tests/linked.rs`. A subprocess `nxf` can't see a test-only plugin, so the acceptance is here.
//!
//! The `file`-fixture is registered from the `plugin-probe` crate (see `src/lib.rs`); linking it (no
//! function called) makes it visible through the facade registry — exactly the consumer-plugin path.
//!
//! `.nxs/files/` CONVENTION + V1 BLOB BOUNDARY: a `file` item's `uri` is a WORKSPACE-RELATIVE path
//! under `<workspace>/.nxs/files/`. The op-log syncs the file ITEMS (metadata: `uri`/title/desc) —
//! the "Metadaten syncen" guarantee the sync round-trip below pins — but NOT the blobs themselves
//! (a later engine theme, E4-neighbourhood). See `src/file-fixture.toml` for the full note.

use nexus_flow_core::store::Store;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::read::{self, SchemaField};
use nexus_flow_facade::write::{self, NewItem};

// Link the `plugin-probe` crate so its `inventory::submit!`ed `file-fixture` is present in the
// facade registry — the consumer never NAMES the plugin crate in code, only links it.
extern crate plugin_probe as _;

const NOW: &str = "2026-07-12T00:00:00Z";
const ACTOR: &str = "alice";
const URI: &str = ".nxs/files/report.md";

fn cfg() -> PluginConfig {
    // The same registry-driven load the CLI/MCP use — proof the linked consumer plugin is loadable.
    plugin::load("file-fixture").expect("the file consumer plugin is registered and loads")
}

/// Create a `file` item with the given custom `--set` assignments, returning the write result.
fn create_file(
    s: &mut Store,
    cfg: &PluginConfig,
    title: &str,
    custom: &[String],
) -> nexus_flow_facade::error::Result<nexus_flow_core::model::ItemRow> {
    write::create(
        s,
        cfg,
        "ab12",
        NOW,
        ACTOR,
        "file",
        title,
        NewItem {
            description: "A short summary of the linked file.",
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
}

#[test]
fn file_fixture_is_registered_and_declares_the_uri_custom_field() {
    // The consumer plugin is visible through the SAME registry the CLI/MCP read, alongside the OSS
    // plugins (no clobbering), and declares exactly the `file`-scoped required `uri` text field.
    let names = plugin::plugin_names();
    assert!(
        names.contains(&"file-fixture"),
        "the linked consumer plugin appears in the registry; got {names:?}"
    );
    assert!(names.contains(&"issue-tracker") && names.contains(&"personal-todo"));

    let cfg = cfg();
    assert!(cfg.types.list.contains("file"), "declares the `file` type");
    let uri = cfg
        .fields
        .get("uri")
        .expect("declares a `uri` custom field");
    assert!(uri.required, "`uri` is required");
    assert_eq!(uri.on, ["file"], "`uri` is scoped to the `file` type");
    assert_eq!(uri.label.as_deref(), Some("File URI"));
}

#[test]
fn create_file_with_uri_validates_stores_and_reads_back() {
    // declare + create + write: `create --type file --set uri=.nxs/files/report.md` VALIDATES and
    // folds — title is the filename, description a summary, uri the workspace-relative path.
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    let sets = vec![format!("uri={URI}")];
    let item = create_file(&mut s, &cfg, "report.md", &sets).expect("a valid file create succeeds");

    assert_eq!(item.item_type.as_deref(), Some("file"));
    assert_eq!(item.title.as_deref(), Some("report.md"));
    assert_eq!(item.status.as_deref(), Some("open"));

    // The custom value folded into the `custom_fields` view under (item_id, "uri").
    let stored = s.custom_fields_of(&item.id).unwrap();
    assert_eq!(
        stored.get("uri").map(String::as_str),
        Some(URI),
        "the uri custom value is stored on the item"
    );
}

#[test]
fn creating_a_file_without_uri_is_rejected_naming_uri() {
    // required guard (TB-CF-7): the required `uri` is enforced at create time — omitting it is a loud
    // validation error that NAMES the missing field, and nothing is written.
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    let err =
        create_file(&mut s, &cfg, "report.md", &[]).expect_err("a file without uri is rejected");
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("uri"),
        "the rejection names the missing `uri` field: {}",
        err.msg
    );
}

#[test]
fn schema_marks_uri_as_a_required_file_scoped_custom_text_field() {
    // schema: `read::schema(&cfg)` (the `nxf schema --json` / `flow_schema` source) lists `uri` marked
    // custom:true, on:["file"], required:true, kind:"text".
    let cfg = cfg();
    let report = read::schema(&cfg);
    let uri: &SchemaField = report
        .fields
        .iter()
        .find(|f| f.field == "uri")
        .expect("schema lists the uri custom field");
    assert!(uri.custom, "uri is a custom field");
    assert!(uri.required, "uri is required");
    assert_eq!(uri.kind, "text", "uri is a text field");
    assert_eq!(uri.on, ["file"], "uri is scoped to `file`");

    // And in the canonical `--json` projection an agent actually reads.
    let v = report.to_value();
    let uri_json = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == "uri")
        .expect("uri appears in schema --json");
    assert_eq!(uri_json["custom"], serde_json::json!(true));
    assert_eq!(uri_json["on"], serde_json::json!(["file"]));
    assert_eq!(uri_json["required"], serde_json::json!(true));
    assert_eq!(uri_json["kind"], "text");
}

#[test]
fn read_surfaces_the_sparse_custom_uri_map_on_show_and_lane() {
    // read: `show --json` and a lane read (`list`/`next --json`) surface the sparse `custom` map
    // { "uri": ".nxs/files/report.md" } — the declared-and-non-empty projection.
    let cfg = cfg();
    let mut s = Store::open_in_memory(1);
    let sets = vec![format!("uri={URI}")];
    let item = create_file(&mut s, &cfg, "report.md", &sets).unwrap();
    let expected = serde_json::json!({ "uri": URI });

    // show
    let sv = read::show_value_with_custom(&cfg, &s, &item.id).unwrap();
    assert_eq!(sv["custom"], expected, "show carries the custom uri map");

    // list lane
    let items = read::list(&cfg, &s, None, None, None).unwrap();
    let lv = read::list_to_value_with_custom(&cfg, &s, &items).unwrap();
    assert_eq!(lv[0]["custom"], expected, "list carries the custom uri map");

    // next lane
    let ready = read::next(&cfg, &s, NOW, None).unwrap();
    let nv = read::next_to_value_with_custom(&cfg, &s, &ready).unwrap();
    assert_eq!(nv[0]["custom"], expected, "next carries the custom uri map");
}

#[test]
fn sync_round_trip_converges_the_uri_custom_value_on_a_second_replica() {
    // THE LOAD-BEARING ACCEPTANCE ("Metadaten syncen"): replica A creates the file item + sets `uri`,
    // exports its op-log; replica B applies it and, folding the same `field set` op, its read surfaces
    // the SAME `uri` custom value. The custom value survives export→apply on a second replica — the v1
    // guarantee for the file ITEM metadata (the blob itself is out of scope, see the module note).
    let cfg = cfg();

    // Replica A: create + set uri.
    let mut a = Store::open_in_memory(1);
    let sets = vec![format!("uri={URI}")];
    let item = create_file(&mut a, &cfg, "report.md", &sets).unwrap();
    let id = item.id.clone();

    // Replica B starts empty and knows nothing of the file item.
    let mut b = Store::open_in_memory(2);
    assert!(
        b.get_item(&id).unwrap().is_none(),
        "replica B has not seen the file item yet"
    );

    // Sync: A exports its full op-log, B applies it (the relay round-trip).
    b.apply(&a.export());

    // B converged: the file item exists AND its declared `uri` custom value is the same.
    assert_eq!(
        b.get_item(&id).unwrap().unwrap().item_type.as_deref(),
        Some("file"),
        "the file item converged on replica B"
    );
    assert_eq!(
        b.custom_fields_of(&id)
            .unwrap()
            .get("uri")
            .map(String::as_str),
        Some(URI),
        "the uri custom value folded into replica B's view"
    );
    let bv = read::show_value_with_custom(&cfg, &b, &id).unwrap();
    assert_eq!(
        bv["custom"],
        serde_json::json!({ "uri": URI }),
        "replica B's read surfaces the SAME uri custom value — the metadata synced"
    );
}

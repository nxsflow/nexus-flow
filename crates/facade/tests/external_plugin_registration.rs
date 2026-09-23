//! r16h.1 seam proof: a crate OTHER than the facade submits a `PluginRegistration` via `inventory`,
//! and the facade's registry-driven `plugin_names`/`available`/`load` pick it up — exactly how a
//! consumer's proprietary plugin (defined in its own crate, its TOML in its own tree) becomes known
//! to a binary that links it, with no OSS source change. Its own test binary, so the fake does not
//! pollute the facade lib tests' "exactly two OSS plugins" assertion.

use nexus_flow_facade::plugin::{self, PluginRegistration};

inventory::submit!(PluginRegistration {
    name: "external-fixture",
    // A minimal but valid plugin TOML — enough for load()'s parse + validation to pass.
    toml: r#"
name = "external-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P1"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [ { field = "id", dir = "asc" } ]
[presentation.list]
columns = ["id"]
[types]
list = ["context", "note"]
max_depth = 2
[types.rules.note]
parents = ["context"]
max = 1
"#,
    order: 100,
});

#[test]
fn an_externally_registered_plugin_is_visible_and_loadable() {
    // Listed (after the two OSS plugins — order 100 > 10/20).
    assert!(plugin::plugin_names().contains(&"external-fixture"));
    // Loadable through the same path the CLI/MCP use.
    let cfg = plugin::load("external-fixture").expect("external plugin loads");
    assert_eq!(cfg.name, "external-fixture");
    // Enumerated by available() (the init chooser's source).
    assert!(plugin::available()
        .expect("all registered plugins load")
        .iter()
        .any(|c| c.name == "external-fixture"));
    // The OSS plugins are still present alongside it.
    assert!(plugin::plugin_names().contains(&"issue-tracker"));
}

#[test]
fn an_externally_registered_container_type_is_exposed_by_schema() {
    // r16h.1 + y5j8: a consumer-registered plugin that declares a `context` container surfaces that
    // role through read::schema — exactly the discovery an MCP agent on the consumer board relies on.
    let cfg = plugin::load("external-fixture").expect("external plugin loads");
    let v = nexus_flow_facade::read::schema(&cfg).to_value();
    assert_eq!(
        v["hierarchy"]["types"]["context"]["container"],
        serde_json::json!(true)
    );
    assert_eq!(
        v["hierarchy"]["types"]["note"]["parents"],
        serde_json::json!(["context"])
    );
    assert_eq!(
        v["hierarchy"]["types"]["note"]["cardinality"],
        serde_json::json!("single")
    );
}

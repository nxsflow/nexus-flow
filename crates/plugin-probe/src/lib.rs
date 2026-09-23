//! A test-only probe crate that registers ONE plugin from its OWN crate — the realistic
//! consumer-plugin shape (r16h.1): a proprietary plugin `inventory::submit!`s a
//! [`PluginRegistration`](nexus_flow_facade::plugin::PluginRegistration) carrying its name + its own
//! embedded TOML, and appears in the facade registry purely by being linked. The facade's own test
//! binary always links its own submissions, so it cannot prove the CROSS-crate case; this crate's
//! integration test (`tests/linked.rs`) does, under both debug and `--release` (so a linker
//! `--gc-sections`/LTO drop of the inventory ctor would fail it).

inventory::submit! {
    nexus_flow_facade::plugin::PluginRegistration {
        name: "probe-fixture",
        toml: include_str!("probe-fixture.toml"),
        order: 100,
    }
}

// The `file` consumer plugin (ky26): the FIRST real user of the plugin custom-fields epic — a `file`
// type with a required custom `uri` field over the `.nxs/files/` convention. Registered from this
// same test-only crate (distinct `name` + `order` so it can't collide with `probe-fixture`), it is
// exercised end-to-end by `tests/ky26_file_consumer.rs`. Not linked into the shipped `nxf`, so it
// adds no plugin to the real binary — exactly like `probe-fixture`.
inventory::submit! {
    nexus_flow_facade::plugin::PluginRegistration {
        name: "file-fixture",
        toml: include_str!("file-fixture.toml"),
        order: 200,
    }
}

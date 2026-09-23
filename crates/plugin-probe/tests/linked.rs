//! Proves a plugin registered from a SEPARATE crate (this `plugin-probe` crate) is visible through
//! the facade registry once linked into a binary — the realistic external-plugin path (r16h.1),
//! which the facade's own test binary (which always links its own submissions) cannot exercise.
//! Runs under `cargo test` (debug) AND `cargo test --release` in CI, so a linker
//! `--gc-sections`/LTO that dropped the inventory ctor for this otherwise-unreferenced crate would
//! turn this red.

use nexus_flow_facade::plugin;

// Link this crate's `inventory::submit!` WITHOUT calling any of its functions — exactly how a
// consumer binary would depend on a proprietary plugin crate it never names in code.
extern crate plugin_probe as _;

#[test]
fn a_plugin_registered_from_a_separate_crate_is_visible_and_loadable() {
    let names = plugin::plugin_names();
    assert!(
        names.contains(&"probe-fixture"),
        "a plugin registered from a separate linked crate must appear in the registry; got {names:?}"
    );
    // Loadable through the same path the CLI/MCP use.
    let cfg = plugin::load("probe-fixture").expect("the externally-registered plugin loads");
    assert_eq!(cfg.name, "probe-fixture");
    // The OSS-bundled plugins are still present alongside it (no clobbering, deterministic order).
    assert!(names.contains(&"issue-tracker"));
    assert!(names.contains(&"personal-todo"));
}

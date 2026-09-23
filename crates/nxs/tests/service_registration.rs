//! The service-registration seam from the outside (6j6v.5zst): `nxs sync unregister` is the
//! counterpart `bind`'s registration never had, and the entry most worth removing is one whose
//! directory is already gone.
//!
//! Black-box, under a controlled `$HOME`, so the real `~/.nexusflow/workspaces.toml` is never
//! touched — the very leak the library seam this verb sits on was built to make impossible.

use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// Run `nxs sync unregister …` with `$HOME` pointed at `home`, returning parsed `--json` stdout.
fn unregister(home: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = Command::new(cargo_bin("nxs"))
        .args(["--json", "sync", "unregister"])
        .args(args)
        .pin_home(home.path())
        .output()
        .expect("nxs runs");
    assert!(
        out.status.success(),
        "unregister failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("--json emits one JSON object")
}

fn registry(home: &TempDir) -> String {
    std::fs::read_to_string(home.path().join(".nexusflow").join("workspaces.toml"))
        .expect("the registry exists")
}

fn seed_registry(home: &TempDir, body: &str) {
    let dir = home.path().join(".nexusflow");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workspaces.toml"), body).unwrap();
}

#[test]
fn unregister_prunes_a_dead_entry_and_leaves_its_siblings_alone() {
    let home = TempDir::new().unwrap();
    seed_registry(
        &home,
        "[[workspace]]\nname = \"gone\"\npath = \"/deleted/long/ago\"\n\
         [[workspace]]\nname = \"live\"\npath = \"/still/here\"\n",
    );

    let got = unregister(&home, &["/deleted/long/ago"]);
    assert_eq!(got["ok"], true);
    assert_eq!(got["removed"], true);
    assert_eq!(got["path"], "/deleted/long/ago");

    let after = registry(&home);
    assert!(
        !after.contains("/deleted/long/ago"),
        "the dead entry is gone: {after}"
    );
    assert!(
        after.contains("/still/here"),
        "the live sibling survives: {after}"
    );
}

#[test]
fn unregistering_twice_is_a_successful_no_op_the_second_time() {
    let home = TempDir::new().unwrap();
    seed_registry(&home, "[[workspace]]\nname = \"p\"\npath = \"/proj/p\"\n");

    assert_eq!(unregister(&home, &["/proj/p"])["removed"], true);
    let after_first = registry(&home);

    let second = unregister(&home, &["/proj/p"]);
    assert_eq!(
        second["removed"], false,
        "a script that unregisters on shutdown must not fail because it already did"
    );
    assert_eq!(second["ok"], true);
    assert_eq!(
        registry(&home),
        after_first,
        "a no-op must not rewrite the file"
    );
}

#[test]
fn unregister_never_reaches_a_registry_outside_the_home_it_was_given() {
    // The guard the whole seam exists for: this test names a path that does not exist anywhere,
    // under a $HOME with no registry at all. It must neither create one nor error.
    let home = TempDir::new().unwrap();
    let got = unregister(&home, &["/nowhere/at/all"]);
    assert_eq!(got["removed"], false);
    assert!(
        !home
            .path()
            .join(".nexusflow")
            .join("workspaces.toml")
            .exists(),
        "a deregister that found nothing must not CREATE the registry"
    );
}

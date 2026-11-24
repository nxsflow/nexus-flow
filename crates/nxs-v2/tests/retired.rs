//! The retired v1 entry points (RFC 0008, "The installed base"): v1 installs `nxf`, `nxm` and
//! `nxc` as symlinks to `nxs`, so once v2's `nxs` replaces v1's they start v2's binary. Started
//! under one of those names, `nxs` says that the command is retired in v2, names its `nxs`
//! counterpart and exits with the error `retired_entry_point`, whatever the arguments.

mod common;

use std::path::Path;

use common::{json, scratch, stdout};
use nxs_v2::Outcome;

/// Runs the binary as `argv0 <args>` in `base/repo`, with the data directory `base/data`.
fn run_as(base: &Path, argv0: &str, args: &[&str]) -> Outcome {
    let data = base.join("data");
    let argv = std::iter::once(argv0).chain(args.iter().copied());
    nxs_v2::run(argv, &base.join("repo"), || Ok(common::options(&data)))
}

/// The error object's message, after checking that the run printed `retired_entry_point`.
fn retired(out: &Outcome) -> String {
    assert_eq!(out.code, 1, "{}", stdout(out));
    assert!(out.stderr.is_empty(), "{:?}", String::from_utf8_lossy(&out.stderr));
    let error = json(out);
    assert_eq!(error["error"]["kind"], "retired_entry_point", "{error}");
    error["error"]["msg"].as_str().unwrap().to_owned()
}

#[test]
fn nxf_is_retired_and_names_nxs_ticket() {
    let base = scratch("retired-nxf");
    let msg = retired(&run_as(&base, "nxf", &["list"]));
    assert!(msg.contains("nxf is retired in v2"), "{msg}");
    assert!(msg.contains("nxs ticket"), "{msg}");
}

#[test]
fn nxm_is_retired_and_names_nxs_decision() {
    let base = scratch("retired-nxm");
    let msg = retired(&run_as(&base, "nxm", &["list"]));
    assert!(msg.contains("nxm is retired in v2"), "{msg}");
    assert!(msg.contains("nxs decision"), "{msg}");
}

#[test]
fn nxc_is_retired_and_names_nxs_factory() {
    let base = scratch("retired-nxc");
    let msg = retired(&run_as(&base, "nxc", &["channel", "list"]));
    assert!(msg.contains("nxc is retired in v2"), "{msg}");
    assert!(msg.contains("nxs factory"), "{msg}");
}

#[test]
fn the_name_is_read_from_the_path_the_symlink_was_started_by() {
    let base = scratch("retired-path");
    let msg = retired(&run_as(&base, "/usr/local/bin/nxf", &[]));
    assert!(msg.contains("nxs ticket"), "{msg}");
}

#[test]
fn a_retired_name_runs_no_command_and_writes_nothing() {
    let base = scratch("retired-init");
    retired(&run_as(&base, "nxf", &["init"]));
    assert!(!base.join("repo/nexus-flow.toml").exists());
    assert!(!base.join("data").exists());
}

#[test]
fn help_and_version_under_a_retired_name_are_refused_too() {
    let base = scratch("retired-help");
    retired(&run_as(&base, "nxm", &["--help"]));
    retired(&run_as(&base, "nxc", &["--version"]));
}

#[test]
fn with_md_the_refusal_goes_to_stderr() {
    let base = scratch("retired-md");
    let out = run_as(&base, "nxf", &["list", "--md"]);
    assert_eq!(out.code, 1);
    assert!(out.stdout.is_empty(), "{}", stdout(&out));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.starts_with("error: nxf is retired in v2"), "{err}");
    assert!(err.contains("nxs ticket"), "{err}");
}

/// v1's install.sh links the three names to `nxs`: the binary started through such a link is
/// refused, before it opens the key store.
#[cfg(unix)]
#[test]
fn the_binary_started_through_a_symlink_is_refused() {
    let base = scratch("retired-binary");
    let link = base.join("nxf");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_nxs-v2"), &link).unwrap();
    let out = std::process::Command::new(&link)
        .arg("list")
        .current_dir(base.join("repo"))
        .env("NXS_DATA_DIR", base.join("data"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(error["error"]["kind"], "retired_entry_point", "{error}");
}

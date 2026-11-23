//! Errors (RFC 0001, "Reading and writing"): v1's error object, `{"error":{"kind","msg"}}`, on
//! stdout with exit code 1, where `kind` is a stable code an agent matches on and `msg` is for a
//! person. With `--md` the message goes to stderr as `error: <msg>`, as v1's human output does.

mod common;

use std::fs;
use std::process::Command;

use common::{json, nxs, scratch, stdout};

/// Asserts that `out` is v1's error object with `kind`, and nothing else, and returns its message.
fn error_object(out: &nxs_v2::Outcome, kind: &str) -> String {
    assert_eq!(out.code, 1, "{}", stdout(out));
    assert!(out.stderr.is_empty(), "nothing on stderr");
    let value = json(out);
    let object = value.as_object().expect("an object");
    assert_eq!(object.keys().collect::<Vec<_>>(), ["error"], "{value}");
    let error = value["error"].as_object().expect("error is an object");
    assert_eq!(error.keys().collect::<Vec<_>>(), ["kind", "msg"], "{value}");
    assert_eq!(value["error"]["kind"], kind, "{value}");
    value["error"]["msg"].as_str().expect("msg").to_owned()
}

#[test]
fn a_file_of_another_format_is_a_validation_error() {
    let base = scratch("err-format");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), "format = 2\n").unwrap();

    let msg = error_object(&nxs(&base, &repo, &["init"]), "validation");

    assert!(msg.contains("format 2"), "{msg}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_data_directory_inside_the_repository_is_a_validation_error() {
    let base = scratch("err-inside");
    let repo = base.join("repo");
    let data = repo.join("data");
    let argv = ["nxs", "init"];

    let out = nxs_v2::run(argv, &repo, || Ok(common::options(&data)));

    let msg = error_object(&out, "validation");
    assert!(msg.contains("NXS_DATA_DIR"), "{msg}");
    assert!(
        !repo.join("nexus-flow.toml").exists(),
        "nothing was written"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_unknown_command_is_a_validation_error() {
    let base = scratch("err-usage");

    error_object(
        &nxs(&base, &base.join("repo"), &["frobnicate"]),
        "validation",
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn with_md_the_message_goes_to_stderr() {
    let base = scratch("err-md");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), "format = 2\n").unwrap();

    let out = nxs(&base, &repo, &["init", "--md"]);

    assert_eq!(out.code, 1);
    assert!(out.stdout.is_empty(), "nothing on stdout: {}", stdout(&out));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.starts_with("error: "), "{stderr}");
    assert!(stderr.contains("format 2"), "{stderr}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_same_error_gives_the_same_bytes() {
    let base = scratch("err-same");
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), "format = 2\n").unwrap();

    let first = nxs(&base, &repo, &["init"]);
    let second = nxs(&base, &repo, &["init"]);

    assert_eq!(first.stdout, second.stdout);
    fs::remove_dir_all(&base).unwrap();
}

/// The binary itself: a relative `NXS_DATA_DIR` fails before any key store is touched.
#[test]
fn the_binary_prints_the_error_object_and_exits_1() {
    let base = scratch("err-binary");

    let out = Command::new(env!("CARGO_BIN_EXE_nxs-v2"))
        .arg("init")
        .current_dir(base.join("repo"))
        .env("NXS_DATA_DIR", "relative/data")
        .output()
        .expect("run nxs-v2");

    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON on stdout");
    assert_eq!(value["error"]["kind"], "validation", "{value}");
    assert!(!base.join("repo").join("nexus-flow.toml").exists());
    fs::remove_dir_all(&base).unwrap();
}

/// NEX-224: on a machine with no key store, `nxs init` refuses and says so, and creates nothing.
/// Linux only: pointing D-Bus at nothing leaves the Secret Service unreachable, while macOS and
/// Windows always have a key store.
#[cfg(target_os = "linux")]
#[test]
fn init_on_a_machine_with_no_key_store_refuses() {
    let base = scratch("err-no-key-store");
    let repo = base.join("repo");

    let out = Command::new(env!("CARGO_BIN_EXE_nxs-v2"))
        .arg("init")
        .current_dir(&repo)
        .env("NXS_DATA_DIR", base.join("data"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/nxs-v2-test-bus",
        )
        .output()
        .expect("run nxs-v2");

    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON on stdout");
    assert_eq!(value["error"]["kind"], "no_key_store", "{value}");
    let msg = value["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("no key store"), "{msg}");
    assert!(!msg.contains("key_file"), "no key-file advice: {msg}");
    assert!(
        !repo.join("nexus-flow.toml").exists(),
        "nothing was written"
    );
    fs::remove_dir_all(&base).unwrap();
}

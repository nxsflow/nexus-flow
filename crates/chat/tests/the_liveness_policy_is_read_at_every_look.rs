//! **The liveness policy at the binary path** (nxf 6j6v.t0jn; design 6j6v.pszd, 6.1).
//!
//! A look is one `nxs chat tick --thread <t>` process the service starts (`Job::argv`), and that
//! process reads the workspace's `[chat.liveness]` from `.nxs/config.toml` when it resolves its
//! workspace. What the read found wrong rides that look's receipt: an invalid value runs the
//! default and is reported, never a reason to skip the look. The in-process ladder tests
//! (`the_ladder.rs`) drive the behaviour; this file proves the REAL process reads the file and puts
//! the warning on the `--json` receipt — and reads it again at the next look, with no restart.
//!
//! Every child runs with the enclosing session's `NXC_*` scrubbed (6j6v.mf1h): an agent running
//! this suite must not hand its own `NXC_DB` or `NXC_SESSION` to the process under test.

use assert_cmd::Command;
use nexus_chat::workspace::{chat_config, setup};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-11-01T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let dir = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("coder.yaml"),
        "handle: coder\nsystem_prompt: You are coder.\n",
    )
    .unwrap();
    tmp
}

/// Replaces the workspace's `[chat.liveness]` with `section` (it is always the file's last table).
fn set_liveness(root: &Path, section: &str) {
    let path = root.join(".nxs").join("config.toml");
    let text = std::fs::read_to_string(&path).expect("config.toml");
    let kept = match text.find("[chat.liveness]") {
        Some(i) => text[..i].to_string(),
        None => text,
    };
    std::fs::write(&path, format!("{kept}\n[chat.liveness]\n{section}\n")).expect("write");
}

/// `nxs <args>` in `root`, with every `NXC_*` of the enclosing process scrubbed first.
fn nxs(root: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxs");
    for (k, _) in std::env::vars() {
        if k.starts_with("NXC_") {
            c.env_remove(k);
        }
    }
    c.current_dir(root)
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", root.join("dry.log"));
    c
}

fn a_thread(root: &Path) -> String {
    let out = nxs(root)
        .args([
            "chat",
            "send",
            "--to",
            "coder",
            "--json",
            "do the thing",
            "--no-ref",
        ])
        .assert()
        .success();
    let receipt: Value = serde_json::from_slice(&out.get_output().stdout).expect("send receipt");
    receipt["thread_id"]
        .as_str()
        .unwrap_or_else(|| panic!("a thread id: {receipt}"))
        .to_string()
}

/// One look as the service runs it, as the parsed `--json` receipt.
fn look(root: &Path, thread: &str) -> Value {
    let out = nxs(root)
        .args(["chat", "tick", "--thread", thread, "--json"])
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).expect("a tick receipt is valid json")
}

fn invalid_settings(receipt: &Value) -> Vec<String> {
    receipt["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("warnings is always present: {receipt}"))
        .iter()
        .filter(|w| w["class"] == "invalid_setting")
        .map(|w| w["detail"].as_str().expect("detail").to_string())
        .collect()
}

#[test]
fn an_invalid_dead_resumes_is_named_on_the_tick_receipt_of_the_real_process() {
    let tmp = workspace();
    let thread = a_thread(tmp.path());
    set_liveness(tmp.path(), "dead_resumes = 5");
    let receipt = look(tmp.path(), &thread);
    let found = invalid_settings(&receipt);
    assert_eq!(found.len(), 1, "{receipt}");
    assert!(
        found[0].contains(".nxs/config.toml")
            && found[0].contains("dead_resumes")
            && found[0].contains('5')
            && found[0].contains("1 to 3"),
        "{}",
        found[0]
    );
}

#[test]
fn the_next_look_reads_the_file_again_with_no_restart() {
    let tmp = workspace();
    let thread = a_thread(tmp.path());
    let before = look(tmp.path(), &thread);
    assert!(
        invalid_settings(&before).is_empty(),
        "no section is B: {before}"
    );
    set_liveness(
        tmp.path(),
        "dead_resumes = 3\nnotice_at_first_stop = \"yes\"",
    );
    let bad = look(tmp.path(), &thread);
    let found = invalid_settings(&bad);
    assert_eq!(found.len(), 1, "{bad}");
    assert!(found[0].contains("notice_at_first_stop"), "{}", found[0]);
    set_liveness(tmp.path(), "dead_resumes = 3\nnotice_at_first_stop = true");
    let fixed = look(tmp.path(), &thread);
    assert!(
        invalid_settings(&fixed).is_empty(),
        "fixed in the file: {fixed}"
    );
}

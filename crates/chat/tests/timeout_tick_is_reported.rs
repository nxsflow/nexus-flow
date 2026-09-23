//! A `timeout:` that will never fire has to SAY so — nxf 6j6v.p3sm.
//!
//! `nxc guide channels` describes a declared `timeout:` as the fuse that releases a round: "settled"
//! means complete || stale, so a step that answered and a step whose window ran out both release the
//! next one. Without the one-shot job that makes a board `stale`, a member that goes quiet holds the
//! round FOREVER — which is exactly the case the window was declared for.
//!
//! In the proving ground (4jgn.g90w) that job was never scheduled once, and the caller could not see
//! it: the failure went to stderr and the `--json` receipt said `warnings: []`. Worse, on macOS there
//! was usually nothing to see at all — `atrun` ships disabled, so `at` ACCEPTS the job, exits 0 and
//! prints a job id while nothing ever runs it. A successful `schedule()` proved nothing.
//!
//! Two things are held here, on the receipt a caller actually reads:
//!
//! * the FIRST arming, on `nxc send --to <channel>` — the call that opens the board;
//! * that it stays quiet when the tick really was scheduled, or the field is noise.
//!
//! The re-arm on `reply`/`tick` reports through the same class from the same constructor
//! (`rearm_after_the_members_moved`), and those two receipts have carried a `warnings` array all
//! along.
//!
//! **The assertions are on the CLASS, never on the detail text.** How the timer fails is
//! platform-dependent by nature — a refusing `at` on one machine, a daemon that will not run jobs on
//! another — and the whole point of a warning CLASS is that a caller branches on it without reading
//! prose.

mod common;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

const NOW: &str = "2026-08-22T10:00:00Z";
const ORIGIN: &str = "local";

/// A workspace with one declared channel that carries a `timeout:` and one member to ask.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let dir = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("coder.yaml"),
        "handle: coder\nsystem_prompt: You are the coder.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("channels.yaml"),
        "- name: coding\n  members: [coder]\n  timeout: 10m\n",
    )
    .unwrap();
    tmp
}

fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", ORIGIN)
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    serde_json::from_str(stdout.trim()).expect("valid json")
}

/// A directory holding an `at` that refuses every job, first on `PATH` — a machine with no working
/// scheduler, without depending on how THIS machine happens to be configured.
fn a_machine_whose_at_cannot_schedule(tmp: &TempDir) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = tmp.path().join("stub-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let at = bin.join("at");
        std::fs::write(
            &at,
            "#!/bin/sh\ncat > /dev/null\necho 'at: refused' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755)).unwrap();
        format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }
    #[cfg(not(unix))]
    {
        let _ = tmp;
        std::env::var("PATH").unwrap_or_default()
    }
}

fn warnings_of(receipt: &Value) -> &Vec<Value> {
    receipt["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("`warnings` is ALWAYS present in --json: {receipt}"))
}

#[test]
fn a_board_whose_timeout_tick_could_not_be_scheduled_says_so_in_warnings() {
    let tmp = workspace();
    let receipt = json_of(
        human(&tmp)
            .env("NXC_TIMER", "at")
            .env("PATH", a_machine_whose_at_cannot_schedule(&tmp))
            .arg("--json")
            .args([
                "send",
                "--no-ref",
                "--to",
                "coding",
                "build the scaffolding",
            ]),
    );

    // The board IS open: a scheduling failure is best-effort and must never fail a persisted board.
    let thread = receipt["thread_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the board was opened: {receipt}"));

    let warnings = warnings_of(&receipt);
    let tick = warnings
        .iter()
        .find(|w| w["class"] == "tick_unscheduled")
        .unwrap_or_else(|| {
            panic!(
                "a caller whose window was never armed must see it as a CLASS, not only on \
                 stderr: {receipt}"
            )
        });
    assert_eq!(
        tick["thread"], thread,
        "the finding names the board whose window has no clock: {receipt}"
    );
    assert!(
        tick["detail"]
            .as_str()
            .is_some_and(|d| d.contains("nxc tick --thread")),
        "and says what to do instead, since the check stays callable by hand: {receipt}"
    );
}

#[test]
fn a_board_whose_tick_was_scheduled_reports_nothing() {
    // The other side of the same field. `NXC_TIMER=dry` schedules successfully (it records instead
    // of shelling out), so this is the ordinary path — and it has to stay silent, or an app that
    // branches on the array treats every timed board as broken.
    let tmp = workspace();
    let receipt = json_of(
        human(&tmp)
            .env("NXC_TIMER", "dry")
            .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
            .arg("--json")
            .args([
                "send",
                "--no-ref",
                "--to",
                "coding",
                "build the scaffolding",
            ]),
    );

    assert!(
        warnings_of(&receipt).is_empty(),
        "a tick that WAS scheduled is not a warning: {receipt}"
    );
    let log = std::fs::read_to_string(tmp.path().join("timer.log")).unwrap_or_default();
    assert!(
        log.contains("schedule thread="),
        "…and the control really did schedule one, or the assertion above proves nothing:\n{log}"
    );
}

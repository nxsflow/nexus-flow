//! Multicall routing (nexus-flow-5jz.7): the one `nxs` binary serves the flow + memory surfaces too.
//! `nxs flow <args>` must be byte-identical to `nxf <args>`, and `nxs memory <args>` to `nxm <args>`,
//! across the whole surface (here exercised on a no-workspace command + `--help`). The bin targets
//! have collapsed: `nxf`/`nxm` are now `argv[0]` symlinks to `nxs` (build.rs creates them), so the
//! `cargo_bin("nxf"|"nxm")` lookups below resolve those symlinks and exercise the REAL persona path.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// Run `bin args…` and return (stdout, stderr, exit-code) for a byte-for-byte comparison.
///
/// Never this machine's real scheduler (nxf 6j6v.74c0). Every case here runs `guide`/`--help`, so
/// nothing schedules anything today — but on macOS an unset `NXC_TIMER` means the real `launchd`
/// backend, and a routing case that ever grows a real verb would bootstrap a one-shot agent into
/// the developer's login session. `crates/chat/tests/no_test_arms_the_real_scheduler.rs` is what
/// asks for this line; setting it on BOTH sides keeps the byte-for-byte comparison honest.
fn capture(mut cmd: Command, args: &[&str]) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let out = cmd
        .env("NXC_TIMER", "dry")
        .args(args)
        .output()
        .expect("binary runs");
    (out.stdout, out.stderr, out.status.code())
}

#[test]
fn nxs_flow_routes_identically_to_nxf() {
    // `guide` needs no workspace and is deterministic; flow's normal output framing applies to both.
    let direct = capture(Command::cargo_bin("nxf").expect("nxf"), &["guide"]);
    let routed = capture(Command::cargo_bin("nxs").expect("nxs"), &["flow", "guide"]);
    assert_eq!(
        direct.0, routed.0,
        "stdout identical (nxf guide == nxs flow guide)"
    );
    assert_eq!(direct.1, routed.1, "stderr identical");
    assert_eq!(direct.2, routed.2, "exit code identical");
}

#[test]
fn nxs_flow_help_shows_nxf_as_the_program_name() {
    // The hidden route is a true persona: `nxs flow --help` renders nxf's own help (program-name
    // `nxf`), byte-identical to `nxf --help` — NOT an `nxs flow` usage string.
    let direct = capture(Command::cargo_bin("nxf").expect("nxf"), &["--help"]);
    let routed = capture(Command::cargo_bin("nxs").expect("nxs"), &["flow", "--help"]);
    assert_eq!(direct, routed, "nxf --help == nxs flow --help");
    assert!(
        String::from_utf8_lossy(&direct.0).contains("Usage: nxf"),
        "flow help names nxf"
    );
}

#[test]
fn nxs_memory_routes_identically_to_nxm() {
    let direct = capture(Command::cargo_bin("nxm").expect("nxm"), &["--help"]);
    let routed = capture(
        Command::cargo_bin("nxs").expect("nxs"),
        &["memory", "--help"],
    );
    assert_eq!(direct, routed, "nxm --help == nxs memory --help");
}

#[test]
fn nxs_chat_routes_identically_to_nxc() {
    // `nxs chat <args>` must be byte-identical to `nxc <args>` (nexus-chat T2 routing).
    // --help renders nxc's own help (program-name `nxc`), byte-identical.
    let direct = capture(Command::cargo_bin("nxc").expect("nxc"), &["--help"]);
    let routed = capture(Command::cargo_bin("nxs").expect("nxs"), &["chat", "--help"]);
    assert_eq!(direct, routed, "nxc --help == nxs chat --help");
    assert!(
        String::from_utf8_lossy(&direct.0).contains("Usage: nxc"),
        "chat help names nxc"
    );
    // A real verb in an EMPTY dir (no workspace) routes identically too — dispatch + error envelope,
    // not just help. A temp cwd guarantees no `.nxs/` is discovered up the tree.
    let tmp = TempDir::new().expect("tempdir");
    let mut c_direct = Command::cargo_bin("nxc").expect("nxc");
    c_direct.current_dir(tmp.path());
    let mut c_routed = Command::cargo_bin("nxs").expect("nxs");
    c_routed.current_dir(tmp.path());
    // `channels` stood here until 6j6v.dvyq §3 removed it, and `inbox` until 6j6v.1gm9 did; a clap
    // "unrecognized subcommand" is identical on both sides for a trivial reason and would have kept
    // this assertion green while testing nothing. `list` is a REAL verb that reaches the workspace
    // resolution and fails there, which is the dispatch + error envelope this case is about — and
    // it takes no argument, so nothing but the routing can make the two sides differ.
    let direct = capture(c_direct, &["--json", "list"]);
    let routed = capture(c_routed, &["chat", "--json", "list"]);
    assert_eq!(
        direct, routed,
        "nxc list == nxs chat list (identical no-workspace error)"
    );
    assert!(
        String::from_utf8_lossy(&direct.1).contains("workspace")
            || String::from_utf8_lossy(&direct.0).contains("workspace"),
        "…and it really is the no-workspace error, not an argument-parsing one"
    );
}

#[test]
fn flow_and_memory_are_hidden_from_the_nxs_umbrella_help() {
    // The routes are intercepted before clap parses, so they never appear in `nxs --help`.
    let out = Command::cargo_bin("nxs")
        .expect("nxs")
        .arg("--help")
        .output()
        .expect("runs");
    let help = String::from_utf8_lossy(&out.stdout);
    // The umbrella verbs are present…
    assert!(
        help.contains("init") && help.contains("prime"),
        "umbrella verbs listed: {help}"
    );
    // …but the multicall personas are not advertised as subcommands.
    for line in help.lines() {
        let l = line.trim_start();
        assert!(
            !l.starts_with("flow ")
                && !l.starts_with("memory ")
                && !l.starts_with("chat ")
                && l != "flow"
                && l != "memory"
                && l != "chat",
            "multicall route leaked into nxs --help: {line:?}"
        );
    }
}

#[test]
fn a_legacy_nexusflow_workspace_still_migrates_through_the_multicall_nxf() {
    // The `.nexusflow/` → `.nxs/` migration is foundation-level (on workspace open), untouched by
    // the multicall collapse — but it runs through the `nxf` SYMLINK now, so pin it: a legacy
    // project folder is still migrated when reached via the persona binary.
    let tmp = TempDir::new().expect("tempdir");
    // Build a real workspace, then rename it to the pre-rename `.nexusflow/` name to simulate one of
    // the legacy folders that must still migrate.
    // **`init` writes to the real service registry unless a home is pinned** (nxf 6j6v.y12q,
    // review of PR #421). Every other call in this file reads (`guide`, `--help`, `list`) and was
    // side-effect-free; since y12q this one puts the workspace on the background service's list,
    // and a bare `Command::cargo_bin` carries none of `nxs_test_support::cargo_bin`'s isolation —
    // so without this the developer's own registry gains an entry for a deleted `TempDir`.
    Command::cargo_bin("nxf")
        .expect("nxf")
        .pin_home(tmp.path().join(".fake-home"))
        .args(["init", "--plugin", "issue-tracker", "--quiet"])
        .current_dir(tmp.path())
        .assert()
        .success();
    std::fs::rename(tmp.path().join(".nxs"), tmp.path().join(".nexusflow")).expect("rename");
    assert!(
        !tmp.path().join(".nxs").exists(),
        "starts as legacy .nexusflow"
    );

    // A normal read through the multicall `nxf` symlink resolves + migrates the legacy dir on open.
    Command::cargo_bin("nxf")
        .expect("nxf")
        .pin_home(tmp.path().join(".fake-home"))
        .args(["list", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert!(
        tmp.path().join(".nxs").exists(),
        "legacy .nexusflow migrated to .nxs via the multicall nxf"
    );
    assert!(
        !tmp.path().join(".nexusflow").exists(),
        "legacy dir consumed by the migration"
    );
}

/// The "no workspace" advice must name the init of the persona the user actually invoked, on BOTH
/// the direct and the routed form — and the two must agree.
///
/// This is the wiring `run()` does with `PERSONA_FLOW`/`PERSONA_MEMORY`/`PERSONA_CHAT`, and it is
/// exactly the sort of glue a swapped constant breaks silently: telling an `nxm` user to run
/// `nxf init` yields a workspace their module was never registered in, so `nxs prime` replays
/// nothing for it — the command appears to work because the store is shared (PR #326 review, Test
/// Quality #1). Pinning the exact per-persona string is what makes a swap fail here rather than in
/// someone's terminal; the pre-existing routing tests use `guide`/`--help`, which never reach this
/// path at all.
#[test]
fn every_persona_is_told_to_run_its_own_init_direct_and_routed() {
    // (direct binary, workspace-needing verb, routed prefix, the init it must name)
    let cases = [
        ("nxf", "next", Some("flow"), "run `nxf init`"),
        ("nxm", "memories", Some("memory"), "run `nxm init`"),
        ("nxc", "list", Some("chat"), "run `nxc init`"),
        ("nxs", "doctor", None, "run `nxs init`"),
    ];
    for (bin, verb, route, expected) in cases {
        let tmp = TempDir::new().expect("tempdir");

        let mut direct_cmd = Command::cargo_bin(bin).unwrap_or_else(|_| panic!("{bin} exists"));
        direct_cmd.current_dir(tmp.path());
        let direct = capture(direct_cmd, &["--json", verb]);
        let msg =
            String::from_utf8_lossy(&direct.1).to_string() + &String::from_utf8_lossy(&direct.0);
        assert!(
            msg.contains(expected),
            "`{bin} {verb}` outside a workspace must say {expected:?}, got: {msg}"
        );

        // The routed form synthesizes argv[0] for the sub-parser only, so the process argv still
        // reads `nxs` — the persona has to be carried deliberately for these to match. The route
        // token has to lead: `run()` intercepts it at `args[1]`, before clap ever parses, which is
        // what keeps `flow`/`memory`/`chat` out of `nxs --help`.
        if let Some(route) = route {
            let mut routed_cmd = Command::cargo_bin("nxs").expect("nxs");
            routed_cmd.current_dir(tmp.path());
            let routed = capture(routed_cmd, &[route, "--json", verb]);
            assert_eq!(
                direct, routed,
                "`{bin} {verb}` == `nxs {route} {verb}` (byte-identical, incl. the init advice)"
            );
        }
    }
}

/// The service persona (nxf 6j6v.8see). `nexus-flow` exists so the macOS background item carries a
/// NAME — macOS takes a process's displayed name from the path it `exec`s — and this is what makes
/// the name mean something: everything after it goes to `nxs sync daemon`.
#[test]
fn nexus_flow_routes_identically_to_nxs_sync_daemon() {
    let home = TempDir::new().expect("home");
    let run = |bin: &str, args: &[&str]| {
        let mut cmd = Command::cargo_bin(bin).unwrap_or_else(|_| panic!("{bin} binary built"));
        cmd.pin_home(home.path());
        capture(cmd, args)
    };
    // `status` reads the heartbeat and never starts a loop, so it is the one verb this can run
    // end-to-end. Under an empty $HOME both sides must report the same "not running".
    let direct = run("nxs", &["--json", "sync", "daemon", "status"]);
    let aliased = run("nexus-flow", &["--json", "status"]);
    assert_eq!(
        aliased, direct,
        "the alias is `nxs sync daemon`, byte for byte — stdout/stderr/exit"
    );
    assert!(
        String::from_utf8_lossy(&direct.0).contains("\"running\":false"),
        "and both really answered the heartbeat question: {:?}",
        String::from_utf8_lossy(&direct.0)
    );
}

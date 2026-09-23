//! `nxs init` brings a workspace to the background service (nxf 6j6v.npf9), from the outside.
//!
//! Until this, the service and its registry were reached on exactly one path — as a side effect of
//! `nxs sync bind` — so a workspace that was set up and never bound to a sync stream had no entry
//! in `~/.nexusflow/workspaces.toml` and no service. After 6j6v.8see that means no CLOCK: nothing
//! in it ever fires a declared window.
//!
//! Black-box, under a controlled `$HOME`, so the real `~/.nexusflow/workspaces.toml` is never
//! touched. **Nothing here installs a service**: `launchctl bootstrap` addresses the real login
//! session whatever `$HOME` says, so the install branch is proven in-process against
//! `background_service`'s `Machine` seam instead — the same reason `sync bind`'s tests all pass
//! `--no-daemon`.

use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// A `$HOME` and a project directory, each the test's own.
fn sandbox() -> (TempDir, TempDir) {
    (TempDir::new().unwrap(), TempDir::new().unwrap())
}

/// `nxs init --json <args…>` in `project`, with `$HOME` at `home`. Returns the parsed object.
fn init(home: &TempDir, project: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = Command::new(cargo_bin("nxs"))
        .args(["--json", "init"])
        .args(args)
        .current_dir(project.path())
        .pin_home(home.path())
        .output()
        .expect("nxs runs");
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("--json emits one JSON object")
}

/// The registry this `$HOME`'s service would sweep, or an empty string when it was never written.
fn registry(home: &TempDir) -> String {
    std::fs::read_to_string(home.path().join(".nexusflow").join("workspaces.toml"))
        .unwrap_or_default()
}

/// The workspace's own config — where the recorded answer lives.
fn config(project: &TempDir) -> String {
    std::fs::read_to_string(project.path().join(".nxs").join("config.toml"))
        .expect("init wrote a workspace config")
}

#[test]
fn a_freshly_initialized_workspace_is_on_the_list_the_service_attends() {
    // The DoD: a workspace that only wants the clock stands in `workspaces.toml`, without anybody
    // having bound it to a stream.
    let (home, project) = sandbox();
    let got = init(&home, &project, &[]);

    assert_eq!(got["ok"], true);
    assert_eq!(got["service"]["registered"], true);
    assert!(
        got["service"].get("installed").is_none(),
        "whether the MACHINE has a service is `nxs sync daemon status`'s question — the one \
         platform-dependent fact stays out of a record two machines have to compare: {}",
        got["service"]
    );

    let registry = registry(&home);
    let root = project.path().display().to_string();
    assert!(
        registry.contains(&root),
        "the PROJECT root is registered, not the `.nxs` directory: {registry}"
    );
    assert!(
        !registry.contains(&format!("{root}/.nxs")),
        "and not the workspace directory inside it: {registry}"
    );
    assert!(
        !project.path().join(".nxs").join("sync.toml").exists(),
        "registering is not binding — nothing was bound to a stream"
    );
}

#[test]
fn a_json_run_never_asks_and_installs_nothing_of_its_own_accord() {
    // The second DoD bullet. `--json` is the agent path: it may register (a file), and it may not
    // decide anything about this machine's background processes.
    let (home, project) = sandbox();
    let got = init(&home, &project, &[]);

    assert_eq!(
        got["service"]["answer"],
        serde_json::Value::Null,
        "nothing was answered, so nothing is recorded"
    );
    assert!(
        !config(&project).contains("[nxs]"),
        "and the workspace records no answer it never got: {}",
        config(&project)
    );
}

/// The DoD's fourth bullet, and the ticket's "a no stays a no" — **on a platform that has a
/// service to decline**. There is exactly one (launchd, macOS); the counterpart below covers the
/// others, and the split is not tidiness.
///
/// The first cut of this test asserted `"declined"` everywhere, passed on the machine it was
/// written on and went red on CI's Linux, where there is no installer and therefore nothing to
/// record. Two tests, each with a name that is true about the platform it runs on, is what keeps
/// the next reader from concluding the recording is broken off macOS when it is absent by design.
#[test]
#[cfg(target_os = "macos")]
fn the_negative_flag_is_the_non_interactive_no_and_it_is_remembered() {
    let (home, project) = sandbox();
    let first = init(&home, &project, &["--no-service"]);
    assert_eq!(first["service"]["answer"], "declined");
    assert!(
        config(&project).contains("declined"),
        "the answer is in the workspace's own config: {}",
        config(&project)
    );

    let second = init(&home, &project, &[]);
    assert_eq!(
        second["service"]["answer"], "declined",
        "a re-run with no flag reports the standing answer rather than clearing it"
    );
}

/// The other half of the DoD's last bullet: where there is no service installer, the question is
/// never put — so `--no-service` answers nothing, and nothing is written into the workspace about
/// a decision that was never available to make.
#[test]
#[cfg(not(target_os = "macos"))]
fn the_negative_flag_records_nothing_where_there_is_no_service_to_decline() {
    let (home, project) = sandbox();
    let got = init(&home, &project, &["--no-service"]);
    assert_eq!(
        got["service"]["answer"],
        serde_json::Value::Null,
        "there is nothing here to decline: {got}"
    );
    assert!(
        !config(&project).contains("[nxs]"),
        "and the workspace records no answer to a question it was never asked: {}",
        config(&project)
    );
    assert_eq!(
        got["service"]["registered"], true,
        "the registry is a file, and files work everywhere: {got}"
    );
}

/// The last DoD bullet, from the outside, on the platform it is about: asking for a service where
/// none can be installed is answered BY NAME rather than silently ignored — and still installs
/// nothing (review of PR #419, Test Quality #2).
#[test]
#[cfg(not(target_os = "macos"))]
fn asking_for_a_service_where_there_is_no_installer_is_answered_by_name() {
    let (home, project) = sandbox();
    let out = Command::new(cargo_bin("nxs"))
        .args(["--json", "init", "--service"])
        .current_dir(project.path())
        .pin_home(home.path())
        .output()
        .expect("nxs runs");
    assert!(
        out.status.success(),
        "a request this platform cannot honour is a note, not a failed init: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nxs sync daemon"),
        "and it names what to run instead: {stderr}"
    );
    let got: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("--json emits one JSON object");
    assert_eq!(
        got["service"]["answer"],
        serde_json::Value::Null,
        "nothing was installed, so nothing is recorded: {got}"
    );
    assert_eq!(got["service"]["registered"], true, "{got}");
}

#[test]
fn a_second_init_registers_the_same_workspace_once() {
    let (home, project) = sandbox();
    init(&home, &project, &[]);
    init(&home, &project, &[]);

    let registry = registry(&home);
    let root = project.path().display().to_string();
    assert_eq!(
        registry.matches(&root).count(),
        1,
        "registration is idempotent — the registry that grew to 98 entries did so one \
         duplicate at a time: {registry}"
    );
}

#[test]
fn the_two_service_flags_contradict_each_other_and_are_refused() {
    let (home, project) = sandbox();
    let out = Command::new(cargo_bin("nxs"))
        .args(["--json", "init", "--service", "--no-service"])
        .current_dir(project.path())
        .pin_home(home.path())
        .output()
        .expect("nxs runs");
    assert!(!out.status.success(), "a contradiction is not a default");
    assert!(
        !project.path().join(".nxs").exists(),
        "and it is refused BEFORE anything is set up"
    );
}

#[test]
fn a_human_reading_the_frame_is_told_where_the_service_stands() {
    // Not `--json`: the summary a person sees. Under `cargo test` stdin/stderr are not terminals,
    // so this is also the pipe/CI case of the DoD — it must finish without prompting.
    let (home, project) = sandbox();
    let out = Command::new(cargo_bin("nxs"))
        .arg("init")
        .current_dir(project.path())
        .pin_home(home.path())
        .output()
        .expect("nxs runs");
    assert!(
        out.status.success(),
        "a pipe must not block on a question: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("service:"),
        "the frame names where the background service stands: {stdout}"
    );
    assert!(
        stdout.contains("attends this workspace"),
        "including the half that is only true because `init` just made it so: {stdout}"
    );
}

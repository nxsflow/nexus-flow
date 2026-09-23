//! **Two named services on one machine, at the surface a user actually sees** (nxf 6j6v.gd9p).
//!
//! `crates/service` proves the mechanism: separate homes mean separate lock, registry, heartbeat
//! and config paths, and a sister instance is never booted out. That is all true and all
//! structural — and the review of PR #395 showed it was ALSO all that existed. Mutation confirmed
//! the gap twice: removing the `warn_if_attended_by_more_than_one()` call from the bind path left
//! every `sync_bind` test green, and reverting `endpoint::config_path()` to its old duplicated
//! `~/.nexusflow` lookup left the whole `nxs` + `nexus-flow-cli` suite green.
//!
//! So this suite drives the real binary, and every test here is a sentence out of the ticket's
//! Definition of Done rather than a property of a function:
//!
//! - two services run side by side, each holding its own lock and writing its own heartbeat;
//! - a workspace attended by both is NAMED — at the bind that creates the overlap, by
//!   `daemon status`, and by the running service — because being silently double-ticked is not one
//!   of the two acceptable answers;
//! - an instance reads its global config out of ITS home, not out of the shared one.
//!
//! Everything is hermetic: a `TempDir` `$HOME`, no launchd, no relay. The one half that is NOT
//! automated is deliberate — installing two real launchd agents would mutate the login session of
//! whoever runs the suite. That half is verified by hand and recorded on the ticket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use tempfile::TempDir;

const DEV: &str = "nexus-flow-dev";

/// `nxs` under a fake `$HOME`, as a named instance (or the default one when `instance` is `None`).
///
/// Built by hand rather than through `nxs_test_support::cargo_bin` because that helper deliberately
/// CLEARS `NXS_SERVICE_INSTANCE` — which is right for every other suite and exactly wrong for this
/// one, whose whole subject is the variable. [`PinHome`] clears it too, and this is the shape that
/// answer is meant to take: pin the home first, then say what this suite wants of the instance.
fn nxs(home: &Path, cwd: &Path, instance: Option<&str>) -> Command {
    let mut c = Command::new(cargo_bin("nxs"));
    c.current_dir(cwd).pin_home(home).env("NXC_TIMER", "dry");
    match instance {
        Some(name) => c.env("NXS_SERVICE_INSTANCE", name),
        None => c.env_remove("NXS_SERVICE_INSTANCE"),
    };
    c
}

/// A real workspace in its own directory.
fn workspace(home: &Path) -> TempDir {
    let dir = TempDir::new().unwrap();
    nxs(home, dir.path(), None)
        .args(["init", "--module", "chat", "--json"])
        .output()
        .expect("nxs runs");
    assert!(
        dir.path().join(".nxs").join("replica.toml").is_file(),
        "the fixture workspace was not created"
    );
    dir
}

/// The service home directory of `instance` under `home`.
fn service_home(home: &Path, instance: Option<&str>) -> PathBuf {
    match instance {
        None => home.join(".nexusflow"),
        Some(name) => home.join(format!(
            ".nexusflow-{}",
            name.strip_prefix("nexus-flow-").expect("a qualified name")
        )),
    }
}

/// The path a workspace is registered UNDER, which is the one the running binary will write.
///
/// Canonicalised, and that is not tidiness: on macOS a `TempDir` lives under `/var/folders/…`,
/// which is a symlink to `/private/var/folders/…`, and a process standing in it resolves its own
/// working directory to the second. The registry deliberately does NOT canonicalise what it stores
/// (its own doc: resolving symlinks would rewrite the very string the caller hands back), so a
/// fixture that plants the un-resolved spelling registers what is, to every reader, a DIFFERENT
/// workspace — and the overlap it was staging silently does not exist. Found by this suite failing
/// for exactly that reason.
fn ws_path(dir: &TempDir) -> PathBuf {
    std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf())
}

/// Register `workspace` with `instance` by writing its registry directly — attendance is one line
/// in a file, and `sync bind` would additionally want a relay.
fn attend(home: &Path, instance: Option<&str>, workspace: &Path) {
    let dir = service_home(home, instance);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("workspaces.toml"),
        format!(
            "[[workspace]]\nname = \"under-test\"\npath = \"{}\"\n",
            workspace.display()
        ),
    )
    .unwrap();
}

/// Kill and reap on the way out, so a panicking assertion never leaves a service running.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(home: &Path, instance: Option<&str>, log: &Path) -> Reaped {
    let out = std::fs::File::create(log).unwrap();
    let err = out.try_clone().unwrap();
    Reaped(
        nxs(home, home, instance)
            .args(["sync", "daemon"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("the service starts"),
    )
}

fn wait_for<F: Fn() -> bool>(secs: u64, f: F) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    f()
}

/// **Two services, side by side, neither refusing the other** — the DoD sentence, run rather than
/// derived from distinct file paths.
///
/// The failure this rules out is the one `RETIRED_LABELS`'s own doc describes from the other side:
/// two agents contending for one single-instance lock, *"refusing one of them to start — every ten
/// seconds, forever, into an unrotated log"*. `ServiceLock` is what would produce it, so the proof
/// has to be two live processes, not two path comparisons.
///
/// launchd is deliberately absent: the lock, the registry and the heartbeat are what can be
/// contended, and none of them is the plist.
#[test]
fn two_instances_run_at_once_each_holding_its_own_lock_and_writing_its_own_heartbeat() {
    let home = TempDir::new().unwrap();
    let ws_a = workspace(home.path());
    let ws_b = workspace(home.path());
    attend(home.path(), None, &ws_path(&ws_a));
    attend(home.path(), Some(DEV), &ws_path(&ws_b));

    let log_prod = home.path().join("prod.log");
    let log_dev = home.path().join("dev.log");
    let prod = start_service(home.path(), None, &log_prod);
    let dev = start_service(home.path(), Some(DEV), &log_dev);

    let hb_prod = service_home(home.path(), None).join("sync-daemon.json");
    let hb_dev = service_home(home.path(), Some(DEV)).join("sync-daemon.json");
    assert!(
        wait_for(20, || hb_prod.is_file() && hb_dev.is_file()),
        "both services must get running: prod log:\n{}\ndev log:\n{}",
        std::fs::read_to_string(&log_prod).unwrap_or_default(),
        std::fs::read_to_string(&log_dev).unwrap_or_default()
    );

    // Each heartbeat names its OWN instance and its OWN pid — the two facts that would collapse
    // into one if they shared a home.
    let read = |p: &Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    };
    let (a, b) = (read(&hb_prod), read(&hb_dev));
    assert_eq!(a["instance"], "nexus-flow");
    assert_eq!(b["instance"], DEV);
    assert_ne!(a["pid"], b["pid"], "two heartbeats, two live processes");

    // Separate locks, held simultaneously: each holds its own and neither could take the other's.
    for instance in [None, Some(DEV)] {
        assert!(
            service_home(home.path(), instance)
                .join("sync-daemon.lock")
                .is_file(),
            "each instance holds a lock of its own"
        );
    }

    // Neither log carries the single-instance refusal — the symptom this whole design exists to
    // make impossible.
    for (label, log) in [("production", &log_prod), ("dev", &log_dev)] {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        assert!(
            !text.to_lowercase().contains("already running"),
            "the {label} service was refused by a lock it should never have reached for:\n{text}"
        );
    }

    // Both are still alive at the end, which is the claim: side by side, not one after the other.
    for (label, svc) in [("production", &prod), ("dev", &dev)] {
        assert!(
            unsafe { libc::kill(svc.0.id() as libc::pid_t, 0) } == 0,
            "the {label} service died while its sister ran"
        );
    }
}

/// Seam 1 of three: **the bind that CREATES the overlap says so**, at the moment it happens.
///
/// Mutation-verified as absent by the review of PR #395 — dropping the call from
/// `RealRegistrar::register` left all fourteen `sync_bind` tests green.
#[test]
fn binding_a_workspace_a_sister_already_attends_says_so_on_stderr() {
    let home = TempDir::new().unwrap();
    let ws = workspace(home.path());
    // Production already attends it; the bind below is the dev instance's.
    attend(home.path(), None, &ws_path(&ws));
    // The dev home must exist for it to be found as a sister at all.
    std::fs::create_dir_all(service_home(home.path(), Some(DEV))).unwrap();

    let out = nxs(home.path(), ws.path(), Some(DEV))
        .args(["sync", "bind", "--no-daemon", "--create"])
        .output()
        .expect("nxs runs");
    assert!(out.status.success(), "the bind itself must still succeed");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(&ws_path(&ws).display().to_string()),
        "the note must name the workspace it is about:\n{err}"
    );
    assert!(
        err.contains("nexus-flow-dev") && err.contains("more than one"),
        "and both instances attending it:\n{err}"
    );
    assert!(
        err.contains("started twice"),
        "and what it COSTS, which is the whole reason to say anything:\n{err}"
    );
}

/// Seam 2 of three: **`daemon status` names it**, in both renderings.
///
/// `crates/cli/tests/sync_daemon.rs` pins the shape of `shared_workspaces` and only ever meets the
/// empty case, so nothing showed what a non-empty one looks like.
#[test]
fn daemon_status_names_a_workspace_two_instances_attend() {
    let home = TempDir::new().unwrap();
    let ws = workspace(home.path());
    attend(home.path(), None, &ws_path(&ws));
    attend(home.path(), Some(DEV), &ws_path(&ws));

    let out = nxs(home.path(), home.path(), Some(DEV))
        .args(["sync", "daemon", "status", "--json"])
        .output()
        .expect("nxs runs");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let shared = v["shared_workspaces"]
        .as_array()
        .expect("an array, not null — this registry reads fine");
    assert_eq!(shared.len(), 1, "{v}");
    assert_eq!(shared[0]["path"], ws_path(&ws).display().to_string());
    assert_eq!(
        shared[0]["instances"],
        serde_json::json!(["nexus-flow", "nexus-flow-dev"]),
        "sorted, so the same overlap reads identically whichever instance reports it"
    );

    let human = nxs(home.path(), home.path(), Some(DEV))
        .args(["sync", "daemon", "status"])
        .output()
        .expect("nxs runs");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("more than one nexus-flow service instance")
            && text.contains("started twice"),
        "the human branch must say it too, or it reaches only a machine:\n{text}"
    );
}

/// The other half of seam 2, and the one the review asked for by name (Integrity #2): when the
/// check could not RUN, `shared_workspaces` is `null` rather than `[]`.
///
/// `[]` says "nobody else attends these"; `null` says "nobody looked". Reporting the second as the
/// first is the silent answer the overlap machinery exists to prevent.
#[test]
fn a_registry_that_cannot_be_read_is_null_rather_than_an_empty_overlap() {
    let home = TempDir::new().unwrap();
    let dir = service_home(home.path(), Some(DEV));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workspaces.toml"), "this is not toml {{{").unwrap();

    let out = nxs(home.path(), home.path(), Some(DEV))
        .args(["sync", "daemon", "status", "--json"])
        .output()
        .expect("nxs runs");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(
        v["shared_workspaces"].is_null(),
        "an unreadable registry is not an empty one: {v}"
    );

    let human = nxs(home.path(), home.path(), Some(DEV))
        .args(["sync", "daemon", "status"])
        .output()
        .expect("nxs runs");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("could not be read") && text.contains("nothing checked"),
        "and the human branch says nobody looked:\n{text}"
    );
}

/// Seam 3 of three: **the running service says it too**, and says it once.
///
/// The service is the one actually ticking the workspace, so its word is the one that carries — and
/// it is the only seam a person who never runs `status` will ever meet.
#[test]
fn the_running_service_logs_a_doubly_attended_workspace_once_per_run() {
    let home = TempDir::new().unwrap();
    let ws = workspace(home.path());
    attend(home.path(), None, &ws_path(&ws));
    attend(home.path(), Some(DEV), &ws_path(&ws));

    let log = home.path().join("dev.log");
    let _svc = start_service(home.path(), Some(DEV), &log);

    let text = || std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        wait_for(20, || text()
            .contains("more than one nexus-flow service instance")),
        "the service that is ticking a shared workspace must say so:\n{}",
        text()
    );
    assert!(
        text().contains(&ws_path(&ws).display().to_string()),
        "and name which workspace:\n{}",
        text()
    );

    // Once per workspace per run, not once per pass — a line a second forever is a line nobody
    // reads. Given time for several more passes before counting.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        text()
            .matches("more than one nexus-flow service instance")
            .count(),
        1,
        "warned once, not once per tick:\n{}",
        text()
    );
}

/// **An instance reads its global config out of ITS home** (nxf 6j6v.gd9p's fourth site).
///
/// `endpoint::config_path()` held the fifth independent copy of `~/.nexusflow` and now goes through
/// `ServiceHome::config()`. Mutation-verified as unpinned by the review of PR #395: restoring the
/// duplicated `directories::BaseDirs` lookup left the whole suite green — a named instance would
/// have kept its registry, lock and heartbeat separate while silently sharing the one file that
/// says where it syncs to.
#[test]
fn the_global_sync_endpoint_lives_in_the_instances_own_home() {
    let home = TempDir::new().unwrap();

    nxs(home.path(), home.path(), Some(DEV))
        .args(["sync", "endpoint", "https://dev.example/relay"])
        .assert_ok();

    assert!(
        service_home(home.path(), Some(DEV))
            .join("config.toml")
            .is_file(),
        "the dev instance must write its own config.toml"
    );
    assert!(
        !service_home(home.path(), None).join("config.toml").exists(),
        "and must not have written into the SHARED one, which is the whole defect"
    );

    // …and read it back from there, rather than finding nothing.
    let out = nxs(home.path(), home.path(), Some(DEV))
        .args(["sync", "endpoint", "--json"])
        .output()
        .expect("nxs runs");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("https://dev.example/relay"),
        "the same instance must read back what it wrote: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // The production instance, standing beside it, sees none of that.
    let other = nxs(home.path(), home.path(), None)
        .args(["sync", "endpoint", "--json"])
        .output()
        .expect("nxs runs");
    assert!(
        !String::from_utf8_lossy(&other.stdout).contains("dev.example"),
        "production must not inherit a sister instance's endpoint: {}",
        String::from_utf8_lossy(&other.stdout)
    );
}

/// `Command::output()` plus a success assertion that shows both streams when it fails.
trait AssertOk {
    fn assert_ok(&mut self);
}

impl AssertOk for Command {
    fn assert_ok(&mut self) {
        let out = self.output().expect("nxs runs");
        assert!(
            out.status.success(),
            "{:?} failed:\n{}\n{}",
            self.get_args().collect::<Vec<_>>(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

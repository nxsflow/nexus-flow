//! **A background service that is not running is named, not left to be inferred** (nxf 6j6v.0j12).
//!
//! Before the singleton (6j6v.8see) the clock was self-carrying: `launchd` fired a job per deadline
//! whether anything was running or not, so "the service is down" was not a state that could cost
//! anything. Since the owner decision of 2026-08-25 that one process is the only clock AND the only
//! sync — so while it is down, a round that should release hangs indefinitely and this machine's op
//! log neither pushes nor pulls. Both were silent: `nxs sync daemon status` says it on request, and
//! nobody asks.
//!
//! Driven black-box through the real `nxc`, with `$HOME` pointed at a `TempDir` so the registry and
//! the heartbeat under test are real files in the real layout and nothing here can reach the
//! developer's own `~/.nexusflow`.
//!
//! **This is the one suite that deliberately unpins `NXC_TIMER` from `dry`**, since the backend IS
//! what is under test — so it owes the argument `no_test_arms_the_real_scheduler.rs` makes. It
//! arms nothing: a `send --to <persona>` carries no window, and the verbs here declare no
//! `timeout:`. And if it ever did, it could not reach outside its own fixture — the working
//! directory is the `TempDir` workspace (which is what an unpinned `ServiceTimer` resolves its
//! deadline book from) and `$HOME` is a `TempDir` too, so both files such a schedule would touch
//! are inside this test's own sandbox.

use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

const NOW: &str = "2026-08-30T10:00:00Z";

/// A chat workspace with two declared personas and a declared CHANNEL over them.
///
/// Both targets are here because `send --to` has two branches and they report this independently:
/// the persona branch through `coordinator_commission`, the channel branch through `channel_open`
/// (review of PR #393, Test Quality #2 — the channel one was claimed and untested).
///
/// The channel declares NO `timeout:` on purpose. That is the shape the class exists for: with no
/// window to arm, nothing ever reached the scheduling site, so this was the one send that could not
/// report a dead service at all.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "reviewer"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: pair\n  members: [coder, reviewer]\n",
    )
    .unwrap();
    tmp
}

/// A `~/.nexusflow` whose heartbeat names a process that does not exist — the state a machine is in
/// after the service died, or was booted out, or never started.
///
/// `registered` decides whether this workspace is on the list the service sweeps, which is the one
/// thing that turns "not running" into a fault worth reporting: a workspace nobody registered never
/// asked the service for anything.
fn service_home(home: &Path, workspace: &Path, registered: bool) {
    let root = home.join(".nexusflow");
    std::fs::create_dir_all(&root).unwrap();
    // The CANONICAL path, which is what `nxs sync bind` writes: the workspace resolver canonicalises
    // (`/var/folders/…` is a symlink to `/private/var/folders/…` on macOS), and the registry is keyed
    // by exact string. Writing the raw `TempDir` path here would register a workspace the service
    // could never match, and the test would then prove nothing while looking green.
    let canonical = std::fs::canonicalize(workspace).expect("the workspace directory exists");
    let registry = if registered {
        format!(
            "[[workspace]]\nname = \"probe\"\npath = {:?}\n",
            canonical.display().to_string()
        )
    } else {
        String::new()
    };
    std::fs::write(root.join("workspaces.toml"), registry).unwrap();
    std::fs::write(
        root.join("sync-daemon.json"),
        serde_json::json!({
            // A pid no process can hold, so the liveness probe answers "proven gone" rather than
            // "unknown" — the difference between a machine that is broken and one that cannot say.
            "pid": u32::MAX,
            "started_at": "2026-08-29T07:13:41Z",
            "last_pass_at": "2026-08-29T07:13:41Z",
            "workspaces": [],
        })
        .to_string(),
    )
    .unwrap();
}

/// `nxc --json send --to coder`, on the REAL service timer, under a controlled `$HOME`.
fn send_under(tmp: &TempDir, home: &TempDir) -> Value {
    send_to_under(tmp, home, "coder")
}

/// The same, against any declared target — a persona or a channel.
fn send_to_under(tmp: &TempDir, home: &TempDir, to: &str) -> Value {
    let mut c = nxs_test_support::cargo_bin("nxc");
    let out = c
        .current_dir(tmp.path())
        .pin_home(home.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ORIGIN", "local")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        // The backend under test. `cargo_bin` pins `dry` structurally so no suite can reach the
        // host's scheduler by forgetting a line; this one is ABOUT the service backend, and it
        // reaches no scheduler either — it reads two files under the `$HOME` above.
        .env("NXC_TIMER", "service")
        .args(["--json", "send", "--to", to, "have a look"])
        .assert()
        .success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

fn warnings(receipt: &Value) -> Vec<Value> {
    receipt["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("`warnings` is never omitted: {receipt}"))
        .clone()
}

#[test]
fn a_send_into_a_registered_workspace_with_no_service_says_so_by_class() {
    let tmp = workspace();
    let home = TempDir::new().unwrap();
    service_home(home.path(), tmp.path(), true);

    let receipt = send_under(&tmp, &home);
    let found = warnings(&receipt);
    let entry = found
        .iter()
        .find(|w| w["class"] == "service_not_running")
        .unwrap_or_else(|| {
            panic!("the send must report that nothing is attending this workspace: {receipt}")
        });

    let detail = entry["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("windows will not fire"),
        "the clock half: {detail}"
    );
    assert!(
        detail.contains("neither pushes nor pulls"),
        "the sync half — the half every earlier message left out: {detail}"
    );
    assert!(
        detail.contains("nxs sync daemon"),
        "and the way out: {detail}"
    );
    assert_eq!(
        entry["thread"].as_str(),
        receipt["thread_id"].as_str(),
        "it names where it was noticed, so an app can go and look"
    );
}

#[test]
fn a_send_to_a_declared_channel_reports_it_too_even_with_no_window_to_arm() {
    // The branch the class was built for and the one no test drove (review of PR #393, Test
    // Quality #2). A channel with no declared `timeout:` arms nothing, so `TickUnscheduled` — the
    // only report there used to be — can never fire here. If this send is silent, the whole "it
    // does not need a window" argument is untrue.
    let tmp = workspace();
    let home = TempDir::new().unwrap();
    service_home(home.path(), tmp.path(), true);

    let receipt = send_to_under(&tmp, &home, "pair");
    let found = warnings(&receipt);
    assert!(
        !found.iter().any(|w| w["class"] == "tick_unscheduled"),
        "no window was armed here, so the older class cannot be what carries it: {receipt}"
    );
    let entry = found
        .iter()
        .find(|w| w["class"] == "service_not_running")
        .unwrap_or_else(|| panic!("the channel branch must report it as well: {receipt}"));
    assert_eq!(
        entry["thread"].as_str(),
        receipt["thread_id"].as_str(),
        "named on the board it was noticed for"
    );
    assert_eq!(
        receipt["target"], "channel",
        "and this really was the channel branch: {receipt}"
    );
}

#[test]
fn the_send_itself_still_succeeds_and_reports_everything_it_did() {
    // The class is about the machine, not about this call: the message is posted, the thread is
    // open and the persona was started. A red exit code here would make every verb in a workspace
    // with a stopped service permanently non-zero.
    let tmp = workspace();
    let home = TempDir::new().unwrap();
    service_home(home.path(), tmp.path(), true);

    let receipt = send_under(&tmp, &home);
    assert!(receipt["thread_id"].is_string(), "{receipt}");
    assert!(receipt["message_id"].is_string(), "{receipt}");
    assert_eq!(receipt["spawned"], Value::Bool(true), "{receipt}");
}

#[test]
fn an_unregistered_workspace_is_never_warned_about_a_service_it_did_not_ask_for() {
    let tmp = workspace();
    let home = TempDir::new().unwrap();
    service_home(home.path(), tmp.path(), false);

    let receipt = send_under(&tmp, &home);
    assert!(
        !warnings(&receipt)
            .iter()
            .any(|w| w["class"] == "service_not_running"),
        "a workspace nobody registered is behaving exactly as arranged: {receipt}"
    );
}

#[test]
fn a_tick_reports_it_too_because_a_tick_is_what_the_service_itself_would_have_run() {
    let tmp = workspace();
    let home = TempDir::new().unwrap();
    service_home(home.path(), tmp.path(), true);
    let thread = send_under(&tmp, &home)["thread_id"]
        .as_str()
        .expect("a thread")
        .to_string();

    let mut c = nxs_test_support::cargo_bin("nxc");
    let out = c
        .current_dir(tmp.path())
        .pin_home(home.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ORIGIN", "local")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "service")
        .args(["--json", "tick", "--thread", &thread])
        .assert()
        .success();
    let receipt: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
            .expect("valid json");
    assert!(
        warnings(&receipt)
            .iter()
            .any(|w| w["class"] == "service_not_running"),
        "an unattended tick is the last place that can afford to be silent: {receipt}"
    );
}

// ---- the LIBRARY seam ---------------------------------------------------------------------
//
// The three cases above drive the CLI. An embedding app never walks that path — it calls
// `Engine::send_to`/`reply_thread`, which hand straight to `surface::*` — and the project rule
// `engine-seam-test-rule` says a new library surface without a test AT THAT SEAM is not finished.
// So the same property is taken again here, with the fault INJECTED through the `Timer` seam it
// travels on rather than staged on disk: what is under test is that `surface` puts it on the
// receipt, and a stub says that without a `$HOME`, a registry or a heartbeat.

use nexus_chat::definitions::Definitions;
use nexus_chat::orchestration::Ctx;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::DryWorker;
use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};
use nxs_test_support::PinHome;

/// A timer that schedules perfectly well and reports that nothing will run what it schedules.
///
/// The two are independent by construction here, which is the point of the class: before it, the
/// only way a caller heard about the service was a schedule that FAILED, so a workspace with no
/// declared window — or one whose window was armed just fine — heard nothing.
struct HealthyButUnattended;

impl Timer for HealthyButUnattended {
    fn schedule(
        &self,
        thread_id: &str,
        _deadline: &str,
        _command: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        Ok(TimerHandle(format!("stub:{thread_id}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> nexus_chat::error::Result<()> {
        Ok(())
    }
    fn service_fault(&self) -> Option<nxs_service::ServiceFault> {
        Some(nxs_service::ServiceFault::NotRunning)
    }
}

struct Seam {
    defs: Definitions,
    worker: DryWorker,
    origin: String,
    db_path: String,
    timer: HealthyButUnattended,
}

impl Seam {
    fn open(tmp: &TempDir) -> (Seam, ChatStore) {
        let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
        let store = ws.open_chat_store().expect("open chat store");
        let seam = Seam {
            defs: Definitions::from_dir(&tmp.path().join(".nxs-personas")).expect("declarations"),
            worker: DryWorker { log: None },
            origin: "local".to_string(),
            db_path: ws.db_path().display().to_string(),
            timer: HealthyButUnattended,
        };
        (seam, store)
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            now: NOW,
            origin: &self.origin,
            actor: "carsten",
            session: None,
            hop: 0,
            defs: &self.defs,
            worker: &self.worker,
            timer: &self.timer,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &self.db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        }
    }
}

#[test]
fn the_library_seam_puts_the_class_on_a_send_receipt_an_app_reads() {
    let tmp = workspace();
    let (seam, mut store) = Seam::open(&tmp);
    let receipt = surface::send_to(
        &seam.ctx(),
        &mut store,
        SendToRequest {
            machine: None,
            to: "coder",
            body: "have a look",
            refs: SendToRefs::ExplicitlyNone,
        },
    )
    .expect("the send itself succeeds — the machine is what is broken");

    let entry = receipt
        .warnings
        .iter()
        .find(|w| w.class == nexus_chat::orchestration::ConsequenceClass::ServiceNotRunning)
        .expect("an app reads this off the receipt or it reads it nowhere");
    assert_eq!(entry.thread.as_deref(), Some(receipt.thread_id.as_str()));
    assert!(entry.reason.is_none(), "nothing was being started or woken");
    assert!(
        entry.detail.contains("neither pushes nor pulls"),
        "both halves reach the app too: {}",
        entry.detail
    );
}

#[test]
fn the_library_seam_puts_it_on_a_reply_receipt_as_well() {
    let tmp = workspace();
    let (seam, mut store) = Seam::open(&tmp);
    let thread = surface::send_to(
        &seam.ctx(),
        &mut store,
        SendToRequest {
            machine: None,
            to: "coder",
            body: "have a look",
            refs: SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send")
    .thread_id;

    let receipt = surface::reply_in_thread(
        &seam.ctx(),
        &mut store,
        ReplyThreadRequest {
            machine: None,
            thread: &thread,
            body: "looked",
            escalate: false,
            needs_rework: false,
            accept: false,
        },
    )
    .expect("reply");
    assert!(
        receipt
            .warnings
            .iter()
            .any(|w| w.class == nexus_chat::orchestration::ConsequenceClass::ServiceNotRunning),
        "the third of the three verbs that carry a warnings array: {receipt:?}"
    );
}

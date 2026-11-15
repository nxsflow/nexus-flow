//! **The smoke run on the real backend** (6j6v.pn9c; epic 6j6v.zjyv §12.10, DoD 12; b7ck notes b7py and 131c):
//! the checks over a run the coordinator commissioned from the terminal on `coding-v2`, stepped by the INSTALLED
//! development service under launchd. Ignored, and inert without `NXR_SMOKE=1`: CI never runs it, and a
//! developer runs it only after the procedure on 6j6v.pn9c has produced a finished run.
//!
//! The commission, the service, the network cut and the timings are the procedure's (the note on 6j6v.pn9c);
//! this file asserts what the run left behind, from the directories and files the run used:
//!
//! - DoD 12 from the real v2 directory: `nxf`, `nxm` and `nxc` resolve the root's workspace, and the old
//!   engine's verbs are refused there naming the root;
//! - every session's start spec: its working directory is `<root>-v2`, its environment exactly the allowlist,
//!   no API key, `claude` and the sidecar on absolute paths, and the sidecar the bundle this build embedded —
//!   never `crates/chat/build.rs`'s fallback to the source tree's `agent-sidecar/src/main.mjs`;
//! - the login: each session started with a backend session id and completed a turn, and its sidecar log
//!   names no authentication failure;
//! - the run finished, and — when the procedure ran it — the network-cut run ended refused, not hung;
//! - the counts the procedure records: events, facts, sessions, and their rate.
//!
//! Inputs, all from the environment: `NXR_SMOKE_ROOT` (the registered root), `NXR_SMOKE_NXS` (the development
//! build's `nxs`, the binary the service runs), `NXR_SMOKE_THREAD` (the smoke run's root thread),
//! `NXR_SMOKE_TICKET` (the ticket it was commissioned with) and, optionally, `NXR_SMOKE_CUT_THREAD` (the
//! network-cut run's root thread).

use std::path::{Path, PathBuf};
use std::process::Command;

use nexus_chat::v2_bridge::{root_of_v2_dir, run_sqlite, RunReader};
use nexus_run_contracts::{RunId, RunStore};
use serde_json::Value;

/// What the session environment may hold: the three the host takes from the service, and its own `NXC_*`.
const ALLOWED: [&str; 3] = ["PATH", "HOME", "USER"];
/// What a failed login says in a sidecar log (b7py note, step 4c).
const NOT_LOGGED_IN: [&str; 4] = [
    "Failed to authenticate",
    "OAuth",
    "Native CLI binary not found",
    "not logged in",
];

struct Smoke {
    root: PathBuf,
    v2: PathBuf,
    nxs: PathBuf,
    thread: String,
    ticket: String,
    cut_thread: Option<String>,
}

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required with NXR_SMOKE=1"))
}

impl Smoke {
    fn from_env() -> Option<Smoke> {
        if std::env::var("NXR_SMOKE").as_deref() != Ok("1") {
            eprintln!("v2_smoke: NXR_SMOKE is not 1, nothing to check");
            return None;
        }
        let root = PathBuf::from(var("NXR_SMOKE_ROOT"));
        let mut v2 = root.clone().into_os_string();
        v2.push("-v2");
        Some(Smoke {
            v2: PathBuf::from(v2),
            nxs: PathBuf::from(var("NXR_SMOKE_NXS")),
            thread: var("NXR_SMOKE_THREAD"),
            ticket: var("NXR_SMOKE_TICKET"),
            cut_thread: std::env::var("NXR_SMOKE_CUT_THREAD").ok(),
            root,
        })
    }

    fn db_path(&self) -> String {
        self.root
            .join(".nxs")
            .join("db.sqlite")
            .to_string_lossy()
            .into_owned()
    }

    /// `nxs <args>` in `dir`, as the owner at the terminal: no session, and the instance the binary decides.
    fn nxs(&self, dir: &Path, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(&self.nxs);
        command.current_dir(dir).args(args);
        for (name, _) in std::env::vars() {
            if name.starts_with("NXC_") || name == "NXF_DB" {
                command.env_remove(name);
            }
        }
        command.output().expect("the development nxs runs")
    }

    fn json(&self, dir: &Path, args: &[&str]) -> Value {
        let out = self.nxs(dir, args);
        assert!(
            out.status.success(),
            "nxs {args:?} in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("JSON")
    }

    fn origin(&self) -> String {
        let ws =
            nexus_chat::workspace::Workspace::resolve(None, &self.root).expect("the root resolves");
        nexus_chat::workspace::origin_of(&ws).to_owned()
    }

    fn run_of(&self, thread: &str) -> RunId {
        RunReader::at_workspace(&self.db_path(), &self.origin())
            .expect("run.sqlite answers")
            .expect("the root has a run.sqlite")
            .run_of_thread(thread)
            .expect("run.sqlite answers")
            .unwrap_or_else(|| panic!("{thread} is no run on the new engine"))
    }

    fn events(&self, run: &RunId) -> Vec<nexus_run_contracts::Event> {
        nexus_run_host::bridge::open_store(&run_sqlite(&self.db_path()))
            .expect("run.sqlite opens")
            .expect("run.sqlite exists")
            .load(run, 0)
            .expect("the run's log reads")
    }

    /// Every session ledger of `run`, by session key.
    fn ledgers(&self, run: &RunId) -> Vec<PathBuf> {
        let base = self.root.join(".nxs").join("agent-logs").join("v2");
        let mut found = Vec::new();
        let mut stack = vec![base.join(run.to_string())];
        while let Some(dir) = stack.pop() {
            if dir.join("spec.json").is_file() {
                found.push(dir.clone());
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        stack.push(entry.path());
                    }
                }
            }
        }
        found.sort();
        found
    }
}

#[test]
#[ignore = "the smoke run: NXR_SMOKE=1 and the procedure on 6j6v.pn9c"]
fn the_smoke_run_on_the_real_backend_left_what_the_new_engine_promises() {
    let Some(smoke) = Smoke::from_env() else {
        return;
    };
    assert_eq!(
        root_of_v2_dir(&smoke.v2).as_deref(),
        Some(smoke.root.as_path()),
        "{} is the root's v2 directory",
        smoke.v2.display()
    );

    // DoD 12: the same workspace from the v2 directory, through its `.nxs` link.
    let ticket_args = ["flow", "show", smoke.ticket.as_str(), "--json"];
    assert_eq!(
        smoke.json(&smoke.v2, &ticket_args),
        smoke.json(&smoke.root, &ticket_args),
        "nxf answers the same from both"
    );
    let doc = smoke.nxs(&smoke.v2, &["memory", "doc", "--check"]);
    assert!(
        doc.status.success(),
        "nxm resolves the memory store from the v2 directory: {}",
        String::from_utf8_lossy(&doc.stderr)
    );
    let run = smoke.run_of(&smoke.thread);
    let status_args = [
        "chat",
        "--json",
        "status",
        "--thread",
        smoke.thread.as_str(),
    ];
    for dir in [&smoke.v2, &smoke.root] {
        let status = smoke.json(dir, &status_args).to_string();
        assert!(
            status.contains(&run.to_string()),
            "nxc reads run {run} from {}: {status}",
            dir.display()
        );
    }
    // And the old engine starts nothing there: a send to a persona is refused, naming the root.
    let refused = smoke.nxs(
        &smoke.v2,
        &["chat", "send", "--to", "pm", "smoke: must be refused"],
    );
    assert!(
        !refused.status.success(),
        "the old engine's send is refused in the v2 directory"
    );
    let said = String::from_utf8_lossy(&refused.stderr).into_owned()
        + &String::from_utf8_lossy(&refused.stdout);
    assert!(
        said.contains(&smoke.root.display().to_string()),
        "it names the root: {said}"
    );

    // The run finished on the new engine.
    let events = smoke.events(&run);
    let kinds: Vec<String> = events.iter().map(|e| e.body.kind().to_owned()).collect();
    assert!(
        kinds.iter().any(|k| k == "finished"),
        "the run finished: {kinds:?}"
    );

    // Every session: where it ran, what it was given, and that it logged in.
    let ledgers = smoke.ledgers(&run);
    assert!(!ledgers.is_empty(), "the run started sessions");
    let mut facts = 0usize;
    for ledger in &ledgers {
        let spec: Value =
            serde_json::from_slice(&std::fs::read(ledger.join("spec.json")).unwrap()).unwrap();
        let at = ledger.display();
        assert_eq!(
            spec["cwd"].as_str().map(PathBuf::from),
            Some(smoke.v2.clone()),
            "{at}: the session works in the v2 directory"
        );
        let env: Vec<(String, String)> = spec["env"]
            .as_array()
            .expect("an env")
            .iter()
            .map(|v| {
                (
                    v["name"].as_str().unwrap().to_owned(),
                    v["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        for (name, _) in &env {
            assert!(
                ALLOWED.contains(&name.as_str()) || name.starts_with("NXC_"),
                "{at}: {name} is outside the allowlist"
            );
        }
        for name in ALLOWED {
            assert!(env.iter().any(|(n, _)| n == name), "{at}: {name} is given");
        }
        assert!(
            !env.iter().any(|(n, _)| n == "ANTHROPIC_API_KEY"),
            "{at}: no API key: the login is the keychain's"
        );
        let claude = spec["claude_path"].as_str().unwrap_or_default();
        assert!(
            Path::new(claude).is_absolute() && Path::new(claude).exists(),
            "{at}: claude on an absolute path that exists: {claude:?}"
        );
        let sidecar = env
            .iter()
            .find(|(n, _)| n == "NXC_SIDECAR")
            .map(|(_, v)| PathBuf::from(v))
            .expect("the sidecar is named");
        assert!(
            sidecar.is_absolute() && sidecar.is_file(),
            "{at}: the sidecar exists: {}",
            sidecar.display()
        );
        assert!(
            sidecar
                .file_name()
                .is_some_and(|n| n == "nxc-agent-sidecar.mjs")
                && !sidecar.ends_with("agent-sidecar/src/main.mjs"),
            "{at}: the bundle build.rs embedded, never the source-tree fallback: {}",
            sidecar.display()
        );
        let log = std::fs::read_to_string(ledger.join("sidecar.log")).unwrap_or_default();
        for words in NOT_LOGGED_IN {
            assert!(!log.contains(words), "{at}: the sidecar log says {words:?}");
        }
        let lines = std::fs::read_to_string(ledger.join("facts.jsonl")).unwrap_or_default();
        facts += lines.lines().count();
        let started = lines.lines().any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|fact| {
                fact["kind"] == "started"
                    && fact["backend_session_id"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty())
            })
        });
        assert!(
            started,
            "{at}: the session started with a backend session id"
        );
        assert!(
            lines.contains("\"usage\""),
            "{at}: the session completed at least one turn"
        );
    }

    // The network-cut run, where the procedure ran one: refused within its window, never hung.
    if let Some(cut) = &smoke.cut_thread {
        let cut_run = smoke.run_of(cut);
        let cut_kinds: Vec<String> = smoke
            .events(&cut_run)
            .iter()
            .map(|e| e.body.kind().to_owned())
            .collect();
        assert!(
            cut_kinds.iter().any(|k| k == "tree_refused"),
            "the run with the network cut ended refused: {cut_kinds:?}"
        );
    }

    // What the procedure records on the ticket.
    let first = events
        .first()
        .map(|e| e.at.unix_millis())
        .unwrap_or_default();
    let last = events
        .last()
        .map(|e| e.at.unix_millis())
        .unwrap_or_default();
    let minutes = ((last - first) as f64 / 60_000.0).max(1.0 / 60.0);
    println!(
        "v2_smoke: run {run} events={} sessions={} facts={} minutes={minutes:.1} events_per_min={:.1} facts_per_min={:.1}",
        events.len(),
        ledgers.len(),
        facts,
        events.len() as f64 / minutes,
        facts as f64 / minutes
    );
}

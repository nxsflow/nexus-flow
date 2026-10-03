//! **Two workspaces on one machine, through the real binary** (nxf 6j6v.70dy) — the host half the
//! chat library cannot test itself: a workspace's stored name (`nxs name`), trusting a neighbour by
//! name (`nxs sync trust add --workspace`), the service registry as the directory of workspaces, and
//! the other side's handover run as a child process in the other directory.
//!
//! The persona sessions are the dry worker's — each start is a line in a log — so what is proved
//! is the routing, not a model. The live acceptance with real sessions is a manual run against two
//! repositories (see the PR).

use std::path::{Path, PathBuf};
use std::process::Command;

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

struct Machine {
    home: TempDir,
    _dirs: TempDir,
    alpha: PathBuf,
    beta: PathBuf,
}

impl Machine {
    fn dry_log(&self) -> PathBuf {
        self.home.path().join("starts.log")
    }

    fn starts(&self) -> String {
        std::fs::read_to_string(self.dry_log()).unwrap_or_default()
    }

    fn nxs(&self, dir: &Path) -> Command {
        nxs_test_support::assert_multicall_binary_fresh();
        let mut c = Command::new(assert_cmd::cargo::cargo_bin("nxs"));
        c.current_dir(dir)
            .pin_home(self.home.path())
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", self.dry_log())
            .env("NXC_TIMER", "dry")
            .env("NXC_ACTOR", "carsten")
            .env_remove("NXC_SESSION")
            .env_remove("NXC_ORIGIN")
            .env_remove("NXC_DB")
            .env_remove("NXC_HOP");
        c
    }

    fn json(&self, dir: &Path, args: &[&str]) -> Value {
        let out = self.nxs(dir).args(args).output().expect("nxs runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "no JSON from `nxs {}` ({e}): {stdout}\nstderr: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    }

    fn run(&self, dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = self.nxs(dir).args(args).output().expect("nxs runs");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

fn git_repo(dir: &Path, remote: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for args in [vec!["init", "-q"], vec!["remote", "add", "origin", remote]] {
        let ok = Command::new("git")
            .args(&args)
            .current_dir(dir)
            .status()
            .expect("git runs")
            .success();
        assert!(ok, "git {args:?}");
    }
}

fn machine() -> Machine {
    let home = TempDir::new().unwrap();
    let dirs = TempDir::new().unwrap();
    let alpha = dirs.path().join("alpha");
    let beta = dirs.path().join("beta");
    git_repo(&alpha, "git@github.com:test/alpha.git");
    git_repo(&beta, "https://github.com/Test/Beta.git");
    let m = Machine {
        home,
        _dirs: dirs,
        alpha,
        beta,
    };
    for dir in [&m.alpha, &m.beta] {
        let (ok, _, err) = m.run(dir, &["init", "--json", "--module", "chat", "--no-service"]);
        assert!(ok, "init: {err}");
    }
    std::fs::create_dir_all(m.alpha.join(".nxs-personas")).unwrap();
    std::fs::write(
        m.alpha.join(".nxs-personas/pm.yaml"),
        "handle: pm\nsystem_prompt: You are the PM of test/alpha.\n",
    )
    .unwrap();
    std::fs::create_dir_all(m.beta.join(".nxs-personas")).unwrap();
    std::fs::write(
        m.beta.join(".nxs-personas/pm.yaml"),
        "handle: pm\nsystem_prompt: You are the PM of test/beta.\naddressable:\n  humans: true\n  external:\n    - \"*/pm\"\n",
    )
    .unwrap();
    m
}

#[test]
fn a_workspace_is_named_after_its_git_origin_and_the_name_is_stored() {
    let m = machine();
    let (ok, out, _) = m.run(&m.alpha, &["name"]);
    assert!(ok);
    assert_eq!(out.trim(), "test/alpha");
    // The host is dropped and the case folded, whatever spelling the remote used.
    let (_, out, _) = m.run(&m.beta, &["name"]);
    assert_eq!(out.trim(), "test/beta");
    let config = std::fs::read_to_string(m.alpha.join(".nxs/config.toml")).unwrap();
    assert!(
        config.contains("[workspace]") && config.contains("name = \"test/alpha\""),
        "{config}"
    );

    // Set explicitly, it wins over the origin from then on.
    let (ok, _, _) = m.run(&m.alpha, &["name", "acme/alpha"]);
    assert!(ok);
    let (_, out, _) = m.run(&m.alpha, &["name"]);
    assert_eq!(out.trim(), "acme/alpha");
    let (ok, _, err) = m.run(&m.alpha, &["name", "NotAName"]);
    assert!(!ok && err.contains("not a workspace name"), "{err}");
}

#[test]
fn a_neighbour_is_trusted_by_name_through_the_registry() {
    let m = machine();
    let added = m.json(
        &m.alpha,
        &["--json", "sync", "trust", "add", "--workspace", "test/beta"],
    );
    assert_eq!(added["workspace"], "test/beta", "{added}");
    assert_eq!(added["added"], true, "{added}");
    let listed = m.json(&m.alpha, &["--json", "sync", "trust", "list"]);
    assert!(
        listed.to_string().contains("test/beta"),
        "the key is listed under the workspace's name: {listed}"
    );
    let (ok, _, err) = m.run(
        &m.alpha,
        &["sync", "trust", "add", "--workspace", "test/gamma"],
    );
    assert!(
        !ok && err.contains("no workspace named test/gamma"),
        "{err}"
    );
}

#[test]
fn a_commission_crosses_to_the_other_workspace_and_its_coordinator_starts_the_persona_there() {
    let m = machine();
    for (dir, other) in [(&m.alpha, "test/beta"), (&m.beta, "test/alpha")] {
        let (ok, _, err) = m.run(dir, &["sync", "trust", "add", "--workspace", other]);
        assert!(ok, "{err}");
    }
    // The person commissions alpha's pm; the dry worker "starts" it and names its session.
    let first = m.json(
        &m.alpha,
        &[
            "chat",
            "--json",
            "send",
            "--to",
            "pm",
            "--no-ref",
            "Ask test/beta.",
        ],
    );
    let session = first["session"].as_str().expect("a session").to_string();

    // As that session, the pm commissions beta's pm by its address.
    let out = m
        .nxs(&m.alpha)
        .env("NXC_SESSION", &session)
        .args([
            "chat",
            "--json",
            "send",
            "--to",
            "test/beta/pm",
            "--no-ref",
            "When does it ship?",
        ])
        .output()
        .unwrap();
    let receipt: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(receipt["border"]["peer"], "test/beta", "{receipt}");
    assert_eq!(receipt["border"]["state"], "working", "{receipt}");

    // Beta's own handover — a child process in beta's directory — admitted it and started beta's pm.
    let starts = m.starts();
    assert!(
        starts.contains("role=pm") && starts.contains("coordinator=border"),
        "beta's pm was started by its border coordinator: {starts}"
    );

    // Both workspaces show the thread with the other side and its state.
    let thread = receipt["thread_id"].as_str().unwrap();
    for (dir, peer, direction) in [
        (&m.alpha, "test/beta", "outbound"),
        (&m.beta, "test/alpha", "inbound"),
    ] {
        let status = m.json(dir, &["chat", "--json", "status", "--all"]);
        let row = status["operations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|op| op["threads"].as_array().unwrap().iter())
            .find(|t| t["thread_id"] == thread)
            .unwrap_or_else(|| panic!("{thread} in {}: {status}", dir.display()))
            .clone();
        assert_eq!(row["border"]["peer"], peer, "{row}");
        assert_eq!(row["border"]["direction"], direction, "{row}");
        assert_eq!(row["border"]["state"], "working", "{row}");
    }
    // And the service has something to carry while it is open.
    assert!(m.alpha.join(".nxs/border-open").exists());
    assert!(m.beta.join(".nxs/border-open").exists());
}

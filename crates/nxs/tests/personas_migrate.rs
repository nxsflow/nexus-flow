//! **`nxs personas migrate`, through the real binary** (nxf 6j6v.h9ee) — the verb over
//! `nexus_chat::persona_migration`: the plan writes nothing, the run rewrites the folder and says
//! per file what it left out, the team reads the same afterwards, a second run has nothing to do,
//! and `--user` migrates the user-level folder. The rewrite itself is pinned field by field in
//! `crates/chat/tests/personas_migrate.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

/// Named so a build reads a user-level folder (nxf 6j6v.7k58).
const INSTANCE: &str = "nexus-flow-personas-migrate";

const PM: &str = "\
# The PM.
handle: pm
job_description: Turns an idea into board items, in a sentence long enough.
addressable:
  humans: true
address_book:
  - to: coder
    why: hand over a work order
system_prompt: |
  You are the PM.
";

const CODER: &str = "\
handle: coder
job_description: Builds what the PM planned, test first, in a sentence.
address_book: []
session: continue
system_prompt: You are the coder.
";

struct Repo {
    home: TempDir,
    dir: TempDir,
}

impl Repo {
    fn new() -> Repo {
        let repo = Repo {
            home: TempDir::new().unwrap(),
            dir: TempDir::new().unwrap(),
        };
        let (ok, _, err) = repo.run(&["init", "--json", "--module", "chat", "--no-service"]);
        assert!(ok, "init: {err}");
        let personas = repo.personas();
        std::fs::write(personas.join("pm.yaml"), PM).unwrap();
        std::fs::write(personas.join("coder.yaml"), CODER).unwrap();
        std::fs::write(
            personas.join("channels.yaml"),
            "# The rounds.\n\n- name: planning\n  members: [pm]\n",
        )
        .unwrap();
        repo
    }

    fn personas(&self) -> PathBuf {
        self.dir.path().join(".nxs-personas")
    }

    fn user_personas(&self) -> PathBuf {
        self.home
            .path()
            .join(".nexusflow-personas-migrate/personas")
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        nxs_test_support::assert_multicall_binary_fresh();
        let out = Command::new(assert_cmd::cargo::cargo_bin("nxs"))
            .current_dir(self.dir.path())
            .pin_home(self.home.path())
            .env("NXS_SERVICE_INSTANCE", INSTANCE)
            .env_remove("NXC_SESSION")
            .env_remove("NXC_DB")
            .args(args)
            .output()
            .expect("nxs runs");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn json(&self, args: &[&str]) -> Value {
        let (ok, out, err) = self.run(args);
        assert!(ok, "nxs {}: {out}{err}", args.join(" "));
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"))
    }
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = walk(dir)
        .into_iter()
        .map(|p| p.strip_prefix(dir).unwrap().display().to_string())
        .collect();
    names.sort();
    names
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        match entry.path().is_dir() {
            true => out.extend(walk(&entry.path())),
            false => out.push(entry.path()),
        }
    }
    out
}

#[test]
fn the_plan_writes_nothing_the_run_rewrites_and_a_second_run_has_nothing_to_do() {
    let repo = Repo::new();
    let before = listing(&repo.personas());
    let team_before = repo.json(&["chat", "--json", "list"]);

    let plan = repo.json(&["personas", "--json", "migrate", "--dry-run"]);
    assert_eq!(plan["applied"], false);
    assert_eq!(plan["personas"].as_array().unwrap().len(), 2);
    assert_eq!(
        listing(&repo.personas()),
        before,
        "the plan wrote something"
    );

    let (ok, out, err) = repo.run(&["personas", "migrate"]);
    assert!(ok, "{out}{err}");
    assert!(
        out.contains("pm.yaml -> pm/SKILL.md  — dropped: address_book (1 entry)"),
        "{out}"
    );
    assert!(
        out.contains("coder.yaml -> coder/SKILL.md  — dropped: address_book ([]), session"),
        "{out}"
    );
    assert!(
        out.contains("channels.yaml -> channels/planning.yaml"),
        "{out}"
    );
    assert!(out.contains("2 address books dropped"), "{out}");
    let after = listing(&repo.personas());
    for expected in [
        "pm/SKILL.md",
        "coder/SKILL.md",
        "channels/planning.yaml",
        "channels/README.md",
    ] {
        assert!(
            after.iter().any(|p| p == expected),
            "{expected} in {after:?}"
        );
    }
    assert!(!after
        .iter()
        .any(|p| p.ends_with(".yaml") && !p.starts_with("channels/")));

    // The same team, as a human and an app see it — the source aside.
    let team_after = repo.json(&["chat", "--json", "list"]);
    let strip = |v: &Value| {
        let mut v = v.clone();
        for kind in ["personas", "channels"] {
            for entry in v[kind].as_array_mut().unwrap() {
                entry.as_object_mut().unwrap().remove("declaration");
            }
        }
        v.as_object_mut().unwrap().remove("declarations");
        v
    };
    assert_eq!(strip(&team_after), strip(&team_before));

    let (ok, out, _) = repo.run(&["personas", "migrate"]);
    assert!(ok);
    assert!(out.starts_with("Nothing to migrate in "), "{out}");
}

#[test]
fn user_migrates_the_user_level_folder() {
    let repo = Repo::new();
    let user = repo.user_personas();
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(
        user.join("helper.yaml"),
        "handle: helper\nsystem_prompt: You help.\n",
    )
    .unwrap();
    let report = repo.json(&["personas", "--json", "migrate", "--user"]);
    assert_eq!(report["folder"], user.display().to_string());
    assert!(user.join("helper/SKILL.md").is_file());
    assert!(
        repo.personas().join("pm.yaml").is_file(),
        "the repository's own folder is untouched"
    );
}

#[test]
fn a_key_it_cannot_place_stops_the_run_naming_the_file() {
    let repo = Repo::new();
    std::fs::write(
        repo.personas().join("odd.yaml"),
        "handle: odd\nsystem_prompt: Odd.\nfavourite_colour: blue\n",
    )
    .unwrap();
    let before = listing(&repo.personas());
    let (ok, out, err) = repo.run(&["personas", "migrate"]);
    assert!(!ok, "{out}");
    assert!(
        err.contains("odd.yaml") && err.contains("`favourite_colour`"),
        "{err}"
    );
    assert_eq!(listing(&repo.personas()), before);
}

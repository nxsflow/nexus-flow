//! **A published skill is a persona, through the real binary** (nxf 6j6v.dw16, epic 6j6v.phcx):
//! a skill folder nobody here wrote — Anthropic's `internal-comms`, vendored UNCHANGED under
//! `tests/fixtures/skills/` (see `PROVENANCE.md` there) — copied into `.nxs-personas/` is listed,
//! addressed and started, with no rewriting and no command; the same folder in the user-level
//! folder is a persona in two repositories that declare nothing of their own; and a name declared
//! in both forms is refused, naming both files.
//!
//! The persona sessions are the dry worker's — each start is a line in a log — so what is proved
//! here is the reading and the routing. The live acceptance, a real model answering from the
//! skill's own `examples/`, is the `#[ignore]`d test at the end.

use std::path::{Path, PathBuf};
use std::process::Command;

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

/// The development instance these tests run under — named so a build reads a user-level folder
/// (nxf 6j6v.7k58).
const INSTANCE: &str = "nexus-flow-skill-persona";

/// The vendored skill folder.
fn published_skill() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills/internal-comms")
}

/// Copy a folder as `cp -R` would — the way a person installs a skill they found.
fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        match entry.path().is_dir() {
            true => copy_folder(&entry.path(), &target),
            false => {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

struct Machine {
    home: TempDir,
    _dirs: TempDir,
    alpha: PathBuf,
    beta: PathBuf,
}

impl Machine {
    fn user_personas(&self) -> PathBuf {
        self.home.path().join(".nexusflow-skill-persona/personas")
    }

    fn starts(&self) -> String {
        std::fs::read_to_string(self.home.path().join("starts.log")).unwrap_or_default()
    }

    fn nxs(&self, dir: &Path) -> Command {
        nxs_test_support::assert_multicall_binary_fresh();
        let mut c = Command::new(assert_cmd::cargo::cargo_bin("nxs"));
        c.current_dir(dir)
            .pin_home(self.home.path())
            .env("NXS_SERVICE_INSTANCE", INSTANCE)
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", self.home.path().join("starts.log"))
            .env("NXC_TIMER", "dry")
            .env("NXC_ACTOR", "carsten")
            .env_remove("NXC_SESSION")
            .env_remove("NXC_ORIGIN")
            .env_remove("NXC_DB")
            .env_remove("NXC_HOP");
        c
    }

    fn run(&self, dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = self.nxs(dir).args(args).output().expect("nxs runs");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn json(&self, dir: &Path, args: &[&str]) -> Value {
        let (_, stdout, stderr) = self.run(dir, args);
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "no JSON from `nxs {}` ({e}): {stdout}\nstderr: {stderr}",
                args.join(" ")
            )
        })
    }
}

fn machine() -> Machine {
    let home = TempDir::new().unwrap();
    let dirs = TempDir::new().unwrap();
    let m = Machine {
        alpha: dirs.path().join("alpha"),
        beta: dirs.path().join("beta"),
        home,
        _dirs: dirs,
    };
    for dir in [&m.alpha, &m.beta] {
        std::fs::create_dir_all(dir).unwrap();
        let (ok, _, err) = m.run(dir, &["init", "--json", "--module", "chat", "--no-service"]);
        assert!(ok, "init: {err}");
    }
    m
}

fn persona<'a>(list: &'a Value, handle: &str) -> &'a Value {
    list["personas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["handle"] == handle)
        .unwrap_or_else(|| panic!("{handle} is not listed: {list:#}"))
}

#[test]
fn a_published_skill_copied_unchanged_is_listed_addressed_and_started() {
    let m = machine();
    let folder = m.alpha.join(".nxs-personas/internal-comms");
    copy_folder(&published_skill(), &folder);

    let list = m.json(&m.alpha, &["chat", "--json", "list"]);
    let entry = persona(&list, "internal-comms");
    assert!(
        entry["job_description"].as_str().unwrap().starts_with(
            "A set of resources to help me write all kinds of internal communications"
        ),
        "the skill's description is the persona's: {entry:#}"
    );
    let declaration = &entry["declaration"];
    assert_eq!(declaration["form"], "skill", "{entry:#}");
    let folder = std::fs::canonicalize(&folder).unwrap();
    assert_eq!(
        declaration["file"],
        folder.join("SKILL.md").display().to_string()
    );
    assert_eq!(declaration["folder"], folder.display().to_string());
    assert_eq!(declaration["license"], "Complete terms in LICENSE.txt");

    // To a human a persona is a persona: the list does not name the form.
    let (ok, human, err) = m.run(&m.alpha, &["chat", "list"]);
    assert!(ok, "{err}");
    assert!(human.contains("`internal-comms`"), "{human}");
    assert!(
        !human.contains("SKILL.md") && !human.contains("skill form"),
        "{human}"
    );

    // Addressed, and started.
    let (ok, out, err) = m.run(
        &m.alpha,
        &[
            "chat",
            "--json",
            "send",
            "--to",
            "internal-comms",
            "--no-ref",
            "Write a 3P update for the platform team.",
        ],
    );
    assert!(ok, "send: {out}{err}");
    assert!(
        m.starts().contains("role=internal-comms"),
        "the persona was started: {}",
        m.starts()
    );
}

#[test]
fn the_same_skill_in_the_user_level_folder_is_a_persona_in_two_repositories() {
    let m = machine();
    let folder = m.user_personas().join("internal-comms");
    copy_folder(&published_skill(), &folder);
    for dir in [&m.alpha, &m.beta] {
        assert!(!dir.join(".nxs-personas/internal-comms").exists());
        let list = m.json(dir, &["chat", "--json", "list"]);
        let entry = persona(&list, "internal-comms");
        assert_eq!(entry["origin"], "user", "{entry:#}");
        // Its references mean ITS folder — the folder `--json` names.
        assert_eq!(
            entry["declaration"]["folder"],
            folder.display().to_string(),
            "{entry:#}"
        );
        assert!(folder.join("examples/3p-updates.md").is_file());
    }
}

#[test]
fn a_name_in_both_forms_is_refused_naming_both_files() {
    let m = machine();
    let personas = m.alpha.join(".nxs-personas");
    copy_folder(&published_skill(), &personas.join("internal-comms"));
    std::fs::write(
        personas.join("internal-comms.yaml"),
        "handle: internal-comms\nsystem_prompt: The old form.\n",
    )
    .unwrap();
    let (ok, out, err) = m.run(&m.alpha, &["chat", "list"]);
    assert!(!ok, "refused: {out}");
    assert!(
        err.contains("internal-comms.yaml") && err.contains("internal-comms/SKILL.md"),
        "both files are named: {err}"
    );
    assert!(err.contains("nxs personas migrate"), "{err}");
}

/// **The live acceptance** (nxf 6j6v.dw16 / epic 6j6v.phcx, acceptance 1 and 2): the published
/// skill, unchanged, answers through a real model — once copied into a repository, once from the
/// user-level folder into a repository that declares nothing — and both times it reads ITS OWN
/// `examples/3p-updates.md`, which is only reachable if the relative reference in its body meant
/// the skill's folder.
///
/// `#[ignore]`d for the reasons every live smoke here is: real subscription auth, real seconds,
/// real cost. It does NOT pin `HOME` — the SDK's credentials live there — so the registry it
/// writes is a development instance of its own, removed at the end. Run it by hand where `claude`
/// is authenticated:
///
/// ```console
/// $ (cd agent-sidecar && npm ci)
/// $ cargo build -p nxs
/// $ cargo test -p nxs --test a_published_skill_is_a_persona -- --ignored --nocapture
/// ```
#[test]
#[ignore = "live: real Claude Agent SDK sessions"]
fn live_a_published_skill_answers_from_its_own_folder() {
    const INSTANCE: &str = "nexus-flow-skill-live";
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits at <root>/crates/nxs")
        .to_path_buf();
    let sidecar = repo.join("agent-sidecar/src/main.mjs");
    assert!(sidecar.is_file(), "no sidecar at {sidecar:?}");
    nxs_test_support::assert_multicall_binary_fresh();
    let bin = assert_cmd::cargo::cargo_bin("nxs");
    let bin_dir = bin.parent().unwrap().to_path_buf();
    let service_home = nxs_service::ServiceHome::for_instance(
        nxs_service::Instance::named(INSTANCE).expect("a valid instance name"),
    )
    .expect("a home directory")
    .root()
    .to_path_buf();
    assert!(service_home.ends_with(".nexusflow-skill-live"));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let user_personas = service_home.join("personas");
    let _cleanup = Cleanup(service_home);

    let dirs = TempDir::new().unwrap();
    let (alpha, beta) = (dirs.path().join("alpha"), dirs.path().join("beta"));
    let nxs = |dir: &Path| {
        let mut c = Command::new(&bin);
        c.current_dir(dir)
            .env("NXS_SERVICE_INSTANCE", INSTANCE)
            .env(
                "PATH",
                format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap()),
            )
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", &sidecar)
            .env("NXC_ACTOR", "owner")
            .env_remove("NXC_SESSION")
            .env_remove("NXC_DB")
            .env_remove("NXC_ORIGIN")
            .env_remove("NXC_HOP")
            .env_remove("NXC_NOW")
            .env_remove("NXC_TIMER")
            .env_remove("ANTHROPIC_API_KEY");
        c
    };
    for dir in [&alpha, &beta] {
        std::fs::create_dir_all(dir).unwrap();
        let ok = nxs(dir)
            .args(["init", "--json", "--module", "chat", "--no-service"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }
    // Alpha: copied into the repository. Beta: nothing of its own, the user-level copy.
    copy_folder(
        &published_skill(),
        &alpha.join(".nxs-personas/internal-comms"),
    );
    copy_folder(&published_skill(), &user_personas.join("internal-comms"));
    assert!(!beta.join(".nxs-personas/internal-comms").exists());

    // A question only the skill's own `examples/3p-updates.md` answers: its instructions say what
    // the three Ps stand for, and the file is nowhere in either repository.
    let ask = "Read your guideline file for 3P updates (examples/3p-updates.md). Reply with \
               `nxc reply` in ONE line: the exact words the file says 3P stands for, in quotes.";
    let mut threads = Vec::new();
    for dir in [&alpha, &beta] {
        let out = nxs(dir)
            .args([
                "chat",
                "--json",
                "send",
                "--to",
                "internal-comms",
                "--no-ref",
                ask,
            ])
            .output()
            .unwrap();
        let receipt: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
        threads.push((
            dir.clone(),
            receipt["thread_id"].as_str().unwrap().to_string(),
        ));
    }
    for (dir, thread) in threads {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(400);
        let answer = loop {
            let out = nxs(&dir)
                .args(["chat", "--json", "threads", "show", &thread])
                .output()
                .unwrap();
            let board: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            if let Some(body) = board["messages"].as_array().and_then(|ms| {
                ms.iter()
                    .find(|m| {
                        m["sender"]
                            .as_str()
                            .is_some_and(|s| s.ends_with("/internal-comms"))
                    })
                    .and_then(|m| m["body"].as_str())
            }) {
                break body.to_string();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "internal-comms never answered in {}: {board:#}",
                dir.display()
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
        };
        eprintln!("{}: {answer}", dir.display());
        assert!(
            !answer.starts_with("sidecar:"),
            "the persona answered itself: {answer}"
        );
        assert!(
            answer.contains("Progress, Plans, Problems"),
            "the answer came from the skill's own examples/ in {}: {answer}",
            dir.display()
        );
    }
}

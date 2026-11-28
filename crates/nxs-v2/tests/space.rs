//! `nxs space create-coordination [--name]`, `join-coordination <space id> [--name]` and
//! `leave-coordination` (RFC 0001, "Creating and finding a space"; RFC 0009, "Creating it",
//! "Repositories in the space"). Each prints the coordination space, whether the run wrote
//! anything, and the repository's record as `nxs repository list` prints it. Creating refuses a
//! `nexus-flow.toml` that already names a coordination space, with `conflict`; joining the same
//! space again writes nothing; another coordination space and a name another repository holds are
//! `conflict`; leaving sets `left` on the record, which stays, and takes the id out of the file. A
//! refusal leaves the file as it was.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{json, nxs, scratch, stdout};

const OTHER: &str = "ffeeddccbbaa99887766554433221100";

/// A repository `folder` beside the data directory, with its space and the git remote `origin`
/// `https://example.com/<folder>.git`.
fn repo(base: &Path, folder: &str) -> PathBuf {
    let repo = base.join(folder);
    fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "remote",
            "add",
            "origin",
            &format!("https://example.com/{folder}.git"),
        ],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(&args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let out = nxs(base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    repo
}

fn file(repo: &Path) -> String {
    fs::read_to_string(repo.join("nexus-flow.toml")).unwrap()
}

fn space_of(repo: &Path) -> String {
    let text = file(repo);
    let line = text.lines().find(|l| l.starts_with("repository")).unwrap();
    line.split('"').nth(1).unwrap().to_owned()
}

fn run(base: &Path, repo: &Path, args: &[&str]) -> nxs_v2::Outcome {
    let out = nxs(base, repo, args);
    assert!(
        out.stderr.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Creates a coordination space from `repo` and returns its id.
fn create(base: &Path, repo: &Path) -> String {
    let out = run(base, repo, &["space", "create-coordination"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    json(&out)["coordination"].as_str().unwrap().to_owned()
}

fn error_kind(out: &nxs_v2::Outcome) -> String {
    assert_eq!(out.code, 1, "{}", stdout(out));
    json(out)["error"]["kind"].as_str().unwrap().to_owned()
}

/// The golden file `name`, with `<coordination>` and `<repository>` replaced.
fn golden(name: &str, coordination: &str, repository: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(path)
        .unwrap()
        .replace("<coordination>", coordination)
        .replace("<repository>", repository)
}

#[test]
fn creating_names_the_space_and_prints_the_record_named_after_origin() {
    let base = scratch("space-create");
    let repo = repo(&base, "nexus-flow");

    let out = run(&base, &repo, &["space", "create-coordination"]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let printed = json(&out);
    let coordination = printed["coordination"].as_str().unwrap();
    assert_eq!(printed["written"], true);
    assert_eq!(printed["repository"]["name"], "nexus-flow");
    assert_eq!(printed["repository"]["space"], space_of(&repo).as_str());
    assert_eq!(printed["repository"]["left"], false);
    assert!(file(&repo).contains(&format!("coordination = \"{coordination}\"")));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn creating_with_a_name_takes_it() {
    let base = scratch("space-create-name");
    let repo = repo(&base, "repo");

    let out = run(
        &base,
        &repo,
        &["space", "create-coordination", "--name", "central-infra"],
    );

    assert_eq!(json(&out)["repository"]["name"], "central-infra");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn creating_again_is_refused_as_a_conflict() {
    let base = scratch("space-create-twice");
    let repo = repo(&base, "repo");
    let first = create(&base, &repo);
    let before = file(&repo);

    let out = run(&base, &repo, &["space", "create-coordination"]);

    assert_eq!(error_kind(&out), "conflict");
    assert!(json(&out)["error"]["msg"]
        .as_str()
        .unwrap()
        .contains(&first));
    assert_eq!(file(&repo), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_writes_the_record_and_again_writes_nothing() {
    let base = scratch("space-join");
    let first = repo(&base, "central-infra");
    let second = repo(&base, "app");
    let coordination = create(&base, &first);

    let joined = run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );
    let before = file(&second);
    let again = run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );

    assert_eq!(joined.code, 0, "{}", stdout(&joined));
    assert_eq!(json(&joined)["written"], true);
    assert_eq!(json(&joined)["repository"]["name"], "app");
    assert_eq!(json(&again)["written"], false);
    assert_eq!(file(&second), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_second_coordination_space_or_a_name_another_repository_holds_is_a_conflict() {
    let base = scratch("space-join-conflict");
    let first = repo(&base, "central-infra");
    let second = repo(&base, "app");
    let coordination = create(&base, &first);
    let before = file(&second);

    let taken = run(
        &base,
        &second,
        &[
            "space",
            "join-coordination",
            &coordination,
            "--name",
            "central-infra",
        ],
    );
    run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );
    let joined = file(&second);
    let other = run(&base, &second, &["space", "join-coordination", OTHER]);

    assert_eq!(error_kind(&taken), "conflict");
    assert_ne!(before, joined);
    assert_eq!(error_kind(&other), "conflict");
    assert_eq!(file(&second), joined);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_own_space_a_text_that_is_no_space_id_and_an_unheld_space_are_validation() {
    let base = scratch("space-join-invalid");
    let repo = repo(&base, "repo");
    let before = file(&repo);
    let own = space_of(&repo);

    for space in [own.as_str(), "not-a-space", OTHER] {
        let out = run(&base, &repo, &["space", "join-coordination", space]);
        assert_eq!(error_kind(&out), "validation", "{space}");
    }
    assert_eq!(file(&repo), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_outside_a_repository_is_no_workspace() {
    let base = scratch("space-join-none");

    let out = run(
        &base,
        &base.join("repo"),
        &["space", "join-coordination", OTHER],
    );

    assert_eq!(error_kind(&out), "no_workspace");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn leaving_marks_the_record_left_and_takes_the_id_out_of_the_file() {
    let base = scratch("space-leave");
    let first = repo(&base, "central-infra");
    let second = repo(&base, "app");
    let coordination = create(&base, &first);
    let before = file(&second);
    run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );

    let left = run(&base, &second, &["space", "leave-coordination"]);
    let again = run(&base, &second, &["space", "leave-coordination"]);

    assert_eq!(left.code, 0, "{}", stdout(&left));
    assert_eq!(json(&left)["coordination"], coordination.as_str());
    assert_eq!(json(&left)["repository"]["left"], true);
    assert_eq!(file(&second), before);
    assert_eq!(error_kind(&again), "validation");
    let listed = json(&run(&base, &first, &["repository", "list"]));
    assert_eq!(listed[0]["name"], "app");
    assert_eq!(listed[0]["left"], true);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_space_commands_print_the_same_bytes_as_the_golden_files() {
    let base = scratch("space-golden");
    let first = repo(&base, "central-infra");
    let second = repo(&base, "app");

    let created = run(&base, &first, &["space", "create-coordination"]);
    let coordination = json(&created)["coordination"].as_str().unwrap().to_owned();
    let created_md = {
        let other = repo(&base, "other");
        let out = run(&base, &other, &["space", "create-coordination", "--md"]);
        let id = stdout(&out).split('`').nth(1).unwrap().to_owned();
        (out, id, space_of(&other))
    };
    let joined = run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );
    let again_md = run(
        &base,
        &second,
        &["space", "join-coordination", &coordination, "--md"],
    );
    let again = run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );
    let left = run(&base, &second, &["space", "leave-coordination"]);
    run(
        &base,
        &second,
        &["space", "join-coordination", &coordination],
    );
    let left_md = run(&base, &second, &["space", "leave-coordination", "--md"]);

    let (one, two) = (space_of(&first), space_of(&second));
    assert_eq!(
        stdout(&created),
        golden("space-create-coordination.json", &coordination, &one)
    );
    assert_eq!(
        stdout(&created_md.0),
        golden("space-create-coordination.md", &created_md.1, &created_md.2)
    );
    assert_eq!(
        stdout(&joined),
        golden("space-join-coordination.json", &coordination, &two)
    );
    assert_eq!(
        stdout(&again),
        golden("space-join-coordination-again.json", &coordination, &two)
    );
    assert_eq!(
        stdout(&again_md),
        golden("space-join-coordination-again.md", &coordination, &two)
    );
    assert_eq!(
        stdout(&left),
        golden("space-leave-coordination.json", &coordination, &two)
    );
    assert_eq!(
        stdout(&left_md),
        golden("space-leave-coordination.md", &coordination, &two)
    );
    fs::remove_dir_all(&base).unwrap();
}

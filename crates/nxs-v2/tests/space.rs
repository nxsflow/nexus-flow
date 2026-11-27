//! `nxs space join-coordination <space id>` (RFC 0001, "Creating and finding a space"; "Spaces":
//! a repository belongs to at most one coordination space): it writes `[space] coordination` into
//! `nexus-flow.toml` and prints what `Engine::space_join_coordination` returns. Joining again
//! with the same id writes nothing; a second, different coordination space is refused with
//! `conflict`, and a refusal leaves the file as it was.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{json, nxs, scratch, stdout};

const REPOSITORY: &str = "0123456789abcdef0123456789abcdef";
const COORDINATION: &str = "00112233445566778899aabbccddeeff";
const OTHER: &str = "ffeeddccbbaa99887766554433221100";

/// A repository whose `nexus-flow.toml` names only its repository space.
fn repo(test: &str) -> (PathBuf, PathBuf) {
    let base = scratch(test);
    let repo = base.join("repo");
    fs::write(
        repo.join("nexus-flow.toml"),
        format!("format = 1\n\n[space]\nrepository = \"{REPOSITORY}\"\n"),
    )
    .unwrap();
    (base, repo)
}

fn file(repo: &Path) -> String {
    fs::read_to_string(repo.join("nexus-flow.toml")).unwrap()
}

fn join(base: &Path, repo: &Path, space: &str, md: bool) -> nxs_v2::Outcome {
    let mut args = vec!["space", "join-coordination", space];
    if md {
        args.push("--md");
    }
    nxs(base, repo, &args)
}

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(path).unwrap()
}

#[test]
fn joining_writes_the_id_and_prints_it() {
    let (base, repo) = repo("space-join");

    let out = join(&base, &repo, COORDINATION, false);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty());
    let printed = json(&out);
    assert_eq!(printed["coordination"], COORDINATION);
    assert_eq!(printed["written"], true);
    assert!(
        file(&repo).contains(&format!("coordination = \"{COORDINATION}\"")),
        "{}",
        file(&repo)
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_again_with_the_same_id_is_a_no_op() {
    let (base, repo) = repo("space-join-again");
    join(&base, &repo, COORDINATION, false);
    let before = file(&repo);

    let out = join(&base, &repo, COORDINATION, false);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert_eq!(json(&out)["written"], false);
    assert_eq!(file(&repo), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_a_second_coordination_space_is_refused_as_a_conflict() {
    let (base, repo) = repo("space-join-other");
    join(&base, &repo, COORDINATION, false);
    let before = file(&repo);

    let out = join(&base, &repo, OTHER, false);

    assert_eq!(out.code, 1);
    let error = &json(&out)["error"];
    assert_eq!(error["kind"], "conflict");
    assert!(
        error["msg"].as_str().unwrap().contains(COORDINATION),
        "{error}"
    );
    assert_eq!(file(&repo), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_repository_space_and_a_text_that_is_no_space_id_are_refused_as_validation() {
    let (base, repo) = repo("space-join-invalid");
    let before = file(&repo);

    for space in [REPOSITORY, "not-a-space"] {
        let out = join(&base, &repo, space, false);
        assert_eq!(out.code, 1, "{space}");
        assert_eq!(json(&out)["error"]["kind"], "validation", "{space}");
    }
    assert_eq!(file(&repo), before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_outside_a_repository_is_no_workspace() {
    let base = scratch("space-join-none");

    let out = join(&base, &base.join("repo"), COORDINATION, false);

    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "no_workspace");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn joining_prints_the_same_bytes_as_the_golden_files() {
    let (base, repo) = repo("space-join-golden");

    let written = join(&base, &repo, COORDINATION, false);
    let again_md = join(&base, &repo, COORDINATION, true);
    let again_json = join(&base, &repo, COORDINATION, false);

    assert_eq!(stdout(&written), golden("space-join-coordination.json"));
    assert_eq!(
        stdout(&again_json),
        golden("space-join-coordination-again.json")
    );
    assert_eq!(
        stdout(&again_md),
        golden("space-join-coordination-again.md")
    );
    fs::remove_dir_all(&base).unwrap();
}

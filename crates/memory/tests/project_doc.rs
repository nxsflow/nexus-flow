//! `NEXUS_MEMORY.md` as a build product (6j6v.8q88), black-box over the built `nxm` binary.
//!
//! The module's own unit tests pin the projection, the write rule and the worktree guard. What they
//! cannot see is the thing most likely to rot: a WRITE VERB that forgets to regenerate. Generation
//! is a side effect wired at each write path, so the risk is not that the rule is wrong but that
//! one caller was missed — today, or by the next verb somebody adds. So the coverage here is
//! deliberately a table over the write verbs rather than one example: adding a verb without adding
//! its row leaves the gap visible, and the row fails until the verb regenerates like its siblings.

use assert_cmd::Command;
use tempfile::TempDir;

const DOC: &str = "NEXUS_MEMORY.md";

fn nxm(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(tmp.path())
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z");
    c
}

/// A workspace holding one memory, with the document already generated from it.
fn seeded() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxm(&tmp).arg("init").assert().success();
    nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT not sessions",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    tmp
}

fn doc(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join(DOC)).expect("the document exists")
}

/// One row of the write-verb table: the verb's name (for the failure message), the argv to run,
/// and what the regenerated document must then say.
type WriteCase<'a> = (&'a str, &'a [&'a str], &'a dyn Fn(&str) -> bool);

#[test]
fn every_write_verb_regenerates_the_document() {
    // One row per verb that changes the store. `import` is covered separately (it needs a source
    // directory) and `init` writes no memory.
    let cases: [WriteCase; 4] = [
        (
            "remember",
            &[
                "remember",
                "always run the tests with -race",
                "--key",
                "race",
                "--introduction",
                "in one line",
            ],
            &|doc: &str| doc.contains("always run the tests with -race"),
        ),
        (
            "classify",
            &[
                "classify",
                "auth-jwt",
                "--scope",
                "item",
                "--refs",
                "6j6v.8q88",
            ],
            // Reach `item` with a named board item moves the memory OUT of the replayed set — the
            // document must lose it, which is the same retrieval rule `prime` applies.
            &|doc: &str| !doc.contains("auth uses JWT"),
        ),
        ("reorder", &["reorder", "auth-jwt"], &|doc: &str| {
            doc.contains("auth uses JWT")
        }),
        ("forget", &["forget", "auth-jwt"], &|doc: &str| {
            !doc.contains("auth uses JWT")
        }),
    ];

    for (verb, args, expected) in cases {
        let tmp = seeded();
        let before = doc(&tmp);
        nxm(&tmp).args(args).assert().success();
        let after = doc(&tmp);
        assert!(
            expected(&after),
            "`nxm {verb}` must project into {DOC}; it still reads:\n{after}"
        );
        if verb != "reorder" {
            assert_ne!(before, after, "`nxm {verb}` changed nothing in {DOC}");
        }
    }
}

#[test]
fn the_document_is_created_by_the_first_write_and_not_by_init() {
    // Rule: a workspace that has never remembered anything does not get an empty document planted
    // in its root. `init` alone must leave the root clean.
    let tmp = TempDir::new().unwrap();
    nxm(&tmp).arg("init").assert().success();
    assert!(
        !tmp.path().join(DOC).exists(),
        "init alone plants no document"
    );

    nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT not sessions",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    assert!(doc(&tmp).contains("auth uses JWT not sessions"));
}

#[test]
fn the_check_goes_red_on_a_hand_edit_and_green_again_after_the_next_write() {
    // The guard, end to end: a non-zero exit is what makes "somebody edited the projection" or "the
    // daemon never ran in this checkout" loud instead of silent.
    let tmp = seeded();
    nxm(&tmp).args(["doc", "--check"]).assert().success();

    std::fs::write(tmp.path().join(DOC), "# Project memory\n\nhand-written\n").unwrap();
    nxm(&tmp).args(["doc", "--check"]).assert().failure();

    nxm(&tmp)
        .args([
            "remember",
            "always run the tests with -race",
            "--key",
            "race",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(&tmp).args(["doc", "--check"]).assert().success();
}

#[test]
fn doc_prints_exactly_what_the_file_holds() {
    let tmp = seeded();
    let out = nxm(&tmp).arg("doc").assert().success();
    let printed = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert_eq!(printed, doc(&tmp), "`nxm doc` prints the file's own bytes");
}

#[test]
fn the_documents_index_half_is_the_prime_index_byte_for_byte() {
    // **What replaced the subset condition** (6j6v.xbnh). The file used to be a byte-for-byte subset
    // of `nxm prime`, which was the whole guarantee on the fallback path
    // (`nxs prime || cat NEXUS_MEMORY.md`): deliver less than the main path, never more. It cannot
    // be that any more, and deliberately so — the owner decision of 2026-08-27 keeps the FULL
    // TEXT in the file, because the file is the versioned, diff-able record of what the memories
    // say and an index-only file would move every body out of every change proposal.
    //
    // So the file now has two halves and a notice between them, and the property that carries the
    // old one's weight is stated over the halves separately:
    //
    // 1. the INDEX half is `prime`'s index byte for byte — otherwise the notice ("you already have
    //    this") would be a lie;
    // 2. the FULL TEXT half is what `nxm recall` serves, and the notice says to read it that way;
    // 3. neither half invents a memory the other does not have.
    let tmp = seeded();
    nxm(&tmp)
        .args([
            "remember",
            "always run the tests with -race",
            "--key",
            "race",
            "--introduction",
            "never run the tests without -race",
        ])
        .assert()
        .success();

    let prime = nxm(&tmp).arg("prime").assert().success();
    let prime = String::from_utf8_lossy(&prime.get_output().stdout).to_string();
    let doc = doc(&tmp);

    let index = doc
        .split_once("## Index (")
        .expect("the document carries an index")
        .1
        .split_once("\n\n")
        .expect("…with entries under its heading")
        .1
        .split_once("\n\n## Full text")
        .expect("…and a full-text half beneath")
        .0;
    assert!(
        prime.contains(index),
        "the index half must occur verbatim in prime:\n--- index ---\n{index}\n--- prime ---\n{prime}"
    );
    assert!(
        doc.contains("You have already been given the index below"),
        "and the file says so, or an agent reads all of it by hand:\n{doc}"
    );
    // The full text is in the file and NOT in prime — that asymmetry is the saving and the reason
    // the notice has to distinguish the two halves.
    assert!(
        doc.contains("## Full text") && doc.contains("always run the tests with -race"),
        "the file keeps the bodies, so a change to one stays visible in a diff:\n{doc}"
    );
    assert!(
        !prime.contains("always run the tests with -race"),
        "…and prime carries none of them:\n{prime}"
    );
}

#[test]
fn a_write_still_succeeds_and_says_so_when_the_projection_cannot_be_written() {
    // PR #287 review, Test Quality #2 (Medium): the contract is that a write is never reported as
    // failed because its projection lagged — the memory is committed before the projection runs, so
    // a failure there would report a write that happened as one that did not. But it must not be
    // SILENT either: the CLI prints a `note:` to stderr. Forced by making the document a DIRECTORY,
    // which no writer can rename a file over.
    let tmp = TempDir::new().unwrap();
    nxm(&tmp).arg("init").assert().success();
    std::fs::create_dir(tmp.path().join(DOC)).unwrap();

    let out = nxm(&tmp)
        .args([
            "remember",
            "auth uses JWT not sessions",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        stderr.contains("the memory was saved") && stderr.contains(DOC),
        "the failure is reported, not swallowed; stderr was:\n{stderr}"
    );
    assert!(
        String::from_utf8_lossy(&out.get_output().stdout).contains("remembered auth-jwt"),
        "the write itself still reports success"
    );

    // And the memory really is stored — the receipt was not a lie.
    let recalled = nxm(&tmp)
        .args(["--json", "recall", "auth-jwt"])
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&recalled.get_output().stdout).contains("auth uses JWT"));
}

/// The v2 working directory (6j6v.zjyv §6.6): a clone beside the root whose `.nxs` is a link to the root's target
/// resolves the same store, and a write from it regenerates ITS document, never the root's. A session reads memory
/// from the store, so this is only about which checkout's tracked file moves. Pinned so the projection cannot
/// silently change sides.
#[cfg(unix)]
#[test]
fn a_write_from_a_linked_directory_regenerates_that_directorys_document_and_not_the_roots() {
    let tmp = seeded();
    let root_doc = doc(&tmp);
    let linked = TempDir::new().unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join(".nxs").canonicalize().unwrap(),
        linked.path().join(".nxs"),
    )
    .unwrap();
    nxm(&linked)
        .args([
            "remember",
            "a fact written from the linked directory",
            "--introduction",
            "A fact written from the linked directory.",
            "--key",
            "from-the-linked-directory",
        ])
        .assert()
        .success();
    assert!(doc(&linked).contains("from-the-linked-directory"));
    assert_eq!(doc(&tmp), root_doc, "the root's document moved");
}

// ---- a write reaches the store, and a publication reaches the repository (6j6v.g0ye) -----------

/// Git with the identity a test needs and the index left alone.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// A workspace in a clone of a file-path origin. The default branch carries the document, and the
/// workspace's store holds one memory it does not know yet, so the checkout lags the store: exactly
/// the state of this repository when this was built.
struct Cloned {
    tmp: TempDir,
    origin: std::path::PathBuf,
    root: TempDir,
}

const PUBLISHED: &str = "<!-- generated by nexus-flow -->\n# an earlier publication\n";

fn cloned() -> Cloned {
    let tmp = TempDir::new().unwrap();
    let origin = tmp.path().join("origin.git");
    let seed = tmp.path().join("seed");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&seed).unwrap();
    git(&origin, &["init", "--bare", "-q", "-b", "main"]);
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join(DOC), PUBLISHED).unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "seed"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "-q", "origin", "main"]);
    let root = TempDir::new().unwrap();
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            root.path().to_str().unwrap(),
        ],
    );
    std::fs::write(
        root.path().join(".git/info/exclude"),
        ".nxs\n.claude\nAGENTS.md\nCLAUDE.md\n",
    )
    .unwrap();
    nxm(&root).arg("init").assert().success();
    nxm(&root)
        .args([
            "remember",
            "a fact",
            "--key",
            "a",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    Cloned { tmp, origin, root }
}

fn seed_advance(c: &Cloned, content: &str) {
    let scratch = c.tmp.path().join("scratch");
    if !scratch.exists() {
        git(
            c.tmp.path(),
            &[
                "clone",
                "-q",
                c.origin.to_str().unwrap(),
                scratch.to_str().unwrap(),
            ],
        );
    }
    std::fs::write(scratch.join(DOC), content).unwrap();
    git(&scratch, &["add", "."]);
    git(&scratch, &["commit", "-q", "-m", "a later publication"]);
    git(&scratch, &["push", "-q", "origin", "main"]);
}

/// A directory for `PATH` holding `git` and nothing else, plus whatever `gh` a test puts there.
#[cfg(unix)]
fn bin_dir_with_only_git() -> TempDir {
    let which = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8_lossy(&which.stdout).trim().to_string();
    let bin = TempDir::new().unwrap();
    std::os::unix::fs::symlink(real, bin.path().join("git")).unwrap();
    bin
}

/// A stand-in `gh` that keeps one open pull request in a file, and logs every call.
#[cfg(unix)]
fn fake_gh(bin: &TempDir) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let state = bin.path().join("open-pr");
    let log = bin.path().join("gh.log");
    let script = format!(
        r#"#!/bin/sh
echo "$@" >> "{log}"
case "$1 $2" in
  "pr list") [ -f "{state}" ] && /bin/cat "{state}"; exit 0 ;;
  "pr create") echo "https://forge.example/pr/7" > "{state}"; echo "https://forge.example/pr/7"; exit 0 ;;
  "pr close") /bin/rm -f "{state}"; exit 0 ;;
esac
exit 1
"#,
        log = log.display(),
        state = state.display(),
    );
    let path = bin.path().join("gh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    log
}

#[test]
fn no_write_verb_touches_the_file_in_a_clone_with_an_origin() {
    // The table of `every_write_verb_regenerates_the_document`, turned around. One row per verb that
    // changes the store, `import` included.
    let c = cloned();
    let before = doc(&c.root);
    let import_source = TempDir::new().unwrap();
    std::fs::write(
        import_source.path().join("fact.md"),
        "---\nname: imported\ndescription: an imported fact\n---\nthe imported fact\n",
    )
    .unwrap();
    let source = import_source.path().to_str().unwrap();
    let cases: [&[&str]; 5] = [
        &[
            "remember",
            "another",
            "--key",
            "b",
            "--introduction",
            "another line",
        ],
        &["classify", "a", "--category", "architecture"],
        &["reorder", "a", "b"],
        &["forget", "b"],
        &["import", "--from", source],
    ];
    for args in cases {
        nxm(&c.root).args(args).assert().success();
        assert_eq!(doc(&c.root), before, "`nxm {args:?}` moved the file");
        assert_eq!(
            git(
                c.root.path(),
                &["status", "--porcelain", "--untracked-files=all"]
            ),
            "",
            "`nxm {args:?}` dirtied the tree"
        );
    }
}

#[test]
fn a_write_in_a_clone_that_never_had_the_file_does_not_plant_one() {
    let tmp = TempDir::new().unwrap();
    let origin = tmp.path().join("origin.git");
    let seed = tmp.path().join("seed");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&seed).unwrap();
    git(&origin, &["init", "--bare", "-q", "-b", "main"]);
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("code.txt"), "code\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "seed"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "-q", "origin", "main"]);
    let root = TempDir::new().unwrap();
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            root.path().to_str().unwrap(),
        ],
    );
    nxm(&root).arg("init").assert().success();
    nxm(&root)
        .args([
            "remember",
            "a fact",
            "--key",
            "a",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    assert!(!root.path().join(DOC).exists());
    let out = nxm(&root).args(["doc", "--check"]).assert().success();
    assert!(
        String::from_utf8_lossy(&out.get_output().stdout).contains("waiting to be published"),
        "a file the first publication will bring is not missing"
    );
}

#[test]
fn a_repository_that_ignores_the_file_keeps_it_written_directly() {
    let c = cloned();
    git(c.root.path(), &["rm", "-q", "--cached", DOC]);
    std::fs::write(c.root.path().join(".gitignore"), format!("{DOC}\n")).unwrap();
    nxm(&c.root)
        .args([
            "remember",
            "another",
            "--key",
            "b",
            "--introduction",
            "another line",
        ])
        .assert()
        .success();
    assert!(doc(&c.root).contains("another"));
    nxm(&c.root).args(["doc", "--check"]).assert().success();
}

#[test]
fn the_check_notes_a_checkout_that_lags_the_store_and_stays_green() {
    // This repository's own state when this was built: three memories changed and the file restored.
    let c = cloned();
    let out = nxm(&c.root).args(["doc", "--check"]).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("waiting to be published"), "{stdout}");
    assert!(stdout.starts_with("note:"), "{stdout}");

    let json = nxm(&c.root)
        .args(["--json", "doc", "--check"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&json.get_output().stdout).unwrap();
    assert_eq!(json["status"], "pending_publication");
}

#[test]
fn the_check_reads_a_checkout_several_publications_behind_as_waiting_too() {
    let c = cloned();
    seed_advance(&c, "# publication two\n");
    seed_advance(&c, "# publication three\n");
    git(c.root.path(), &["fetch", "-q", "origin"]);
    // The checkout still holds the first; its `origin/main` knows two more.
    assert_eq!(doc(&c.root), PUBLISHED);
    nxm(&c.root).args(["doc", "--check"]).assert().success();

    // And one that pulled a later one reads the same.
    git(c.root.path(), &["pull", "-q", "origin", "main"]);
    assert_eq!(doc(&c.root), "# publication three\n");
    let out = nxm(&c.root).args(["doc", "--check"]).assert().success();
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("waiting to be published"));
}

#[test]
fn the_check_fails_on_a_hand_edit_uncommitted_or_committed_on_a_branch() {
    let c = cloned();
    std::fs::write(c.root.path().join(DOC), "# edited by hand\n").unwrap();
    let out = nxm(&c.root).args(["doc", "--check"]).assert().failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("edited by hand"), "{stderr}");

    git(c.root.path(), &["switch", "-q", "-c", "feat/an-edit"]);
    git(c.root.path(), &["add", DOC]);
    git(c.root.path(), &["commit", "-q", "-m", "an edit"]);
    nxm(&c.root).args(["doc", "--check"]).assert().failure();
}

#[test]
fn the_check_fails_when_the_file_the_default_branch_carries_was_deleted() {
    let c = cloned();
    std::fs::remove_file(c.root.path().join(DOC)).unwrap();
    nxm(&c.root).args(["doc", "--check"]).assert().failure();
}

#[cfg(unix)]
#[test]
fn doc_publish_opens_a_pull_request_through_gh_and_leaves_the_checkout_alone() {
    let c = cloned();
    let bin = bin_dir_with_only_git();
    let log = fake_gh(&bin);
    let before = doc(&c.root);

    let out = nxm(&c.root)
        .env("PATH", bin.path())
        .args(["--json", "doc", "--publish"])
        .assert()
        .success();

    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["status"], "opened", "{json}");
    assert_eq!(json["pull_request"], "https://forge.example/pr/7");
    assert_eq!(json["branch"], "nxm/publish-memory");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("pr create --head nxm/publish-memory --base main"),
        "{calls}"
    );
    let commit = json["commit"].as_str().unwrap();
    assert!(git(&c.origin, &["show", &format!("{commit}:{DOC}")]).contains("a fact"));
    assert_eq!(doc(&c.root), before);
    assert_eq!(
        git(
            c.root.path(),
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );

    // Publishing again creates nothing, in words too.
    let out = nxm(&c.root)
        .env("PATH", bin.path())
        .args(["doc", "--publish"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("nothing new to publish"), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .matches("pr create")
            .count(),
        1
    );

    // And the check still reads the checkout's older copy as waiting, not as a hand edit.
    nxm(&c.root)
        .env("PATH", bin.path())
        .args(["doc", "--check"])
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn a_machine_with_no_gh_is_told_why_and_loses_no_memory() {
    let c = cloned();
    let bin = bin_dir_with_only_git();

    let out = nxm(&c.root)
        .env("PATH", bin.path())
        .args(["doc", "--publish"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("gh was not found"), "{stderr}");
    assert_eq!(
        git(
            &c.origin,
            &["for-each-ref", "--format=%(refname)", "refs/heads"]
        ),
        "refs/heads/main",
        "no branch was left behind"
    );

    // The next write is untouched by it.
    nxm(&c.root)
        .env("PATH", bin.path())
        .args([
            "remember",
            "another",
            "--key",
            "b",
            "--introduction",
            "another line",
        ])
        .assert()
        .success();

    // And the reason stands where a person looks: green, with the failure named.
    let out = nxm(&c.root)
        .env("PATH", bin.path())
        .args(["doc", "--check"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("waiting to be published"), "{stdout}");
    assert!(stdout.contains("gh was not found"), "{stdout}");
}

#[test]
fn doc_publish_in_a_workspace_with_no_origin_says_there_is_nothing_to_publish_to() {
    let tmp = seeded();
    let out = nxm(&tmp).args(["doc", "--publish"]).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("written directly"), "{stdout}");
    let json = nxm(&tmp)
        .args(["--json", "doc", "--publish"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&json.get_output().stdout).unwrap();
    assert_eq!(json["status"], "not_published");
}

#[test]
fn check_and_publish_are_two_different_questions() {
    let tmp = seeded();
    nxm(&tmp)
        .args(["doc", "--check", "--publish"])
        .assert()
        .failure();
}

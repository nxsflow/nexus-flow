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

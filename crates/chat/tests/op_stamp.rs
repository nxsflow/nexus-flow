//! The unpinned actor path for `nxc`, black-box over the built binary — the chat-side twin of
//! `crates/cli/tests/op_stamp.rs` (review finding Test Quality #1, PR #311).
//!
//! `std::env::var` reports a variable that is SET BUT EMPTY as `Ok("")`, so the `actor()` chain
//! took it literally. chat is the surface where an op eventually authorizes an agent ACTION, and
//! its op author is the QUALIFIED handle `<origin>/<actor>` — so a blank actor would not even trip
//! the substrate's non-blank assert, it would land as the degenerate `"<origin>/"`. That makes the
//! wiring here worth pinning through the real binary, not only in the unit test of the rule.
//!
//! **The origin half is DERIVED, not pinned** (nxf 6j6v.07me). It is the workspace's own replica
//! prefix now, random per `TempDir`; this file removes determinism knobs on purpose (a blank actor
//! must fall through with nothing else helping it), so it reads the prefix back out of the
//! workspace the invocation actually wrote to rather than spelling one. Pinning `NXC_ORIGIN` would
//! have worked too and would have been the wrong pin: the subject here is the ACTOR tier, and a
//! hardcoded origin would quietly stop checking that the two halves are joined at all.

use assert_cmd::Command;
use tempfile::TempDir;

/// An `nxc` invocation with a set-but-BLANK actor at both tiers, determinism knobs removed.
fn nxc_blank_actor(dir: &std::path::Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(dir)
        .env_remove("NXC_NOW")
        .env("NXC_ACTOR", "")
        .env("USER", "   ");
    c
}

/// The `<origin>` half every op author is qualified by: this workspace's own replica prefix
/// (`nexus_chat::workspace::origin_of`), which is exactly what the CLI resolved while writing them.
fn origin(dir: &std::path::Path) -> String {
    let ws = nexus_chat::workspace::Workspace::resolve(None, dir).expect("resolve workspace");
    nexus_chat::workspace::origin_of(&ws).to_string()
}

fn authors(dir: &std::path::Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(dir.join(".nxs/db.sqlite")).unwrap();
    let mut stmt = conn.prepare("SELECT author FROM ops").unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

#[test]
fn a_set_but_empty_actor_env_never_authors_an_op() {
    let tmp = TempDir::new().unwrap();
    nxc_blank_actor(tmp.path())
        .args(["--json", "init"])
        .assert()
        .success();
    // A declared persona, so the blank-actor invocation has something to write TO. `channels create`
    // + `send <cid> <body>` stood here and both went with 6j6v.dvyq §3; what this test needs is
    // simply A WRITE performed with every identity tier blank, and `send --to <persona>` is one.
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(
        personas.join("pm.yaml"),
        "handle: pm\njob_title: Product manager\nsystem_prompt: You are the PM.\n",
    )
    .unwrap();
    nxc_blank_actor(tmp.path())
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .args(["send", "--to", "pm", "a message"])
        .assert()
        .success();

    let authors = authors(tmp.path());
    assert!(!authors.is_empty(), "the send emitted ops");
    assert!(
        authors.iter().all(|a| !a.trim().is_empty()),
        "a blank NXC_ACTOR/USER must fall through, never author an op: {authors:?}"
    );
    assert!(
        authors.iter().all(|a| !a.ends_with('/')),
        "and never as the half-empty qualified handle `<origin>/`: {authors:?}"
    );
    // The origin half read back from the workspace this very invocation wrote into — the same
    // answer `cli.rs`'s `origin()` fell back to, taken from the source rather than restated.
    let expected = format!("{}/nxc", origin(tmp.path()));
    assert!(
        authors.iter().all(|a| *a == expected),
        "with every tier blank the binary's own name is the identity of record, \
         qualified by the workspace's own origin ({expected}), got {authors:?}"
    );
}

//! **The retrieval rule** (nxf 6j6v.srpg) — the acceptance suite, one test per cell.
//!
//! Every memory carries a reach, and the reach decides **where and how deep** it surfaces:
//!
//! | reach     | `nxf next`                 | `nxf show <id>` | `nxs prime` |
//! |-----------|----------------------------|-----------------|-------------|
//! | `item`    | a hint that there are some | **full text**   | —           |
//! | `project` | —                          | —               | **full text** |
//! | `global`  | —                          | —               | **full text** |
//!
//! The ticket is accepted on "the same memory appears at exactly the places in the table and
//! nowhere else", so the nine cells are nine assertions here — each over the REAL binaries against
//! one shared `.nxs/` workspace, because the rule spans all three: flow renders two columns, memory
//! the third, and `nxs prime` is the fan-out a session actually gets.
//!
//! This is also the deterministic answer to "how does an agent find the right knowledge" — reach
//! plus references, no embedding, no similarity measure, no network.
//!
//! The table's top row assumes the memory NAMES board items; one that is `item`-scoped with empty
//! `refs` has no board item to read on and is replayed at session start instead, so the rule stays
//! total. That tenth case has its own test below.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-08-03T12:00:00Z";

/// The body of each memory, one per reach. Distinctive strings, so "appears nowhere else" can be
/// asserted by substring over a whole surface's stdout.
const ITEM_BODY: &str = "TICKET-REACH-BODY: the closing comment must name the flake";
const PROJECT_BODY: &str = "PROJECT-REACH-BODY: this workspace releases from main only";
const GLOBAL_BODY: &str = "GLOBAL-REACH-BODY: never rewrite a published tag";

/// The written INTRODUCTION of each memory — what `nxs prime` replays since nxf 6j6v.xbnh, where it
/// used to replay the body. The reach still decides WHICH surface a memory reads on (that is this
/// file's subject and it is unchanged); what changed is how much of it the session start carries.
/// Keeping both markers distinct is what lets a test say "the line is here AND the body is not".
const ITEM_INTRO: &str = "TICKET-REACH-INTRO: name the flake in the closing comment";
const PROJECT_INTRO: &str = "PROJECT-REACH-INTRO: releases are cut from main only";
const GLOBAL_INTRO: &str = "GLOBAL-REACH-INTRO: never rewrite a published tag";

fn bin(name: &str, dir: &Path) -> Command {
    // `Command::cargo_bin` like the sibling e2e suite: `-p nxs` builds the one multicall binary
    // these three names all resolve to, so the stale-binary trap (0yyw) cannot bite here.
    let mut c = Command::cargo_bin(name).unwrap_or_else(|_| panic!("{name} binary built"));
    // Never this machine's real `~` (nxf 6j6v.npf9): `nxs init` writes to the background service's
    // workspace registry, and an unpinned suite leaves a dead entry per case in the developer's
    // own `~/.nexusflow/workspaces.toml`.
    c.pin_home(nxs_test_support::pinned_home())
        .current_dir(dir)
        .env("NXF_ACTOR", "alice")
        .env("NXM_ACTOR", "alice")
        .env("NXF_NOW", NOW)
        .env("NXM_NOW", NOW)
        .env("NXS_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

fn stdout_of(name: &str, dir: &Path, args: &[&str]) -> String {
    let out = bin(name, dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

fn json_of(name: &str, dir: &Path, args: &[&str]) -> Value {
    let mut full = args.to_vec();
    full.push("--json");
    serde_json::from_str(&stdout_of(name, dir, &full)).expect("json stdout")
}

/// One flow+memory workspace holding one open item and one memory per reach — the item-scoped one
/// filed against that item. Returns the workspace and the item's id.
fn workspace() -> (TempDir, String) {
    let tmp = TempDir::new().unwrap();
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    bin("nxm", tmp.path()).arg("init").assert().success();

    let created = json_of(
        "nxf",
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Ship the retrieval rule",
            "--description",
            "The reach decides where a memory surfaces",
            "--priority",
            "P1",
        ],
    );
    let id = created["id"].as_str().expect("created id").to_string();

    bin("nxm", tmp.path())
        .args([
            "remember",
            ITEM_BODY,
            "--key",
            "ticket",
            "--scope",
            "item",
            "--refs",
            &id,
            "--introduction",
            ITEM_INTRO,
        ])
        .assert()
        .success();
    bin("nxm", tmp.path())
        .args([
            "remember",
            PROJECT_BODY,
            "--key",
            "proj",
            "--scope",
            "project",
            "--introduction",
            PROJECT_INTRO,
        ])
        .assert()
        .success();
    bin("nxm", tmp.path())
        .args([
            "remember",
            GLOBAL_BODY,
            "--key",
            "glob",
            "--scope",
            "global",
            "--introduction",
            GLOBAL_INTRO,
        ])
        .assert()
        .success();

    (tmp, id)
}

// ---- column 1: `nxf next` — a hint for item reach, nothing for the other two -------------------

#[test]
fn next_hints_at_item_scoped_memories_and_never_prints_their_text() {
    let (tmp, _id) = workspace();
    let out = stdout_of("nxf", tmp.path(), &["next"]);
    assert!(
        out.contains("(1 memory)"),
        "the ticket-reach cell is a hint that there are some: {out}"
    );
    assert!(
        !out.contains(ITEM_BODY),
        "a hint, NOT the full text — `next` stays a work list: {out}"
    );
}

#[test]
fn next_says_nothing_about_project_or_global_memories() {
    let (tmp, _id) = workspace();
    let out = stdout_of("nxf", tmp.path(), &["next"]);
    assert!(!out.contains(PROJECT_BODY), "project reach: — cell: {out}");
    assert!(!out.contains(GLOBAL_BODY), "global reach: — cell: {out}");
    // The hint counts ONLY the item-scoped ones; a project/global memory must not inflate it.
    assert!(
        out.contains("(1 memory)") && !out.contains("(3 memories)"),
        "the count is the item's own: {out}"
    );
}

#[test]
fn next_stays_readable_when_an_item_carries_many_memories() {
    // The DoD's readability clause: forty memories on one item must still be ONE marker on ONE
    // row, not forty paragraphs in the work list.
    let (tmp, id) = workspace();
    for i in 0..40 {
        bin("nxm", tmp.path())
            .args([
                "remember",
                &format!("bulk memory number {i}"),
                "--key",
                &format!("bulk-{i}"),
                "--scope",
                "item",
                "--refs",
                &id,
                "--introduction",
                "in one line",
            ])
            .assert()
            .success();
    }
    let out = stdout_of("nxf", tmp.path(), &["next"]);
    assert!(
        out.contains("(41 memories)"),
        "one marker, one count: {out}"
    );
    assert!(!out.contains("bulk memory number"), "no bodies: {out}");
    assert_eq!(
        out.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "one item, one row — the count did not become rows: {out}"
    );
}

#[test]
fn next_json_carries_the_count_not_the_records() {
    let (tmp, id) = workspace();
    let rows = json_of("nxf", tmp.path(), &["next"]);
    let row = rows
        .as_array()
        .expect("array")
        .iter()
        .find(|r| r["id"] == id.as_str())
        .expect("the item is in next");
    assert_eq!(row["memories"], 1, "the machine form of the same hint");
    assert!(
        !stdout_of("nxf", tmp.path(), &["next", "--json"]).contains(ITEM_BODY),
        "still a hint on the json surface"
    );
}

// ---- column 2: `nxf show <id>` — full text for item reach, nothing for the other two ------------

#[test]
fn show_prints_the_item_scoped_memory_in_full() {
    let (tmp, id) = workspace();
    let out = stdout_of("nxf", tmp.path(), &["show", &id]);
    assert!(out.contains("MEMORIES"), "the section is present: {out}");
    assert!(
        out.contains(ITEM_BODY),
        "the full text, where the reader already is: {out}"
    );
    assert!(out.contains("ticket"), "under its key: {out}");
}

#[test]
fn show_says_nothing_about_project_or_global_memories() {
    let (tmp, id) = workspace();
    let out = stdout_of("nxf", tmp.path(), &["show", &id]);
    assert!(!out.contains(PROJECT_BODY), "project reach: — cell: {out}");
    assert!(!out.contains(GLOBAL_BODY), "global reach: — cell: {out}");
}

#[test]
fn show_carries_the_records_on_the_json_surface_and_omits_the_key_without_any() {
    let (tmp, id) = workspace();
    let v = json_of("nxf", tmp.path(), &["show", &id]);
    let memories = v["memories"].as_array().expect("memories array");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["key"], "ticket");
    assert_eq!(memories[0]["body"], ITEM_BODY);
    assert_eq!(memories[0]["scope"], "item");

    // Sparse: an item nobody filed a memory against is byte-identical to the pre-rule output.
    let other = json_of(
        "nxf",
        tmp.path(),
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Unrelated",
            "--description",
            "Nothing filed against this",
            "--priority",
            "P2",
        ],
    );
    let other_id = other["id"].as_str().unwrap();
    let v = json_of("nxf", tmp.path(), &["show", other_id]);
    assert!(
        v.get("memories").is_none(),
        "no memories ⇒ no key at all: {v}"
    );
}

#[test]
fn a_memory_does_not_leak_onto_an_item_it_does_not_name() {
    // Membership is exact: `refs` names the items, and nothing walks the board on the reader's
    // behalf. A sibling item shows nothing.
    let (tmp, _id) = workspace();
    let other = json_of(
        "nxf",
        tmp.path(),
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Sibling",
            "--description",
            "Not named by the memory",
            "--priority",
            "P2",
        ],
    );
    let other_id = other["id"].as_str().unwrap();
    let out = stdout_of("nxf", tmp.path(), &["show", other_id]);
    assert!(!out.contains(ITEM_BODY), "not this item's memory: {out}");
    assert!(!out.contains("MEMORIES"), "no empty section either: {out}");
}

// ---- column 3: `nxs prime` — project + global in full, item reach absent -----------------------

#[test]
fn prime_replays_the_project_and_global_memories_as_their_written_line() {
    // The cell used to read "in full" and reads "as one written line" since nxf 6j6v.xbnh. The
    // RULE is untouched — both reaches still surface here and nowhere else — but the session start
    // carries the introduction and never the body, so the cell has to be asserted on both halves
    // or it would go on passing against a renderer that had quietly gone back to the bodies.
    let (tmp, _id) = workspace();
    let out = stdout_of("nxs", tmp.path(), &["prime"]);
    assert!(
        out.contains(PROJECT_INTRO),
        "project reach: its line: {out}"
    );
    assert!(out.contains(GLOBAL_INTRO), "global reach: its line: {out}");
    assert!(
        !out.contains(PROJECT_BODY) && !out.contains(GLOBAL_BODY),
        "…and neither body: that is the saving: {out}"
    );
}

#[test]
fn prime_leaves_the_item_scoped_memory_out() {
    // The cell that keeps the session start from growing every ticket-local note ever written.
    let (tmp, _id) = workspace();
    let out = stdout_of("nxs", tmp.path(), &["prime"]);
    assert!(
        !out.contains(ITEM_BODY),
        "an item-scoped memory reads on its board item, not at session start: {out}"
    );
    assert!(
        !out.contains("### `ticket`"),
        "not even as a heading: {out}"
    );
}

#[test]
fn an_item_memory_that_names_no_item_still_reaches_a_reader() {
    // PR #282 review, Code Quality #1 — the hole the table alone does not close. `--scope item`
    // without `--refs` is reachable in one command (and was already reachable before this rule
    // existed), it appears on no board item, and withholding it from the bootstrap too would make
    // it invisible EVERYWHERE. The rule is total: it reads at session start until it names an item.
    let (tmp, id) = workspace();
    let orphan = "ORPHAN-BODY: filed as item-scoped, naming nothing yet";
    let orphan_intro = "ORPHAN-INTRO: item-scoped and naming nothing yet";
    bin("nxm", tmp.path())
        .args([
            "remember",
            orphan,
            "--key",
            "orphan",
            "--scope",
            "item",
            "--introduction",
            orphan_intro,
        ])
        .assert()
        .success();

    assert!(
        stdout_of("nxs", tmp.path(), &["prime"]).contains(orphan_intro),
        "it has nowhere else to appear, so the bootstrap keeps it"
    );
    assert!(
        !stdout_of("nxf", tmp.path(), &["show", &id]).contains(orphan),
        "…and it is on no board item, which is exactly why"
    );

    // Naming the item moves it across in the same step — the natural two-step of a seam whose
    // registers move one at a time, which is why rejecting the first half would be the wrong fix.
    bin("nxm", tmp.path())
        .args(["classify", "orphan", "--refs", &id])
        .assert()
        .success();
    assert!(!stdout_of("nxs", tmp.path(), &["prime"]).contains(orphan));
    assert!(stdout_of("nxf", tmp.path(), &["show", &id]).contains(orphan));
}

#[test]
fn prime_json_counts_only_what_it_replays() {
    let (tmp, _id) = workspace();
    let v = json_of("nxs", tmp.path(), &["prime"]);
    let memory_prime = v["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|m| m["module"] == "memory")
        .expect("the memory module fans out")["prime"]
        .clone();
    assert_eq!(
        memory_prime["count"], 2,
        "project + global, not the item-scoped one: {memory_prime}"
    );
    let keys: Vec<&str> = memory_prime["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        ["proj", "glob"],
        "in reading order — nothing here is filed, so: as written (6j6v.643z) — and the ticket is \
         absent, which is the cell under test"
    );
}

// ---- the rule as a whole ------------------------------------------------------------------------

#[test]
fn every_memory_appears_somewhere_and_none_appears_twice() {
    // The rule's promise read as a whole: each of the three bodies surfaces on exactly ONE of the
    // three surfaces. A future fourth reach that fell through every cell — silently invisible
    // everywhere — is precisely what this catches.
    let (tmp, id) = workspace();
    let surfaces = [
        ("nxf next", stdout_of("nxf", tmp.path(), &["next"])),
        ("nxf show", stdout_of("nxf", tmp.path(), &["show", &id])),
        ("nxs prime", stdout_of("nxs", tmp.path(), &["prime"])),
    ];
    // **The marker per reach is what that reach's surface actually renders** (nxf 6j6v.xbnh): the
    // board shows an item memory's BODY, the session start shows a memory's written LINE. Asserting
    // the body everywhere would now be asserting that the saving does not exist; asserting the line
    // everywhere would let an item memory's body go missing from `nxf show` unnoticed.
    for marker in [ITEM_BODY, PROJECT_INTRO, GLOBAL_INTRO] {
        let hits: Vec<&str> = surfaces
            .iter()
            .filter(|(_, out)| out.contains(marker))
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{marker} must read on exactly one surface, got {hits:?}"
        );
    }
    // And no session start carries a body at all — the half the markers above cannot state.
    assert!(
        [ITEM_BODY, PROJECT_BODY, GLOBAL_BODY]
            .iter()
            .all(|b| !surfaces[2].1.contains(b)),
        "no body reaches the session start:\n{}",
        surfaces[2].1
    );
    // …and the item-scoped one is the one `show` carries, with `next` holding only its hint.
    assert!(surfaces[1].1.contains(ITEM_BODY));
    assert!(surfaces[0].1.contains("(1 memory)"));
}

#[test]
fn a_flow_only_workspace_is_untouched_by_the_rule() {
    // The gate that keeps this free for everyone who never ran `nxm init`: no memory module, no
    // memory read, no `memories` view created behind their back — and identical output.
    let tmp = TempDir::new().unwrap();
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let created = json_of(
        "nxf",
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Flow only",
            "--description",
            "No memory module here",
            "--priority",
            "P1",
        ],
    );
    let id = created["id"].as_str().unwrap();

    assert!(!stdout_of("nxf", tmp.path(), &["show", id]).contains("MEMORIES"));
    assert!(json_of("nxf", tmp.path(), &["show", id])
        .get("memories")
        .is_none());
    assert!(json_of("nxf", tmp.path(), &["next"])[0]
        .get("memories")
        .is_none());
    let has_memories_view: i64 = rusqlite::Connection::open(tmp.path().join(".nxs/db.sqlite"))
        .expect("open the shared store")
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memories'",
            [],
            |r| r.get(0),
        )
        .expect("query sqlite_master");
    assert_eq!(
        has_memories_view, 0,
        "flow must not materialize memory's view just by rendering an item"
    );
}

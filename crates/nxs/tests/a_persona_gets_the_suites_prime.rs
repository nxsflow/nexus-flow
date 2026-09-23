//! **`nxs prime --persona <handle>`, and the filter that decides which services contribute**
//! (nxf 6j6v.k8zq).
//!
//! Owner, 2026-08-23: *"Jede Rolle sollte den Inhalt von `nxs prime --persona` erhalten. Wir muessen
//! in der Deklaration die Moeglichkeit bekommen, einzelne Dienste zu filtern (z. B. das Brett oder
//! die Erinnerungen auszuschliessen)."*
//!
//! It spans all three modules, so it is tested where all three are real: one `.nxs/` workspace, the
//! shipped binaries, and two personas whose declarations differ only in what they admit. A pure
//! reviewer does not need the board; a PM does — and the point of the test is that BOTH sides hold
//! at once, in one workspace, because a filter that turned out to be global would satisfy either
//! half on its own.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-08-26T12:00:00Z";

/// Distinctive bodies, so "this section is absent" can be asserted by substring over the whole
/// composed block rather than by parsing headings.
const MEMORY_BODY: &str = "MEMORY-MARKER: this workspace releases from main only";
/// The memory's INTRODUCTION — the only part of it a session start carries since nxf 6j6v.xbnh, and
/// therefore the marker a test asserting "memory contributed" has to look for. The body above is
/// what `nxm recall` serves and is deliberately absent from every block below.
const MEMORY_INTRO: &str = "MEMORY-INTRO-MARKER: releases are cut from main and nowhere else";
const TICKET_TITLE: &str = "FLOW-MARKER-the-board-is-here";

/// `-p nxs` builds the one multicall binary all four names resolve to, like the sibling suites.
///
/// **`NXC_TIMER=dry` is not decoration** (nxf 6j6v.74c0): this suite builds its own `nxc` command
/// rather than going through `nxs_test_support::cargo_bin`, so it inherits no default — and on
/// macOS an unset `NXC_TIMER` means the REAL `launchd` backend. `no_test_arms_the_real_scheduler`
/// holds this file to saying so by name.
///
/// **`NXC_WORKER=dry` is what makes it hermetic**, and it was learned the expensive way: the sends
/// below summon a persona, the real `SidecarWorker` drives the `claude` executable, and a machine
/// without Claude Code installed fails the send outright. This suite is about the composed BLOCK,
/// not about starting anything — so it starts nothing. (A developer machine usually has `claude`
/// and CI does not, which is the shape that goes green locally and red on the first push.)
fn bin(name: &str, dir: &Path) -> Command {
    let mut c = Command::cargo_bin(name).unwrap_or_else(|_| panic!("{name} binary built"));
    // Never this machine's real `~` (nxf 6j6v.npf9): `nxs init` writes to the background service's
    // workspace registry, and an unpinned suite leaves a dead entry per case in the developer's
    // own `~/.nexusflow/workspaces.toml`.
    c.pin_home(nxs_test_support::pinned_home())
        .env("NXC_TIMER", "dry")
        // …and never a real model for a thread name either (nxf 6j6v.e76c) — `cargo_bin` sets
        // this for every other black-box invocation in the repo, and this suite builds its own
        // command, so it says it itself for the reason the two lines around it do.
        .env("NXC_NAMER", "dry")
        .env("NXC_WORKER", "dry")
        .current_dir(dir)
        .env("NXF_ACTOR", "alice")
        .env("NXM_ACTOR", "alice")
        .env("NXC_ACTOR", "alice")
        .env("NXF_NOW", NOW)
        .env("NXM_NOW", NOW)
        .env("NXC_NOW", NOW)
        .env("NXS_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

fn run(name: &str, dir: &Path, args: &[&str]) -> String {
    let out = bin(name, dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

/// A workspace with all three modules live, one memory, one board item, and two personas that
/// differ only in their `prime:` filter.
fn suite() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(
        "nxf",
        dir,
        &["init", "--quiet", "--plugin", "issue-tracker"],
    );
    run("nxm", dir, &["init"]);
    run("nxc", dir, &["init"]);
    // `rules`, so the memory is replayed in FULL and its marker is findable (nxf 6j6v.waq9).
    run(
        "nxm",
        dir,
        &[
            "remember",
            MEMORY_BODY,
            "--key",
            "release-rule",
            "--category",
            "rules",
            "--introduction",
            MEMORY_INTRO,
        ],
    );
    run(
        "nxf",
        dir,
        &[
            "create",
            "--title",
            TICKET_TITLE,
            "--description",
            "so the board has something to show",
            "--priority",
            "1",
            "--type",
            "chore",
        ],
    );

    let roles = dir.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: Plan the work.\njob_title: Product manager\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("reviewer.yaml"),
        "handle: reviewer\nsystem_prompt: Review the diff.\njob_title: Reviewer\n\
         prime:\n  flow: false\n",
    )
    .unwrap();
    tmp
}

/// The absolute path to a workspace's store — what `--db` takes.
fn db_of(ws: &TempDir) -> String {
    ws.path()
        .join(".nxs")
        .join("db.sqlite")
        .to_str()
        .expect("utf8 path")
        .to_string()
}

/// Put a board item into `ws` that no other workspace has, so "which workspace answered" is a
/// positive assertion instead of the absence of something both of them carry.
fn marker(ws: &TempDir, title: &str) {
    run(
        "nxf",
        ws.path(),
        &[
            "create",
            "--title",
            title,
            "--description",
            "a marker only this workspace carries",
            "--priority",
            "1",
            "--type",
            "chore",
        ],
    );
}

/// **The acceptance, both halves in one workspace**: the persona that excludes the board gets no
/// `nxf prime` section, and the one that does not still gets one. A filter that had turned out to
/// be global — an env var, a config key — would pass either assertion alone and fail this pair.
#[test]
fn the_board_is_in_one_personas_block_and_absent_from_the_others() {
    let tmp = suite();
    let dir = tmp.path();

    let pm = run("nxs", dir, &["prime", "--persona", "pm"]);
    let reviewer = run("nxs", dir, &["prime", "--persona", "reviewer"]);

    assert!(
        pm.contains(TICKET_TITLE),
        "the PM's block carries the board:\n{pm}"
    );
    assert!(
        !reviewer.contains(TICKET_TITLE) && !reviewer.contains("# nexus-flow"),
        "`flow: false` removes the section for THIS persona, not an item from it:\n{reviewer}"
    );

    // What the filter did NOT touch: everything it did not name still contributes to both. Memory
    // contributes its INDEX (nxf 6j6v.xbnh) — the written introduction, not the body, which is what
    // `nxm recall` is for.
    for (who, block) in [("pm", &pm), ("reviewer", &reviewer)] {
        assert!(
            block.contains(MEMORY_INTRO) && !block.contains(MEMORY_BODY),
            "{who} keeps the memories it never excluded, as one line each:\n{block}"
        );
        assert!(
            block.contains(&format!("## You are `{who}`")),
            "{who} is told who it is:\n{block}"
        );
    }
}

/// The block is assembled in the umbrella's registry order — **flow, chat, memory** — so the two
/// surfaces that compose it (this verb and the spawn path) can be compared byte for byte.
///
/// **memory moved to the end in nxf 6j6v.xbnh, displacing chat from that position**, and the reason
/// is measured rather than aesthetic. The earlier argument was that the persona's identity should be
/// the nearest thing above the job it is about to be given. It lost to the host's cut-off: a large
/// session start is TRUNCATED (25.893 bytes pass, 32.000 are filed away behind a preview), so the
/// last block is not the most emphatic one — it is the one a truncation eats. What may be eaten is
/// what a session can fetch back afterwards, and that is memory (`nxm recall`, `nxm memories`).
/// Neither the board nor who-you-are-and-who-is-waiting is written down anywhere else. Identity is
/// still last among the blocks that must survive.
#[test]
fn the_sections_arrive_in_registry_order_with_the_recoverable_block_last() {
    let tmp = suite();
    let pm = run("nxs", tmp.path(), &["prime", "--persona", "pm"]);
    let at = |needle: &str| {
        pm.find(needle)
            .unwrap_or_else(|| panic!("{needle} is in the block:\n{pm}"))
    };
    let (flow, chat, memory) = (at("# nexus-flow"), at("# nexus-chat"), at("# nexus-memory"));
    assert!(
        flow < chat && chat < memory,
        "flow, then chat, then memory: {flow}/{chat}/{memory}\n{pm}"
    );
}

/// `--json` wraps the composed text rather than the modules' records: this verb answers "what does
/// this persona read", and the answer is one document. The per-module JSON is what the plain
/// `nxs prime --json` is for, and it is untouched.
#[test]
fn the_json_form_carries_the_persona_and_its_one_document() {
    let tmp = suite();
    let dir = tmp.path();
    let v: Value =
        serde_json::from_str(run("nxs", dir, &["--json", "prime", "--persona", "reviewer"]).trim())
            .expect("valid json");
    assert_eq!(v["persona"], "reviewer", "{v}");
    let text = v["prime"].as_str().expect("the composed block");
    assert!(text.contains("## You are `reviewer`"), "{text}");
    assert!(
        !text.contains("# nexus-flow"),
        "the filter holds here too: {text}"
    );

    // The plain fan-out is unchanged and still per-module.
    let plain: Value =
        serde_json::from_str(run("nxs", dir, &["--json", "prime"]).trim()).expect("valid json");
    assert!(
        plain["modules"].is_array() && plain["now"].is_string(),
        "`nxs prime --json` keeps its own shape: {plain}"
    );
}

/// **THE claim the changelog and the guide make, taken end to end** (review of PR #379, Code
/// Quality #2): a spawned persona reads what `nxs prime --persona <handle>` prints.
///
/// `crates/chat/tests/a_persona_reads_what_the_suite_knows.rs` compares a spawn against
/// `nxc prime --persona`, which is chat's own half and never touches the sibling modules — in a
/// chat-only workspace the two are the same thing, so that test holds without saying anything about
/// the composition. This one runs in a workspace where flow AND memory are live, so the claim has
/// three sections to get right and an order to get right between them.
#[test]
fn a_spawned_persona_reads_exactly_what_nxs_prime_persona_prints() {
    let tmp = suite();
    let dir = tmp.path();

    bin("nxc", dir)
        .env("NXC_WORKER", "sidecar")
        .env("NXC_SIDECAR", dir.join("no-such").join("main.mjs"))
        .args(["send", "--no-ref", "--to", "pm", "do the thing"])
        .assert()
        .success();

    let spec = std::fs::read_dir(dir.join(".nxs/agent-logs"))
        .expect("agent-logs")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".spec.json"))
        .expect("one spec written");
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(&spec).expect("readable")).expect("json");
    let prompt = spec["systemPrompt"].as_str().expect("a composed prompt");

    let printed = run("nxs", dir, &["prime", "--persona", "pm"]);
    let printed = printed.trim_end();
    assert!(
        prompt.starts_with(printed),
        "the spawned prompt must OPEN with exactly what the verb prints, byte for byte.\n         --- printed ({} B) ---\n{printed}\n--- prompt ({} B) ---\n{prompt}",
        printed.len(),
        prompt.len()
    );
    // …and it is genuinely the three-section composition, not chat's half twice over.
    for section in ["# nexus-flow", "# nexus-memory", "# nexus-chat"] {
        assert!(
            printed.contains(section),
            "the compared text carries {section}, so the comparison has something to prove:\n{printed}"
        );
    }
    assert!(
        prompt.ends_with("Plan the work."),
        "…and the persona's own system_prompt still stands last:\n{prompt}"
    );
}

/// **`--db` decides the workspace a persona is primed from, not the process working directory**
/// (review of PR #379, Code Quality #1).
///
/// The first cut of `RegistrySiblingPrimes` resolved its own workspace with
/// `Workspace::resolve(None, cwd)`, which made the persona spawn the ONE verb in this CLI that
/// ignored `--db`/`NXC_DB`. From a cwd inside workspace B, a `--db` naming workspace A put B's
/// board and B's memories into a persona that then ran against A — silently, because both
/// workspaces answer every other question correctly.
///
/// Driven from the OTHER workspace's directory on purpose: run from A's own cwd the bug is
/// invisible, which is exactly why it shipped.
///
/// **Both halves are asserted against a marker only ONE workspace has.** The first cut of this test
/// checked for `suite()`'s own ticket and memory, which both workspaces carry — so it passed with
/// the bug still in place. A test that cannot tell the two workspaces apart proves nothing about
/// which one answered.
#[test]
fn the_db_flag_and_not_the_working_directory_decides_which_board_a_persona_is_primed_from() {
    // This one guards `nxs prime --persona` itself, which never had the bug (it resolves its
    // workspace through `prime_cmd`). It is here so the verb the docs point people at cannot
    // acquire it later.
    let (a, b) = (suite(), suite());
    marker(&b, "WRONG-WORKSPACE-MARKER");

    let block = run(
        "nxs",
        b.path(),
        &["prime", "--persona", "pm", "--db", &db_of(&a)],
    );
    assert!(
        block.contains(TICKET_TITLE),
        "the block carries the board of the workspace `--db` named:\n{block}"
    );
    assert!(
        !block.contains("WRONG-WORKSPACE-MARKER"),
        "…and not the one the working directory would have resolved to:\n{block}"
    );
}

/// **THE regression test for Code Quality #1** — the path that actually had the bug: a REAL persona
/// spawn, where the composed block becomes a system prompt. `nxs prime --persona` never had it; the
/// static wiring the multicall dispatch hands `nexus_chat::run_from_with` did.
///
/// Verified to FAIL against the unfixed `RegistrySiblingPrimes` (`Workspace::resolve(None, cwd)`):
/// the persona's prompt then carries B's marker and not A's.
#[test]
fn a_persona_spawned_with_db_pointed_elsewhere_reads_that_workspaces_board() {
    let (a, b) = (suite(), suite());
    marker(&b, "WRONG-WORKSPACE-MARKER");

    // Summon from B's directory, against A's db, with the real sidecar worker over a nonexistent
    // script: `node` fails fast, but the spec JSON the model would have been started from is
    // already on disk.
    bin("nxc", b.path())
        .env("NXC_WORKER", "sidecar")
        .env("NXC_SIDECAR", b.path().join("no-such").join("main.mjs"))
        .args([
            "send",
            "--no-ref",
            "--db",
            &db_of(&a),
            "--to",
            "pm",
            "do the thing",
        ])
        .assert()
        .success();

    let spec = std::fs::read_dir(a.path().join(".nxs/agent-logs"))
        .expect("the session was minted in workspace A")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".spec.json"))
        .expect("one spec written");
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(&spec).expect("readable")).expect("json");
    let prompt = spec["systemPrompt"].as_str().expect("a composed prompt");

    assert!(
        prompt.contains(TICKET_TITLE),
        "the persona is primed from the workspace `--db` named:\n{prompt}"
    );
    assert!(
        !prompt.contains("WRONG-WORKSPACE-MARKER"),
        "…and NOT from the one its working directory would have resolved to. This is the finding: \
         both workspaces answer every other question correctly, so a prompt built from the wrong \
         one looks exactly like a prompt built from the right one:\n{prompt}"
    );
}

/// **The measurement the item is accepted on** (nxf 6j6v.4mmk / 6j6v.waq9, multiplied by this one):
/// what a persona is handed does not grow with the messages it has not read, nor with the memories
/// it does not need before it acts. Stated over a workspace that has been given plenty of both.
#[test]
fn the_block_does_not_grow_with_unread_messages_or_with_context_memories() {
    let tmp = suite();
    let dir = tmp.path();
    let before = run("nxs", dir, &["prime", "--persona", "pm"]).len();

    for i in 0..10 {
        run(
            "nxm",
            dir,
            &[
                "remember",
                &format!("a long architecture note {i}: {}", "x".repeat(2_000)),
                "--key",
                &format!("note-{i}"),
                "--category",
                "architecture",
                "--introduction",
                "in one line",
            ],
        );
        run(
            "nxc",
            dir,
            &[
                "send",
                "--no-ref",
                "--to",
                "pm",
                &format!("message {i}: {}", "y".repeat(2_000)),
            ],
        );
    }

    let after = run("nxs", dir, &["prime", "--persona", "pm"]).len();
    // 10 memories of ~2 KB and 10 messages of ~2 KB — 40 KB of material. What may legitimately
    // arrive is ten index lines; anything replaying a body would be an order of magnitude more.
    assert!(
        after < before + 2_000,
        "the block grew from {before} to {after} bytes on 40 KB of new material — something is \
         replaying bodies again"
    );
}

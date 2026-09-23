//! Integration tests for `nxc`'s contract verbs (nexus-chat-M1 T3, spec §5.2): init self-assembly,
//! agent-manifest, and prime — parity with `nxf`/`nxm`. Black-box over the built binary.

use assert_cmd::Command;
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs};
use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

/// **`NXC_ORIGIN` is PINNED here** (nxf 6j6v.07me), joining the actor and the clock. The CLI's
/// default `<origin>` became the workspace's own replica prefix — random per `TempDir` unless
/// `NXF_DETERMINISTIC_IDS` is set, which this file deliberately does not set. What these cases are
/// about is `init`'s self-assembly and what `prime` RENDERS, so an override keeps the handle they
/// quote (`local/alice`) a fixed, readable string instead of noise this suite would then have to
/// derive at every assertion. The real default is exercised where it is the subject:
/// `tests/parity.rs`'s `both_adapters_resolve_one_origin_for_one_workspace`, and `tests/golden.rs`,
/// which shows it to a reader as `ab12/alice`.
fn nxc(dir: &std::path::Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(dir)
        .env("NXC_ORIGIN", "local")
        .env("NXC_ACTOR", "alice")
        .env("NXC_NOW", "2026-06-20T10:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// Every SessionStart hook command in `.claude/settings.json`.
fn session_start_hooks(dir: &std::path::Path) -> Vec<String> {
    let settings = json_of(read(&dir.join(".claude/settings.json")).as_bytes());
    settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect()
}

#[test]
fn init_self_assembles_workspace_agent_files_and_hook() {
    let tmp = TempDir::new().unwrap();
    let out = nxc(tmp.path()).args(["--json", "init"]).assert().success();
    let v = json_of(&out.get_output().stdout);

    assert_eq!(v["ok"], true);
    // chat registered as an active module.
    assert_eq!(v["modules"], serde_json::json!(["chat"]));
    // foundation delegated: the substrate db exists.
    assert!(
        tmp.path().join(".nxs/db.sqlite").is_file(),
        "substrate db materialized"
    );

    // The shared AGENTS.md carries the single nxs-owned discovery pointer (naming the active tools +
    // pointing at `nxs prime`), NOT a per-module chat block — the chat rule lives in `nxc prime`.
    let agents = read(&tmp.path().join("AGENTS.md"));
    assert!(
        agents.contains("<!-- BEGIN NEXUS "),
        "single nxs-owned pointer: {agents}"
    );
    assert!(agents.contains("(chat)"), "names the active tool: {agents}");
    assert!(
        agents.contains("nxs prime"),
        "points at the umbrella: {agents}"
    );
    assert!(
        !agents.contains("BEGIN NEXUS-CHAT"),
        "no per-module NEXUS-CHAT block: {agents}"
    );

    // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this used to assert the single SessionStart
    // hook → the umbrella fan-out, "NOT `nxc prime` (that is only the fan-out target the manifest
    // declares)". The host truncates per hook OUTPUT, so the wiring is one entry per active module
    // and `nxc prime` is what a chat-only workspace wires.
    //
    // **Superseded 2026-08-29 (nxf 6j6v.1k6y): there is no tail here at all.** The line above used
    // to build the expectation as `"nxc prime" + HOOK_FALLBACK`, "rather than spelled out, since
    // the tail grew in 6j6v.8q88 and could move again". It did move again — onto memory's entry,
    // because `NEXUS_MEMORY.md` is memory's projection and stands in for `nxm prime` alone. A
    // chat-only workspace has no memory module, so no document and no tail; the defensive
    // interpolation could express a tail that MOVED but not one that is absent.
    let hook = "nxc prime".to_string();
    assert_eq!(session_start_hooks(tmp.path()), vec![hook.clone()]);
    assert_eq!(v["hook"]["hook_added"], true);
    assert_eq!(v["hook"]["commands"], serde_json::json!([hook]));

    // A chat-only init leaves flow + memory addable + inactive → the additive `advertisement` field
    // cross-sells them (agent-addressed; no setup without the user's permission). This is the T3
    // "discoverable through the suite" contract, working in both directions.
    let ad = v["advertisement"]
        .as_str()
        .expect("advertisement field present when flow + memory are inactive");
    assert!(
        ad.contains("issue tracker") && ad.contains("nexus-memory"),
        "cross-sells flow and memory: {ad}"
    );
    assert!(ad.contains("nxs init"), "points at nxs init: {ad}");
}

#[test]
fn init_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    nxc(tmp.path()).arg("init").assert().success();
    let agents_first = read(&tmp.path().join("AGENTS.md"));
    let settings_first = read(&tmp.path().join(".claude/settings.json"));

    // A second init must not duplicate the block or the hook.
    let out = nxc(tmp.path()).args(["--json", "init"]).assert().success();
    assert_eq!(
        json_of(&out.get_output().stdout)["hook"]["hook_added"],
        false
    );
    assert_eq!(
        read(&tmp.path().join("AGENTS.md")),
        agents_first,
        "block stable"
    );
    assert_eq!(
        read(&tmp.path().join(".claude/settings.json")),
        settings_first,
        "settings stable"
    );
    assert_eq!(
        agents_first.matches("<!-- BEGIN NEXUS ").count(),
        1,
        "one nxs-owned pointer"
    );
}

#[test]
fn agent_manifest_declares_nxcs_contribution() {
    let tmp = TempDir::new().unwrap();
    // Static contribution — no workspace needed.
    let out = nxc(tmp.path())
        .args(["--json", "agent-manifest"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["prime_command"], "nxc prime");
    assert_eq!(v["hook"]["event"], "SessionStart");
    assert_eq!(v["hook"]["command"], "nxc prime");
    assert!(
        !tmp.path().join(".nxs").exists(),
        "agent-manifest creates no workspace"
    );
}

/// A channel with members, and a post into it from a named actor — the fixture `channels create`
/// + `channels join` + `send <cid> <body>` used to build.
///
/// All three verbs went with 6j6v.dvyq §3: a channel is a DECLARATION now, and a channel no
/// declaration names is not addressable. The READS under test in this file — `prime`'s catch-up
/// fields, `--consumer` — are membership-scoped and still need a channel with members and traffic
/// in it, so the fixture is written where the verbs used to write it: straight to the store, with
/// the same fields and the same membership ops.
fn seed_channel(dir: &std::path::Path, name: &str, members: &[&str]) -> String {
    let mut store = Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    let cid = store.mint_channel_id();
    let author = members.first().copied().unwrap_or("local/alice");
    store.set_channel_field(&cid, "name", name, author);
    store.set_channel_field(&cid, "kind", "group", author);
    store.set_channel_field(&cid, "origin", "local", author);
    for m in members {
        store.add_member(&cid, m, author);
    }
    cid
}

/// Post into `cid` as `actor` — the library half of the `send <cid> <body>` this file used to run.
fn post(dir: &std::path::Path, cid: &str, actor: &str, body: &str, disposition: Disposition) {
    let mut store = Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    store.set_wall_clock("2026-06-20T10:00:00Z");
    store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: cid.into(),
        sender: format!("local/{actor}"),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition,
        thread_id: None,
        refs: Refs::default(),
        body: body.into(),
    });
}

/// Seed a channel and two messages (one per disposition) that the caller `alice` will read as
/// unread. Returns the minted channel id.
/// Two messages from ANOTHER agent, one per disposition. It was `seed_unread` until nxf 6j6v.4d2z:
/// there is no unread set for it to seed any more, and the dispositions it still writes are a field
/// on the wire rather than a read state. What it is for is unchanged — the block below must stay
/// byte-identical over a channel that has traffic in it.
fn seed_two_messages(dir: &std::path::Path) -> String {
    let cid = seed_channel(dir, "review", &["local/alice", "local/bob"]);
    // A second agent posts, so the messages are genuine inbound work for alice.
    post(dir, &cid, "bob", "please review PR 42", Disposition::InTurn);
    post(
        dir,
        &cid,
        "bob",
        "look next session",
        Disposition::NextSession,
    );
    cid
}

/// It was `prime_pins_the_rule_lists_commands_and_replays_both_dispositions` until nxf 6j6v.4d2z,
/// and by then the name asserted the opposite of the body: nxf 6j6v.4mmk had already stopped the
/// block replaying anything, and point (3) below says so in as many words. A test's NAME is a claim
/// about its body and can be false while everything stays green (PR #450 review, Code Quality #2);
/// renamed here rather than left, since this change is what removed the two sibling tests beneath
/// it and the dispositions themselves.
#[test]
fn prime_pins_the_rule_lists_the_commands_and_replays_no_message_text() {
    let tmp = TempDir::new().unwrap();
    nxc(tmp.path()).arg("init").assert().success();
    seed_two_messages(tmp.path());

    let out = nxc(tmp.path()).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout);

    // (1) the coordination rule, folded into the intro paragraph as a prohibition rather than a
    // separately labelled rule (nxf h4d3, task 3) — coordination goes through nxc, not ad-hoc
    // notes/files.
    assert!(
        text.contains("coordinate") && text.to_lowercase().contains("ad-hoc"),
        "mandates nxc over ad-hoc coordination: {text}"
    );
    assert!(
        text.contains("never replayed"),
        "spells out the consequence of ad-hoc notes: {text}"
    );
    // (2) the verbs task 3 kept (nxf h4d3): `send --to` and `reply --thread`. It kept a third,
    // `withdraw`, as the rescue verb for a commission parked behind the working copy — and nxf
    // 6j6v.ezbr took it out again on the owner's decision of 2026-09-20: it is a person's verb, a
    // session this workspace started is refused it, and this block is what every session reads
    // first. An agent that wants a round stopped escalates, and the `--escalate` line says so.
    // `list`, `status`,
    // `threads show`, `search` and `transcript show` were pulled from this block — `nxc --help`
    // teaches all five on demand, and the new intro paragraph says so — even though `threads show`
    // was itself nxf 6j6v.4mmk's own MEASURED addition (28 of 66 persona sessions used it). `inbox`
    // and `read` stay off — 0 of 66, and the dump that was their only reason is gone. They are not
    // merely off this list since 6j6v.1gm9: they are not verbs at all (`consolidated_entrances.rs`
    // pins the refusal).
    assert!(
        text.contains("nxc send --to")
            && text.contains("nxc reply --thread")
            && text.contains("nxc reply --thread <id> --escalate"),
        "lists the verbs the block keeps, escalation included: {text}"
    );
    assert!(
        !text.contains("withdraw"),
        "and does not offer `withdraw` to an agent in any form (nxf 6j6v.ezbr): {text}"
    );
    assert!(
        !text.contains("nxc list")
            && !text.contains("nxc threads show")
            && !text.contains("nxc status")
            && !text.contains("nxc search")
            && !text.contains("nxc transcript"),
        "and does not teach the five verbs an app can pull on demand instead: {text}"
    );
    assert!(
        !text.contains("nxc inbox") && !text.contains("nxc read"),
        "and does NOT teach the two that left the agent surface: {text}"
    );
    // (3) NEITHER disposition is rendered any more (nxf 6j6v.4mmk). The catch-up moment is the
    // TRIGGER MESSAGE, not this block; what stood here cost 98 % of a measured 199.491 bytes.
    assert!(
        !text.contains("please review PR 42") && !text.contains("look next session"),
        "renders no message text at all: {text}"
    );
    assert!(
        !text.contains("## Unread"),
        "and has no unread section to count into: {text}"
    );
    // (4) the when-stuck paragraph moved to `nxc guide limits-and-safety` (nxf h4d3, task 3) —
    // already documented there in full, so it no longer rides every session start.
    assert!(
        !text.contains("When something does not move"),
        "the paragraph moved to `nxc guide limits-and-safety`, and no longer rides every prime: {text}"
    );
}

// Two tests stood here and went with the unread apparatus (nxf 6j6v.4d2z).
//
// `prime_json_carries_both_dispositions_and_count` was the `--json` half of the pair above: the
// human block renders no message text, and the DATA still carried it, split by disposition with a
// `count`. There is no such data any more.
//
// `prime_consumer_override_reads_a_different_agents_inbox` made `--consumer`'s effect OBSERVABLE
// rather than merely echoed — two agents with different unread sets, and the flag selecting between
// them — and it said of itself that since 6j6v.1gm9 it was the last place the override was
// observable over the unread. That was true, and the unread is gone. **The flag is not**, and it is
// still observable: `nxc status --consumer`, `nxc search --consumer` and `nxc threads list
// --consumer` all select a different agent's view, and each is exercised where it lives. What is
// lost with this test is nothing about `--consumer`; it is the last CLI-visible consequence of
// `hj12` (a sender's own posts are not unread to itself), which had no unread set left to be a rule
// about — see `docs/specs/nexus-chat-M2.md` §4.5.

/// Replaces `prime_with_no_unread_says_caught_up` (nxf 6j6v.4mmk): there is no unread section to
/// be empty any more, so "caught up" has nothing to say and is gone with it. The block a workspace
/// with nothing unread emits is the block EVERY workspace emits — that is the property now.
#[test]
fn prime_says_nothing_about_unread_at_all() {
    let tmp = TempDir::new().unwrap();
    nxc(tmp.path()).arg("init").assert().success();
    let out = nxc(tmp.path()).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(!text.contains("## Unread"), "no unread section: {text}");
    assert!(
        !text.to_lowercase().contains("caught up"),
        "and no line about being caught up either: {text}"
    );
}

#[test]
fn init_quiet_prints_nothing_yet_still_wires_workspace_and_one_hook() {
    // Driven/quiet mode: `nxc init --quiet` renders no banner (so no sub-init output bleeds through
    // when the umbrella drives it) while still wiring the module's own SessionStart hook (one per
    // active module since nxf n2m6 + a2a1).
    let tmp = TempDir::new().unwrap();
    let out = nxc(tmp.path()).args(["init", "--quiet"]).assert().success();
    assert!(
        out.get_output().stdout.is_empty(),
        "quiet init prints nothing to stdout: {:?}",
        String::from_utf8_lossy(&out.get_output().stdout)
    );
    assert!(
        tmp.path().join(".nxs/db.sqlite").is_file(),
        "workspace still created"
    );
    assert_eq!(
        session_start_hooks(tmp.path()),
        vec!["nxc prime".to_string()],
        "the module's own hook is wired even when quiet — and carries no `|| cat` tail, because \
         the document it would read is memory's and memory is not active here (nxf 6j6v.1k6y)"
    );
}

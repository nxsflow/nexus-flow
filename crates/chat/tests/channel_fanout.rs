//! Black-box CLI tests for channel fan-out on `send`/`ask` (nxf ticket 6j6v.cahk): once a
//! `send`/`ask` target resolves to a DECLARED channel (`channels.yaml`), `nxc` opens a real thread
//! (via `facade::ask`) and triggers every declared member except the sender via the existing
//! role-trigger machinery, exactly as `verbs.rs`'s `send_role_mints_session_stamps_return_address_
//! and_triggers`/`workflow_start.rs` exercise triggering: spawn the real `nxc` binary against a
//! `DryWorker` (`NXC_WORKER=dry`/`NXC_DRY_LOG`, never spawns anything real) and assert on the dry
//! log plus the folded substrate state.
//!
//! Determinism mirrors `verbs.rs`: `NXF_DETERMINISTIC_IDS=1`, a fixed `NXC_ACTOR=alice`/
//! `NXC_ORIGIN=local`/`NXC_NOW`. The declared-channel mechanism operates entirely in the BARE
//! role-handle namespace (`channel.members`/`expects`, matching `resolve_role`'s own namespace —
//! see `channel.rs`'s `fan_out_targets` doc) — DISTINCT from the qualified `origin/actor` chat
//! identity `caller_handle()` uses for message `sender`/DM membership elsewhere. `sender` as
//! compared against declared members is therefore the bare `actor()` value (`alice`, the `nxc()`
//! helper's `NXC_ACTOR`), not the qualified `local/alice`.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", "2026-07-22T00:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn personas_dir(tmp: &TempDir) -> PathBuf {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    roles
}

fn write_role(roles: &std::path::Path, handle: &str) {
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
    )
    .unwrap();
}

fn write_channels_yaml(roles: &std::path::Path, yaml: &str) {
    std::fs::write(roles.join("channels.yaml"), yaml).unwrap();
}

/// Every dry-log line that opens a trigger entry (`trigger role=... session=... resume=... msg=
/// ...`). The message field itself may embed real newlines (this ticket's trigger text embeds
/// `\n\n` before the `nxc reply` instruction), so only the FIRST physical line of each logical
/// entry starts with `trigger role=` — this is exactly what distinguishes one trigger from another.
fn trigger_header_lines(dry: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dry)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(str::to_string)
        .collect()
}

/// Extract the `key=` token from a trigger header line (up to the next space) — `role`/`session`/
/// `resume` are all single-token values on that line.
fn field(line: &str, key: &str) -> String {
    let marker = format!("{key}=");
    let start = line
        .find(&marker)
        .unwrap_or_else(|| panic!("no {marker} in {line}"))
        + marker.len();
    line[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The qualified identity of the channel supervisor (nxf 6j6v.pf6j) — what a `send`/`ask` into a
/// declared channel now reports as the CHANNEL THREAD's `expects`, because that thread has exactly
/// two ends: the requester and the supervisor.
const SUPERVISOR: &str = "local/__channel__";

/// The effective member set, read from where it now lives.
///
/// Before 6j6v.pf6j this WAS the receipt's `expects`: one thread with every member in its
/// `expects_reply_from`. It is now one expectation per MEMBER THREAD below the channel thread, so
/// every assertion that used to read the receipt reads this instead — the same set, the same
/// qualified shape, a different (and two-ended) place.
fn member_expects(tmp: &TempDir, channel_thread: &str) -> Vec<String> {
    let store = open_store(tmp);
    let mut out: Vec<String> = store
        .supervised_children(channel_thread)
        .unwrap()
        .iter()
        .filter_map(|t| store.thread_quorum(t, "2026-07-22T00:00:00Z").unwrap())
        .filter(|q| q.opener.as_deref() == Some(SUPERVISOR))
        .flat_map(|q| q.expects)
        .collect();
    out.sort();
    out
}

fn message_count(store: &ChatStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap()
}

fn thread_count(store: &ChatStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))
        .unwrap()
}

fn session_count(store: &ChatStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM session_map", [], |r| r.get(0))
        .unwrap()
}

// ---- ask on a declared channel ----------------------------------------------

#[test]
fn send_to_on_a_declared_channel_fans_out_to_every_member_except_the_sender() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    // "alice" (the sender) is deliberately ALSO listed as a declared member — proves the exclusion
    // is real, not coincidental (alice has no .nxs-personas/alice.yaml, so if she were ever
    // resolved/ triggered, this would fail loudly rather than silently passing).
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(&roles, "- name: review\n  members: [alice, bob, carol]\n");
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    // `send --to`'s own receipt, not `ask`'s: it names what the target turned out to BE, which is
    // the resolution that decides everything below. `persisted: true` used to stand here and was
    // `AskReceipt`'s field; the receipt this entrance returns carries the message id instead.
    assert_eq!(v["target"], "channel", "{v}");
    assert!(
        v["message_id"]
            .as_str()
            .is_some_and(|m| m.starts_with("m-")),
        "{v}"
    );
    let thread = v["thread_id"].as_str().unwrap().to_string();
    // The effective expects = declared members minus the sender — computed once, reused for the
    // trigger loop below too. QUALIFIED (`origin/handle`), not the bare role-handle shape
    // `channel.members` declares them in: `thread_quorums`' completion join matches
    // `expects_reply_from` against a real reply's `sender`, which is ALWAYS qualified.
    assert_eq!(
        v["expects"],
        serde_json::json!([SUPERVISOR]),
        "the CHANNEL thread has exactly two ends — the requester and the supervisor (6j6v.pf6j)"
    );
    assert_eq!(
        member_expects(&tmp, &thread),
        vec!["local/bob", "local/carol"],
        "…and the effective member set is one expectation per member thread below it"
    );

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        2,
        "N members minus sender must fire: {lines:?}"
    );
    let roles_triggered: Vec<String> = lines.iter().map(|l| field(l, "role")).collect();
    assert!(roles_triggered.contains(&"bob".to_string()));
    assert!(roles_triggered.contains(&"carol".to_string()));
    assert!(
        !roles_triggered.contains(&"alice".to_string()),
        "the sender must never be triggered: {lines:?}"
    );

    // Fresh mints a DISTINCT internal session per member.
    let sessions: Vec<String> = lines.iter().map(|l| field(l, "session")).collect();
    assert_ne!(
        sessions[0], sessions[1],
        "each member gets its own fresh session: {lines:?}"
    );

    // The trigger message embeds a literal thread id as text (mirrors `reply`'s quorum-wake
    // pattern) — but since 6j6v.pf6j it is the member's OWN thread, never the channel thread: a
    // member answers in its own two-ended conversation, and nothing points it at the level above.
    let log = std::fs::read_to_string(&dry).unwrap();
    for member_thread in open_store(&tmp).supervised_children(&thread).unwrap() {
        assert!(
            log.contains(&format!("nxc reply --thread {member_thread}")),
            "trigger body must carry the member's own thread id: {log}"
        );
    }
    assert!(
        !log.contains(&format!("--thread {thread}")),
        "no member is pointed at the channel thread: {log}"
    );
}

// `send_on_a_declared_channel_fans_out_the_same_as_ask` was here, and it was worth having for
// exactly as long as a declared channel had TWO doors: it drove `send <channel> <body>` and
// asserted the fan-out matched `ask`'s. 6j6v.dvyq §3 leaves one door, so there is no second answer
// to compare against — `send_to_on_a_declared_channel_fans_out_to_every_member_except_the_sender`
// above is that assertion, made once.
//
// The three `asks_explicit_expect_*` tests went with it. They pinned the per-call `--expect`
// override — that it beat the declared policy, that it accepted a bare handle, that it still
// excluded the sender. The override is gone (§3): with every channel declared, who must answer is
// written in the channel, and `ChannelOpenRequest` no longer carries a field to override it with.

#[test]
fn expects_subset_triggers_only_the_named_subset_not_every_declared_member() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "dave");
    // Deliberately NO .nxs-personas/carol.yaml: carol is a declared MEMBER but outside the Subset,
    // so she must never be resolved/triggered — if the implementation mistakenly used `members`
    // instead of `expects` here, this would fail loudly (unknown role) rather than silently
    // passing.
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [alice, bob, carol, dave]\n  expects: [bob, dave]\n",
    );
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["expects"], serde_json::json!([SUPERVISOR]));
    assert_eq!(
        member_expects(&tmp, v["thread_id"].as_str().unwrap()),
        vec!["local/bob", "local/dave"]
    );

    let lines = trigger_header_lines(&dry);
    let roles_triggered: Vec<String> = lines.iter().map(|l| field(l, "role")).collect();
    assert_eq!(roles_triggered.len(), 2, "{lines:?}");
    assert!(roles_triggered.contains(&"bob".to_string()));
    assert!(roles_triggered.contains(&"dave".to_string()));
    assert!(!roles_triggered.contains(&"carol".to_string()));
}

// ---- preflight-before-persist -------------------------------------------------

#[test]
fn an_unresolvable_declared_member_fails_the_whole_call_with_nothing_persisted() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    // "ghost" is declared but has no .nxs-personas/ghost.yaml — deliberately unresolvable.
    write_channels_yaml(&roles, "- name: review\n  members: [alice, ghost]\n");
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .failure();
    let v = json_of(&out.get_output().stdout);
    let kind = v["error"]["kind"].as_str().unwrap();
    assert!(
        kind == "validation" || kind == "not_found",
        "expected a validation/not_found rejection, got: {v}"
    );

    assert!(
        !dry.exists(),
        "no member may be triggered before the whole invocation's preflight passes"
    );
    let store = open_store(&tmp);
    assert!(
        !store.channel_exists("decl:review").unwrap(),
        "the declared channel must not be materialized on a failed preflight"
    );
    assert_eq!(message_count(&store), 0, "no message may be persisted");
    assert_eq!(thread_count(&store), 0, "no thread may be opened");
    assert_eq!(session_count(&store), 0, "no session may be minted");
}

/// A declaration written before nxf 6j6v.fepb retired `member_session:` must now RUN, not be
/// refused. This is the same guarantee `channel_load.rs` holds at the parse layer, taken all the
/// way through the CLI: `member_session: resume` was rejected by name in preflight before this
/// item — the whole invocation failed with nothing persisted — and an author who never touches
/// their `channels.yaml` again would have met that rejection on upgrade if the removal had left
/// `deny_unknown_fields` (or the check) behind.
///
/// The value chosen is `resume`, deliberately: `fresh` was the default and would pass a stale
/// check anyway, so only the rejected value proves the rejection is gone.
#[test]
fn a_channel_yaml_still_setting_the_retired_member_session_key_fans_out_normally() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [alice, bob]\n  member_session: resume\n",
    );
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert!(
        v["thread_id"].as_str().is_some_and(|t| !t.is_empty()),
        "the fan-out must open a real board thread: {v}"
    );

    // The fan-out really happened, rather than merely not failing: the declared channel is
    // materialized and the one effective member (alice is the sender) was triggered.
    let store = open_store(&tmp);
    assert!(store.channel_exists("decl:review").unwrap());
    assert!(store.is_member("decl:review", "bob").unwrap());
    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.iter().map(|l| field(l, "role")).collect::<Vec<_>>(),
        vec!["bob".to_string()],
        "the one effective member (alice is the sender) must fire: {lines:?}"
    );
    assert_eq!(
        field(&lines[0], "resume"),
        "-",
        "and the retired key steered nothing: a fan-out mints a fresh session either way, which \
         is exactly why `resume` had nothing to ask for"
    );
}

// ---- membership materialization -----------------------------------------------

#[test]
fn sender_and_every_triggered_member_can_read_the_thread_back_afterward() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    // The declared members do NOT include the sender (the common real-world shape: a `pm` opens a
    // review channel it isn't itself declared a member of).
    write_channels_yaml(&roles, "- name: review\n  members: [bob, carol]\n");
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let thread = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let store = open_store(&tmp);
    assert!(
        store.is_member("decl:review", "bob").unwrap(),
        "declared member bob must be a substrate member"
    );
    assert!(
        store.is_member("decl:review", "carol").unwrap(),
        "declared member carol must be a substrate member"
    );
    assert!(
        store.is_member("decl:review", "local/alice").unwrap(),
        "the sender's own chat identity must ALSO be a substrate member, even though it wasn't \
         itself declared, so it can read its own opened thread back"
    );

    // Not just inference — an actual `threads show` call for the opener (default consumer, its
    // own real ambient identity) and for each bare declared member handle.
    nxc(&tmp)
        .args(["--json", "threads", "show", &thread])
        .assert()
        .success();
    nxc(&tmp)
        .args(["--json", "threads", "show", &thread, "--consumer", "bob"])
        .assert()
        .success();
    nxc(&tmp)
        .args(["--json", "threads", "show", &thread, "--consumer", "carol"])
        .assert()
        .success();
}

#[test]
fn ensure_declared_channel_materialization_is_idempotent_across_two_sends() {
    // Re-opening a SECOND board on the same declared channel must not inflate membership past the
    // 3 real participants (sender + 2 members) — mirrors `ensure_dm_channel`'s own idempotent-
    // reopen guarantee.
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(&roles, "- name: review\n  members: [bob, carol]\n");
    let dry = tmp.path().join("dry.log");

    for _ in 0..2 {
        nxc(&tmp)
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", &dry)
            .args(["--json", "send", "--to", "review", "please review"])
            .assert()
            .success();
    }

    let store = open_store(&tmp);
    let members = store.resolve_membership("decl:review").unwrap();
    assert_eq!(
        members.len(),
        3,
        "sender + 2 declared members, no inflation on re-open: {members:?}"
    );
}

// ---- round-trip: a REAL reply from every triggered member completes the CHANNEL ----
//
// The regression this closes: the expectation a member is measured against must carry the QUALIFIED
// `origin/handle` chat identity, not the bare role handle `channel.members`/`resolve_role` use —
// `thread_quorums`' completion join matches `expects_reply_from` against a real reply's `sender`
// field with an EXACT STRING comparison, and that `sender` is always qualified. Fanning out members
// and asserting on `--json expects` alone (the other tests above) cannot catch this — only actually
// replying AS each triggered member (its own ambient identity, exactly as a real triggered role
// session would) and checking the derived state closes the loop.
//
// REWRITTEN for nxf 6j6v.pf6j. It used to drive both members into ONE board thread and assert that
// thread's `outstanding` shrink from 2 to 0. There is no such thread any more: each member answers
// in its own two-ended conversation, and what completes the CHANNEL thread is the SUPERVISOR — it
// waits for the set of member threads and then runs the channel's `on_complete` policy, which for
// `pass_through` self-satisfies the channel thread. The end state asserted here is the same one the
// old test asserted (`complete: true` on the channel thread, visible through `threads show`); what
// changed is that it is reached through four threads instead of one.

#[test]
fn a_real_reply_from_each_triggered_member_actually_completes_the_channel() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(&roles, "- name: review\n  members: [alice, bob, carol]\n");
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let thread = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Not yet complete: the supervisor owes the answer up here and has nothing to give yet.
    let store = open_store(&tmp);
    let q = store
        .thread_quorum(&thread, "2026-07-22T00:00:00Z")
        .unwrap()
        .expect("quorum row exists for a freshly-opened channel thread");
    assert!(!q.complete, "must not be complete before anyone replies");
    assert_eq!(q.outstanding, vec![SUPERVISOR.to_string()], "{q:?}");
    // Which member answers where: one thread each, expectation qualified.
    let mut member_threads: Vec<(String, String)> = store
        .supervised_children(&thread)
        .unwrap()
        .into_iter()
        .map(|t| {
            let mq = store
                .thread_quorum(&t, "2026-07-22T00:00:00Z")
                .unwrap()
                .unwrap();
            (t, mq.expects[0].clone())
        })
        .collect();
    member_threads.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        member_threads
            .iter()
            .map(|(_, h)| h.as_str())
            .collect::<Vec<_>>(),
        vec!["local/bob", "local/carol"]
    );
    drop(store);

    // bob replies AS bob, in HIS OWN thread — exactly the identity and the target a real triggered
    // role session is given (its `NXC_ACTOR` is the bare role handle, and its wake text names this
    // thread).
    let bob_thread = member_threads[0].0.clone();
    let carol_thread = member_threads[1].0.clone();
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .args(["reply", "--thread", &bob_thread, "lgtm"])
        .assert()
        .success();
    let store = open_store(&tmp);
    assert!(
        store
            .thread_quorum(&bob_thread, "2026-07-22T00:00:00Z")
            .unwrap()
            .unwrap()
            .complete,
        "bob's own thread is discharged"
    );
    let q = store
        .thread_quorum(&thread, "2026-07-22T00:00:00Z")
        .unwrap()
        .unwrap();
    assert!(!q.complete, "carol hasn't replied yet: {q:?}");
    assert_eq!(q.outstanding, vec![SUPERVISOR.to_string()], "{q:?}");
    drop(store);

    // carol replies too — the SET is now settled, so the supervisor consolidates and the channel
    // thread genuinely completes.
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .args(["reply", "--thread", &carol_thread, "approved"])
        .assert()
        .success();
    let store = open_store(&tmp);
    let q = store
        .thread_quorum(&thread, "2026-07-22T00:00:00Z")
        .unwrap()
        .unwrap();
    assert!(
        q.complete,
        "both member threads answered — the supervisor must have consolidated: {q:?}"
    );
    assert!(q.outstanding.is_empty(), "{:?}", q.outstanding);

    // Also verifiable through the ordinary CLI surface, not just the store directly.
    let show = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "show", &thread])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(show["complete"], true, "{show}");
}

// ---- declared `visibility` is actually enforced by `threads show` (independent review, Code
// Quality #1 / Integrity #3, High) -----------------------------------------------------------

fn bodies_of(v: &Value) -> Vec<String> {
    v["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|m| m["body"].as_str().unwrap().to_string())
        .collect()
}

/// This used to hardcode `Visibility::AllMembers` regardless of what the channel declared —
/// silently never enforcing `requester_only`, letting any reviewer read every other reviewer's reply
/// via `nxc threads show`. Proves the real end-to-end wiring.
///
/// REWRITTEN for nxf 6j6v.pf6j. It used to drive both members into ONE board thread and then check
/// that the FILTER hid each one's reply from the other. There is no such thread any more, so the
/// question this test asks is now asked of the member threads — where the two answers actually live —
/// and it covers the one thing the two levels genuinely changed about the policy: a member thread's
/// OPENER is the supervisor, which is machinery and can never be a reader, so a supervised thread
/// inherits its parent's opener as the requester. Without that the requester who commissioned the
/// channel would be locked out of the very answers `requester_only` reserves FOR it.
#[test]
fn requester_only_visibility_is_actually_enforced_across_the_member_threads() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  visibility: requester_only\n",
    );
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let channel_thread = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let thread_of = |handle: &str| -> String {
        let store = open_store(&tmp);
        let want = format!("local/{handle}");
        store
            .supervised_children(&channel_thread)
            .unwrap()
            .into_iter()
            .find(|t| {
                store
                    .thread_quorum(t, "2026-07-22T00:00:00Z")
                    .unwrap()
                    .is_some_and(|q| q.expects == [want.clone()])
            })
            .expect("a thread per member")
    };
    let bob_thread = thread_of("bob");
    let carol_thread = thread_of("carol");

    for (who, thread) in [("bob", &bob_thread), ("carol", &carol_thread)] {
        nxc(&tmp)
            .env("NXC_ACTOR", who)
            .args(["reply", "--thread", thread, &format!("{who}'s finding")])
            .assert()
            .success();
    }

    let show = |thread: &str, consumer: &str| -> Value {
        json_of(
            &nxc(&tmp)
                .args(["--json", "threads", "show", thread, "--consumer", consumer])
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };

    // bob in his OWN thread: the supervisor's task plus his own finding.
    assert_eq!(
        bodies_of(&show(&bob_thread, "bob")),
        vec!["please review", "bob's finding"],
        "his own conversation, unchanged"
    );
    // …and in CAROL's: the task only. Under `requester_only` a member reads no other member's
    // finding — and here it is not even a filtered view of a shared board, it is somebody else's
    // conversation.
    assert_eq!(
        bodies_of(&show(&carol_thread, "bob")),
        vec!["please review"],
        "bob must not see carol's finding under requester_only"
    );
    assert_eq!(
        bodies_of(&show(&bob_thread, "carol")),
        vec!["please review"],
        "…and symmetrically"
    );

    // The REQUESTER (alice, who ran the `ask`) sees every member's finding, in every member thread.
    // The supervisor opened those threads, so without inheriting the requester from the channel
    // thread this read would be filtered down to the task line and the policy would mean the
    // opposite of what it says.
    assert_eq!(
        bodies_of(&show(&bob_thread, "local/alice")),
        vec!["please review", "bob's finding"],
    );
    assert_eq!(
        bodies_of(&show(&carol_thread, "local/alice")),
        vec!["please review", "carol's finding"],
    );
}

/// Regression: a declared channel that explicitly declares `visibility: all_members` must show
/// every reply to every member, unfiltered.
///
/// REWRITTEN for nxf 6j6v.pf6j (fix round 1). It was left in the pre-model shape — both members
/// replying into the CHANNEL thread — where it still passed and no longer covered its own name: the
/// filter it is about was being asked of a thread that, in the model this branch ships, carries no
/// member reply at all. `all_members` now has to be checked where the replies actually are, which is
/// across the member threads, and that is a STRONGER claim than the old one: under `requester_only`
/// a member sees nothing of a sibling's thread (the test above), and the only thing that separates
/// the two policies is this one.
#[test]
fn all_members_visibility_on_a_declared_channel_shows_everything_to_every_member() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  visibility: all_members\n",
    );
    let dry = tmp.path().join("dry.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let channel_thread = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let thread_of = |handle: &str| -> String {
        let store = open_store(&tmp);
        let want = format!("local/{handle}");
        store
            .supervised_children(&channel_thread)
            .unwrap()
            .into_iter()
            .find(|t| {
                store
                    .thread_quorum(t, "2026-07-22T00:00:00Z")
                    .unwrap()
                    .is_some_and(|q| q.expects == [want.clone()])
            })
            .expect("a thread per member")
    };
    let bob_thread = thread_of("bob");
    let carol_thread = thread_of("carol");

    for (who, thread) in [("bob", &bob_thread), ("carol", &carol_thread)] {
        nxc(&tmp)
            .env("NXC_ACTOR", who)
            .args(["reply", "--thread", thread, &format!("{who}'s finding")])
            .assert()
            .success();
    }

    let show = |thread: &str, consumer: &str| -> Value {
        json_of(
            &nxc(&tmp)
                .args(["--json", "threads", "show", thread, "--consumer", consumer])
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };

    // The point of the policy: bob reads CAROL's thread in full — the one thing `requester_only`
    // refuses. Under `all_members` there is no filtering at any level.
    assert_eq!(
        bodies_of(&show(&carol_thread, "bob")),
        vec!["please review", "carol's finding"],
        "all_members must show a sibling member's whole thread"
    );
    assert_eq!(
        bodies_of(&show(&bob_thread, "carol")),
        vec!["please review", "bob's finding"],
        "…symmetrically"
    );
    assert_eq!(
        bodies_of(&show(&bob_thread, "bob")),
        vec!["please review", "bob's finding"],
        "…and a member's own thread is unfiltered as it always was"
    );
}

// ---- regression: unchanged behavior for a non-declared channel ---------------

#[test]
fn a_channel_name_that_matches_no_declared_channel_is_refused_by_name() {
    // Three tests stood here, and all three described the SAME shape from different angles: a name
    // that matched no declaration was a raw substrate channel id, so `ask` opened a board on it
    // with an explicit `--expect` (and refused without one), and `send` posted to it plainly.
    //
    // 6j6v.dvyq §3 removes that shape, so the three collapse into one: the id resolves to nothing a
    // declaration names, and the refusal SAYS that rather than "no such target" — the channel is
    // real and spelled correctly, and the missing thing is the declaration. The direct conversation
    // `send --to <persona>` opens is the only undeclared channel a workspace can still produce,
    // which is why it is the fixture: `channels create` is gone too.
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");

    let opened = json_of(
        &nxc(&tmp)
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
            .args(["--json", "send", "--to", "bob", "hello"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let cid = opened["channel"].as_str().expect("the DM's channel id");

    let out = nxc(&tmp)
        .args(["--json", "send", "--to", cid, "please review"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned()
        + &String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        err.contains(cid) && err.contains("no declaration names it"),
        "the channel is real; what is missing is the declaration: {err}"
    );
}

// ---- fail-closed re-validation at the point of use (independent review, Integrity #4, Medium) --

/// `validate_channels`'s own `on_complete: summarize requires summary_prompt` rule used to be
/// surfaced ONLY as advisory text in `nxc prime` — never re-checked by `ask`/`send`'s own
/// `resolve_declared_channel`, so a malformed declaration would silently degrade (an empty
/// synthesizer system prompt) instead of failing loudly, right here, before anything is even
/// opened.
#[test]
fn sending_to_a_declared_channel_with_a_malformed_summarize_declaration_is_rejected_with_nothing_persisted(
) {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob]\n  on_complete: summarize\n",
    ); // no summary_prompt

    let out = nxc(&tmp)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .failure();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["error"]["kind"], "validation", "{v}");
    assert!(
        v["error"]["msg"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("summary_prompt"),
        "{v}"
    );

    let store = open_store(&tmp);
    assert!(
        !store.channel_exists("decl:review").unwrap(),
        "a rejected malformed channel must never be materialized"
    );
}

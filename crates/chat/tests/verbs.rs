//! Black-box integration tests for `nxc`'s messaging verbs (spec §5.1): send / reply / search /
//! prime / the quorum boards, over the built binary. (`inbox`/`read` were in that list until nxf
//! 6j6v.1gm9 removed them, and `channels`/`agents` until 6j6v.dvyq §3.) Each test stands up its own `.nxs/` chat
//! workspace via the foundation setup (the `nxc init` that would do this is T3), so the verbs are
//! exercised exactly as an agent would call them.
//!
//! Determinism: `NXF_DETERMINISTIC_IDS=1` (sortable `m-<seq>` message/channel ids), `NXC_ACTOR=alice`,
//! `NXC_ORIGIN=local`, and a pinned `NXC_NOW`, so `--json` is byte-stable. The env is set on the CHILD
//! process only (`.env`), never process-global, so parallel tests do not race. `--json` objects built
//! with `serde_json::json!` emit keys ALPHABETICALLY (BTreeMap, no preserve_order); typed row structs
//! keep declaration order — the byte-exact assertions below reflect both.

use assert_cmd::Command;
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

/// A fresh chat workspace. Returns the temp dir (kept alive by the caller).
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
        .env("NXC_NOW", "2026-07-08T00:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

/// Assert `obj[key] == expected` AND that `key` is a genuinely PRESENT key in `obj` — not the same
/// check (PR review of nxf 6j6v.qk5b): `serde_json::Value`'s `Index` operator returns the identical
/// static `Value::Null` for a MISSING key as for a key present holding JSON `null` (its own
/// documented behaviour), so `assert_eq!(obj[key], Value::Null)` alone cannot tell "an additive
/// field that always renders" from "a field silently dropped by a `skip_serializing_if`
/// regression". Use this instead of a bare `assert_eq!` on any field whose ORDINARY value is
/// `null` — verified to actually discriminate: this assertion goes red under a temporary
/// `skip_serializing_if` on `working_tree` while a bare `assert_eq!(.., Value::Null)` stays green.
fn assert_present(obj: &Value, key: &str, expected: Value) {
    assert!(
        obj.as_object()
            .unwrap_or_else(|| panic!("{obj} is not a JSON object"))
            .contains_key(key),
        "expected key {key:?} to be present (even when its value is null) in {obj}"
    );
    assert_eq!(obj[key], expected, "value at key {key:?} in {obj}");
}

/// A group channel with `local/alice` in it, and its minted id — the fixture `nxc channels create`
/// used to build.
///
/// **Written to the store, not typed at the CLI**, since 6j6v.dvyq §3: a channel is a DECLARATION
/// now and there is no verb that mints one. The reads this file exercises — `inbox`, `read`,
/// `search`, the quorum boards — are membership-scoped and still need a channel with members and
/// traffic in it, so the fixture writes exactly the fields and membership ops the verb wrote.
fn create_channel(tmp: &TempDir, name: &str) -> String {
    let mut store = open_store(tmp);
    let cid = store.mint_channel_id();
    store.set_channel_field(&cid, "name", name, CALLER);
    store.set_channel_field(&cid, "kind", "group", CALLER);
    store.set_channel_field(&cid, "origin", "local", CALLER);
    store.add_member(&cid, CALLER, CALLER);
    cid
}

/// Post into `channel` as `actor` — the library half of the `nxc send <channel> <body>` this file
/// used to run. That entrance went with the raw channels (6j6v.dvyq §3); the WRITE it delegated to
/// did not, and a message in a channel is what most of these tests need to have happened, not
/// something they are testing.
fn post(tmp: &TempDir, channel: &str, actor: &str, body: &str, disposition: Disposition) -> String {
    post_from(tmp, channel, actor, body, disposition, None)
}

/// [`post`] carrying a RETURN ADDRESS — what `NXC_SESSION` stamped into `refs.session_id` when the
/// same fixture was a `send`. Several tests below are about what `reply` does with that address, so
/// it has to be settable without the verb that used to set it.
fn post_from(
    tmp: &TempDir,
    channel: &str,
    actor: &str,
    body: &str,
    disposition: Disposition,
    session_id: Option<&str>,
) -> String {
    let mut store = open_store(tmp);
    store.set_wall_clock(NOW);
    store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: channel.into(),
        sender: format!("local/{actor}"),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition,
        thread_id: None,
        refs: Refs {
            session_id: session_id.map(str::to_string),
            ..Default::default()
        },
        body: body.into(),
    })
}

/// Open a thread on an existing channel and hand back `(thread_id, message_id)`.
///
/// Through the library for the same reason [`post`] is: `send --to <raw-channel>` was how this used
/// to seed one, and a channel no declaration names stopped being a target in 6j6v.dvyq §3.
fn open_thread(tmp: &TempDir, channel: &str, body: &str) -> (String, String) {
    let mut store = open_store(tmp);
    store.set_wall_clock(NOW);
    let thread_id = store.mint_thread_id();
    store.open_thread(
        &thread_id,
        &ThreadRoot {
            origin: "local".into(),
            channel_id: channel.into(),
            opener: CALLER.into(),
            created: NOW.into(),
            parent: None,
        },
        CALLER,
    );
    let message_id = store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: channel.into(),
        sender: CALLER.into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some(thread_id.clone()),
        refs: Refs::default(),
        body: body.into(),
    });
    (thread_id, message_id)
}

/// Open a quorum board on `channel` expecting `expect` — the fixture
/// `nxc ask <channel> <body> --expect <h>…` used to build, returning the receipt as the JSON that
/// verb printed.
///
/// **Through the library since 6j6v.dvyq §3**, which removes `ask`: with every channel declared,
/// who must answer is written IN the channel and a per-call `--expect` is a second answer to a
/// question the declaration already answers. What that leaves without a CLI door is the
/// MULTI-HANDLE board on a channel nothing declares — the shape most of the quorum tests below need
/// — and `facade::ask`, the write the verb delegated to, is untouched and still builds it. The
/// receipt is the same type the verb serialized, so the assertions below read unchanged.
fn open_board(
    tmp: &TempDir,
    channel: &str,
    body: &str,
    expect: &[&str],
    deadline: Option<&str>,
) -> Value {
    open_board_from(tmp, channel, body, expect, deadline, None)
}

/// [`open_board`] carrying the opener's RETURN ADDRESS — what an ambient `NXC_SESSION` stamped onto
/// the board's root message, and what the completion wake resumes.
fn open_board_from(
    tmp: &TempDir,
    channel: &str,
    body: &str,
    expect: &[&str],
    deadline: Option<&str>,
    session_id: Option<&str>,
) -> Value {
    let mut store = open_store(tmp);
    let expect: Vec<String> = expect.iter().map(|h| (*h).to_string()).collect();
    let receipt = nexus_chat::facade::ask(
        &mut store,
        nexus_chat::facade::AskRequest {
            now: NOW,
            origin: "local",
            actor: "alice",
            channel,
            body,
            expect: &expect,
            deadline,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs {
                session_id: session_id.map(str::to_string),
                ..Default::default()
            },
        },
    )
    .expect("the board opens");
    serde_json::to_value(receipt).expect("the receipt serializes")
}

/// [`open_thread`] whose ROOT MESSAGE carries a return address — what an ambient `NXC_SESSION`
/// stamped when the same fixture was a `send`, and what `reply` routes back to.
///
/// The tests below used to post a bare message, dig its id out of `search --json`, and reply to
/// THAT. `reply --thread` takes a thread id and only a thread id (its own preflight says so), and a
/// message id was the second address 6j6v.dvyq §3 removes — so the fixture is a conversation now,
/// and the return address rides its root exactly as it did.
fn open_thread_from(
    tmp: &TempDir,
    channel: &str,
    sender: &str,
    body: &str,
    session_id: &str,
) -> (String, String) {
    let mut store = open_store(tmp);
    store.set_wall_clock(NOW);
    let thread_id = store.mint_thread_id();
    store.open_thread(
        &thread_id,
        &ThreadRoot {
            origin: "local".into(),
            channel_id: channel.into(),
            opener: format!("local/{sender}"),
            created: NOW.into(),
            parent: None,
        },
        CALLER,
    );
    let message_id = store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: channel.into(),
        sender: format!("local/{sender}"),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some(thread_id.clone()),
        refs: Refs {
            session_id: Some(session_id.to_string()),
            ..Default::default()
        },
        body: body.into(),
    });
    (thread_id, message_id)
}

/// The handle every `nxc()` invocation in this file writes as.
const CALLER: &str = "local/alice";
const NOW: &str = "2026-07-08T00:00:00Z";

// ---- send ------------------------------------------------------------------

/// `send` has no positional channel and no plain-post path any more (6j6v.dvyq §3), so the two
/// tests that stood here — `send_returns_a_synchronous_persist_ack` (post to a minted channel) and
/// `send_to_an_unknown_channel_is_not_found` — describe a verb that no longer exists. What replaces
/// them is the one thing `send` now insists on: a TARGET, named by a flag, resolved against the
/// declarations.
///
/// The receipts of the surviving shapes are pinned where those shapes live: `send --to <persona>`
/// and `send --to <channel>` in `surface_cli.rs`/`channel_fanout.rs`, and the three rejections (no
/// declarations at all, no such target, a channel no declaration names) in `declaration_source.rs`.
#[test]
fn send_without_a_target_flag_is_refused_and_names_the_one_that_is_a_target() {
    // `--to` is REQUIRED in clap since block (b) of 6j6v.dvyq §3 left it the only target flag, so
    // this refusal is clap's and it names the missing flag. It used to be hand-written and list
    // three flags, because while `--role` and `--session` also named a target none of the three was
    // individually mandatory and clap could only say "the following required arguments were not
    // provided" — which told a caller nothing about what a target IS. With one flag left, naming it
    // is the whole answer.
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "send", "ship it"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stdout).into_owned()
        + &String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        err.contains("--to"),
        "the refusal names the target flag: {err}"
    );
    assert!(
        !err.contains("--role") && !err.contains("--session"),
        "and does not offer two flags that no longer exist: {err}"
    );
}

/// The two other ways to name a target are gone with block (b) of 6j6v.dvyq §3, and clap is what
/// says so. Pinned as its own case rather than folded into the refusal above: an unknown flag and a
/// missing required flag are different failures, and a `send --role` that started being read as
/// something else — an abbreviation, a future flag — would pass the test above untouched.
#[test]
fn send_no_longer_takes_role_or_session() {
    let tmp = workspace();
    for args in [
        vec!["send", "hello", "--role", "coding"],
        vec!["send", "hello", "--session", "s-pm"],
    ] {
        let out = nxc(&tmp).args(&args).assert().failure();
        let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
        assert!(
            err.contains("unexpected argument"),
            "{args:?} is refused by clap as unknown, not resolved: {err}"
        );
    }
}

/// The positional channel is gone, and clap is what says so — a second positional has no argument
/// to land in.
#[test]
fn send_no_longer_takes_a_channel_positionally() {
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    let out = nxc(&tmp).args(["send", &cid, "ship it"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        err.contains("unexpected argument") || err.contains("--to"),
        "{err}"
    );
}

// ---- channels: the verb group is gone, the READS are on the seam --------------------------------
//
// Four tests stood here — `create_a_group_channel_then_list_it`,
// `dm_is_deterministic_and_exactly_two_members`, `join_and_leave_toggle_membership` and
// `channels_list_flags_a_degraded_dm`. Their SUBJECT was `nxc channels`, which 6j6v.dvyq §3 removes
// whole: a channel is a DECLARATION, its membership is that file's `members:`, and
// `ensure_declared_channel` materialises it on the first `send --to`.
//
// What they exercised is not all gone with the verb, and the difference is exactly §5's trap:
//
//   * the membership-scoped channel LIST and the degraded-DM flag stay on the seam as
//     `Engine::channels` — an app renders them — and are pinned in `tests/seam_reads.rs`, which is
//     where the reads that outlived their verb are held (owner decision of 2026-08-19: the parity
//     differential keeps its core, and a read with no CLI half keeps its own gate rather than
//     losing coverage);
//   * the DETERMINISTIC DM id and its exactly-two rule are `orchestration::ensure_dm_channel`'s,
//     reached now by `send --to <persona>`, and pinned there;
//   * membership toggling has no successor at all and needs none: nothing writes membership from
//     outside a declaration any more.

// ---- inbox + read ----------------------------------------------------------
//
// `inbox_partitions_by_disposition_and_read_advances_the_cursor` and
// `read_and_inbox_forbid_a_non_member_but_not_found_an_unknown_channel` stood here. Both went with
// their verbs (nxf 6j6v.1gm9) without losing their subject at the time — the disposition partition
// and the cursor advance were `facade::inbox`/`facade::mark_read`, which stayed on the seam, so the
// coverage MOVED to `tests/seam_reads.rs`, the file this repo keeps for exactly that shape ("a read
// that outlived its verb", set up by the `channels` removal of 6j6v.dvyq §3 two blocks above).
//
// **nxf 6j6v.4d2z has since removed the seam halves too**, and the moved tests went with them; that
// file's own note says why a removed BEHAVIOUR is not the case it exists to refuse. What is still
// asserted HERE about the pair is that the two entrances are refused
// (`consolidated_entrances.rs`).

// ---- reply -----------------------------------------------------------------

#[test]
fn reply_posts_into_the_targets_channel_carrying_the_thread() {
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    // Open a thread so `reply` has a conversation to answer into.
    let (thread, _qid) = open_thread(&tmp, &cid, "which db?");
    // The THREAD is the address. `reply <message-id>` was the other one and went with the
    // positional target (6j6v.dvyq §3): a message id names a point in a conversation, and answering
    // "the conversation that message is in" is what it always meant — `--thread` says so.
    let out = nxc(&tmp)
        .args(["--json", "reply", "--thread", &thread, "sqlite"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["posted"], true);
    assert_eq!(v["thread_id"], thread);
    // The reply lands in the same channel and is found by search there.
    let out = nxc(&tmp)
        .args(["--json", "search", "sqlite"])
        .assert()
        .success();
    let hits = json_of(&out.get_output().stdout);
    assert_eq!(hits[0]["channel_id"], cid);

    // Reply to an unknown target → not_found.
    let out = nxc(&tmp)
        .args(["--json", "reply", "--thread", "m-nope", "x"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
}

#[test]
fn reply_ambient_stamps_its_own_return_address() {
    // Fix round (6j6v.zenf final review, Fix 2): `reply` must ambient-stamp `refs.session_id`
    // from `NXC_SESSION`, mirroring `send`'s identical fallback (`send_role_mints_session_stamps_
    // return_address_and_triggers` below) — otherwise a multi-hop resume chain silently dies the
    // moment an intermediate `reply` forgets an explicit `--ref session_id=...`.
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    let (thread, _) = open_thread(&tmp, &cid, "which db?");
    nxc(&tmp)
        .env("NXC_SESSION", "s-coding")
        .args(["--json", "reply", "--thread", &thread, "sqlite"])
        .assert()
        .success();
    let store = open_store(&tmp);
    let refs_json: String = store
        .connection()
        .query_row("SELECT refs FROM messages WHERE body = 'sqlite'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let refs: Refs = serde_json::from_str(&refs_json).unwrap();
    assert_eq!(refs.session_id.as_deref(), Some("s-coding"));
}

// `reply_does_not_clobber_an_explicit_ref_session_id` stood here — "the ambient `NXC_SESSION` fills
// `refs.session_id` only when the caller did NOT pass one explicitly". REMOVED with `reply --ref`
// (nxf 6j6v.ckeq): a reply INHERITS its subject from the thread, so the refs obligation landed on
// `send --to` and the flag came off `reply` entirely. With no explicit value possible there is no
// precedence left to assert.
//
// `with_return_address`'s "only when absent" rule is untouched in `orchestration` and still has a
// caller — `send --to --ref session_id=…` — and the live half of THIS pair,
// `reply_ambient_stamps_its_own_return_address` above, is what keeps the chain-resume property
// covered: a reply stamps its own return address from the ambient session, which is the only way it
// is ever stamped now.

#[test]
fn reply_with_no_ambient_session_leaves_refs_session_id_unset() {
    // A bare-terminal `reply` (no `NXC_SESSION` at all) must leave `refs.session_id` as `None` —
    // mirrors `send_with_no_ambient_session_leaves_refs_session_id_unset` (review finding #2's
    // `session()` contract: no shared placeholder).
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    let (thread, _) = open_thread(&tmp, &cid, "which db?");
    nxc(&tmp)
        .args(["--json", "reply", "--thread", &thread, "no ambient session"])
        .assert()
        .success();
    let store = open_store(&tmp);
    let refs_json: String = store
        .connection()
        .query_row(
            "SELECT refs FROM messages WHERE body = 'no ambient session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let refs: Refs = serde_json::from_str(&refs_json).unwrap();
    assert_eq!(refs.session_id, None, "{refs_json}");
}

// ---- search ----------------------------------------------------------------

#[test]
fn search_finds_body_substrings_only_in_the_callers_channels() {
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    post(
        &tmp,
        &cid,
        "alice",
        "ship the RELEASE now",
        Disposition::InTurn,
    );
    post(
        &tmp,
        &cid,
        "alice",
        "unrelated chatter",
        Disposition::InTurn,
    );

    let out = nxc(&tmp)
        .args(["--json", "search", "release"])
        .assert()
        .success();
    let hits = json_of(&out.get_output().stdout);
    assert_eq!(
        hits.as_array().unwrap().len(),
        1,
        "case-insensitive substring over body"
    );
    assert_eq!(hits[0]["channel_id"], cid);
    assert_eq!(hits[0]["body"], "ship the RELEASE now");

    // A non-member sees nothing (channel scoping).
    let out = nxc(&tmp)
        .args([
            "--json",
            "search",
            "release",
            "--consumer",
            "local/outsider",
        ])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0
    );
}

#[test]
fn reply_to_a_thread_id_resolves_via_a_threaded_message() {
    // A thread is addressable only through the messages stamped with it; since 6j6v.dvyq §3 the
    // one way to stamp the first of those is `send --to`, which mints the id rather than taking
    // one. `reply <thread_id>` must still resolve it.
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    let (thread, _) = open_thread(&tmp, &cid, "root q");
    // Reply targeting the THREAD id (not a message id) resolves the channel via that message.
    let out = nxc(&tmp)
        .args(["--json", "reply", "--thread", &thread, "in this thread"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["posted"], true);
    assert_eq!(v["thread_id"], thread);
    // The reply landed in the thread's channel.
    let out = nxc(&tmp)
        .args(["--json", "search", "in this thread"])
        .assert()
        .success();
    assert_eq!(json_of(&out.get_output().stdout)[0]["channel_id"], cid);
}

#[test]
fn search_treats_a_like_underscore_literally() {
    // `_` is a LIKE single-char wildcard; the bound + ESCAPE query must match it literally (Test
    // #4). Taken over the MESSAGE search since 6j6v.dvyq §3 removed `agents search` — the escaping
    // is `ChatStore`'s, shared by both reads, and this is the one that still has a door.
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    post(&tmp, &cid, "alice", "abc", Disposition::InTurn);
    // "a_c" as a wildcard would match "abc"; escaped, it matches only a literal underscore → 0.
    let out = nxc(&tmp)
        .args(["--json", "search", "a_c"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0,
        "_ is literal, not a single-char wildcard"
    );
    // A message that literally contains "a_c" DOES match.
    post(&tmp, &cid, "alice", "x a_c y", Disposition::InTurn);
    let out = nxc(&tmp)
        .args(["--json", "search", "a_c"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        1,
        "a literal a_c matches"
    );
}

#[test]
fn a_separator_byte_is_a_validation_error_not_a_panic() {
    // A U+001F byte in an identifier that becomes a composite op id must be a structured
    // `validation` error (exit 1), NOT a raw store `assert!` panic (exit 101) — the CLI's
    // no-raw-panic contract (Integrity #1, reproduced against the real binary).
    //
    // THE REACHABLE PATH HAS MOVED TWICE, and each move is why this test still exists rather than
    // having quietly lost its subject. `channels create`/`join` — whose auto-join and handle
    // argument both became membership ids — went with the raw channels (6j6v.dvyq §3); the READ
    // CURSOR's composite id, typed as `nxc read`, went with 6j6v.1gm9. What is left is the one
    // write that still turns the CALLER'S OWN handle into a membership id:
    // `orchestration::ensure_dm_channel`, reached by `send --to <persona>`, which joins both sides
    // of the DM it materialises.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: c\n",
    )
    .unwrap();
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_ACTOR", "evil\u{1f}actor")
        .args(["--json", "send", "--to", "coder", "hi", "--no-ref"])
        .assert()
        .failure();
    let err = json_of(&out.get_output().stdout);
    assert_eq!(err["error"]["kind"], "validation");
    assert_eq!(
        err["error"]["msg"], "handle must not contain the U+001F (unit separator) byte",
        "and it is the SEP guard that refused, not some other validation on the way: {err}"
    );
    assert_eq!(
        out.get_output().status.code(),
        Some(1),
        "structured validation error, not a panic (101)"
    );
    // And nothing was written on the way out: the guard runs before the membership op. Asked of the
    // `messages` table directly rather than through `search_messages`, which since nxf 6j6v.cs03
    // takes a channel SCOPE — and the claim here is about the whole workspace: no message at all,
    // in any channel this call might have minted.
    let written: i64 = open_store(&tmp)
        .connection()
        .query_row("SELECT COUNT(*) FROM messages WHERE body = 'hi'", [], |r| {
            r.get(0)
        })
        .expect("count the messages");
    assert_eq!(written, 0, "the refusal is before the write, not after it");
}

// ---- byte-stable --json contract -------------------------------------------

#[test]
fn json_output_is_byte_stable_under_the_determinism_switch() {
    // The three writes this pinned — `channels create`, a plain `send <channel> <body>` and
    // `channels dm` — all went with 6j6v.dvyq §3. The PROPERTY is the same and belongs on the write
    // that survives: under `NXF_DETERMINISTIC_IDS` the ids are counter-derived, and the receipt is
    // a typed struct, so it renders in DECLARATION order rather than the alphabetical order a
    // hand-built `serde_json::json!` object (a `BTreeMap` here) would.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("bob.yaml"),
        "handle: bob\njob_title: Reviewer\nsystem_prompt: r\n",
    )
    .unwrap();
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .args(["--json", "send", "--to", "bob", "ship it"])
        .assert()
        .success();
    let rendered = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let v: Value = serde_json::from_str(rendered.trim()).expect("valid json");
    assert_eq!(v["thread_id"], "m-00000000000000000000000001");
    assert_eq!(v["message_id"], "m-00000000000000000000000001");
    assert_eq!(v["target"], "persona");
    // Declaration order, not alphabetical: `thread_id` really is rendered before `message_id`.
    let (t, m) = (
        rendered.find("\"thread_id\"").expect("thread_id present"),
        rendered.find("\"message_id\"").expect("message_id present"),
    );
    assert!(t < m, "typed receipt keeps declaration order: {rendered}");
}

// ---- ask + threads (M2) ----------------------------------------------------

#[test]
fn ask_opens_a_board_shown_by_threads_list_and_show() {
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    // alice (the creator, auto-joined) opens a board expecting bob + carol.
    let out = open_board(
        &tmp,
        &cid,
        "please review PR 42",
        &["local/bob", "local/carol"],
        None,
    );
    let v = out.clone();
    assert_eq!(v["persisted"], true);
    let thread = v["thread_id"].as_str().unwrap().to_string();
    assert!(thread.starts_with("m-"), "minted thread ulid");
    assert!(v["message_id"].as_str().unwrap().starts_with("m-"));
    assert_eq!(
        v["expects"],
        serde_json::json!(["local/bob", "local/carol"])
    );
    assert!(v.get("deadline").is_none(), "no deadline → key omitted");

    // threads list shows the board for the opener (a member), both handles outstanding.
    let out = nxc(&tmp)
        .args(["--json", "threads", "list"])
        .assert()
        .success();
    let list = json_of(&out.get_output().stdout);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["thread_id"], thread.as_str());
    assert_eq!(list[0]["channel_id"], cid.as_str());
    assert_eq!(list[0]["opener"], "local/alice");
    assert_eq!(
        list[0]["expects"],
        serde_json::json!(["local/bob", "local/carol"])
    );
    assert_eq!(
        list[0]["outstanding"],
        serde_json::json!(["local/bob", "local/carol"])
    );
    assert_eq!(list[0]["complete"], false);
    assert_eq!(list[0]["stale"], false);

    // threads show renders the board plus the request message.
    let out = nxc(&tmp)
        .args(["--json", "threads", "show", &thread])
        .assert()
        .success();
    let show = json_of(&out.get_output().stdout);
    assert_eq!(show["thread_id"], thread.as_str());
    assert_eq!(show["complete"], false);
    assert_eq!(show["messages"].as_array().unwrap().len(), 1);
    assert_eq!(show["messages"][0]["body"], "please review PR 42");
}

#[test]
fn replies_from_expected_handles_complete_the_board() {
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    let out = open_board(&tmp, &cid, "review", &["local/bob", "local/carol"], None);
    let thread = out.clone()["thread_id"].as_str().unwrap().to_string();
    // bob replies (the reply's sender is the acting handle) — still incomplete (carol outstanding).
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .args(["reply", "--thread", &thread, "lgtm"])
        .assert()
        .success();
    let show = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "show", &thread])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(show["replied"], serde_json::json!(["local/bob"]));
    assert_eq!(show["outstanding"], serde_json::json!(["local/carol"]));
    assert_eq!(show["complete"], false);
    // carol replies → the board completes.
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .args(["reply", "--thread", &thread, "approved"])
        .assert()
        .success();
    let show = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "show", &thread])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(show["complete"], true);
    assert_eq!(show["outstanding"].as_array().unwrap().len(), 0);
    assert_eq!(
        show["messages"].as_array().unwrap().len(),
        3,
        "request + two replies"
    );
}

#[test]
fn threads_show_is_unfiltered_for_a_non_opener_consumer_too() {
    // Regression guard (6j6v.xrk3): `thread_board` gained a `visibility` parameter, and
    // `threads_show` now passes `Visibility::AllMembers` at that call site (no declared-channel
    // resolution exists yet to look up a real policy — see the comment at the call site, 6j6v.cahk).
    // `nxc threads show`'s behavior must stay EXACTLY what it was: every member — opener or not —
    // sees every reply.
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    // Being `--expect`ed to reply is independent of channel membership — `threads show` (via
    // `thread_board`) membership-gates, so bob and carol must actually join the channel too.
    open_store(&tmp).add_member(&cid, "local/bob", CALLER);
    open_store(&tmp).add_member(&cid, "local/carol", CALLER);
    let out = open_board(&tmp, &cid, "review", &["local/bob", "local/carol"], None);
    let thread = out.clone()["thread_id"].as_str().unwrap().to_string();
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .args(["reply", "--thread", &thread, "bob's reply"])
        .assert()
        .success();
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .args(["reply", "--thread", &thread, "carol's reply"])
        .assert()
        .success();
    // alice (the opener) is the channel creator; bob is a member but NOT the opener.
    let show = json_of(
        &nxc(&tmp)
            .args([
                "--json",
                "threads",
                "show",
                &thread,
                "--consumer",
                "local/bob",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let bodies: Vec<&str> = show["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert_eq!(
        bodies,
        vec!["review", "bob's reply", "carol's reply"],
        "a non-opener consumer still sees every reply — AllMembers, unchanged"
    );
}

// `threads_expect_is_opener_only_and_unsticks_a_board` was here. The VERB it drove
// (`nxc threads expect`) was removed by 6j6v.dvyq §3 — the owner's own reason, and the
// consolidation's rule that a decided removal is pulled forward rather than compromised around when
// it collides with a live change (here 6j6v.cg8g's turn-scoped discharge, which made a narrowing a
// re-declaration rather than a completion). What the verb tested about the SEAM — opener-only,
// `not_found` for an unknown thread, and the re-derived board it hands back — is unchanged and
// still pinned at `facade::set_expects` itself: `facade::tests::set_expects_is_opener_only_and_
// re_derives_the_board` and `actor_contract.rs`'s boundary table.

#[test]
fn threads_list_and_show_report_working_tree_holding_waiting_and_the_ordinary_case() {
    // Definition of done (nxf 6j6v.qk5b): three threads — one holds the working-tree lease, one
    // waits behind it at position 1, one needs it not at all — and `threads list --json` shows
    // exactly that, additively (`working_tree`/`working_tree_queue_position` present on EVERY
    // entry, `null` for the ordinary case, never an omitted key). The lease/queue are seeded
    // directly against the store (the mechanics ticket 3/4 already own and pin) — this ticket is
    // pure visibility, derived from them, never stored on the thread itself.
    let tmp = workspace();
    let cid = create_channel(&tmp, "review"); // alice (NXC_ACTOR) auto-joins as creator
    let mut store = open_store(&tmp);
    let open = |store: &mut ChatStore, id: &str| {
        store.open_thread(
            id,
            &ThreadRoot {
                origin: "local".into(),
                channel_id: cid.clone(),
                opener: "local/alice".into(),
                created: "2026-07-08T00:00:00Z".into(),
                parent: None,
            },
            "local/alice",
        );
    };
    open(&mut store, "m-th-holding");
    open(&mut store, "m-th-waiting");
    open(&mut store, "m-th-neither");

    let now = "2026-07-08T00:00:00Z";
    assert!(store
        .acquire_working_tree(
            &nexus_chat::working_tree::WorkScope::Thread("m-th-holding".into()),
            now,
            "2026-07-08T02:00:00Z",
        )
        .unwrap());
    store
        .enqueue_working_tree(
            &nexus_chat::working_tree::QueuedTrigger {
                id: 0,
                scope_key: nexus_chat::working_tree::WorkScope::Thread("m-th-waiting".into()).key(),
                role: "bob".into(),
                session: "s-1".into(),
                thread: Some("m-th-waiting".into()),
                message: "review it".into(),
                model: None,
                depth: 0,
                priority: Priority::Normal,
                enqueued_at: None,
            },
            now,
        )
        .unwrap();
    drop(store);

    let list = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "list"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let entries = list.as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let entry = |id: &str| {
        entries
            .iter()
            .find(|e| e["thread_id"] == id)
            .unwrap_or_else(|| panic!("no entry for {id} in {entries:?}"))
    };
    assert_present(
        entry("m-th-holding"),
        "working_tree",
        serde_json::json!("holding"),
    );
    assert_present(
        entry("m-th-holding"),
        "working_tree_queue_position",
        Value::Null,
    );
    assert_present(
        entry("m-th-waiting"),
        "working_tree",
        serde_json::json!("waiting"),
    );
    assert_present(
        entry("m-th-waiting"),
        "working_tree_queue_position",
        serde_json::json!(1),
    );
    assert_present(entry("m-th-neither"), "working_tree", Value::Null);
    assert_present(
        entry("m-th-neither"),
        "working_tree_queue_position",
        Value::Null,
    );

    // `threads show` derives the identical answer for each thread individually.
    let show = |id: &str| {
        json_of(
            &nxc(&tmp)
                .args(["--json", "threads", "show", id])
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    assert_present(
        &show("m-th-holding"),
        "working_tree",
        serde_json::json!("holding"),
    );
    assert_present(
        &show("m-th-waiting"),
        "working_tree",
        serde_json::json!("waiting"),
    );
    assert_present(
        &show("m-th-waiting"),
        "working_tree_queue_position",
        serde_json::json!(1),
    );
    assert_present(&show("m-th-neither"), "working_tree", Value::Null);
    assert_present(
        &show("m-th-neither"),
        "working_tree_queue_position",
        Value::Null,
    );
}

#[test]
fn a_declared_timeout_resolves_a_duration_and_an_instant_alike_and_rejects_bad_input() {
    // Was `send_to_resolves_a_duration_deadline_and_rejects_bad_input`, driving `send --deadline`,
    // which nxf 6j6v.ckeq removed: a board's window is the CHANNEL's own answer, not a per-call
    // one. The GRAMMAR it proved is untouched and so is this test's job — the flag's own arrival
    // note claimed "a DURATION has a declared form (a channel's `timeout:`) but an ABSOLUTE INSTANT
    // has none", and that was already untrue: `timeout:` is parsed by the very same
    // `facade::resolve_deadline_spec`, which tries RFC3339 first. So all three cases are still
    // expressible, at the declaration, and this is the construction proof for THAT.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("bob.yaml"),
        "handle: bob\njob_title: Reviewer\nsystem_prompt: r\n",
    )
    .unwrap();
    let dry = tmp.path().join("dry.log");
    let send = |name: &str, timeout: &str, body: &str| {
        std::fs::write(
            tmp.path().join(".nxs-personas/channels.yaml"),
            format!("- name: {name}\n  members: [alice, bob]\n  timeout: {timeout}\n"),
        )
        .unwrap();
        let mut c = nxc(&tmp);
        c.env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", &dry)
            .args(["--json", "send", "--to", name, body, "--no-ref"]);
        c
    };

    // NXC_NOW is pinned to 2026-07-08T00:00:00Z, so a 24h duration resolves to the next day.
    let out = send("review", "24h", "review").assert().success();
    assert_eq!(
        json_of(&out.get_output().stdout)["deadline"],
        "2026-07-09T00:00:00Z"
    );
    // An absolute RFC3339 instant is kept verbatim.
    let out = send("review2", "2026-07-20T00:00:00Z", "review2")
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout)["deadline"],
        "2026-07-20T00:00:00Z"
    );
    // Neither RFC3339 nor a `<n><unit>` duration → validation, refused before anything persists.
    let out = send("review3", "whenever", "review3").assert().failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );
}

#[test]
fn threads_show_forbids_a_non_member_and_not_founds_unknown() {
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");
    let out = open_board(&tmp, &cid, "review", &["local/bob"], None);
    let thread = out.clone()["thread_id"].as_str().unwrap().to_string();
    // a non-member consumer → forbidden.
    let out = nxc(&tmp)
        .args([
            "--json",
            "threads",
            "show",
            &thread,
            "--consumer",
            "local/outsider",
        ])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "forbidden"
    );
    // an unknown thread → not_found.
    let out = nxc(&tmp)
        .args(["--json", "threads", "show", "m-nope"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
}

// `ask_json_is_byte_stable_under_the_determinism_switch` stood here and pinned the exact bytes
// `nxc ask --json` printed. The verb is gone (6j6v.dvyq §3) and `facade::ask` — the write it
// delegated to — has no CLI surface of its own to be byte-stable ON.
// `json_output_is_byte_stable_under_the_determinism_switch` above carries the property on the write
// that survives.

/// Mirror of the CLI's `dm_channel_id` so the test can assert the exact id (kept in-test to avoid
/// exporting the helper).
fn dm_id(a: &str, b: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut pair = [a, b];
    pair.sort_unstable();
    let mut h = Sha256::new();
    h.update(pair[0].as_bytes());
    h.update([0u8]);
    h.update(pair[1].as_bytes());
    let hex = h
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("dm:{}", &hex[..24])
}

// ---- hj12 + requester-wake (spec §4.4/§4.5/§5.2) ---------------------------

/// Open the workspace's chat store directly, to SEED thread/quorum state the T3 verb surface cannot
/// create yet (`ask` is T2, and re-declaring `expects_reply_from` never became a verb at all — its
/// CLI door went with 6j6v.dvyq). The `nxc` subprocess then reads the SAME db.
fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Post a `report` message from `sender` into thread `m-th` / channel `m-ch`; returns its id.
fn post_in(store: &mut ChatStore, sender: &str, body: &str) -> String {
    // Stamped like production (nxf 6j6v.2hx9): `facade::send`/`reply` set the wall clock before
    // every post, and `messages.created` is what the session-start window is measured against.
    store.set_wall_clock(NOW);
    store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: "m-ch".into(),
        sender: sender.into(),
        kind: MessageKind::Report,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some("m-th".into()),
        refs: Refs::default(),
        body: body.into(),
    })
}

/// Seed channel `m-ch` (alice + bob) and thread `m-th` opened by alice expecting bob, with alice's
/// request posted. The caller accrues replies to drive completion / staleness.
fn seed_review_board(store: &mut ChatStore) {
    store.set_channel_field("m-ch", "kind", "group", "local/alice");
    store.add_member("m-ch", "local/alice", "local/alice");
    store.add_member("m-ch", "local/bob", "local/alice");
    let root = ThreadRoot {
        origin: "local".into(),
        channel_id: "m-ch".into(),
        opener: "local/alice".into(),
        created: "2026-07-08T00:00:00Z".into(),
        parent: None,
    };
    store.open_thread("m-th", &root, "local/alice");
    store.set_expects_reply_from("m-th", "[\"local/bob\"]", "local/alice");
    post_in(store, "local/alice", "please review PR 42");
    a_previous_session_of(store, "alice");
}

/// A previous session of `role` that has ENDED — the watermark a session start's notice is a window
/// over (nxf 6j6v.2hx9). Without one there is no window and no notice at all, which is the right
/// answer for a caller that has never run here and the wrong fixture for a test about what the
/// notice CONTAINS. Ended before everything these fixtures date, so all of it lands inside.
fn a_previous_session_of(store: &mut ChatStore, role: &str) {
    let id = format!("s-{role}");
    store.create_pending_session(&id, role).expect("mint");
    store
        .mark_session_ended(&id, "2026-07-06T00:00:00Z")
        .expect("end it");
}

// `own_send_is_not_unread_to_self_but_is_to_other_members` stood here — hj12 (spec §4.5): a
// sender's own post never appeared in its OWN unread but did in another member's, read off
// `prime --json`'s catch-up fields, which were the one surface carrying the unread to a caller
// after nxf 6j6v.1gm9 removed `nxc inbox`. nxf 6j6v.4d2z removed the unread set itself, so the
// exclusion rule has nothing left to be a rule ABOUT: what a caller reads of a channel is its whole
// conversation, own posts included, which is what `nxc threads show` and `nxc status` have always
// shown.

#[test]
fn the_wake_reaches_a_caller_as_data_and_never_as_message_text() {
    // §4.4/§5.2, as nxf 6j6v.1gm9 left it and nxf 6j6v.2hx9 re-derived it. When the sole expected
    // reviewer replies, the opener's `threads_you_opened.complete` carries the board with its
    // replies and the completing annotation — inside the caller's own WINDOW, which is what the
    // read cursor used to gate and no longer does.
    //
    // What went is the RENDERING. This test used to assert a `## Threads you opened` block in the
    // human `prime`, and it now asserts the opposite: not one byte of the conversation reaches the
    // block. That block was measured at 46.487 of a 63.787-byte composed session start, replaying
    // finished commissions in full to every session that started afterwards, while all three
    // delivery paths had already pushed the same text to whoever it was for.
    let tmp = workspace();
    let completing = {
        let mut store = open_store(&tmp);
        seed_review_board(&mut store);
        post_in(&mut store, "local/bob", "approved") // bob is the sole reviewer → completes the board
    };

    // prime (human): nothing of the board, and no heading it could hide under.
    let out = nxc(&tmp).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for gone in [
        "Threads you opened",
        "Complete (1)",
        "completes `m-th`",
        "approved",
        "please review PR 42",
    ] {
        assert!(
            !text.contains(gone),
            "the block still carries {gone:?}: {text}"
        );
    }

    // prime (json): threads_you_opened.complete carries the board, unchanged.
    let out = nxc(&tmp).args(["--json", "prime"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    let complete = v["threads_you_opened"]["complete"].as_array().unwrap();
    assert_eq!(complete.len(), 1);
    assert_eq!(complete[0]["thread_id"], "m-th");
    assert_eq!(complete[0]["completing_message_id"], completing);
    assert_eq!(complete[0]["expects"], serde_json::json!(["local/bob"]));

    // **And nothing a caller can do takes it off** (nxf 6j6v.2hx9). An ack past the completing
    // reply stood here, asserting the board STAYED; before 2hx9 the same call was asserted to clear
    // it, and that clearing rule was the defect — nothing ever acked, so the notice never cleared
    // and a session start accumulated finished work without bound. What takes a board off the list
    // is the WINDOW moving past it, which is a caller's own session ending. The ack itself is gone
    // since nxf 6j6v.4d2z, so the assertion has no call left to make.
    //
    // Asked twice instead, which is the property that outlived it: a session start is not
    // self-consuming — reading it does not change it.
    let out = nxc(&tmp).args(["--json", "prime"]).assert().success();
    assert_eq!(
        json_of(&out.get_output().stdout)["threads_you_opened"]["complete"]
            .as_array()
            .expect("still there")
            .len(),
        1,
        "a session start is a window, not a queue that empties as it is read"
    );
}

#[test]
fn a_stale_board_past_its_deadline_is_data_too_and_not_a_line_in_the_block() {
    // §4.3/§5.2: a board past its deadline with a reviewer still outstanding is `stale` — advisory,
    // not cursor-gated. NXC_NOW is 2026-07-08; the deadline is before.
    //
    // It rendered as a "Waiting past deadline" line until nxf 6j6v.1gm9, and that line went with
    // the section rather than surviving it. The reason is the ticket's own dividing line: what a
    // session must be told at its start is a thread THIS session owes an answer on, and a stale
    // board is the opposite — somebody else owes the caller. The caller learns it by asking
    // (`nxc status`, `nxc threads show`), which is the human surface this ticket left untouched.
    let tmp = workspace();
    {
        let mut store = open_store(&tmp);
        store.set_channel_field("m-ch", "kind", "group", "local/alice");
        store.add_member("m-ch", "local/alice", "local/alice");
        let root = ThreadRoot {
            origin: "local".into(),
            channel_id: "m-ch".into(),
            opener: "local/alice".into(),
            created: "2026-07-06T00:00:00Z".into(),
            parent: None,
        };
        store.open_thread("m-th", &root, "local/alice");
        store.set_expects_reply_from("m-th", "[\"local/bob\"]", "local/alice");
        store.set_deadline("m-th", "2026-07-07T00:00:00Z", "local/alice"); // before NXC_NOW → stale
        post_in(&mut store, "local/alice", "please review by tomorrow");
        a_previous_session_of(&mut store, "alice");
    }

    let out = nxc(&tmp).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for gone in ["Waiting past deadline", "still waiting on", "local/bob"] {
        assert!(
            !text.contains(gone),
            "the block still carries {gone:?}: {text}"
        );
    }

    let out = nxc(&tmp).args(["--json", "prime"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    let stale = v["threads_you_opened"]["stale"].as_array().unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0]["thread_id"], "m-th");
    assert_eq!(stale[0]["outstanding"], serde_json::json!(["local/bob"]));
    assert_eq!(stale[0]["deadline"], "2026-07-07T00:00:00Z");
    assert!(
        v["threads_you_opened"]["complete"]
            .as_array()
            .unwrap()
            .is_empty(),
        "nothing completed"
    );
    // And the pull surface a human is pointed at instead does carry it.
    let out = nxc(&tmp)
        .args(["threads", "show", "m-th"])
        .assert()
        .success();
    let board = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        board.contains("outstanding: local/bob") && board.contains("please review by tomorrow"),
        "`nxc threads show` is where the caller asks: {board}"
    );
}

// ---- reply --if-unanswered (nxf 6j6v.ww0a) ---------------------------------

#[test]
fn reply_if_unanswered_posts_once_then_is_a_silent_no_op() {
    // Ticket 7's whole point: the sidecar's teardown call (ticket 8) is UNCONDITIONAL — it does not
    // know whether the session already answered — so the enforcement has to live here, not in JS.
    // `coder` still owes a reply on `m-th` (declared via `expects_reply_from`, exactly what ticket 5
    // stamps on a role trigger): the first call posts; the repeat call — the sidecar's normal case,
    // not an error — posts NOTHING and still exits 0.
    let tmp = workspace();
    {
        let mut store = open_store(&tmp);
        store.set_channel_field("m-ch", "kind", "group", "local/alice");
        store.add_member("m-ch", "local/alice", "local/alice");
        store.add_member("m-ch", "local/coder", "local/alice");
        let root = ThreadRoot {
            origin: "local".into(),
            channel_id: "m-ch".into(),
            opener: "local/alice".into(),
            created: "2026-07-08T00:00:00Z".into(),
            parent: None,
        };
        store.open_thread("m-th", &root, "local/alice");
        store.set_expects_reply_from("m-th", "[\"local/coder\"]", "local/alice");
        post_in(&mut store, "local/alice", "please implement the thing");
    }

    // 1st call: coder is expected and has not replied yet → posts, `posted: true`.
    let out = nxc(&tmp)
        .env("NXC_ACTOR", "coder")
        .args([
            "--json",
            "reply",
            "--thread",
            "m-th",
            "--if-unanswered",
            "done",
        ])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["posted"], true);
    let message_id = v["message_id"]
        .as_str()
        .expect("a posted reply carries its message id")
        .to_string();
    assert!(message_id.starts_with("m-"));
    assert_eq!(v["thread_id"], "m-th");

    // 2nd call: coder already replied → a deliberate no-op, `posted: false`, no `message_id`, and
    // still EXIT 0 — the repeat case is success, not a failure.
    let out = nxc(&tmp)
        .env("NXC_ACTOR", "coder")
        .args([
            "--json",
            "reply",
            "--thread",
            "m-th",
            "--if-unanswered",
            "done again",
        ])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["posted"], false, "coder already answered this thread");
    assert!(
        v.get("message_id").is_none(),
        "nothing was posted, so there is no message id: {v}"
    );

    // Exactly one reply from coder actually landed — the repeat call truly wrote nothing.
    let store = open_store(&tmp);
    let coder_replies = store
        .messages_in_thread("m-th")
        .unwrap()
        .into_iter()
        .filter(|m| m.sender == "local/coder")
        .count();
    assert_eq!(
        coder_replies, 1,
        "the repeat --if-unanswered call must not have posted a second time"
    );
}

#[test]
fn reply_without_if_unanswered_posts_unconditionally_as_before() {
    // The flag is opt-in: a plain `reply` into the SAME already-answered thread still posts, exactly
    // as it always has — `--if-unanswered` changes nothing about the default verb. `posted` itself IS
    // rendered either way (review finding 4: it is a routine, always-meaningful fact like `resumed`,
    // not an additive finding like `wake_skipped`) — unconditionally `true` here, since nothing ever
    // gated the write.
    let tmp = workspace();
    {
        let mut store = open_store(&tmp);
        store.set_channel_field("m-ch", "kind", "group", "local/alice");
        store.add_member("m-ch", "local/alice", "local/alice");
        store.add_member("m-ch", "local/coder", "local/alice");
        let root = ThreadRoot {
            origin: "local".into(),
            channel_id: "m-ch".into(),
            opener: "local/alice".into(),
            created: "2026-07-08T00:00:00Z".into(),
            parent: None,
        };
        store.open_thread("m-th", &root, "local/alice");
        store.set_expects_reply_from("m-th", "[\"local/coder\"]", "local/alice");
        post_in(&mut store, "local/alice", "please implement the thing");
    }

    for body in ["first", "second"] {
        let out = nxc(&tmp)
            .env("NXC_ACTOR", "coder")
            .args(["--json", "reply", "--thread", "m-th", body])
            .assert()
            .success();
        let v = json_of(&out.get_output().stdout);
        assert_eq!(
            v["posted"], true,
            "unconditional post, unconditional posted: {v}"
        );
        assert!(v["message_id"].as_str().unwrap().starts_with("m-"));
    }

    let store = open_store(&tmp);
    let coder_replies = store
        .messages_in_thread("m-th")
        .unwrap()
        .into_iter()
        .filter(|m| m.sender == "local/coder")
        .count();
    assert_eq!(coder_replies, 2, "both ordinary replies land");
}

// ---- session bind (nxf epic 6j6v.zenf, T5) ---------------------------------

#[test]
fn session_bind_fills_in_the_real_sdk_session_id() {
    // Mint the pending session Rust-side (T4 seeds; the verb that mints one for real is
    // `send --to <persona>`), then bind it through the `nxc` CLI verb T1's sidecar shells out to.
    let tmp = workspace();
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-1", "coding").unwrap();
    }
    let out = nxc(&tmp)
        .args(["--json", "session", "bind", "s-1", "real-abc"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["internal"], "s-1");
    assert_eq!(v["bound"], true);

    // The bind is durable — a fresh store handle over the same db sees it.
    let store = open_store(&tmp);
    assert_eq!(
        store.resolve_real("s-1").unwrap().as_deref(),
        Some("real-abc")
    );
}

#[test]
fn session_bind_of_an_unknown_internal_id_is_not_found() {
    // A fat-fingered/never-minted internal id must not silently "succeed" — the sidecar's
    // narrow try/catch (T1) depends on this surfacing as a clean error, not a panic.
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "session", "bind", "s-ghost", "real-x"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
}

// ---- summoning a persona (nxf epic 6j6v.zenf, T6; the entrance since 6j6v.dvyq §3) -----------
//
// The entrance was `send --role <handle>`; block (b) of §3 removed it, and `send --to <persona>`
// is the one that remains. It runs the same `orchestration::coordinator_commission` body — so every
// case below asserts what it always asserted — and additionally opens a THREAD and tells the
// persona which one to answer into, which is why the collapse went this way round rather than the
// other: a threadless trigger left its answers unaddressable by `reply --thread`.

#[test]
fn send_to_persona_mints_session_stamps_return_address_and_triggers() {
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();
    // NOTE: a channel's minted `m-…` id is independent of the display `name`
    // (`store.mint_channel_id()`, unrelated to `"work"`), so every test in this file resolves it
    // through `create_channel`'s returned id rather than the literal name. That mattered when the
    // name could be typed at a channel-taking verb; since 6j6v.dvyq §3 no verb takes one, and the
    // fixture is a store write.
    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-caller")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["send", "hello", "--to", "coding", "--json"])
        .assert()
        .success();
    let so = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(so.contains("\"session\":\"m-"), "{so}");
    assert!(std::fs::read_to_string(&dry)
        .unwrap()
        .contains("trigger role=coding"));

    // The ambient return address (spec §4.1/§4.3): the caller's NXC_SESSION must land in the
    // message's refs.session_id. NEITHER `search` nor `inbox`'s `--json` surfaces `refs` today
    // (`agent-sidecar/README.md`'s "Task 2 smoke" finding — `MessageHit`/`MessageHitView`/`InboxOut`
    // all omit it, and extending that is out of this task's scope), so verify directly against the
    // store, mirroring `ChatStore::message_channel`'s direct-column-read precedent.
    let store = open_store(&tmp);
    let refs_json: String = store
        .connection()
        .query_row("SELECT refs FROM messages WHERE body = 'hello'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let refs: Refs = serde_json::from_str(&refs_json).unwrap();
    assert_eq!(refs.session_id.as_deref(), Some("s-caller"));
}

#[test]
fn an_explicit_ref_session_id_is_what_the_envelope_carries() {
    // A self-review guard: the ambient `NXC_SESSION` fills `refs.session_id` only when the caller
    // did NOT pass one explicitly — an explicit `--ref session_id=…` wins.
    //
    // Named for what it pins rather than for a verb: the fixture became a library write when
    // 6j6v.dvyq §3 took the raw-channel post ([`post_from`]'s own note), so the CLI flag this was
    // called after (`send --role`) is two removals behind the body.
    let tmp = workspace();
    let cid = create_channel(&tmp, "work");
    post_from(
        &tmp,
        &cid,
        "alice",
        "explicit wins",
        Disposition::InTurn,
        Some("s-explicit"),
    );
    let store = open_store(&tmp);
    let refs_json: String = store
        .connection()
        .query_row(
            "SELECT refs FROM messages WHERE body = 'explicit wins'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let refs: Refs = serde_json::from_str(&refs_json).unwrap();
    assert_eq!(refs.session_id.as_deref(), Some("s-explicit"));
}

#[test]
fn send_with_no_ambient_session_leaves_refs_session_id_unset() {
    // Review finding #2: a bare-terminal kickoff (no `NXC_SESSION` at all, e.g. `nxc(&tmp)` here
    // never sets it) must leave `refs.session_id` as `None` — NOT the old fixed placeholder
    // `"local"` — since there is no real ambient session to record as a return address, and a
    // shared sentinel string would be a false signal later mistaken for a genuine session identity.
    let tmp = workspace();
    let cid = create_channel(&tmp, "work");
    post(
        &tmp,
        &cid,
        "alice",
        "no ambient session",
        Disposition::InTurn,
    );
    let store = open_store(&tmp);
    let refs_json: String = store
        .connection()
        .query_row(
            "SELECT refs FROM messages WHERE body = 'no ambient session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let refs: Refs = serde_json::from_str(&refs_json).unwrap();
    assert_eq!(refs.session_id, None, "{refs_json}");
}

// `send_without_role_mints_no_session_and_triggers_no_worker` stood here — "a plain `send` with no
// `--role` posts and summons nobody". REMOVED: there is no such shape left. `send` has one target
// flag and it is required, so every `send` reaches a declared persona or a declared channel and
// every one of them summons somebody. The case had already been hollowed out by block (a), which
// turned its fixture into a library write ([`post_from`]) and left it asserting that a store write
// starts no worker; what replaces it is `send_without_a_target_flag_is_refused_and_names_the_one_
// that_is_a_target` above, which pins that the shape cannot be typed at all.

#[test]
fn send_to_persona_rejects_a_hop_count_past_the_depth_guard_even_with_the_counter_wiped() {
    // nxf 6j6v.m48m, at the seam the ticket was filed against. A role with Bash can `unset NXC_HOP`
    // before its own `nxc send --to` — here, simply never setting it — and used to reset the chain
    // to zero every hop. Its own `NXC_SESSION` is what the guard reads now, and the depth behind that
    // was written by whoever spawned it, not by this process.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-deep", "coding").unwrap();
        store.record_trigger_depth("s-deep", 33).unwrap();
    }
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-deep")
        .env_remove("NXC_HOP")
        .args(["--json", "send", "hello", "--to", "coding"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );
    let out = nxc(&tmp)
        .args(["--json", "search", "hello"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0,
        "nothing was persisted past the cap"
    );
}

#[test]
fn send_to_persona_rejects_a_hop_count_past_the_depth_guard() {
    // spec §5.3: a claimed `NXC_HOP` > 32 is a `validation` error, checked before any work (no
    // message persisted, no session minted). The claim can only ever RAISE the depth resolved from
    // the store (nxf 6j6v.m48m), so this path is unchanged.
    //
    // The role IS declared here, unlike when this went through `send --role`. The refusals are now
    // reached in the other order: `surface::send_to` resolves the TARGET first and would answer
    // "no such target" for an undeclared handle, where `--role` met the depth guard before it
    // resolved anything. Both refuse and neither persists; declaring the role is what keeps this
    // case about the guard it is named for.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();
    let out = nxc(&tmp)
        .env("NXC_HOP", "33")
        .args(["--json", "send", "hello", "--to", "coding"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );
    // Nothing was persisted — the guard is the first thing the verb does, before any write.
    let out = nxc(&tmp)
        .args(["--json", "search", "hello"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0
    );
}

#[test]
fn send_to_persona_accepts_a_hop_count_at_the_depth_guard_boundary() {
    // Test Quality #3: the existing depth-guard tests only ever exercise the REJECTED side
    // (`NXC_HOP=33`). `32` is the last ALLOWED value (the guard is `hop() > 32`) — prove it
    // succeeds AND actually triggers, not merely that it doesn't get rejected.
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();
    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_HOP", "32")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "hello", "--to", "coding"])
        .assert()
        .success();
    assert!(
        json_of(&out.get_output().stdout)["session"]
            .as_str()
            .unwrap()
            .starts_with("m-"),
        "NXC_HOP=32 (the boundary) is allowed and mints a session"
    );
    assert!(
        std::fs::read_to_string(&dry)
            .unwrap()
            .contains("trigger role=coding"),
        "and actually triggers the worker"
    );
}

// ---- reply routes to the return address + send --session (nxf epic 6j6v.zenf, T7) -----------

#[test]
fn reply_routes_to_return_address_and_resumes() {
    // Resolve the channel through `create_channel`'s minted id, not the literal display name
    // "work" — the two are unrelated (`store.mint_channel_id()`).
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let cid = create_channel(&tmp, "work");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();

    // Reconciliation (brief's own note): `reply` only routes when the return-address session is a
    // KNOWN ROLE session (`session_role` returns a role). In the real end-to-end flow this holds
    // naturally — every caller session was itself minted via a prior `send --role`. This unit test
    // skips that full chain, so seed it explicitly: `s-pm` is registered under role "pm" (Rust-side,
    // mirroring `session_bind_fills_in_the_real_sdk_session_id`'s precedent) and bound to a real SDK
    // id, so the reply's resume resolves a real id too.
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // PM (s-pm) sends a work order; the ambient NXC_SESSION lands in the message's refs.session_id
    // (T6) — that IS the return address `reply` below must route back to.
    let (mid, _root) = open_thread_from(&tmp, &cid, "pm", "order", "s-pm");

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-coding")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &mid, "which db?"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    assert!(log.contains("session=s-pm"), "{log}");
    assert!(log.contains("role=pm"), "{log}");
    assert!(log.contains("resume=real-pm"), "{log}");
    assert!(log.contains("msg=which db?"), "{log}");
    // Code Quality #4: `reply`'s `--json` surfaces whether it actually triggered a resume.
    assert_eq!(json_of(&out.get_output().stdout)["resumed"], true);
}

#[test]
fn reply_rejects_a_hop_count_past_the_depth_guard() {
    // Fix round (T7 review finding): `reply`'s return-address routing shares `trigger_role`/
    // `trigger_env` with `send`, so a pair of roles that only ever bounce `reply` back and forth
    // (never `send --role`/`--session`) needs the identical `NXC_HOP` depth guard `send` already
    // has — checked before any work, same as `send`'s. Seed the SAME known-role return address as
    // `reply_routes_to_return_address_and_resumes` so the routing would otherwise fire, to prove the
    // guard trips even when routing is fully eligible.
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let cid = create_channel(&tmp, "work");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();
    let (mid, _root) = open_thread_from(&tmp, &cid, "pm", "order", "s-pm");

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_HOP", "33")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &mid, "which db?"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );
    // No worker was ever triggered...
    assert!(!dry.exists(), "guard trips before the trigger runs");
    // ...AND the reply itself was never persisted — the guard blocks the whole call, matching
    // `send`'s own guard shape (no message post past the cap either).
    let out = nxc(&tmp)
        .args(["--json", "search", "which db?"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0
    );
}

#[test]
fn reply_to_a_message_with_no_known_role_return_address_triggers_no_worker() {
    // Self-review guard: a reply to a message whose SENDER was never session-tracked (a plain
    // human/ad-hoc post, or a return address that isn't a registered role session) must not crash
    // or misbehave — it just doesn't trigger a worker.
    let tmp = workspace();
    let cid = create_channel(&tmp, "work");
    // A conversation whose root carries NO return address at all — the shape this test is about.
    let (mid, _root) = open_thread(&tmp, &cid, "plain post");

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &mid, "just a reply"])
        .assert()
        .success();
    assert!(out.get_output().status.success());
    assert!(!dry.exists(), "no known role return address — no trigger");
    assert_eq!(
        json_of(&out.get_output().stdout)["resumed"],
        false,
        "no address at all"
    );
}

#[test]
fn reply_to_an_unregistered_return_address_triggers_no_worker() {
    // Test Quality #1: distinct from the "no return address at ALL" case above — here
    // `refs.session_id` IS set on the target message, but that internal id was never
    // `create_pending_session`'d, so `session_role` returns `None` for it. `reply` must still
    // succeed cleanly with no worker triggered.
    let tmp = workspace();
    let cid = create_channel(&tmp, "work");
    let (mid, _root) = open_thread_from(
        &tmp,
        &cid,
        "pm",
        "plain post with an address",
        "s-never-registered",
    );

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &mid, "just a reply"])
        .assert()
        .success();
    assert!(
        !dry.exists(),
        "return address set but not a registered role session — no trigger"
    );
    let json = json_of(&out.get_output().stdout);
    assert_eq!(
        json["resumed"], false,
        "an unregistered return address must not report resumed:true"
    );
    // nxf 6j6v.0akf's companion guard: `resumed: false` here means "there was nothing to wake", and
    // the ONLY thing distinguishing it from a resume that was attempted and lost is the absence of
    // `wake_skipped`. The key must stay out entirely — not a null, not an empty record.
    assert_eq!(
        json["wake_skipped"],
        Value::Null,
        "nothing was owed a wake, so nothing is reported: {json}"
    );
}

// `send_session_resumes_a_given_session_without_minting_or_role` and
// `send_session_of_an_unknown_session_is_not_found` stood here. REMOVED with `send --session`
// itself, by block (b) of 6j6v.dvyq §3 — and nothing takes their place, because nothing takes the
// verb's place. §3 writes "send --session -> reply --thread"; the two are not the same act, and
// the owner ruled on 2026-08-19 that this one is a LOSS rather than a collapse. `reply --thread`
// answers a conversation and reaches the other side through the thread's return address, which
// `surface::thread_return_address` reads off the newest message somebody ELSE posted there — so a
// persona that has not answered yet cannot be reached by it at all. Delivering into a session that
// is mid-turn is nxf 6j6v.xr3z's `reply --thread <id> --force`, and it waits on a runtime signal
// that does not exist.
//
// What the two cases pinned lives on where the body does: `orchestration::role_resume` keeps its
// `Ctx`-level coverage in `orchestration_verbs.rs` and `orchestration_reply.rs`. No surface drives
// it any more, so there is no entrance left for a CLI case to enter by.

#[test]
fn send_to_an_unknown_persona_is_not_found_and_persists_nothing() {
    // Review Code Quality #1/#2, Integrity #2/#3: an unknown handle must be `not_found` (NOT the
    // generic `io` that `load_role`'s own `read_to_string` failure would otherwise produce), and
    // resolved BEFORE the message posts. Both halves survive the move from `--role` to `--to`
    // unchanged; what differs is only WHICH resolution answers — `surface::send_to`'s own target
    // lookup rather than `Definitions::role`.
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "send", "hello", "--to", "no-such-role"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
    let out = nxc(&tmp)
        .args(["--json", "search", "hello"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0,
        "nothing was persisted"
    );
}

// Three cases stood here and are REMOVED with the flags they were typed at (6j6v.dvyq §3, block b).
//
// `send_role_rejects_a_path_traversal_handle_before_any_file_read` and
// `send_role_rejects_the_reserved_synth_and_delivered_handles` both fed a hostile string to
// `--role`, which resolved it through `Definitions::role` and thus through `validate_role_handle`.
// `send --to` never does: it matches its target against the handles already IN the catalogue, so a
// `../../etc/passwd` matches nothing and reaches no filesystem read to guard. The guard itself is
// untouched and still enforced on BOTH sides it applies to — construction (a declaration file that
// claims such a handle) and lookup — with its coverage in `definitions.rs`
// (`new_rejects_a_role_handle_with_a_path_separator`,
// `new_rejects_every_engine_internal_handle_as_a_class_not_as_a_list`,
// `role_returns_validation_for_a_malformed_handle`). What went is the CLI's way of REACHING it,
// not the guard.
//
// `send_role_and_send_session_are_mutually_exclusive` pinned a `conflicts_with` between two flags
// that no longer exist. `send_no_longer_takes_role_or_session` above is what stands in its place.

// ---- which model a `send` trigger runs on (nxf 6j6v.q6m2, then 6j6v.dvyq §3) ------------------
//
// **The per-call override is GONE.** `nxc send --model <fable|opus|sonnet>` was on 6j6v.dvyq §3's
// decided removal list (owner: "vielleicht nehmen wir es spaeter wieder rein") and 6j6v.e9qj pulled
// the removal forward, because that item moves "which model" onto the DECLARATION and the two would
// have been two answers to one question. The four tests that drove the flag went with it; what is
// left is the claim that outlives it — a trigger runs on what its own declaration says, and a role
// that declares nothing sends no model key at all.
//
// The seam is untouched (dvyq §5): `Commission`/`SendToRequest` still carry `model`, so an
// embedding app still chooses per call. There is simply no way for an AGENT to type it.
//
// `DryWorker`'s log line records role/session/resume/message only — the resolved model is
// observable ONLY in the spec JSON the real `SidecarWorker` writes, so this test points
// `NXC_SIDECAR` at a NONEXISTENT script: `node` still resolves on `PATH` and fails fast on the
// missing path, but the spec JSON has already been written before that doomed spawn (the pattern
// `trigger_workflow.rs` and `channel_complete.rs` established).

/// `nxc` wired to the REAL `SidecarWorker` over a nonexistent sidecar script — see the section note.
fn nxc_sidecar(tmp: &TempDir) -> Command {
    let mut c = nxc(tmp);
    c.env("NXC_WORKER", "sidecar").env(
        "NXC_SIDECAR",
        tmp.path().join("does-not-exist").join("main.mjs"),
    );
    c
}

/// The spec JSON `SidecarWorker::trigger` wrote for `session`.
fn spec_json(tmp: &TempDir, session: &str) -> Value {
    let path = tmp
        .path()
        .join(".nxs/agent-logs")
        .join(format!("{session}.spec.json"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("valid spec json")
}

/// A workspace with a single `coding` role whose YAML carries `yaml_extra` verbatim.
fn workspace_with_coding_role(yaml_extra: &str) -> TempDir {
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        format!("handle: coding\nsystem_prompt: c\ntools: [Bash]\n{yaml_extra}"),
    )
    .unwrap();
    tmp
}

#[test]
fn a_send_trigger_runs_on_the_roles_own_declared_model_or_on_none_at_all() {
    // With the per-call flag gone, the role's own declaration is the only thing above the SDK: a
    // role that names a model runs on it, and a role that names none must leave the key ABSENT —
    // the `tools` discipline that keeps "no choice" distinguishable from "a choice".
    let tmp = workspace_with_coding_role("model: sonnet\n");
    let out = nxc_sidecar(&tmp)
        .args(["--json", "send", "hello", "--to", "coding"])
        .assert()
        .success();
    let session = json_of(&out.get_output().stdout)["session"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(spec_json(&tmp, &session)["model"], "claude-sonnet-5");

    let tmp = workspace_with_coding_role("");
    let out = nxc_sidecar(&tmp)
        .args(["--json", "send", "hi", "--to", "coding"])
        .assert()
        .success();
    let session = json_of(&out.get_output().stdout)["session"]
        .as_str()
        .unwrap()
        .to_string();
    let spec = spec_json(&tmp, &session);
    assert_eq!(
        spec["model"],
        Value::Null,
        "absent when nothing is declared"
    );
}

// ---- summoning a persona auto-opens the DM (6j6v.kr3k) ----------------------

#[test]
fn send_to_a_persona_auto_opens_the_dm() {
    // No prior `channels dm` step: naming the PERSONA must auto-resolve the deterministic DM
    // between the caller and that persona, post there, and mint a fresh session. There is no other
    // shape left to compare it against — this used to be `send --role` without a channel
    // positional, next to a `--role` form that took one, and 6j6v.dvyq §3 removed both.
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "do the thing", "--to", "coding"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert!(
        v["session"].as_str().unwrap().starts_with("m-"),
        "fresh session minted: {v}"
    );
    assert!(std::fs::read_to_string(&dry)
        .unwrap()
        .contains("trigger role=coding"));

    // The channel actually used is the deterministic `dm:` id between the caller (`local/alice`,
    // the harness's `NXC_ACTOR`/`NXC_ORIGIN`) and the qualified target role (`local/coding`) — a
    // direct store read of the posted message's own channel_id, not just the persist-ack.
    let expected = dm_id("local/alice", "local/coding");
    let store = open_store(&tmp);
    let channel_id: String = store
        .connection()
        .query_row(
            "SELECT channel_id FROM messages WHERE body = 'do the thing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(channel_id, expected);

    // Both the caller and the role are members — exactly two, no leaked/missing side.
    let rows = store.list_member_channels("local/alice").unwrap();
    let row = rows.iter().find(|c| c.channel_id == expected).unwrap();
    assert_eq!(row.members, 2);
    assert_eq!(row.kind.as_deref(), Some("direct"));
}

#[test]
fn send_to_the_callers_own_handle_is_a_validation_error_and_persists_nothing() {
    // Test Quality finding (final independent review of PR #254): `ensure_dm_channel`'s self-DM
    // guard (`a == b` -> validation, cli.rs) already has coverage at the `channels dm` call site
    // (`dm_is_deterministic_and_exactly_two_members`), but the NEW auto-open call
    // site had no equivalent — a caller whose own actor handle matches the target role's handle
    // hits this exact branch and it was unexercised.
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("alice.yaml"),
        "handle: alice\nsystem_prompt: a\ntools: [Bash]\n",
    )
    .unwrap();

    // The harness's default `NXC_ACTOR`/`NXC_ORIGIN` is `local/alice` — matches the target
    // role's qualified handle exactly, so the auto-opened DM would need to be with itself.
    let out = nxc(&tmp)
        .args(["--json", "send", "do the thing", "--to", "alice"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "validation"
    );

    // Nothing persisted — same discipline as `send`'s other pre-flight validation failures.
    let store = open_store(&tmp);
    let count: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "a validation error before posting must persist nothing"
    );
}

#[test]
fn send_role_auto_open_is_idempotent_across_calls() {
    // Calling it twice (same caller/role pair) must reuse the SAME `dm:` channel id — no duplicate
    // channel, no error on the second open.
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();
    let dry = tmp.path().join("dry.log");

    for body in ["first task", "second task"] {
        nxc(&tmp)
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", &dry)
            .args(["--json", "send", body, "--to", "coding"])
            .assert()
            .success();
    }

    let expected = dm_id("local/alice", "local/coding");
    let store = open_store(&tmp);
    for body in ["first task", "second task"] {
        let channel_id: String = store
            .connection()
            .query_row(
                "SELECT channel_id FROM messages WHERE body = ?1",
                [body],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(channel_id, expected, "same dm: channel reused for {body}");
    }
    // Still exactly two members — the second open did not duplicate membership.
    let rows = store.list_member_channels("local/alice").unwrap();
    let row = rows.iter().find(|c| c.channel_id == expected).unwrap();
    assert_eq!(row.members, 2);
}

#[test]
fn send_with_no_target_flag_persists_nothing() {
    // A `send "<body>"` naming no target at all has nothing to address and must fail, persisting
    // nothing. The reason has changed twice: "no channel to auto-resolve one from", then — when
    // 6j6v.dvyq §3 took the positional — a hand-written refusal naming the three target flags, and
    // now, with block (b) leaving `--to` alone and REQUIRED, clap's own.
    //
    // Which is why this no longer reads an `{"error":…}` envelope: a clap refusal happens before
    // the program runs and carries no JSON, whatever `--json` says. That is the deliberate trade
    // (`Cli::Send`'s own note, and `reply --thread`'s identical shape from block (a)) — a help page
    // that showed `--to` as optional would be untrue, and clap names the missing flag precisely
    // because there is only one. The failure and the empty store are what this case is for, and
    // `send_without_a_target_flag_is_refused_and_names_the_one_that_is_a_target` above reads the
    // message itself.
    let tmp = workspace();
    nxc(&tmp)
        .args(["--json", "send", "orphan body"])
        .assert()
        .failure();
    let out = nxc(&tmp)
        .args(["--json", "search", "orphan body"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout).as_array().unwrap().len(),
        0,
        "nothing was persisted"
    );
}

#[test]
fn reply_routes_to_return_address_through_an_auto_opened_dm() {
    // The auto-DM path (6j6v.kr3k) only changes WHERE `send --role` posts — `reply`'s
    // return-address routing (unchanged) must compose with it exactly as it already does with an
    // ordinary channel (mirrors `reply_routes_to_return_address_and_resumes`).
    let tmp = tempfile::tempdir().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("coding.yaml"),
        "handle: coding\nsystem_prompt: c\ntools: [Bash]\n",
    )
    .unwrap();

    // Seed the caller's own ambient session as a KNOWN role session (mirrors
    // `reply_routes_to_return_address_and_resumes`) so reply's resume has a real return address.
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // No prior channel step: `send --to <persona>` auto-opens the DM between the caller and coding
    // AND the thread inside it, and hands back the thread id — which is the address `reply` takes
    // since 6j6v.dvyq §3. `send --role` (still standing, and on §3's list for the next block) posts
    // into the same auto-opened DM but mints no thread, so it has no address to hand back; the
    // entrance under test here is the one that does.
    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "coding", "task for coding"])
        .assert()
        .success();
    let mid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-coding")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &mid, "which db?"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    assert!(log.contains("session=s-pm"), "{log}");
    assert!(log.contains("role=pm"), "{log}");
    assert!(log.contains("resume=real-pm"), "{log}");
    assert!(log.contains("msg=which db?"), "{log}");
    // Code Quality #4: `reply`'s `--json` surfaces whether it actually triggered a resume.
    assert_eq!(json_of(&out.get_output().stdout)["resumed"], true);
}

// ---- quorum PUSH-wake: ask's ambient return address + reply's second check (6j6v.zebd) ----

/// Bump the `threads` table's row count by `n` inert decoy rows (direct store writes — mirrors
/// `seed_review_board`'s seeding technique). Pure test plumbing: `mint_thread_id`/`post_message`
/// each derive their deterministic id from their OWN table's row count (`mint_id`, store.rs), so a
/// fresh workspace's very FIRST thread and its own root request message mint the exact SAME number
/// by coincidence (`ask_json_is_byte_stable_under_the_determinism_switch` documents this: both
/// read `m-...0001`). Left alone, a quorum test's `ask`-minted `thread_id` could then collide with a
/// message id it goes on to mint, and a `reply` targeting the thread id would resolve to the WRONG
/// (channel, thread) pair via `message_channel`'s message-id branch. Bumping the threads counter
/// first (a table `ask` never touches otherwise) pushes the real `ask`'s `thread_id` sequence number
/// safely past every message id the test mints afterward.
fn bump_thread_counter(store: &mut ChatStore, n: usize) {
    let root = ThreadRoot {
        origin: "local".into(),
        channel_id: "decoy-channel".into(),
        opener: "local/decoy".into(),
        created: "2026-07-08T00:00:00Z".into(),
        parent: None,
    };
    for i in 0..n {
        store.open_thread(&format!("decoy-thread-{i}"), &root, "local/decoy");
    }
}

#[test]
fn reply_completing_a_review_quorum_wakes_the_pm_exactly_once() {
    // 6j6v.zebd sub-design (PUSH-wake): `ask` ambient-stamps the thread's ROOT message with the
    // opener's own session as its return address; the LAST expected handle's `reply` then resumes
    // that session via the SAME return-address mechanism `reply`'s direct routing already uses,
    // applied to the thread's root instead of the reply's own target — proving the PM wakes exactly
    // once, exactly when the quorum completes, never before.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");

    // The PM's ambient session ("s-pm") is a KNOWN role session, bound to a real SDK id — mirrors
    // `reply_routes_to_return_address_and_resumes`'s seeding precedent.
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // PM (s-pm) commissions a small 2-handle quorum via `ask` — the ambient stamp under test lands
    // the PM's session as the thread ROOT message's return address.
    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1", "local/r2"],
        None,
        Some("s-pm"),
    );
    let v = out.clone();
    let tid = v["thread_id"].as_str().unwrap().to_string();
    // `ask`'s existing `--json` contract is unaffected by the new ambient-stamping: no `refs` field
    // leaks into `AskReceipt` (spec `{thread_id, message_id, persisted, expects, deadline?}`).
    assert!(v.get("refs").is_none(), "{v}");

    // The root message really did get the PM's ambient session as its return address.
    let store = open_store(&tmp);
    let root_id = store.messages_in_thread(&tid).unwrap()[0]
        .message_id
        .clone();
    assert_eq!(
        store.message_return_address(&root_id).unwrap().as_deref(),
        Some("s-pm")
    );
    drop(store);

    let dry = tmp.path().join("dry.log");

    // r1 replies first — the quorum is still 1/2 → no wake yet.
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: looks fine"])
        .assert()
        .success();
    assert!(
        !dry.exists(),
        "quorum incomplete after only 1 of 2 expected replies — no wake yet"
    );

    // r2 replies — completes the quorum (2/2) → exactly ONE new trigger: role=pm, resume=real-pm.
    nxc(&tmp)
        .env("NXC_ACTOR", "r2")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r2: approved"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one wake, fired exactly on the completing reply: {log}"
    );
    assert!(lines[0].contains("role=pm"), "{log}");
    assert!(lines[0].contains("session=s-pm"), "{log}");
    assert!(lines[0].contains("resume=real-pm"), "{log}");
    assert!(
        lines[0].contains(&format!("thread {tid} is complete (2/2 replied)")),
        "synthesized wake message, not the raw reviewer finding text: {log}"
    );
    // Final whole-branch review of the role-runtime-v3 epic: this LEGACY (non-declared-channel)
    // wake path predates the epic and used to hardcode a domain-specific tail naming "the coder" —
    // the ONE remaining domain-specific string left in engine code. The epic's own NEW
    // declared-channel completion path is already fully generic (`compose_pass_through_wake`/
    // `compose_synthesis_trigger`); this assertion pins the same genericity here, on the
    // pre-existing v2 path this test already exercises (a channel with no declaration behind it,
    // not a declared v3 `ChannelDecl`).
    assert!(
        !lines[0].contains("coder"),
        "wake message must stay role-agnostic, not name a specific domain role: {log}"
    );
}

#[test]
fn reply_completing_a_quorum_with_no_ambient_ask_time_session_triggers_no_wake() {
    // Test Quality finding (final independent review of PR #254): the direct 1:1 routing path
    // already covers "no known return address, no worker triggered"
    // (`reply_to_a_message_with_no_known_role_return_address_triggers_no_worker`); the fan-in
    // quorum PUSH-wake path had no equivalent test. `ask` run from a bare terminal (no
    // `NXC_SESSION`) never stamps the thread root's `refs.session_id`, so quorum completion must
    // not crash or spuriously wake anything once the last expected handle replies.
    let tmp = workspace();
    let cid = create_channel(&tmp, "review");

    // `ask` with NO `NXC_SESSION` set — the root message gets no return address.
    let out = open_board(&tmp, &cid, "please review", &["local/r1", "local/r2"], None);
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    let store = open_store(&tmp);
    let root_id = store.messages_in_thread(&tid).unwrap()[0]
        .message_id
        .clone();
    assert_eq!(store.message_return_address(&root_id).unwrap(), None);
    drop(store);

    let dry = tmp.path().join("dry.log");
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: looks fine"])
        .assert()
        .success();
    assert!(
        !dry.exists(),
        "quorum incomplete after 1 of 2 replies — no wake yet"
    );

    // r2 completes the quorum (2/2) — must not crash and must not trigger any wake: there is no
    // return address to resume.
    let out = nxc(&tmp)
        .env("NXC_ACTOR", "r2")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r2: approved"])
        .assert()
        .success();
    assert_eq!(json_of(&out.get_output().stdout)["posted"], true);
    assert!(
        !dry.exists(),
        "quorum completed with no ambient ask-time session — no return address to resume, no wake"
    );
}

#[test]
fn reply_to_a_review_quorum_does_not_wake_the_replying_session_itself() {
    // Self-review guard: if the session replying into the thread happens to BE the return-address
    // session the completion would otherwise resume (a role somehow replying into its own opened
    // thread), the wake must not re-trigger it.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // A single-handle quorum expecting only "local/r1" — its reply alone completes it.
    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1"],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    let dry = tmp.path().join("dry.log");
    // r1's reply carries `NXC_SESSION=s-pm` itself (the pathological case under test) — the
    // completing reply's OWN session equals the resolved return address.
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: approved"])
        .assert()
        .success();
    assert!(
        !dry.exists(),
        "a session replying into its own opened (now-complete) thread must not re-trigger itself"
    );
}

// ---- quorum PUSH-wake fix round: two reproduced bugs + error hardening (6j6v.zebd CORRECTION
// note 01KY0SDTZPRZXWPDZG4EK2PGC5) ----

#[test]
fn a_reply_into_a_board_does_not_wake_early_with_raw_content() {
    // Bug 1, held on the address that survives. This was
    // `reply_to_the_asks_root_message_id_does_not_wake_early_with_raw_content`: the review-role
    // YAMLs used to say `nxc reply <thread-or-message-id> "…"`, so a reviewer could reply to the
    // board's own ROOT MESSAGE id, and the root carries the requester's `refs.session_id` — which
    // made the direct-routing block find that address and resume the PM on the FIRST reply,
    // carrying the reviewer's RAW body instead of a synthesized wake.
    //
    // 6j6v.dvyq §3 removes the message id as an address entirely: `reply --thread` takes a thread
    // id, so "the thread id or a message id within it" is one thing now. The PROPERTY is unchanged
    // and is what this still holds — a board defers ENTIRELY to the completion check, wakes once,
    // and never carries raw reviewer text.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1", "local/r2"],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    let dry = tmp.path().join("dry.log");

    // r1 answers the board.
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: looks fine"])
        .assert()
        .success();
    assert!(
        !dry.exists(),
        "quorum incomplete after only 1 of 2 expected replies — must not wake early, and must \
         never carry raw reviewer content"
    );

    // r2 completes the quorum.
    nxc(&tmp)
        .env("NXC_ACTOR", "r2")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r2: approved"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one wake, fired exactly on the completing reply: {log}"
    );
    assert!(lines[0].contains("role=pm"), "{log}");
    assert!(lines[0].contains("session=s-pm"), "{log}");
    assert!(lines[0].contains("resume=real-pm"), "{log}");
    assert!(
        lines[0].contains(&format!("thread {tid} is complete (2/2 replied)")),
        "synthesized wake message, not the raw reviewer finding text: {log}"
    );
    assert!(
        !lines[0].contains("r2: approved") && !lines[0].contains("r1: looks fine"),
        "must never carry raw reviewer content: {log}"
    );
}

#[test]
fn a_later_reply_into_an_already_complete_quorum_does_not_refire_the_wake() {
    // Bug 2: `thread_quorum(...).complete` is a stateless "is complete NOW" snapshot, not "did THIS
    // reply cause completion." A later, unrelated reply from an ALREADY-replied handle into an
    // already-complete thread must not re-trigger the identical wake.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1", "local/r2"],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    let dry = tmp.path().join("dry.log");

    // r1 replies first — quorum 1/2, no wake yet.
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: looks fine"])
        .assert()
        .success();
    assert!(!dry.exists());

    // r2 completes the quorum (2/2) — exactly one wake.
    nxc(&tmp)
        .env("NXC_ACTOR", "r2")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r2: approved"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    assert_eq!(log.lines().count(), 1, "one wake on completion: {log}");

    // r1 posts a later, unrelated follow-up into the now-already-complete thread — must NOT
    // re-trigger the identical wake a second time.
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: one more thought"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    assert_eq!(
        log.lines().count(),
        1,
        "a later reply into an already-complete thread must not re-fire the wake: {log}"
    );
}

#[test]
fn a_bystander_not_in_expect_replying_into_an_already_complete_quorum_does_not_refire_the_wake() {
    // Re-review finding (6j6v.zebd, after the two-bug fix round above): `is_completing_reply` only
    // checked that the replying sender's post was their FIRST message in the thread — it never
    // checked that the sender is actually a MEMBER of `q.expects`. `reply` has no membership gate
    // (facade::reply posts unconditionally), so any handle in the system — one never named in the
    // `ask --expect ...` call, who has never posted in this thread before — could post into an
    // ALREADY-COMPLETE quorum thread for the first time and re-fire the wake, because "their post is
    // their first message in the thread" is trivially true for anyone who has never posted there,
    // expected or not. This reproduces exactly that: a `"bystander"` handle NOT in `--expect`, first
    // reply into an already-complete 2/2 thread, must produce NO new trigger.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1", "local/r2"],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    let dry = tmp.path().join("dry.log");

    // r1 replies first — quorum 1/2, no wake yet.
    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: looks fine"])
        .assert()
        .success();
    assert!(!dry.exists());

    // r2 completes the quorum (2/2) — exactly one wake.
    nxc(&tmp)
        .env("NXC_ACTOR", "r2")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r2: approved"])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    assert_eq!(log.lines().count(), 1, "one wake on completion: {log}");

    // A "bystander" handle — never named in `--expect`, never posted here before — replies into the
    // now-already-complete thread for the FIRST time. Its post trivially satisfies "first message in
    // the thread", but it is not a member of `q.expects`, so it must NOT be treated as completing.
    nxc(&tmp)
        .env("NXC_ACTOR", "bystander")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "--json",
            "reply",
            "--thread",
            &tid,
            "bystander: just passing by",
        ])
        .assert()
        .success();
    let log = std::fs::read_to_string(&dry).unwrap();
    assert_eq!(
        log.lines().count(),
        1,
        "a non-expected handle's first post into an already-complete quorum must not re-fire the \
         wake: {log}"
    );
}

#[test]
fn reply_completing_a_quorum_with_a_since_deleted_role_does_not_fail_the_reply() {
    // Error-handling hardening ("Also" in the CORRECTION note): if resolving the wake target fails
    // (session_role/resolve_real/resolve_role — here, the role file has since been deleted), that
    // must not propagate as a hard error out of an already-successfully-persisted `reply` call.
    // Skip the wake attempt instead; the reply itself must still succeed.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1"],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    // The role file has since been deleted (or gone malformed) between commissioning and
    // completion — a real-world race the wake resolution must tolerate.
    std::fs::remove_file(roles.join("pm.yaml")).unwrap();

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "reply", "--thread", &tid, "r1: approved"])
        .assert()
        .success();
    assert_eq!(json_of(&out.get_output().stdout)["posted"], true);
    assert!(
        !dry.exists(),
        "wake target unresolvable (deleted role) — no trigger, but the reply must still succeed"
    );
    // Observability fix (final independent review of PR #254, Integrity & Robustness #3): the
    // skip must not be completely silent — a stderr breadcrumb makes it diagnosable.
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("quorum wake skipped") && stderr.contains("s-pm"),
        "expected a wake-skipped breadcrumb naming the unresolvable session, got: {stderr}"
    );
    assert_eq!(
        json_of(&out.get_output().stdout)["wake_skipped"]["reason"],
        "unresolved",
        "`--json` carries the same finding the breadcrumb does — an agent reading stdout must not \
         have to parse stderr to learn its hand-off never happened"
    );
}

#[test]
fn reply_whose_quorum_wake_cannot_spawn_still_succeeds_and_reports_the_skip_in_json() {
    // nxf 6j6v.bxdd — the user-visible half of the owner's decision. On the NON-declared quorum path
    // `nxc reply` used to EXIT NONZERO when the opener's spawn failed, with the reply already
    // written: an error for a message that is in the store and readable. It now exits 0 like the
    // declared path, and `--json` gains `wake_skipped` so the nonzero exit is replaced by a
    // machine-readable finding rather than by silence — a stderr line alone would be a downgrade for
    // exactly the agent consumers `--json` exists for.
    //
    // The trigger is made to fail hermetically by pointing `NXC_DRY_LOG` at a DIRECTORY: `DryWorker`
    // opens it for append and gets `EISDIR`, which is a genuine `Worker::trigger` failure with no
    // sidecar, no network and no timing involved.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    let cid = create_channel(&tmp, "review");
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let out = open_board_from(
        &tmp,
        &cid,
        "please review",
        &["local/r1"],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    let unwritable = tmp.path().join("dry-log-dir");
    std::fs::create_dir(&unwritable).unwrap();

    let out = nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &unwritable)
        .args(["--json", "reply", "--thread", &tid, "r1: approved"])
        .assert()
        .success();

    let json = json_of(&out.get_output().stdout);
    assert_eq!(json["posted"], true);
    assert_eq!(json["wake_skipped"]["session"], "s-pm");
    assert_eq!(json["wake_skipped"]["reason"], "trigger_failed");
    assert!(
        json["wake_skipped"]["detail"]
            .as_str()
            .is_some_and(|d| d.contains("dry log")),
        "the underlying trigger failure travels with the finding: {json}"
    );
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("quorum wake failed") && stderr.contains("s-pm"),
        "and the breadcrumb stays, for the human reading the terminal: {stderr}"
    );

    // The reply is real content — the whole reason `Err` was the wrong answer.
    assert!(
        open_store(&tmp)
            .messages_in_thread(&tid)
            .unwrap()
            .iter()
            .any(|m| m.body == "r1: approved"),
        "the reply that used to come back as an error is in the thread"
    );
}

#[test]
fn reply_whose_direct_resume_cannot_spawn_still_succeeds_and_reports_the_skip_in_json() {
    // nxf 6j6v.0akf — the same user-visible change one level up, on the path `nxc reply` takes far
    // more often than the quorum one: the direct 1:1 return-address resume. It used to EXIT NONZERO
    // when the target's session could not be spawned, with the reply already written. It now exits
    // 0 and puts the finding in `--json`, so the exit code an agent lost is replaced by something
    // machine-readable rather than by silence.
    //
    // Same hermetic trigger failure as the quorum twin above: `NXC_DRY_LOG` points at a DIRECTORY,
    // so `DryWorker`'s append open gets `EISDIR` — a genuine `Worker::trigger` failure with no
    // sidecar, no network and no timing involved.
    let tmp = workspace();
    let cid = create_channel(&tmp, "work");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // The order carries `s-pm` as its return address — that is what the reply resumes.
    let (mid, _root) = open_thread_from(&tmp, &cid, "pm", "order", "s-pm");

    let unwritable = tmp.path().join("dry-log-dir");
    std::fs::create_dir(&unwritable).unwrap();

    let out = nxc(&tmp)
        .env("NXC_ACTOR", "coder")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &unwritable)
        .args(["--json", "reply", "--thread", &mid, "which db?"])
        .assert()
        .success();

    let json = json_of(&out.get_output().stdout);
    assert_eq!(json["posted"], true);
    assert_eq!(
        json["resumed"], false,
        "the resume did not happen — and on its own that is ambiguous, which is why the finding \
         below has to be there: {json}"
    );
    assert_eq!(json["wake_skipped"]["session"], "s-pm");
    assert_eq!(json["wake_skipped"]["reason"], "trigger_failed");
    assert!(
        json["wake_skipped"]["detail"]
            .as_str()
            .is_some_and(|d| d.contains("dry log")),
        "the underlying trigger failure travels with the finding: {json}"
    );
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("resume failed") && stderr.contains("s-pm"),
        "and the breadcrumb stays, for the human reading the terminal: {stderr}"
    );

    // The reply is real content — the whole reason `Err` was the wrong answer. Counted in the
    // `messages` view directly rather than through `nxc search`, which is membership-scoped and
    // would read empty for reasons that have nothing to do with persistence.
    let persisted: i64 = open_store(&tmp)
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE body = 'which db?'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        persisted, 1,
        "the reply that used to come back as an error is in the store"
    );
}

// ---- workflow.yaml's real commissioning form: qualified `--expect` handles (final whole-branch
// review of epic 6j6v.nmpk, THIRD fix round) ---------------------------------------------------

#[test]
fn review_quorum_commissioned_with_workflow_yamls_qualified_expect_handles_completes_and_wakes_pm()
{
    // Regression test for the third bug the epic 6j6v.nmpk final whole-branch review found by
    // running the real binary: `workflow.yaml`'s `pm_validate` step used to commission the review
    // quorum with BARE `--expect` handles (`--expect general`, not `--expect local/general`).
    // `thread_quorums`'s join (store.rs: `m.sender = je.value`) is an EXACT STRING match between
    // each `--expect` value and a reply's `sender` column, and a reply's `sender` is ALWAYS
    // origin-qualified (`facade.rs`: `format!("{}/{}", req.origin, req.actor)`). A reviewer spawned
    // via `nxc send --to <handle>` runs as actor `<handle>` (`worker.rs`'s `SidecarWorker::
    // trigger` stamps `NXC_ACTOR` to the bare `req.role.handle`) and therefore replies as
    // `local/<handle>` under the default, non-federated `local` origin (`cli.rs`'s `origin()`,
    // unset `NXC_ORIGIN`) — every workspace this example targets. `"general" != "local/general"` as
    // strings, so NONE of the four reviewers' replies ever counted toward `expects`/`replied`,
    // `outstanding` never emptied, `complete` never flipped `true`, and the PM was never woken — the
    // loop deadlocked at quorum completion instead of at reviewer-spawn (the previous round's bug).
    //
    // None of the EXISTING quorum tests in this file catch this: every one of them already uses
    // fully-qualified `--expect` values (`local/r1`, `local/r2`, …), so none exercises the example's
    // own (previously bare, now-fixed) commissioning form. This test drives that ACTUAL form
    // end-to-end: `ask` with the SAME qualified `--expect` values now in the fixed `workflow.yaml`,
    // `send --to <handle>` fan-out to all four reviewers (dry worker — no live SDK call; this
    // just proves the trigger fires and pins the sender identity a real triggered session would
    // reply with, per `worker.rs`'s `NXC_ACTOR` stamp cited above), then each reviewer's own
    // `nxc reply --thread <id> "…"` as its own actor (`NXC_ACTOR=general` etc., mirroring exactly
    // how
    // a real triggered session would have `NXC_ACTOR` set to its own role handle). Reverting
    // THIS TEST's own `--expect`/reply values back to bare handles (not `workflow.yaml`, which
    // this test never loads) reproduces the original bug: the fourth reviewer's reply never flips
    // `complete`, so no wake ever fires — that was the TDD RED this task recorded before qualifying
    // both this test's values and `workflow.yaml`'s own commissioning text. `workflow.yaml`'s
    // fixture content is separately pinned by `trigger_workflow.rs`'s needle string.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: p\ntools: [Bash]\n",
    )
    .unwrap();
    const REVIEWERS: [&str; 4] = ["general", "code-quality", "test-quality", "integrity"];
    for handle in REVIEWERS {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: r\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    let cid = create_channel(&tmp, "review");

    // The PM's ambient session ("s-pm") is a known role session bound to a real SDK id — mirrors
    // every other PUSH-wake test's seeding precedent above.
    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
        bump_thread_counter(&mut store, 10);
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // PM commissions the quorum board with the SAME qualified `--expect` handles workflow.yaml's
    // `pm_validate` step now instructs: `local/general`, `local/code-quality`, `local/test-quality`,
    // `local/integrity` (the fix under test — bare handles were the bug).
    let out = open_board_from(
        &tmp,
        &cid,
        "please review the branch",
        &[
            "local/general",
            "local/code-quality",
            "local/test-quality",
            "local/integrity",
        ],
        None,
        Some("s-pm"),
    );
    let tid = out.clone()["thread_id"].as_str().unwrap().to_string();

    // Trigger fan-out: the PM's own next instructed step, one `nxc send "<task>" --to <handle>`
    // call per reviewer (workflow.yaml's literal form) — proves the trigger fires for all four AND,
    // via the dry worker (never a live SDK call), that each spawned session's role handle is the
    // BARE handle: exactly what a real triggered session's own `NXC_ACTOR` would be stamped to.
    // The flag was `--role` until 6j6v.dvyq §3 collapsed it into `--to`; the body it reaches, and
    // therefore everything asserted below, is the same one.
    let trigger_log = tmp.path().join("trigger.log");
    for handle in REVIEWERS {
        nxc(&tmp)
            .env("NXC_SESSION", "s-pm")
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", &trigger_log)
            .args([
                "send",
                &format!("please review, thread {tid}"),
                "--to",
                handle,
            ])
            .assert()
            .success();
    }
    let log = std::fs::read_to_string(&trigger_log).unwrap();
    // The TRIGGER lines, not every line: since 6j6v.dvyq §3 a summon hands the persona
    // `surface::wake_message` — the caller's text plus the `reply --thread` obligation — and that
    // is two lines and a blank one per trigger, so counting raw lines counts the message body.
    let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("trigger ")).collect();
    assert_eq!(lines.len(), 4, "one trigger per reviewer: {log}");
    for handle in REVIEWERS {
        assert!(
            lines.iter().any(|l| l.contains(&format!("role={handle}"))),
            "missing trigger for {handle}: {log}"
        );
    }

    // Each reviewer replies as ITS OWN actor (`NXC_ACTOR=<handle>`) — mirroring exactly how a real
    // triggered session's own `nxc reply` would run (the same stamp the fan-out above just proved).
    let wake_log = tmp.path().join("wake.log");
    for (i, handle) in REVIEWERS.iter().enumerate() {
        nxc(&tmp)
            .env("NXC_ACTOR", handle)
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", &wake_log)
            .args([
                "--json",
                "reply",
                "--thread",
                &tid,
                &format!("{handle}: Grade: A"),
            ])
            .assert()
            .success();
        if i < REVIEWERS.len() - 1 {
            assert!(
                !wake_log.exists(),
                "quorum incomplete after only {}/4 expected replies — no wake yet",
                i + 1
            );
        }
    }

    // The quorum genuinely completed: every qualified handle counted, none outstanding.
    let store = open_store(&tmp);
    let q = store
        .thread_quorum(&tid, "2026-07-08T00:00:00Z")
        .unwrap()
        .unwrap();
    assert!(q.outstanding.is_empty(), "{q:?}");
    assert!(
        q.complete,
        "quorum must complete once all four qualified handles have replied: {q:?}"
    );
    drop(store);

    // …and the PM was woken exactly once, exactly on the fourth (completing) reply.
    let log = std::fs::read_to_string(&wake_log).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one PM wake, fired exactly on the completing reply: {log}"
    );
    assert!(lines[0].contains("role=pm"), "{log}");
    assert!(lines[0].contains("session=s-pm"), "{log}");
    assert!(lines[0].contains("resume=real-pm"), "{log}");
    assert!(
        lines[0].contains(&format!("thread {tid} is complete (4/4 replied)")),
        "synthesized wake message, not raw reviewer content: {log}"
    );
}

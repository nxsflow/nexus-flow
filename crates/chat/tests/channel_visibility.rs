//! Integration tests for the channel visibility filter — every reader that applies it.
//!
//! **The first section (6j6v.xrk3)** drives `thread_board` with an explicit `visibility: Visibility`
//! parameter — a CAPABILITY, not real wiring: when it was written, no name→channel_id resolution
//! from a declared `channels.yaml` to a message-substrate `channel_id` existed yet. Those tests call
//! `nexus_chat::facade` functions directly against an in-memory `ChatStore` (mirroring
//! `crates/chat/tests/workflow.rs`'s style), never spawning `nxc`. Acceptance, verbatim from that
//! ticket: under `requester_only` a member board excludes others' replies; the opener sees all.
//!
//! **The two sections below it drive `Engine` on declarations that live on DISK**, which is the only
//! path on which the policy is resolvable at all — a reader handed the value as a parameter proves
//! nothing about which value a real channel gets. 6j6v.yr59 moved the filter onto `thread`, the one
//! message reader the seam kept; 6j6v.px98 put the SAME rule on `search`, the reader beside it that
//! answered the same question differently. One question, one answer, whichever reader is asked.

use nexus_chat::channel::Visibility;
use nexus_chat::facade::{ask, reply, thread_board, AskRequest, ReplyRequest};
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs};
use nexus_chat::store::ChatStore;

const NOW: &str = "2026-07-21T00:00:00Z";

/// A fresh in-memory store with channel `c-1` and its three members: the opener plus two other
/// members whose replies are the subject of the visibility filter.
fn seed() -> ChatStore {
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("c-1", "kind", "group", "o/opener");
    s.set_channel_field("c-1", "name", "review", "o/opener");
    s.add_member("c-1", "o/opener", "o/opener");
    s.add_member("c-1", "o/member-a", "o/opener");
    s.add_member("c-1", "o/member-b", "o/opener");
    s
}

/// Open a board on `c-1` via the real `ask` facade — opener `o/opener`, expecting member-a +
/// member-b. Returns the minted thread id.
fn open_board(s: &mut ChatStore) -> String {
    ask(
        s,
        AskRequest {
            now: NOW,
            origin: "o",
            actor: "opener",
            channel: "c-1",
            body: "opening message",
            expect: &["o/member-a".to_string(), "o/member-b".to_string()],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
    )
    .unwrap()
    .thread_id
}

/// Post a reply into `thread_id` from `actor` (origin `o`) via the real `reply` facade.
fn reply_from(s: &mut ChatStore, thread_id: &str, actor: &str, body: &str) -> String {
    reply(
        s,
        ReplyRequest {
            now: NOW,
            origin: "o",
            actor,
            target: thread_id,
            body,
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            refs: Refs::default(),
            if_unanswered: false,
        },
    )
    .unwrap()
    .expect("if_unanswered is false, so this always posts")
    .message_id
}

fn bodies(messages: &[nexus_chat::facade::MessageView]) -> Vec<&str> {
    messages.iter().map(|m| m.body.as_str()).collect()
}

#[test]
fn requester_only_excludes_a_non_opener_members_view_of_anothers_reply() {
    let mut s = seed();
    let thread_id = open_board(&mut s);
    reply_from(&mut s, &thread_id, "member-a", "a's reply");
    reply_from(&mut s, &thread_id, "member-b", "b's reply");

    // member-b's own board: the opening message + member-b's own reply, NOT member-a's.
    let board = thread_board(&s, &thread_id, NOW, "o/member-b", Visibility::RequesterOnly).unwrap();
    assert_eq!(
        bodies(&board.messages),
        vec!["opening message", "b's reply"]
    );
}

#[test]
fn requester_only_never_filters_the_opener() {
    let mut s = seed();
    let thread_id = open_board(&mut s);
    reply_from(&mut s, &thread_id, "member-a", "a's reply");
    reply_from(&mut s, &thread_id, "member-b", "b's reply");

    // The opener (checked via quorum.opener, not some other field) sees everything, unfiltered.
    let board = thread_board(&s, &thread_id, NOW, "o/opener", Visibility::RequesterOnly).unwrap();
    assert_eq!(board.quorum.opener.as_deref(), Some("o/opener"));
    assert_eq!(
        bodies(&board.messages),
        vec!["opening message", "a's reply", "b's reply"]
    );
}

#[test]
fn all_members_visibility_is_a_true_no_op() {
    let mut s = seed();
    let thread_id = open_board(&mut s);
    reply_from(&mut s, &thread_id, "member-a", "a's reply");
    reply_from(&mut s, &thread_id, "member-b", "b's reply");

    // Under all_members, member-b sees everything — including member-a's reply.
    let board = thread_board(&s, &thread_id, NOW, "o/member-b", Visibility::AllMembers).unwrap();
    assert_eq!(
        bodies(&board.messages),
        vec!["opening message", "a's reply", "b's reply"]
    );
}

#[test]
fn requester_only_member_with_no_replies_sees_only_the_opening_message() {
    let mut s = seed();
    let thread_id = open_board(&mut s);
    // Only member-a replies; member-b has posted nothing of its own.
    reply_from(&mut s, &thread_id, "member-a", "a's reply");

    let board = thread_board(&s, &thread_id, NOW, "o/member-b", Visibility::RequesterOnly).unwrap();
    assert_eq!(bodies(&board.messages), vec!["opening message"]);
}

#[test]
fn requester_only_preserves_the_readers_own_original_reply_order() {
    let mut s = seed();
    let thread_id = open_board(&mut s);
    // Interleaved with member-a's reply, to prove filtering doesn't reorder — it only drops.
    reply_from(&mut s, &thread_id, "member-b", "b1");
    reply_from(&mut s, &thread_id, "member-a", "a1");
    reply_from(&mut s, &thread_id, "member-b", "b2");

    let board = thread_board(&s, &thread_id, NOW, "o/member-b", Visibility::RequesterOnly).unwrap();
    assert_eq!(bodies(&board.messages), vec!["opening message", "b1", "b2"]);
}

/// Independent review fix-round: `ensure_declared_channel` records a declared channel's `members`
/// as BARE handles (`cli.rs`), while every message `sender`/`ThreadQuorum::opener` is always fully
/// qualified (`origin/handle`) — the same bare-vs-qualified split `SYNTHESIS_HANDLE`'s own doc warns
/// about elsewhere in this codebase. Wiring `requester_only` up to the REAL declared-channel read
/// paths surfaced this: a caller passing the bare form (the only form that satisfies `is_member`
/// for a declared member — see `channel_fanout.rs`) must still see their OWN reply, not just the
/// opening message. `seed` here mirrors `ensure_declared_channel`'s exact membership shape: bare
/// declared members alongside the qualified opener.
#[test]
fn requester_only_recognizes_a_bare_handle_as_the_qualified_senders_own_reply() {
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("decl:review", "kind", "group", "o/opener");
    s.set_channel_field("decl:review", "name", "review", "o/opener");
    s.add_member("decl:review", "o/opener", "o/opener");
    s.add_member("decl:review", "member-a", "o/opener"); // bare, as ensure_declared_channel does
    s.add_member("decl:review", "member-b", "o/opener");

    let thread_id = ask(
        &mut s,
        AskRequest {
            now: NOW,
            origin: "o",
            actor: "opener",
            channel: "decl:review",
            body: "opening message",
            expect: &["o/member-a".to_string(), "o/member-b".to_string()],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
    )
    .unwrap()
    .thread_id;
    reply_from(&mut s, &thread_id, "member-a", "a's reply");
    reply_from(&mut s, &thread_id, "member-b", "b's reply");

    // "member-b" (bare), not "o/member-b" (qualified) — the shape a declared channel's own
    // membership is recorded in, and the only form `is_member` accepts for it.
    let board = thread_board(&s, &thread_id, NOW, "member-b", Visibility::RequesterOnly).unwrap();
    assert_eq!(
        bodies(&board.messages),
        vec!["opening message", "b's reply"],
        "a bare handle must still see its own qualified reply, not just the opening message"
    );
}

#[test]
fn quorum_metadata_is_identical_regardless_of_visibility_or_reader() {
    let mut s = seed();
    let thread_id = open_board(&mut s);
    reply_from(&mut s, &thread_id, "member-a", "a's reply");
    reply_from(&mut s, &thread_id, "member-b", "b's reply");

    let as_member_b_requester_only =
        thread_board(&s, &thread_id, NOW, "o/member-b", Visibility::RequesterOnly).unwrap();
    let as_member_b_all_members =
        thread_board(&s, &thread_id, NOW, "o/member-b", Visibility::AllMembers).unwrap();
    let as_opener_requester_only =
        thread_board(&s, &thread_id, NOW, "o/opener", Visibility::RequesterOnly).unwrap();

    // The who-has/hasn't-replied lists (and the rest of the derived quorum) never change with
    // `visibility` or who's asking — only message CONTENT is filtered (the epic spec §4.2 scoping).
    assert_eq!(
        as_member_b_requester_only.quorum,
        as_member_b_all_members.quorum
    );
    assert_eq!(
        as_member_b_requester_only.quorum,
        as_opener_requester_only.quorum
    );
    assert_eq!(
        as_member_b_requester_only.quorum.replied,
        vec!["o/member-a", "o/member-b"]
    );
    assert!(as_member_b_requester_only.quorum.outstanding.is_empty());
    assert_eq!(
        as_member_b_requester_only.quorum.expects,
        vec!["o/member-a", "o/member-b"]
    );
}

// ---- the ONE message reader carries the policy (nxf 6j6v.yr59) --------------
//
// `thread_board` was the reader that applied a channel's declared `visibility`; `thread` was the
// reader beside it that did not. yr59 leaves exactly ONE message reader on the handle, and it is
// `thread` — so the policy has to travel with it, or the cut would WIDEN what an app may read
// under `requester_only` while claiming to be a consolidation.
//
// Driven through `Engine`, on declarations that live on DISK, because that is the only path on
// which the policy is resolvable at all: `visibility` is a field of `channels.yaml`, and a reader
// that takes it as a parameter proves nothing about which value a real channel gets.

/// A workspace declaring one three-member channel — `visibility` unset, which MEANS
/// `requester_only` ([`nexus_chat::channel::Visibility`]'s `#[default]`) — plus the personas it
/// names. THREE members, not two, because the filter has to be told apart from "sees their own":
/// with only a requester and one member, everything the member may see it also wrote.
fn declared_workspace() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    let dir = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&dir).unwrap();
    for handle in ["pm", "dev", "qa"] {
        std::fs::write(
            dir.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("channels.yaml"),
        "- name: review\n  members: [pm, dev, qa]\n",
    )
    .unwrap();
    tmp
}

#[test]
fn the_one_message_reader_applies_the_channels_declared_visibility() {
    let (_tmp, e, thread) = a_board_with_two_members_answers();

    let mine = e
        .thread(&thread, "pm")
        .expect("the requester reads its own thread");
    assert_eq!(
        bodies(&mine.messages),
        ["please look at this", "dev looked", "qa looked"],
        "the requester sees the whole board unfiltered"
    );

    // The point: the SAME read, by a member who is NOT the requester, under a declaration that
    // says `requester_only`. Before yr59 this reader had no policy at all and handed back
    // everything — the filter lived on `thread_board`, which is the reader that went.
    let theirs = e
        .thread(&thread, "dev")
        .expect("a member reads the thread it takes part in");
    assert_eq!(
        bodies(&theirs.messages),
        ["please look at this", "dev looked"],
        "`requester_only`: the REQUEST and their own words, never the other member's answer"
    );
}

// ---- the SECOND reader answers the same question the same way (nxf 6j6v.px98) ----------------
//
// `search` joined the caller's live MEMBERSHIPS and nothing else, so on a `requester_only` board it
// handed a member a body that `thread` beside it withheld from that same caller. Two readers on one
// seam, two answers to "may I see this?" — and the owner ruled on 2026-08-23 that `visibility` is an
// ACCESS rule, not a display rule ("Versprechen für px98 und yxsa"), which makes the wider answer a
// broken confidentiality promise rather than a mere inconsistency.
//
// Driven through `Engine` on declarations that live on DISK for the reason the test above is: the
// policy is a field of `channels.yaml`, and a reader handed the value as a parameter proves nothing
// about which value a real channel gets.

/// The engine of [`declared_workspace`], with one board open on the declared channel `review` and
/// the two other members' answers posted into it — the fixture both declared-visibility reads share.
/// Returns the engine, its temp dir (which must outlive it) and the thread id.
fn a_board_with_two_members_answers() -> (tempfile::TempDir, nexus_chat::engine::Engine, String) {
    use nexus_chat::engine::{Engine, EngineConfig};
    use nexus_chat::model::MessageEnvelope;
    use nexus_chat::orchestration::Caller;
    use nexus_chat::surface::{SendToRefs, SendToRequest};
    use nexus_chat::timer::TimerConfig;
    use nexus_chat::worker::WorkerConfig;
    use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};

    let tmp = declared_workspace();
    let e = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let thread = e
        .send_to(
            Caller {
                session: None,
                actor: Some("pm"),
                now: Some(NOW),
            },
            SendToRequest {
                machine: None,
                to: "review",
                body: "please look at this",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("open the declared channel")
        .thread_id;
    let channel = e
        .thread(&thread, "pm")
        .expect("the requester reads its own thread")
        .channel_id;
    {
        let mut s = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.set_wall_clock(NOW);
        for (sender, body) in [("dev", "dev looked"), ("qa", "qa looked")] {
            s.post_message(&MessageEnvelope {
                origin: e.origin(),
                channel_id: channel.clone(),
                sender: format!("{}/{sender}", e.origin()),
                kind: MessageKind::Report,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                thread_id: Some(thread.clone()),
                refs: Refs::default(),
                body: body.into(),
            });
        }
    }
    (tmp, e, thread)
}

#[test]
fn the_text_search_applies_the_same_declared_visibility_as_the_one_message_reader() {
    let (_tmp, e, _thread) = a_board_with_two_members_answers();

    let hits = |as_handle: &str| -> Vec<String> {
        e.search(as_handle, "looked")
            .expect("search reads")
            .into_iter()
            .map(|h| h.body)
            .collect()
    };

    assert_eq!(
        hits("pm"),
        ["dev looked", "qa looked"],
        "the requester searches its own board unfiltered, exactly as it reads it"
    );
    assert_eq!(
        hits("dev"),
        ["dev looked"],
        "`requester_only`: a member finds its OWN words and never the other member's answer — the \
         same answer `thread` gives that same caller"
    );
}

#[test]
fn the_text_search_answers_a_declared_member_that_has_only_ever_replied() {
    // nxf 6j6v.cs03, at the seam and on a declaration that lives on DISK — the shape a user meets
    // first, and the one the test above could not see because it asks as the BARE handle. A caller
    // arrives QUALIFIED (`cli.rs`'s `resolve_consumer` falls back to `<origin>/<actor>`), while
    // `ensure_declared_channel` records a declared channel's `members:` BARE and additionally the
    // AUTHOR of the send. So the sender searched its own board and every member that had only
    // REPLIED matched no membership row at all: `nxc search` answered "no matches" to most of the
    // team, including for their own words. Membership is the declaration's answer for both readers
    // now, and `same_identity` accepts either shape.
    let (_tmp, e, _thread) = a_board_with_two_members_answers();

    let qualified = format!("{}/dev", e.origin());
    let hits: Vec<String> = e
        .search(&qualified, "looked")
        .expect("search reads")
        .into_iter()
        .map(|h| h.body)
        .collect();
    assert_eq!(
        hits,
        ["dev looked"],
        "a declared member finds its own reply under the identity the surface gives it — and \
         `requester_only` still reserves the other member's answer for whoever asked"
    );
}

#[test]
fn an_edit_to_declared_members_reaches_the_text_search_at_the_seam_in_both_directions() {
    // nxf 6j6v.cs03's other two directions, AT THE SEAM and on a declaration that lives on DISK.
    // Raised by the independent review of PR #440 (Test Quality #1) against this project's own
    // `engine-seam-test-rule`: a facade test does not count for a seam, because the products speak
    // `Engine::*`. The facade tests in `operation_opener_reads.rs` hold the RULE; this holds that a
    // real `channels.yaml` edit reaches it, in both directions, with nothing sent in between.
    let (tmp, e, thread) = a_board_with_two_members_answers();
    let personas = tmp.path().join(".nxs-personas");
    std::fs::write(
        personas.join("arch.yaml"),
        "handle: arch\nsystem_prompt: You are arch.\n",
    )
    .unwrap();

    let bodies = |as_handle: &str, q: &str| -> Vec<String> {
        e.search(as_handle, q)
            .expect("search reads")
            .into_iter()
            .map(|h| h.body)
            .collect()
    };
    let arch = format!("{}/arch", e.origin());
    let qa = format!("{}/qa", e.origin());

    // Where the two stand BEFORE the edit — so what follows is a change and not a coincidence.
    assert!(
        bodies(&arch, "please look").is_empty(),
        "no declaration names `arch` yet"
    );
    assert_eq!(
        bodies(&qa, "looked"),
        ["qa looked"],
        "`qa` is declared, and finds its own answer"
    );

    // The edit: `arch` written in, `qa` struck, and NOTHING sent afterwards.
    std::fs::write(
        personas.join("channels.yaml"),
        "- name: review\n  members: [pm, dev, arch]\n",
    )
    .unwrap();

    assert!(
        !bodies(&arch, "please look").is_empty(),
        "written into `members:`: the channel's request is findable at once, with no `send` to \
         materialise the edit"
    );
    assert!(
        bodies(&qa, "looked").is_empty(),
        "struck from `members:`: the answer it found a moment ago is gone"
    );
    assert_eq!(
        e.thread(&thread, &qa).unwrap_err().kind,
        nxs_foundation::error::ErrorKind::Forbidden,
        "and the reader beside it refuses that same caller in the same breath — one membership rule, \
         whichever seam read is asked"
    );
}

#[test]
fn the_declared_request_itself_stays_findable_by_a_member_that_did_not_open_it() {
    // The counterpart, and the reason this is a FILTER and not a narrowing of the membership join:
    // under `requester_only` a member still sees a thread's OPENING message, because it stands for
    // the REQUEST — hiding it would hide the task along with the other answers
    // (`filter_board_messages` carries that reasoning, and this reader has to mean the same by it).
    //
    // Asserted against the UNFILTERED store read rather than a hardcoded number: a two-level
    // declared channel (nxf 6j6v.pf6j) puts the same task into the requester's board AND into each
    // member's own thread, so "the request" is several messages and how many is the fan-out's
    // business, not this test's. Every one of them is its thread's opening message, so the filter
    // drops NONE of them — which is the claim, exactly stated.
    use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};

    let (tmp, e, thread) = a_board_with_two_members_answers();

    // The store read takes the SCOPE and applies no policy at all (nxf 6j6v.cs03 moved membership
    // out of it too), so this is the board's every matching body — what the filter is measured
    // against.
    let channel = e
        .thread(&thread, "pm")
        .expect("the requester reads it")
        .channel_id;
    let unfiltered = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap()
        .search_messages(&[channel], "please look")
        .expect("the store read applies no policy");
    let found = e.search("qa", "please look").expect("search reads");

    assert!(
        !unfiltered.is_empty(),
        "the fixture has to actually contain the request"
    );
    assert_eq!(
        found.iter().map(|h| h.body.as_str()).collect::<Vec<_>>(),
        unfiltered
            .iter()
            .map(|h| h.body.as_str())
            .collect::<Vec<_>>(),
        "an opening message is the REQUEST and is never filtered — from anybody"
    );
}

// ---- the review's coverage findings on the search filter (PR #437) ---------------------------
//
// The four ways a message reaches a reader are one function now (`facade::visible_under`), and
// `search` establishes its four inputs differently from `thread` — through a per-hit lookup rather
// than a position in an ordered thread. These tests exercise the inputs that lookup has to get
// right, each of which was unexercised when the fix landed.

/// Post a message straight into `channel` from `sender` (origin-qualified by the engine's own
/// origin), optionally stamped with a thread. Direct to the store, like the fixture above and for
/// the same reason: what is under test is the READ.
fn post_into(
    e: &nexus_chat::engine::Engine,
    dir: &std::path::Path,
    channel: &str,
    sender: &str,
    thread: Option<&str>,
    body: &str,
) -> String {
    use nexus_chat::model::MessageEnvelope;
    use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};

    let mut s = Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap();
    s.set_wall_clock(NOW);
    s.post_message(&MessageEnvelope {
        origin: e.origin(),
        channel_id: channel.to_string(),
        sender: format!("{}/{sender}", e.origin()),
        kind: MessageKind::Report,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: thread.map(str::to_string),
        refs: Refs::default(),
        body: body.into(),
    })
}

/// The bodies `as_handle` finds for `query`, through the seam's own search.
fn found(e: &nexus_chat::engine::Engine, as_handle: &str, query: &str) -> Vec<String> {
    e.search(as_handle, query)
        .expect("search reads")
        .into_iter()
        .map(|h| h.body)
        .collect()
}

#[test]
fn a_message_with_no_thread_is_withheld_from_everyone_but_its_author() {
    // THE FAIL-CLOSED BRANCH, and it is the one place a regression would WIDEN the promise this
    // whole ticket exists to keep. `requester_only` is a rule about a BOARD; a message stamped with
    // no thread has none, and `thread` — the reader this one must agree with — cannot serve it at
    // all, being keyed by thread. So it is withheld, and `requester_of`'s own rule says why: an
    // unresolvable requester must never widen a `requester_only` view.
    //
    // Flipping that `else` to `Ok(true)` passed every test this file had before this one.
    let (tmp, e, board) = a_board_with_two_members_answers();
    let channel = e.thread(&board, "pm").unwrap().channel_id;
    post_into(
        &e,
        tmp.path(),
        &channel,
        "dev",
        None,
        "an aside with no board",
    );

    assert_eq!(
        found(&e, "dev", "aside"),
        ["an aside with no board"],
        "its author finds its own words — nobody is kept from those"
    );
    assert!(
        found(&e, "qa", "aside").is_empty(),
        "another member does not: there is no board to be the requester of"
    );
    assert!(
        found(&e, "pm", "aside").is_empty(),
        "and NEITHER DOES THE CHANNEL'S REQUESTER, which is the price of failing closed and is \
         stated here rather than discovered: `requester_only` reserves a ROUND's answers, and this \
         message is in no round"
    );
}

/// The member thread `expects` was opened for, under `board` — the supervisor-opened thread a
/// two-level declared channel gives each member (nxf 6j6v.pf6j). Read through `Engine::status`,
/// the seam's own tree read, rather than reconstructed from the store.
fn member_thread_of(e: &nexus_chat::engine::Engine, board: &str, expects: &str) -> String {
    let report = e
        .status(NOW, nexus_chat::facade::StatusScope::Threads(&[board]))
        .expect("status reads the operation tree");
    report
        .operations
        .iter()
        .flat_map(|o| o.threads.iter())
        .find(|t| {
            t.parent.as_deref() == Some(board) && t.expects.iter().any(|h| h.ends_with(expects))
        })
        .unwrap_or_else(|| panic!("no member thread for {expects} under {board}"))
        .thread_id
        .clone()
}

#[test]
fn the_requester_reads_a_supervisor_opened_member_thread_and_a_peer_does_not() {
    // `requester_for` resolves the requester of a MEMBER thread by inheriting its parent's opener,
    // because the thread's own opener is the reserved supervisor identity — machinery, which can
    // never be a reader (nxf 6j6v.pf6j). `thread` has that path covered; `search` reached it
    // through the same shared helper and nothing pinned it here.
    //
    // The fixture's own answers sit in the REQUESTER's board, whose opener is `pm` directly, so
    // they never exercise the inheritance. This one puts the answer where the fan-out really puts
    // it: in dev's own member thread, opened by the supervisor.
    let (tmp, e, board) = a_board_with_two_members_answers();
    let channel = e.thread(&board, "pm").unwrap().channel_id;
    let dev_thread = member_thread_of(&e, &board, "dev");
    post_into(
        &e,
        tmp.path(),
        &channel,
        "dev",
        Some(&dev_thread),
        "the finding from dev's own thread",
    );

    assert_eq!(
        found(&e, "pm", "finding from dev"),
        ["the finding from dev's own thread"],
        "the requester inherits through the supervisor-opened thread and reads the answer it \
         commissioned"
    );
    assert_eq!(
        found(&e, "dev", "finding from dev"),
        ["the finding from dev's own thread"],
        "its author reads it"
    );
    assert!(
        found(&e, "qa", "finding from dev").is_empty(),
        "a PEER of the same round does not — which is the door 6j6v.yr59 closed and this must not \
         reopen"
    );
}

#[test]
fn the_bare_declared_handle_is_recognized_against_the_qualified_identities_in_the_store() {
    // Every message `sender` and every thread `opener` is a QUALIFIED `origin/handle`; a declared
    // channel's `members:` are BARE, and a caller naming itself the way the declaration does passes
    // the bare form. `same_identity` accepts either shape, and both of the filter's identity
    // questions depend on it — "is this my own message?" and "am I this round's requester?".
    //
    // The tests above already pass bare handles throughout, so this property was covered by
    // accident. Named here so that breaking it fails with the reason rather than as a puzzling
    // change in three unrelated assertions.
    let (_tmp, e, board) = a_board_with_two_members_answers();
    let opener = e.thread(&board, "pm").unwrap().opener;

    assert_eq!(
        opener.as_deref(),
        Some(format!("{}/pm", e.origin()).as_str()),
        "the board's opener is stored QUALIFIED"
    );
    assert_eq!(
        found(&e, "pm", "looked").len(),
        2,
        "…and the BARE `pm` is still recognized as that opener, so it reads the whole board"
    );
    assert_eq!(
        found(&e, "dev", "looked"),
        ["dev looked"],
        "…and the BARE `dev` is still recognized as the author of the qualified sender's message"
    );
}

#[test]
fn two_channels_in_one_search_are_each_judged_under_their_own_declaration() {
    // The per-channel verdict is memoized inside one `search` call. A cache keyed on the wrong
    // thing — or reused across channels — would hand the first channel's declaration to every
    // later hit, which on this fixture would leak `requester_only` content the moment an
    // `all_members` channel is searched first (the channels sort `decl:open` before `decl:review`).
    let (tmp, e, board) = a_board_with_two_members_answers();
    let review = e.thread(&board, "pm").unwrap().channel_id;

    // A second declared channel, `all_members`, materialized the way `ensure_declared_channel`
    // materializes one: the declared members, bare.
    {
        use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};
        let mut s = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.set_channel_field("decl:open", "kind", "group", "pm");
        s.set_channel_field("decl:open", "name", "open", "pm");
        for h in ["pm", "dev", "qa"] {
            s.add_member("decl:open", h, "pm");
        }
    }
    std::fs::write(
        tmp.path().join(".nxs-personas/channels.yaml"),
        "- name: review\n  members: [pm, dev, qa]\n\
         - name: open\n  members: [pm, dev, qa]\n  visibility: all_members\n",
    )
    .unwrap();

    post_into(
        &e,
        tmp.path(),
        "decl:open",
        "dev",
        None,
        "shared word openly",
    );
    // Into the EXISTING board, so this hit is an ordinary reserved reply: not `qa`'s own, not the
    // round it requested, and not the thread's opening message. (Stamping it with a thread of its
    // own would make it that thread's opening message and visible to everyone — correctly, and
    // the first draft of this test did exactly that and proved nothing.)
    post_into(
        &e,
        tmp.path(),
        &review,
        "dev",
        Some(&board),
        "shared word in the reserved round",
    );

    assert_eq!(
        found(&e, "qa", "shared word"),
        ["shared word openly"],
        "the `all_members` channel's hit comes through and the `requester_only` one does not — one \
         call, two declarations, each hit judged under its own"
    );
}

/// The declared catalogue `facade::search` takes, parsed from a real `channels.yaml` through the
/// module's own loader — so the `visibility` under test is the one a declaration really produces,
/// not a value handed in by the test.
fn declared(yaml: &str) -> (tempfile::TempDir, Vec<nexus_chat::channel::ChannelDecl>) {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("channels.yaml"), yaml).unwrap();
    let decls = nexus_chat::channel::load_all_channels(tmp.path()).expect("channels.yaml parses");
    (tmp, decls)
}

#[test]
fn whoever_opened_the_operation_finds_the_answers_it_commissioned_in_a_reserved_round() {
    // `requester_for`'s second door (nxf 6j6v.v39s): the reader who opened the OPERATION is not a
    // peer of the round it is reading — it is the party the whole chain answers to. `thread` has
    // that path covered; `search` reaches it through the same helper and nothing pinned it.
    //
    // The tree: `o/boss` opens the operation in its own channel, `o/lead` commissions a round
    // inside the declared `review` channel under it, and `o/member` answers there. Membership is
    // the boss's own — `search` is membership-scoped and always was, so an operation opener that is
    // not a member of the inner channel finds nothing here whatever the visibility says (that
    // asymmetry with `thread` is nxf 6j6v.cs03, and it is not what this test is about).
    use nexus_chat::model::{MessageEnvelope, ThreadRoot};

    let (_decl_dir, decls) = declared("- name: review\n  members: [boss, lead, member, other]\n");
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("decl:review", "kind", "group", "o/lead");
    for h in ["o/boss", "o/lead", "o/member", "o/other"] {
        s.add_member("decl:review", h, "o/lead");
    }
    s.set_wall_clock(NOW);
    let root = ThreadRoot {
        origin: "o".into(),
        channel_id: "c-outer".into(),
        opener: "o/boss".into(),
        created: NOW.into(),
        parent: None,
    };
    s.open_thread("t-root", &root, "o/boss");
    s.open_thread(
        "t-round",
        &ThreadRoot {
            channel_id: "decl:review".into(),
            opener: "o/lead".into(),
            parent: Some("t-root".into()),
            ..root
        },
        "o/lead",
    );
    let post = |s: &mut ChatStore, sender: &str, body: &str| {
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "decl:review".into(),
            sender: sender.into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-round".into()),
            refs: Refs::default(),
            body: body.into(),
        })
    };
    post(&mut s, "o/lead", "please settle this question");
    post(&mut s, "o/member", "the settled answer");

    let hits = |as_handle: &str| -> Vec<String> {
        // "settle" matches BOTH bodies; the filter is what separates them, not the query.
        nexus_chat::facade::search(&s, as_handle, "settle", &decls)
            .expect("search reads")
            .into_iter()
            .map(|h| h.body)
            .collect()
    };

    assert_eq!(
        hits("o/lead"),
        ["please settle this question", "the settled answer"],
        "the round's own requester reads it all"
    );
    assert_eq!(
        hits("o/boss"),
        ["please settle this question", "the settled answer"],
        "and so does whoever opened the OPERATION, two levels up — it is not a peer of this round"
    );
    assert_eq!(
        hits("o/other"),
        ["please settle this question"],
        "a member that is neither gets the REQUEST and nothing else"
    );
}

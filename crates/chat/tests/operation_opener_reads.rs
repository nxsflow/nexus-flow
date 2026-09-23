//! The two halves of nxf 6j6v.v39s, at the READ layer where both of them decide something.
//!
//! The finding came out of the `nxc` proving ground (4jgn.g90w): a human opened an operation, the
//! chain `ckoch → planning → pm → coding → coder → review` ran to the end, and from the FIRST
//! channel an agent opened along the way the human could see the shape of its own operation and not
//! a word of it. `nxc status` crosses the channel border; the message reader did not.
//!
//! Two independent causes, one per half of the item, and one test group each:
//!
//! 1. **The gate knew nothing about the tree.** `decl:coding` was opened by the pm and `decl:review`
//!    by the coder; neither channel's `members:` names a human, and nothing related those channels
//!    to the operation they belong to. The opener of the operation now reads its whole tree — and
//!    the channel's declared `visibility` still decides what it sees INSIDE a thread, which is the
//!    door nxf 6j6v.yr59 closed and this must not reopen: a PEER member of a `requester_only` round
//!    is as filtered as it ever was.
//! 2. **`members:` was materialised, not read.** The substrate's member list is a snapshot taken the
//!    first time somebody sent to the channel, so an edit to the declaration rendered in `nxc list`
//!    at once and reached the gate only on the next `send`. The file is the membership now, in both
//!    directions.
//!
//! **A THIRD SECTION arrived with nxf 6j6v.cs03**, and it is here rather than in a file of its own
//! for the reason `channel_visibility.rs` keeps px98's section beside yr59's: the rule has TWO
//! readers now, and a rule whose readers are pinned in two files is a rule that can be changed in
//! one of them. `facade::search` answered the membership question out of the substrate's
//! materialised member set while `facade::thread` asked the declaration — so a handle struck from
//! `members:` kept finding bodies through the text search, a handle just written into it found none
//! until the next `send`, and (the shape a user met first) a declared member that had only ever
//! REPLIED found nothing at all, not even its own words, because the declaration materialises a
//! member BARE and a caller arrives QUALIFIED. Both readers decide through ONE rule now —
//! `facade::declared_seat`, which is what `is_channel_member` is built out of.
//!
//! Driven through `nexus_chat::facade` against a real `ChatStore`, the style
//! `channel_visibility.rs` next door established — what is under test is the read, and the fan-out
//! that would open these threads for real is a different question.

use nexus_chat::channel::{ChannelDecl, ChannelPolicy, Visibility};
use nexus_chat::facade::{ask_under, reply, thread, AskRequest, MessageView, ReplyRequest};
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs};
use nexus_chat::store::ChatStore;

const NOW: &str = "2026-08-22T00:00:00Z";

/// The human who opened the operation — a member of `decl:planning` and of nothing below it.
const HUMAN: &str = "o/ckoch";

fn seed() -> ChatStore {
    let mut s = ChatStore::open_in_memory(1);
    // `decl:planning` — the human's own channel, opened by the human.
    s.set_channel_field("decl:planning", "kind", "group", HUMAN);
    s.set_channel_field("decl:planning", "name", "planning", HUMAN);
    s.add_member("decl:planning", "pm", HUMAN);
    s.add_member("decl:planning", HUMAN, HUMAN);
    // `decl:coding` — opened by the PM, exactly as the proving ground's was: it materialises its
    // declared member and its own author, and the human is neither.
    s.set_channel_field("decl:coding", "kind", "group", "o/pm");
    s.set_channel_field("decl:coding", "name", "coding", "o/pm");
    s.add_member("decl:coding", "coder", "o/pm");
    s.add_member("decl:coding", "o/pm", "o/pm");
    s
}

/// The proving ground's chain, two channels deep: the human's root thread in `decl:planning`, and
/// under it the board the PM opened in `decl:coding`. Returns `(root, child)`.
fn two_channel_operation(s: &mut ChatStore) -> (String, String) {
    let root = ask_under(
        s,
        AskRequest {
            now: NOW,
            origin: "o",
            actor: "ckoch",
            channel: "decl:planning",
            body: "here is the product idea",
            expect: &["pm".to_string()],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
        None,
    )
    .unwrap()
    .thread_id;
    let child = ask_under(
        s,
        AskRequest {
            now: NOW,
            origin: "o",
            actor: "pm",
            channel: "decl:coding",
            body: "build the scaffolding",
            expect: &["coder".to_string()],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
        Some(&root),
    )
    .unwrap()
    .thread_id;
    (root, child)
}

fn reply_from(s: &mut ChatStore, thread_id: &str, actor: &str, body: &str) {
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
    .expect("if_unanswered is false, so this always posts");
}

fn bodies(messages: &[MessageView]) -> Vec<&str> {
    messages.iter().map(|m| m.body.as_str()).collect()
}

/// The declaration catalogue `facade::search` takes, declaring `decl:coding` with whatever
/// `members:` the test is asserting about.
///
/// Through the REAL declaration path — parsed `channels.yaml` text — so what these tests hold is
/// what an adapter actually produces, not a hand-built value. It is the ONE source both readers
/// under test are handed: [`coding_policy`] derives `thread`'s parameter from exactly this, so no
/// test here can hand the two readers two different declarations and call them agreed.
fn coding_decls(members: &[&str], visibility: Visibility) -> Vec<ChannelDecl> {
    let visibility = match visibility {
        Visibility::RequesterOnly => "requester_only",
        Visibility::AllMembers => "all_members",
    };
    let yaml = format!(
        "- name: coding\n  members: [{}]\n  visibility: {visibility}\n",
        members.join(", ")
    );
    serde_yaml::from_str(&yaml).expect("the declaration parses")
}

/// The declared policy for `decl:coding` as the adapters resolve it — [`coding_decls`] through the
/// same `declared_policy` reverse lookup `nxc threads show` and `Engine::thread` resolve through.
fn coding_policy(members: &[&str], visibility: Visibility) -> ChannelPolicy {
    nexus_chat::channel::declared_policy(&coding_decls(members, visibility), Some("decl:coding"))
}

// ---- 1. the operation's opener reads its whole tree -------------------------------------------

#[test]
fn the_operation_opener_reads_a_thread_in_a_channel_an_agent_opened_below_it() {
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");

    // Before this item the human got `forbidden` here — `nxc status` showed the thread, `nxc
    // threads show` refused it, and the only door that worked was `nxc transcript show`.
    let view = thread(
        &s,
        &child,
        HUMAN,
        coding_policy(&["coder"], Visibility::AllMembers),
    )
    .expect("whoever opened the operation reads the tree `nxc status` shows them");
    assert_eq!(
        bodies(&view.messages),
        vec!["build the scaffolding", "the scaffolding is up"],
    );
}

#[test]
fn a_reader_who_opened_no_operation_here_is_still_refused() {
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);

    let err = thread(
        &s,
        &child,
        "o/stranger",
        coding_policy(&["coder"], Visibility::AllMembers),
    )
    .expect_err("the tree door opens for the operation's opener, not for anyone who asks");
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Forbidden);
}

#[test]
fn an_ancestry_this_workspace_has_not_folded_yet_does_not_open_the_door() {
    // Ops arrive in relay order, so a child can be here before its parent. The walk then ends at a
    // thread with no `threads` row, which has no opener — and a reader must not be handed a subtree
    // because half of its chain is still in flight.
    let mut s = seed();
    let orphan = ask_under(
        &mut s,
        AskRequest {
            now: NOW,
            origin: "o",
            actor: "pm",
            channel: "decl:coding",
            body: "build the scaffolding",
            expect: &["coder".to_string()],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
        Some("m-01M0NOTHINGHEREYETXXXXXXXX"),
    )
    .unwrap()
    .thread_id;

    let err = thread(
        &s,
        &orphan,
        HUMAN,
        coding_policy(&["coder"], Visibility::AllMembers),
    )
    .expect_err("an unfolded parent is fail-closed, not fail-open");
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Forbidden);
}

#[test]
fn requester_only_still_hides_a_peer_members_reply_from_another_member() {
    // The door nxf 6j6v.yr59 closed, held shut: the operation opener's new standing is about the
    // opener of the TREE, and gives no member of a round any sight of its neighbour's answer.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    s.add_member("decl:coding", "reviewer", "o/pm");
    reply_from(&mut s, &child, "coder", "the scaffolding is up");

    let view = thread(
        &s,
        &child,
        "reviewer",
        coding_policy(&["coder", "reviewer"], Visibility::RequesterOnly),
    )
    .expect("a declared member reads the board");
    assert_eq!(
        bodies(&view.messages),
        vec!["build the scaffolding"],
        "a peer sees the request and its own replies, never another member's answer"
    );
}

#[test]
fn requester_only_does_not_stand_between_the_operations_opener_and_the_answers_it_commissioned() {
    // `requester_only` is a rule BETWEEN the participants of one round. Whoever opened the
    // operation is not in the round: it is the party the whole chain answers to.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");

    let view = thread(
        &s,
        &child,
        HUMAN,
        coding_policy(&["coder"], Visibility::RequesterOnly),
    )
    .expect("the operation's opener reads its own tree");
    assert_eq!(
        bodies(&view.messages),
        vec!["build the scaffolding", "the scaffolding is up"],
    );
}

// ---- 2. `members:` is the membership, read from the file ---------------------------------------

#[test]
fn adding_a_handle_to_declared_members_reaches_the_gate_with_no_send_in_between() {
    // The proving ground's second half: `members: [coder, ckoch]` rendered in `nxc list` at once and
    // the door kept using the snapshot taken when the PM first sent — so the file said one thing and
    // the gate did another, and only the file was visible.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");
    assert!(
        !s.is_member("decl:coding", "outsider").unwrap(),
        "nothing has materialised this handle — that is the whole point of the case"
    );

    thread(
        &s,
        &child,
        "outsider",
        coding_policy(&["coder", "outsider"], Visibility::AllMembers),
    )
    .expect("the declaration IS the membership, with no `send` needed to materialise the edit");
}

#[test]
fn a_handle_the_declaration_no_longer_names_loses_the_gate_although_the_store_still_lists_it() {
    // The other direction, and the one that makes "the file and the door agree" true rather than
    // half-true: the substrate's member set is grow-only, so a handle removed from `members:` stays
    // in it forever. For a DECLARED channel the file decides.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    assert!(
        s.is_member("decl:coding", "coder").unwrap(),
        "the store still carries the materialised seat"
    );

    let err = thread(
        &s,
        &child,
        "coder",
        coding_policy(&["someone-else"], Visibility::AllMembers),
    )
    .expect_err("a seat the declaration no longer names is not a seat");
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Forbidden);
}

#[test]
fn a_declared_member_reads_its_own_channel_under_its_qualified_identity() {
    // The property the gate's own comparison rests on, pinned so it is deliberate rather than
    // accidental (raised by the independent review of this branch, which read the doc comment and
    // correctly found it describing a DIFFERENT rule than the code).
    //
    // `members:` names personas BARE; a caller's own identity is always QUALIFIED
    // (`cli.rs`'s `caller_handle` is `<origin>/<actor>`). A bare-only comparison would therefore
    // refuse `o/coder` from the very channel `members: [coder]` declares it into — and the store
    // cannot rescue it either, because `ensure_declared_channel` materialises declared members BARE.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");
    assert!(
        !s.is_member("decl:coding", "o/coder").unwrap(),
        "the store carries the BARE seat only — which is exactly why the comparison cannot be \
         a bare-only one"
    );

    thread(
        &s,
        &child,
        "o/coder",
        coding_policy(&["coder"], Visibility::AllMembers),
    )
    .expect("a declared member reads the channel it is declared into, under its own identity");
}

#[test]
fn a_board_opener_keeps_its_own_channel_although_no_declaration_names_it() {
    // The bare/qualified split `ChannelPolicy` rests on: `members:` names PERSONAS, so a qualified
    // `origin/handle` identity that opened a board here is not a declared seat and no
    // `channels.yaml` can revoke it. Without this the PM would lose the channel it opened itself.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");

    thread(
        &s,
        &child,
        "o/pm",
        coding_policy(&["coder"], Visibility::AllMembers),
    )
    .expect("whoever opened the board reads it back, declared member or not");
}

#[test]
fn an_undeclared_channel_is_still_answered_by_the_store_alone() {
    // No declaration names it, so there is nothing to read and the behaviour is what it always was.
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("c-1", "kind", "group", "o/a");
    s.add_member("c-1", "o/a", "o/a");
    let thread_id = ask_under(
        &mut s,
        AskRequest {
            now: NOW,
            origin: "o",
            actor: "a",
            channel: "c-1",
            body: "hello",
            expect: &[],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
        None,
    )
    .unwrap()
    .thread_id;

    thread(&s, &thread_id, "o/a", ChannelPolicy::undeclared()).expect("a member reads it");
    let err = thread(&s, &thread_id, "o/b", ChannelPolicy::undeclared())
        .expect_err("a non-member does not");
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Forbidden);
}

// ---- 3. and the SECOND reader answers the same membership question (nxf 6j6v.cs03) -------------
//
// `facade::search` is the other reader on this seam. It was membership-SCOPED through the
// substrate's materialised member set — `membership_adds` minus `membership_removes`, which
// `ensure_declared_channel` only ever ADDS to — while `thread` above asks the declaration. Same
// question, two answers. The fixture is the one the two sections above share, so what these hold is
// the same rule over the same store, asked through the other verb.

/// Bodies matching `q` in `decl:coding` as `handle` finds them — [`coding_decls`] is the catalogue,
/// the same declaration [`coding_policy`] hands `thread`.
fn found(
    s: &ChatStore,
    handle: &str,
    q: &str,
    members: &[&str],
    visibility: Visibility,
) -> Vec<String> {
    nexus_chat::facade::search(s, handle, q, &coding_decls(members, visibility))
        .expect("search reads")
        .into_iter()
        .map(|h| h.body)
        .collect()
}

#[test]
fn a_declared_member_finds_its_own_words_under_its_qualified_identity() {
    // MEASURED IN A LIVE `nxc` RUN, not deduced: on a declared channel `nxc search` answered "no
    // matches" to every member that had not itself opened a round there — including for that
    // member's own words. `resolve_consumer` falls back to the caller's QUALIFIED handle
    // (`origin/actor`), `ensure_declared_channel` records a declared channel's `members:` BARE and
    // additionally the AUTHOR of the send qualified — so the sender searched its board and a member
    // that had only ever REPLIED matched no membership row at all. `thread` never had this: its
    // comparison goes through `same_identity`, which accepts either shape.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");
    assert!(
        !s.is_member("decl:coding", "o/coder").unwrap(),
        "the store carries the BARE seat only — which is the whole of this case"
    );

    assert_eq!(
        found(
            &s,
            "o/coder",
            "scaffolding",
            &["coder"],
            Visibility::AllMembers
        ),
        ["build the scaffolding", "the scaffolding is up"],
        "a declared member finds the channel it is declared into, under the identity the surface \
         gives it — its own reply included"
    );
}

#[test]
fn adding_a_handle_to_declared_members_reaches_the_text_search_with_no_send_in_between() {
    // The counterpart of `adding_a_handle_to_declared_members_reaches_the_gate_…` above, one verb
    // over: an edit to `members:` reached `thread` at once and `search` only when the next `send`
    // materialised it, so the file and this reader disagreed for as long as nobody sent.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");
    assert!(
        !s.is_member("decl:coding", "outsider").unwrap(),
        "nothing has materialised this handle — that is the whole point of the case"
    );

    assert_eq!(
        found(
            &s,
            "outsider",
            "scaffolding",
            &["coder", "outsider"],
            Visibility::AllMembers
        ),
        ["build the scaffolding", "the scaffolding is up"],
        "the declaration IS the membership for this reader too, with no `send` in between"
    );
}

#[test]
fn a_handle_the_declaration_no_longer_names_stops_finding_bodies_although_the_store_still_lists_it()
{
    // The confidentiality direction, and the same promise nxf 6j6v.px98 was about: the substrate's
    // member set is grow-only, so a handle struck from `members:` stays in it forever and kept
    // finding bodies here after `thread` had stopped serving them.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");
    assert!(
        s.is_member("decl:coding", "coder").unwrap(),
        "the store still carries the materialised seat"
    );

    assert!(
        found(
            &s,
            "coder",
            "scaffolding",
            &["someone-else"],
            Visibility::AllMembers
        )
        .is_empty(),
        "a seat the declaration no longer names is not a seat, whichever reader is asked"
    );
}

#[test]
fn the_two_readers_agree_about_membership_in_every_shape_a_reader_arrives_in() {
    // The invariant itself, over the six shapes the three tests above and the four in section 2 are
    // each one case of: `thread` serving the board and `search` finding a body in it are ONE
    // answer. Stated as a differential rather than as six more expectations, because the defect was
    // never one reader being wrong on its own — it was the two of them disagreeing.
    //
    // The query matches the board's own two bodies and the declaration says `all_members`, so
    // `visibility` — a different rule, pinned in `channel_visibility.rs` — cannot decide anything
    // here: whoever is a member finds both bodies, and a non-member finds none.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");

    for (handle, members, expected, why) in [
        (
            "coder",
            &["coder"][..],
            true,
            "the bare declared seat, the shape a declaration materialises",
        ),
        (
            "o/coder",
            &["coder"][..],
            true,
            "the same seat under the qualified identity the surface hands a caller",
        ),
        (
            "outsider",
            &["coder", "outsider"][..],
            true,
            "written into the file and nothing sent since",
        ),
        (
            "coder",
            &["someone-else"][..],
            false,
            "struck from the file and still materialised in the store",
        ),
        (
            "o/pm",
            &["coder"][..],
            true,
            "the board's own opener, which no declaration names and none may revoke",
        ),
        (
            "o/stranger",
            &["coder"][..],
            false,
            "neither declared nor materialised nor the opener of anything",
        ),
    ] {
        let policy = coding_policy(members, Visibility::AllMembers);
        let serves = thread(&s, &child, handle, policy).is_ok();
        let finds = !found(&s, handle, "scaffolding", members, Visibility::AllMembers).is_empty();
        assert_eq!(
            (serves, finds),
            (expected, expected),
            "{handle} under members {members:?} — {why}: `thread` serves={serves}, \
             `search` finds={finds}"
        );
    }
}

#[test]
fn the_text_search_does_not_follow_an_operation_across_a_channel_border() {
    // Direction (3) of nxf 6j6v.cs03, DECIDED and deliberately not built — pinned as a test rather
    // than left as prose, on the independent review's Test Quality #2 and on this project's own
    // history: px98 named its residual in a doc comment and that residual became cs03. A later
    // reader who "completes" the fix by copying `require_thread_readable`'s operation door into
    // `searchable_channels` would widen a body-substring read across channel borders — a
    // confidentiality change, not a tidy-up — and this is the red test that stops them. An
    // operation-wide text search is a capability question and belongs to the owner.
    let mut s = seed();
    let (_root, child) = two_channel_operation(&mut s);
    reply_from(&mut s, &child, "coder", "the scaffolding is up");
    assert!(
        !s.is_member("decl:coding", HUMAN).unwrap(),
        "the operation's opener is not a member of the channel its agent opened below it — that is \
         the whole shape of the case"
    );

    // `thread` DOES open that door, and must keep doing so (6j6v.v39s): the human at the top of the
    // chain reads the round it commissioned, whatever channel border it sits behind.
    thread(
        &s,
        &child,
        HUMAN,
        coding_policy(&["coder"], Visibility::AllMembers),
    )
    .expect("the operation's opener reads its whole tree");

    assert!(
        found(&s, HUMAN, "scaffolding", &["coder"], Visibility::AllMembers).is_empty(),
        "and `search` does not follow it: this read stays inside the channels the caller is a member \
         of, which is the decision recorded on the item and in `nxc guide commands`"
    );

    // And the reader is not simply blind: the SAME call finds its OWN channel's words. So what the
    // assertion above pins is the channel border and not a read that answers nobody.
    assert_eq!(
        found(
            &s,
            HUMAN,
            "product idea",
            &["coder"],
            Visibility::AllMembers
        ),
        ["here is the product idea"],
        "the channel it IS a member of answers exactly as it always did"
    );
}

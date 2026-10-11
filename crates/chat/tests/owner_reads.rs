//! nxf 6j6v.6gpr — a hosted replica reads as the board's OWNER, at the embedding seam.
//!
//! Measured in staging (app-foundations 41j0.hnpn): the web chat listed the operations and every
//! thread it opened answered `forbidden` — "not a member of this conversation, and not the opener
//! of its operation" — because the hosted replica read as its own throwaway prefix plus the server
//! process's actor, which is never a member and never an opener. Nor could a better handle have
//! fixed it: an operation opened in a terminal carries a machine-minted `<origin>/<actor>` no
//! signed-in account knows.
//!
//! The owner's decision (2026-10-05): **a board has an owner, and the owner reads everything** — the
//! account signed in as the board's owner reads as if it had opened every operation on it; any other
//! account sees no conversations at all. The hosted engine decides WHO the owner is; this crate
//! decides what reading as the owner MEANS, through `facade::Reader`. These tests hold that meaning
//! where an embedder meets it — on `Engine` — for every reader-keyed read:
//!
//! * the owner reads every thread of every operation exactly as that operation's opener does —
//!   unfiltered, across channels it is a member of nowhere;
//! * the owner searches every channel the board holds;
//! * the owner's `status` and `directory` are the same caller-free records any reader gets;
//! * an account that is not the owner is REFUSED on every one of them — `forbidden`, never an empty
//!   answer, and before `not_found`, so asking reveals nothing;
//! * nothing on the local surface can read as the owner, and a chat identity reads exactly as it did.

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::{
    ask_under, reply, AskRequest, MessageView, Reader, ReplyRequest, StatusScope,
};
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-10-11T09:00:00Z";

/// The opener of the operation as a terminal mints it — an `<origin>/<actor>` that no signed-in
/// account knows, which is the half of the finding a better replica handle could never have fixed.
const TERMINAL_OPENER: &str = "4jgn/ckoch";

/// A workspace declaring the two channels of a two-channel operation — `planning` (the human's,
/// with the pm) and `review` (the pm's round, `requester_only` by default) — plus the personas they
/// name. THREE members in `review`, so the filter can be told apart from "sees its own words".
fn declared_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
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
        "- name: planning\n  members: [pm]\n- name: review\n  members: [pm, dev, qa]\n",
    )
    .unwrap();
    tmp
}

fn ask(
    s: &mut nexus_chat::store::ChatStore,
    actor: &str,
    channel: &str,
    body: &str,
    expect: &[&str],
    parent: Option<&str>,
) -> String {
    let expect: Vec<String> = expect.iter().map(|h| h.to_string()).collect();
    ask_under(
        s,
        AskRequest {
            now: NOW,
            origin: "4jgn",
            actor,
            channel,
            body,
            expect: &expect,
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
        parent,
    )
    .unwrap()
    .thread_id
}

fn answer(s: &mut nexus_chat::store::ChatStore, thread_id: &str, actor: &str, body: &str) {
    reply(
        s,
        ReplyRequest {
            now: NOW,
            origin: "4jgn",
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

/// The board as a hosted replica finds it: an operation a human opened in a terminal, two channels
/// deep — the human's root thread in `decl:planning`, and under it the `requester_only` review round
/// the pm commissioned in `decl:review`, which both reviewers have answered. Returns the open
/// engine (over the same workspace, as an embedder holds it), its temp dir and `(root, round)`.
fn hosted_board() -> (TempDir, Engine, String, String) {
    let tmp = declared_workspace();
    let (root, round) = {
        let mut s = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.set_wall_clock(NOW);
        s.set_channel_field("decl:planning", "kind", "group", TERMINAL_OPENER);
        s.set_channel_field("decl:planning", "name", "planning", TERMINAL_OPENER);
        s.add_member("decl:planning", "pm", TERMINAL_OPENER);
        s.add_member("decl:planning", TERMINAL_OPENER, TERMINAL_OPENER);
        s.set_channel_field("decl:review", "kind", "group", "4jgn/pm");
        s.set_channel_field("decl:review", "name", "review", "4jgn/pm");
        for member in ["pm", "dev", "qa"] {
            s.add_member("decl:review", member, "4jgn/pm");
        }
        let root = ask(
            &mut s,
            "ckoch",
            "decl:planning",
            "ship the release",
            &["pm"],
            None,
        );
        let round = ask(
            &mut s,
            "pm",
            "decl:review",
            "review the release",
            &["dev", "qa"],
            Some(&root),
        );
        answer(&mut s, &round, "dev", "dev looked: fine");
        answer(&mut s, &round, "qa", "qa looked: one nit");
        (root, round)
    };
    let engine = open_engine(tmp.path());
    (tmp, engine, root, round)
}

fn open_engine(dir: &Path) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

/// What the hosted replica read as before this item: its own prefix plus the server's actor.
fn replica_handle(e: &Engine) -> String {
    format!("{}/engine-server", e.origin())
}

fn bodies(messages: &[MessageView]) -> Vec<&str> {
    messages.iter().map(|m| m.body.as_str()).collect()
}

fn refused<T: std::fmt::Debug>(result: nexus_chat::error::Result<T>) -> ErrorKind {
    result
        .expect_err("a reader that is not the owner is refused")
        .kind
}

// ---- thread: the owner reads every operation as its opener ------------------------------------

#[test]
fn the_replica_reading_as_itself_is_refused_which_is_the_measured_finding() {
    let (_tmp, e, root, round) = hosted_board();
    let replica = replica_handle(&e);
    for thread in [&root, &round] {
        assert_eq!(
            refused(e.thread(thread, Reader::Handle(&replica))),
            ErrorKind::Forbidden,
            "the replica's own identity is a member of nothing and opened nothing — the 403 the \
             web chat met, and still the answer for that identity: a chat identity reads under the \
             local rules, unchanged"
        );
    }
}

#[test]
fn the_owner_reads_every_thread_of_the_operation_exactly_as_its_opener_does() {
    let (_tmp, e, root, round) = hosted_board();

    for thread in [&root, &round] {
        let as_owner = e
            .thread(thread, Reader::Owner)
            .expect("the owner reads every thread on its board");
        let as_opener = e
            .thread(thread, Reader::Handle(TERMINAL_OPENER))
            .expect("the opener of the operation reads its whole tree");
        assert_eq!(
            as_owner, as_opener,
            "reading as the owner IS reading as if it had opened the operation — no more, no less"
        );
    }
    let round_view = e.thread(&round, Reader::Owner).unwrap();
    assert_eq!(
        bodies(&round_view.messages),
        [
            "review the release",
            "dev looked: fine",
            "qa looked: one nit"
        ],
        "a `requester_only` round reads unfiltered for the owner, because the opener of the \
         operation is the requester of every round in it"
    );
    assert_eq!(round_view.channel_id, "decl:review");
}

#[test]
fn a_peer_of_the_round_is_still_filtered_because_the_local_rules_did_not_move() {
    let (_tmp, e, _root, round) = hosted_board();
    let peer = e
        .thread(&round, Reader::Handle("dev"))
        .expect("a member reads the round it takes part in");
    assert_eq!(
        bodies(&peer.messages),
        ["review the release", "dev looked: fine"],
        "owner mode widened nothing for a chat identity: the peer rule of `requester_only` holds"
    );
}

#[test]
fn an_unknown_thread_is_not_found_for_the_owner() {
    let (_tmp, e, _root, _round) = hosted_board();
    assert_eq!(
        refused(e.thread("m-no-such-thread", Reader::Owner)),
        ErrorKind::NotFound,
        "the owner reads everything that exists, and is told so when it does not"
    );
}

// ---- search: the owner searches every channel the board holds ---------------------------------

#[test]
fn the_owner_searches_every_channel_including_one_this_replica_holds_no_row_for() {
    let (tmp, e, _root, _round) = hosted_board();
    // A message that arrived by sync into a channel this replica never materialised — no channel
    // row, no membership: "the owner reads everything" is the LOG, not the channel list.
    {
        let mut s = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.set_wall_clock(NOW);
        s.post_message(&MessageEnvelope {
            origin: "far".into(),
            channel_id: "c-arrived-by-sync".into(),
            sender: "far/coder".into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: "looked at the sync side".into(),
        });
    }

    let mut found: Vec<String> = e
        .search(Reader::Owner, "looked")
        .expect("the owner searches the board")
        .into_iter()
        .map(|h| h.body)
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            "dev looked: fine",
            "looked at the sync side",
            "qa looked: one nit"
        ],
        "every channel, unfiltered — across the border of a channel the owner is a member of nowhere"
    );

    assert!(
        e.search(Reader::Handle(&replica_handle(&e)), "looked")
            .expect("a chat identity searches its own channels")
            .is_empty(),
        "and a chat identity still searches only its own channels — none, for the replica"
    );
}

// ---- status and directory: admitted, never projected ------------------------------------------

#[test]
fn the_owner_reads_the_same_status_and_directory_as_any_admitted_reader() {
    let (_tmp, e, root, _round) = hosted_board();
    let replica = replica_handle(&e);
    for scope in [
        StatusScope::Workspace,
        StatusScope::All(None),
        StatusScope::Threads(&[root.as_str()]),
    ] {
        assert_eq!(
            e.status(Reader::Owner, NOW, scope).unwrap().to_value(),
            e.status(Reader::Handle(&replica), NOW, scope)
                .unwrap()
                .to_value(),
            "status is caller-free: the reader admits, it never projects ({scope:?})"
        );
    }
    let as_owner = e
        .status(Reader::Owner, NOW, StatusScope::All(None))
        .unwrap();
    assert_eq!(
        as_owner.operations.len(),
        1,
        "the owner sees the operation: {as_owner:?}"
    );

    assert_eq!(
        e.directory(Reader::Owner, None).unwrap(),
        e.directory(Reader::Handle(&replica), None).unwrap(),
        "the directory is the same record for the owner"
    );
}

// ---- the refusal: any other account sees no conversations at all ------------------------------

#[test]
fn an_account_that_is_not_the_owner_is_refused_on_every_read() {
    let (_tmp, e, root, round) = hosted_board();

    for thread in [root.as_str(), round.as_str(), "m-no-such-thread"] {
        assert_eq!(
            refused(e.thread(thread, Reader::NotOwner)),
            ErrorKind::Forbidden,
            "`thread` refuses — and before `not_found`, so a non-owner cannot probe which ids \
             exist ({thread})"
        );
    }
    assert_eq!(
        refused(e.search(Reader::NotOwner, "looked")),
        ErrorKind::Forbidden,
        "`search` refuses rather than answering an empty list, which would read as 'nothing here'"
    );
    for scope in [
        StatusScope::Workspace,
        StatusScope::All(None),
        StatusScope::Channel("planning"),
        StatusScope::Threads(&[root.as_str()]),
        StatusScope::Threads(&["m-no-such-thread"]),
    ] {
        assert_eq!(
            refused(e.status(Reader::NotOwner, NOW, scope)),
            ErrorKind::Forbidden,
            "`status` refuses in every form ({scope:?})"
        );
    }
    assert_eq!(
        refused(e.directory(Reader::NotOwner, None)),
        ErrorKind::Forbidden,
        "`directory` refuses"
    );
    assert_eq!(
        refused(e.directory(Reader::NotOwner, Some("pm"))),
        ErrorKind::Forbidden,
        "`directory` refuses for a named audience too"
    );
}

#[test]
fn the_refusal_says_why_in_words_an_app_can_show() {
    let (_tmp, e, root, _round) = hosted_board();
    let err = e.thread(&root, Reader::NotOwner).unwrap_err();
    assert!(
        err.msg.contains("not the owner"),
        "the refusal names the reason: {}",
        err.msg
    );
}

// ---- the local surface cannot read as the owner -----------------------------------------------

/// The owner reader is the EMBEDDER's to select: no `nxc` flag, variable or declaration may produce
/// it, or a local caller could escalate itself past every gate. Held at the source — the only files
/// that may name the owner variants are the two that define and apply the rule.
#[test]
fn nothing_on_the_local_surface_can_read_as_the_owner() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&src).expect("the crate's sources are readable") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name == "facade.rs" || name == "engine.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for variant in ["Reader::Owner", "Reader::NotOwner", "Standing::Owner"] {
            if text.contains(variant) {
                offenders.push(format!("{name} names {variant}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "only an embedder selects the owner reader; the local surface reads as a chat identity: \
         {offenders:?}"
    );
    let cli = std::fs::read_to_string(src.join("cli.rs")).expect("cli.rs reads");
    assert!(
        cli.contains("Reader::Handle("),
        "the CLI's reads name their reader — as a chat identity — rather than going around the rule"
    );
}

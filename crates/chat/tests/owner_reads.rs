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
//! * the owner's `status`, `directory`, `prime_as`, `persona_prime`, `transcript_page` and
//!   `held_deliveries` are the same records any admitted reader gets;
//! * an account that is not the owner is REFUSED on every one of them — `forbidden`, never an empty
//!   answer, and before `not_found` and before any declaration is read, so asking reveals nothing;
//! * the owner reads no declaration, so a malformed one cannot refuse it a read it never consults;
//! * `facade::thread` / `facade::search` hold the same rule below the engine;
//! * nothing on the local surface can read as the owner, and a chat identity reads exactly as it did.

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::{
    self, ask_under, reply, AskRequest, MessageView, Reader, ReplyRequest, StatusScope,
};
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs};
use nexus_chat::timer::TimerConfig;
use nexus_chat::transcript::TranscriptEntry;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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

#[test]
fn the_owner_reads_a_thread_in_a_channel_no_declaration_names() {
    // The hosted default: a replica without the repository's `channels.yaml` holds channels no
    // declaration names, which fall back to `all_members`. The owner reads them as their opener.
    let (tmp, e, _root, _round) = hosted_board();
    let side = {
        let mut s = open_store(tmp.path());
        s.set_wall_clock(NOW);
        s.set_channel_field("c-side", "kind", "group", TERMINAL_OPENER);
        s.add_member("c-side", TERMINAL_OPENER, TERMINAL_OPENER);
        s.add_member("c-side", "4jgn/pm", TERMINAL_OPENER);
        let side = ask(&mut s, "ckoch", "c-side", "a side question", &["pm"], None);
        answer(&mut s, &side, "pm", "a side answer");
        side
    };
    let as_owner = e
        .thread(&side, Reader::Owner)
        .expect("the owner reads a thread in an undeclared channel");
    assert_eq!(as_owner.channel_id, "c-side");
    assert_eq!(
        bodies(&as_owner.messages),
        ["a side question", "a side answer"]
    );
    assert_eq!(
        as_owner,
        e.thread(&side, Reader::Handle(TERMINAL_OPENER)).unwrap(),
        "exactly the opener's view"
    );
    assert_eq!(
        refused(e.thread(&side, Reader::Handle(&replica_handle(&e)))),
        ErrorKind::Forbidden,
        "and the replica's own identity is still a member of nothing"
    );
    assert_eq!(
        refused(e.thread(&side, Reader::NotOwner)),
        ErrorKind::Forbidden
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

#[test]
fn the_owner_finds_a_threadless_hit_in_a_requester_only_channel_that_a_peer_does_not() {
    // A message stamped with NO thread has no board to judge it against, so a chat identity that
    // did not write it never finds it in a `requester_only` channel (the closed direction). The
    // owner reads the log, so it finds it: the one place the owner's search differs from the
    // opener's view in a channel the opener is a member of — deliberately, "the owner reads
    // everything".
    let (tmp, e, _root, _round) = hosted_board();
    {
        let mut s = open_store(tmp.path());
        s.set_wall_clock(NOW);
        s.post_message(&MessageEnvelope {
            origin: "4jgn".into(),
            channel_id: "decl:review".into(),
            sender: "4jgn/dev".into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: "looked outside any thread".into(),
        });
    }
    let search = |reader| -> Vec<String> {
        let mut found: Vec<String> = e
            .search(reader, "looked")
            .unwrap()
            .into_iter()
            .map(|h| h.body)
            .collect();
        found.sort();
        found
    };
    assert_eq!(
        search(Reader::Owner),
        [
            "dev looked: fine",
            "looked outside any thread",
            "qa looked: one nit"
        ]
    );
    assert_eq!(
        search(Reader::Handle("qa")),
        ["qa looked: one nit"],
        "a peer finds neither its neighbour's answer nor the thread-less line it did not write"
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
        StatusScope::All(Some("planning")),
        StatusScope::All(Some("review")),
        StatusScope::Channel("planning"),
        StatusScope::Channel("review"),
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

    for persona in [None, Some("pm"), Some("dev")] {
        assert_eq!(
            e.directory(Reader::Owner, persona).unwrap(),
            e.directory(Reader::Handle(&replica), persona).unwrap(),
            "the directory is the same record for the owner ({persona:?})"
        );
    }
}

#[test]
fn the_owner_reads_the_same_session_start_transcript_and_held_queue_as_any_admitted_reader() {
    let (_tmp, e, _root, _round) = hosted_board();
    let replica = replica_handle(&e);
    let consumer = format!("{}/pm", e.origin());

    assert_eq!(
        e.prime_as(Reader::Owner, &consumer, Some("pm"), NOW)
            .unwrap(),
        e.prime_as(Reader::Handle(&replica), &consumer, Some("pm"), NOW)
            .unwrap(),
        "`prime_as` is admitted by the reader and decided by `consumer`"
    );
    assert_eq!(
        e.persona_prime(Reader::Owner, "pm", NOW).unwrap(),
        e.persona_prime(Reader::Handle(&replica), "pm", NOW)
            .unwrap(),
        "`persona_prime` is the same text for the owner"
    );

    let entry = TranscriptEntry {
        kind: "assistant".into(),
        at: Some(NOW.into()),
        tool_use_id: None,
        parent_tool_use_id: None,
        subagent_type: None,
        data: serde_json::json!({ "text": "read the diff" }),
    };
    assert_eq!(e.transcript_append("s-owner", &[entry]).unwrap(), 1);
    let as_owner = e
        .transcript_page(Reader::Owner, "s-owner", -1, None)
        .unwrap();
    assert_eq!(
        as_owner.entries.len(),
        1,
        "the owner reads the session: {as_owner:?}"
    );
    assert_eq!(
        as_owner,
        e.transcript_page(Reader::Handle(&replica), "s-owner", -1, None)
            .unwrap(),
        "`transcript_page` is the same window for the owner"
    );

    assert_eq!(
        e.held_deliveries(Reader::Owner, "s-owner").unwrap(),
        e.held_deliveries(Reader::Handle(&replica), "s-owner")
            .unwrap(),
        "`held_deliveries` is the same queue for the owner"
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
    let consumer = format!("{}/pm", e.origin());
    for persona in [None, Some("pm")] {
        assert_eq!(
            refused(e.prime_as(Reader::NotOwner, &consumer, persona, NOW)),
            ErrorKind::Forbidden,
            "`prime_as` refuses — it carries the directory `directory` refuses ({persona:?})"
        );
    }
    for persona in ["pm", "nobody-declared"] {
        assert_eq!(
            refused(e.persona_prime(Reader::NotOwner, persona, NOW)),
            ErrorKind::Forbidden,
            "`persona_prime` refuses, before `not_found` for an undeclared persona ({persona})"
        );
    }
    assert_eq!(
        refused(e.transcript_page(Reader::NotOwner, "s-any", -1, None)),
        ErrorKind::Forbidden,
        "`transcript_page` refuses — raw tool inputs and results are conversation"
    );
    assert_eq!(
        refused(e.held_deliveries(Reader::NotOwner, "s-any")),
        ErrorKind::Forbidden,
        "`held_deliveries` refuses — every held row carries an answer's body"
    );
}

// ---- the declarations: refused before they are read, and never read for the owner -------------

/// Break the workspace's `channels.yaml` so that LOADING it fails — the probe for "the reader is
/// checked before any declaration is read".
fn break_the_declarations(dir: &Path) {
    std::fs::write(
        dir.join(".nxs-personas").join("channels.yaml"),
        "- name: planning\n  members: [pm\n",
    )
    .unwrap();
}

#[test]
fn a_malformed_declaration_cannot_turn_a_refusal_into_a_parse_error() {
    let (tmp, e, root, _round) = hosted_board();
    break_the_declarations(tmp.path());
    // The probe is real: a chat identity, whose rules ARE the declarations, now fails to read.
    let broken = e
        .thread(&root, Reader::Handle(TERMINAL_OPENER))
        .expect_err("a malformed channels.yaml fails the reader that loads it");
    assert_ne!(
        broken.kind,
        ErrorKind::Forbidden,
        "the probe must fail by PARSING, or this test proves nothing: {broken:?}"
    );

    for thread in [root.as_str(), "m-no-such-thread"] {
        assert_eq!(
            refused(e.thread(thread, Reader::NotOwner)),
            ErrorKind::Forbidden,
            "a non-owner is refused before the declarations are read, so it cannot even learn \
             that they are broken ({thread})"
        );
    }
    assert_eq!(
        refused(e.search(Reader::NotOwner, "looked")),
        ErrorKind::Forbidden,
        "`search` refuses before the declarations are read"
    );
}

#[test]
fn the_owner_reads_no_declaration_so_a_malformed_one_does_not_refuse_it() {
    let (tmp, e, _root, round) = hosted_board();
    let before = e.thread(&round, Reader::Owner).unwrap();
    break_the_declarations(tmp.path());
    assert_eq!(
        e.thread(&round, Reader::Owner)
            .expect("the owner's view consults no declaration"),
        before,
        "a broken channels.yaml changes nothing the owner reads"
    );
    assert_eq!(
        e.search(Reader::Owner, "looked")
            .expect("nor does the owner's search")
            .len(),
        2
    );
}

// ---- below the engine: facade::thread and facade::search hold the same rule -------------------

fn open_store(dir: &Path) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap()
}

#[test]
fn facade_thread_answers_for_the_reader_and_refuses_before_not_found() {
    let (tmp, _e, root, round) = hosted_board();
    let s = open_store(tmp.path());
    let requester_only = nexus_chat::channel::Visibility::RequesterOnly;

    let as_owner = facade::thread(&s, &round, Reader::Owner, requester_only).unwrap();
    assert_eq!(
        bodies(&as_owner.messages),
        [
            "review the release",
            "dev looked: fine",
            "qa looked: one nit"
        ],
        "the owner passes the gate and the filter at the facade, not only at the engine"
    );
    assert_eq!(
        bodies(
            &facade::thread(&s, &round, Reader::Handle("dev"), requester_only)
                .unwrap()
                .messages
        ),
        ["review the release", "dev looked: fine"],
        "a chat identity is still filtered at the facade"
    );
    for thread in [root.as_str(), round.as_str(), "m-no-such-thread"] {
        assert_eq!(
            refused(facade::thread(&s, thread, Reader::NotOwner, requester_only)),
            ErrorKind::Forbidden,
            "the facade refuses a non-owner before it looks the thread up ({thread})"
        );
    }
    assert_eq!(
        refused(facade::thread(
            &s,
            "m-no-such-thread",
            Reader::Owner,
            requester_only
        )),
        ErrorKind::NotFound
    );
}

#[test]
fn facade_search_answers_for_the_reader() {
    let (tmp, _e, _root, _round) = hosted_board();
    let s = open_store(tmp.path());
    let mut found: Vec<String> = facade::search(&s, Reader::Owner, "looked", &[])
        .unwrap()
        .into_iter()
        .map(|h| h.body)
        .collect();
    found.sort();
    assert_eq!(
        found,
        ["dev looked: fine", "qa looked: one nit"],
        "the owner searches every channel at the facade, with no catalogue handed in"
    );
    assert_eq!(
        refused(facade::search(&s, Reader::NotOwner, "looked", &[])),
        ErrorKind::Forbidden
    );
    assert!(
        facade::search(&s, Reader::Handle("4jgn/stranger"), "looked", &[])
            .unwrap()
            .is_empty(),
        "a chat identity that is a member of nothing still finds nothing"
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
/// it, or a local caller could escalate itself past every gate. Held at the source, over every crate
/// that links `nexus-chat` (the local surface: this crate, `nxs`, and any other that depends on it),
/// walked recursively: no CODE line names `Owner` or `NotOwner` as a word — which catches the
/// variant whatever path, glob import (`use Reader::*`) or alias (`use Reader as R`) it is reached
/// by — except the lines that DEFINE the reader and its one rule in `facade.rs`, listed one by one
/// below. Comments are skipped, so the doc prose naming the variants is free; everything else,
/// `engine.rs` included, is not exempt.
///
/// Not a type-level guarantee — a variant of a public enum is constructible anywhere in its crate,
/// and a cargo feature would unify across the workspace the `nxs` binary is built in — so this is
/// the guarantee, and it is written to be hard to slip past rather than easy to satisfy.
#[test]
fn nothing_on_the_local_surface_can_read_as_the_owner() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits in the workspace's crates/ folder")
        .to_path_buf();
    let mut linking: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("crates/ is readable") {
        let dir = entry.unwrap().path();
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        let name_line = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("name"))
            .unwrap_or_default();
        if name_line.contains("\"nexus-chat\"") || manifest.contains("nexus-chat = ") {
            linking.push(dir);
        }
    }
    linking.sort();
    let names: Vec<String> = linking
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for required in ["chat", "nxs"] {
        assert!(
            names.iter().any(|n| n == required),
            "the scan must cover `{required}` — found {names:?}"
        );
    }

    // The ONLY code lines that may name the variants: the definition of `Reader`, of `Standing`,
    // the one rule (`Reader::standing`) and the one bypass shape (`Standing::handle`).
    let allowed_in_facade: BTreeMap<&str, usize> = BTreeMap::from([
        ("Owner,", 2),
        ("NotOwner,", 1),
        ("Reader::Owner => Ok(Standing::Owner),", 1),
        ("Reader::NotOwner => Err(NxfError::forbidden(", 1),
        ("Standing::Owner => None,", 1),
    ]);
    let chat_facade = crates.join("chat").join("src").join("facade.rs");

    let mut offenders = Vec::new();
    let mut facade_lines: BTreeMap<String, usize> = BTreeMap::new();
    let mut scanned = 0usize;
    for dir in &linking {
        let mut files = Vec::new();
        rust_sources(&dir.join("src"), &mut files);
        for path in files {
            scanned += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in code_lines(&text).iter().enumerate() {
                if !names_an_owner_variant(line) {
                    continue;
                }
                if path == chat_facade {
                    *facade_lines.entry(line.trim().to_string()).or_default() += 1;
                } else {
                    offenders.push(format!("{}:{} {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(scanned > 40, "the walk found the sources ({scanned} files)");
    assert!(
        offenders.is_empty(),
        "only an embedder selects the owner reader; the local surface reads as a chat identity: \
         {offenders:#?}"
    );
    let allowed: BTreeMap<String, usize> = allowed_in_facade
        .iter()
        .map(|(l, c)| (l.to_string(), *c))
        .collect();
    assert_eq!(
        facade_lines, allowed,
        "facade.rs names the owner variants in code only where it defines the reader and its one \
         rule — a further use is a new way to read as the owner, and belongs on this list only by \
         deliberate decision"
    );
}

/// Every `.rs` file under `dir`, recursively.
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `text` with every comment blanked — `//` to end of line (doc comments included) and `/* … */`
/// — line for line, so a line number still points at the source. String literals are tracked so a
/// `//` inside one is not taken for a comment; a string's own content is kept, which can only make
/// the check stricter.
fn code_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut in_string = false;
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut code = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if in_block {
                if c == '*' && next == Some('/') {
                    in_block = false;
                    i += 1;
                }
            } else if in_string {
                code.push(c);
                if c == '\\' {
                    if let Some(n) = next {
                        code.push(n);
                        i += 1;
                    }
                } else if c == '"' {
                    in_string = false;
                }
            } else if c == '/' && next == Some('/') {
                break;
            } else if c == '/' && next == Some('*') {
                in_block = true;
                i += 1;
            } else if c == '\'' && next == Some('"') && chars.get(i + 2) == Some(&'\'') {
                code.push_str("'\"'");
                i += 2;
            } else {
                if c == '"' {
                    in_string = true;
                }
                code.push(c);
            }
            i += 1;
        }
        out.push(code);
    }
    out
}

/// Whether a code line names `Owner` or `NotOwner` as a whole identifier.
fn names_an_owner_variant(line: &str) -> bool {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "Owner" || word == "NotOwner")
}

/// The scanner itself is held, so the guard cannot go quietly blind.
#[test]
fn the_owner_scan_sees_through_globs_aliases_and_trailing_comments_but_not_into_comments() {
    let code = "use nexus_chat::facade::Reader::*;\nlet r = Owner;\nuse nexus_chat::facade::Reader as R;\nlet n = R::NotOwner; // a note\n/// Reader::Owner in a doc\n// Reader::Owner in a comment\n/* Reader::Owner */ let x = 1;\nlet s = \"// not a comment\"; let o = Owner;\nlet h = OwnerName;\n";
    let flagged: Vec<usize> = code_lines(code)
        .iter()
        .enumerate()
        .filter(|(_, l)| names_an_owner_variant(l))
        .map(|(n, _)| n + 1)
        .collect();
    assert_eq!(flagged, [2, 4, 8]);
}

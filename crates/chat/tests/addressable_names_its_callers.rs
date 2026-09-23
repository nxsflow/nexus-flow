//! **Who may open a conversation, decided by WHO IS CALLING** — the acceptance suite of nxf
//! 6j6v.st83 (with 6j6v.d575's "it must name callers, not only channels").
//!
//! `addressable:` had exactly two states and both missed the case the owner kept hitting: `general`
//! admitted everyone including the person at the terminal, and a channel list admitted nobody
//! including the person at the terminal. The two shipped needs are each one half of the same
//! missing bit:
//!
//! * the PM in `watch-bundestag` — the OWNER may send it a direct message, every persona goes
//!   through the `planning` round, so the sentence in its prompt ("that channel is the only way
//!   anybody reaches you") becomes an assurance instead of a convention;
//! * a marketing specialist — the head of marketing may address it individually, a person may not,
//!   so the round stays the way in for everybody else.
//!
//! **Both caller classes are played for real here**, which is what the item's DoD asks for: a human
//! is a `Caller` with no session, a running persona is a `Caller` whose session the session map
//! knows — the same resolution `nxc` itself performs, not a flag a test sets.
//!
//! The refusal's ROUTE IN is derived rather than declared (nxf 6j6v.g0yn): the channels whose CAST
//! names the target. `route_in_is_derived_from_the_channel_that_casts_the_target` is the one that
//! holds it.

mod common;

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-09-13T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// A persona declared with `addressable: <value>` verbatim — the YAML a user writes, parsed by the
/// real loader, so these cases prove the declared surface and not a constructor.
fn role_with(handle: &str, addressable: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\naddressable: {addressable}\n"
    ))
    .expect("test role parses")
}

fn plain_role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn engine(dir: &Path, defs: Definitions) -> Engine {
    common::write_declarations(dir, &defs);
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            // Dry, not disabled: the admitted cases here SPAWN, and a handle that refuses to spawn
            // would make an admission look like a refusal.
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

fn open_store(dir: &Path) -> ChatStore {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The person at the terminal: no session, so the identity resolves to `Human` exactly as it does
/// for `nxc` run from a shell.
fn human(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// A call from inside a running persona. It names only its session; which persona that IS comes
/// back out of the session map the spawn wrote — provenance, not assertion.
fn in_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

/// Mint the session map entry a spawn would have written, so `in_session` resolves to `handle`.
fn running(dir: &Path, session: &str, handle: &str) {
    let mut store = open_store(dir);
    store
        .create_pending_session(session, handle)
        .expect("mint the session");
}

fn send<'a>(to: &'a str, body: &'a str) -> SendToRequest<'a> {
    SendToRequest {
        machine: None,
        to,
        body,
        refs: SendToRefs::ExplicitlyNone,
    }
}

// ---- the PM: a person may, a persona may not ---------------------------------------------------

#[test]
fn a_human_may_open_a_conversation_the_declaration_reserves_for_humans() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![role_with("pm", "{humans: true}"), plain_role("coder")],
        vec![],
    )
    .unwrap();
    let e = engine(tmp.path(), defs);

    let receipt = e
        .send_to(human("ckoch"), send("pm", "Make the field two lines high."))
        .expect("the human the declaration names is admitted");
    assert_eq!(receipt.to, "pm");
}

#[test]
fn a_running_persona_is_refused_by_a_declaration_that_names_only_humans() {
    // The other half of the same declaration, and the half that makes the PM's prompt true: the
    // round is the only way an AGENT reaches it.
    let tmp = workspace();
    let defs = Definitions::new(
        vec![role_with("pm", "{humans: true}"), plain_role("coder")],
        vec![],
    )
    .unwrap();
    let e = engine(tmp.path(), defs);
    running(tmp.path(), "s-coder", "coder");

    let err = e
        .send_to(
            in_session("s-coder"),
            send("pm", "Sneaking past the round."),
        )
        .expect_err("a persona is not a human");
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("only the human at the terminal"),
        "the refusal must say who MAY, got: {}",
        err.msg
    );
    assert!(
        err.msg.contains("you are `coder`"),
        "…and which class the caller fell into, got: {}",
        err.msg
    );
}

// ---- the specialist: one named persona may, a person may not -----------------------------------

#[test]
fn a_named_persona_may_open_a_conversation_a_human_may_not() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![
            role_with("specialist", "{personas: [head-of-marketing]}"),
            plain_role("head-of-marketing"),
        ],
        vec![],
    )
    .unwrap();
    let e = engine(tmp.path(), defs);
    running(tmp.path(), "s-head", "head-of-marketing");

    let receipt = e
        .send_to(
            in_session("s-head"),
            send("specialist", "Draft the positioning."),
        )
        .expect("the persona the whitelist names is admitted");
    assert_eq!(receipt.to, "specialist");

    let err = e
        .send_to(human("ckoch"), send("specialist", "Draft the positioning."))
        .expect_err("an omitted `humans:` is false, not absent");
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("only head-of-marketing"),
        "the refusal must name who may, got: {}",
        err.msg
    );
}

#[test]
fn a_persona_the_whitelist_does_not_name_is_refused() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![
            role_with("specialist", "{personas: [head-of-marketing]}"),
            plain_role("head-of-marketing"),
            plain_role("intern"),
        ],
        vec![],
    )
    .unwrap();
    let e = engine(tmp.path(), defs);
    running(tmp.path(), "s-intern", "intern");

    let err = e
        .send_to(in_session("s-intern"), send("specialist", "Hi."))
        .expect_err("a whitelist admits exactly whom it names");
    assert_eq!(err.kind, ErrorKind::Validation);
}

// ---- the two caller-blind forms are unchanged --------------------------------------------------

#[test]
fn general_still_admits_both_caller_classes() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![role_with("anyone", "general"), plain_role("coder")],
        vec![],
    )
    .unwrap();
    let e = engine(tmp.path(), defs);
    running(tmp.path(), "s-coder", "coder");

    e.send_to(human("ckoch"), send("anyone", "From a person."))
        .expect("a human may");
    e.send_to(in_session("s-coder"), send("anyone", "From a persona."))
        .expect("and so may a persona");
}

#[test]
fn none_refuses_both_caller_classes() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![role_with("reviewer", "none"), plain_role("coder")],
        vec![],
    )
    .unwrap();
    let e = engine(tmp.path(), defs);
    running(tmp.path(), "s-coder", "coder");

    for (caller, who) in [
        (human("ckoch"), "a human"),
        (in_session("s-coder"), "a persona"),
    ] {
        assert!(
            e.send_to(caller, send("reviewer", "Direct.")).is_err(),
            "{who} must be refused by `addressable: none`"
        );
    }
}

// ---- the route in is derived, not declared -----------------------------------------------------

#[test]
fn route_in_is_derived_from_the_channel_that_casts_the_target() {
    // nxf 6j6v.g0yn: the persona no longer repeats which channels it takes part in, so the refusal
    // has to find the way in from the channel side. `review` casts `reviewer` through its STEPS
    // alone — no `members:` entry — which is exactly the shape that used to be invisible.
    let tmp = workspace();
    let channels: Vec<nexus_chat::channel::ChannelDecl> = serde_yaml::from_str(
        "- name: review\n  members: []\n  steps:\n    - id: judge\n      target: reviewer\n",
    )
    .expect("test channel parses");
    let defs = Definitions::new(
        vec![role_with("reviewer", "none"), plain_role("coder")],
        channels,
    )
    .unwrap();
    let e = engine(tmp.path(), defs);

    let err = e
        .send_to(human("ckoch"), send("reviewer", "Direct."))
        .expect_err("`none` refuses everyone");
    assert!(
        err.msg.contains("send --to review instead"),
        "the refusal must name the derived route in, got: {}",
        err.msg
    );
}

#[test]
fn the_deprecated_channel_list_still_names_its_own_route() {
    // A workspace mid-migration reads exactly as it did: the author's list wins over the derived
    // answer, so the message is byte-comparable with the one 0.95.0 produced.
    let tmp = workspace();
    let channels: Vec<nexus_chat::channel::ChannelDecl> =
        serde_yaml::from_str("- name: review\n  members: [reviewer]\n").expect("parses");
    let defs = Definitions::new(vec![role_with("reviewer", "[review]")], channels).unwrap();
    let e = engine(tmp.path(), defs);

    let err = e
        .send_to(human("ckoch"), send("reviewer", "Direct."))
        .expect_err("the list form still refuses everyone");
    assert!(
        err.msg.contains("send --to review instead"),
        "got: {}",
        err.msg
    );
}

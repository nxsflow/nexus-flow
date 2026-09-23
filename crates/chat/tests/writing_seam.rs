//! **Two writing verbs, three statements each** — the SEAM half of nxf 6j6v.ckeq's acceptance.
//!
//! `Engine` carried FOUR writing verbs and eleven caller options. It carries two verbs now, and
//! `send_to` takes `to`/`body`/`refs` while `reply_thread` takes `thread`/`body` and the escalation
//! note. Everything else was a per-call answer to a question the DECLARATION already answers —
//! owner, 2026-08-21: "sonst wird ein Agent vielleicht uebereifrig Optionen aendern."
//!
//! Two of the removals take a mechanism's INPUT away while the mechanism itself stays wired up, and
//! both are places where a silent regression would look exactly like success. This file pins them at
//! the seam; `surface_cli.rs`'s own 6j6v.ckeq section pins the two that are only visible from the
//! command line (the FIFO queue and the sidecar teardown).
//!
//! 1. **`kind` stays as a CARRIER.** `--escalate` IS `kind: escalation` (`KIND_ESCALATION`), and
//!    `ChatStore::last_reply_escalated` reads that column. Two rules hang off it — 6j6v.1xw1 (does
//!    the claim thread close and release the working copy?) and 6j6v.e9qj (does the consolidator
//!    fold an escalating set or pass it through?). Both keep their own end-to-end suites
//!    (`working_tree_claim_scope.rs`, `reply_escalate.rs`); what is pinned HERE is that the bit
//!    survives the collapse of five `--kind` values into one boolean, and that the boolean really is
//!    the same column those rules read.
//! 2. **"Not specified" is not "explicitly none".** `refs` became mandatory on `send_to`, so the
//!    request has to carry a THREE-state answer: a set of pointers, a deliberate none, or nothing
//!    said at all. Only the third warns. An `Option<Refs>` cannot express that — `None` would have
//!    to mean both silent states — which is why [`SendToRefs`] is an enum and why the warning is a
//!    FIELD on the receipt rather than a line on stderr an app never meets (the breadcrumb class
//!    6j6v.93zd closed).

mod common;

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::model::{Refs, KIND_ESCALATION};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-21T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn engine_with_coder(tmp: &TempDir) -> Engine {
    let defs = Definitions::new(
        vec![
            serde_yaml::from_str::<RoleDecl>("handle: coder\nsystem_prompt: You are coder.\n")
                .unwrap(),
        ],
        vec![],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

/// Who is calling, for a case with no session of its own: a human, naming its `actor` because
/// there is no session map entry to read a persona off (nxf 6j6v.07me).
fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

fn summon_coder(eng: &Engine, refs: SendToRefs) -> String {
    eng.send_to(
        caller("carsten"),
        SendToRequest {
            machine: None,
            to: "coder",
            body: "please do the thing",
            refs,
        },
    )
    .expect("send_to")
    .thread_id
}

// ---- 1. the escalation note: `kind` survives as a carrier ------------------------------------

#[test]
fn the_escalation_note_is_the_whole_of_what_kind_still_carries_at_the_seam() {
    // `kind` is gone as a CALLER option and stays as a CARRIER: the seam offers one boolean, and the
    // boolean lands in `messages.kind` as `escalation` — the exact column 6j6v.1xw1's claim rule and
    // 6j6v.e9qj's fold rule both read. Both of those rules keep their own end-to-end suites; this is
    // the seam half, and it is what makes "exactly two values stay in use" a fact about the API
    // rather than a note in a ticket.
    let tmp = workspace();
    let eng = engine_with_coder(&tmp);
    let thread = summon_coder(&eng, SendToRefs::ExplicitlyNone);

    let receipt = eng
        .reply_thread(
            caller("coder"),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "I cannot: the credentials are missing",
                escalate: true,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("reply_thread");
    assert!(receipt.posted, "{receipt:?}");

    let store = open_store(&tmp);
    assert_eq!(
        store
            .messages_in_thread(&thread)
            .unwrap()
            .into_iter()
            .map(|m| m.kind)
            .collect::<Vec<_>>(),
        vec!["info".to_string(), KIND_ESCALATION.to_string()],
        "the opening message is the default `info` and the escalation rides the reply's own kind"
    );
    assert_eq!(
        store.last_reply_escalated(&thread).unwrap(),
        Some(true),
        "…and the derivation 6j6v.1xw1/6j6v.e9qj branch on reads it back"
    );
}

#[test]
fn a_reply_without_the_note_is_the_default_kind_and_reads_as_no_escalation() {
    // The control that makes the test above mean something: one boolean, two values, and the false
    // one is an ordinary answer rather than the absence of one.
    let tmp = workspace();
    let eng = engine_with_coder(&tmp);
    let thread = summon_coder(&eng, SendToRefs::ExplicitlyNone);

    eng.reply_thread(
        caller("coder"),
        ReplyThreadRequest {
            machine: None,
            thread: &thread,
            body: "done",
            escalate: false,
            needs_rework: false,
            accept: false,
        },
    )
    .expect("reply_thread");

    assert_eq!(
        open_store(&tmp).last_reply_escalated(&thread).unwrap(),
        Some(false)
    );
}

// ---- 2. "not specified" is a different answer from "explicitly none" --------------------------

#[test]
fn the_seam_tells_not_specified_from_explicitly_none() {
    // The design 6j6v.ckeq §5 asked for, at the seam rather than at the flag. Both states post the
    // message and both stamp no pointers; what differs is whether the caller ANSWERED, and only the
    // unanswered one warns. `Option<Refs>` could not carry this: `None` would have to mean both.
    let tmp = workspace();
    let eng = engine_with_coder(&tmp);

    let unspecified = eng
        .send_to(
            caller("carsten"),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "nothing said about what this is for",
                refs: SendToRefs::Unspecified,
            },
        )
        .expect("send_to");
    assert!(
        unspecified.refs_warning.is_some(),
        "silence is not an answer, and the receipt says so: {unspecified:?}"
    );

    let explicit = eng
        .send_to(
            caller("carsten"),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "this one really is about nothing",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("send_to");
    assert_eq!(
        explicit.refs_warning, None,
        "an explicit none IS an answer: {explicit:?}"
    );
}

#[test]
fn an_empty_set_handed_to_the_declared_constructor_is_not_a_declared_nothing() {
    // The one way the obligation could otherwise be met by accident: wrapping an empty `Refs` in
    // `Declared`. The constructor decides, so a caller that assembles pointers and finds none ends
    // up in the state that warns rather than in the state that says "I looked, there is nothing".
    assert_eq!(
        SendToRefs::declared(Refs::default()),
        SendToRefs::Unspecified
    );
    assert_eq!(
        SendToRefs::declared(Refs {
            nxf_ids: vec!["6j6v.ckeq".into()],
            ..Refs::default()
        }),
        SendToRefs::Declared(Refs {
            nxf_ids: vec!["6j6v.ckeq".into()],
            ..Refs::default()
        })
    );
}

#[test]
fn what_the_caller_named_is_what_the_message_carries() {
    // The pointers are not merely accepted, they are stamped — a list of them, in order (owner: "Es
    // muss eine LISTE angegeben werden koennen").
    let tmp = workspace();
    let eng = engine_with_coder(&tmp);
    let thread = summon_coder(
        &eng,
        SendToRefs::declared(Refs {
            nxf_ids: vec!["6j6v.ckeq".into(), "6j6v.m4xe".into()],
            ..Refs::default()
        }),
    );

    let store = open_store(&tmp);
    let refs: Refs = store
        .messages_in_thread(&thread)
        .unwrap()
        .into_iter()
        .find_map(|m| m.refs)
        .map(|raw| serde_json::from_str(&raw).expect("refs parse"))
        .expect("the opening message carries refs");
    assert_eq!(
        refs.nxf_ids,
        vec!["6j6v.ckeq".to_string(), "6j6v.m4xe".to_string()]
    );
}

//! **What a caller cannot be asked** — the acceptance suite of nxf 6j6v.07me.
//!
//! `Caller` carried five ambient values (`now`, `origin`, `actor`, `session`, `hop`). Four of them
//! were derivable, and the owner's question at the source on 2026-08-21 is what settled it: once
//! `send --to` and `reply --thread` are the only interaction verbs, does everything not follow from
//! "who sends and who receives"? For four of the five, yes:
//!
//! * `origin` comes from the HANDLE — one workspace per handle, one minting identity, and no way
//!   left for two calls on it to claim different ones. It is the value access rights will hang off
//!   (6j6v.6aza), so a per-call parameter for it was an opportunity to get exactly that wrong.
//! * `hop` comes from the SESSION MAP — `resolve_hop` has taken `persisted.max(claimed)` since
//!   6j6v.m48m, so a caller could only ever raise the depth. A caller that works through sessions
//!   passes nothing and loses nothing.
//! * `now` becomes an OVERRIDE with the system clock as its default. The one reason it was ever
//!   mandatory is determinism, and that is a property of the CLI's goldens and of these tests, not
//!   of an app.
//! * `actor` collapses into `session` — identity comes from PROVENANCE, not from assertion (design
//!   2026-08-05 §4.1). It is needed only where there is no session at all: a human at a terminal, or
//!   an app acting for its logged-in user.
//!
//! **`session` stays, and it is the SENDER's, not the recipient's.** The recipient's session really
//! is derived from the thread. `Caller.session` is the RETURN ADDRESS stamped onto the message
//! (`refs.session_id`, `with_return_address`), and it is what makes the chain run in both
//! directions — a supervisor commissions the coder, and the coder's `reply --thread` wakes the
//! supervisor BECAUSE its session is stamped there. Without it a commission is a one-way trip.
//!
//! Each acceptance point of the item has a test here, in the item's own order.

mod common;

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::orchestration::{Caller, MAX_HOP};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use std::path::Path;
use tempfile::TempDir;

/// The pinned clock these tests hand over as [`Caller::now`]'s override — every case but the one
/// that proves the default.
const NOW: &str = "2026-08-21T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn simple_role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

/// Declare `roles` in the workspace's own `.nxs-personas/` folder and open a handle over it, with a
/// dry worker: the persona path SPAWNS on every send here, and a handle that refuses to spawn would
/// make this suite prove the wrong thing.
fn engine(dir: &Path, roles: Vec<RoleDecl>) -> Engine {
    common::write_declarations(dir, &Definitions::new(roles, vec![]).unwrap());
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

fn open_store(dir: &Path) -> ChatStore {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// A human at a terminal, or an app acting for its logged-in user: no session, so it must name its
/// actor — and it pins the clock, as every case here does but one.
fn human(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// A call made from inside a running persona session. It names NOTHING else: the persona, the
/// workspace identity and the chain depth are all read back from where they are already recorded.
fn in_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

fn send_req<'a>(to: &'a str, body: &'a str) -> SendToRequest<'a> {
    SendToRequest {
        machine: None,
        to,
        body,
        refs: SendToRefs::ExplicitlyNone,
    }
}

fn reply_req<'a>(thread: &'a str, body: &'a str) -> ReplyThreadRequest<'a> {
    ReplyThreadRequest {
        machine: None,
        thread,
        body,
        escalate: false,
        needs_rework: false,
        accept: false,
    }
}

// ---- 1. the origin comes from the handle -------------------------------------------------------

#[test]
fn two_calls_on_one_handle_can_no_longer_claim_different_origins() {
    // The item's second acceptance point. `origin` was a `&str` a caller passed per call, beside
    // three other `&str`-shaped values — and it is the MINTING WORKSPACE identity, the one access
    // rights will later hang off (6j6v.6aza). Two calls through one handle could name two different
    // workspaces, and nothing in the type could notice.
    //
    // Now the handle answers it, for every call, and there is no field left to disagree with. What
    // this test can still observe is the consequence: two callers as different as they get — a human
    // with no session and a persona speaking from inside its own — mint under the SAME origin, and
    // it is the one the handle reports.
    let tmp = workspace();
    let eng = engine(tmp.path(), vec![simple_role("coder")]);
    let origin = eng.origin();

    let opened = eng
        .send_to(human("carsten"), send_req("coder", "please do the thing"))
        .expect("the human opens the conversation");
    let coder_session = opened.session.clone().expect("the persona was started");

    let answered = eng
        .reply_thread(
            in_session(&coder_session),
            reply_req(&opened.thread_id, "done"),
        )
        .expect("the persona answers from inside its own session");
    assert!(answered.posted, "{answered:?}");

    let store = open_store(tmp.path());
    let origins: Vec<String> = store
        .messages_in_thread(&opened.thread_id)
        .unwrap()
        .into_iter()
        .map(|m| m.origin)
        .collect();
    assert_eq!(
        origins,
        vec![origin.clone(), origin.clone()],
        "both writes are minted under the handle's own origin — the callers had no say"
    );

    // And the qualified handles are built from that same value, so an app can address what it just
    // wrote without having been told the origin by anyone.
    let senders: Vec<String> = store
        .messages_in_thread(&opened.thread_id)
        .unwrap()
        .into_iter()
        .map(|m| m.sender)
        .collect();
    assert_eq!(
        senders,
        vec![format!("{origin}/carsten"), format!("{origin}/coder")],
        "`<origin>/<actor>`, in that order — never `carsten/local`"
    );
}

// ---- 2. the depth guard is carried by the session map ------------------------------------------

#[test]
fn a_chain_runs_into_the_depth_cap_while_no_caller_ever_passes_a_hop() {
    // The item's third acceptance point. `Caller.hop` is gone, and the guard is untouched: the
    // depth lives in `session_map`, written by whoever SPAWNS a session (6j6v.m48m), and every hop
    // of this chain reads it back from the session it speaks from.
    //
    // This is the shape the guard exists for — commissions only, nobody ever answering — driven
    // through the SEAM, one `send_to` per hop, each from the session the previous hop started.
    // Distinct personas per hop so that what bounds the chain is the depth and nothing else.
    let tmp = workspace();
    let roles: Vec<RoleDecl> = (0..MAX_HOP + 5)
        .map(|i| simple_role(&format!("p{i}")))
        .collect();
    let eng = engine(tmp.path(), roles);

    // Hop 1 comes from a human: no session, therefore depth 0, and the persona it summons is
    // recorded at depth 1.
    let mut receipt = eng
        .send_to(human("carsten"), send_req("p0", "start the chain"))
        .expect("the human starts the chain");
    let mut hops = 1_u32;

    for i in 1..MAX_HOP + 5 {
        let session = receipt.session.clone().expect("every hop starts a persona");
        assert_eq!(
            open_store(tmp.path()).session_depth(&session).unwrap(),
            Some(hops),
            "the session map carries the depth of hop {hops}, and nobody passed it"
        );
        match eng.send_to(in_session(&session), send_req(&format!("p{i}"), "carry on")) {
            Ok(next) => {
                receipt = next;
                hops += 1;
            }
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Validation, "{e:?}");
                assert!(e.msg.contains("depth guard"), "refused by name: {}", e.msg);
                assert!(
                    hops > MAX_HOP,
                    "the cap must bound the chain, not cut it short: {hops} hops"
                );
                return;
            }
        }
    }
    panic!("the chain never ran out of depth after {hops} hops");
}

// ---- 3. the return address is still stamped ----------------------------------------------------

#[test]
fn the_senders_session_is_stamped_as_the_return_address_and_the_reply_wakes_it() {
    // The item's fourth acceptance point, and the reason `session` is the one field that STAYS.
    // It is the SENDER's session, not the recipient's: it rides onto the message as
    // `refs.session_id` (`with_return_address`), and that is the address `reply --thread` wakes.
    //
    //     supervisor -> send --to coder -> T   (the SUPERVISOR's session is stamped)
    //     coder      -> reply --thread T       (wakes the supervisor, BECAUSE it is stamped there)
    //
    // Without it the supervisor could commission the coder and the coder's answer would have nobody
    // to wake — a one-way chain.
    let tmp = workspace();
    let eng = engine(
        tmp.path(),
        vec![simple_role("supervisor"), simple_role("coder")],
    );

    // A human summons the supervisor, which is how the supervisor comes to have a session at all.
    let kickoff = eng
        .send_to(human("carsten"), send_req("supervisor", "run the release"))
        .expect("summon the supervisor");
    let supervisor_session = kickoff.session.clone().expect("the supervisor was started");

    // The supervisor commissions the coder — naming its own session and nothing else.
    let commissioned = eng
        .send_to(
            in_session(&supervisor_session),
            send_req("coder", "implement the thing"),
        )
        .expect("the supervisor commissions the coder");
    let coder_session = commissioned.session.clone().expect("the coder was started");

    let stamped = open_store(tmp.path())
        .message_return_address(&commissioned.message_id)
        .expect("read the return address");
    assert_eq!(
        stamped.as_deref(),
        Some(supervisor_session.as_str()),
        "the SENDER's session is the return address on the commission"
    );

    // The coder answers into that thread: the supervisor is woken because it is stamped there.
    let answered = eng
        .reply_thread(
            in_session(&coder_session),
            reply_req(&commissioned.thread_id, "implemented"),
        )
        .expect("the coder answers");
    assert_eq!(
        answered.woke.as_deref(),
        Some(supervisor_session.as_str()),
        "the SUPERVISOR's own session is what the answer woke, not a fresh one: {answered:?}"
    );
    assert!(
        answered.wake_skipped.is_none(),
        "and the wake landed: {answered:?}"
    );
    // `resumed` is deliberately FALSE here and that is not a weaker result. The commission
    // registered an expectation on the supervisor's thread, so this answer DISCHARGES a quorum and
    // is routed by the completion path; `resumed` names the other one — the direct 1:1 return to a
    // thread nobody was expected to answer on. The two are mutually exclusive by construction (see
    // `ReplyReceipt::resumed`), and `woke` is the field that is true of both.
    assert!(!answered.resumed, "{answered:?}");
}

// ---- 4. an actor is required exactly where the session cannot answer ---------------------------

#[test]
fn a_call_with_neither_a_session_nor_an_actor_is_a_named_validation_error() {
    // The item's fifth acceptance point, and it is about what must NOT happen: no `nxc`, no `USER`,
    // no "unknown" — the CLI's own defaulting chain is the CLI's, and a library that invented one
    // would attribute a write to an identity nobody chose.
    let tmp = workspace();
    let eng = engine(tmp.path(), vec![simple_role("coder")]);

    let err = eng
        .send_to(
            Caller {
                session: None,
                actor: None,
                now: Some(NOW),
            },
            send_req("coder", "who is this from?"),
        )
        .map(|_| ())
        .expect_err("nobody is sending this");
    assert_eq!(err.kind, ErrorKind::Validation, "{err:?}");
    assert!(
        err.msg.contains("actor") && err.msg.contains("session"),
        "the error names both ways out: {}",
        err.msg
    );

    // The same rule at the other end of it: a session id that names no session of ours answers
    // nothing either, so it does not stand in for an actor. (`resolve_hop` has always treated such
    // an id as contributing no depth — for the identical reason.)
    let err = eng
        .send_to(
            Caller {
                session: Some("s-not-ours"),
                actor: None,
                now: Some(NOW),
            },
            send_req("coder", "who is this from?"),
        )
        .map(|_| ())
        .expect_err("a session we never minted names no persona");
    assert_eq!(err.kind, ErrorKind::Validation, "{err:?}");
    assert!(err.msg.contains("actor"), "{}", err.msg);
}

#[test]
fn the_session_map_names_the_persona_and_an_asserted_actor_does_not_override_it() {
    // Identity comes from PROVENANCE, not from assertion (design 2026-08-05 §4.1). Where a session
    // answers, it wins; `actor` is the answer for the case the map cannot answer, which is why
    // passing both is a fallback rather than a conflict.
    let tmp = workspace();
    let eng = engine(tmp.path(), vec![simple_role("coder")]);

    let opened = eng
        .send_to(human("carsten"), send_req("coder", "please do the thing"))
        .expect("the human opens the conversation");
    let coder_session = opened.session.clone().expect("the persona was started");

    eng.reply_thread(
        Caller {
            session: Some(&coder_session),
            actor: Some("mallory"),
            now: Some(NOW),
        },
        reply_req(&opened.thread_id, "done"),
    )
    .expect("the persona answers");

    let senders: Vec<String> = open_store(tmp.path())
        .messages_in_thread(&opened.thread_id)
        .unwrap()
        .into_iter()
        .map(|m| m.sender)
        .collect();
    let origin = eng.origin();
    assert_eq!(
        senders,
        vec![format!("{origin}/carsten"), format!("{origin}/coder")],
        "the session says `coder`, and what the caller asserted about itself is not consulted"
    );
}

// ---- 5. `now` is an override, not an obligation ------------------------------------------------

#[test]
fn now_is_an_override_and_the_default_is_the_handles_own_clock() {
    // The only reason `now` was ever mandatory is determinism — the library reads no clock, so a
    // test can pin "is this stale?" reproducibly. That reason belongs to the CLI's goldens and to
    // suites like this one, not to an app, which had to format an RFC3339 string on every call to
    // say "just use the clock".
    let tmp = workspace();
    let eng = engine(tmp.path(), vec![simple_role("coder")]);

    let pinned = eng
        .send_to(human("carsten"), send_req("coder", "pinned"))
        .expect("send with the clock pinned");
    let defaulted = eng
        .send_to(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: None,
            },
            send_req("coder", "defaulted"),
        )
        .expect("send with no clock at all");

    let store = open_store(tmp.path());
    let created = |thread: &str| -> String {
        store
            .messages_in_thread(thread)
            .unwrap()
            .into_iter()
            .next()
            .expect("the opening message")
            .created
            .expect("every message carries its created stamp")
    };
    assert_eq!(created(&pinned.thread_id), NOW, "the override is honoured");

    let real = created(&defaulted.thread_id);
    assert_ne!(real, NOW, "the default is the clock, not the last pin");
    time::OffsetDateTime::parse(&real, &time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|e| {
            panic!("the default is an RFC3339 instant, not a placeholder: {real} ({e})")
        });
}

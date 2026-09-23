//! The summon / fan-out / quorum verbs, driven through the library seam (nxf 6j6v.rws4).
//!
//! These lived in `cli.rs` as `send`'s role/session branches, `open_declared_channel_and_fan_out`
//! and `ask`. What only this seam can show — and what the black-box CLI suites cannot — is that
//! they run against a `Ctx`-supplied catalogue with **no declaration folder anywhere**, and that
//! the invariants previous review rounds paid for survive the move.
//!
//! "`Ctx`-supplied" is the `&Definitions` an `orchestration` verb BORROWS, and it is deliberately
//! not the removed seam injection: `DefinitionSource::Supplied`/`Engine::set_definitions` went in
//! nxf 6j6v.dvyq step 6, so a handle resolves its catalogue from `.nxs-personas/` and hands the
//! borrow in. What these tests exercise is the layer below that resolution.

use std::sync::Mutex;

use nexus_chat::definitions::Definitions;
use nexus_chat::error::ErrorKind;
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs};
use nexus_chat::orchestration::{
    self, ChannelOpenRequest, Commission, Ctx, RoleResumeRequest, MAX_HOP,
};
use nexus_chat::role::{Model, RoleDecl};
use nexus_chat::store::ChatStore;
use nexus_chat::worker::{TriggerRequest, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-07-31T10:00:00Z";

#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl RecordingWorker {
    /// The triggered role handles, in trigger order.
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The model each trigger was spawned on, in trigger order — `None` where nothing above the
    /// role declared one, which is what keeps "no choice" distinguishable from "a choice" all the
    /// way down to the spec JSON (`RoleSpec::model`'s own discipline).
    fn models(&self) -> Vec<Option<Model>> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.model)
            .collect()
    }

    /// `(role handle, internal session, the thread it was told it OWES a reply on)` per trigger, in
    /// trigger order — `reply_thread` is what the sidecar's teardown net settles against, and its
    /// being `Some` on a channel member is what nxf 6j6v.pf6j's acceptance point 4 is about.
    fn obligations(&self) -> Vec<(String, String, Option<String>)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.role.handle.clone(),
                    r.internal_session.clone(),
                    r.reply_thread.clone(),
                )
            })
            .collect()
    }
}

/// The channel supervisor's qualified identity (nxf 6j6v.pf6j).
const SUPERVISOR: &str = "local/__channel__";

/// The effective member set, read from the MEMBER THREADS below `channel_thread`.
///
/// Before 6j6v.pf6j this was the receipt's own `expects`; that field now reports the channel
/// thread's two ends (requester ↔ supervisor), and the member set lives one level down, one
/// expectation per thread.
fn member_expects(store: &ChatStore, channel_thread: &str) -> Vec<String> {
    let mut out: Vec<String> = store
        .supervised_children(channel_thread)
        .unwrap()
        .iter()
        .filter_map(|t| store.thread_quorum(t, NOW).unwrap())
        .filter(|q| q.opener.as_deref() == Some(SUPERVISOR))
        .flat_map(|q| q.expects)
        .collect();
    out.sort();
    out
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}

/// These suites never assert on SCHEDULING, and since the timer became an injected value
/// (PR #269 review, Code Quality #3) they no longer have to tolerate whatever `NXC_TIMER`
/// happens to be — unset, that used to mean a real `at` subprocess per armed deadline.
static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

fn simple_role(handle: &str) -> RoleDecl {
    role(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
}

fn channel_decl(yaml: &str) -> nexus_chat::channel::ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// A fresh workspace + store. No declaration folder is ever created — that is the point.
fn fresh_store() -> (TempDir, ChatStore) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let store = ws.open_chat_store().expect("open chat store");
    (tmp, store)
}

fn ctx<'a>(defs: &'a Definitions, worker: &'a dyn Worker, hop: u32) -> Ctx<'a> {
    Ctx {
        now: NOW,
        origin: "local",
        actor: "alice",
        session: None,
        hop,
        defs,
        worker,
        timer: &NO_TIMER,
        namer: &nexus_chat::naming::DryNamer,
        db_path: "/w/.nxs/db.sqlite",
        project_claude_md: None,
        module_primes: None,
        machines: None,
    }
}

fn message_count(store: &ChatStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap()
}

fn trigger_req<'a>(role: &'a str, body: &'a str) -> Commission<'a> {
    Commission {
        machine: None,
        role,
        body,
        channel: None,
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread: None,
        refs: Refs::default(),
        model: None,
        continue_session: None,
    }
}

// ---- coordinator_commission
// -------------------------------------------------------------------------

#[test]
fn a_commission_auto_opens_the_deterministic_dm_and_mints_a_pending_session() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let receipt =
        orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "do it"))
            .unwrap();

    assert!(receipt.message_id.starts_with("m-"), "{receipt:?}");
    assert!(
        receipt.channel.starts_with("dm:"),
        "a channel-less role trigger auto-opens the deterministic DM, got {}",
        receipt.channel
    );
    // The minted session is registered as pending and bound to the role, so a later `resolve_real`
    // and the workflow's own reverse lookup both find it.
    assert_eq!(
        store.session_role(&receipt.session).unwrap().as_deref(),
        Some("coder")
    );
    assert_eq!(worker.handles(), vec!["coder"]);
}

#[test]
fn a_commission_reuses_the_same_dm_on_a_second_call() {
    // `ensure_dm_channel` is idempotent by design: `add_member`'s OR-set is NOT, so a repeat
    // membership add would mint a fresh add-tag and inflate the live-tag member count past 2,
    // mislabelling the DM `degraded`. Every channel-less trigger goes through this path.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let first =
        orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "one")).unwrap();
    let second =
        orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "two")).unwrap();

    assert_eq!(first.channel, second.channel);
    let dm = store
        .list_member_channels("local/alice")
        .unwrap()
        .into_iter()
        .find(|c| c.channel_id == first.channel)
        .expect("the caller is a member of the DM it opened");
    assert_eq!(
        (dm.members, dm.degraded),
        (2, false),
        "a re-opened DM must still count exactly two live members, not accumulate add-tags"
    );
}

#[test]
fn a_commission_rejects_an_unknown_role_before_persisting_anything() {
    // Preflight-before-persist (review Code Quality #1 / Integrity #2): the old ordering posted the
    // message first and only then discovered the unknown role, leaving an orphan message behind a
    // misleading non-zero exit.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let err = orchestration::coordinator_commission(&c, &mut store, trigger_req("ghost", "do it"))
        .map(|_| ())
        .unwrap_err();

    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(message_count(&store), 0, "nothing may be persisted");
    assert_eq!(worker.count(), 0, "and nothing may be triggered");
}

#[test]
fn a_commission_rejects_a_hop_past_the_cap_before_persisting() {
    // The depth guard, checked before any work: a spawn chain cannot loop forever, and the refusal
    // must not leave a message behind either.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, MAX_HOP + 1);

    let err = orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "do it"))
        .map(|_| ())
        .unwrap_err();

    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("depth guard"), "{}", err.msg);
    assert_eq!(message_count(&store), 0);
    assert_eq!(worker.count(), 0);
}

// ---- channel_open -------------------------------------------------------------------------

#[test]
fn channel_open_fans_out_to_every_declared_member_except_the_sender() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![
            simple_role("alice"),
            simple_role("bob"),
            simple_role("carol"),
        ],
        vec![channel_decl("name: review\nmembers: [alice, bob, carol]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0); // actor is `alice`

    let receipt = orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "review",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();

    let mut handles = worker.handles();
    handles.sort();
    assert_eq!(
        handles,
        vec!["bob", "carol"],
        "the requester is never expected to reply to itself"
    );
    assert_eq!(
        receipt.board.expects,
        vec![SUPERVISOR],
        "the channel thread has two ends: the requester and the supervisor (nxf 6j6v.pf6j)"
    );
    assert_eq!(
        member_expects(&store, &receipt.board.thread_id),
        vec!["local/bob", "local/carol"],
        "…and the member set is one expectation per member thread"
    );
}

#[test]
fn channel_open_expects_are_qualified_while_trigger_targets_stay_bare() {
    // The deadlock regression this epic already diagnosed once: `expects_reply_from` is matched
    // against a real reply's `sender`, which is ALWAYS the qualified `origin/actor` identity, while
    // role resolution and triggering live in the bare handle namespace. Passing bare targets
    // straight through as expects meant `"bob" != "local/bob"` forever — outstanding never emptied,
    // complete never flipped. Both forms are derived from the SAME preflight-passed list.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("alice"), simple_role("bob")],
        vec![channel_decl("name: review\nmembers: [alice, bob]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let receipt = orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "review",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();

    assert_eq!(receipt.board.expects, vec![SUPERVISOR], "two ends up here");
    assert_eq!(
        member_expects(&store, &receipt.board.thread_id),
        vec!["local/bob"],
        "qualified for the quorum"
    );
    assert_eq!(worker.handles(), vec!["bob"], "bare for resolve/trigger");
}

// `channel_open_accepts_an_explicit_expect_in_either_form_without_double_qualifying` stood here.
// It pinned the qualification and de-duplication of a caller-supplied `--expect` — an entry could
// be written bare (`bob`) or qualified (`local/bob`) and had to come out qualified exactly once.
// The per-call override went with `ask --expect` (6j6v.dvyq §3), and `ChannelOpenRequest` no longer
// carries a field to write one into, so there is nothing left to qualify twice: the effective set
// comes from the declaration through `fan_out_targets`, in one place.

/// nxf 6j6v.pf6j, acceptance point 4: **the member is addressed as a PERSONA.**
///
/// The fan-out used to hand-roll `trigger_role` and pass `reply_thread: None` — the shape 6j6v.rp8k
/// filed: a role started with nothing to answer into, no obligation recorded against it and nothing
/// for its teardown net to discharge. It now goes through `coordinator_commission`, the same path
/// `send --to <persona>` takes, which since 6j6v.jepk declares the expectation on the FRESH thread it
/// was handed and relays that obligation to the spawned session. So on this path the thread-less
/// trigger cannot arise: every member trigger carries a thread it owes an answer on, that thread's
/// register agrees, and the session is recorded as standing there.
#[test]
fn every_member_is_triggered_as_a_persona_owing_a_reply_on_its_own_fresh_thread() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("bob"), simple_role("carol")],
        vec![channel_decl("name: review\nmembers: [bob, carol]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let receipt = orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "review",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();

    let member_threads = store.supervised_children(&receipt.board.thread_id).unwrap();
    assert_eq!(member_threads.len(), 2, "{member_threads:?}");
    let obligations = worker.obligations();
    assert_eq!(obligations.len(), 2, "{obligations:?}");
    for (handle, session, reply_thread) in &obligations {
        let thread = reply_thread
            .as_deref()
            .unwrap_or_else(|| panic!("{handle} was told nothing to answer: {obligations:?}"));
        assert!(
            member_threads.iter().any(|t| t == thread),
            "the obligation names one of the member threads: {obligations:?}"
        );
        // The register agrees with what the role was told — the half 6j6v.enrs insists on, so that
        // `reply --if-unanswered` can actually discharge what the prompt claims is owed.
        let q = store.thread_quorum(thread, NOW).unwrap().unwrap();
        assert_eq!(q.expects, vec![format!("local/{handle}")], "{q:?}");
        assert_eq!(
            q.opener.as_deref(),
            Some(SUPERVISOR),
            "the supervisor opened it, so only the supervisor may re-declare it: {q:?}"
        );
        // …and the session is standing in that thread, so what IT opens hangs under the member
        // thread rather than detaching from the operation (nxf 6j6v.a71h §3.1).
        assert_eq!(
            store.session_thread(session).unwrap().as_deref(),
            Some(thread),
            "{obligations:?}"
        );
    }
    // No two members share a thread, which is the whole of "the members do not see each other".
    assert_ne!(obligations[0].2, obligations[1].2, "{obligations:?}");
}

/// A channel with nobody to ask is refused, not opened (nxf 6j6v.pf6j, fix round 1).
///
/// Two ordinary shapes reach an empty effective set — a channel whose only member is the sender, and
/// an explicit `--expect` that filters down to nobody. Under the two levels neither is harmless any
/// more: the channel thread would expect the SUPERVISOR, and a supervisor whose set of member threads
/// is empty never resumes, so the requester would wait on that thread for good. Refused at the
/// preflight, before anything is persisted, exactly like the other two preflight rejections.
#[test]
fn channel_open_refuses_a_channel_with_nobody_left_to_ask() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("alice")],
        vec![channel_decl("name: solo\nmembers: [alice]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    // `alice` is the caller AND the channel's only member, so the effective set is empty.
    let err = orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "solo",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: None,
        },
    )
    .map(|_| ())
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation, "{err:?}");
    assert!(
        err.msg.contains("nobody to ask"),
        "the refusal says what is wrong: {}",
        err.msg
    );

    // Nothing was persisted — no half-open channel thread waiting on a supervisor that has nothing
    // to wait for.
    assert_eq!(message_count(&store), 0);
    assert_eq!(worker.count(), 0);
    assert_eq!(
        store
            .connection()
            .query_row("SELECT COUNT(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn channel_open_rejects_a_member_that_resolves_to_no_role_before_persisting() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("alice")],
        vec![channel_decl("name: review\nmembers: [alice, ghost]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let err = orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "review",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: None,
        },
    )
    .map(|_| ())
    .unwrap_err();

    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(message_count(&store), 0, "no board nobody will ever join");
    assert_eq!(worker.count(), 0);
}

// ---- send --to <channel>: the one entrance a declared channel has ---------------------------
//
// `orchestration::ask` and its three tests stood here. It routed between a DECLARED channel (fan
// out under its own policy) and a RAW one (open a board, trigger nobody, and demand an explicit
// `--expect`, because there was no declaration to read one from). 6j6v.dvyq §3 removes the raw
// channels, so the router has nothing to route between.
//
// What replaces the first test is not a deletion but a RETARGET: the same assertion, driven through
// the entrance that survives. That is the acceptance point's "expressible by construction, not by a
// paper mapping". The other two tested the raw half; the second of them is replaced by the
// rejection that now stands where it used to succeed.

#[test]
fn send_to_routes_a_declared_channel_through_the_fan_out() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("alice"), simple_role("bob")],
        vec![channel_decl("name: review\nmembers: [alice, bob]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    let receipt = nexus_chat::surface::send_to(
        &c,
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "review",
            body: "please review",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .unwrap();

    assert_eq!(worker.handles(), vec!["bob"], "a declared channel fans out");
    assert_eq!(receipt.expects, vec![SUPERVISOR]);
    assert_eq!(
        member_expects(&store, &receipt.thread_id),
        vec!["local/bob"]
    );
}

#[test]
fn send_to_refuses_a_channel_that_exists_but_no_declaration_names() {
    // The raw channel, at the seam. It really is in the store — `channel_exists` says so — and it
    // is still not a target, because nothing declares who answers on it, by when, or what becomes
    // of the answers. The rejection NAMES that rather than answering "no such target", which would
    // send the caller hunting for a typo in an id that is spelled correctly.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("alice")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);
    store.set_channel_field("c-1", "kind", "group", "local/alice");

    let err = nexus_chat::surface::send_to(
        &c,
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "c-1",
            body: "please review",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .map(|_| ())
    .unwrap_err();

    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("c-1") && err.msg.contains("no declaration names it"),
        "{}",
        err.msg
    );
    assert_eq!(worker.count(), 0, "and nothing was started");
}

// ---- the per-call model on the SEAM (nxf 6j6v.q6m2, coverage restored by 6j6v.e9qj) -----------
//
// `nxc send --model` went with 6j6v.dvyq §3, and its four CLI tests went with it. The SEAM field
// deliberately stayed (dvyq §5 is explicit that removing a CLI verb is not removing a seam read),
// and an embedding app is the caller it stayed for — but `ChannelOpenRequest.model` and
// `RoleResumeRequest.model` were left with no test at all, which is how a capability becomes the
// next removal's silent casualty. These two are that coverage, at the seam rather than through a
// CLI that no longer has the flag.

#[test]
fn channel_open_applies_its_per_call_model_to_every_fanned_out_member() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![
            simple_role("alice"),
            simple_role("bob"),
            simple_role("carol"),
        ],
        vec![channel_decl("name: review\nmembers: [alice, bob, carol]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0); // actor is `alice`

    orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "review",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: Some(Model::Opus),
        },
    )
    .unwrap();

    assert_eq!(worker.handles().len(), 2, "bob and carol were triggered");
    assert_eq!(
        worker.models(),
        vec![Some(Model::Opus), Some(Model::Opus)],
        "the per-call model is a modifier of WHAT triggers, so it rides every member of the fan-out \
         — neither role declares one, so this is the only source of a model at all"
    );
}

#[test]
fn channel_open_without_a_model_leaves_the_choice_to_each_members_own_declaration() {
    // The companion guard, and the half that keeps the field honest: absent means ABSENT, so a role
    // that declares nothing still reaches the sidecar with no model key.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![
            simple_role("alice"),
            role("handle: bob\nsystem_prompt: You are bob.\nmodel: sonnet\n"),
            simple_role("carol"),
        ],
        vec![channel_decl("name: review\nmembers: [alice, bob, carol]\n")],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);

    orchestration::channel_open(
        &c,
        &mut store,
        ChannelOpenRequest {
            channel: "review",
            body: "please review",
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();

    let by_handle: Vec<(String, Option<Model>)> =
        worker.handles().into_iter().zip(worker.models()).collect();
    assert!(
        by_handle.contains(&("bob".to_string(), Some(Model::Sonnet))),
        "bob's own declaration decides for bob: {by_handle:?}"
    );
    assert!(
        by_handle.contains(&("carol".to_string(), None)),
        "and carol, who declares none, sends no model key at all: {by_handle:?}"
    );
}

#[test]
fn role_resume_carries_its_per_call_model_into_the_resumed_trigger() {
    // The other seam field the CLI flag's removal left uncovered. A resume is a trigger like any
    // other as far as the model is concerned: the call's choice beats the role's declaration.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![role(
            "handle: coder\nsystem_prompt: You are coder.\nmodel: sonnet\n",
        )],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);
    store.create_pending_session("s-coder", "coder").unwrap();

    orchestration::role_resume(
        &c,
        &mut store,
        RoleResumeRequest {
            session: "s-coder",
            body: "carry on",
            channel: None,
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
            model: Some(Model::Fable),
        },
    )
    .unwrap();

    assert_eq!(
        worker.models(),
        vec![Some(Model::Fable)],
        "the call's model beats the role's declared `sonnet` on a resume too"
    );
}

#[test]
fn role_resume_without_a_model_defers_to_the_roles_own_declaration() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![role(
            "handle: coder\nsystem_prompt: You are coder.\nmodel: sonnet\n",
        )],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);
    store.create_pending_session("s-coder", "coder").unwrap();

    orchestration::role_resume(
        &c,
        &mut store,
        RoleResumeRequest {
            session: "s-coder",
            body: "carry on",
            channel: None,
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();

    assert_eq!(worker.models(), vec![Some(Model::Sonnet)]);
}

#[test]
fn a_resume_that_names_no_thread_reports_none_and_keeps_its_session_scope() {
    // **The one verb that can genuinely have no thread** (nxf 6j6v.ntp9). Every COMMISSION goes
    // through `coordinator_commission`, which opens one when it is handed none — but a resume takes
    // its caller's `thread` as it is and must keep doing so: giving it one would hand a `WorkScope`
    // to a trigger that has deliberately had none since nxf 6j6v.303b, and turn an un-surfaced verb
    // into a competitor for the working-tree lease.
    //
    // Both ends of that are pinned here (independent review of PR #378, Test Quality #9 / Integrity
    // #8), because the receipt's own doc is the only other place either is stated: the field reads
    // empty, and — the half that actually matters to a `--json` consumer — an empty one is OMITTED
    // rather than rendered as `"thread": ""`, which a reader would take for an id.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![role("handle: coder\nsystem_prompt: You are coder.\n")],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, 0);
    store.create_pending_session("s-coder", "coder").unwrap();

    let receipt = orchestration::role_resume(
        &c,
        &mut store,
        RoleResumeRequest {
            session: "s-coder",
            body: "carry on",
            channel: None,
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();

    assert_eq!(receipt.thread, "", "a resume into no thread reports none");
    let json = serde_json::to_value(&receipt).expect("the receipt serializes");
    assert!(
        json.get("thread").is_none(),
        "and an empty one is absent from the JSON rather than an empty id: {json}"
    );
}

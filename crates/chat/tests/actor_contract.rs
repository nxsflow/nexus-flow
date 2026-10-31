//! chat's half of the actor-contract detector (nexus-flow-6j6v.d4dw): the public write entry
//! points of `nexus-chat`, held against every boundary of the shared actor rule.
//!
//! The table, the boundaries and the reasoning live once, in
//! [`nxs_test_support::actor_contract`] — read that first. This file is only the wiring.
//!
//! Two things are chat's own and are wired here rather than assumed:
//!
//! * **What ends up on the op.** `send`/`reply`/`ask` attribute `<origin>/<actor>`, so the same
//!   boundary expects a different author here than in flow or memory. `set_expects` takes the
//!   whole handle and puts it on the op unchanged.
//! * **`set_expects` is opener-only**, and its fixture is therefore "the caller is the one who
//!   opened the thread" — the same relation the other 25 entry points have to what they write.

use nexus_chat::error::Result;
use nexus_chat::facade::{self, AskRequest, ReplyRequest, SendRequest};
use nexus_chat::model::{
    Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot, DOMAIN_MESSAGE,
    FIELD_ROOT, KIND_THREAD, OP_OPEN,
};
use nexus_chat::store::ChatStore;
use nxs_foundation::model::Op;
use nxs_test_support::actor_contract::{self, Boundary, Deviation, Outcome, Verdict};

const NOW: &str = "2026-08-09T09:00:00Z";
const ORIGIN: &str = "o";
const CHANNEL: &str = "c-1";
/// The identity that builds every fixture — never a boundary value.
const SEEDER: &str = "o/seeder";

/// A fresh store with one group channel, seeded by [`SEEDER`].
fn fixture() -> ChatStore {
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field(CHANNEL, "kind", "group", SEEDER);
    s.set_channel_field(CHANNEL, "name", "general", SEEDER);
    s.add_member(CHANNEL, SEEDER, SEEDER);
    s.add_member(CHANNEL, "o/reader", SEEDER);
    s
}

/// A posted message in [`CHANNEL`], for `reply` to aim at.
fn seed_message(s: &mut ChatStore) -> String {
    s.post_message(&MessageEnvelope {
        origin: ORIGIN.into(),
        channel_id: CHANNEL.into(),
        sender: SEEDER.into(),
        kind: MessageKind::Task,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: None,
        refs: Refs::default(),
        body: "seed".into(),
    })
}

/// A thread whose OPENER is `handle`, delivered the only way such a thread can carry an arbitrary
/// opener: as a foreign op over sync. `threads.opener` is `op.author` (6j6v.aym3), and the local
/// substrate refuses to emit an op with an unattributable one, so `apply` is the only door.
///
/// The reducer then decides whether the thread materializes at all — which is exactly what makes
/// the `not_found` deviation below true rather than convenient.
fn thread_opened_by(s: &mut ChatStore, thread_id: &str, handle: &str) {
    s.apply(&[Op {
        op_id: format!("01KZACTORCONTRACT{thread_id:0>8}"),
        lamport: 7,
        site: 9,
        domain: DOMAIN_MESSAGE.into(),
        target_kind: KIND_THREAD.into(),
        target_id: thread_id.into(),
        field: FIELD_ROOT.into(),
        op_type: OP_OPEN.into(),
        value: Some(
            serde_json::to_string(&ThreadRoot {
                origin: ORIGIN.into(),
                channel_id: CHANNEL.into(),
                opener: handle.into(),
                created: NOW.into(),
                parent: None,
            })
            .unwrap(),
        ),
        author: handle.into(),
        wall_clock: NOW.into(),
        key_id: None,
        sig: None,
    }]);
}

/// What the call did, plus the author of every op IT appended. Reported on BOTH paths: a refusal
/// that had already written is a defect the verdict alone would not show.
fn outcome<T>(s: &ChatStore, before: usize, r: Result<T>) -> Outcome {
    // Collected, not lazy: `export()` hands back an owned Vec, so an iterator over a slice of
    // it would outlive the temporary.
    let appended = || -> Vec<String> {
        s.export()[before..]
            .iter()
            .map(|o| o.author.clone())
            .collect()
    };
    match r {
        Ok(_) => Outcome::accepted(appended()),
        Err(e) => Outcome::rejected(e.kind.as_str(), appended()),
    }
}

/// The author `send`/`reply`/`ask` stamp: the actor's stored form under this origin.
fn prefixed(actor: &str) -> String {
    format!("{ORIGIN}/{actor}")
}

type Case = (
    &'static str,
    fn(&str) -> String,
    &'static [Deviation],
    Box<dyn FnMut(&Boundary) -> Outcome>,
);

/// `set_expects` answers the three refused boundaries as `not_found`, not `validation` — see the
/// reason. Declared rather than quietly expected: this is the one entry point whose actor check is
/// not the first thing a caller meets.
const SET_EXPECTS_DEVIATIONS: &[Deviation] = &[
    Deviation {
        boundary: "empty",
        verdict: Verdict::Rejected("not_found"),
        reason: SET_EXPECTS_REASON,
    },
    Deviation {
        boundary: "whitespace_only",
        verdict: Verdict::Rejected("not_found"),
        reason: SET_EXPECTS_REASON,
    },
    Deviation {
        boundary: "invisible_only",
        verdict: Verdict::Rejected("not_found"),
        reason: SET_EXPECTS_REASON,
    },
];

const SET_EXPECTS_REASON: &str = "\
    `set_expects` is opener-only, and a thread whose opener names no one CANNOT EXIST: `opener` is \
    `op.author` (6j6v.aym3), the substrate refuses to emit an unattributable author, and the \
    reducer refuses to fold a foreign one (store-don't-fold). So the caller never reaches the \
    actor check — there is no thread to be the opener of, and the honest answer is `not_found`. \
    The `validate_author` call in `set_expects` is the backstop for that unreachable state, and \
    `a_valid_handle_that_is_not_the_opener_is_forbidden_before_anything_else` pins the ORDER that \
    makes it a backstop rather than the first gate.";

/// One closure per public write entry point — also the covered set the completeness gate holds
/// against the source.
fn cases() -> Vec<Case> {
    vec![
        (
            "send",
            prefixed,
            &[],
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::send(
                    &mut s,
                    SendRequest {
                        now: NOW,
                        origin: ORIGIN,
                        actor: b.actor,
                        channel: CHANNEL,
                        body: "under test",
                        kind: MessageKind::Task,
                        priority: Priority::Normal,
                        disposition: Disposition::InTurn,
                        thread: None,
                        refs: Refs::default(),
                    },
                );
                outcome(&s, before, r)
            }),
        ),
        (
            "reply",
            prefixed,
            &[],
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let target = seed_message(&mut s);
                let before = s.export().len();
                let r = facade::reply(
                    &mut s,
                    ReplyRequest {
                        now: NOW,
                        origin: ORIGIN,
                        actor: b.actor,
                        target: &target,
                        body: "under test",
                        kind: MessageKind::Task,
                        priority: Priority::Normal,
                        disposition: Disposition::InTurn,
                        refs: Refs::default(),
                        if_unanswered: false,
                    },
                );
                outcome(&s, before, r)
            }),
        ),
        (
            // `facade::ask` is the thin wrapper (nxf 6j6v.a71h): it passes `parent: None` to
            // `ask_under`, which is where the actor is validated and every op is written. The gate
            // pins the entry point that OWNS the rule, so the pin moved with it.
            "ask_under",
            prefixed,
            &[],
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::ask_under(
                    &mut s,
                    AskRequest {
                        now: NOW,
                        origin: ORIGIN,
                        actor: b.actor,
                        channel: CHANNEL,
                        body: "under test",
                        expect: &["o/reader".to_string()],
                        deadline: None,
                        kind: MessageKind::Question,
                        priority: Priority::Normal,
                        refs: Refs::default(),
                    },
                    None,
                );
                outcome(&s, before, r)
            }),
        ),
        (
            // `name_thread` (nxf 6j6v.e76c) takes the whole handle and puts it on the op unchanged,
            // exactly as `set_expects` below does — and unlike that one it is NOT opener-gated, so
            // the actor check IS the first thing a caller meets and there is nothing to deviate.
            // The fixture's thread is opened by the SEEDER for that reason: what is under test is
            // the boundary VALUE of the caller, not its standing.
            "name_thread",
            str::to_owned,
            &[],
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                thread_opened_by(&mut s, "t-1", SEEDER);
                let before = s.export().len();
                let r = facade::name_thread(&mut s, NOW, b.actor, "t-1", "a readable name");
                outcome(&s, before, r)
            }),
        ),
        (
            "set_expects",
            str::to_owned,
            SET_EXPECTS_DEVIATIONS,
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                thread_opened_by(&mut s, "t-1", b.actor);
                let before = s.export().len();
                let r = facade::set_expects(&mut s, NOW, b.actor, "t-1", &["o/reader".to_string()]);
                outcome(&s, before, r)
            }),
        ),
    ]
}

#[test]
fn every_write_entry_point_honours_the_actor_contract() {
    for (entry, author_of, deviations, mut call) in cases() {
        actor_contract::assert_actor_contract_with(
            &format!("nexus_chat::facade::{entry}"),
            deviations,
            author_of,
            &mut call,
        );
    }
}

#[test]
fn the_pinned_set_is_every_entry_point_that_validates_an_actor() {
    let covered: Vec<&str> = cases().iter().map(|(name, ..)| *name).collect();
    actor_contract::assert_every_entry_point_is_covered(
        "nexus-chat",
        &nxs_test_support::verb_seam::crate_relative(env!("CARGO_MANIFEST_DIR"), "src"),
        &covered,
        &[],
    );
}

/// The ORDER inside `set_expects`, which the boundary table cannot reach: the opener check runs
/// first, so a caller with no standing hears the specific `forbidden` rather than a complaint about
/// its handle. Moving the actor check ahead of it would flip this to `validation` — a behavioural
/// change at a consumed entry point that nothing else in the chain can see.
#[test]
fn a_valid_handle_that_is_not_the_opener_is_forbidden_before_anything_else() {
    let mut s = fixture();
    thread_opened_by(&mut s, "t-1", "o/opener");
    let err = facade::set_expects(&mut s, NOW, "o/someone-else", "t-1", &[])
        .expect_err("only the opener may re-declare");
    assert_eq!(err.kind.as_str(), "forbidden");
}

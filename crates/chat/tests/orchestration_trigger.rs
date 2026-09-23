//! `orchestration::Ctx` + the trigger/compose path (nxf 6j6v.p3vn).
//!
//! The first cut of the seam: `cli.rs::trigger_role` composed a role's system prompt and called the
//! worker while reading the declaration directory, the filesystem, `origin()` and `NXC_*` on the
//! way. The decision logic moves here and takes all of that explicitly, so a library caller can
//! drive it.

use std::sync::Mutex;

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::orchestration::{
    self, ChainMove, CheckedHop, Commission, Ctx, RoleSpawn, TriggerAdmission,
};
use nexus_chat::role::{Model, RoleDecl, WorkingTree};
use nexus_chat::store::ChatStore;
use nexus_chat::worker::{Coordinator, TriggerRequest, Worker};
use nexus_chat::working_tree::WORKING_TREE_LEASE_BOUND;

/// A worker that records what it was handed instead of spawning anything — the trigger path's
/// observable output is exactly the `TriggerRequest` it produces.
#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl RecordingWorker {
    fn only(&self) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "expected exactly one trigger");
        seen[0].clone()
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}

/// The store the trigger path writes its chain depth into (nxf 6j6v.m48m). In memory and empty:
/// these tests drive `trigger_role` directly rather than through a verb, so nothing has minted a
/// session row — which is itself the case worth passing through here, since a depth recorded against
/// a row that does not exist must be a no-op rather than a failed trigger.
fn store() -> ChatStore {
    ChatStore::open_in_memory(1)
}

/// The trigger inputs these tests share, as a value each one varies a field or two of
/// (`RoleSpawn { message: "x", ..base_spawn(decl, hop) }`).
///
/// `hop` comes in already checked because that is the only way to get one — `trigger_role` takes a
/// [`CheckedHop`], not a `u32`, so a test cannot hand it a depth nobody guarded either (PR #267
/// review, Integrity & Robustness #1). Obtain it with [`checked_hop`].
fn base_spawn<'a>(decl: &'a RoleDecl, hop: CheckedHop) -> RoleSpawn<'a> {
    RoleSpawn {
        decl,
        internal_session: "s-1".to_string(),
        resume_real: None,
        message: "go",
        coordinator: Coordinator::Persona,
        model_override: None,
        hop,
        chain: ChainMove::Deeper,
        reply_thread: None,
        thread: None,
        priority: Priority::Normal,
        queued_since: None,
    }
}

/// This context's depth, through the real resolution the verbs use. These contexts carry no ambient
/// session, so it resolves to whatever `ctx.hop` claims.
fn checked_hop(c: &Ctx, store: &ChatStore) -> CheckedHop {
    orchestration::resolve_hop(c, store).expect("under the cap")
}

/// These suites never assert on SCHEDULING, and since the timer became an injected value
/// (PR #269 review, Code Quality #3) they no longer have to tolerate whatever `NXC_TIMER`
/// happens to be — unset, that used to mean a real `at` subprocess per armed deadline.
static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// A `Ctx` over the given definitions and worker, with the ambient values a host would inject.
fn ctx<'a>(
    defs: &'a Definitions,
    worker: &'a dyn Worker,
    project_claude_md: Option<&'a str>,
    hop: u32,
) -> Ctx<'a> {
    Ctx {
        now: "2026-07-31T10:00:00Z",
        origin: "o",
        actor: "a",
        session: None,
        hop,
        defs,
        worker,
        timer: &NO_TIMER,
        namer: &nexus_chat::naming::DryNamer,
        db_path: "/w/.nxs/db.sqlite",
        project_claude_md,
        module_primes: None,
        machines: None,
    }
}

// ---- model precedence ---------------------------------------------------------------------

#[test]
fn resolve_model_precedence_is_call_then_step_then_role() {
    // All eight combinations, because the precedence is the kind of rule that silently inverts
    // during a refactor and only shows up as "the wrong model ran" in a live session.
    use Model::{Fable, Opus, Sonnet};
    let cases = [
        (None, None, None, None),
        (None, None, Some(Opus), Some(Opus)),
        (None, Some(Sonnet), None, Some(Sonnet)),
        (None, Some(Sonnet), Some(Opus), Some(Sonnet)),
        (Some(Fable), None, None, Some(Fable)),
        (Some(Fable), None, Some(Opus), Some(Fable)),
        (Some(Fable), Some(Sonnet), None, Some(Fable)),
        (Some(Fable), Some(Sonnet), Some(Opus), Some(Fable)),
    ];
    for (call, step, role_model, want) in cases {
        assert_eq!(
            orchestration::resolve_model(call, step, role_model),
            want,
            "call={call:?} step={step:?} role={role_model:?}"
        );
    }
}

#[test]
fn a_call_override_beats_the_roles_own_declared_model() {
    let defs = Definitions::new(
        vec![role("handle: coder\nsystem_prompt: C\nmodel: sonnet\n")],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);

    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();
    orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            message: "do it",
            coordinator: Coordinator::Persona,
            model_override: Some(Model::Fable),
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();

    assert_eq!(worker.only().role.model, Some(Model::Fable));
}

#[test]
fn a_declared_stage_reaches_the_spawn_as_its_model_and_a_call_override_still_beats_it() {
    // The `stage:` band's END-TO-END wiring (nxf 6j6v.p6m1; PR #295 review, Test Quality #2). Both
    // halves were unit-tested apart — `RoleDecl::effective_model` maps the band, `resolve_model`
    // orders the precedence — but nothing pinned that `trigger_role` actually threads a band-only
    // role through to the spawn. A role author who writes `stage: junior` and gets whatever the SDK
    // defaults to has no way to tell from either unit test.
    let defs = Definitions::new(
        vec![role("handle: coder\nsystem_prompt: C\nstage: junior\n")],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();
    assert_eq!(
        decl.model, None,
        "the band is the ONLY model this role declares"
    );

    orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            message: "do it",
            coordinator: Coordinator::Persona,
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();
    assert_eq!(
        worker.only().role.model,
        Some(Model::Sonnet),
        "`stage: junior` is what the session runs on"
    );

    // And the precedence holds through the same path: the call still wins over the band, exactly as
    // it wins over a declared `model:` in the test above.
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            message: "do it",
            coordinator: Coordinator::Persona,
            model_override: Some(Model::Fable),
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();
    assert_eq!(worker.only().role.model, Some(Model::Fable));
}

// ---- prompt composition -------------------------------------------------------------------

/// A [`ModulePrimes`] provider that hands back one fixed block per module — the seam hole nxf
/// 6j6v.k8zq opened, filled with something a test can find by name.
///
/// In production `crates/nxs` fills it by running `nxf prime` / `nxm prime`; nothing about the
/// composition depends on where the text came from, which is the point of the text being opaque
/// here.
#[derive(Debug)]
struct FixedPrimes;

impl nexus_chat::facade::ModulePrimes for FixedPrimes {
    fn module_primes(
        &self,
        services: nexus_chat::role::PrimeServices,
        db_path: &str,
    ) -> nexus_chat::error::Result<Vec<nexus_chat::facade::ModulePrime>> {
        // The workspace the CALLER resolved reaches the provider (review of PR #379, Code Quality
        // #1). Echoed into the flow block so the layering test below can see that it arrived,
        // rather than the parameter being accepted and dropped.
        assert!(
            !db_path.is_empty(),
            "a provider is handed the resolved workspace db, never left to guess it"
        );
        let mut out = Vec::new();
        if services.flow {
            out.push(nexus_chat::facade::ModulePrime {
                module: "flow".into(),
                text: format!("FLOW-BLOCK db={db_path}"),
            });
        }
        if services.memory {
            out.push(nexus_chat::facade::ModulePrime {
                module: "memory".into(),
                text: "MEMORY-BLOCK".into(),
            });
        }
        Ok(out)
    }
}

#[test]
fn trigger_composes_the_prime_block_then_claude_md_then_the_roles_own_prompt() {
    // The layering is load-bearing and was decided across three tickets: cvsp's prime block first,
    // then CLAUDE.md, then the role's own prompt. Pin the ORDER, not the exact prose.
    //
    // A FOURTH layer stood at the end until 6j6v.dvyq §3 — the run's per-role workflow block — and
    // two tests beside this one covered it (that omitting the nomination produced a byte-identical
    // prompt, and that the block came off the catalogue rather than the filesystem). All three went
    // with the run engine; what is left is the layering that survives it.
    //
    // WHAT LAYER 1 IS made of changed with nxf 6j6v.k8zq — it was `nxc_usage_block`'s four
    // hand-written verbs and is the composed `nxs prime --persona` text now — so the marker this
    // pins the order by comes from the provider rather than from a literal in the source. Chat's
    // own half is absent here because these definitions were built IN MEMORY and have no folder for
    // `prime_for` to read a roster and an address book from; the CLI and `Engine` paths both carry
    // one, and `tests/a_persona_reads_what_the_suite_knows.rs` is where that half is proven.
    let defs = Definitions::new(
        vec![role("handle: coder\nsystem_prompt: ROLE-PROMPT\n")],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let mut c = ctx(&defs, &worker, Some("CLAUDE-MD-BODY"), 0);
    c.module_primes = Some(&FixedPrimes);

    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();
    orchestration::trigger_role(&c, &mut store, base_spawn(decl, hop)).unwrap();

    let prompt = worker.only().role.system_prompt;
    let at = |needle: &str| {
        prompt
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing from composed prompt:\n{prompt}"))
    };
    assert!(
        at("FLOW-BLOCK") < at("MEMORY-BLOCK"),
        "the suite's modules arrive in registry order:\n{prompt}"
    );
    assert!(
        at("MEMORY-BLOCK") < at("CLAUDE-MD-BODY"),
        "the prime block anchors everything after it, including project conventions"
    );
    assert!(at("CLAUDE-MD-BODY") < at("ROLE-PROMPT"));
}

/// The filter, at the seam it is read from: a module a persona excludes is never even asked for its
/// block, so a provider that would have produced one is not paid for it (nxf 6j6v.k8zq).
#[test]
fn a_persona_that_excludes_the_board_never_asks_for_it() {
    let defs = Definitions::new(
        vec![role(
            "handle: reviewer\nsystem_prompt: ROLE-PROMPT\nprime:\n  flow: false\n",
        )],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let mut c = ctx(&defs, &worker, None, 0);
    c.module_primes = Some(&FixedPrimes);

    let hop = checked_hop(&c, &store);
    let decl = defs.role("reviewer").unwrap();
    orchestration::trigger_role(&c, &mut store, base_spawn(decl, hop)).unwrap();

    let prompt = worker.only().role.system_prompt;
    assert!(
        !prompt.contains("FLOW-BLOCK"),
        "`flow: false` removes the section for this persona:\n{prompt}"
    );
    assert!(
        prompt.contains("MEMORY-BLOCK"),
        "…and leaves what it did not name alone:\n{prompt}"
    );
}

// ---- env / depth guard --------------------------------------------------------------------

#[test]
fn trigger_advances_the_hop_for_the_next_hop() {
    // Review finding #1 on 6j6v.zenf: the guard was checked but never propagated, so a real
    // A->B->C chain never advanced the counter and the cap could only fire if someone hand-set it.
    // The trigger stamps hop+1 so the NEXT hop reads one higher.
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 7);

    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();
    orchestration::trigger_role(&c, &mut store, base_spawn(decl, hop)).unwrap();

    let env = worker.only().env;
    let get = |k: &str| {
        env.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("{k} missing from {env:?}"))
    };
    assert_eq!(get("NXC_HOP"), "8", "the next hop reads one higher");
    assert_eq!(get("NXC_ORIGIN"), "o");
    assert_eq!(get("NXC_DB"), "/w/.nxs/db.sqlite");
}

#[test]
fn trigger_passes_the_session_and_resume_through_untouched() {
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);

    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();
    orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            internal_session: "s-42".to_string(),
            resume_real: Some("real-abc".to_string()),
            message: "continue",
            coordinator: Coordinator::Persona,
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();

    let req = worker.only();
    assert_eq!(req.internal_session, "s-42");
    assert_eq!(req.resume_real.as_deref(), Some("real-abc"));
    assert_eq!(req.message, "continue");
    assert_eq!(req.role.handle, "coder");
}

#[test]
fn trigger_threads_reply_thread_through_to_the_worker() {
    // The one funnel (nxf 6j6v.7e9d, ticket 8): `RoleSpawn::reply_thread` is the guidance half
    // (ticket 6, what the composed prompt tells the role), and `TriggerRequest::reply_thread` is
    // the enforcement half (this ticket, what the sidecar's teardown acts on) — `trigger_role` is
    // the only place that has to carry one into the other, and nothing else pins that it still
    // does after a refactor of either side.
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);

    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();
    orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            reply_thread: Some("thread-xyz"),
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();
    assert_eq!(worker.only().reply_thread.as_deref(), Some("thread-xyz"));

    // And the common case: no obligation declared, nothing carried.
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    orchestration::trigger_role(&c, &mut store, base_spawn(decl, hop)).unwrap();
    assert_eq!(worker.only().reply_thread, None);
}

// ---- expects_reply_from (nxf 6j6v.jepk) -----------------------------------------------------

fn trigger_req<'a>(role: &'a str, body: &'a str, thread: Option<&'a str>) -> Commission<'a> {
    Commission {
        machine: None,
        role,
        body,
        channel: None,
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread,
        refs: Refs::default(),
        model: None,
        continue_session: None,
    }
}

#[test]
fn a_commission_declares_the_triggered_role_expected_on_its_own_thread() {
    // The first of the epic's three moves that make "this role session is finished" a fact rather
    // than a guess: every role trigger now declares, on its thread, that it expects a reply from
    // the role it just summoned — the fact `ask` and the declared-channel fan-out already leave,
    // and the one a bare direct-persona trigger left nobody to observe (nxf 6j6v.jepk).
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut s = store();
    let c = ctx(&defs, &worker, None, 0);

    // A thread the caller already has — the shape a real `--thread` reuse takes (e.g. a role
    // triggering a second role INTO a thread `nxc reply` is already routing through).
    let tid = s.mint_thread_id();
    s.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "dm:a~coder".to_string(),
            opener: "o/a".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/a",
    );

    orchestration::coordinator_commission(&c, &mut s, trigger_req("coder", "go", Some(&tid)))
        .unwrap();

    let q = s.thread_quorum(&tid, c.now).unwrap().unwrap();
    assert_eq!(q.expects, vec!["o/coder".to_string()], "{q:?}");
    assert!(
        q.replied.is_empty(),
        "the just-triggered role has not answered yet: {q:?}"
    );
    assert_eq!(q.outstanding, vec!["o/coder".to_string()]);
}

#[test]
fn a_commission_does_not_overwrite_a_thread_that_already_expects_a_quorum() {
    // `ask`/the declared-channel fan-out may have put a whole multi-handle quorum on a thread
    // before a role triggers ANOTHER role into that same thread via `--thread` (looping in a
    // second reviewer while a board is open). Setting the new trigger's own expectation
    // unconditionally would silently collapse that quorum down to one handle.
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut s = store();
    let c = ctx(&defs, &worker, None, 0);

    let tid = s.mint_thread_id();
    s.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "decl:review".to_string(),
            opener: "o/pm".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/pm",
    );
    let existing = serde_json::to_string(&["o/bob", "o/carol"]).unwrap();
    s.set_expects_reply_from(&tid, &existing, "o/pm");

    orchestration::coordinator_commission(
        &c,
        &mut s,
        trigger_req("coder", "please also weigh in", Some(&tid)),
    )
    .unwrap();

    let q = s.thread_quorum(&tid, c.now).unwrap().unwrap();
    assert_eq!(
        q.expects,
        vec!["o/bob".to_string(), "o/carol".to_string()],
        "the pre-existing quorum survives untouched: {q:?}"
    );
}

#[test]
fn a_commission_that_names_no_thread_gets_one_opened_for_it_and_the_expectation_declared_on_it() {
    // **INVERTED by nxf 6j6v.ntp9**, and the inversion is the point of that item. This used to
    // assert that a commission naming no thread posted none — "nothing regresses here, there is
    // simply nothing to declare an expectation ON". That was true and it was the defect: a role
    // summoned with nothing to answer into, nobody expecting it and no obligation for the sidecar's
    // teardown to settle is nxf 6j6v.rp8k's shape, and the only thing keeping it off the real paths
    // was that `crate::surface::send_to` remembered to open a thread first — a property of the CLI
    // and app surface, not of the verb (nxf 41j0.vhsk).
    //
    // The coordinator opens it now, so there is no caller left that can omit it.
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut s = store();
    let c = ctx(&defs, &worker, None, 0);

    let receipt =
        orchestration::coordinator_commission(&c, &mut s, trigger_req("coder", "go", None))
            .unwrap();

    assert!(!receipt.thread.is_empty(), "the coordinator opened one");
    assert_eq!(
        s.message_channel(&receipt.message_id)
            .expect("the message was persisted")
            .1
            .as_deref(),
        Some(receipt.thread.as_str()),
        "and the message landed in it"
    );
    let q = s.thread_quorum(&receipt.thread, c.now).unwrap().unwrap();
    assert_eq!(
        q.expects,
        vec!["o/coder".to_string()],
        "and the summoned role is what it expects a reply from: {q:?}"
    );
}

// ---- the prime-block obligation (nxf 6j6v.enrs) ---------------------------------------------
//
// The guidance half of the bookkeeping the suite above pins: the summoned role is not just made to
// owe a reply in the store, it is TOLD so in its own composed system prompt — gated on EXACTLY the
// same condition `coordinator_commission` uses to decide whether to WRITE `expects_reply_from` (PR
// review, 2026-08-13, fixing this suite's own original gate that checked `receipt.thread_id.is_some()`
// alone): a thread must exist AND carry no expectation yet. `a_commission_does_not_overwrite_a_
// thread_that_already_expects_a_quorum` above proves the bookkeeping half of that; the sibling test
// below proves the guidance half agrees with it on the SAME scenario.

#[test]
fn a_commission_primes_the_summoned_role_with_the_reply_obligation_naming_its_thread() {
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut s = store();
    let c = ctx(&defs, &worker, None, 0);

    let tid = s.mint_thread_id();
    s.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "dm:a~coder".to_string(),
            opener: "o/a".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/a",
    );

    orchestration::coordinator_commission(&c, &mut s, trigger_req("coder", "go", Some(&tid)))
        .unwrap();

    let prompt = worker.only().role.system_prompt;
    assert!(
        prompt.contains(&tid),
        "the composed prompt must name the concrete thread id, got:\n{prompt}"
    );
    assert!(
        prompt.contains(&format!("nxc reply --thread {tid}")),
        "the composed prompt must give the exact reply command, got:\n{prompt}"
    );
}

#[test]
fn a_commission_that_names_no_thread_is_still_primed_with_the_obligation_on_the_one_it_was_given() {
    // Mirrors `a_commission_that_names_no_thread_gets_one_opened_for_it_…` one section up: same
    // commission, same inversion, the OTHER half of it. The two halves are driven by ONE boolean by
    // design (`declared_here`), so a test that pinned the register without pinning the guidance
    // would let them drift into telling a role it owes a reply the register disagrees about.
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut s = store();
    let c = ctx(&defs, &worker, None, 0);

    let receipt =
        orchestration::coordinator_commission(&c, &mut s, trigger_req("coder", "go", None))
            .unwrap();

    let spawned = worker.only();
    assert!(
        // On the OBLIGATION's own word, not on the reply command: the composed prime block names
        // `nxc reply --thread <id>` in every primed prompt (`PRIME_COMMANDS`, and before nxf
        // 6j6v.k8zq the hand-written `nxc_usage_block` did the same), so the command alone does not
        // distinguish "you owe a reply here" from "this is how replying works" (mutation-checked:
        // asserting the command still passes with the obligation present, which is what makes it
        // the wrong string to key on).
        spawned.role.system_prompt.contains("Obligation: thread"),
        "the coordinator opened a thread, so there IS an obligation to prime, got:\n{}",
        spawned.role.system_prompt
    );
    assert_eq!(
        spawned.reply_thread.as_deref(),
        Some(receipt.thread.as_str()),
        "and it is the thread the coordinator opened — which is what lets the sidecar's teardown \
         net settle a debt this commission could not previously register at all"
    );
}

#[test]
fn a_commission_into_a_thread_that_already_expects_a_quorum_primes_no_obligation_either() {
    // The Important review finding on this ticket: gating the GUIDANCE on `receipt.thread_id.
    // is_some()` alone (this test's original form) disagreed with the BOOKKEEPING gate one
    // function up (`!already_expects` too) — a role triggered into an existing board would be TOLD
    // "thread X is waiting on your reply" while its handle was never added to `expects_reply_from`.
    // Ticket 8's `--if-unanswered` discharge reads that same register, so it would silently find
    // nothing owed for a role that believed otherwise — the exact "thread hangs silently" failure
    // mode this epic exists to remove, reintroduced on this one path. Same scenario as
    // `a_commission_does_not_overwrite_a_thread_that_already_expects_a_quorum` above; this pins the
    // guidance side of the SAME fixture so the two gates cannot drift apart again unnoticed.
    let defs = Definitions::new(vec![role("handle: coder\nsystem_prompt: C\n")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut s = store();
    let c = ctx(&defs, &worker, None, 0);

    let tid = s.mint_thread_id();
    s.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "decl:review".to_string(),
            opener: "o/pm".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/pm",
    );
    let existing = serde_json::to_string(&["o/bob", "o/carol"]).unwrap();
    s.set_expects_reply_from(&tid, &existing, "o/pm");

    orchestration::coordinator_commission(
        &c,
        &mut s,
        trigger_req("coder", "please also weigh in", Some(&tid)),
    )
    .unwrap();

    let prompt = worker.only().role.system_prompt;
    assert!(
        // On the OBLIGATION's own word, not on the reply command: the composed prime block names
        // `nxc reply --thread <id>` in every primed prompt (`PRIME_COMMANDS`, and before nxf
        // 6j6v.k8zq the hand-written `nxc_usage_block` did the same), so the command alone does not
        // distinguish "you owe a reply here" from "this is how replying works" (mutation-checked:
        // asserting the command still passes with the obligation present, which is what makes it
        // the wrong string to key on).
        !prompt.contains("Obligation: thread"),
        "a trigger into a thread that already carries someone else's quorum must prime no \
obligation — the register never recorded one for this role, so telling it otherwise would be a \
lie, got:\n{prompt}"
    );
}

// ---- the working-tree lease (nxf 6j6v.303b) -------------------------------------------------
//
// The gate in `trigger_role`: acquire, inherit, or queue. Everything here drives `trigger_role`
// directly, so the evidence is the RecordingWorker's log — a queued trigger is one the worker never
// heard about — plus the lease/queue tables the store exposes.
//
// `working_tree: shared` is the default every declaration in every other test in this file reads
// as, which is why none of them had to change: the gate's first question is answered "no" for them
// before anything is read.

/// A role that declares it needs the working copy to itself.
fn exclusive_role(handle: &str) -> RoleDecl {
    role(&format!(
        "handle: {handle}\nsystem_prompt: {handle}\nworking_tree: exclusive\n"
    ))
}

/// A `RoleSpawn` for `decl` that resolves to `thread:<thread>` — the claim area a direct persona
/// trigger takes (`send --to <persona>` opens the thread before it triggers, nxf 6j6v.jepk).
///
/// Both thread fields are set, which is exactly what `coordinator_commission` does on a FRESH
/// thread: it
/// registers the obligation on the same thread the trigger belongs to. [`spawn_belonging_to`] is the
/// other shape — belongs-to only, no obligation — which is what the fan-out and a re-used board
/// produce.
fn spawn_in_thread<'a>(
    decl: &'a RoleDecl,
    hop: CheckedHop,
    session: &str,
    thread: &'a str,
) -> RoleSpawn<'a> {
    RoleSpawn {
        internal_session: session.to_string(),
        reply_thread: Some(thread),
        thread: Some(thread),
        ..base_spawn(decl, hop)
    }
}

/// A `RoleSpawn` that BELONGS to a thread but registered no obligation on it — the shape
/// `open_declared_channel_and_fan_out` produces for every member (the quorum is `facade::ask`'s, not
/// the trigger's), and the shape `coordinator_commission` produces when it triggers into a thread
/// that
/// already carries somebody else's quorum.
fn spawn_belonging_to<'a>(
    decl: &'a RoleDecl,
    hop: CheckedHop,
    session: &str,
    thread: &'a str,
) -> RoleSpawn<'a> {
    RoleSpawn {
        internal_session: session.to_string(),
        reply_thread: None,
        thread: Some(thread),
        ..base_spawn(decl, hop)
    }
}

#[test]
fn a_second_exclusive_trigger_in_a_different_scope_is_queued_and_never_reaches_the_worker() {
    // Definition of done #1: an `exclusive` trigger that meets a held lease does NOT spawn, and says
    // so in its answer. Two DIFFERENT claim areas (two threads = two conversations), one working
    // copy.
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    let first =
        orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", "th-a"))
            .unwrap();
    let second =
        orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-2", "th-b"))
            .unwrap();

    assert_eq!(
        first,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(
        second,
        TriggerAdmission::Queued {
            behind: Some("thread:th-a".to_string()),
            position: 1,
            // NOTHING to report — these suites run a `DryTimer`, which schedules successfully, and
            // no park was attempted on the way in. A finding here would mean either that the parked
            // commission has no clock behind it (nxf 6j6v.fabb) or that the expired holder's work
            // could not be saved before the copy moved (nxf 6j6v.8bv9); the caller reads both on
            // `warnings`.
            findings: Vec::new(),
        },
        "the second names the holder by its scope key and its own 1-based place in the queue"
    );
    // The load-bearing half: the worker never heard about the second one at all. `only()` asserts
    // exactly one trigger, which is the whole claim.
    assert_eq!(worker.only().internal_session, "s-1");
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        Some("thread:th-a".to_string())
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 1);

    // And what was queued is the trigger's raw INPUTS, ready to be re-composed at release time —
    // not a frozen prompt (see `QueuedTrigger`'s own doc for why that distinction is load-bearing).
    let head = store.peek_working_tree_queue().unwrap().unwrap();
    assert_eq!(head.scope_key, "thread:th-b");
    assert_eq!(head.role, "coder");
    assert_eq!(head.session, "s-2");
    assert_eq!(head.thread.as_deref(), Some("th-b"));
    assert_eq!(head.message, "go");
}

#[test]
fn a_second_exclusive_trigger_in_the_same_scope_inherits_and_spawns_without_waiting() {
    // Definition of done #2: the holder is the CHAIN, not the session. A chain that already holds
    // the working copy passes straight through — code -> review -> mitigate -> merge must not queue
    // behind itself. Two different SESSIONS, one thread: one claim area.
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    let first =
        orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", "th-a"))
            .unwrap();
    let second =
        orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-2", "th-a"))
            .unwrap();

    assert_eq!(
        first,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(
        second,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        },
        "inheriting is indistinguishable from acquiring to the caller, deliberately"
    );
    assert_eq!(worker.seen.lock().unwrap().len(), 2, "both spawned");
    assert_eq!(
        store.working_tree_queue_len().unwrap(),
        0,
        "nothing was queued"
    );
}

#[test]
fn a_shared_role_is_never_queued_even_while_another_chain_holds_the_lease() {
    // Definition of done #3: `shared` is untouched — no lease, no wait, no queue, whatever else is
    // going on. This is what every existing declaration reads as.
    let defs = Definitions::new(
        vec![
            exclusive_role("coder"),
            role("handle: pm\nsystem_prompt: P\n"),
        ],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);

    orchestration::trigger_role(
        &c,
        &mut store,
        spawn_in_thread(defs.role("coder").unwrap(), hop, "s-1", "th-a"),
    )
    .unwrap();
    let shared = orchestration::trigger_role(
        &c,
        &mut store,
        spawn_in_thread(defs.role("pm").unwrap(), hop, "s-2", "th-b"),
    )
    .unwrap();

    assert_eq!(
        shared,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(worker.seen.lock().unwrap().len(), 2);
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        Some("thread:th-a".to_string()),
        "the `shared` trigger did not take, displace or renew anybody's lease"
    );
}

#[test]
fn an_exclusive_trigger_that_resolves_to_session_scope_takes_no_lease_at_all() {
    // The deliberate hole, pinned so nobody closes it by accident (owner ruling, 2026-08-12).
    //
    // `resolve_scope` answers `session:<id>` for a trigger carrying neither a run nor a thread —
    // a thread-less trigger (which mints no thread at all: nxf 6j6v.jepk's finding, confirmed
    // twice), the ephemeral synthesizer, a bare resume. Such a trigger has no thread to declare an
    // expectation on, so nothing can ever discharge one, so nothing would ever RELEASE a lease it
    // took — and the queue behind it would stall until the two-hour backstop. A lease nobody can
    // release is worse than no lease, so this path is deliberately left exactly as it is today.
    //
    // (The CLI entrance that produced this shape, `send --role`, was removed in favour of
    // `send --to` — which always opens a thread — by 6j6v.dvyq §3. What is left reaches it only
    // from inside `orchestration`; not a path worth retrofitting, only one worth being explicit
    // about.)
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    // No `reply_thread`, no run: session scope.
    let first = orchestration::trigger_role(&c, &mut store, base_spawn(decl, hop)).unwrap();
    let second = orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            internal_session: "s-2".to_string(),
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();

    assert_eq!(
        first,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(
        second,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        },
        "two session-scoped triggers do not contend, because neither ever claimed anything"
    );
    assert_eq!(worker.seen.lock().unwrap().len(), 2);
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        None,
        "an exclusive role with no releasable chain must leave the lease table untouched"
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
}

#[test]
fn a_channel_declared_exclusive_makes_even_a_shared_role_take_the_lease() {
    // The CHANNEL's declaration counts too, not just the role's (epic §4: the example declares
    // `working_tree: exclusive` on the `coding` channel). A reviewer that says nothing about the
    // working copy, fanned into a board that does, still needs it — the work is exclusive because of
    // what it is part of, not because of who does it.
    let defs = Definitions::new(
        vec![role("handle: reviewer\nsystem_prompt: R\n")],
        vec![channel(
            "name: coding\nmembers: [reviewer]\nworking_tree: exclusive\n",
        )],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("reviewer").unwrap();
    assert_eq!(
        decl.working_tree,
        WorkingTree::Shared,
        "the ROLE says nothing — the channel is the only reason a lease is needed here"
    );

    // A thread that lives in the declared channel's own substrate id, which is how a declared
    // channel is recognised everywhere in this crate.
    let tid = store.mint_thread_id();
    store.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "decl:coding".to_string(),
            opener: "o/a".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/a",
    );

    let first =
        orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", &tid))
            .unwrap();
    assert_eq!(
        first,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        Some(format!("thread:{tid}")),
        "the channel's declaration alone put the lease in this chain's hands"
    );

    // And it really excludes: a SECOND board on the same declared channel — a different chain, and
    // therefore a different claim area — is queued behind the first.
    let other = store.mint_thread_id();
    store.open_thread(
        &other,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "decl:coding".to_string(),
            opener: "o/a".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/a",
    );
    let second =
        orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-2", &other))
            .unwrap();
    assert_eq!(
        second,
        TriggerAdmission::Queued {
            behind: Some(format!("thread:{tid}")),
            position: 1,
            findings: Vec::new(),
        }
    );
}

#[test]
fn a_thread_in_an_undeclared_channel_does_not_make_a_shared_role_exclusive() {
    // The other side of the channel check: a DM thread (or any thread whose channel no declaration
    // names) leaves a `shared` role exactly as `shared` as it was. Without this, the lookup could
    // pass its own test by answering "exclusive" to everything with a thread.
    let defs = Definitions::new(
        vec![role("handle: reviewer\nsystem_prompt: R\n")],
        vec![channel(
            "name: coding\nmembers: [reviewer]\nworking_tree: exclusive\n",
        )],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("reviewer").unwrap();

    let tid = store.mint_thread_id();
    store.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: "dm:a~reviewer".to_string(),
            opener: "o/a".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/a",
    );

    orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", &tid)).unwrap();
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        None,
        "a thread outside any declared channel claims nothing"
    );
}

#[test]
fn an_answer_travelling_back_up_the_chain_is_never_queued() {
    // Only a commission (`ChainMove::Deeper`) competes for the working copy. An `Unwind` carries the
    // RESULT of work already admitted — and the arrival of that result is what eventually RELEASES
    // the lease, so parking one would deadlock the holder against a queue entry nothing could ever
    // reach. This is also why the three wake sites in `orchestration.rs` may read their `Ok(_)` as
    // "spawned" rather than having to report a deferral they have no receipt field for.
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", "th-a")).unwrap();

    let unwound = orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            chain: ChainMove::Unwind,
            ..spawn_in_thread(decl, hop, "s-2", "th-b")
        },
    )
    .unwrap();
    assert_eq!(
        unwound,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(worker.seen.lock().unwrap().len(), 2);
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
}

#[test]
fn a_later_urgent_trigger_is_queued_ahead_of_an_earlier_normal_one() {
    // Why `RoleSpawn` carries a `priority` at all, in the owner's own example (epic §5.2): "change
    // the button label" must not sit forty minutes behind "build the website" just because it
    // arrived later, if its sender called it more urgent. Without this the queue's ordering column
    // would be written with one constant and the ordering would be pure arrival order.
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", "holder"))
        .unwrap();
    let website = orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            message: "build the website",
            coordinator: Coordinator::Persona,
            ..spawn_in_thread(decl, hop, "s-2", "th-website")
        },
    )
    .unwrap();
    let button = orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            message: "change the button label",
            coordinator: Coordinator::Persona,
            priority: Priority::Urgent,
            ..spawn_in_thread(decl, hop, "s-3", "th-button")
        },
    )
    .unwrap();

    assert_eq!(
        website,
        TriggerAdmission::Queued {
            behind: Some("thread:holder".to_string()),
            position: 1,
            findings: Vec::new(),
        }
    );
    assert_eq!(
        button,
        TriggerAdmission::Queued {
            behind: Some("thread:holder".to_string()),
            position: 1,
            findings: Vec::new(),
        },
        "the urgent trigger enqueued LATER reports position 1 — it goes first"
    );
    assert_eq!(
        store.peek_working_tree_queue().unwrap().unwrap().message,
        "change the button label",
        "and the queue itself agrees about who comes out next"
    );
}

#[test]
fn the_lease_a_trigger_takes_expires_at_the_named_backstop_and_not_before() {
    // The bound is a named constant with a written-out rationale (`WORKING_TREE_LEASE_BOUND`), and
    // it is what `trigger_role` actually stamps — pinned here because a lease that expired sooner
    // than the constant says would hand a live chain's working copy to a second one, silently.
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    orchestration::trigger_role(&c, &mut store, spawn_in_thread(decl, hop, "s-1", "th-a")).unwrap();

    assert_eq!(WORKING_TREE_LEASE_BOUND, "2h");
    // `ctx()` pins now to 2026-07-31T10:00:00Z.
    assert_eq!(
        store.working_tree_holder("2026-07-31T11:59:59Z").unwrap(),
        Some("thread:th-a".to_string()),
        "one second inside the bound the lease is still held"
    );
    assert_eq!(
        store.working_tree_holder("2026-07-31T12:00:01Z").unwrap(),
        None,
        "one second past it the lease is reclaimable"
    );
}

// ---- the two thread meanings, kept apart (PR review of 6j6v.303b, Finding 1) -----------------
//
// `reply_thread` is an OBLIGATION this trigger registered; `thread` is the conversation it BELONGS
// to. They coincide on a fresh 1:1 summon and diverge on exactly the two paths below — and while
// the lease scoped itself off the obligation, both of those paths silently fell through to
// `session:` scope and took no lease at all.

/// A thread in a declared channel's own substrate id, opened for real, the way `facade::ask` opens a
/// board.
fn open_thread_in(store: &mut ChatStore, c: &Ctx, channel_id: &str) -> String {
    let tid = store.mint_thread_id();
    store.open_thread(
        &tid,
        &ThreadRoot {
            origin: "o".to_string(),
            channel_id: channel_id.to_string(),
            opener: "o/a".to_string(),
            created: c.now.to_string(),
            parent: None,
        },
        "o/a",
    );
    tid
}

/// The declarations the epic's own §4 example uses: an `exclusive` CHANNEL whose members declare
/// nothing about the working copy at all.
fn coding_channel_defs() -> Definitions {
    Definitions::new(
        vec![
            role("handle: reviewer\nsystem_prompt: R\n"),
            role("handle: builder\nsystem_prompt: B\n"),
        ],
        vec![channel(
            "name: coding\nmembers: [reviewer, builder]\nworking_tree: exclusive\n",
        )],
    )
    .unwrap()
}

#[test]
fn a_fan_out_shaped_trigger_contends_for_the_lease_even_though_it_registers_no_obligation() {
    // Finding 1, the case that made `send --to <channel>` never contend at all: the fan-out opens a
    // real board thread and passes `reply_thread: None`, because the quorum on that thread is
    // `facade::ask`'s and not the trigger's. Scoping off the obligation therefore resolved to
    // `session:` and skipped the lease — on the ONE path the epic's §4 example is written for.
    let defs = coding_channel_defs();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("reviewer").unwrap();
    assert_eq!(
        decl.working_tree,
        WorkingTree::Shared,
        "no member declares anything — the CHANNEL is the only reason a lease is needed"
    );

    let board = open_thread_in(&mut store, &c, "decl:coding");
    let first =
        orchestration::trigger_role(&c, &mut store, spawn_belonging_to(decl, hop, "s-1", &board))
            .unwrap();
    assert_eq!(
        first,
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        Some(format!("thread:{board}")),
        "the board thread is the claim area, obligation or no obligation"
    );

    // Every other member of the SAME board inherits it — one board is one chain.
    let sibling = defs.role("builder").unwrap();
    assert_eq!(
        orchestration::trigger_role(
            &c,
            &mut store,
            spawn_belonging_to(sibling, hop, "s-2", &board)
        )
        .unwrap(),
        TriggerAdmission::Spawned {
            declaration_changed: None,
        },
        "a second member of the same board must not queue behind the first"
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);

    // A SECOND board on the same channel is a different chain and waits.
    let other_board = open_thread_in(&mut store, &c, "decl:coding");
    assert_eq!(
        orchestration::trigger_role(
            &c,
            &mut store,
            spawn_belonging_to(decl, hop, "s-3", &other_board)
        )
        .unwrap(),
        TriggerAdmission::Queued {
            behind: Some(format!("thread:{board}")),
            position: 1,
            findings: Vec::new(),
        }
    );
    assert_eq!(
        worker.seen.lock().unwrap().len(),
        2,
        "only the first board ran"
    );
}

#[test]
fn an_exclusive_role_summoned_into_a_thread_that_already_has_a_quorum_still_takes_the_lease() {
    // Finding 1's second case. `coordinator_commission` deliberately leaves `reply_thread` unset
    // when the
    // thread already carries somebody else's quorum (setting one would collapse that quorum to a
    // single handle, which ticket 5 pins) — yet that thread's OUTSTANDING expectation is exactly the
    // deterministic release signal the session rule demands, so it must scope to the thread.
    let defs = Definitions::new(
        vec![
            exclusive_role("coder"),
            role("handle: pm\nsystem_prompt: P\n"),
        ],
        vec![],
    )
    .unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);

    // A board somebody else opened and put a quorum on.
    let board = open_thread_in(&mut store, &c, "decl:review");
    let existing = serde_json::to_string(&["o/bob", "o/carol"]).unwrap();
    store.set_expects_reply_from(&board, &existing, "o/pm");

    let coder = defs.role("coder").unwrap();
    assert_eq!(
        orchestration::trigger_role(
            &c,
            &mut store,
            spawn_belonging_to(coder, hop, "s-1", &board)
        )
        .unwrap(),
        TriggerAdmission::Spawned {
            declaration_changed: None,
        }
    );
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        Some(format!("thread:{board}")),
        "the claim area is the board, not this one session"
    );
    let q = store.thread_quorum(&board, c.now).unwrap().unwrap();
    assert_eq!(
        q.expects,
        vec!["o/bob".to_string(), "o/carol".to_string()],
        "and the existing quorum — the thing that will release it — is untouched: {q:?}"
    );
}

#[test]
fn a_thread_known_only_through_its_messages_still_resolves_its_declared_channel() {
    // Finding 5: `thread_channel` reads the `threads` view, which store.rs records as empty for
    // M1-style threads that were never formally opened. Stopping there fails OPEN — a thread known
    // only through its messages would read as `shared` and skip the lease, which is the permissive
    // direction. The two-step resolution (`threads`, then the thread's own messages) is the one
    // `surface::reply_in_thread` and `facade::reply` already use.
    let defs = coding_channel_defs();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("reviewer").unwrap();

    // No `open_thread` at all: the thread exists only as a `thread_id` on a message.
    store.set_channel_field("decl:coding", "kind", "group", "o/a");
    store.post_message(&nexus_chat::model::MessageEnvelope {
        origin: "o".into(),
        channel_id: "decl:coding".into(),
        sender: "o/a".into(),
        kind: MessageKind::Task,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some("t-m1".into()),
        refs: Refs::default(),
        body: "go".into(),
    });
    assert_eq!(
        store.thread_channel("t-m1"),
        None,
        "precondition: the `threads` view knows nothing about this thread"
    );

    orchestration::trigger_role(&c, &mut store, spawn_belonging_to(decl, hop, "s-1", "t-m1"))
        .unwrap();
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        Some("thread:t-m1".to_string()),
        "the message-only thread still resolves to its declared, exclusive channel"
    );
}

#[test]
fn an_obligation_on_a_thread_this_trigger_does_not_belong_to_does_not_scope_the_lease() {
    // The other direction of the split, so the two fields cannot quietly be re-merged: `reply_thread`
    // alone no longer scopes anything. A trigger that registers an obligation but belongs to no
    // thread is session-scoped and takes no lease — the deliberate hole is keyed on `thread`.
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    let mut store = store();
    let c = ctx(&defs, &worker, None, 0);
    let hop = checked_hop(&c, &store);
    let decl = defs.role("coder").unwrap();

    orchestration::trigger_role(
        &c,
        &mut store,
        RoleSpawn {
            reply_thread: Some("th-obligation"),
            thread: None,
            ..base_spawn(decl, hop)
        },
    )
    .unwrap();
    assert_eq!(
        store.working_tree_holder(c.now).unwrap(),
        None,
        "the claim area comes from `thread`, never from `reply_thread`"
    );
    assert_eq!(
        worker.only().reply_thread.as_deref(),
        Some("th-obligation"),
        "and the obligation still reaches the worker untouched — the split changed scoping only"
    );
}

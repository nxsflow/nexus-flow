//! **A step says what its role does THERE** (nxf 6j6v.6kam and nxf 6j6v.4c3e, the two children of
//! epic 6j6v.hbs2 that sit on the step declaration).
//!
//! Both answer one question — what this station makes of a general role — and they are built
//! together because they open the same seam. A step may carry a TASK, which becomes a fourth prompt
//! layer under the persona's own `system_prompt`, and it may raise or lower the STAGE its target
//! runs at. The case that forced both: the same three reviewer personas should judge a diff at one
//! step and a SPECIFICATION at another, without a second set of personas to keep in step with the
//! first — and a design judgement has to imagine what WOULD happen, which is dearer thinking than
//! reading a diff.
//!
//! Everything here goes through [`Engine`], the seam the products speak, with a worker that KEEPS
//! what it was handed: both capabilities land in the `TriggerRequest` — one in `role.system_prompt`,
//! the other in `role.model` — and neither is visible in the dry worker's log line.

#[path = "common/mod.rs"]
mod common;

use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::{Model, RoleDecl};
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-09-09T10:00:00Z";

/// The declared task of the `check` step, verbatim in the declaration and verbatim in the prompt.
const SPEC_TASK: &str =
    "You are judging a SPECIFICATION, not a diff. Ask what would happen if what \
                         is written here were built.";

/// The persona whose job description is deliberately several lines long — so a test can show that
/// the fourth layer added nothing at the persona's expense.
const JUDGE_PROMPT: &str = "You are judge.\nYou speak for ONE dimension and no other.\nYou grade \
                            High / Medium / Low, and you send work back below Medium.";

/// Keeps every request verbatim, and answers the one read the engine makes of a worker: which of
/// the sessions it started are still running. The dry worker denies every session, which is the
/// mode a stepped flow cannot be driven through — the liveness gate never lets step two start.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<std::collections::HashSet<String>>,
}

impl Recorder {
    fn newest_for(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        match seen.iter().rev().find(|r| r.role.handle == handle) {
            Some(req) => req.clone(),
            None => {
                let handles: Vec<&str> = seen.iter().map(|r| r.role.handle.as_str()).collect();
                panic!("no trigger for {handle}, only: {handles:?}")
            }
        }
    }

    /// Every request for `handle`, oldest first — what a persona serving TWO steps of one run
    /// produces, and the only way to compare the two.
    fn all_for(&self, handle: &str) -> Vec<TriggerRequest> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.role.handle == handle)
            .cloned()
            .collect()
    }

    /// The process behind `session` has exited — what a host worker learns from its own runtime.
    fn exited(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
}

impl Worker for Recorder {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.running
            .lock()
            .unwrap()
            .insert(req.internal_session.clone());
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
}

fn team(tmp: &TempDir, roles: &str, channels: &str) -> (Engine, Arc<Recorder>) {
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles: Vec<RoleDecl> = roles
        .split(';')
        .map(|r| serde_yaml::from_str(r).expect("role parses"))
        .collect();
    let defs = Definitions::new(
        roles,
        serde_yaml::from_str(channels).expect("channels parse"),
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(Recorder::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (engine, worker)
}

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// A spawned session speaking for itself — what a persona's own `nxc send` arrives as.
fn as_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

fn open(engine: &Engine, channel: &str) {
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: channel,
                body: "judge the draft in docs/spec.md",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel is addressed");
}

/// Settle the step `handle` is serving, and report its session gone — the two halves a host runtime
/// performs, and what it takes for the next step to start.
fn settle(engine: &Engine, worker: &Recorder, handle: &str) {
    let req = worker.newest_for(handle);
    engine
        .reply_thread(
            caller(handle),
            ReplyThreadRequest {
                machine: None,
                thread: req
                    .reply_thread
                    .as_deref()
                    .expect("the step owes an answer"),
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply settles the step");
    engine
        .session_ended(caller(handle), &req.internal_session)
        .expect("the runtime reports the process gone");
    worker.exited(&req.internal_session);
}

const ROLES: &str = "handle: pm\nsystem_prompt: You are pm.\ntools: [Bash]\n;\
                     handle: coder\nsystem_prompt: You are coder.\ntools: [Bash]\n;\
                     handle: judge\nsystem_prompt: |\n  You are judge.\n  You speak for ONE \
                     dimension and no other.\n  You grade High / Medium / Low, and you send work \
                     back below Medium.\nstage: junior\ntools: [Bash]\n";

/// `build` declares no task and no stage; `check` declares both. One channel, so the two are read
/// off ONE run and neither can be explained by a difference in the workspace.
const CODING: &str = "- name: coding\n  members: [coder, judge]\n  steps:\n    \
                      - id: build\n      target: coder\n      next: check\n    \
                      - id: check\n      target: judge\n      stage: senior\n      task: |\n        \
                      You are judging a SPECIFICATION, not a diff. Ask what would happen if what \
                      is written here were built.\n";

// ---- nxf 6j6v.6kam — the fourth prompt layer ------------------------------------------------

#[test]
fn the_step_puts_its_task_under_the_persona_and_says_where_it_came_from() {
    // DoD 1. The layer sits UNDER the `system_prompt` — closest to the conversation, last thing
    // read — and the prompt says it comes from the step, so a model can tell it from the persona
    // it is layered onto.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, ROLES, CODING);
    open(&engine, "coding");
    settle(&engine, &worker, "coder");

    let prompt = worker.newest_for("judge").role.system_prompt;
    let task_at = prompt.find(SPEC_TASK).unwrap_or_else(|| {
        panic!("the step's declared task is not in the prompt at all: {prompt}")
    });
    let persona_at = prompt
        .find("You grade High / Medium / Low")
        .expect("the persona's own prompt is there");
    assert!(
        persona_at < task_at,
        "the step's task is the FOURTH layer — under the persona, not above it: {prompt}"
    );
    assert!(
        prompt.contains("Your task at this step"),
        "…and it is marked as coming from the step rather than blending into the persona: {prompt}"
    );
}

#[test]
fn a_step_that_declares_no_task_adds_no_layer_to_the_prompt() {
    // DoD 2 as far as a RUN can show it: `build` declares no task, so the composed prompt ends
    // where it always ended — at the persona's own `system_prompt`, with nothing appended and the
    // marker nowhere in it.
    //
    // **The name says "adds no layer" and not "byte-identical", because the body cannot prove the
    // second** (review of PR #454, Test Quality #1): two triggers of one persona differ in the
    // thread id their forced ending names, so nothing here can compare one whole prompt against
    // another. The byte-identical claim is pinned where it CAN be —
    // `role::tests::the_steps_task_is_the_last_layer_and_a_step_without_one_changes_not_one_byte`
    // asserts the composed string in full — and this test is the end-to-end half: whatever the run
    // does around it, no fourth layer appears.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, ROLES, CODING);
    open(&engine, "coding");

    let prompt = worker.newest_for("coder").role.system_prompt;
    assert!(
        prompt.ends_with("You are coder."),
        "nothing follows the persona's own prompt for a step that declared nothing: {prompt}"
    );
    assert!(
        !prompt.contains("Your task at this step"),
        "…not even the marker: {prompt}"
    );
}

#[test]
fn one_persona_over_two_steps_reads_two_different_tasks() {
    // DoD 5, and the case the whole item exists for: ONE declared persona, two steps, two composed
    // prompts. Before this, a second task meant a second persona file to keep in step with the
    // first.
    const TWO_TASKS: &str = "- name: reading\n  members: [judge]\n  steps:\n    \
                             - id: diff\n      target: judge\n      next: spec\n      task: Judge \
                             the DIFF that is attached.\n    \
                             - id: spec\n      target: judge\n      task: Judge the SPECIFICATION \
                             that is attached.\n";
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, ROLES, TWO_TASKS);
    open(&engine, "reading");
    settle(&engine, &worker, "judge");

    let prompts: Vec<String> = worker
        .all_for("judge")
        .into_iter()
        .map(|r| r.role.system_prompt)
        .collect();
    assert_eq!(prompts.len(), 2, "both steps ran their own session");
    assert!(
        prompts[0].contains("Judge the DIFF that is attached."),
        "step one reads its own task: {}",
        prompts[0]
    );
    assert!(
        prompts[1].contains("Judge the SPECIFICATION that is attached."),
        "step two reads its own: {}",
        prompts[1]
    );
    assert!(
        !prompts[1].contains("Judge the DIFF that is attached."),
        "…and only its own — a step's task is not cumulative: {}",
        prompts[1]
    );
}

#[test]
fn the_task_layer_adds_and_the_persona_comes_through_it_whole() {
    // DoD 4. The layer may not replace the persona or switch a part of it off, so every line the
    // role declared is still in the composed prompt with the task under it.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, ROLES, CODING);
    open(&engine, "coding");
    settle(&engine, &worker, "coder");

    let prompt = worker.newest_for("judge").role.system_prompt;
    assert!(
        prompt.contains(SPEC_TASK),
        "the step laid its task on: {prompt}"
    );
    assert!(
        prompt.contains(JUDGE_PROMPT),
        "…and the persona's own prompt is still there in full, line for line — nothing was cut \
         out to make room for it: {prompt}"
    );
}

#[test]
fn a_step_onto_a_whole_channel_hands_the_task_to_every_member() {
    // DoD 3. The review round is addressed as a CHANNEL, and what makes the round mean something
    // different is a property of the STEP that called it — so it has to reach each member, none of
    // which the step names.
    const PANEL: &str = "- name: panel\n  members: [judge, second]\n\
                         - name: planning\n  members: [panel]\n  steps:\n    \
                         - id: check\n      target: panel\n      task: |\n        You are judging \
                         a SPECIFICATION, not a diff. Ask what would happen if what is written \
                         here were built.\n";
    let roles = format!("{ROLES};handle: second\nsystem_prompt: You are second.\ntools: [Bash]\n");
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &roles, PANEL);
    open(&engine, "planning");

    for member in ["judge", "second"] {
        let prompt = worker.newest_for(member).role.system_prompt;
        assert!(
            prompt.contains(SPEC_TASK),
            "{member} is a member of the channel the step commissioned, so the step's task \
             reaches it: {prompt}"
        );
    }
}

#[test]
fn a_persona_a_steps_session_commissions_itself_gets_neither_the_task_nor_the_band() {
    // **The one level up is for the MEMBERS of a commissioned channel and for nobody else.** A
    // session serving a step may open a conversation of its own, and that thread hangs under the
    // step's slot — so a naive "no mark of its own, take the parent's" hands the step's task and its
    // band to a persona the step never named. Both fields say what the step's TARGET is to do, and
    // this is the boundary of that.
    let roles = format!(
        "{ROLES};handle: helper\nsystem_prompt: You are helper.\nstage: junior\ntools: [Bash]\n"
    );
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &roles, CODING);
    open(&engine, "coding");
    settle(&engine, &worker, "coder");
    let judging = worker.newest_for("judge");
    assert!(
        judging.role.system_prompt.contains(SPEC_TASK),
        "the step's own target has the task — without that this test proves nothing"
    );

    engine
        .send_to(
            as_session(&judging.internal_session),
            SendToRequest {
                machine: None,
                to: "helper",
                body: "look something up for me",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("a step's session may commission somebody itself");

    let helper = worker.newest_for("helper");
    assert!(
        !helper.role.system_prompt.contains("Your task at this step"),
        "the helper serves no step and is told about none: {}",
        helper.role.system_prompt
    );
    assert_eq!(
        helper.role.model,
        Some(Model::Sonnet),
        "…and runs at its own band, not at the one the commissioning step asked for itself"
    );
}

#[test]
fn a_channel_named_after_the_role_its_own_step_commissions_leaks_nothing() {
    // **The boundary above asked its question by NAME, and a name is not an identity** (review of
    // PR #454, Code Quality #1 / Integrity #1, found independently by two reviewers).
    //
    // A role wins a name a channel also declares — `channel::flow_step_target` says so, and
    // `tests/public_channel.rs` ships that shape — while `Definitions::new` polices role handles and
    // channel names in SEPARATE namespaces. So a channel may be named after the very role its own
    // step commissions. Its member's slot thread is opened in the CONTAINING channel, so the slot's
    // channel name and the step's target string are then the same word for two different things,
    // and a check that compares them as strings concludes the slot IS the channel the step
    // commissioned. Everything the test above rules out comes back through that door.
    //
    // The boundary asks the same question the OPENER asked instead: did this step's target resolve
    // to a channel at all? A role-target step can never satisfy it, whatever anybody is named.
    const COLLIDING: &str = "- name: judge\n  members: [judge]\n  steps:\n    \
                             - id: check\n      target: judge\n      stage: senior\n      \
                             task: |\n        You are judging a SPECIFICATION, not a diff. Ask \
                             what would happen if what is written here were built.\n";
    let roles = format!(
        "{ROLES};handle: helper\nsystem_prompt: You are helper.\nstage: junior\ntools: [Bash]\n"
    );
    let tmp = TempDir::new().unwrap();
    // `send --to` breaks the same tie the other way, so this opens the CHANNEL `judge`, whose one
    // step commissions the ROLE `judge`.
    let (engine, worker) = team(&tmp, &roles, COLLIDING);
    open(&engine, "judge");
    let judging = worker.newest_for("judge");
    assert!(
        judging.role.system_prompt.contains(SPEC_TASK),
        "the step's own target still has the task — the fix must not cost the feature"
    );

    engine
        .send_to(
            as_session(&judging.internal_session),
            SendToRequest {
                machine: None,
                to: "helper",
                body: "look something up for me",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("a step's session may commission somebody itself");

    let helper = worker.newest_for("helper");
    assert!(
        !helper.role.system_prompt.contains("Your task at this step"),
        "a shared NAME is not the channel identity the boundary asks about: {}",
        helper.role.system_prompt
    );
    assert_eq!(
        helper.role.model,
        Some(Model::Sonnet),
        "…and the band the step asked for itself does not travel either"
    );
}

// ---- nxf 6j6v.4c3e — the stage the step runs its target at -----------------------------------

#[test]
fn the_step_decides_which_stage_its_target_runs_at() {
    // DoD 1, proved at the run: `judge` declares `stage: junior`, and the `check` step declares
    // `senior`, so the session that actually starts runs on the senior band's model.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, ROLES, CODING);
    open(&engine, "coding");
    settle(&engine, &worker, "coder");

    assert_eq!(
        worker.newest_for("judge").role.model,
        Some(Model::Opus),
        "`stage: senior` on the step beats `stage: junior` on the persona"
    );
    assert_eq!(
        worker.newest_for("coder").role.model,
        None,
        "…and a step that declares no stage changes nothing about its target"
    );
}

#[test]
fn a_channel_step_runs_every_member_at_the_steps_stage() {
    // DoD 2, the twin of the task layer's own channel case: the members are what a channel step
    // actually runs, so a stage that stopped at the channel thread would change nothing at all.
    const PANEL: &str = "- name: panel\n  members: [judge, second]\n\
                         - name: planning\n  members: [panel]\n  steps:\n    \
                         - id: check\n      target: panel\n      stage: principal\n";
    let roles = format!(
        "{ROLES};handle: second\nsystem_prompt: You are second.\nstage: junior\ntools: [Bash]\n"
    );
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &roles, PANEL);
    open(&engine, "planning");

    for member in ["judge", "second"] {
        assert_eq!(
            worker.newest_for(member).role.model,
            Some(Model::Fable),
            "{member} runs at the stage the step declared, not at its own"
        );
    }
}

#[test]
fn a_persona_that_names_its_model_keeps_it_through_a_steps_stage() {
    // DoD 4, the transitional rule: a step may raise or lower a BAND, and a named model is the
    // author saying something a band cannot express — so it is never overridden from above.
    //
    // **Two steps declaring the SAME stage, and only one of them moves anything.** A test with the
    // pinned persona alone would pass against a build that ignores `stage:` altogether, which is
    // exactly the claim it must not be able to make; the second step is what shows the stage was
    // live in this very run.
    const PINNED: &str = "- name: reading\n  members: [pinned, judge]\n  steps:\n    \
                          - id: spec\n      target: pinned\n      stage: principal\n      next: \
                          band\n    \
                          - id: band\n      target: judge\n      stage: principal\n";
    let roles = format!(
        "{ROLES};handle: pinned\nsystem_prompt: You are pinned.\nmodel: sonnet\ntools: [Bash]\n"
    );
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &roles, PINNED);
    open(&engine, "reading");

    assert_eq!(
        worker.newest_for("pinned").role.model,
        Some(Model::Sonnet),
        "an explicitly declared `model:` runs unchanged, whatever stage the step asks for"
    );

    settle(&engine, &worker, "pinned");
    assert_eq!(
        worker.newest_for("judge").role.model,
        Some(Model::Fable),
        "…while the same declared stage moves a persona that named no model of its own"
    );
}

#[test]
fn the_same_persona_over_two_steps_runs_at_two_stages() {
    // DoD 6. One persona, one run, two steps, two bands — which is the whole claim of the item,
    // and the thing neither a unit test of the mapping nor one of the precedence can make.
    const TWO_STAGES: &str = "- name: reading\n  members: [judge]\n  steps:\n    \
                              - id: diff\n      target: judge\n      next: spec\n    \
                              - id: spec\n      target: judge\n      stage: principal\n";
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, ROLES, TWO_STAGES);
    open(&engine, "reading");
    settle(&engine, &worker, "judge");

    let models: Vec<Option<Model>> = worker
        .all_for("judge")
        .into_iter()
        .map(|r| r.role.model)
        .collect();
    assert_eq!(
        models,
        vec![Some(Model::Sonnet), Some(Model::Fable)],
        "step one takes the persona's own band, step two the one it declared itself"
    );
}

//! The scripted harness of §12.1: `Script { def, inputs }` with a simulated clock and a recorder. It
//! lives under `tests/` and each test binary includes it, because the engine has no feature to carry it
//! in (the layering gate allows none) and production code carries no test harness.
//!
//! `decide` sees what a host hands it, in the order a host hands it: the input in its envelope, then
//! each decision folded after it. Every step is checked as it runs: a replay of the log so far must give
//! the same state, so an `evolve` that disagrees with what `decide` simulated cannot pass.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use nexus_run_contracts::{
    Base, Decision, Disposition, EndCause, Event, EventBody, FlowRef, HeartbeatKind,
    LivenessPolicy, MessageId, Origin, ParkOutcome, RunId, Secs, SessionFact, Slot, StopKind,
    Timestamp, TreeRefusal, WithdrawnBy,
};
use nexus_run_engine::{
    compile, decide, decision_key, delivered_key, evolve, pending_effects, status, ChannelDecl,
    DeliverBody, Effect, FlowDef, RunState, SlotState,
};

pub const RUN: &str = "r-01JAAAAAAAAAAAAAAAAAAAAAAA";
pub const WORKSPACE: &str = "workspace-1";
pub const ROOT_THREAD: &str = "m-root";
pub const ROOT_MESSAGE: &str = "m-root-message";
/// Where the simulated clock starts unless a case says otherwise: 2026-11-04T10:00:00Z.
pub const CLOCK_START: i64 = 1_793_786_400_000;

pub fn coding_def() -> FlowDef {
    let coding: ChannelDecl =
        serde_yaml::from_str(include_str!("fixtures/coding.yaml")).expect("coding.yaml parses");
    let review: ChannelDecl =
        serde_yaml::from_str(include_str!("fixtures/review.yaml")).expect("review.yaml parses");
    compile(&coding, &review).expect("the real channel compiles")
}

/// One input, with the key a host would give it where the reducer reads that key.
pub struct Input {
    pub body: EventBody,
    pub key: Option<String>,
}

impl From<EventBody> for Input {
    fn from(body: EventBody) -> Self {
        Self { body, key: None }
    }
}

pub struct Script {
    pub def: FlowDef,
    pub policy: LivenessPolicy,
    pub state: RunState,
    pub log: Vec<Event>,
    pub lists: Vec<Vec<Decision>>,
    /// Where a fold of `log` starts: the initial state, or the state a case was handed.
    origin: RunState,
    pub now: Timestamp,
    start: Timestamp,
    seq: u64,
    next_message: u32,
    recorder: BTreeMap<String, u32>,
    /// Every reason code this script saw: in a decision, an end cause or the status of the run.
    pub codes: BTreeSet<String>,
    last: Vec<Decision>,
}

impl Script {
    pub fn coding() -> Self {
        Self::at(CLOCK_START)
    }

    /// The coding flow under a simulated clock that starts elsewhere.
    pub fn at(start: i64) -> Self {
        Self::with(coding_def(), LivenessPolicy::default(), start)
    }

    pub fn with(def: FlowDef, policy: LivenessPolicy, start: i64) -> Self {
        let run = RunId::parse(RUN).unwrap();
        let origin = RunState::initial(run, def.clone());
        Self {
            state: origin.clone(),
            origin,
            def,
            policy,
            log: Vec::new(),
            lists: Vec::new(),
            now: Timestamp::from_unix_millis(start),
            start: Timestamp::from_unix_millis(start),
            seq: 0,
            next_message: 0,
            recorder: BTreeMap::new(),
            codes: BTreeSet::new(),
            last: Vec::new(),
        }
    }

    /// A script that carries on from a state somebody else reached, under its own definition.
    pub fn from_state(state: RunState) -> Self {
        let run_at = Timestamp::from_unix_millis(CLOCK_START);
        Self {
            def: state.def.clone(),
            policy: state.policy.clone(),
            seq: state.seq,
            origin: state.clone(),
            state,
            log: Vec::new(),
            lists: Vec::new(),
            now: run_at,
            start: run_at,
            next_message: 0,
            recorder: BTreeMap::new(),
            codes: BTreeSet::new(),
            last: Vec::new(),
        }
    }

    /// Hands one input to `decide`, folds it and the decisions, and records all of it.
    pub fn input(&mut self, input: impl Into<Input>) -> &mut Self {
        let input = input.into();
        let run = self.state.run.clone();
        let origin = origin_of(&input.body);
        self.seq += 1;
        let key = input.key.unwrap_or_else(|| format!("input/{}", self.seq));
        let event = Event::new(run.clone(), self.seq, self.now, key, origin, input.body);
        let decisions = decide(&self.state, &event, &self.def);
        assert_eq!(
            decide(&self.state, &event, &self.def),
            decisions,
            "decide twice"
        );

        // Folded first, as the reducer's own log order has it: the input, then what it decided.
        self.fold(event.clone());
        for (index, decision) in decisions.iter().enumerate() {
            self.seq += 1;
            let key = decision_key(&event.key, index, decision);
            let body = EventBody::Decision(decision.clone());
            let event = Event::new(run.clone(), self.seq, self.now, key, Origin::Reducer, body);
            self.fold(event);
        }
        self.record(&decisions);
        self.lists.push(decisions.clone());
        self.last = decisions;
        self.assert_a_replay_agrees();
        self.assert_invariants();
        self
    }

    /// A decision the host appends itself: `parked { poison_input }` has no input behind it (§13).
    pub fn host_decision(&mut self, decision: Decision) -> &mut Self {
        self.seq += 1;
        let event = Event::new(
            self.state.run.clone(),
            self.seq,
            self.now,
            format!("host/{}", self.seq),
            Origin::Host,
            EventBody::Decision(decision.clone()),
        );
        self.fold(event);
        self.record(std::slice::from_ref(&decision));
        self.lists.push(vec![decision]);
        self.assert_invariants();
        self
    }

    /// What no case may ever see, checked after every input of every case.
    fn assert_invariants(&mut self) {
        let effects = self.effects();
        for effect in &effects {
            if let Effect::StopSession {
                kind: StopKind::Kill,
                session_key,
                ..
            } = effect
            {
                assert!(
                    self.state.withdrawn,
                    "a SIGKILL of {session_key} is owed and nobody withdrew: {:?}",
                    self.lists.last()
                );
            }
            if let Effect::StartSession { session_key, .. } = effect {
                assert!(
                    self.state.tree_prepared(),
                    "{session_key} starts before the tree"
                );
            }
        }
        if !self.state.tree_prepared() {
            for (timer, armed) in &self.state.timers {
                assert_ne!(
                    armed.purpose,
                    nexus_run_contracts::TimerPurpose::StartDeadline,
                    "{timer} is armed with no prepared tree"
                );
            }
        }
        if self.state.is_finished() {
            assert!(self.state.timers.is_empty(), "a timer outlived the run");
            assert!(effects.is_empty(), "a finished run owes {effects:?}");
        }
        if let Some(reason) = status(&self.state, 0, self.now).reason {
            self.codes.insert(reason.as_str().to_owned());
        }
    }

    pub fn fold_state(&self) -> RunState {
        self.log
            .iter()
            .try_fold(self.origin.clone(), evolve)
            .expect("the log folds")
    }

    fn fold(&mut self, event: Event) {
        let state = std::mem::replace(
            &mut self.state,
            RunState::initial(RunId::parse(RUN).unwrap(), self.def.clone()),
        );
        self.state = evolve(state, &event).expect("the script folds what it decided");
        self.log.push(event);
    }

    fn record(&mut self, decisions: &[Decision]) {
        for decision in decisions {
            match decision {
                Decision::Finished { reason, .. } | Decision::Finishing { reason, .. } => {
                    if matches!(decision, Decision::Finished { .. }) {
                        *self.recorder.entry("finished".to_owned()).or_default() += 1;
                    }
                    if let Some(reason) = reason {
                        self.codes.insert(reason.as_str().to_owned());
                    }
                }
                Decision::Ignored { reason, .. }
                | Decision::Liveness { reason, .. }
                | Decision::Held { reason, .. }
                | Decision::Parked { reason, .. } => {
                    self.codes.insert(reason.as_str().to_owned());
                }
                _ => {}
            }
        }
        if let Some(Event {
            body:
                EventBody::SessionFact {
                    fact: SessionFact::Ended { cause },
                    ..
                },
            ..
        }) = self
            .log
            .iter()
            .rev()
            .find(|e| !matches!(e.body, EventBody::Decision(_)))
        {
            self.codes.insert(cause_code(cause).to_owned());
        }
    }

    fn assert_a_replay_agrees(&self) {
        assert_eq!(
            self.fold_state(),
            self.state,
            "a fold of the log differs from the run"
        );
    }

    /// Asserts the decision list of the last input, field by field. See [`render`].
    #[track_caller]
    pub fn expect<S: AsRef<str>>(&mut self, expected: &[S]) -> &mut Self {
        let got = self.decisions();
        let want: Vec<String> = expected
            .iter()
            .map(|line| line.as_ref().to_owned())
            .collect();
        assert_eq!(got, want, "the decision list");
        self
    }

    /// An input whose list must hold nothing: the one thing `expect(&[])` cannot infer a type for.
    #[track_caller]
    pub fn expect_none(&mut self) -> &mut Self {
        self.expect::<&str>(&[])
    }

    /// The effects owed for this one session.
    pub fn effects_of(&self, key: &str) -> Vec<Effect> {
        self.effects()
            .into_iter()
            .filter(|effect| match effect {
                Effect::StartSession { session_key, .. }
                | Effect::NudgeSession { session_key, .. }
                | Effect::StopSession { session_key, .. } => session_key == key,
                _ => false,
            })
            .collect()
    }

    pub fn slot_state(&self, key: &str) -> SlotState {
        let index = self
            .state
            .find(key)
            .unwrap_or_else(|| panic!("no slot {key}"));
        self.state.slots[index].state.clone()
    }

    /// Passes every step on the way until `step` is the one running.
    pub fn pass_until(&mut self, step: &str) -> &mut Self {
        for _ in 0..16 {
            if self.state.current_step.as_deref() == Some(step) {
                return self;
            }
            let current = self.state.current_step.clone().expect("a step is running");
            if current == "judge" {
                self.run_fan_out_to_pass("judge");
            } else {
                self.run_step_to_pass(&current);
            }
        }
        // A reducer that stopped advancing must turn a case red, never hang it.
        panic!("the run never reached {step}: {:?}", self.state.view());
    }

    /// Whether any input of this script decided exactly this. Only for lines with no relative instant:
    /// an instant renders against the clock at the time of asking.
    pub fn decided(&self, line: &str) -> bool {
        self.lists
            .iter()
            .flatten()
            .any(|decision| render(decision, self.now) == line)
    }

    pub fn decisions(&self) -> Vec<String> {
        self.last.iter().map(|d| render(d, self.now)).collect()
    }

    pub fn last_list(&self) -> &[Decision] {
        &self.last
    }

    pub fn effects(&self) -> Vec<Effect> {
        pending_effects(&self.state)
    }

    pub fn advance(&mut self, secs: u64) -> &mut Self {
        self.now = self.now.plus(Secs(secs));
        self
    }

    /// How often this was recorded, delivered results among them.
    pub fn count(&self, what: &str) -> u32 {
        self.recorder.get(what).copied().unwrap_or(0)
    }

    pub fn message(&mut self) -> MessageId {
        self.next_message += 1;
        format!("m-{}", self.next_message)
    }

    pub fn workspace(&self) -> String {
        WORKSPACE.to_owned()
    }

    pub fn key(&self, step: &str, visit: u32, member: &str, attempt: u32) -> String {
        slot(step, visit, member, attempt).session_key(&self.state.run)
    }

    // ------------------------------------------------------------------ the inputs a host enqueues

    pub fn commissioned(&self) -> EventBody {
        EventBody::Commissioned {
            run: self.state.run.clone(),
            workspace_id: WORKSPACE.to_owned(),
            flow: FlowRef {
                hash: self.def.flow_ref().hash,
            },
            policy: self.policy.clone(),
            root_thread: ROOT_THREAD.to_owned(),
            requester: "person".to_owned(),
        }
    }

    pub fn root_opened(&self) -> EventBody {
        EventBody::RootOpened {
            thread: ROOT_THREAD.to_owned(),
            message: ROOT_MESSAGE.to_owned(),
        }
    }

    pub fn claim_taken(&self) -> EventBody {
        EventBody::ClaimTaken
    }

    pub fn base(&self) -> Base {
        Base {
            branch: "main".to_owned(),
            commit: "c0".to_owned(),
        }
    }

    pub fn tree_prepared(&self) -> EventBody {
        EventBody::TreePrepared {
            base: self.base(),
            found: None,
        }
    }

    pub fn tree_refused(&self, reason: TreeRefusal) -> Input {
        Input {
            body: EventBody::TreeRefused { reason },
            key: Some(self.state.tree_refused_key()),
        }
    }

    pub fn tree_parked_clean(&self) -> EventBody {
        self.tree_parked(ParkOutcome::Clean { branch: None })
    }

    pub fn tree_parked(&self, outcome: ParkOutcome) -> EventBody {
        EventBody::TreeParked { outcome }
    }

    /// The result of the first `Deliver` the run owes now, under its effect's key.
    pub fn delivered(&self) -> Input {
        let key = self
            .effects()
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Deliver { key, .. } => Some(key),
                _ => None,
            })
            .expect("a Deliver is owed");
        self.delivered_key(&key)
    }

    pub fn delivered_key(&self, key: &str) -> Input {
        Input {
            body: EventBody::Delivered {
                message: format!("m-delivered-{key}"),
            },
            key: Some(delivered_key(key)),
        }
    }

    pub fn claim_released(&self) -> EventBody {
        EventBody::ClaimReleased
    }

    pub fn fact(&self, key: &str, fact: SessionFact) -> EventBody {
        EventBody::SessionFact {
            session_key: key.to_owned(),
            fact,
        }
    }

    pub fn started(&self, key: &str) -> EventBody {
        self.fact(
            key,
            SessionFact::Started {
                backend_session_id: format!("backend/{key}"),
                pid: Some(4242),
            },
        )
    }

    pub fn beat(&self, key: &str, beat: HeartbeatKind) -> EventBody {
        self.fact(key, SessionFact::Heartbeat { beat })
    }

    pub fn ended(&self, key: &str, cause: EndCause) -> EventBody {
        self.fact(key, SessionFact::Ended { cause })
    }

    pub fn reply(&self, key: &str, message: &str) -> EventBody {
        self.reply_with(key, message, false, false)
    }

    pub fn reply_with(
        &self,
        key: &str,
        message: &str,
        escalate: bool,
        needs_rework: bool,
    ) -> EventBody {
        EventBody::Reply {
            session_key: key.to_owned(),
            message: message.to_owned(),
            escalate,
            needs_rework,
        }
    }

    pub fn answer(&self, question: &str, disposition: Disposition, message: &str) -> EventBody {
        EventBody::Answer {
            question: question.to_owned(),
            disposition,
            message: message.to_owned(),
        }
    }

    pub fn withdrawn(&self) -> EventBody {
        EventBody::Withdrawn {
            message: None,
            by: WithdrawnBy::Requester,
        }
    }

    pub fn nudged(&self, key: &str) -> EventBody {
        EventBody::Nudged {
            session_key: key.to_owned(),
        }
    }

    pub fn stop_signalled(&self, key: &str, nth: u32, signal: StopKind) -> EventBody {
        EventBody::StopSignalled {
            session_key: key.to_owned(),
            nth,
            signal,
        }
    }

    /// The firing of an armed timer, as the host enqueues it when it is due.
    #[track_caller]
    pub fn fired(&self, timer: &str) -> EventBody {
        let armed = self
            .state
            .timers
            .get(timer)
            .unwrap_or_else(|| panic!("{timer} is not armed: {:?}", self.state.timers.keys()));
        EventBody::TimerFired {
            timer: timer.to_owned(),
            fire_at: armed.fire_at,
        }
    }

    /// Advances the clock to the timer's instant and fires it.
    #[track_caller]
    pub fn fire(&mut self, timer: &str) -> &mut Self {
        let due = self
            .state
            .timers
            .get(timer)
            .unwrap_or_else(|| panic!("{timer} is not armed: {:?}", self.state.timers.keys()))
            .fire_at;
        self.now = self.now.max(due);
        self.input(self.fired(timer))
    }

    // ------------------------------------------------------------------ whole walks

    /// From nothing to the first step started on a prepared tree.
    pub fn reach_first_session(&mut self) -> &mut Self {
        self.input(self.commissioned());
        self.input(self.root_opened());
        self.input(self.claim_taken());
        self.input(self.tree_prepared())
    }

    /// The session started, replied without a bit, and its process gone.
    pub fn answer_session(&mut self, key: &str, escalate: bool, needs_rework: bool) -> MessageId {
        if matches!(self.slot_state(key), SlotState::Starting { .. }) {
            self.input(self.started(key));
        }
        let message = self.message();
        self.input(self.reply_with(key, &message, escalate, needs_rework));
        self.input(self.ended(key, EndCause::TurnComplete));
        message
    }

    /// Every member of the current single step passes.
    pub fn run_step_to_pass(&mut self, step: &str) -> MessageId {
        self.run_step(step, false)
    }

    /// The current single step's only member sends the work back.
    pub fn run_step_to_rework(&mut self, step: &str) -> MessageId {
        self.run_step(step, true)
    }

    fn run_step(&mut self, step: &str, needs_rework: bool) -> MessageId {
        let key = self.latest_key(step, None);
        self.answer_session(&key, false, needs_rework)
    }

    /// The latest attempt of the step's latest visit, for one member or for its first.
    pub fn latest_key(&self, step: &str, member: Option<&str>) -> String {
        let record = self
            .state
            .slots
            .iter()
            .rev()
            .find(|r| r.slot.step == step && member.is_none_or(|m| r.slot.member == m))
            .unwrap_or_else(|| panic!("no slot of {step} ({member:?})"));
        self.state.session_key(&record.slot)
    }

    pub fn run_fan_out_to_pass(&mut self, step: &str) -> MessageId {
        self.run_fan_out(step, &[])
    }

    /// All three reply, `rework` among them with the bit; then the consolidator.
    pub fn run_fan_out(&mut self, step: &str, rework: &[&str]) -> MessageId {
        for member in ["code-quality", "test-quality", "integrity"] {
            let key = self.latest_key(step, Some(member));
            self.answer_session(&key, false, rework.contains(&member));
        }
        let key = self.latest_key(step, Some("__synth__"));
        self.answer_session(&key, false, false)
    }

    /// `Finishing` from a delivered outcome onward: the park, the delivery, the release.
    pub fn finish_clean(&mut self) -> &mut Self {
        self.input(self.tree_parked_clean());
        self.input(self.delivered());
        self.input(self.claim_released())
    }

    /// Every message this script delivered, by effect key.
    pub fn deliveries(&self) -> Vec<(String, DeliverBody)> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Deliver { key, body, .. } => Some((key, body)),
                _ => None,
            })
            .collect()
    }

    /// Every decision of this script with its instants as seconds from the script's start, one line per
    /// decision and an empty line per input: what a different clock must not change.
    pub fn transcript(&self) -> String {
        let mut text = String::new();
        for list in &self.lists {
            for decision in list {
                text.push_str(&render(decision, self.start));
                text.push('\n');
            }
            text.push('\n');
        }
        text
    }
}

pub fn slot(step: &str, visit: u32, member: &str, attempt: u32) -> Slot {
    Slot {
        step: step.to_owned(),
        visit,
        member: member.to_owned(),
        attempt,
    }
}

fn origin_of(body: &EventBody) -> Origin {
    match body {
        EventBody::Reply { .. }
        | EventBody::Answer { .. }
        | EventBody::Withdrawn { .. }
        | EventBody::Resume { .. }
        | EventBody::ResumeRun
        | EventBody::Commissioned { .. } => Origin::Cli,
        EventBody::SessionFact { .. } => Origin::Adapter,
        EventBody::TimerFired { .. } => Origin::Timer,
        _ => Origin::Host,
    }
}

fn cause_code(cause: &EndCause) -> &'static str {
    match cause {
        EndCause::TurnComplete => "turn_complete",
        EndCause::Exit { .. } => "exit",
        EndCause::UsageLimit { .. } => "usage_limit",
        EndCause::ProviderUnavailable => "provider_unavailable",
        EndCause::PermissionRefused { .. } => "permission_refused",
        EndCause::StoppedByHost => "stopped_by_host",
        EndCause::Lost => "lost",
        EndCause::Unknown(_) => "unknown",
    }
}

fn slot_name(slot: &Slot) -> String {
    format!(
        "{}/v{}/{}/a{}",
        slot.step, slot.visit, slot.member, slot.attempt
    )
}

fn word<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("a code serialises") {
        serde_json::Value::String(word) => word,
        other => other.to_string(),
    }
}

/// One line per decision with EVERY field of the contract type, instants as seconds from `now`, so a
/// list asserted as text is the whole decision and reads as the specification does.
pub fn render(decision: &Decision, now: Timestamp) -> String {
    let from_now = |at: &Timestamp| {
        let secs = (at.unix_millis() - now.unix_millis()) / 1000;
        format!("{secs:+}s")
    };
    match decision {
        Decision::RootRequested => "root_requested".to_owned(),
        Decision::ClaimRequested => "claim_requested".to_owned(),
        Decision::RunQueued { behind } => format!(
            "run_queued behind={}",
            behind.as_ref().map_or("none".to_owned(), |b| b.to_string())
        ),
        Decision::StepStarted {
            step,
            visit,
            attempt,
            members,
        } => {
            format!(
                "step_started {step} v{visit} a{attempt} [{}]",
                members.join(",")
            )
        }
        Decision::StepFinished {
            step,
            visit,
            outcome,
        } => {
            format!("step_finished {step} v{visit} {}", word(outcome))
        }
        Decision::Rework {
            from,
            to,
            declaring,
            pass,
        } => {
            format!("rework {from}->{to} declaring={declaring} pass={pass}")
        }
        Decision::Escalated {
            question,
            slot,
            message,
        } => {
            format!(
                "escalated question={question} slot={} message={message}",
                slot_name(slot)
            )
        }
        Decision::Ceiling {
            question,
            step,
            pass,
            last_verdict,
        } => {
            format!("ceiling question={question} step={step} pass={pass} verdict={last_verdict}")
        }
        Decision::TimerSet {
            timer,
            fire_at,
            purpose,
        } => {
            format!("timer_set {timer} {} {}", from_now(fire_at), word(purpose))
        }
        Decision::TimerCleared { timer } => format!("timer_cleared {timer}"),
        Decision::Liveness {
            slot,
            reason,
            since,
        } => format!(
            "liveness {} {} since={}",
            slot_name(slot),
            reason.as_str(),
            from_now(since)
        ),
        Decision::Held {
            slot,
            reason,
            until,
        } => format!(
            "held {} {} until={}",
            slot_name(slot),
            reason.as_str(),
            from_now(until)
        ),
        Decision::Parked { slot, reason } => format!(
            "parked {} {}",
            slot.as_ref().map_or("run".to_owned(), slot_name),
            reason.as_str()
        ),
        Decision::Finishing { outcome, reason } => format!(
            "finishing {} {}",
            word(outcome),
            reason.as_ref().map_or("none", |r| r.as_str())
        ),
        Decision::WithdrawForced => "withdraw_forced".to_owned(),
        Decision::Finished { outcome, reason } => format!(
            "finished {} {}",
            word(outcome),
            reason.as_ref().map_or("none", |r| r.as_str())
        ),
        Decision::Ignored {
            input_seq: _,
            reason,
        } => format!("ignored {}", reason.as_str()),
    }
}

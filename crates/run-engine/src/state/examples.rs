//! One instance of every non-final state, for `every_non_final_state_has_a_remedy_and_a_human_exit`. Each
//! is reached by folding real inputs through `decide`, so its timers are the ones a run really has. A new
//! variant of [`RunView`] or [`SlotState`] fails to compile in [`run_row`] or [`slot_row`] until it is
//! listed, and the test fails until an example reaches its row.

use nexus_run_contracts::{
    Base, Decision, EndCause, Event, EventBody, FlowRef, HeartbeatKind, LivenessPolicy, Origin,
    RunId, SessionFact, Slot, Timestamp,
};

use super::{ParkPhase, Phase, Process, RunState, RunView, SlotState, TreePhase};
use crate::decide::decide;
use crate::evolve::evolve;
use crate::flow::{FlowDef, StepDef, StepKind, WorkingTree, SUPERVISOR};

/// How many rows [`run_row`] has, `Finished` not among them.
pub const RUN_ROWS: usize = 9;
/// How many rows [`slot_row`] has.
pub const SLOT_ROWS: usize = 16;

/// The row of the table of §6.5 a run is on. `Finishing` is two rows, by its tree. `None` is final.
pub fn run_row(state: &RunState) -> Option<usize> {
    Some(match state.view() {
        RunView::Created => 0,
        RunView::Queued { .. } => 1,
        RunView::Running {
            tree: TreePhase::Unprepared,
        } => 2,
        RunView::Running {
            tree: TreePhase::Prepared,
        } => 3,
        RunView::NeedsDecision { .. } => 4,
        RunView::Held { .. } => 5,
        RunView::Parked { .. } => 6,
        RunView::Finishing { .. } => match state.phase {
            Phase::Finishing {
                tree: ParkPhase::Unparked,
                ..
            } => 7,
            _ => 8,
        },
        RunView::Finished { .. } => return None,
    })
}

/// The row of the table of §6.5 a slot is on: every `Ended` cause is a row of its own.
pub fn slot_row(state: &SlotState) -> usize {
    match state {
        SlotState::Pending => 0,
        SlotState::Starting { .. } => 1,
        SlotState::Running {
            open_call: None, ..
        } => 2,
        SlotState::Running {
            open_call: Some(_), ..
        } => 3,
        SlotState::Quiet { .. } => 4,
        SlotState::Nudged { .. } => 5,
        SlotState::Stopping { .. } => 6,
        SlotState::Replied {
            process: Process::Live,
            ..
        } => 7,
        SlotState::Replied {
            process: Process::Ended,
            ..
        } => 8,
        SlotState::Ended { cause } => match cause {
            EndCause::TurnComplete => 9,
            EndCause::Exit { .. } => 10,
            EndCause::UsageLimit { .. } => 11,
            EndCause::ProviderUnavailable => 12,
            EndCause::PermissionRefused { .. } => 13,
            EndCause::StoppedByHost => 14,
            EndCause::Lost => 15,
        },
    }
}

/// A run with a slot on a row of the table: the state, and which of its slots it is about.
#[derive(Clone, Debug)]
pub struct SlotExample {
    pub run: RunState,
    pub slot: Slot,
}

impl RunState {
    /// One run on every row of [`run_row`].
    pub fn non_final_examples() -> Vec<RunState> {
        let refused = || EndCause::PermissionRefused {
            rule: "rule".to_owned(),
            program: "program".to_owned(),
        };
        vec![
            Walk::commissioned(1).state,
            Walk::commissioned(1).opened().state,
            Walk::commissioned(1).opened().claimed().state,
            Walk::commissioned(1).running().started().state,
            Walk::commissioned(1).running().started().escalating().state,
            Walk::commissioned(1)
                .running()
                .started()
                .ended(EndCause::ProviderUnavailable)
                .state,
            Walk::commissioned(1)
                .running()
                .started()
                .ended(refused())
                .state,
            Walk::commissioned(1).running().started().withdrawn().state,
            Walk::commissioned(1)
                .running()
                .started()
                .withdrawn()
                .ended(EndCause::StoppedByHost)
                .parked()
                .state,
        ]
    }

    /// One run with a slot on every row of [`slot_row`].
    pub fn slot_examples() -> Vec<SlotExample> {
        let ended = |cause| {
            Walk::commissioned(1)
                .running()
                .started()
                .ended(cause)
                .first_slot()
        };
        vec![
            Walk::commissioned(3).opened().claimed().first_slot(),
            Walk::commissioned(3).running().first_slot(),
            Walk::commissioned(3).running().started().first_slot(),
            Walk::commissioned(3)
                .running()
                .started()
                .call_open()
                .first_slot(),
            Walk::commissioned(3)
                .running()
                .started()
                .quiet()
                .first_slot(),
            Walk::commissioned(3)
                .running()
                .started()
                .quiet()
                .nudged()
                .first_slot(),
            Walk::commissioned(3)
                .running()
                .started()
                .quiet()
                .nudged()
                .hung()
                .first_slot(),
            Walk::commissioned(3)
                .running()
                .started()
                .replied()
                .first_slot(),
            Walk::commissioned(3)
                .running()
                .started()
                .replied()
                .ended(EndCause::TurnComplete)
                .first_slot(),
            ended(EndCause::TurnComplete),
            ended(EndCause::Exit { status: Some(1) }),
            ended(EndCause::UsageLimit { resets_at: None }),
            ended(EndCause::ProviderUnavailable),
            ended(EndCause::PermissionRefused {
                rule: "rule".to_owned(),
                program: "program".to_owned(),
            }),
            ended(EndCause::StoppedByHost),
            ended(EndCause::Lost),
        ]
    }
}

impl SlotState {
    /// One instance of every row of [`slot_row`].
    pub fn non_final_examples() -> Vec<SlotState> {
        RunState::slot_examples()
            .iter()
            .filter_map(|example| {
                let index = example.run.find_slot(&example.slot)?;
                Some(example.run.slots[index].state.clone())
            })
            .collect()
    }
}

/// A two-step flow: the first step's reply leaves the run going, so a replied slot is not its end.
fn example_def() -> FlowDef {
    let step = |id: &str, target: &str, next: Option<&str>| StepDef {
        id: id.to_owned(),
        kind: StepKind::Single {
            target: target.to_owned(),
        },
        next: next.map(str::to_owned),
        on_needs_rework: None,
        max_passes: 1,
        input: None,
        resume: false,
        task: None,
        stage: None,
        channel: "example".to_owned(),
        opener: SUPERVISOR.to_owned(),
    };
    FlowDef {
        name: "example".to_owned(),
        steps: vec![
            step("first", "alpha", Some("second")),
            step("second", "beta", None),
        ],
        working_tree: WorkingTree::Exclusive,
        ignored_timeouts: Vec::new(),
    }
}

/// Folds inputs as a host does. Its instant stays put: these states are compared by what they hold.
struct Walk {
    state: RunState,
    def: FlowDef,
    seq: u64,
}

const AT: Timestamp = Timestamp::from_unix_millis(1_793_786_400_000);

impl Walk {
    fn commissioned(max_attempts: u32) -> Self {
        let def = example_def();
        let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").expect("a valid literal");
        let policy = LivenessPolicy {
            max_attempts,
            ..LivenessPolicy::default()
        };
        let walk = Self {
            state: RunState::initial(run.clone(), def.clone()),
            def,
            seq: 0,
        };
        walk.input(EventBody::Commissioned {
            run,
            workspace_id: "workspace".to_owned(),
            flow: FlowRef {
                hash: "sha256:example".to_owned(),
            },
            policy,
            root_thread: "m-root".to_owned(),
            requester: "person".to_owned(),
        })
    }

    fn input(mut self, body: EventBody) -> Self {
        self.seq += 1;
        let key = format!("example/{}", self.seq);
        let run = self.state.run.clone();
        let event = Event::new(run.clone(), self.seq, AT, key, Origin::Host, body);
        let decisions = decide(&self.state, &event, &self.def);
        self.fold(&event);
        for decision in decisions {
            self.decision(run.clone(), decision);
        }
        self
    }

    fn decision(&mut self, run: RunId, decision: Decision) {
        self.seq += 1;
        let key = format!("example/{}", self.seq);
        let body = EventBody::Decision(decision);
        let event = Event::new(run, self.seq, AT, key, Origin::Reducer, body);
        self.fold(&event);
    }

    fn fold(&mut self, event: &Event) {
        let state = self.state.clone();
        self.state = evolve(state, event).expect("the example's own events fold");
    }

    fn opened(self) -> Self {
        self.input(EventBody::RootOpened {
            thread: "m-root".to_owned(),
            message: "m-root-message".to_owned(),
        })
    }

    fn claimed(self) -> Self {
        self.input(EventBody::ClaimTaken)
    }

    fn running(self) -> Self {
        self.opened().claimed().input(EventBody::TreePrepared {
            base: Base {
                branch: "main".to_owned(),
                commit: "c0".to_owned(),
            },
            found: None,
        })
    }

    fn key(&self) -> String {
        self.state.session_key(&self.state.slots[0].slot)
    }

    fn fact(self, fact: SessionFact) -> Self {
        let session_key = self.key();
        self.input(EventBody::SessionFact { session_key, fact })
    }

    fn started(self) -> Self {
        self.fact(SessionFact::Started {
            backend_session_id: "backend".to_owned(),
            pid: Some(1),
        })
    }

    fn call_open(self) -> Self {
        self.fact(SessionFact::Heartbeat {
            beat: HeartbeatKind::CallsOpen,
        })
    }

    fn ended(self, cause: EndCause) -> Self {
        self.fact(SessionFact::Ended { cause })
    }

    fn fire(self, timer: &str) -> Self {
        let fire_at = self.state.timers[timer].fire_at;
        let timer = timer.to_owned();
        self.input(EventBody::TimerFired { timer, fire_at })
    }

    fn quiet(self) -> Self {
        self.fire("quiet/first/v1/alpha/a1")
    }

    fn nudged(self) -> Self {
        let session_key = self.key();
        self.input(EventBody::Nudged { session_key })
    }

    fn hung(self) -> Self {
        self.fire("confirm/first/v1/alpha/a1")
    }

    fn reply(self, escalate: bool) -> Self {
        let session_key = self.key();
        self.input(EventBody::Reply {
            session_key,
            message: "m-reply".to_owned(),
            escalate,
            needs_rework: false,
        })
    }

    fn replied(self) -> Self {
        self.reply(false)
    }

    fn escalating(self) -> Self {
        self.reply(true)
    }

    fn withdrawn(self) -> Self {
        self.input(EventBody::Withdrawn {
            message: None,
            by: nexus_run_contracts::WithdrawnBy::Requester,
        })
    }

    fn parked(self) -> Self {
        self.input(EventBody::TreeParked {
            outcome: nexus_run_contracts::ParkOutcome::Clean { branch: None },
        })
    }

    fn first_slot(&self) -> SlotExample {
        SlotExample {
            run: self.state.clone(),
            slot: self.state.slots[0].slot.clone(),
        }
    }
}

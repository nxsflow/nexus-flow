//! Folding one run: its state from the log, and one input at a time through `decide` and `evolve` into one
//! `append` (§6.2). The error classification of §13 lives here.
//!
//! A `Busy` or any other store error is retried next tick with the row untouched, so a healthy reply is never
//! consumed by a lock. What counts towards `poison_input` is a panic inside `decide` or `evolve`, a `CasFailed`
//! from `append`'s list that a re-read and one retry did not clear, and an inbox row that does not decode:
//! retrying cannot mend that, and the run would otherwise never get past it (the decision k8cw left to this
//! ticket). A damaged row in the LOG cannot be consumed, so it never counts: that run is read-only, like a
//! replay that panics, and keeps the withdraw exit.

use std::panic::{catch_unwind, AssertUnwindSafe};

use nexus_run_contracts::{
    Cas, Decision, Epoch, Event, EventBody, Liveness, Origin, Outcome, ReasonCode, RunId, RunRow,
    RunStore, SessionFact, Slot, StoreError,
};
use nexus_run_engine::{decide, decision_key, evolve, FlowDef, RunState};

use crate::boundary::{crash_at, wrote, Boundary};
use crate::host::{Host, ReadOnly};

/// The budget of a run that has no policy to read yet: the §6.5 default.
const DEFAULT_MAX_ATTEMPTS: u32 = 3;

#[cfg(feature = "test-support")]
fn input_hook(key: &str) {
    crate::test_support::panics::on_input(key);
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
fn input_hook(_key: &str) {}

/// The comparison a scripted `CasFailed` rides on: it can never hold, so no re-read clears it.
#[cfg(feature = "test-support")]
fn cas_hook(key: &str, cas: &mut Vec<Cas>) {
    if crate::test_support::panics::cas_fails(key) {
        cas.push(Cas {
            key: "scripted/cas".to_owned(),
            from: Some(b"never".to_vec()),
            to: None,
        });
    }
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
fn cas_hook(_key: &str, _cas: &mut Vec<Cas>) {}

#[cfg(feature = "test-support")]
fn replay_hook(key: &str) {
    crate::test_support::panics::on_replay(key);
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
fn replay_hook(_key: &str) {}

/// Where an input really came from, whatever its row says: the inbox is not trusted for this (M6). An unknown
/// kind can never pass for the reducer's, which would refuse ownership of the run.
fn true_origin(body: &EventBody) -> Origin {
    match body {
        EventBody::Commissioned { .. }
        | EventBody::Reply { .. }
        | EventBody::Answer { .. }
        | EventBody::Resume { .. }
        | EventBody::ResumeRun
        | EventBody::Withdrawn { .. } => Origin::Cli,
        EventBody::SessionFact { .. } => Origin::Adapter,
        EventBody::TimerFired { .. } => Origin::Timer,
        _ => Origin::Host,
    }
}

/// The definition a run was pinned to, by the hash its first event names.
pub(crate) fn pinned_def(store: &impl RunStore, first: &Event) -> Option<FlowDef> {
    let EventBody::Commissioned { flow, .. } = &first.body else {
        return None;
    };
    let bytes = store.get(&format!("flow/{}", flow.hash)).ok()??;
    serde_json::from_slice(&bytes).ok()
}

/// The fold of a log with what stopped it, for the host and for the bridge's read-only status.
pub(crate) enum Folded {
    /// Every event folded.
    Whole(RunState),
    NewerNxs(Option<RunState>),
    Unfoldable(Option<RunState>),
}

/// Folds `events`. A panic on a logged event or an unknown decision keeps what folded before it.
pub(crate) fn fold_events(run: &RunId, def: FlowDef, events: &[Event]) -> Folded {
    let mut state = RunState::initial(run.clone(), def);
    for event in events {
        let step = catch_unwind(AssertUnwindSafe(|| {
            replay_hook(&event.key);
            evolve(state.clone(), event)
        }));
        match step {
            Ok(Ok(next)) => state = next,
            Ok(Err(_)) => return Folded::NewerNxs(Some(state)),
            Err(_) => return Folded::Unfoldable(Some(state)),
        }
    }
    Folded::Whole(state)
}

impl Host {
    /// The run's state: cached, or folded once from the log (§7). `None` while it cannot be stepped.
    pub(crate) fn loaded_state(&mut self, row: &RunRow) -> Option<RunState> {
        let run = &row.run;
        if self.cache(run).read_only.is_some() {
            return None;
        }
        if let Some(state) = &self.cache(run).state {
            return Some(state.clone());
        }
        match self.store.load(run, 0) {
            Ok(events) => self.fold_loaded(row, &events),
            Err(StoreError::DamagedRow { seq: Some(_), .. }) => {
                self.mark_read_only(run, ReadOnly::Unfoldable, "a logged row does not decode");
                None
            }
            Err(error) => {
                self.note_store_error(&error);
                None
            }
        }
    }

    fn fold_loaded(&mut self, row: &RunRow, events: &[Event]) -> Option<RunState> {
        let run = &row.run;
        let inbox_first;
        let first = if let Some(first) = events.first() {
            first
        } else {
            inbox_first = match self.store.next_input(run) {
                Ok(Some(event)) => event,
                Ok(None) => return None,
                Err(error) => {
                    self.note_store_error(&error);
                    return None;
                }
            };
            &inbox_first
        };
        let Some(def) = pinned_def(self.store.as_ref(), first) else {
            self.mark_read_only(
                run,
                ReadOnly::Unfoldable,
                "the pinned definition does not read",
            );
            return None;
        };
        match fold_events(run, def, events) {
            Folded::Whole(state) => {
                self.cache(run).state = Some(state.clone());
                Some(state)
            }
            Folded::NewerNxs(_) => {
                self.mark_read_only(
                    run,
                    ReadOnly::NewerNxs,
                    "a decision this version cannot fold",
                );
                None
            }
            Folded::Unfoldable(_) => {
                self.mark_read_only(run, ReadOnly::Unfoldable, "a fold panicked");
                None
            }
        }
    }

    pub(crate) fn mark_read_only(&mut self, run: &RunId, why: ReadOnly, what: &'static str) {
        let cache = self.cache(run);
        cache.read_only = Some(why);
        cache.state = None;
        if cache.logged.insert(what.to_owned()) {
            self.log(run, "not stepped:", what);
        }
    }

    /// Every inbox entry of the run, oldest first, one `append` per input.
    pub(crate) fn fold_run(&mut self, row: &RunRow, epoch: Epoch) {
        while self.may_work() {
            if self.loaded_state(row).is_none() {
                return;
            }
            if !self.step_input(row, epoch) {
                return;
            }
        }
    }

    /// One input. `false` when there is nothing more to do for this run in this tick.
    fn step_input(&mut self, row: &RunRow, epoch: Epoch) -> bool {
        let run = &row.run;
        let input = match self.store.next_input(run) {
            Ok(Some(input)) => input,
            Ok(None) => return false,
            Err(StoreError::DamagedRow { key, seq: None }) => {
                return self.count_poison(row, epoch, &key)
            }
            Err(error) => {
                self.note_store_error(&error);
                return false;
            }
        };
        // A decision is not an input: folding one from the inbox would let a row write the reducer's mind.
        if matches!(input.body, EventBody::Decision(_)) {
            return self.count_poison(row, epoch, &input.key);
        }
        let Some(state) = self.loaded_state(row) else {
            return false;
        };
        let Ok(Some((events, next))) = self.reduce(&state, &input) else {
            return self.count_poison(row, epoch, &input.key);
        };
        self.commit_input(row, epoch, &state, &input.key, events, next)
    }

    /// `decide`, then `evolve` over the input and each decision. `Err` is a panic in either; `Ok(None)` is a
    /// fold that could not read what the reducer wrote, which is the same thing for this row.
    #[allow(clippy::type_complexity)]
    fn reduce(
        &self,
        state: &RunState,
        input: &Event,
    ) -> Result<Option<(Vec<Event>, RunState)>, ()> {
        let def = state.def.clone();
        let mut logged = input.clone();
        logged.seq = state.seq + 1;
        logged.origin = true_origin(&logged.body);
        let caught = catch_unwind(AssertUnwindSafe(|| {
            input_hook(&input.key);
            let decisions = decide(state, &logged, &def);
            let mut events = vec![logged.clone()];
            for (index, decision) in decisions.into_iter().enumerate() {
                events.push(Event::new(
                    state.run.clone(),
                    logged.seq + 1 + index as u64,
                    logged.at,
                    decision_key(&logged.key, index, &decision),
                    Origin::Reducer,
                    EventBody::Decision(decision),
                ));
            }
            let mut next = state.clone();
            for event in &events {
                match evolve(next, event) {
                    Ok(folded) => next = folded,
                    Err(_) => return None,
                }
            }
            Some((events, next))
        }));
        caught.map_err(|_| ())
    }

    fn commit_input(
        &mut self,
        row: &RunRow,
        epoch: Epoch,
        state: &RunState,
        key: &str,
        events: Vec<Event>,
        next: RunState,
    ) -> bool {
        let run = &row.run;
        let consume = [key.to_owned()];
        let mut retried = false;
        loop {
            let cas = match self.fold_cas(run, state, &events, key) {
                Ok(cas) => cas,
                Err(error) => {
                    self.note_store_error(&error);
                    return false;
                }
            };
            crash_at(Boundary::BeforeAppend);
            wrote("append");
            match self
                .store
                .append(run, state.seq, epoch, &events, &consume, &cas)
            {
                Ok(_) => {
                    self.cache(run).state = Some(next);
                    self.report.folded += 1;
                    crash_at(Boundary::AfterAppendBeforeEffect);
                    return true;
                }
                Err(StoreError::CasFailed { .. }) if !retried => retried = true,
                Err(StoreError::CasFailed { .. }) => return self.count_poison(row, epoch, key),
                Err(error) => return self.append_refused(run, error),
            }
        }
    }

    /// What a refused append means for this run's cached view.
    fn append_refused(&mut self, run: &RunId, error: StoreError) -> bool {
        match &error {
            StoreError::Fenced => {
                // The lease moved on: drop it, and re-acquire on the next tick (§13).
                self.log(run, "append refused:", error.kind());
                self.runs.remove(run);
            }
            // Already in the log, which is success: the store consumed the row, and the state is stale.
            StoreError::DuplicateKey { .. } | StoreError::SeqMismatch => {
                self.cache(run).state = None;
            }
            StoreError::VersionTooOld { .. } => {
                self.mark_read_only(run, ReadOnly::NewerNxs, "a newer version wrote to this run");
            }
            _ => self.note_store_error(&error),
        }
        false
    }

    /// The compare-and-sets that ride in this append (§7): the commission's body leaves with `root_opened`,
    /// everything of the run's own text with `finished`, and `started` records the persona's backend session.
    fn fold_cas(
        &self,
        run: &RunId,
        state: &RunState,
        events: &[Event],
        input_key: &str,
    ) -> Result<Vec<Cas>, StoreError> {
        let mut cas = Vec::new();
        let finished = events
            .iter()
            .any(|event| matches!(event.body, EventBody::Decision(Decision::Finished { .. })));
        match events.first().map(|event| &event.body) {
            Some(EventBody::RootOpened { .. }) if !is_ignored(events) => {
                self.delete_if_present(&format!("pending/{run}"), &mut cas)?;
            }
            Some(EventBody::SessionFact {
                session_key,
                fact:
                    SessionFact::Started {
                        backend_session_id, ..
                    },
            }) if !is_ignored(events) => {
                if let Some(index) = state.find(session_key) {
                    let handle = &state.slots[index].slot.member;
                    let key = format!("persona_session/{run}/{handle}");
                    let current = self.store.get(&key)?;
                    let value = backend_session_id.clone().into_bytes();
                    if current.as_deref() != Some(value.as_slice()) {
                        cas.push(Cas {
                            key,
                            from: current,
                            to: Some(value),
                        });
                    }
                }
            }
            _ => {}
        }
        if finished {
            self.delete_if_present(&format!("pending/{run}"), &mut cas)?;
            self.delete_if_present(&format!("tree_refusal/{run}"), &mut cas)?;
        }
        self.delete_if_present(&format!("poison/{run}/{input_key}"), &mut cas)?;
        cas_hook(input_key, &mut cas);
        Ok(cas)
    }

    pub(crate) fn delete_if_present(
        &self,
        key: &str,
        cas: &mut Vec<Cas>,
    ) -> Result<(), StoreError> {
        if let Some(value) = self.store.get(key)? {
            cas.push(Cas {
                key: key.to_owned(),
                from: Some(value),
                to: None,
            });
        }
        Ok(())
    }

    /// One failure of this row against `max_attempts`, the count in `kv['poison/<run>/<key>']`, so it
    /// survives a restart. At the budget the row is consumed by a synthetic `parked { poison_input }`.
    fn count_poison(&mut self, row: &RunRow, epoch: Epoch, key: &str) -> bool {
        let run = &row.run;
        let budget = self
            .cache(run)
            .state
            .as_ref()
            .map_or(DEFAULT_MAX_ATTEMPTS, |state| state.policy.max_attempts);
        let count_key = format!("poison/{run}/{key}");
        let current = match self.store.get(&count_key) {
            Ok(current) => current,
            Err(error) => {
                self.note_store_error(&error);
                return false;
            }
        };
        let count = current
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<u32>(bytes).ok())
            .unwrap_or(0)
            + 1;
        self.log(run, "an input did not fold:", key);
        if count >= budget {
            self.consume_poison(row, epoch, key);
            return false;
        }
        let value = serde_json::to_vec(&count).unwrap_or_default();
        if let Err(error) = self
            .store
            .compare_and_set(&count_key, current.as_deref(), Some(&value))
        {
            self.note_store_error(&error);
        }
        false
    }

    /// The row goes, and the run keeps its human exit. The event's key names the row, which is how status
    /// names it (§13).
    fn consume_poison(&mut self, row: &RunRow, epoch: Epoch, key: &str) {
        let run = &row.run;
        let Some(state) = self.loaded_state(row) else {
            return;
        };
        let at = self.clock.now();
        let event = Event::new(
            run.clone(),
            state.seq + 1,
            at,
            format!("poison/{run}/{key}"),
            Origin::Host,
            EventBody::Decision(Decision::Parked {
                slot: None,
                reason: ReasonCode::PoisonInput,
            }),
        );
        let mut cas = Vec::new();
        if let Err(error) = self.delete_if_present(&format!("poison/{run}/{key}"), &mut cas) {
            self.note_store_error(&error);
            return;
        }
        let events = [event];
        let consume = [key.to_owned()];
        crash_at(Boundary::BeforeAppend);
        match self
            .store
            .append(run, state.seq, epoch, &events, &consume, &cas)
        {
            Ok(_) => {
                self.log(run, "parked:", "poison_input");
                self.cache(run).state = None;
            }
            Err(error) => {
                self.append_refused(run, error);
            }
        }
    }

    /// §13's one exception: `nxc withdraw` on a run that cannot fold releases the claim through the host with
    /// no fold and no `tree_parked`, once no session of the run lives. It is safe because the next run's
    /// `PrepareTree` parks whatever it finds.
    pub(crate) fn withdraw_exit(
        &mut self,
        row: &RunRow,
        epoch: Epoch,
        now: nexus_run_contracts::Timestamp,
    ) {
        if self.cache(&row.run).read_only != Some(ReadOnly::Unfoldable) {
            return;
        }
        let run = &row.run;
        let Ok(waiting) = self.store.waiting_inputs(run) else {
            return;
        };
        let withdrawal = waiting.iter().find_map(|input| match input {
            Ok(event) if matches!(event.body, EventBody::Withdrawn { .. }) => Some(event.clone()),
            _ => None,
        });
        let Some(withdrawal) = withdrawal else {
            return;
        };
        let Ok(readable) = self.store.load_readable(run) else {
            return;
        };
        let live = readable.iter().any(|event| match &event.body {
            EventBody::Decision(Decision::StepStarted {
                step,
                visit,
                attempt,
                members,
            }) => members.iter().any(|member| {
                let slot = Slot {
                    step: step.clone(),
                    visit: *visit,
                    member: member.clone(),
                    attempt: *attempt,
                };
                !matches!(
                    self.backend.alive(&slot.session_key(run)),
                    Ok(Liveness::Dead)
                )
            }),
            _ => false,
        });
        if live {
            return;
        }
        let claim = format!("claim/{}", row.workspace_id);
        match self.store.get(&claim) {
            Ok(Some(holder)) if holder == run.as_str().as_bytes() => {
                if self
                    .store
                    .compare_and_set(&claim, Some(&holder), None)
                    .is_err()
                {
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                self.note_store_error(&error);
                return;
            }
        }
        let mut cas = Vec::new();
        let cleared = self
            .delete_if_present(&format!("pending/{run}"), &mut cas)
            .and_then(|()| self.delete_if_present(&format!("tree_refusal/{run}"), &mut cas));
        if let Err(error) = cleared {
            self.note_store_error(&error);
            return;
        }
        let mut input = withdrawal;
        input.seq = row.last_seq + 1;
        input.origin = Origin::Cli;
        let finished = Event::new(
            run.clone(),
            row.last_seq + 2,
            now,
            format!("{}/finished", input.key),
            Origin::Host,
            EventBody::Decision(Decision::Finished {
                outcome: Outcome::Withdrawn,
                reason: Some(ReasonCode::WithdrawnByRequester),
            }),
        );
        let consume = [input.key.clone()];
        let events = [input, finished];
        match self
            .store
            .append(run, row.last_seq, epoch, &events, &consume, &cas)
        {
            Ok(_) => {
                self.log(run, "withdrawn with no fold:", "claim released");
                self.runs.remove(run);
            }
            Err(error) => {
                self.append_refused(run, error);
            }
        }
    }
}

/// `decide` answered this input with one `ignored`: it changed nothing, and nothing rides with it.
fn is_ignored(events: &[Event]) -> bool {
    matches!(
        events.get(1).map(|event| &event.body),
        Some(EventBody::Decision(Decision::Ignored { .. }))
    )
}

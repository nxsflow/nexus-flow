//! Executing what a run owes (§8.2, §8.3): every effect through its port, adapter or tree worker, each behind
//! a re-check of the lease, each recorded by a result input, so a crash anywhere re-derives it and the ledger
//! says what was already done.

use nexus_run_contracts::{
    Epoch, EventBody, Key, MessageId, Origin, RunId, RunRow, RunStore, StopKind, StoreError,
    Timestamp,
};
use nexus_run_engine::{
    delivered_key, pending_effects, Asked, DeliverBody, Effect, Phase, RunState, SUPERVISOR,
};
use nexus_run_store_sqlite::EffectEntry;
use serde::{Deserialize, Serialize};

use crate::boundary::{crash_at, wrote, Boundary};
use crate::compose;
use crate::host::Host;
use crate::lease::still_ours;
use crate::ports::Refs;

/// The commission text and its refs, waiting in `pending/<run>` until the root is open (§6.4 item 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingCommission {
    pub root_thread: MessageId,
    pub channel: String,
    pub sender: String,
    pub body: String,
    pub refs: Refs,
}

/// Where an effect's ledger entry stands.
pub(crate) enum EntryPhase {
    /// Written now, or found from before a crash: the world may hold the effect already.
    Pending,
    Done(String),
}

pub(crate) const DONE: &str = "done:";
pub(crate) const PENDING: &str = "pending";

impl Host {
    pub(crate) fn effect_key(run: &RunId, key: &str) -> String {
        format!("effect/{run}/{key}")
    }

    fn entry_bytes(state: &str, epoch: Epoch) -> Result<Vec<u8>, StoreError> {
        serde_json::to_vec(&EffectEntry {
            state: state.to_owned(),
            epoch,
        })
        .map_err(|error| StoreError::Backend {
            what: error.to_string(),
        })
    }

    /// Reads an entry's state, or writes `initial` from absent. The store refuses a stale epoch as `Fenced`.
    pub(crate) fn entry_or_write(
        &self,
        run: &RunId,
        key: &str,
        epoch: Epoch,
        initial: &str,
    ) -> Result<String, StoreError> {
        let full = Self::effect_key(run, key);
        if let Some(bytes) = self.store.get(&full)? {
            let entry: EffectEntry =
                serde_json::from_slice(&bytes).map_err(|_| StoreError::DamagedRow {
                    key: full.clone(),
                    seq: None,
                })?;
            return Ok(entry.state);
        }
        let value = Self::entry_bytes(initial, epoch)?;
        wrote("ledger");
        self.store.compare_and_set(&full, None, Some(&value))?;
        Ok(initial.to_owned())
    }

    /// Moves an entry to a new state under this epoch, from whatever bytes it holds.
    pub(crate) fn entry_set(
        &self,
        run: &RunId,
        key: &str,
        epoch: Epoch,
        state: &str,
    ) -> Result<(), StoreError> {
        let full = Self::effect_key(run, key);
        let current = self.store.get(&full)?;
        let value = Self::entry_bytes(state, epoch)?;
        wrote("ledger");
        self.store
            .compare_and_set(&full, current.as_deref(), Some(&value))
    }

    fn entry_phase(&self, run: &RunId, key: &str, epoch: Epoch) -> Result<EntryPhase, StoreError> {
        let state = self.entry_or_write(run, key, epoch, PENDING)?;
        Ok(match state.strip_prefix(DONE) {
            Some(id) => EntryPhase::Done(id.to_owned()),
            None => EntryPhase::Pending,
        })
    }

    /// `pending` is reconciled by `find` first, so a crash after the post never posts twice (§8.3). A port
    /// that refuses posts nothing, and status says what the run waits for.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn find_or_post(
        &mut self,
        run: &RunId,
        epoch: Epoch,
        thread: &str,
        sender: &str,
        refs: &Refs,
        key: &str,
        body: impl FnOnce(&Self) -> Option<String>,
    ) -> Option<MessageId> {
        crash_at(Boundary::AfterLedgerBeforePost);
        let messages = self.ports.messages.clone();
        let found = match messages.find(thread, key) {
            Ok(found) => found,
            Err(_) => {
                self.mark_chat_refused(run, epoch);
                return None;
            }
        };
        let id = match found {
            Some(id) => id,
            None => {
                let text = body(self)?;
                wrote("post");
                match messages.post(thread, sender, &text, refs, key) {
                    Ok(id) => id,
                    Err(_) => {
                        self.mark_chat_refused(run, epoch);
                        return None;
                    }
                }
            }
        };
        crash_at(Boundary::AfterPostBeforeDone);
        self.clear_chat_refused(run);
        Some(id)
    }

    /// The row status reads `waiting_for: "the chat store"` from.
    fn mark_chat_refused(&mut self, run: &RunId, epoch: Epoch) {
        if let Err(error) = self.entry_or_write(run, CHAT_REFUSED, epoch, "the chat store") {
            self.note_store_error(&error);
        }
    }

    fn clear_chat_refused(&mut self, run: &RunId) {
        let full = Self::effect_key(run, CHAT_REFUSED);
        if let Ok(Some(bytes)) = self.store.get(&full) {
            if let Err(error) = self.store.compare_and_set(&full, Some(&bytes), None) {
                self.note_store_error(&error);
            }
        }
    }

    /// What the run owes now, in the order `pending_effects` gives it. `TakeClaim` is the tick's last part.
    pub(crate) fn execute_run(&mut self, row: &RunRow, epoch: Epoch, now: Timestamp) {
        let Some(state) = self.loaded_state(row) else {
            return;
        };
        self.cancel_if_withdrawn(&state);
        for effect in pending_effects(&state) {
            if !self.may_work() {
                return;
            }
            // Immediately before each effect: a host that lost the run executes nothing for it (§7).
            if !still_ours(
                &self.store,
                &row.run,
                &self.owner_id,
                epoch,
                self.clock.now(),
            ) {
                self.log(&row.run, "effect skipped:", "the lease moved on");
                self.runs.remove(&row.run);
                return;
            }
            let result = self.execute(&state, row, epoch, effect, now);
            if let Err(error) = result {
                if matches!(error, StoreError::Fenced) {
                    self.log(&row.run, "effect refused:", error.kind());
                    self.runs.remove(&row.run);
                    return;
                }
                self.note_store_error(&error);
            }
        }
    }

    fn execute(
        &mut self,
        state: &RunState,
        row: &RunRow,
        epoch: Epoch,
        effect: Effect,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let run = &row.run;
        match effect {
            Effect::OpenRoot { .. } => self.open_root(state, epoch, now)?,
            Effect::TakeClaim { .. } => {}
            Effect::PrepareTree { .. } => self.prepare_tree(run, now),
            Effect::CancelTree { .. } => self.cancel_tree(state, now),
            Effect::ParkTree {
                workspace,
                occasion,
            } => self.park_tree(run, &workspace, occasion, now)?,
            Effect::ReleaseClaim { workspace } => self.release_claim(run, &workspace, now)?,
            Effect::StartSession {
                session_key,
                turn,
                resume,
            } => self.start_session(state, epoch, &session_key, &turn, resume, now)?,
            Effect::NudgeSession { session_key, text } => {
                self.nudge(state, &session_key, &text, now);
            }
            Effect::StopSession {
                session_key,
                kind,
                nth,
            } => self.stop(run, epoch, &session_key, kind, nth, now)?,
            Effect::Deliver { thread, body, key } => {
                self.deliver(state, epoch, &thread, &body, &key, now)?;
            }
        }
        Ok(())
    }

    fn open_root(
        &mut self,
        state: &RunState,
        epoch: Epoch,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let run = &state.run;
        let ledger = format!("root/{run}");
        let message = match self.entry_phase(run, &ledger, epoch)? {
            EntryPhase::Done(id) => id,
            EntryPhase::Pending => {
                let Some(bytes) = self.store.get(&format!("pending/{run}"))? else {
                    self.log(run, "owes its root", "and no commission waits for it");
                    return Ok(());
                };
                let pending: PendingCommission =
                    serde_json::from_slice(&bytes).map_err(|_| StoreError::DamagedRow {
                        key: format!("pending/{run}"),
                        seq: None,
                    })?;
                crash_at(Boundary::AfterThreadIdBeforeOpen);
                wrote("open");
                let opened = self.ports.messages.open_thread(
                    &pending.root_thread,
                    None,
                    &pending.channel,
                    &pending.sender,
                );
                if opened.is_err() {
                    self.mark_chat_refused(run, epoch);
                    return Ok(());
                }
                let body = pending.body.clone();
                let Some(id) = self.find_or_post(
                    run,
                    epoch,
                    &pending.root_thread,
                    &pending.sender,
                    &pending.refs,
                    &ledger,
                    |_| Some(body),
                ) else {
                    return Ok(());
                };
                self.entry_set(run, &ledger, epoch, &format!("{DONE}{id}"))?;
                id
            }
        };
        crash_at(Boundary::AfterEffectBeforeResult);
        let thread = state.root_thread.clone().unwrap_or_default();
        self.enqueue(
            run,
            format!("root_opened/{run}"),
            Origin::Host,
            EventBody::RootOpened { thread, message },
            now,
        );
        Ok(())
    }

    fn deliver(
        &mut self,
        state: &RunState,
        epoch: Epoch,
        thread: &str,
        body: &DeliverBody,
        key: &str,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let run = &state.run;
        let message = match self.entry_phase(run, key, epoch)? {
            EntryPhase::Done(id) => id,
            EntryPhase::Pending => {
                let Some(id) = self.find_or_post(
                    run,
                    epoch,
                    thread,
                    SUPERVISOR,
                    &Refs::default(),
                    key,
                    |host| host.delivery_text(state, body),
                ) else {
                    return Ok(());
                };
                self.entry_set(run, key, epoch, &format!("{DONE}{id}"))?;
                id
            }
        };
        crash_at(Boundary::AfterEffectBeforeResult);
        self.enqueue(
            run,
            delivered_key(key),
            Origin::Host,
            EventBody::Delivered { message },
            now,
        );
        Ok(())
    }

    /// `None` leaves the effect owed: a failed read is never turned into an absence (§5).
    fn delivery_text(&self, state: &RunState, body: &DeliverBody) -> Option<String> {
        let dir = self.ports.layout.v2_dir();
        let dir = dir.to_string_lossy();
        match body {
            DeliverBody::Message(id) => Some(self.ports.messages.read(id).ok()??.body),
            DeliverBody::Discharge {
                step,
                pass,
                verdict,
                parked,
            } => Some(compose::discharge(
                state.outcome().as_ref(),
                step.as_deref(),
                *pass,
                verdict.as_ref(),
                parked.as_ref(),
                &dir,
            )),
            DeliverBody::ParkNotice { branch, commit } => {
                Some(compose::park_notice(branch, commit, &dir))
            }
            DeliverBody::Question(id) => {
                let question = state.questions.iter().find(|q| q.id == *id)?;
                let message = match &question.asked {
                    Asked::Escalation { message, .. } => message,
                    Asked::Ceiling { last_verdict, .. } => last_verdict,
                };
                let said = self.ports.messages.read(message).ok()??.body;
                let root = state.root_thread.as_deref().unwrap_or_default();
                Some(compose::question(question, root, Some(&said)))
            }
        }
    }

    /// A key that holds another run or nothing is the release already done (§6.4 item 9).
    fn release_claim(
        &mut self,
        run: &RunId,
        workspace: &str,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let key = format!("claim/{workspace}");
        let ours = run.as_str().as_bytes();
        if self.store.get(&key)?.as_deref() == Some(ours) {
            wrote("release");
            self.store.compare_and_set(&key, Some(ours), None)?;
        }
        crash_at(Boundary::AfterReleaseCasBeforeClaimReleased);
        self.enqueue(
            run,
            format!("claim_released/{run}"),
            Origin::Host,
            EventBody::ClaimReleased,
            now,
        );
        Ok(())
    }

    /// The slot state suppresses a second nudge for one `Quiet`: the result folds before the effect is
    /// derived again. A crash in between sends one more, which asks a session to speak and harms nothing.
    fn nudge(&mut self, state: &RunState, session_key: &str, text: &str, now: Timestamp) {
        if self.backend.nudge(session_key, text).is_err() {
            return;
        }
        wrote("nudge");
        crash_at(Boundary::AfterEffectBeforeResult);
        let since = state
            .find(session_key)
            .map_or(0, |index| match state.slots[index].state {
                nexus_run_engine::SlotState::Quiet { since, .. } => since.unix_millis(),
                _ => 0,
            });
        self.enqueue(
            &state.run,
            format!("nudged/{session_key}/{since}"),
            Origin::Host,
            EventBody::Nudged {
                session_key: session_key.to_owned(),
            },
            now,
        );
    }

    /// Exactly the table's signals across any restart (§6.5): the ledger entry is written BEFORE the signal
    /// and a found one is taken as sent, so a crash in between can only record, never signal twice. A signal
    /// lost that way is covered by the grace, and the next rung sends it.
    fn stop(
        &mut self,
        run: &RunId,
        epoch: Epoch,
        session_key: &str,
        kind: StopKind,
        nth: u32,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let ledger = format!("stop/{session_key}/{nth}");
        if self.store.get(&Self::effect_key(run, &ledger))?.is_none() {
            self.entry_or_write(run, &ledger, epoch, PENDING)?;
            crash_at(Boundary::BeforeSignal);
            wrote("signal");
            // A backend that cannot signal has confirmed nothing; the ladder and status carry on from here.
            if self
                .backend
                .stop(session_key, kind == StopKind::Kill)
                .is_err()
            {
                self.log(run, "a stop could not be sent:", session_key);
            }
        }
        crash_at(Boundary::BetweenSignalAndRecord);
        self.enqueue(
            run,
            format!("stop_signalled/{session_key}/{nth}"),
            Origin::Host,
            EventBody::StopSignalled {
                session_key: session_key.to_owned(),
                nth,
                signal: kind,
            },
            now,
        );
        Ok(())
    }

    /// The tick's last part: oldest queued run first, one CAS of the claim per workspace per tick (§6.6). A key
    /// that holds this run is the claim taken; an unchanged holder writes nothing.
    pub(crate) fn take_claims(&mut self, owned: &[(RunRow, Epoch)], now: Timestamp) {
        let mut tried: Vec<String> = Vec::new();
        for (row, _) in owned {
            if !self.may_work() {
                return;
            }
            let Some(state) = self.cache(&row.run).state.clone() else {
                continue;
            };
            let Phase::Queued { behind } = &state.phase else {
                continue;
            };
            let run = &row.run;
            let key = format!("claim/{}", row.workspace_id);
            let holder = match self.store.get(&key) {
                Ok(holder) => holder,
                Err(error) => {
                    self.note_store_error(&error);
                    return;
                }
            };
            let taken = match holder {
                Some(bytes) if bytes == run.as_str().as_bytes() => true,
                Some(bytes) => {
                    self.note_holder(&state, behind.as_ref(), &bytes, now);
                    false
                }
                None if tried.contains(&row.workspace_id) => false,
                None => {
                    tried.push(row.workspace_id.clone());
                    wrote("claim");
                    match self
                        .store
                        .compare_and_set(&key, None, Some(run.as_str().as_bytes()))
                    {
                        Ok(()) => true,
                        Err(StoreError::CasFailed { .. }) => false,
                        Err(error) => {
                            self.note_store_error(&error);
                            return;
                        }
                    }
                }
            };
            if taken {
                crash_at(Boundary::AfterClaimCasBeforeClaimTaken);
                self.enqueue(
                    run,
                    format!("claim_taken/{run}"),
                    Origin::Host,
                    EventBody::ClaimTaken,
                    now,
                );
            }
        }
    }

    /// `claim_busy` only when the holder differs from the one the state already carries (§6.4 item 2).
    fn note_holder(
        &mut self,
        state: &RunState,
        behind: Option<&RunId>,
        holder: &[u8],
        now: Timestamp,
    ) {
        let Some(holder) = std::str::from_utf8(holder).ok().and_then(RunId::parse) else {
            return;
        };
        if behind == Some(&holder) {
            return;
        }
        let key: Key = state.claim_busy_key();
        self.enqueue(
            &state.run,
            key,
            Origin::Host,
            EventBody::ClaimBusy { holder },
            now,
        );
    }
}

/// The ledger key behind `waiting_for: "the chat store"`.
pub(crate) const CHAT_REFUSED: &str = "waiting/chat_store";

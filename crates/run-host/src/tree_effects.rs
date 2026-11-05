//! The tree side of the host (§6.6, §8.3): `PrepareTree` and `ParkTree` start an operation and return,
//! `poll_tree` folds what finished, and refusals live in `kv['tree_refusal/<run>']` until the window or a
//! person decides. What is owed after a restart comes from `pending_effects`, never from the worker's ledger.

use std::collections::BTreeMap;

use nexus_run_contracts::{
    Epoch, EventBody, GitReason, Origin, Outcome, ParkOutcome, RunId, RunRow, RunStore, StoreError,
    Timestamp, TreeRefusal,
};
use nexus_run_engine::RunState;
use serde::{Deserialize, Serialize};

use crate::boundary::{crash_at, Boundary};
use crate::host::Host;
use crate::lease::still_ours;
use crate::tree_worker::{TreeOp, TreeResult};

const BACKOFF_START_MS: i64 = 1_000;
const BACKOFF_MAX_MS: i64 = 60_000;
const REFUSAL_TEXT_CAP: usize = 2_000;

/// A `tree_window` is not closed on fewer attempts than this, beside the elapsed time: a machine that slept
/// through the window has not tried. An operation in flight counts as one.
pub(crate) const MIN_TREE_ATTEMPTS: u32 = 3;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Attempts {
    pub refused: u32,
    pub next_at: Option<Timestamp>,
}

/// In memory only: a restart tries again at once, and the window it is measured against is in the log.
#[derive(Default)]
pub(crate) struct TreeTrack {
    pub running: Option<TreeOp>,
    pub cancelled: bool,
    pub attempts: BTreeMap<RunId, Attempts>,
}

/// The value of `tree_refusal/<run>`. `at` is when this refusal's text last changed, the contract's field,
/// and `first_refused_at` when the standing refusal began: they are never the same question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredRefusal {
    pub reason: TreeRefusal,
    pub text: String,
    pub at: Timestamp,
    pub first_refused_at: Timestamp,
}

/// Permanent refusals park at once. One this version does not know is treated as permanent, as the
/// reducer treats it.
fn is_permanent(reason: &TreeRefusal) -> bool {
    matches!(
        reason,
        TreeRefusal::NotARepository | TreeRefusal::ForeignDirectory | TreeRefusal::Unknown(_)
    )
}

/// Capped, and with anything between `://` and `@` removed, so a token in a URL never reaches a row.
fn scrub(text: &str) -> String {
    let mut clean = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("://") {
        let (head, tail) = rest.split_at(start + 3);
        clean.push_str(head);
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        rest = match tail[..end].rfind('@') {
            Some(at) => &tail[at + 1..],
            None => tail,
        };
    }
    clean.push_str(rest);
    clean.chars().take(REFUSAL_TEXT_CAP).collect()
}

impl Host {
    fn may_begin(&mut self, run: &RunId, now: Timestamp) -> bool {
        if self.track.running.is_some() {
            return false;
        }
        let attempts = self.track.attempts.entry(run.clone()).or_default();
        attempts.next_at.is_none_or(|at| at <= now)
    }

    fn begin(&mut self, op: TreeOp) {
        if self.tree_op(op.clone()).is_ok() {
            self.track.running = Some(op);
            self.track.cancelled = false;
        }
    }

    pub(crate) fn prepare_tree(&mut self, run: &RunId, now: Timestamp) {
        if self.may_begin(run, now) {
            let layout = &self.ports.layout;
            let op = TreeOp::Prepare {
                run: run.clone(),
                dir: layout.v2_dir(),
                origin_url: layout.origin_url.clone(),
            };
            self.begin(op);
        }
    }

    /// Executed only while the claim key holds this run: a run that never took the claim never touches a
    /// tree another run holds (§6.2).
    pub(crate) fn park_tree(
        &mut self,
        run: &RunId,
        workspace: &str,
        occasion: Outcome,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let holds = self.store.get(&format!("claim/{workspace}"))?;
        if holds.as_deref() != Some(run.as_str().as_bytes()) {
            self.enqueue(
                run,
                format!("tree_parked/{run}"),
                Origin::Host,
                EventBody::TreeParked {
                    outcome: ParkOutcome::NotHeld,
                },
                now,
            );
            return Ok(());
        }
        if self.may_begin(run, now) {
            let op = TreeOp::Park {
                run: run.clone(),
                dir: self.ports.layout.v2_dir(),
                occasion,
            };
            self.begin(op);
        }
        Ok(())
    }

    /// `tree_window` closed: cancel what is in flight, if anything, and enqueue the standing refusal, which
    /// is owed until it folds.
    pub(crate) fn cancel_tree(&mut self, state: &RunState, now: Timestamp) {
        let run = &state.run;
        if self
            .track
            .running
            .as_ref()
            .is_some_and(|op| op.run() == run)
        {
            self.cancel_running();
        }
        let standing = self.standing_refusal(run);
        let reason = standing.map_or(
            TreeRefusal::Git {
                reason: GitReason::BoundExceeded,
            },
            |refusal| refusal.reason,
        );
        self.enqueue(
            run,
            state.tree_refused_key(),
            Origin::Host,
            EventBody::TreeRefused { reason },
            now,
        );
    }

    fn cancel_running(&mut self) {
        if !self.track.cancelled {
            self.tree.cancel();
            self.track.cancelled = true;
        }
    }

    /// `withdrawn` cancels an in-flight Prepare: its result would only fold to `ignored`, and the park waits.
    pub(crate) fn cancel_if_withdrawn(&mut self, state: &RunState) {
        let preparing = matches!(
            &self.track.running,
            Some(TreeOp::Prepare { run, .. }) if *run == state.run
        );
        if preparing && state.is_finishing() {
            self.cancel_running();
        }
    }

    /// Whether to hold back a due `tree_window`, the minimum attempt count beside the elapsed time.
    pub(crate) fn holds_tree_window(&self, run: &RunId) -> bool {
        let refused = self.track.attempts.get(run).map_or(0, |a| a.refused);
        let running = self
            .track
            .running
            .as_ref()
            .is_some_and(|op| op.run() == run);
        refused + u32::from(running) < MIN_TREE_ATTEMPTS
    }

    /// Folds what the worker finished. A result is enqueued only under a live lease: without one it is
    /// dropped, and whoever owns the run re-derives the operation.
    pub(crate) fn poll_tree(&mut self, owned: &[(RunRow, Epoch)], now: Timestamp) {
        while let Some((op, result)) = self.tree.poll() {
            self.track.running = None;
            self.track.cancelled = false;
            let run = op.run().clone();
            let holding = owned.iter().find(|(row, _)| row.run == run);
            let live = holding.is_some_and(|(_, epoch)| {
                still_ours(&self.store, &run, &self.owner_id, *epoch, self.clock.now())
            });
            if !live {
                self.log(&run, "a tree result was dropped:", "the lease moved on");
                continue;
            }
            crash_at(Boundary::AfterTreeResultBeforeInput);
            self.fold_tree_result(&op, result, now);
        }
    }

    fn fold_tree_result(&mut self, op: &TreeOp, result: TreeResult, now: Timestamp) {
        let run = op.run().clone();
        match (op, result) {
            (TreeOp::Prepare { .. }, TreeResult::Prepared { base, found }) => {
                self.track.attempts.remove(&run);
                self.clear_refusal(&run);
                self.enqueue(
                    &run,
                    format!("tree_prepared/{run}"),
                    Origin::Host,
                    EventBody::TreePrepared { base, found },
                    now,
                );
            }
            (TreeOp::Park { .. }, TreeResult::Parked(outcome)) => {
                self.track.attempts.remove(&run);
                self.clear_refusal(&run);
                self.enqueue(
                    &run,
                    format!("tree_parked/{run}"),
                    Origin::Host,
                    EventBody::TreeParked { outcome },
                    now,
                );
            }
            (TreeOp::Prepare { .. }, TreeResult::Refused { reason, text }) => {
                self.record_refusal(&run, &reason, &text, now);
                let state = self.cache(&run).state.clone();
                let finishing = state.as_ref().is_some_and(RunState::is_finishing);
                if is_permanent(&reason) || finishing {
                    if let Some(state) = state {
                        self.enqueue(
                            &run,
                            state.tree_refused_key(),
                            Origin::Host,
                            EventBody::TreeRefused { reason },
                            now,
                        );
                    }
                }
            }
            // A refused park holds the claim: it retries whatever a window says, because a park that gives
            // up loses work, and status carries the refusal and the remedy (§6.5).
            (TreeOp::Park { .. }, TreeResult::Refused { reason, text }) => {
                self.record_refusal(&run, &reason, &text, now);
            }
            (op, _) => self.log(&run, "a tree result of another kind:", op.name()),
        }
    }

    /// One more refusal: its backoff, and the kv row, rewritten only when the refusal changes (no per-tick
    /// write, §7).
    fn record_refusal(&mut self, run: &RunId, reason: &TreeRefusal, text: &str, now: Timestamp) {
        let attempts = self.track.attempts.entry(run.clone()).or_default();
        attempts.refused += 1;
        let doubling =
            BACKOFF_START_MS.saturating_mul(1 << attempts.refused.min(16).saturating_sub(1));
        attempts.next_at = Some(Timestamp::from_unix_millis(
            now.unix_millis() + doubling.min(BACKOFF_MAX_MS),
        ));
        let key = format!("tree_refusal/{run}");
        let current = match self.store.get(&key) {
            Ok(current) => current,
            Err(error) => return self.note_store_error(&error),
        };
        let stored: Option<StoredRefusal> = current
            .as_deref()
            .and_then(|bytes| serde_json::from_slice(bytes).ok());
        let text = scrub(text);
        if stored
            .as_ref()
            .is_some_and(|s| s.reason == *reason && s.text == text)
        {
            return;
        }
        let refusal = StoredRefusal {
            reason: reason.clone(),
            text,
            at: now,
            first_refused_at: stored.map_or(now, |s| s.first_refused_at),
        };
        let Ok(value) = serde_json::to_vec(&refusal) else {
            return;
        };
        if let Err(error) = self
            .store
            .compare_and_set(&key, current.as_deref(), Some(&value))
        {
            self.note_store_error(&error);
        }
    }

    fn standing_refusal(&self, run: &RunId) -> Option<StoredRefusal> {
        let bytes = self.store.get(&format!("tree_refusal/{run}")).ok()??;
        serde_json::from_slice(&bytes).ok()
    }

    fn clear_refusal(&mut self, run: &RunId) {
        let key = format!("tree_refusal/{run}");
        if let Ok(Some(bytes)) = self.store.get(&key) {
            if let Err(error) = self.store.compare_and_set(&key, Some(&bytes), None) {
                self.note_store_error(&error);
            }
        }
    }
}

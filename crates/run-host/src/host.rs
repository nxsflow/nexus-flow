//! `Host::tick` (§8.1), per workspace and in this order: refresh the open runs; acquire or renew each lease;
//! drain adapter facts and the tree worker's results into the inboxes; enqueue due timers; fold the inboxes;
//! execute what every run owes; one `TakeClaim` for the oldest queued run. A result is drained BEFORE the fold,
//! so the state the effects read has it, and the operation it finished is not owed a second time. Every part
//! is bounded: the
//! budget yields what is left to the next tick, a `Busy` ends the writing for this tick, no call waits on a
//! session or on git, and a panic in one run's step stays in that run.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nexus_run_contracts::{
    EndCause, EnvVar, Epoch, Event, EventBody, Key, Origin, RunId, RunRow, RunStore,
    SessionBackend, SessionFact, SessionKey, StoreError, TimerPurpose, Timestamp,
};
use nexus_run_engine::RunState;
use nexus_run_store_sqlite::SqliteRunStore;
use ulid::Ulid;

use crate::boundary::{crash_at, wrote, Boundary};
use crate::lease::{acquire, Holding};
use crate::ports::{Clock, DeclarationPort, MessagePort, SidecarPort};
use crate::tree_effects::TreeTrack;
use crate::tree_worker::{TreeBinding, TreeOp, WorkTree};

/// 200 ms of work per workspace per tick (§8.1).
pub const TICK_BUDGET: Duration = Duration::from_millis(200);

/// Where a line of the host's log goes. A line carries ids and codes only, never a body (M8).
pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Where a workspace's things are. `session_env` holds the values of the allowlist the old side knows.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    pub agent_logs: PathBuf,
    pub session_env: Vec<EnvVar>,
}

impl Layout {
    /// `<workspace root>-v2`: derived at execution and never stored (§6.6, M6).
    pub fn v2_dir(&self) -> PathBuf {
        let mut name = self.root.as_os_str().to_owned();
        name.push("-v2");
        PathBuf::from(name)
    }

    pub fn ledger_dir(&self, session_key: &str) -> PathBuf {
        self.agent_logs.join("v2").join(session_key)
    }
}

#[derive(Clone)]
pub struct Ports {
    pub messages: Arc<dyn MessagePort>,
    pub declarations: Arc<dyn DeclarationPort>,
    pub sidecar: Arc<dyn SidecarPort>,
    pub layout: Layout,
}

/// What one tick did, for the service's own measures and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The budget ran out, and what was left waits for the next tick.
    pub yielded: bool,
    /// The store answered `Busy`, and the rest of the writing waits for the next tick.
    pub busy: bool,
    pub folded: u32,
}

/// The parts of a tick that give each run a turn, each with its own place in the rotation.
#[derive(Clone, Copy)]
enum Part {
    Drain,
    Fold,
    Execute,
}

/// Why a run is read-only: not stepped, only projected (§13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadOnly {
    /// A decision this version cannot fold: `waiting_for: "a newer nxs"`.
    NewerNxs,
    /// A logged event panicked the fold, or does not decode: the withdraw exit applies.
    Unfoldable,
}

/// What the host keeps about one run. All of it can be lost: a restart reads it again.
#[derive(Default)]
pub(crate) struct RunCache {
    pub state: Option<RunState>,
    pub read_only: Option<ReadOnly>,
    /// Facts delivered so far, per session: read from the ledger's `offset` file once.
    pub offsets: BTreeMap<SessionKey, u64>,
    /// `StartSession` executed by this host: the ledger answers for what a restart forgot.
    pub started: BTreeSet<SessionKey>,
    /// `parked { poison_input }` and read-only are logged once, not per tick.
    pub logged: BTreeSet<String>,
}

pub struct Host {
    pub(crate) store: Arc<SqliteRunStore>,
    pub(crate) ports: Ports,
    pub(crate) backend: Arc<dyn SessionBackend>,
    pub(crate) tree: Arc<dyn WorkTree>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) owner_id: String,
    pub(crate) runs: BTreeMap<RunId, RunCache>,
    pub(crate) track: TreeTrack,
    budget: Duration,
    log: LogSink,
    pub(crate) report: TickReport,
    started_at: Instant,
    /// Where each part of the tick goes on, by index into the open runs: an open run finishing moves it by one,
    /// and that only changes whose turn is next. Lost on a restart, like the rest of what is in memory.
    next_turn: [usize; 3],
    /// The run being stepped has this tick's one turn, whatever is left of the budget.
    pub(crate) turn_owed: bool,
}

/// An opaque host id: no person, no user name, no machine name (the tombstone rule of contract (b)). It is
/// minted per `Host`, so a restarted service waits out the old lease, and two services on one store can
/// never pass for one owner.
pub fn mint_owner_id() -> String {
    format!("h-{}", Ulid::new())
}

impl Host {
    pub fn new(
        store: Arc<SqliteRunStore>,
        ports: Ports,
        backend: Arc<dyn SessionBackend>,
        tree: Arc<dyn WorkTree>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let owner_id = mint_owner_id();
        tree.bind(TreeBinding {
            owner_id: owner_id.clone(),
            leases: store.clone(),
            clock: clock.clone(),
        });
        Self {
            store,
            ports,
            backend,
            tree,
            clock,
            owner_id,
            runs: BTreeMap::new(),
            track: TreeTrack::default(),
            budget: TICK_BUDGET,
            log: Arc::new(|line| eprintln!("{line}")),
            report: TickReport::default(),
            started_at: Instant::now(),
            next_turn: [0; 3],
            turn_owed: false,
        }
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// Sends the log lines somewhere else than stderr.
    pub fn set_log_sink(&mut self, sink: LogSink) {
        self.log = sink;
    }

    /// Replaces the budget of §8.1, which is a policy value.
    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    /// Takes ids and codes only, so a body cannot reach a line by accident (M8).
    pub(crate) fn log(&self, run: &RunId, what: &'static str, code: &str) {
        (self.log)(&format!("run-host {run}: {what} {code}"));
    }

    /// Whether this tick may go on. The budget is real time, whatever the injected clock says. A `Busy` stops
    /// everything; a budget that ran out stops everything but the turn a part of the tick owes (see `each_run`).
    pub(crate) fn may_work(&mut self) -> bool {
        if self.report.busy {
            return false;
        }
        if self.started_at.elapsed() >= self.budget {
            self.report.yielded = true;
            return self.turn_owed;
        }
        true
    }

    /// One iteration, in the order of §8.1.
    pub fn tick(&mut self, now: Timestamp) -> TickReport {
        self.report = TickReport::default();
        self.started_at = Instant::now();
        let rows = match self.store.open_runs() {
            Ok(rows) => rows,
            Err(error) => {
                self.note_store_error(&error);
                return self.report;
            }
        };
        let open: BTreeSet<RunId> = rows.iter().map(|row| row.run.clone()).collect();
        self.runs.retain(|run, _| open.contains(run));
        let owned = self.leases(&rows, now);
        self.mark_seen(&rows, now);
        self.each_run(Part::Drain, &owned, |host, row, epoch| {
            host.drain_run(row, epoch, now)
        });
        self.poll_tree(&owned, now);
        self.fire_due_timers(&owned, now);
        self.each_run(Part::Fold, &owned, |host, row, epoch| {
            host.fold_run(row, epoch)
        });
        self.each_run(Part::Execute, &owned, |host, row, epoch| {
            host.withdraw_exit(row, epoch, now);
            host.execute_run(row, epoch, now);
        });
        self.take_claims(&owned, now);
        self.report
    }

    /// Every open run's lease, whatever the budget says, because a lease not renewed loses the run. What this
    /// costs comes out of the budget; `each_run` keeps a tick from ending with nothing else done.
    fn leases(&mut self, rows: &[RunRow], now: Timestamp) -> Vec<(RunRow, Epoch)> {
        let mut owned = Vec::new();
        for row in rows {
            // A lease that needs no write would still answer, but one locked store costs one timeout per
            // tick, not one per run, or the tick outlasts its budget. The rest wait for the next tick.
            if self.report.busy {
                self.runs.remove(&row.run);
                continue;
            }
            match acquire(&self.store, &row.run, &self.owner_id, now) {
                Ok(Holding::Ours(epoch)) => owned.push((row.clone(), epoch)),
                Ok(Holding::Skip) => {
                    // Whatever was cached is another host's to move now.
                    self.runs.remove(&row.run);
                }
                Err(error) => {
                    self.runs.remove(&row.run);
                    self.note_store_error(&error);
                }
            }
        }
        owned
    }

    fn mark_seen(&mut self, rows: &[RunRow], now: Timestamp) {
        let workspaces: BTreeSet<&str> = rows.iter().map(|row| row.workspace_id.as_str()).collect();
        for workspace in workspaces {
            if self.report.busy {
                return;
            }
            if let Err(error) = self.store.mark_host_seen(workspace, now) {
                self.note_store_error(&error);
            }
        }
    }

    /// The run boundary of §8.1: a panic in one run's step is logged once with its id, its cached state is
    /// dropped, and that run is skipped until the next tick.
    ///
    /// The budget decides how far a tick gets, never who is served. A part of the tick starts where it last ran
    /// out, so a run that spends every tick cannot keep the runs behind it from their turn; and the first run
    /// of a part has its turn however little is left, so a tick whose budget is already gone still serves one
    /// run. As many ticks as there are runs give every run a turn at each part, on any machine.
    fn each_run(
        &mut self,
        part: Part,
        owned: &[(RunRow, Epoch)],
        mut step: impl FnMut(&mut Self, &RunRow, Epoch),
    ) {
        // A tick that holds nothing, a `Busy` at the first lease, has not been through the runs.
        if owned.is_empty() {
            return;
        }
        let start = self.next_turn[part as usize] % owned.len();
        for offset in 0..owned.len() {
            let index = (start + offset) % owned.len();
            self.turn_owed = offset == 0;
            if !self.may_work() {
                self.turn_owed = false;
                self.next_turn[part as usize] = index;
                return;
            }
            let (row, epoch) = &owned[index];
            let caught = catch_unwind(AssertUnwindSafe(|| step(self, row, *epoch)));
            if caught.is_err() {
                self.log(&row.run, "a step panicked", "skipped until the next tick");
                self.runs.remove(&row.run);
            }
        }
        self.turn_owed = false;
    }

    pub(crate) fn note_store_error(&mut self, error: &StoreError) {
        if matches!(error, StoreError::Busy) {
            self.report.busy = true;
        }
    }

    pub(crate) fn cache(&mut self, run: &RunId) -> &mut RunCache {
        self.runs.entry(run.clone()).or_default()
    }

    /// Puts an input into the run's inbox. `origin` is set here, from where the input really came (M6).
    pub(crate) fn enqueue(
        &mut self,
        run: &RunId,
        key: impl Into<Key>,
        origin: Origin,
        body: EventBody,
        at: Timestamp,
    ) -> bool {
        let event = Event::new(run.clone(), 0, at, key, origin, body);
        wrote("enqueue");
        match self.store.enqueue(run, &event, &[]) {
            Ok(_) => true,
            Err(error) => {
                self.note_store_error(&error);
                false
            }
        }
    }

    /// Drains every session of this run that may have a process (§9). The facts reach the inbox before the
    /// offset moves, so a crash delivers a fact twice and the key makes the second a `Duplicate`.
    fn drain_run(&mut self, row: &RunRow, _epoch: Epoch, now: Timestamp) {
        let Some(state) = self.loaded_state(row) else {
            return;
        };
        let live: Vec<(SessionKey, bool)> = state
            .slots
            .iter()
            .filter(|record| record.state.is_live())
            .map(|record| {
                (
                    state.session_key(&record.slot),
                    record.state.ladder().is_some(),
                )
            })
            .collect();
        for (session_key, stopping) in live {
            if !self.may_work() {
                return;
            }
            self.drain_session(&row.run, &session_key, stopping, now);
        }
    }

    fn drain_session(&mut self, run: &RunId, key: &str, stopping: bool, now: Timestamp) {
        let from = self.offset_of(run, key);
        let Ok((facts, next)) = self.backend.drain_facts(key, from) else {
            return;
        };
        let quiet_now = facts.is_empty();
        for drained in facts {
            let fact_key = format!("fact/{key}/{}", drained.offset);
            let body = EventBody::SessionFact {
                session_key: key.to_owned(),
                fact: drained.fact,
            };
            if !self.enqueue(run, fact_key, Origin::Adapter, body, now) {
                return;
            }
        }
        crash_at(Boundary::BetweenFactsReadAndOffset);
        if next != from {
            wrote("offset");
            self.store_offset(run, key, next);
        }
        // An end is confirmed by the backend, and only once its facts are drained to the end (§8.3).
        if stopping && quiet_now {
            if let Ok(nexus_run_contracts::Liveness::Dead) = self.backend.alive(key) {
                let body = EventBody::SessionFact {
                    session_key: key.to_owned(),
                    fact: SessionFact::Ended {
                        cause: EndCause::StoppedByHost,
                    },
                };
                self.enqueue(
                    run,
                    format!("fact/{key}/stopped"),
                    Origin::Adapter,
                    body,
                    now,
                );
            }
        }
    }

    fn offset_path(&self, key: &str) -> PathBuf {
        self.ports.layout.ledger_dir(key).join("offset")
    }

    fn offset_of(&mut self, run: &RunId, key: &str) -> u64 {
        if let Some(offset) = self.cache(run).offsets.get(key) {
            return *offset;
        }
        let offset = std::fs::read_to_string(self.offset_path(key))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        self.cache(run).offsets.insert(key.to_owned(), offset);
        offset
    }

    /// By temp file and rename. A write that fails is only a second delivery after the next restart.
    fn store_offset(&mut self, run: &RunId, key: &str, offset: u64) {
        let path = self.offset_path(key);
        let temp = path.with_extension("tmp");
        let wrote = std::fs::write(&temp, offset.to_string())
            .and_then(|()| std::fs::rename(&temp, &path))
            .is_ok();
        if wrote {
            self.cache(run).offsets.insert(key.to_owned(), offset);
        }
    }

    /// Due timers of the runs this host owns, with the real now: the wake rule is the reducer's, by
    /// `timer_fired.at` against its `fire_at` (§6.5), so nothing is deferred here. The one thing held back is
    /// a `tree_window` that closes before its minimum attempts.
    /// Not on the budget: a timer is how a stuck run gets out, a tick owes few of them, and a machine out of
    /// budget is where it is needed.
    fn fire_due_timers(&mut self, owned: &[(RunRow, Epoch)], now: Timestamp) {
        if self.report.busy {
            return;
        }
        let due = match self.store.due_timers(now) {
            Ok(due) => due,
            Err(error) => {
                self.note_store_error(&error);
                return;
            }
        };
        for timer in due {
            if !owned.iter().any(|(row, _)| row.run == timer.run) {
                continue;
            }
            if timer.purpose == TimerPurpose::TreeWindow && self.holds_tree_window(&timer.run) {
                continue;
            }
            let key = format!("timer/{}/{}", timer.timer, timer.fire_at);
            let body = EventBody::TimerFired {
                timer: timer.timer,
                fire_at: timer.fire_at,
            };
            self.enqueue(&timer.run, key, Origin::Timer, body, now);
        }
    }

    pub(crate) fn tree_op(&self, op: TreeOp) -> Result<(), crate::tree_worker::InFlight> {
        self.tree.begin(op)
    }
}

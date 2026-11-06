//! The ledger of who is doing a tree operation (§8.3): the file `<agent-logs>/v2/tree/<workspace_id>.json`, written
//! before each spawn with the owner and epoch the operation runs under, and read first by whoever comes next. The
//! tree itself is the ledger for WHAT is done, so what a run still owes never comes from here.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nexus_run_contracts::status::InFlight as InFlightStatus;
use nexus_run_contracts::{Epoch, RunId, Timestamp};
use serde::{Deserialize, Serialize};

use nexus_run_session_claude::procs::ProcessGuard;

/// How long a ledger entry that says a spawn is under way, with no child record yet, is waited on: the child
/// writes that record as its first act (§8.3's `spawn.json` and `child.json`, the same gap). Real time, because
/// the child's start is, and no injected clock moves it.
pub const SPAWN_GRACE: Duration = Duration::from_secs(2);

/// One entry of the ledger file. `pid` and `start_time` are absent until the parent has recorded the child.
/// `spawning` says one may exist by then: the child also records itself in the file beside this one as its first
/// act, so a crash in that gap still leaves a child somebody can name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub run: RunId,
    pub op: String,
    pub step: String,
    pub pid: Option<u32>,
    pub start_time: Option<String>,
    pub began_at: Timestamp,
    pub owner_id: String,
    pub epoch: Epoch,
    /// Whether `index.lock` was there before the child: only a lock a call can prove it created is cleared.
    pub lock_existed_before: bool,
    #[serde(default)]
    pub spawning: bool,
    /// Every lock `.git` held before the child, by the same rule for each kind: what is not named here and is
    /// there once the child's group is gone, the child made.
    #[serde(default)]
    pub locks_before: Vec<String>,
}

/// The file `<agent-logs>/v2/tree/<workspace_id>.json`.
#[derive(Clone, Debug)]
pub struct TreeLedger {
    path: PathBuf,
}

impl TreeLedger {
    pub fn new(agent_logs: &Path, workspace_id: &str) -> Self {
        Self {
            path: agent_logs
                .join("v2")
                .join("tree")
                .join(format!("{workspace_id}.json")),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `None` for a file that is gone, and for one that does not read: a torn entry names nobody to kill.
    pub fn read(&self) -> Option<LedgerEntry> {
        let bytes = fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// By temp file and rename, so a reader never meets half an entry.
    pub fn write(&self, entry: &LedgerEntry) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, serde_json::to_vec(entry)?)?;
        fs::rename(&temp, &self.path)
    }

    pub fn delete(&self) {
        // A file already gone is the wanted state.
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.child_record());
    }

    /// Where a child records its own pid as its first act.
    pub fn child_record(&self) -> PathBuf {
        self.path.with_extension("child")
    }

    /// The pid a child recorded for itself. Torn or absent reads as none.
    pub(crate) fn self_recorded_pid(&self) -> Option<u32> {
        fs::read_to_string(self.child_record())
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Whether the entry is young enough that a spawn it announces may still be starting.
    pub(crate) fn within_spawn_grace(&self) -> bool {
        fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age < SPAWN_GRACE)
    }

    /// Deletes the entry only if `owner_id` wrote it: a worker that lost its run finishes after the new
    /// owner's worker has written, and that entry is not its to remove.
    pub(crate) fn delete_own(&self, owner_id: &str) {
        if self.read().is_some_and(|entry| entry.owner_id == owner_id) {
            self.delete();
        }
    }

    /// `tree.in_flight` of status: the entry with its pid checked, a dead child reading as none.
    pub fn in_flight(&self, guard: &dyn ProcessGuard) -> Option<InFlightStatus> {
        let entry = self.read()?;
        let (pid, start_time) = (entry.pid?, entry.start_time?);
        guard.is_alive(pid, &start_time).then_some(InFlightStatus {
            op: entry.op,
            step: entry.step,
            since: entry.began_at,
        })
    }
}

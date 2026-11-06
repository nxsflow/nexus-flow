//! The once-a-minute sample (§6.5), as tv58's throwaway run measured it on macOS and Linux with the real CLI:
//!
//! - The session's own process group holds the leader, the CLI and every stdio MCP server, and nothing else. They all
//!   spend CPU for ever, a waiting CLI about 2 s a minute, so they are left out, and none of them can hide a hang.
//! - A Bash tool's shell is a child of the CLI in a session and a group of its own. Its children may take further
//!   groups, and a child that reparents to `init` keeps the session. So the work is everything below the leader that is
//!   outside its group, everything in a session one of those leads, and whatever an earlier sample saw that still lives,
//!   which is how a worker that left both (`setsid`, then its parent ended) is followed.
//! - `sleep 60; gh ...` spends less than a tick on Linux, so a sample compares WHICH processes run as well as their CPU.
//!
//! What this cannot see: a process that left session and group AND lost its parent between two samples.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::procs::{ProcIdentity, ProcInfo, START_SLACK_MS};

/// What a sample leaves in the ledger, so the pids and the comparison survive a restart of the reader.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRecord {
    /// When it was taken, in unix milliseconds. Zero for none yet.
    pub at_ms: i64,
    /// What the work was at that sample, by pid: its start, and its CPU in the platform's raw unit.
    pub work: BTreeMap<u32, (i64, u64)>,
    /// When a comparison last found movement.
    pub moved_at_ms: Option<i64>,
    /// Whether `blind` has been said.
    pub blind_said: bool,
    /// The pids `group_lingering` last named, so one set is said once.
    #[serde(default)]
    pub lingering_said: Vec<u32>,
}

impl SampleRecord {
    /// What an earlier sample recorded, as identities.
    pub fn recorded(&self) -> Vec<ProcIdentity> {
        self.work
            .iter()
            .map(|(pid, (start_ms, _))| ProcIdentity {
                pid: *pid,
                start_ms: *start_ms,
            })
            .collect()
    }
}

/// The session's work among all processes, with their CPU. `leader_pgid` is the leader's pid: it was spawned as the
/// leader of its group. Also reads below the leader's GROUP, because a leader that died leaves its CLI reparented.
pub fn work_of(
    all: &[ProcInfo],
    leader_pgid: u32,
    leader_start_ms: i64,
    recorded: &[ProcIdentity],
) -> BTreeMap<u32, (i64, u64)> {
    let born_with_or_after = |p: &ProcInfo| p.start_ms + START_SLACK_MS >= leader_start_ms;
    let by_pid: BTreeMap<u32, &ProcInfo> = all
        .iter()
        .filter(|p| !p.zombie)
        .map(|p| (p.pid, p))
        .collect();
    let mut children: BTreeMap<u32, Vec<&ProcInfo>> = BTreeMap::new();
    for p in by_pid.values() {
        children.entry(p.ppid).or_default().push(p);
    }

    let mut work: BTreeSet<u32> = BTreeSet::new();
    // A process of another start at the leader's pid holds the group id: nothing below it, or in it, is this session's.
    let foreign_leader = by_pid
        .get(&leader_pgid)
        .is_some_and(|p| p.start_ms.abs_diff(leader_start_ms) > START_SLACK_MS.unsigned_abs());
    let mut walk: Vec<u32> = if foreign_leader {
        Vec::new()
    } else {
        by_pid
            .values()
            .filter(|p| p.pgid == leader_pgid && born_with_or_after(p))
            .map(|p| p.pid)
            .chain(std::iter::once(leader_pgid))
            .collect()
    };
    let mut seen: BTreeSet<u32> = walk.iter().copied().collect();
    while let Some(parent) = walk.pop() {
        for child in children.get(&parent).into_iter().flatten() {
            if !born_with_or_after(child) || !seen.insert(child.pid) {
                continue;
            }
            // Every member of the group is a root of this walk, so what is found here is outside it.
            walk.push(child.pid);
            work.insert(child.pid);
        }
    }
    for identity in recorded {
        let same = by_pid.get(&identity.pid).is_some_and(|p| {
            p.start_ms.abs_diff(identity.start_ms) <= START_SLACK_MS.unsigned_abs()
        });
        if same {
            work.insert(identity.pid);
        }
    }
    // Everything in a session one of them leads: a worker reparented to `init` stays in it. Never the session the
    // leader itself was started in, which is the service's and everybody's.
    let sessions: BTreeSet<u32> = work
        .iter()
        .filter_map(|pid| by_pid.get(pid))
        .filter(|p| p.sid == p.pid)
        .map(|p| p.sid)
        .collect();
    for p in by_pid.values() {
        if sessions.contains(&p.sid) && p.sid != 0 && born_with_or_after(p) && p.pgid != leader_pgid
        {
            work.insert(p.pid);
        }
    }
    work.into_iter()
        .filter_map(|pid| by_pid.get(&pid))
        .map(|p| (p.pid, (p.start_ms, p.cpu)))
        .collect()
}

/// Whether anything moved between two samples: the CPU of a process that stayed, or which processes run. A first sample
/// compares with nothing, and work that has just appeared is movement.
pub fn moved(before: &BTreeMap<u32, (i64, u64)>, now: &BTreeMap<u32, (i64, u64)>) -> bool {
    before != now
}

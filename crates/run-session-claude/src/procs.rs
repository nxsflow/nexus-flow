//! The identity guard (§8.3): a process is a pid AND the instant it started, re-read immediately before every signal,
//! so a pid the system handed to somebody else is never signalled. Everything else that needs to know about processes
//! asks here, because `libc` is on this crate's row of the layering gate and on no other.

use nexus_run_contracts::Timestamp;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
#[path = "procs/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "procs/macos.rs"]
mod platform;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "procs/other.rs"]
mod platform;

/// How far two readings of one process's start may lie apart. A child can only estimate its own start (its clock read
/// comes after the exec, which on a machine at load 74 is seconds), Linux derives it from a boot instant that NTP can
/// move, and a pid is not reused within this window on any system with a pid space worth the name.
pub const START_SLACK_MS: i64 = 10_000;

/// What the system says of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    /// The session it is in, which a Bash tool's shell leads (measured on both platforms, tv58 step 0).
    pub sid: u32,
    pub start_ms: i64,
    /// CPU time in the platform's raw unit: compared for change, never converted.
    pub cpu: u64,
    pub zombie: bool,
}

/// Every process the system lists, or `None` where this host cannot (`blind`). A process that vanished between the
/// listing and its read is skipped.
pub fn list() -> Option<Vec<ProcInfo>> {
    platform::list()
}

/// One process, zombie or not.
pub fn info(pid: u32) -> Option<ProcInfo> {
    platform::info(pid)
}

/// A process by its identity. The same type in `child.json` and in the sampler's record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcIdentity {
    pub pid: u32,
    pub start_ms: i64,
}

fn close_enough(a: i64, b: i64) -> bool {
    a.abs_diff(b) <= START_SLACK_MS.unsigned_abs()
}

/// Whether this identity is a live process right now. A zombie is not.
pub fn verify(identity: &ProcIdentity) -> bool {
    info(identity.pid).is_some_and(|p| !p.zombie && close_enough(p.start_ms, identity.start_ms))
}

pub fn alive(pid: u32) -> bool {
    info(pid).is_some_and(|p| !p.zombie)
}

/// Reaps our own dead child, if this identity is one: a long-lived service has no `init` to do it, and an unreaped
/// leader answers `kill(pid, 0)` alive and `kill(-pgid, 0)` `EPERM` on Darwin and success on Linux (measured,
/// tv58 step 0). A child adopted after a restart is `init`'s, and this does nothing for it.
///
/// Darwin answers nothing at all about a zombie (`proc_pidinfo` fails, measured), so there the start cannot be compared
/// and only "there is no live process at this pid" can: what `waitpid` then takes is a dead child of ours at that pid.
pub fn reap(identity: &ProcIdentity) {
    let ours = match info(identity.pid) {
        Some(p) => p.zombie && close_enough(p.start_ms, identity.start_ms),
        None => true,
    };
    if ours && identity.pid > 1 {
        platform::reap(identity.pid);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Term,
    Kill,
    Usr2,
}

/// The group of a verified leader. `false` when nothing was sent: an unverified leader or a pgid of 0 or 1, for which
/// `kill(2)` means this process's group or everybody.
pub fn signal_group(leader: &ProcIdentity, signal: Signal) -> bool {
    leader.pid > 1 && verify(leader) && platform::kill(-(leader.pid as i32), signal)
}

/// One verified process. `false` when nothing was sent.
pub fn signal_member(member: &ProcIdentity, signal: Signal) -> bool {
    member.pid > 1 && verify(member) && platform::kill(member.pid as i32, signal)
}

/// What is left of a process group whose leader is dead, asked at decision time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupState {
    /// `kill(-pgid, 0)` answers `ESRCH`.
    Empty,
    /// The id is held by a group of somebody else's: a process of another start stands at the leader's place, or every
    /// member started before the leader did. Nothing of this session is there and nothing of it may be signalled.
    Foreign,
    /// Members of this session, with their identities.
    Members(Vec<ProcIdentity>),
    /// The system says something is there and cannot say what: an unreaped member (`EPERM`), or a host that cannot
    /// list. Not confirmed, and nothing to signal: the named rung of §6.5.
    Unverifiable,
}

pub fn group_state(pgid: u32, leader_start_ms: i64) -> GroupState {
    if pgid <= 1 || platform::group_is_gone(pgid) {
        return GroupState::Empty;
    }
    if info(pgid).is_some_and(|p| !p.zombie && !close_enough(p.start_ms, leader_start_ms)) {
        return GroupState::Foreign;
    }
    let Some(all) = list() else {
        return GroupState::Unverifiable;
    };
    let live: Vec<&ProcInfo> = all.iter().filter(|p| p.pgid == pgid && !p.zombie).collect();
    let ours: Vec<ProcIdentity> = live
        .iter()
        .filter(|p| p.start_ms + START_SLACK_MS >= leader_start_ms)
        .map(|p| ProcIdentity {
            pid: p.pid,
            start_ms: p.start_ms,
        })
        .collect();
    if ours.is_empty() {
        if live.is_empty() {
            GroupState::Unverifiable
        } else {
            GroupState::Foreign
        }
    } else {
        GroupState::Members(ours)
    }
}

/// The form a start time travels in where only a string fits: the host's tree ledger.
pub fn start_text(start_ms: i64) -> String {
    Timestamp::from_unix_millis(start_ms).to_string()
}

fn start_of_text(text: &str) -> Option<i64> {
    Timestamp::parse(text).map(Timestamp::unix_millis)
}

/// Signals a process only under its verified identity, never a pgid <= 1. A trait, so the host's tests can stage a
/// guard that cannot read or cannot end something.
pub trait ProcessGuard: Send + Sync {
    /// The start time of this pid, or `None` when it is gone. A zombie is gone.
    fn start_time(&self, pid: u32) -> Option<String>;

    fn is_alive(&self, pid: u32, start_time: &str) -> bool {
        match (self.start_time(pid), start_of_text(start_time)) {
            (Some(read), Some(recorded)) => {
                start_of_text(&read).is_some_and(|read| close_enough(read, recorded))
            }
            _ => false,
        }
    }

    /// SIGKILL of the group, re-verifying the leader immediately before. `false` when nothing was sent.
    fn kill_group(&self, pid: u32, start_time: &str) -> bool;

    /// Whether no live process is left in this group. A zombie holds nothing.
    fn group_gone(&self, pgid: u32) -> bool;

    /// SIGKILL of what is left of a group whose leader has ended and been reaped, member by member: only those that
    /// started with it or after, under their own identities. `false` when nothing was sent.
    fn kill_members(&self, _pgid: u32, _leader_start: &str) -> bool {
        false
    }
}

/// [`ProcessGuard`] over the system.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProcesses;

impl ProcessGuard for SystemProcesses {
    fn start_time(&self, pid: u32) -> Option<String> {
        info(pid)
            .filter(|p| !p.zombie)
            .map(|p| start_text(p.start_ms))
    }

    fn kill_group(&self, pid: u32, start_time: &str) -> bool {
        start_of_text(start_time)
            .is_some_and(|start_ms| signal_group(&ProcIdentity { pid, start_ms }, Signal::Kill))
    }

    fn group_gone(&self, pgid: u32) -> bool {
        list().is_some_and(|all| !all.iter().any(|p| p.pgid == pgid && !p.zombie))
    }

    fn kill_members(&self, pgid: u32, leader_start: &str) -> bool {
        let Some(start_ms) = start_of_text(leader_start) else {
            return false;
        };
        let GroupState::Members(members) = group_state(pgid, start_ms) else {
            return false;
        };
        // Every member, whatever the one before answered: no short circuit.
        let mut sent = false;
        for member in &members {
            sent |= signal_member(member, Signal::Kill);
        }
        sent
    }
}

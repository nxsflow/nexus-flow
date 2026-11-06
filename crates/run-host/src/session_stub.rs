//! What `nexus-run-session-claude` brings with ticket tv58, standing behind the same seam until then: the
//! real backend behind the contract's [`SessionBackend`] trait, and the identity guard every signal goes
//! through (contract (c), §8.3).
//!
//! The guard here asks `ps` and signals through `kill`, because `libc` is on the adapter's list in the
//! layering gate and not on the host's. Both run off the tick, or on the rare `begin` that finds a recorded
//! child. The adapter's `procs` module replaces this file.

use std::process::{Command, Stdio};

use nexus_run_contracts::{
    BackendError, Capabilities, DrainedFact, Liveness, SessionBackend, StartOutcome, StartSpec,
};

/// Signals a process only under its verified identity (pid and start time), never a pgid <= 1.
pub trait ProcessGuard: Send + Sync {
    /// The start time of this pid, or `None` when it is gone. A zombie is gone.
    fn start_time(&self, pid: u32) -> Option<String>;

    fn is_alive(&self, pid: u32, start_time: &str) -> bool {
        self.start_time(pid).as_deref() == Some(start_time)
    }

    /// SIGKILL of the group, re-verifying the leader immediately before. `false` when nothing was sent.
    fn kill_group(&self, pid: u32, start_time: &str) -> bool;
}

/// [`ProcessGuard`] over `ps` and `kill`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProcesses;

impl ProcessGuard for SystemProcesses {
    fn start_time(&self, pid: u32) -> Option<String> {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-o", "lstart=", "-p", &pid.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let (stat, start) = text.trim().split_once(char::is_whitespace)?;
        (!stat.starts_with('Z')).then(|| start.trim().to_owned())
    }

    fn kill_group(&self, pid: u32, start_time: &str) -> bool {
        if pid <= 1 || !self.is_alive(pid, start_time) {
            return false;
        }
        Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// The real backend's place until ticket tv58: it refuses, so a start is a `started`-less slot that
/// `start_deadline` parks, which is what §13 says of a spawn failure.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBackend;

fn refused<T>() -> Result<T, BackendError> {
    Err(BackendError {
        what: "the Claude adapter arrives with ticket tv58".to_owned(),
    })
}

impl SessionBackend for NoBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: "none".to_owned(),
            capabilities: Vec::new(),
        }
    }

    fn start(&self, _spec: &StartSpec) -> Result<StartOutcome, BackendError> {
        refused()
    }

    fn nudge(&self, _key: &str, _text: &str) -> Result<(), BackendError> {
        refused()
    }

    fn stop(&self, _key: &str, _force: bool) -> Result<(), BackendError> {
        refused()
    }

    fn alive(&self, _key: &str) -> Result<Liveness, BackendError> {
        Ok(Liveness::Dead)
    }

    fn drain_facts(
        &self,
        _key: &str,
        from_offset: u64,
    ) -> Result<(Vec<DrainedFact>, u64), BackendError> {
        Ok((Vec::new(), from_offset))
    }
}

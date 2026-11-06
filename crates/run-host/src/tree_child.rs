//! What every tree child needs around it: its output collected with a bound on the bytes and a bound on the wait,
//! the gate it starts behind, and the rule for the locks a killed git leaves.
//!
//! A helper git started can hold a pipe open long after git itself is gone, so nothing here waits for an end of
//! file without a limit. A reader that never sees one is left behind, holding at most its cap, until the helper
//! lets go; the answer has by then gone out as a refusal, never as an empty success.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::session_stub::ProcessGuard;
use crate::tree_worker::StepOutput;

/// What one stream of one child may hold in memory. What is past it is read and dropped, so the child never
/// blocks on a full pipe. Every call the tree parses is far below this, and a dirty status is dirty at any length.
pub(crate) const OUTPUT_CAP: usize = 8 * 1024 * 1024;

/// How long the output is waited on once the child has ended. Real time, because a pipe's end is.
pub(crate) const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How long a killed group is given to be gone before its locks are left to whoever comes next. Real time too.
const GROUP_GONE_WAIT: Duration = Duration::from_millis(500);

const POLL: Duration = Duration::from_millis(2);

#[derive(Default)]
struct Stream {
    bytes: Mutex<Vec<u8>>,
    ended: AtomicBool,
}

fn read_capped<R: Read + Send + 'static>(source: Option<R>) -> Arc<Stream> {
    let stream = Arc::new(Stream::default());
    let filling = Arc::clone(&stream);
    std::thread::spawn(move || {
        if let Some(mut source) = source {
            let mut chunk = [0_u8; 64 * 1024];
            // What arrived before a read error is still what the child said.
            while let Ok(read @ 1..) = source.read(&mut chunk) {
                let mut bytes = filling.bytes.lock().unwrap_or_else(PoisonError::into_inner);
                let room = OUTPUT_CAP.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..read.min(room)]);
            }
        }
        filling.ended.store(true, Ordering::SeqCst);
    });
    stream
}

/// The two streams of one child, being read.
pub(crate) struct Output {
    stdout: Arc<Stream>,
    stderr: Arc<Stream>,
}

impl Output {
    pub(crate) fn collect(child: &mut Child) -> Self {
        Self::from_sources(child.stdout.take(), child.stderr.take())
    }

    pub(crate) fn from_sources<A, B>(stdout: Option<A>, stderr: Option<B>) -> Self
    where
        A: Read + Send + 'static,
        B: Read + Send + 'static,
    {
        Self {
            stdout: read_capped(stdout),
            stderr: read_capped(stderr),
        }
    }

    fn ended(&self) -> bool {
        self.stdout.ended.load(Ordering::SeqCst) && self.stderr.ended.load(Ordering::SeqCst)
    }

    /// Whether both streams reached their end within `grace`.
    pub(crate) fn wait_for_end(&self, grace: Duration) -> bool {
        let until = Instant::now() + grace;
        while !self.ended() {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(POLL);
        }
        true
    }

    pub(crate) fn into_step(self, success: bool) -> StepOutput {
        let take = |stream: &Stream| {
            std::mem::take(&mut *stream.bytes.lock().unwrap_or_else(PoisonError::into_inner))
        };
        StepOutput {
            success,
            stdout: take(&self.stdout),
            stderr: take(&self.stderr),
        }
    }
}

/// The same command behind the gate, writing its own pid first. `exec` keeps the pid, the process group and the
/// start time, so what it wrote is the identity the ledger means. It then waits on stdin for the line the worker
/// writes once the ledger names it, and on an end of file with no line it exits without ever running the program:
/// a worker that dies in that window leaves nothing that can touch the tree (§6.6, decided at the cut). The worker
/// closes the pipe after that line, so the program reads an empty stdin. No `pre_exec`: this works on a threaded
/// process.
#[cfg(unix)]
pub(crate) fn gated(command: Command, record: &Path) -> Command {
    let mut wrapped = Command::new("sh");
    wrapped
        .arg("-c")
        .arg(r#"printf %s "$$" > "$0"; IFS= read -r go || exit 97; exec "$@""#)
        .arg(record)
        .arg(command.get_program())
        .args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        wrapped.current_dir(dir);
    }
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => wrapped.env(name, value),
            None => wrapped.env_remove(name),
        };
    }
    wrapped
}

#[cfg(not(unix))]
pub(crate) fn gated(command: Command, _record: &Path) -> Command {
    command
}

#[cfg(unix)]
pub(crate) fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn own_process_group(_command: &mut Command) {}

/// Every lock a killed git can leave in `.git`: its own directory and the ref tree, named relative to `.git`.
pub(crate) fn git_locks(dir: &Path) -> Vec<String> {
    let git_dir = dir.join(".git");
    let mut found = Vec::new();
    collect_locks(&git_dir, "", false, &mut found);
    collect_locks(&git_dir.join("refs"), "refs/", true, &mut found);
    found.sort();
    found
}

fn collect_locks(dir: &Path, prefix: &str, descend: bool, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_dir && descend {
            collect_locks(&entry.path(), &format!("{prefix}{name}/"), true, found);
        } else if !is_dir && name.ends_with(".lock") {
            found.push(format!("{prefix}{name}"));
        }
    }
}

/// The rule for the locks of a dead child, every kind alike (§5 names only the index's): a lock is cleared when
/// it was not there before the child and the child's group is confirmed gone, and only then. Whatever else holds a
/// lock is `lock_held`, named, for a person.
pub(crate) fn clear_locks_of_a_gone_group(dir: &Path, before: &[String], index_before: bool) {
    let git_dir = dir.join(".git");
    for lock in git_locks(dir) {
        let kept = before.contains(&lock) || (index_before && lock == "index.lock");
        if !kept {
            // A lock already gone is the wanted state.
            let _ = fs::remove_file(git_dir.join(&lock));
        }
    }
}

/// Whether the group emptied within a short wait. A group SIGKILL has reached is gone in a moment.
pub(crate) fn group_gone_soon(guard: &dyn ProcessGuard, pgid: u32) -> bool {
    let until = Instant::now() + GROUP_GONE_WAIT;
    while !guard.group_gone(pgid) {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}

/// One child with nobody's ledger behind it: the status read of the CLI process. In its own group, killed by group
/// at the bound, with the same collection rule.
pub(crate) fn run_bounded(
    mut command: Command,
    bound: Duration,
    guard: &dyn ProcessGuard,
) -> Result<StepOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("git did not start: {error}"))?;
    let start_time = guard.start_time(child.id());
    let output = Output::collect(&mut child);
    let until = Instant::now() + bound;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if output.wait_for_end(DRAIN_GRACE) {
                    Ok(output.into_step(status.success()))
                } else {
                    Err("a helper still holds git's output".to_owned())
                };
            }
            Ok(None) if Instant::now() < until => std::thread::sleep(POLL),
            Ok(None) | Err(_) => {
                if let Some(start_time) = &start_time {
                    guard.kill_group(child.id(), start_time);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git did not answer within {bound:?}"));
            }
        }
    }
}

#[cfg(test)]
mod tests;

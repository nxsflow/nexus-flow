use std::io::Read;
use std::time::Duration;

use super::{git_locks, Output, OUTPUT_CAP};

#[test]
fn a_child_pouring_output_is_held_to_the_cap_and_read_to_its_end() {
    let flood = std::io::repeat(b'x').take(OUTPUT_CAP as u64 * 2 + 17);
    let output = Output::from_sources(Some(flood), Some(std::io::empty()));
    assert!(
        output.wait_for_end(Duration::from_secs(60)),
        "read to its end"
    );
    let step = output.into_step(true);
    assert_eq!(step.stdout.len(), OUTPUT_CAP);
    assert_eq!(step.stderr.len(), 0);
}

/// A stream that never ends, as a pipe a helper still holds.
struct Held;

impl Read for Held {
    fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_secs(3_600));
        Ok(0)
    }
}

#[test]
fn a_stream_that_never_ends_answers_no_within_the_grace() {
    let output = Output::from_sources(Some(Held), Some(std::io::empty()));
    assert!(!output.wait_for_end(Duration::from_millis(20)));
}

#[test]
fn locks_are_named_in_git_and_in_the_ref_tree_and_nowhere_else() {
    let temp = tempfile::tempdir().unwrap();
    let git = temp.path().join(".git");
    std::fs::create_dir_all(git.join("refs/heads/feat")).unwrap();
    std::fs::create_dir_all(git.join("objects")).unwrap();
    for lock in [
        "index.lock",
        "HEAD.lock",
        "config.lock",
        "packed-refs.lock",
        "refs/heads/feat/x.lock",
        "objects/not-a-ref.lock",
    ] {
        std::fs::write(git.join(lock), b"").unwrap();
    }
    assert_eq!(
        git_locks(temp.path()),
        [
            "HEAD.lock",
            "config.lock",
            "index.lock",
            "packed-refs.lock",
            "refs/heads/feat/x.lock"
        ]
    );
}

/// git can be gone before its start is read - a reader preempted for the few milliseconds a `sleep & exit` takes.
/// The helper it left holding the pipe is still ended: the refusal never leaves a helper behind (6j6v.zame).
#[cfg(unix)]
#[test]
fn a_helper_is_ended_though_git_was_gone_before_its_start_was_read() {
    use super::run_bounded;
    use nexus_run_session_claude::procs::{self, ProcessGuard, SystemProcesses};
    use std::process::Command;

    /// The system, read late: the reader comes back only after git has ended.
    struct Late;
    impl ProcessGuard for Late {
        fn start_time(&self, pid: u32) -> Option<String> {
            let until = std::time::Instant::now() + Duration::from_secs(10);
            while procs::alive(pid) && std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(2));
            }
            SystemProcesses.start_time(pid)
        }
        fn kill_group(&self, pid: u32, start_time: &str) -> bool {
            SystemProcesses.kill_group(pid, start_time)
        }
        fn group_gone(&self, pgid: u32) -> bool {
            SystemProcesses.group_gone(pgid)
        }
        fn kill_members(&self, pgid: u32, leader_start: &str) -> bool {
            SystemProcesses.kill_members(pgid, leader_start)
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("helper-pid");
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(r#"sleep 600 & echo $! > "$0"; exit 0"#)
        .arg(&pid_file);

    let read = run_bounded(command, Duration::from_secs(10), &Late);

    let refused = read.expect_err("a read through a helper holding its pipe answered");
    assert_eq!(refused, "a helper still holds git's output");
    let helper: u32 = std::fs::read_to_string(&pid_file)
        .expect("the shell wrote its helper's pid")
        .trim()
        .parse()
        .unwrap();
    // A SIGKILL sent is not yet a process gone: on Linux a look at once finds the helper running in nearly every round
    // (measured, 6j6v.zame). A helper nobody signalled sleeps 600 s, far past this wait.
    let gone_by = std::time::Instant::now() + Duration::from_secs(5);
    while procs::alive(helper) && std::time::Instant::now() < gone_by {
        std::thread::sleep(Duration::from_millis(10));
    }
    let alive = procs::alive(helper);
    let _ = Command::new("kill")
        .args(["-KILL", &helper.to_string()])
        .status();
    assert!(!alive, "the helper stayed behind the read");
}

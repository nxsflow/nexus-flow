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

/// PROBE (6j6v.zame, temporary): when a status read refuses over a helper and the helper is still alive, which is it -
/// the leader's start never read (gone before `start_time`, so `kill_members` is skipped), or a SIGKILL sent and not
/// yet landed when aliveness is checked?
#[cfg(target_os = "linux")]
#[test]
fn probe_why_a_helper_outlives_the_status_read() {
    use nexus_run_session_claude::procs::{self, ProcessGuard, SystemProcesses};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::process::Command;
    use std::sync::Mutex;
    use super::run_bounded;

    struct Recording {
        start_none: AtomicUsize,
        sent: AtomicUsize,
    }
    impl ProcessGuard for Recording {
        fn start_time(&self, pid: u32) -> Option<String> {
            let read = SystemProcesses.start_time(pid);
            if read.is_none() {
                self.start_none.fetch_add(1, Ordering::SeqCst);
            }
            read
        }
        fn kill_group(&self, pid: u32, start_time: &str) -> bool {
            SystemProcesses.kill_group(pid, start_time)
        }
        fn group_gone(&self, pgid: u32) -> bool {
            SystemProcesses.group_gone(pgid)
        }
        fn kill_members(&self, pgid: u32, leader_start: &str) -> bool {
            let sent = SystemProcesses.kill_members(pgid, leader_start);
            if sent {
                self.sent.fetch_add(1, Ordering::SeqCst);
            }
            sent
        }
    }

    let temp = Arc::new(tempfile::tempdir().unwrap());
    let tally = Arc::new(Mutex::new([0_usize; 6]));
    let handles: Vec<_> = (0..8)
        .map(|thread| {
            let temp = Arc::clone(&temp);
            let tally = Arc::clone(&tally);
            std::thread::spawn(move || {
                for round in 0..40 {
                    let pid_file = temp.path().join(format!("p-{thread}-{round}"));
                    let guard = Recording {
                        start_none: AtomicUsize::new(0),
                        sent: AtomicUsize::new(0),
                    };
                    let mut command = Command::new("sh");
                    command
                        .arg("-c")
                        .arg(r#"sleep 600 & echo $! > "$0"; exit 0"#)
                        .arg(&pid_file);
                    let read = run_bounded(command, Duration::from_secs(10), &guard);
                    let helper: u32 = std::fs::read_to_string(&pid_file)
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    let at_once = procs::alive(helper);
                    std::thread::sleep(Duration::from_millis(300));
                    let later = procs::alive(helper);
                    let _ = Command::new("kill")
                        .args(["-KILL", &helper.to_string()])
                        .status();
                    let mut tally = tally.lock().unwrap();
                    tally[0] += 1;
                    tally[1] += usize::from(read.is_err());
                    tally[2] += guard.start_none.load(Ordering::SeqCst);
                    tally[3] += guard.sent.load(Ordering::SeqCst);
                    tally[4] += usize::from(at_once);
                    tally[5] += usize::from(later);
                    if at_once || later {
                        eprintln!(
                            "PROBE survivor: start_none={} sent={} at_once={at_once} later={later}",
                            guard.start_none.load(Ordering::SeqCst),
                            guard.sent.load(Ordering::SeqCst)
                        );
                    }
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let [rounds, refused, start_none, sent, at_once, later] = *tally.lock().unwrap();
    eprintln!(
        "PROBE rounds={rounds} refused={refused} start_none={start_none} kill_sent={sent} alive_at_once={at_once} alive_after_300ms={later}"
    );
    assert_eq!((at_once, later), (0, 0), "a helper outlived the status read");
}

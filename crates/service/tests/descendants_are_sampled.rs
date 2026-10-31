//! The subprocess sample (nxf 6j6v.pszd cut 2; design 8nx4 section 1 (b), test 3).
//!
//! Against real processes, because what this guards is the FFI and `/proc` reading underneath: a
//! `proc_listchildpids` that listed nothing, or a counter read from the wrong field, would pass any
//! test against a stub — and an agent whose `gh run watch` is the only thing alive would be killed
//! as stalled.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use nxs_service::process::descendants_started_since;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// A shell whose one child loops a little and sleeps a second: a few ms of CPU per second.
const BUSY: &str =
    "sh -c 'while :; do i=0; while [ $i -lt 2000 ]; do i=$((i+1)); done; sleep 1; done' & wait";

/// A `sh -c <script>` in a process group of its own, killed WHOLE on drop. Killing only the outer
/// shell would orphan its looping child — which then runs forever and holds the test harness's
/// output pipe open — and a failed assertion must not leak it either.
struct Tree(Child);

impl Tree {
    fn spawn(script: &str) -> Tree {
        Tree(
            Command::new("sh")
                .args(["-c", script])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap(),
        )
    }

    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: `kill` touches no caller memory; the negative pid names this child's own group,
        // which `process_group(0)` made, never the harness's.
        unsafe { libc::kill(-(self.0.id() as libc::pid_t), libc::SIGKILL) };
        let _ = self.0.wait();
    }
}

/// Polls until the sample moves or the bound passes; returns whether it moved.
fn moves_within(pid: u32, since: SystemTime, bound: Duration) -> bool {
    let first = descendants_started_since(pid, since).expect("readable");
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        let now = descendants_started_since(pid, since).expect("readable");
        if now != first {
            return true;
        }
    }
    false
}

#[test]
fn a_child_that_burns_cpu_moves_the_sample_and_a_sleeping_child_does_not() {
    let since = SystemTime::now() - Duration::from_secs(1);
    let busy = Tree::spawn(BUSY);
    let idle = Tree::spawn("sleep 60 & wait");
    std::thread::sleep(Duration::from_millis(300));
    let s = descendants_started_since(busy.id(), since).unwrap();
    assert!(s.count >= 1, "{s:?}");
    let idle_sample = descendants_started_since(idle.id(), since).unwrap();
    assert!(
        idle_sample.count >= 1,
        "the sleeping child is listed: {idle_sample:?}"
    );
    assert!(
        moves_within(busy.id(), since, Duration::from_secs(10)),
        "the busy child's counter moved"
    );
    assert!(
        !moves_within(idle.id(), since, Duration::from_secs(3)),
        "a sleeping child does not move"
    );
}

#[test]
fn a_child_started_before_since_is_not_in_the_sample() {
    let parent = Tree::spawn(BUSY);
    std::thread::sleep(Duration::from_millis(300));
    // The ticking child IS there: with an early `since` it is counted.
    let early = SystemTime::now() - Duration::from_secs(1);
    let counted = descendants_started_since(parent.id(), early).unwrap();
    assert!(counted.count >= 1, "{counted:?}");
    // `since` well after the child's start, past the slack: the ticking child is excluded.
    let since = SystemTime::now() + Duration::from_secs(120);
    let s = descendants_started_since(parent.id(), since).unwrap();
    assert_eq!(s.count, 0, "{s:?}");
    assert!(!moves_within(parent.id(), since, Duration::from_secs(3)));
}

#[test]
fn an_unreadable_pid_is_none() {
    assert_eq!(
        descendants_started_since(u32::MAX - 7, SystemTime::now()),
        None
    );
}

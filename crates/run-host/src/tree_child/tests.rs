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

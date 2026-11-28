//! stdin (RFC 0001, "Reading and writing"): only a command that reads stdin, such as
//! `nxs ticket create -`, touches it. Another command never waits on it, even while another run
//! in the same process holds it, as `nxs sync --watch` in a test's thread does for as long as it
//! runs.

mod common;

use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use common::{nxs, scratch};

#[test]
fn a_command_that_reads_no_stdin_never_waits_on_it() {
    let base = scratch("stdin-held");
    let (held, holding) = mpsc::channel();
    thread::spawn(move || {
        let _lock = std::io::stdin().lock();
        held.send(()).unwrap();
        thread::park();
    });
    holding.recv().unwrap();

    let (done, finished) = mpsc::channel();
    let run = base.clone();
    thread::spawn(move || {
        let out = nxs(&run, &run.join("repo"), &["init"]);
        done.send(out.code).unwrap();
    });

    assert_eq!(finished.recv_timeout(Duration::from_secs(30)), Ok(0));
    fs::remove_dir_all(&base).unwrap();
}

//! **The pulse is read on the path the `nxc` binary takes** (nxf 6j6v.pszd cut 1, design 8nx4 test
//! 7).
//!
//! The binary's worker is `cli.rs`'s `LazyWorker`, and it holds no worker of its own: every call
//! resolves one through [`nexus_chat::worker::select_worker`] — the environment's `NXC_WORKER` and
//! `NXC_SIDECAR`, as the binary reads them — and forwards. No verb reads the pulse yet (the ladder
//! does, in cut 3), so this file takes that same selection, from the same variables, and asks the
//! worker it yields about a pulse file the test wrote where the sidecar writes it. That the wrapper
//! FORWARDS both methods rather than resting on the trait's defaults is the gate
//! `every_worker_answers_for_itself.rs`, which reds when either forwarder is removed.
//!
//! One test, alone in its binary, so the two variables it sets are nobody else's.

#![cfg(unix)]

use nexus_chat::worker::{agent_logs_dir, select_worker, Pulse};

#[test]
fn the_cli_worker_reads_a_written_pulse_and_withdraws_the_asks() {
    let tmp = tempfile::tempdir().unwrap();
    // What the binary reads to pick its worker; the sidecar path is never started here.
    std::env::set_var("NXC_WORKER", "sidecar");
    std::env::set_var("NXC_SIDECAR", tmp.path().join("main.mjs"));
    let worker = select_worker(tmp.path().to_path_buf()).expect("the sidecar worker is selected");

    let logs = agent_logs_dir(tmp.path());
    std::fs::create_dir_all(&logs).unwrap();
    assert_eq!(worker.session_pulse("s-1"), Pulse::Unreported);

    std::fs::write(logs.join("s-1.alive"), "").unwrap();
    assert_eq!(worker.session_pulse("s-1"), Pulse::NoneYet);

    std::fs::write(logs.join("s-1.alive"), "2026-10-30T09:25:22.418Z\n").unwrap();
    assert_eq!(
        worker.session_pulse("s-1"),
        Pulse::At("2026-10-30T09:25:22.418Z".to_string()),
        "the instant the sidecar wrote, verbatim"
    );

    std::fs::write(logs.join("s-1.ask.01jask.json"), "{}").unwrap();
    worker.withdraw_asks("s-1").unwrap();
    assert!(
        !logs.join("s-1.ask.01jask.json").exists(),
        "the ask is withdrawn"
    );
    assert!(
        logs.join("s-1.alive").exists(),
        "and nothing else is touched"
    );
}

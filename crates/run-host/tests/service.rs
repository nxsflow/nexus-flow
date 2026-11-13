//! The host the background service runs (6j6v.pn9c; §8.1, §10): built over a workspace's `run.sqlite` with the
//! Claude adapter, the real `GitTree` and the machine's clock, and asked by the service whether any session of
//! an open run may still have a process.

use nexus_run_engine::SlotState;
use nexus_run_host::bridge::{self, HOST_BUSY_TIMEOUT};
use nexus_run_host::service;
use nexus_run_host::test_support::scripted_backend::{pass, Cue, SessionInfo};
use nexus_run_host::test_support::{store_path, Rig, CLOCK_START};
use nexus_run_host::Ports;

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

fn passes(_: &SessionInfo) -> Cue {
    pass()
}

#[test]
fn the_service_host_runs_its_sessions_through_the_claude_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let ports = Ports {
        messages: rig.ports.clone(),
        declarations: rig.ports.clone(),
        sidecar: rig.ports.clone(),
        layout: Rig::layout(dir.path()),
    };
    let host = service::service_host(&store_path(dir.path()), ports, "ws").expect("the host opens");
    assert_eq!(host.backend_name(), "claude");
}

#[test]
fn a_run_is_alive_while_a_slot_runs_and_while_a_replied_one_winds_down() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    assert!(
        rig.host.live_sessions().unwrap().is_empty(),
        "no run, no session"
    );
    let run = rig.commission("build it").run;
    rig.run_until(400, |r| {
        r.state(&run)
            .slots
            .iter()
            .any(|s| matches!(s.state, SlotState::Running { .. }))
    });
    let live = rig.host.live_sessions().unwrap();
    assert_eq!(live.len(), 1, "the running slot: {live:?}");
    assert_eq!(live[0].run, run);
    assert_eq!(
        live,
        bridge::live_sessions(&rig.bridge).unwrap(),
        "one answer"
    );
}

#[test]
fn a_replied_slot_whose_process_has_not_ended_is_still_alive() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(passes));
    let run = rig.commission("build it").run;
    rig.run_until(400, |r| {
        r.state(&run).slots.iter().any(|s| {
            matches!(
                s.state,
                SlotState::Replied {
                    process: nexus_run_engine::Process::Live,
                    ..
                }
            )
        })
    });
    let state = rig.state(&run);
    let replied: Vec<String> = state
        .slots
        .iter()
        .filter(|s| matches!(s.state, SlotState::Replied { .. }))
        .map(|s| state.session_key(&s.slot).to_string())
        .collect();
    let live: Vec<String> = rig
        .host
        .live_sessions()
        .unwrap()
        .into_iter()
        .map(|s| s.session_key.to_string())
        .collect();
    assert!(
        replied.iter().all(|key| live.contains(key)),
        "a replied slot winding down holds the machine awake: replied {replied:?}, live {live:?}"
    );
    rig.run_until(400, |r| {
        r.state(&run).slots.iter().all(|s| !s.state.is_live())
    });
    assert!(
        rig.host.live_sessions().unwrap().is_empty(),
        "ended: nothing alive"
    );
}

#[test]
fn the_service_host_opens_its_store_with_the_hosts_busy_timeout() {
    // The service loop must not wait out the bridge's five seconds on a held lock (§7, §8.1).
    assert!(HOST_BUSY_TIMEOUT < nexus_run_host::TICK_BUDGET);
    assert_eq!(service::SERVICE_BUSY_TIMEOUT, HOST_BUSY_TIMEOUT);
}

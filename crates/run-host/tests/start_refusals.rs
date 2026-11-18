//! A session that cannot be started says why (6j6v.yt1a): every port that refuses a `StartSession` reaches the
//! service log once, with the run, the session and the port, and the run's local status names it until the
//! session starts or the run finishes. Ids and codes only: what the port said stays off both (M8, PV1).

mod common;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use nexus_run_contracts::{RunId, RunStore, StartPort, StartRefusal};
use nexus_run_host::bridge;
use nexus_run_host::test_support::{scenario_named, Rig, CLOCK_START, SECRET_IN_A_PORT_ERROR};

type Lines = Arc<Mutex<Vec<String>>>;

fn listening(rig: &mut Rig) -> Lines {
    let lines: Lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    rig.host.set_log_sink(Arc::new(move |line| {
        sink.lock().unwrap().push(line.to_owned())
    }));
    lines
}

fn refusals(rig: &Rig, run: &RunId) -> Vec<StartRefusal> {
    bridge::status(&rig.bridge, run, rig.now())
        .expect("status reads")
        .expect("the run is known")
        .local
        .expect("this machine's view")
        .start_refusals
}

fn coder_key(run: &RunId) -> String {
    format!("{run}/build/v1/coder/a1")
}

/// How a port is made to refuse, and the port the host must name for it.
fn refuse(rig: &Rig, call: &'static str) {
    match call {
        "spawn" => rig.backend.refuse_starts.store(true, Ordering::SeqCst),
        other => rig.ports.refuse(other),
    }
}

fn answer(rig: &Rig, call: &'static str) {
    match call {
        "spawn" => rig.backend.refuse_starts.store(false, Ordering::SeqCst),
        other => rig.ports.answer(other),
    }
}

const CASES: &[(&str, StartPort)] = &[
    ("open_thread", StartPort::OpenThread),
    ("post_turn", StartPort::PostTurn),
    ("turn", StartPort::Turn),
    ("prompt", StartPort::Prompt),
    ("persona", StartPort::Persona),
    ("bundle_path", StartPort::BundlePath),
    ("spawn", StartPort::Spawn),
];

/// The refusal is in status before the start deadline (60 s, 30 ticks) can burn an attempt.
fn refused_coder(call: &'static str) -> (tempfile::TempDir, Rig, RunId, Lines) {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, scenario_named("happy"));
    let lines = listening(&mut rig);
    // After the send and before the first tick: the send checks every persona it would start (6j6v.yw6x), so
    // a persona refused here is one the declarations lost between the send and the start.
    let run = rig.commission("the commission").run;
    refuse(&rig, call);
    let check = run.clone();
    rig.run_until(20, move |rig| !refusals(rig, &check).is_empty());
    (dir, rig, run, lines)
}

#[test]
fn every_refused_start_names_its_port_once_in_the_log_and_in_the_runs_status() {
    for (call, port) in CASES {
        let call = *call;
        let (_dir, mut rig, run, lines) = refused_coder(call);
        for _ in 0..5 {
            rig.tick();
        }
        let key = coder_key(&run);
        assert_eq!(
            refusals(&rig, &run),
            vec![StartRefusal {
                session_key: key.clone(),
                handle: "coder".into(),
                port: port.clone(),
                at: refusals(&rig, &run)[0].at,
            }],
            "{call}"
        );
        let lines = lines.lock().unwrap().clone();
        let named: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains(&key) && line.contains(port.as_str()))
            .collect();
        assert_eq!(
            named.len(),
            1,
            "{call}: one line per refusal, not one per tick: {lines:?}"
        );
        assert!(named[0].contains(run.as_str()), "{call}: {}", named[0]);
        assert!(
            lines
                .iter()
                .all(|line| !line.contains(SECRET_IN_A_PORT_ERROR)),
            "{call}: the port's words reached the log: {lines:?}"
        );
        assert!(rig.spawned().is_empty(), "{call}: {:?}", rig.spawned());
    }
}

#[test]
fn a_session_that_starts_after_a_refusal_leaves_no_refusal_in_status() {
    let (_dir, mut rig, run, _lines) = refused_coder("persona");
    answer(&rig, "persona");
    let key = coder_key(&run);
    let started = key.clone();
    rig.run_until(20, move |rig| rig.spawned().contains(&started));
    assert_eq!(refusals(&rig, &run), Vec::new());
}

#[test]
fn a_refusal_that_moves_to_another_port_is_logged_again_and_status_names_the_new_port() {
    let (_dir, mut rig, run, lines) = refused_coder("persona");
    answer(&rig, "persona");
    refuse(&rig, "bundle_path");
    let check = run.clone();
    rig.run_until(10, move |rig| {
        refusals(rig, &check)
            .first()
            .is_some_and(|refusal| refusal.port == StartPort::BundlePath)
    });
    let lines = lines.lock().unwrap().clone();
    let key = coder_key(&run);
    for port in [StartPort::Persona, StartPort::BundlePath] {
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains(&key) && line.contains(port.as_str()))
                .count(),
            1,
            "{port:?}: {lines:?}"
        );
    }
}

#[test]
fn a_finished_run_leaves_no_start_refusal_behind() {
    let (_dir, mut rig, run, _lines) = refused_coder("persona");
    let root = rig.state(&run).root_thread.expect("the root is open");
    bridge::withdraw(&rig.bridge, &root, &Rig::requester(), "stop", rig.now())
        .expect("the requester withdraws");
    rig.run_to_finished(&run);
    assert_eq!(
        rig.store.get(&format!("start_refusal/{run}")).unwrap(),
        None
    );
}

/// The turn is composed twice: once for the post, and again for the start when the post is already in the
/// store, as after a refusal further on or a restart. A refusal at that second composition is the
/// declarations', and is named as theirs.
#[test]
fn a_turn_already_posted_that_cannot_be_composed_again_names_the_turn_port() {
    let (_dir, mut rig, run, lines) = refused_coder("persona");
    answer(&rig, "persona");
    refuse(&rig, "turn");
    let check = run.clone();
    rig.run_until(10, move |rig| {
        refusals(rig, &check)
            .first()
            .is_some_and(|refusal| refusal.port == StartPort::Turn)
    });
    let key = coder_key(&run);
    assert!(lines
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains(&key) && line.contains(StartPort::Turn.as_str())));
    assert!(rig.spawned().is_empty(), "{:?}", rig.spawned());
}

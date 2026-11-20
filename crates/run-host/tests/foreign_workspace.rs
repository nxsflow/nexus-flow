//! A run commissioned under another workspace id than the one this host attends (6j6v.4b43): sends from before
//! the workspace had its origin left runs under `local` in `run.sqlite`, and the first service to tick the
//! workspace woke them. Such a run is never claimed here and starts nothing: its claim key is another
//! workspace's, so a claim would sit beside this workspace's own on the one `<root>-v2`. The service log says
//! so once, and status names what the run waits for and how to end it.
//!
//! And the exit for it: the requester of such a run is an identity of a workspace that is gone, so the owner at
//! this workspace's terminal withdraws it, recorded as the owner's (`withdrawn_by_owner`).

use std::sync::{Arc, Mutex};

use nexus_run_contracts::{
    Base, Event, EventBody, Origin, ReasonCode, RunId, RunStore, StartPort, StartRefusal,
};
use nexus_run_host::bridge::{self, Caller, CommissionReceipt, Refused};
use nexus_run_host::ports::Refs;
use nexus_run_host::test_support::{replies_pass, Rig, CLOCK_START, WORKSPACE};

const STALE: &str = "local";
const STALE_REQUESTER: &str = "local/carsten";

type Lines = Arc<Mutex<Vec<String>>>;

fn listening(rig: &mut Rig) -> Lines {
    let lines: Lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    rig.host.set_log_sink(Arc::new(move |line| {
        sink.lock().unwrap().push(line.to_owned())
    }));
    lines
}

/// A send from before the workspace had its origin: the same store, another workspace id.
fn stale_commission(rig: &Rig) -> CommissionReceipt {
    let mut stale = rig.bridge.clone();
    stale.workspace_id = STALE.to_owned();
    bridge::commission(
        &stale,
        "coding",
        "a stale commission",
        &Refs::default(),
        STALE_REQUESTER,
        rig.policy.clone(),
        rig.now(),
    )
    .expect("the stale send is accepted")
}

fn claim(rig: &Rig, workspace: &str) -> Option<String> {
    rig.store
        .get(&format!("claim/{workspace}"))
        .unwrap()
        .map(|bytes| String::from_utf8(bytes).unwrap())
}

fn owner() -> Caller {
    Caller {
        actor: "ws-1/owner".to_owned(),
        session: None,
    }
}

#[test]
fn a_run_of_another_workspace_is_never_claimed_and_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let lines = listening(&mut rig);
    let run = stale_commission(&rig).run;
    for _ in 0..30 {
        rig.tick();
    }
    assert_eq!(claim(&rig, STALE), None, "the stale run took its claim");
    assert_eq!(claim(&rig, WORKSPACE), None);
    assert_eq!(rig.tree.begins(), 0, "a tree operation began for it");
    assert!(rig.spawned().is_empty(), "{:?}", rig.spawned());
    assert!(!rig.is_finished(&run));

    let lines = lines.lock().unwrap().clone();
    let named: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(run.as_str()) && line.contains(STALE))
        .collect();
    assert_eq!(named.len(), 1, "one line, not one per tick: {lines:?}");
    assert!(named[0].contains("workspace"), "{}", named[0]);
    assert!(
        named[0].contains("nxc status"),
        "the line points to the exit: {}",
        named[0]
    );
}

#[test]
fn status_names_the_other_workspace_and_how_to_end_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = stale_commission(&rig);
    for _ in 0..10 {
        rig.tick();
    }
    let status = bridge::status(&rig.bridge, &receipt.run, rig.now())
        .unwrap()
        .expect("the run is known");
    assert!(!status.claim.held);
    assert_eq!(status.claim.workspace_id, STALE);
    let waiting = status.waiting_for.expect("it waits for somebody");
    assert!(waiting.contains(STALE), "{waiting}");
    let next = status.next_action.expect("it says how to end it");
    assert!(next.contains(STALE) && next.contains(WORKSPACE), "{next}");
    assert!(
        next.contains(&format!("nxc withdraw --thread {}", receipt.root_thread)),
        "{next}"
    );
}

/// The annotation is for a run of another workspace only: a run of this one reads as it did before.
#[test]
fn status_of_a_run_of_this_workspace_names_no_other_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = rig.commission("this workspace's run");
    for _ in 0..3 {
        rig.tick();
    }
    let status = bridge::status(&rig.bridge, &receipt.run, rig.now())
        .unwrap()
        .expect("the run is known");
    let waiting = status.waiting_for.unwrap_or_default();
    assert!(!waiting.contains("a service of workspace"), "{waiting}");
    let next = status.next_action.unwrap_or_default();
    assert!(!next.contains("commissioned under workspace"), "{next}");
}

/// The question the ticket asks: the claim key is per workspace id, so before this a run under `local` and a run
/// under this workspace each held a claim — on the one `<root>-v2`.
#[test]
fn a_run_of_another_workspace_holds_no_claim_beside_this_workspaces_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    // The stale run is the older one: the queue would serve it first.
    let stale = stale_commission(&rig).run;
    rig.clock.advance(1);
    let ours = rig.commission("this workspace's run").run;
    let check = ours.clone();
    rig.run_until(30, move |rig| {
        !rig.spawned().is_empty() && claim(rig, WORKSPACE).as_deref() == Some(check.as_str())
    });
    assert_eq!(claim(&rig, STALE), None, "two claims on one directory");
    assert!(
        rig.spawned()
            .iter()
            .all(|key| !key.starts_with(stale.as_str())),
        "{:?}",
        rig.spawned()
    );
}

/// A stale run that took its claim before this change — the smoke's, which entered build — touches the tree no
/// further and starts no session, and status says why in the start-refusal vocabulary.
#[test]
fn a_run_of_another_workspace_that_holds_its_claim_already_prepares_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let run = stale_commission(&rig).run;
    for _ in 0..3 {
        rig.tick();
    }
    took_its_claim_before_the_change(&rig, &run);
    for _ in 0..30 {
        rig.tick();
    }
    assert_eq!(rig.tree.begins(), 0, "a tree operation began for it");
    assert!(rig.spawned().is_empty(), "{:?}", rig.spawned());
}

fn took_its_claim_before_the_change(rig: &Rig, run: &RunId) {
    rig.store
        .compare_and_set(
            &format!("claim/{STALE}"),
            None,
            Some(run.as_str().as_bytes()),
        )
        .unwrap();
    let event = Event::new(
        run.clone(),
        0,
        rig.now(),
        format!("claim_taken/{run}"),
        Origin::Host,
        EventBody::ClaimTaken,
    );
    rig.store.enqueue(run, &event, &[]).unwrap();
}

/// …and once its tree is prepared, a start the workspace refuses is named like any refused start.
#[test]
fn a_prepared_run_of_another_workspace_names_the_workspace_as_the_refusing_port() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let run = stale_commission(&rig).run;
    for _ in 0..3 {
        rig.tick();
    }
    took_its_claim_before_the_change(&rig, &run);
    let prepared = Event::new(
        run.clone(),
        0,
        rig.now(),
        format!("tree_prepared/{run}"),
        Origin::Host,
        EventBody::TreePrepared {
            base: Base {
                branch: "main".to_owned(),
                commit: "b".repeat(40),
            },
            found: None,
        },
    );
    rig.store.enqueue(&run, &prepared, &[]).unwrap();
    let check = run.clone();
    rig.run_until(20, move |rig| {
        !bridge::status(&rig.bridge, &check, rig.now())
            .unwrap()
            .unwrap()
            .local
            .unwrap()
            .start_refusals
            .is_empty()
    });
    let refusals: Vec<StartRefusal> = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap()
        .local
        .unwrap()
        .start_refusals;
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0].port, StartPort::Workspace);
    assert!(rig.spawned().is_empty(), "{:?}", rig.spawned());
}

#[test]
fn a_withdrawn_run_of_another_workspace_finishes_without_parking_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = stale_commission(&rig);
    let run = receipt.run.clone();
    for _ in 0..3 {
        rig.tick();
    }
    took_its_claim_before_the_change(&rig, &run);
    for _ in 0..3 {
        rig.tick();
    }
    bridge::withdraw_as_owner(
        &rig.bridge,
        &receipt.root_thread,
        &owner(),
        "stale",
        rig.now(),
    )
    .expect("the owner withdraws a stale run");
    rig.run_to_finished(&run);
    assert_eq!(
        rig.tree.begins(),
        0,
        "the shared directory was parked for it"
    );
    assert_eq!(claim(&rig, STALE), None, "its stale claim is released");
}

// ---- (b): the owner's exit for a run whose requester is an identity of another workspace ----------------

#[test]
fn the_owner_withdraws_a_run_of_another_workspace_and_the_log_says_it_was_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = stale_commission(&rig);
    for _ in 0..3 {
        rig.tick();
    }
    assert_eq!(
        bridge::withdraw(&rig.bridge, &receipt.root_thread, &owner(), "x", rig.now()),
        Err(Refused::NotTheRequester),
        "the requester's own exit stays the requester's"
    );
    bridge::withdraw_as_owner(
        &rig.bridge,
        &receipt.root_thread,
        &owner(),
        "stale",
        rig.now(),
    )
    .expect("the owner withdraws");
    rig.run_to_finished(&receipt.run);
    let finished = rig
        .events(&receipt.run)
        .into_iter()
        .find_map(|event| match event.body {
            EventBody::Decision(nexus_run_contracts::Decision::Finished { reason, .. }) => {
                Some(reason)
            }
            _ => None,
        });
    assert_eq!(finished, Some(Some(ReasonCode::WithdrawnByOwner)));
}

#[test]
fn the_owners_exit_is_refused_on_a_run_of_this_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = rig.commission("this workspace's run");
    rig.tick();
    assert_eq!(
        bridge::withdraw_as_owner(&rig.bridge, &receipt.root_thread, &owner(), "x", rig.now()),
        Err(Refused::NotTheRequester)
    );
    assert!(!rig.is_finished(&receipt.run));
}

#[test]
fn the_owners_exit_is_refused_to_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = stale_commission(&rig);
    rig.tick();
    let session = Caller {
        actor: "ws-1/owner".to_owned(),
        session: Some("some-session".to_owned()),
    };
    assert_eq!(
        bridge::withdraw_as_owner(&rig.bridge, &receipt.root_thread, &session, "x", rig.now()),
        Err(Refused::NotTheRequester)
    );
}

/// Before the host opens the root there is nowhere to post: the withdrawal goes straight into the inbox and
/// nothing is returned. Once the root is open it is posted there, and the message is returned.
#[test]
fn the_owners_withdrawal_posts_only_once_the_root_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let unopened = stale_commission(&rig);
    assert_eq!(
        bridge::withdraw_as_owner(
            &rig.bridge,
            &unopened.root_thread,
            &owner(),
            "stale",
            rig.now()
        ),
        Ok(None)
    );
    let opened = stale_commission(&rig);
    let check = opened.run.clone();
    rig.run_until(10, move |rig| rig.state(&check).root_message.is_some());
    let posted = bridge::withdraw_as_owner(
        &rig.bridge,
        &opened.root_thread,
        &owner(),
        "stale",
        rig.now(),
    )
    .expect("the owner withdraws");
    assert!(
        posted.is_some(),
        "the withdrawal is posted on the open root"
    );
    for run in [&unopened.run, &opened.run] {
        rig.run_to_finished(run);
    }
}

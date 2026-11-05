//! Every non-final state has a remedy and a human exit (§6.5). The rows come from `run_row` and `slot_row`,
//! whose exhaustive `match`es fail to compile when a variant is added, and from examples reached by folding
//! real inputs. This binary fails when a row has no example.

#[path = "script.rs"]
mod script;

use std::collections::BTreeSet;

use nexus_run_contracts::{EndCause, Outcome, ParkOutcome, StopKind};
use nexus_run_engine::{
    run_row, slot_row, DeliverBody, Effect, Process, RunState, RunView, SlotExample, SlotState,
    RUN_ROWS, SLOT_ROWS,
};
use script::Script;

/// Answers whatever the run owes, as a host does, until it is `Finished`. Sessions stop on the first signal.
/// Returns the number of signals and the stop kinds it saw, for the callers' own assertions.
fn drive_to_finished(s: &mut Script) -> Vec<StopKind> {
    let mut signals = Vec::new();
    for _ in 0..64 {
        if s.state.is_finished() {
            return signals;
        }
        let effects = s.effects();
        match effects.first() {
            Some(Effect::StopSession {
                session_key,
                kind,
                nth,
            }) => {
                signals.push(*kind);
                let (key, kind, nth) = (session_key.clone(), *kind, *nth);
                s.input(s.stop_signalled(&key, nth, kind));
                s.input(s.ended(&key, EndCause::StoppedByHost));
            }
            Some(Effect::ParkTree { .. }) => {
                let outcome = if s.state.claim_since.is_some() {
                    ParkOutcome::Clean { branch: None }
                } else {
                    ParkOutcome::NotHeld
                };
                s.input(s.tree_parked(outcome));
            }
            Some(Effect::Deliver { .. }) => {
                s.input(s.delivered());
            }
            Some(Effect::ReleaseClaim { .. }) => {
                s.input(s.claim_released());
            }
            // A replied slot inside its wind-down ends on its own.
            None => {
                let live = s
                    .state
                    .slots
                    .iter()
                    .find(|r| r.state.is_live())
                    .map(|r| s.state.session_key(&r.slot))
                    .unwrap_or_else(|| {
                        panic!("nothing owed and nothing live: {:?}", s.state.phase)
                    });
                s.input(s.ended(&live, EndCause::TurnComplete));
            }
            Some(other) => panic!("a withdrawn run owes {other:?}"),
        }
    }
    panic!("the run did not finish: {:?}", s.state.phase);
}

/// A withdrawal from here reaches `Finished`, with every stop through the table and the claim last.
fn assert_withdrawal_finishes(state: &RunState) {
    let finishing_before = state.is_finishing();
    let outcome_before = state.outcome();
    let mut s = Script::from_state(state.clone());
    s.input(s.withdrawn());
    if finishing_before {
        let line = s.decisions();
        assert!(
            line == ["withdraw_forced"] || line == ["ignored already_finishing"],
            "{line:?}"
        );
        assert_eq!(s.state.outcome(), outcome_before, "the outcome is kept");
    } else {
        assert_eq!(s.state.outcome(), Some(Outcome::Withdrawn));
        assert!(
            s.decisions()
                .iter()
                .any(|l| l.starts_with("finishing withdrawn")),
            "{:?}",
            s.decisions()
        );
        assert_every_live_slot_owes_its_stop(&s);
    }
    drive_to_finished(&mut s);
    assert!(s.state.is_finished());
    assert!(s.effects().is_empty());
    assert!(s.state.timers.is_empty());
    let released = s
        .log
        .iter()
        .position(|e| matches!(e.body, nexus_run_contracts::EventBody::ClaimReleased))
        .expect("the release is in the log");
    if let Some(parked) = s
        .log
        .iter()
        .position(|e| matches!(e.body, nexus_run_contracts::EventBody::TreeParked { .. }))
    {
        assert!(parked < released, "the park is recorded before the release");
    }
}

/// Live slots outside a wind-down each owe a stop at once; nothing else is owed before they end.
fn assert_every_live_slot_owes_its_stop(s: &Script) {
    let stops: BTreeSet<String> = s
        .effects()
        .iter()
        .filter_map(|e| match e {
            Effect::StopSession { session_key, .. } => Some(session_key.clone()),
            _ => None,
        })
        .collect();
    for record in &s.state.slots {
        let key = s.state.session_key(&record.slot);
        let in_wind_down = matches!(record.state, SlotState::Replied { ladder: None, .. });
        let ladder_waits = record.state.ladder().is_some();
        if record.state.is_live() && !in_wind_down && !ladder_waits {
            assert!(stops.contains(&key), "{key} is live and owes no stop");
        }
        if !record.state.is_live() {
            assert!(!stops.contains(&key), "{key} is not live and owes a stop");
        }
    }
    if s.state.any_live() {
        for effect in s.effects() {
            assert!(
                matches!(effect, Effect::StopSession { .. }),
                "{effect:?} before the stops"
            );
        }
    }
}

#[test]
fn every_non_final_state_has_a_remedy_and_a_human_exit() {
    // Every run row has an example, and every example is on the row it says.
    let examples = RunState::non_final_examples();
    let rows: BTreeSet<usize> = examples.iter().filter_map(run_row).collect();
    assert_eq!(
        rows,
        (0..RUN_ROWS).collect(),
        "a row of the run table has no example"
    );

    for state in &examples {
        assert_withdrawal_finishes(state);
        assert_has_a_remedy(state);
    }

    // Every slot row, every `Ended` cause, by example.
    let slots = RunState::slot_examples();
    let rows: BTreeSet<usize> = slots
        .iter()
        .map(|e| slot_row(&e.run.slots[e.run.find_slot(&e.slot).unwrap()].state))
        .collect();
    assert_eq!(
        rows,
        (0..SLOT_ROWS).collect(),
        "a row of the slot table has no example"
    );
    assert_eq!(
        SlotState::non_final_examples()
            .iter()
            .map(slot_row)
            .collect::<BTreeSet<_>>(),
        (0..SLOT_ROWS).collect::<BTreeSet<_>>()
    );

    for example in &slots {
        assert_withdrawal_finishes(&example.run);
        assert_has_a_remedy(&example.run);
        assert_next_rung(example);
    }
}

/// A hang has something that will notice it, or the wait is a person's by declaration.
fn assert_has_a_remedy(state: &RunState) {
    let owes = !nexus_run_engine::pending_effects(state).is_empty();
    let armed = !state.timers.is_empty();
    let a_persons_wait = matches!(
        state.view(),
        RunView::NeedsDecision { .. } | RunView::Parked { .. } | RunView::Held { .. }
    );
    assert!(
        owes || armed || a_persons_wait,
        "no remedy and no declared wait: {:?}",
        state.phase
    );
}

fn first_timer(state: &RunState, prefix: &str) -> String {
    state
        .timers
        .keys()
        .find(|timer| timer.starts_with(prefix))
        .cloned()
        .unwrap_or_else(|| panic!("no {prefix} timer is armed: {:?}", state.timers.keys()))
}

/// For each timed row: the named timer is armed in the state, and its firing leads to the next rung.
fn assert_next_rung(example: &SlotExample) {
    let index = example.run.find_slot(&example.slot).unwrap();
    let slot_state = example.run.slots[index].state.clone();
    let key = example.run.session_key(&example.slot);
    let mut s = Script::from_state(example.run.clone());
    match &slot_state {
        SlotState::Pending => {
            // A slot waiting for the tree spends nothing: the tree's window is what is armed.
            first_timer(&s.state, "tree_window");
            assert_eq!(s.effects().len(), 1, "only PrepareTree: {:?}", s.effects());
            s.input(s.tree_prepared());
            assert!(matches!(s.slot_state(&key), SlotState::Starting { .. }));
        }
        SlotState::Starting { .. } => {
            s.fire(&first_timer(&example.run, "start_deadline/"));
            assert!(matches!(s.slot_state(&key), SlotState::Stopping { .. }));
        }
        SlotState::Running {
            open_call: None, ..
        } => {
            s.fire(&first_timer(&example.run, "quiet/"));
            assert!(matches!(s.slot_state(&key), SlotState::Quiet { .. }));
            assert!(matches!(s.effects_of(&key)[0], Effect::NudgeSession { .. }));
        }
        SlotState::Running {
            open_call: Some(_), ..
        } => {
            first_timer(&example.run, "call_bound/");
            s.fire(&first_timer(&example.run, "subprocess_idle/"));
            assert!(matches!(s.slot_state(&key), SlotState::Quiet { .. }));
        }
        SlotState::Quiet { .. } | SlotState::Nudged { .. } => {
            s.fire(&first_timer(&example.run, "confirm/"));
            assert!(matches!(s.slot_state(&key), SlotState::Stopping { .. }));
        }
        SlotState::Stopping { .. } => {
            assert_eq!(
                s.effects_of(&key),
                vec![Effect::StopSession {
                    session_key: key.clone(),
                    kind: StopKind::Term,
                    nth: 1
                }]
            );
            s.input(s.stop_signalled(&key, 1, StopKind::Term));
            s.fire(&first_timer(&s.state, "stop_grace/1/"));
            assert_eq!(
                s.effects_of(&key),
                vec![Effect::StopSession {
                    session_key: key.clone(),
                    kind: StopKind::Term,
                    nth: 2
                }]
            );
        }
        SlotState::Replied {
            process: Process::Live,
            ..
        } => {
            s.fire(&first_timer(&example.run, "wind_down/"));
            assert!(matches!(
                s.effects_of(&key)[0],
                Effect::StopSession {
                    kind: StopKind::Term,
                    ..
                }
            ));
        }
        // Its end folded; nothing of it stays armed or owed.
        SlotState::Replied {
            process: Process::Ended,
            ..
        } => {
            assert!(s.effects_of(&key).is_empty());
        }
        SlotState::Ended { cause } => match cause {
            EndCause::UsageLimit { .. } | EndCause::ProviderUnavailable => {
                s.fire(&first_timer(&example.run, "hold_retry/"));
                let next =
                    s.state
                        .latest(&example.slot.step, example.slot.visit, &example.slot.member);
                assert_eq!(next.unwrap().slot.attempt, example.slot.attempt + 1);
            }
            // The table's remedy is immediate: with one attempt budgeted, it parked.
            EndCause::TurnComplete
            | EndCause::Exit { .. }
            | EndCause::Lost
            | EndCause::PermissionRefused { .. }
            | EndCause::StoppedByHost => {
                assert!(
                    matches!(s.state.view(), RunView::Parked { .. }),
                    "{cause:?}"
                );
            }
        },
    }
}

#[test]
fn a_delivered_run_has_no_discharge_and_a_withdrawn_one_has_no_result() {
    let mut delivered = Script::coding();
    delivered.to_first_session();
    delivered.pass_until("finish");
    delivered.run_step_to_pass("finish");
    delivered.input(delivered.tree_parked_clean());
    assert!(matches!(
        delivered.deliveries()[0].1,
        DeliverBody::Message(_)
    ));

    let mut withdrawn = Script::coding();
    withdrawn.to_first_session();
    withdrawn.pass_until("finish");
    let key = withdrawn.latest_key("finish", None);
    withdrawn.input(withdrawn.started(&key));
    withdrawn.input(withdrawn.withdrawn());
    withdrawn.input(withdrawn.ended(&key, EndCause::StoppedByHost));
    withdrawn.input(withdrawn.tree_parked_clean());
    assert!(matches!(
        withdrawn.deliveries()[0].1,
        DeliverBody::Discharge { .. }
    ));
}

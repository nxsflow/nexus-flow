//! The engine's half of the determinism gate (§11), run as its own step of the `lint` job by
//! `.github/scripts/run-engine-determinism.sh`.
//!
//! In this process: for every scripted case, a fold of the log equals the state the case ended in, and the
//! case run again under a different simulated clock gives identical decision lists, instants measured from
//! its start. The script runs this file twice in separate processes, with another time zone and environment,
//! and compares what it wrote to `$NXR_DETERMINISM_OUT` byte for byte, because a process-seeded order or
//! anything read at startup would show only there.

#[path = "script.rs"]
pub mod script;

/// Here a case is an entry of `ALL`, and the `cases` binary owns its `#[test]`.
macro_rules! case_test {
    ($name:ident) => {};
}

include!("cases/group.rs");

#[path = "cases/groups.rs"]
mod groups;

use script::{Script, CLOCK_START};

/// A week and a bit later, off every round second a timer could fall on by accident.
const ANOTHER_CLOCK: i64 = CLOCK_START + 777_777_777;

fn run(case: fn(&mut Script), start: i64) -> Script {
    let mut script = Script::at(start);
    case(&mut script);
    script
}

fn all_cases() -> impl Iterator<Item = (&'static str, fn(&mut Script))> {
    groups::ALL.iter().flat_map(|group| group.iter().copied())
}

#[test]
fn a_fold_of_the_log_equals_the_state_every_case_ended_in() {
    for (name, case) in all_cases() {
        let script = run(case, CLOCK_START);
        assert_eq!(script.fold_state(), script.state, "{name}");
    }
}

#[test]
fn every_case_decides_the_same_lists_under_a_different_clock() {
    for (name, case) in all_cases() {
        let first = run(case, CLOCK_START);
        let second = run(case, ANOTHER_CLOCK);
        assert_eq!(first.transcript(), second.transcript(), "{name}");
    }
}

#[test]
fn the_decision_lists_go_to_the_gates_file_when_it_names_one() {
    let Some(path) = std::env::var_os("NXR_DETERMINISM_OUT") else {
        return;
    };
    let mut text = String::new();
    for (name, case) in all_cases() {
        text.push_str(&format!("## {name}\n"));
        text.push_str(&run(case, CLOCK_START).transcript());
    }
    assert!(!text.is_empty());
    std::fs::write(path, text).expect("the gate's file is writable");
}

//! The reducer cases of §12.1, without processes. Each asserts the decision list of every input it
//! cares about, never only the end state (memory a-test-name-is-a-claim).

#[path = "../script.rs"]
pub mod script;

/// Under this binary every case is a `#[test]` of its own name.
macro_rules! case_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            super::$name(&mut crate::script::Script::coding());
        }
    };
}

include!("group.rs");

mod groups;

/// §12.1: every reason code of §6.7 appears in at least one case, by what the cases actually folded.
#[test]
fn every_reason_code_appears_in_at_least_one_case() {
    let mut seen = std::collections::BTreeSet::new();
    for group in groups::ALL {
        for (_, case) in *group {
            let mut script = script::Script::coding();
            case(&mut script);
            seen.append(&mut script.codes);
        }
    }
    let missing: Vec<&&str> = nexus_run_contracts::ReasonCode::ALL
        .iter()
        .filter(|code| !seen.contains(**code))
        .collect();
    assert!(missing.is_empty(), "no case reaches {missing:?}");
}

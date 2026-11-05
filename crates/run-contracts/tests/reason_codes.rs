//! `ReasonCode` is one closed enum, and the list of §6.7 is generated from it. `tree_refused` is the
//! only tree code in it; the detail is the separate closed `GitReason` and `TreeRefusal`.

use nexus_run_contracts::{GitReason, ReasonCode, TreeRefusal, REMEDY_TABLE};
use serde::Deserialize;

/// §6.7 of the specification, typed in here on purpose: the one list that does not come from the enum.
const SECTION_6_7: &[&str] = &[
    "stale_timer",
    "duplicate_reply",
    "late_reply_after_skip",
    "unknown_session",
    "final_state",
    "already_finishing",
    // Added by the engine ticket (6j6v.r426): `ignored` needs a code for an input the present state has
    // no use for. §6.7 names no such code; a finding on the ticket says so.
    "out_of_state",
    "quiet",
    "hung",
    "start_failed",
    "lost",
    "exit",
    "turn_complete",
    "stopped_by_host",
    "would_not_stop",
    "root_failed",
    "tree_refused",
    "poison_input",
    "permission_refused",
    "ended_without_reply",
    "fan_out_incomplete",
    "usage_limit",
    "provider_unavailable",
    "hold_limit",
    "ceiling",
    "accepted_by_requester",
    "withdrawn_by_requester",
    "withdrawn_by_owner",
    "delivered",
];

#[derive(Deserialize)]
struct Golden {
    reason_codes: Vec<String>,
    git_reasons: Vec<String>,
    tree_refusals: Vec<String>,
    remedy_table: Vec<(String, String)>,
}

fn golden() -> Golden {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/reason-codes.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_section_6_7_list_equals_the_enum() {
    assert_eq!(ReasonCode::ALL, SECTION_6_7);
}

#[test]
fn the_golden_pins_the_reason_codes_the_git_reasons_and_the_tree_refusals() {
    let golden = golden();
    assert_eq!(golden.reason_codes, ReasonCode::ALL);
    assert_eq!(golden.git_reasons, GitReason::ALL);
    assert_eq!(golden.tree_refusals, TreeRefusal::ALL);
}

#[test]
fn every_listed_code_serialises_to_itself_and_reads_back_as_a_named_variant() {
    for &code in ReasonCode::ALL {
        let parsed = ReasonCode::parse(code);
        assert!(!matches!(parsed, ReasonCode::Unknown(_)), "{code}");
        assert_eq!(serde_json::to_value(&parsed).unwrap(), code);
    }
    for &code in GitReason::ALL {
        let parsed = GitReason::parse(code);
        assert!(!matches!(parsed, GitReason::Unknown(_)), "{code}");
        assert_eq!(serde_json::to_value(&parsed).unwrap(), code);
    }
}

#[test]
fn git_reason_has_no_hook_failed_and_no_no_identity() {
    assert_eq!(
        GitReason::ALL,
        [
            "bound_exceeded",
            "fetch_failed",
            "lock_held",
            "exit_nonzero"
        ]
    );
    assert!(matches!(
        GitReason::parse("hook_failed"),
        GitReason::Unknown(_)
    ));
    assert!(matches!(
        GitReason::parse("no_identity"),
        GitReason::Unknown(_)
    ));
}

#[test]
fn a_tree_refusal_detail_is_never_a_reason_code() {
    assert!(ReasonCode::ALL.contains(&"tree_refused"));
    for detail in TreeRefusal::ALL.iter().chain(GitReason::ALL) {
        assert!(
            !ReasonCode::ALL.contains(detail),
            "{detail} belongs to the tree refusal, not to ReasonCode"
        );
    }
}

#[test]
fn the_golden_pins_the_remedy_table_and_every_row_names_a_listed_code() {
    let written: Vec<(String, String)> = REMEDY_TABLE
        .iter()
        .map(|(row, code)| ((*row).to_owned(), code.as_str().to_owned()))
        .collect();
    assert_eq!(golden().remedy_table, written);
    for (row, code) in REMEDY_TABLE {
        assert!(ReasonCode::ALL.contains(&code.as_str()), "{row}");
    }
}

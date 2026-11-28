//! NEX-173: `Engine::merge_gate_decide`, the counterpart of `nxs merge-gate decide` (RFC 0010,
//! "`nxs merge-gate decide`", "What the build must prove"): the gate never gives a pull request to
//! pm when the rules at the base cannot be read, or when there is no valid verdict for the base and
//! head given. Each test fails if its guarantee breaks.

mod decide_common;

use decide_common::{
    assert_pm_merges, assert_waits_for_owner, codes, entry, Pr, GATE, GATE_PATH, GENESIS,
    GENESIS_PATH,
};
use nexus_flow_seam::merge_gate::{GitError, Reason};
use nexus_flow_seam::Error;
use serde_json::{json, Value};

/// The pull request's report when the base holds `genesis` and `gate`, `None` for a missing file,
/// with a verdict that would let pm merge.
fn decide_at_base(test: &str, genesis: Option<&str>, gate: Option<&str>) -> Vec<&'static str> {
    let mut files = Vec::new();
    files.extend(genesis.map(|g| (GENESIS_PATH, g)));
    files.extend(gate.map(|g| (GATE_PATH, g)));
    let pr = Pr::with_base(test, &files);
    let report = pr.decide(&pr.verdict());
    assert_waits_for_owner(&report);
    let Reason::RulesUnreadable { errors } = &report.reasons[0] else {
        panic!("expected rules_unreadable, got {report:?}");
    };
    assert_eq!(
        report.reasons.len(),
        1,
        "nothing else is checked: {report:?}"
    );
    errors.iter().map(|e| e.code).collect()
}

#[test]
fn a_valid_verdict_whose_pm_rule_is_met_lets_pm_merge() {
    let pr = Pr::new("pm-rule");

    let report = pr.decide(&pr.verdict());

    assert_pm_merges(&report);
    assert_eq!(report.base, pr.base);
    assert_eq!(report.head, pr.head);
    assert_eq!(
        report.reasons,
        [Reason::PmRule {
            rules: vec!["decided-design".to_string()]
        }]
    );
    assert!(!report.owner_approved);
}

#[test]
fn genesis_missing_at_the_base_goes_to_the_owner() {
    let errors = decide_at_base("genesis-missing", None, Some(GATE));
    assert_eq!(errors, ["file_missing"]);
}

#[test]
fn genesis_without_the_mark_goes_to_the_owner() {
    let errors = decide_at_base(
        "genesis-no-mark",
        Some("# GENESIS\n\n- a rule\n"),
        Some(GATE),
    );
    assert_eq!(errors, ["mark_missing"]);
}

#[test]
fn genesis_with_two_marks_goes_to_the_owner() {
    let twice = format!("{GENESIS}\n<!-- nexus-flow: owner-rules -->\n\n- another\n");
    let errors = decide_at_base("genesis-two-marks", Some(&twice), Some(GATE));
    assert_eq!(errors, ["mark_doubled"]);
}

#[test]
fn a_paragraph_between_the_mark_and_the_list_goes_to_the_owner() {
    let genesis = "<!-- nexus-flow: owner-rules -->\n\nThese come back:\n\n- a rule\n";
    let errors = decide_at_base("genesis-paragraph", Some(genesis), Some(GATE));
    assert_eq!(errors, ["list_missing"]);
}

#[test]
fn gate_toml_missing_at_the_base_goes_to_the_owner() {
    let errors = decide_at_base("gate-missing", Some(GENESIS), None);
    assert_eq!(errors, ["file_missing"]);
}

#[test]
fn an_invalid_gate_toml_goes_to_the_owner() {
    let errors = decide_at_base("gate-invalid", Some(GENESIS), Some("format = [\n"));
    assert_eq!(errors, ["toml_invalid"]);
}

#[test]
fn a_gate_toml_of_another_format_goes_to_the_owner() {
    let gate = GATE.replace("format = 1", "format = 2");
    let errors = decide_at_base("gate-format", Some(GENESIS), Some(&gate));
    assert_eq!(errors, ["format_unknown"]);
}

#[test]
fn a_gate_toml_without_owners_goes_to_the_owner() {
    let gate = GATE.replace("owners = [\"cabcookie\"]", "owners = []");
    let errors = decide_at_base("gate-no-owners", Some(GENESIS), Some(&gate));
    assert_eq!(errors, ["owners_empty"]);
}

#[test]
fn a_gate_toml_without_verdict_authors_goes_to_the_owner() {
    let gate = GATE.replace(
        "verdict_authors = [\"nexusflow-ug\"]",
        "verdict_authors = []",
    );
    let errors = decide_at_base("gate-no-authors", Some(GENESIS), Some(&gate));
    assert_eq!(errors, ["verdict_authors_empty"]);
}

/// The report's `verdict_invalid` detail, after asserting that the pull request waits for the
/// owner for that reason alone.
fn invalid_detail(pr: &Pr, verdict: Option<&[u8]>) -> Vec<String> {
    let report = nexus_flow_seam::Engine::merge_gate_decide(
        &pr.path, &pr.base, &pr.head, None, verdict, None,
    )
    .unwrap();
    assert_waits_for_owner(&report);
    assert_eq!(codes(&report), ["verdict_invalid"], "{report:?}");
    let Reason::VerdictInvalid { detail } = &report.reasons[0] else {
        unreachable!()
    };
    assert!(!detail.is_empty());
    detail.clone()
}

fn invalid(pr: &Pr, verdict: &Value) -> Vec<String> {
    invalid_detail(pr, Some(verdict.to_string().as_bytes()))
}

#[test]
fn no_verdict_goes_to_the_owner() {
    let pr = Pr::new("verdict-none");
    invalid_detail(&pr, None);
}

#[test]
fn a_verdict_that_is_not_json_goes_to_the_owner() {
    let pr = Pr::new("verdict-not-json");
    invalid_detail(&pr, Some(b"{ not json"));
}

#[test]
fn a_verdict_of_another_format_goes_to_the_owner() {
    let pr = Pr::new("verdict-format");
    let mut verdict = pr.verdict();
    verdict["format"] = json!(2);
    invalid(&pr, &verdict);
}

#[test]
fn a_verdict_for_another_base_goes_to_the_owner() {
    let pr = Pr::new("verdict-base");
    let mut verdict = pr.verdict();
    verdict["base"] = json!(pr.head);
    invalid(&pr, &verdict);
}

#[test]
fn a_verdict_for_another_head_goes_to_the_owner() {
    let pr = Pr::new("verdict-head");
    let mut verdict = pr.verdict();
    verdict["head"] = json!(pr.base);
    invalid(&pr, &verdict);
}

#[test]
fn a_verdict_missing_a_rule_goes_to_the_owner() {
    let pr = Pr::new("verdict-missing-rule");
    let mut verdict = pr.verdict();
    verdict["verdicts"].as_array_mut().unwrap().remove(3);
    invalid(&pr, &verdict);
}

#[test]
fn a_verdict_with_an_unknown_rule_goes_to_the_owner() {
    let pr = Pr::new("verdict-unknown-rule");
    let mut verdict = pr.verdict();
    verdict["verdicts"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "rule": "made-up", "outcome": "met", "reason": "A rule nobody wrote." }));
    invalid(&pr, &verdict);
}

#[test]
fn an_outcome_the_rules_effect_does_not_allow_goes_to_the_owner() {
    let pr = Pr::new("verdict-outcome");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "stored-format")["outcome"] = json!("met");
    invalid(&pr, &verdict);
}

#[test]
fn a_decision_whose_path_is_missing_at_the_base_goes_to_the_owner() {
    let pr = Pr::new("verdict-decision-path");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "genesis.2")["decision"] = json!({ "path": "docs/rfc/9999-nothing.md" });
    invalid(&pr, &verdict);
}

#[test]
fn a_decision_that_exists_only_on_the_head_goes_to_the_owner() {
    let mut pr = Pr::new("verdict-decision-on-head");
    pr.push("docs/rfc/0011-new.md", "# RFC 0011\n");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "genesis.2")["decision"] = json!({ "path": "docs/rfc/0011-new.md" });
    invalid(&pr, &verdict);
}

/// A comment of GitHub's "list issue comments" response.
fn comment(id: u64, login: &str, created_at: &str, body: String) -> Value {
    json!({ "id": id, "user": { "login": login }, "created_at": created_at, "body": body })
}

fn verdict_body(verdict: &Value) -> String {
    format!("<!-- nexus-flow: merge-gate-verdict -->\n\n```json\n{verdict}\n```\n")
}

#[test]
fn a_verdict_comment_by_its_author_lets_pm_merge() {
    let pr = Pr::new("comments-author");
    let comments = json!([comment(
        1,
        "NexusFlow-UG",
        "2026-11-27T10:00:00Z",
        verdict_body(&pr.verdict())
    )]);

    let report = pr.decide_with(Some(&comments), None, None).unwrap();

    assert_pm_merges(&report);
}

#[test]
fn a_verdict_comment_only_by_a_stranger_goes_to_the_owner() {
    let pr = Pr::new("comments-stranger");
    let comments = json!([comment(
        1,
        "someone",
        "2026-11-27T10:00:00Z",
        verdict_body(&pr.verdict())
    )]);

    let report = pr.decide_with(Some(&comments), None, None).unwrap();

    assert_waits_for_owner(&report);
    assert_eq!(codes(&report), ["verdict_invalid"]);
}

#[test]
fn the_newest_verdict_comment_for_an_older_head_goes_to_the_owner_though_an_older_one_matches() {
    let mut pr = Pr::new("comments-older-head");
    let old_head = pr.head.clone();
    pr.push("src/lib.rs", "// three\n");
    // The older comment names the current head; the newer one still names the old head.
    let matching = pr.verdict();
    let mut stale = pr.verdict();
    stale["head"] = json!(old_head);
    let comments = json!([
        comment(
            1,
            "nexusflow-ug",
            "2026-11-27T10:00:00Z",
            verdict_body(&matching)
        ),
        comment(
            2,
            "nexusflow-ug",
            "2026-11-27T11:00:00Z",
            verdict_body(&stale)
        ),
    ]);

    let report = pr.decide_with(Some(&comments), None, None).unwrap();

    assert_waits_for_owner(&report);
    assert_eq!(codes(&report), ["verdict_invalid"]);
}

#[test]
fn a_base_that_is_no_commit_cannot_be_decided() {
    let pr = Pr::new("no-base");
    let verdict = pr.verdict().to_string();
    let result = nexus_flow_seam::Engine::merge_gate_decide(
        &pr.path,
        "no-such-ref",
        &pr.head,
        None,
        Some(verdict.as_bytes()),
        None,
    );
    assert!(
        matches!(result, Err(Error::Git(GitError::NotACommit { .. }))),
        "{result:?}"
    );
}

#[test]
fn a_head_that_is_no_commit_cannot_be_decided() {
    let pr = Pr::new("no-head");
    let verdict = pr.verdict().to_string();
    let result = nexus_flow_seam::Engine::merge_gate_decide(
        &pr.path,
        &pr.base,
        "0000000000000000000000000000000000000001",
        None,
        Some(verdict.as_bytes()),
        None,
    );
    assert!(
        matches!(result, Err(Error::Git(GitError::NotACommit { .. }))),
        "{result:?}"
    );
}

#[test]
fn comments_and_a_verdict_together_cannot_be_decided() {
    let pr = Pr::new("both-inputs");
    let comments = json!([]);
    let result = pr.decide_with(Some(&comments), Some(&pr.verdict()), None);
    assert!(
        matches!(result, Err(Error::MergeGateInput(_))),
        "{result:?}"
    );
}

#[test]
fn the_same_inputs_give_the_same_report() {
    let pr = Pr::new("same-inputs");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "stored-format")["outcome"] = json!("applies");

    let first = pr.decide(&verdict);
    let second = pr.decide(&verdict);

    assert_eq!(first, second);
}

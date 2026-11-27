//! NEX-172: a verdict is valid only in format 1, for the base and head given, with exactly one
//! entry per rule, an outcome its rule's effect allows, a reason, and a decision exactly for
//! `decided` whose path is a file at the base commit (RFC 0010, "The verdict", "`nxs merge-gate
//! decide`" step 2). Every problem is named; unknown fields are ignored.

mod common;

use common::{Repo, GATE, GATE_PATH, GENESIS, GENESIS_PATH};
use nexus_flow_merge_gate::rules::{rules_at, Rule};
use nexus_flow_merge_gate::verdict::{
    validate_verdict, Decision, Outcome, RuleVerdict, Verdict, VerdictError,
};
use serde_json::{json, Value};

const RFC_PATH: &str = "docs/rfc/0010-merge-gate.md";
const HEAD: &str = "9a07bd0000000000000000000000000000000000";

struct Fixture {
    repo: Repo,
    base: String,
    rules: Vec<Rule>,
}

fn fixture(test: &str) -> Fixture {
    let repo = Repo::new(test);
    repo.write(GENESIS_PATH, GENESIS);
    repo.write(GATE_PATH, GATE);
    repo.write(RFC_PATH, "# RFC 0010\n");
    let base = repo.commit("rules");
    let rules = rules_at(&repo.path, &base).unwrap().rules;
    Fixture { repo, base, rules }
}

/// A valid verdict for the fixture's four rules.
fn verdict(base: &str) -> Value {
    json!({
        "format": 1,
        "base": base,
        "head": HEAD,
        "verdicts": [
            { "rule": "genesis.1", "outcome": "decided", "reason": "Adds the command RFC 0010 names.",
              "decision": { "path": RFC_PATH, "section": "nxs merge-gate rules" } },
            { "rule": "genesis.2", "outcome": "not_concerned", "reason": "No stored format changes." },
            { "rule": "decided-design", "outcome": "met", "reason": "Only implements RFC 0010." },
            { "rule": "stored-format", "outcome": "not_concerned", "reason": "No stored format changes." }
        ]
    })
}

fn validate(f: &Fixture, verdict: &Value) -> Result<Verdict, VerdictError> {
    validate_verdict(
        &f.repo.path,
        &f.base,
        HEAD,
        &f.rules,
        verdict.to_string().as_bytes(),
    )
}

/// The problems of an invalid verdict.
fn problems(f: &Fixture, verdict: &Value) -> Vec<String> {
    match validate(f, verdict) {
        Err(VerdictError::Invalid(problems)) => problems,
        other => panic!("expected an invalid verdict, got {other:?}"),
    }
}

fn assert_one_problem_naming(problems: &[String], words: &[&str]) {
    assert_eq!(problems.len(), 1, "{problems:?}");
    for word in words {
        assert!(problems[0].contains(word), "{word:?} in {problems:?}");
    }
}

fn entry<'a>(v: &'a mut Value, rule: &str) -> &'a mut Value {
    v["verdicts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["rule"] == rule)
        .unwrap()
}

#[test]
fn a_valid_verdict_is_read_in_rule_order() {
    let f = fixture("valid");
    let mut v = verdict(&f.base);
    v["verdicts"].as_array_mut().unwrap().reverse();
    v["note"] = json!("unknown fields are ignored");

    let read = validate(&f, &v).unwrap();

    assert_eq!(read.base, f.base);
    assert_eq!(read.head, HEAD);
    assert_eq!(
        read.verdicts,
        vec![
            RuleVerdict {
                rule: "genesis.1".into(),
                outcome: Outcome::Decided,
                reason: "Adds the command RFC 0010 names.".into(),
                decision: Some(Decision {
                    path: RFC_PATH.into(),
                    section: Some("nxs merge-gate rules".into()),
                }),
            },
            RuleVerdict {
                rule: "genesis.2".into(),
                outcome: Outcome::NotConcerned,
                reason: "No stored format changes.".into(),
                decision: None,
            },
            RuleVerdict {
                rule: "decided-design".into(),
                outcome: Outcome::Met,
                reason: "Only implements RFC 0010.".into(),
                decision: None,
            },
            RuleVerdict {
                rule: "stored-format".into(),
                outcome: Outcome::NotConcerned,
                reason: "No stored format changes.".into(),
                decision: None,
            },
        ]
    );
}

#[test]
fn a_missing_rule_makes_it_invalid() {
    let f = fixture("missing-rule");
    let mut v = verdict(&f.base);
    v["verdicts"].as_array_mut().unwrap().remove(1);

    assert_one_problem_naming(&problems(&f, &v), &["genesis.2"]);
}

#[test]
fn an_unknown_rule_makes_it_invalid() {
    let f = fixture("unknown-rule");
    let mut v = verdict(&f.base);
    v["verdicts"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "rule": "genesis.3", "outcome": "not_concerned", "reason": "Nothing." }));

    assert_one_problem_naming(&problems(&f, &v), &["genesis.3"]);
}

#[test]
fn a_rule_judged_twice_makes_it_invalid() {
    let f = fixture("twice");
    let mut v = verdict(&f.base);
    let again = v["verdicts"][1].clone();
    v["verdicts"].as_array_mut().unwrap().push(again);

    assert_one_problem_naming(&problems(&f, &v), &["genesis.2"]);
}

#[test]
fn an_outcome_of_the_other_effect_makes_it_invalid() {
    let f = fixture("wrong-outcome");
    let mut v = verdict(&f.base);
    entry(&mut v, "stored-format")["outcome"] = json!("met");
    entry(&mut v, "decided-design")["outcome"] = json!("not_concerned");

    let problems = problems(&f, &v);

    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems
        .iter()
        .any(|p| p.contains("stored-format") && p.contains("met")));
    assert!(problems
        .iter()
        .any(|p| p.contains("decided-design") && p.contains("not_concerned")));
}

#[test]
fn an_unknown_outcome_makes_it_invalid() {
    let f = fixture("unknown-outcome");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.2")["outcome"] = json!("fine");

    assert_one_problem_naming(&problems(&f, &v), &["genesis.2", "fine"]);
}

#[test]
fn every_owner_and_pm_outcome_is_allowed_for_its_effect() {
    let f = fixture("outcomes");
    for (owner, pm) in [("applies", "not_met"), ("unsure", "unsure")] {
        let mut v = verdict(&f.base);
        entry(&mut v, "stored-format")["outcome"] = json!(owner);
        entry(&mut v, "decided-design")["outcome"] = json!(pm);
        assert!(validate(&f, &v).is_ok(), "{owner} / {pm}");
    }
}

#[test]
fn a_missing_decision_path_makes_it_invalid() {
    let f = fixture("missing-path");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.1")["decision"]["path"] = json!("docs/rfc/0099-nothing.md");

    assert_one_problem_naming(
        &problems(&f, &v),
        &["genesis.1", "docs/rfc/0099-nothing.md"],
    );
}

#[test]
fn a_decision_path_is_read_at_the_base_not_the_working_tree() {
    let f = fixture("path-at-base");
    f.repo.write("docs/rfc/0011-new.md", "# Not committed\n");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.1")["decision"]["path"] = json!("docs/rfc/0011-new.md");

    assert_one_problem_naming(&problems(&f, &v), &["docs/rfc/0011-new.md"]);
}

#[test]
fn a_decision_path_that_is_a_directory_makes_it_invalid() {
    let f = fixture("path-dir");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.1")["decision"]["path"] = json!("docs/rfc");

    assert_one_problem_naming(&problems(&f, &v), &["docs/rfc"]);
}

#[test]
fn decided_without_a_decision_makes_it_invalid() {
    let f = fixture("no-decision");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.1")
        .as_object_mut()
        .unwrap()
        .remove("decision");

    assert_one_problem_naming(&problems(&f, &v), &["genesis.1", "decision"]);
}

#[test]
fn a_decision_for_another_outcome_makes_it_invalid() {
    let f = fixture("extra-decision");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.2")["decision"] = json!({ "path": RFC_PATH });

    assert_one_problem_naming(&problems(&f, &v), &["genesis.2", "decision"]);
}

#[test]
fn an_empty_or_missing_reason_makes_it_invalid() {
    let f = fixture("reason");
    let mut v = verdict(&f.base);
    entry(&mut v, "genesis.2")["reason"] = json!("  ");
    entry(&mut v, "stored-format")
        .as_object_mut()
        .unwrap()
        .remove("reason");

    let problems = problems(&f, &v);

    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("genesis.2"), "{problems:?}");
    assert!(problems[1].contains("stored-format"), "{problems:?}");
}

#[test]
fn another_base_or_head_makes_it_invalid() {
    let f = fixture("base-head");
    let mut v = verdict(&f.base);
    v["base"] = json!("4f1c2e0000000000000000000000000000000000");
    v["head"] = json!("ffffff0000000000000000000000000000000000");

    let problems = problems(&f, &v);

    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("base"), "{problems:?}");
    assert!(problems[1].contains("head"), "{problems:?}");
}

#[test]
fn a_format_other_than_1_makes_it_invalid() {
    let f = fixture("format");
    for format in [json!(2), json!("1"), Value::Null] {
        let mut v = verdict(&f.base);
        v["format"] = format;
        assert_one_problem_naming(&problems(&f, &v), &["format"]);
    }
}

#[test]
fn a_verdict_that_is_not_a_json_object_is_invalid() {
    let f = fixture("not-json");
    for text in ["not json", "[]", "\u{feff}{}"] {
        let result = validate_verdict(&f.repo.path, &f.base, HEAD, &f.rules, text.as_bytes());
        assert!(
            matches!(result, Err(VerdictError::Invalid(ref p)) if p.len() == 1),
            "{text:?}: {result:?}"
        );
    }
}

#[test]
fn an_entry_without_a_rule_id_makes_it_invalid() {
    let f = fixture("no-rule-id");
    let mut v = verdict(&f.base);
    v["verdicts"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "outcome": "met", "reason": "x" }));

    assert_one_problem_naming(&problems(&f, &v), &["5"]);
}

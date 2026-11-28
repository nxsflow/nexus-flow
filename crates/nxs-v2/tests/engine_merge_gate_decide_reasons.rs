//! NEX-173: with a valid verdict, `Engine::merge_gate_decide` collects every reason the pull
//! request needs the owner (RFC 0010, "`nxs merge-gate decide`", steps 3 and 4): CODEOWNERS on a
//! changed path, both paths of a rename included, an owner's request for changes, an `unsure`
//! rule, and an `owner` rule that `applies`, which no `pm` rule overrides. The rules are the base's,
//! so a rule loosened only on the head changes nothing.

mod decide_common;

use decide_common::{
    assert_pm_merges, assert_waits_for_owner, codes, entry, Pr, GATE, GATE_PATH, GENESIS,
    GENESIS_PATH,
};
use nexus_flow_seam::merge_gate::Reason;
use serde_json::json;

#[test]
fn an_owner_rule_that_applies_goes_to_the_owner_though_a_pm_rule_is_met() {
    let pr = Pr::new("applies");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "stored-format")["outcome"] = json!("applies");
    entry(&mut verdict, "stored-format")["reason"] = json!("Adds a field to the ticket record.");

    let report = pr.decide(&verdict);

    assert_waits_for_owner(&report);
    assert_eq!(
        report.reasons,
        [Reason::Applies {
            rule: "stored-format".to_string(),
            reason: "Adds a field to the ticket record.".to_string(),
        }]
    );
}

#[test]
fn an_unsure_owner_rule_goes_to_the_owner() {
    let pr = Pr::new("unsure-owner");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "genesis.1")["outcome"] = json!("unsure");

    let report = pr.decide(&verdict);

    assert_waits_for_owner(&report);
    assert_eq!(codes(&report), ["unsure"]);
}

#[test]
fn an_unsure_pm_rule_goes_to_the_owner_though_the_default_is_pm() {
    let gate = GATE.replace("default = \"owner\"", "default = \"pm\"");
    let pr = Pr::with_base("unsure-pm", &[(GENESIS_PATH, GENESIS), (GATE_PATH, &gate)]);
    let mut verdict = pr.verdict();
    entry(&mut verdict, "decided-design")["outcome"] = json!("unsure");

    let report = pr.decide(&verdict);

    assert_waits_for_owner(&report);
    assert_eq!(
        report.reasons,
        [Reason::Unsure {
            rule: "decided-design".to_string(),
            reason: "Only implements RFC 0010.".to_string(),
        }]
    );
}

#[test]
fn every_reason_is_collected_in_order() {
    let mut pr = Pr::new("every-reason");
    pr.push("factory/CONFIG.md", "# CONFIG\n");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "genesis.1")["outcome"] = json!("applies");
    entry(&mut verdict, "decided-design")["outcome"] = json!("unsure");
    entry(&mut verdict, "stored-format")["outcome"] = json!("applies");
    let reviews = json!([review("cabcookie", "CHANGES_REQUESTED", &pr.base, "2026-11-27T10:00:00Z")]);

    let report = pr.decide_with(None, Some(&verdict), Some(&reviews)).unwrap();

    assert_waits_for_owner(&report);
    assert_eq!(
        codes(&report),
        ["codeowners", "changes_requested", "applies", "unsure", "applies"]
    );
}

#[test]
fn without_a_met_pm_rule_the_default_owner_decides() {
    let pr = Pr::new("default-owner");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "decided-design")["outcome"] = json!("not_met");

    let report = pr.decide(&verdict);

    assert_waits_for_owner(&report);
    assert_eq!(report.reasons, [Reason::Default]);
}

#[test]
fn the_default_pm_lets_pm_merge_without_a_met_pm_rule() {
    let gate = GATE.replace("default = \"owner\"", "default = \"pm\"");
    let pr = Pr::with_base("default-pm", &[(GENESIS_PATH, GENESIS), (GATE_PATH, &gate)]);
    let mut verdict = pr.verdict();
    entry(&mut verdict, "decided-design")["outcome"] = json!("not_met");

    let report = pr.decide(&verdict);

    assert_pm_merges(&report);
    assert_eq!(report.reasons, [Reason::Default]);
}

#[test]
fn a_changed_path_under_codeowners_goes_to_the_owner() {
    let mut pr = Pr::new("codeowners");
    pr.push(".github/workflows/ci.yml", "on: push\n");
    pr.push("factory/roles/pm.md", "# pm\n");

    let report = pr.decide(&pr.verdict());

    assert_waits_for_owner(&report);
    assert_eq!(
        report.reasons,
        [Reason::Codeowners {
            paths: vec![
                ".github/workflows/ci.yml".to_string(),
                "factory/roles/pm.md".to_string()
            ]
        }]
    );
}

#[test]
fn the_old_path_of_a_rename_under_codeowners_goes_to_the_owner() {
    let mut pr = Pr::with_base(
        "codeowners-rename",
        &[
            (GENESIS_PATH, GENESIS),
            (GATE_PATH, GATE),
            (".github/release.sh", "echo release\n"),
        ],
    );
    pr.git(&["mv", ".github/release.sh", "scripts/release.sh"]);
    pr.head = pr.commit("move the release script out of .github");

    let report = pr.decide(&pr.verdict());

    assert_waits_for_owner(&report);
    assert_eq!(
        report.reasons,
        [Reason::Codeowners {
            paths: vec![".github/release.sh".to_string()]
        }]
    );
}

#[test]
fn codeowners_is_read_from_the_base_not_the_head() {
    let mut pr = Pr::new("codeowners-base");
    pr.write(".github/CODEOWNERS", "/docs/ @cabcookie\n");
    pr.push("factory/roles/pm.md", "# pm\n");

    let report = pr.decide(&pr.verdict());

    assert_waits_for_owner(&report);
    let Reason::Codeowners { paths } = &report.reasons[0] else {
        panic!("expected codeowners, got {report:?}");
    };
    assert_eq!(paths, &[".github/CODEOWNERS", "factory/roles/pm.md"]);
}

#[test]
fn changes_on_the_base_after_the_merge_base_are_not_the_pull_requests() {
    let mut pr = Pr::new("merge-base");
    pr.git(&["checkout", "-q", "main"]);
    pr.write(".github/workflows/ci.yml", "on: push\n");
    pr.base = pr.commit("the base moves on");
    pr.git(&["checkout", "-q", "pr"]);

    let report = pr.decide(&pr.verdict());

    assert_pm_merges(&report);
}

#[test]
fn a_rule_loosened_only_on_the_head_still_holds() {
    let mut pr = Pr::with_base(
        "loosened",
        &[(GENESIS_PATH, GENESIS), (GATE_PATH, GATE), (".github/CODEOWNERS", "")],
    );
    let loose = GATE.replace("default = \"owner\"", "default = \"pm\"");
    let loose = &loose[..loose.find("[[rule]]\nid = \"stored-format\"").unwrap()];
    pr.push(GATE_PATH, loose);
    let mut verdict = pr.verdict();
    entry(&mut verdict, "stored-format")["outcome"] = json!("applies");

    let report = pr.decide(&verdict);

    assert_waits_for_owner(&report);
    assert_eq!(codes(&report), ["applies"]);

    // A verdict written for the head's rules misses the base's rule.
    verdict["verdicts"].as_array_mut().unwrap().remove(3);
    let report = pr.decide(&verdict);
    assert_waits_for_owner(&report);
    assert_eq!(codes(&report), ["verdict_invalid"]);
}

#[test]
fn the_description_stays_within_140_characters() {
    let mut gate = GATE.to_string();
    for n in 0..30 {
        gate.push_str(&format!(
            "\n[[rule]]\nid = \"a-rather-long-rule-id-number-{n}\"\neffect = \"owner\"\ntext = \"t\"\n"
        ));
    }
    let pr = Pr::with_base("description", &[(GENESIS_PATH, GENESIS), (GATE_PATH, &gate)]);
    let mut verdict = pr.verdict();
    for n in 0..30 {
        verdict["verdicts"].as_array_mut().unwrap().push(json!({
            "rule": format!("a-rather-long-rule-id-number-{n}"), "outcome": "unsure", "reason": "r"
        }));
    }

    let report = pr.decide(&verdict);

    assert_waits_for_owner(&report);
    assert!(report.description.chars().count() <= 140, "{}", report.description);
    assert!(report.description.starts_with("Waits for the owner"), "{}", report.description);
}

/// A review of GitHub's "list reviews for a pull request" response.
pub fn review(login: &str, state: &str, commit: &str, submitted_at: &str) -> serde_json::Value {
    json!({ "user": { "login": login }, "state": state, "commit_id": commit, "submitted_at": submitted_at })
}

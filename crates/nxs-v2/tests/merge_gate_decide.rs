//! NEX-173: `nxs merge-gate decide --base <sha> --head <sha> [--comments <file> | --verdict
//! <file>] [--reviews <file>]` (RFC 0010, "`nxs merge-gate decide`"): the decision as JSON by
//! default and Markdown with `--md`, the same bytes for the same inputs, and exit 0 whenever it
//! decided, `owner` included. It exits non-zero only when it cannot decide. It opens no database,
//! and its result is `Engine::merge_gate_decide`'s.

mod decide_common;

use std::fs;
use std::path::Path;

use decide_common::{entry, Pr};
use nexus_flow_seam::Engine;
use nxs_v2::Outcome;
use serde_json::{json, Value};

/// Runs `nxs <args>` in the repository; a command that asks for the database's options fails.
fn nxs(pr: &Pr, args: &[&str]) -> Outcome {
    let argv = std::iter::once("nxs").chain(args.iter().copied());
    nxs_v2::run(argv, &pr.path, || {
        panic!("nxs merge-gate decide opens no database")
    })
}

fn stdout(outcome: &Outcome) -> String {
    String::from_utf8(outcome.stdout.clone()).unwrap()
}

fn json_of(outcome: &Outcome) -> Value {
    serde_json::from_slice(&outcome.stdout)
        .unwrap_or_else(|e| panic!("stdout is JSON ({e}): {}", stdout(outcome)))
}

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Writes `value` into `name` in the repository's `.git/`, outside the working tree.
fn input(pr: &Pr, name: &str, value: &Value) -> String {
    let path = pr.path.join(".git").join(name);
    fs::write(&path, value.to_string()).unwrap();
    path.to_string_lossy().into_owned()
}

/// A pull request that touches CODEOWNERS and whose verdict says a rule applies.
fn owner_pr(test: &str) -> (Pr, String, String) {
    let mut pr = Pr::new(test);
    pr.push(".github/workflows/ci.yml", "on: push\n");
    let mut verdict = pr.verdict();
    entry(&mut verdict, "stored-format")["outcome"] = json!("applies");
    entry(&mut verdict, "stored-format")["reason"] = json!("Adds a field to the ticket record.");
    let comments = json!([{
        "id": 7, "user": { "login": "nexusflow-ug" }, "created_at": "2026-11-27T10:00:00Z",
        "body": format!("<!-- nexus-flow: merge-gate-verdict -->\n```json\n{verdict}\n```\n")
    }]);
    let reviews = json!([{
        "user": { "login": "cabcookie" }, "state": "COMMENTED", "commit_id": pr.head,
        "submitted_at": "2026-11-27T11:00:00Z"
    }]);
    let comments = input(&pr, "comments.json", &comments);
    let reviews = input(&pr, "reviews.json", &reviews);
    (pr, comments, reviews)
}

#[test]
fn an_owner_decision_prints_the_rfcs_json_and_exits_zero() {
    let (pr, comments, reviews) = owner_pr("decide-json");

    let out = nxs(
        &pr,
        &[
            "merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--comments",
            &comments, "--reviews", &reviews,
        ],
    );

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty());
    assert_eq!(
        json_of(&out),
        json!({
            "base": pr.base,
            "head": pr.head,
            "decision": "owner",
            "reasons": [
                { "code": "codeowners", "paths": [".github/workflows/ci.yml"] },
                { "code": "applies", "rule": "stored-format", "reason": "Adds a field to the ticket record." }
            ],
            "owner_approved": false,
            "status": "pending",
            "description": "Waits for the owner: CODEOWNERS, stored-format"
        })
    );
}

#[test]
fn the_same_inputs_give_the_golden_bytes_every_time() {
    let (pr, comments, reviews) = owner_pr("decide-golden");
    let args = [
        "merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--comments", &comments,
        "--reviews", &reviews,
    ];

    let first = nxs(&pr, &args);
    let second = nxs(&pr, &args);
    let md: Vec<&str> = std::iter::once("--md").chain(args).collect();
    let md = nxs(&pr, &md);

    assert_eq!(stdout(&first), golden("merge-gate-decide.json"));
    assert_eq!(second.stdout, first.stdout);
    assert_eq!(md.code, 0);
    assert_eq!(stdout(&md), golden("merge-gate-decide.md"));
}

#[test]
fn the_output_is_the_seams_report() {
    let (pr, comments, reviews) = owner_pr("decide-seam");

    let out = nxs(
        &pr,
        &[
            "merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--comments",
            &comments, "--reviews", &reviews,
        ],
    );
    let report = Engine::merge_gate_decide(
        &pr.path,
        &pr.base,
        &pr.head,
        Some(&fs::read(&comments).unwrap()),
        None,
        Some(&fs::read(&reviews).unwrap()),
    )
    .unwrap();

    let json = json_of(&out);
    assert_eq!(json["decision"], report.decision.as_str());
    assert_eq!(json["status"], report.status.as_str());
    assert_eq!(json["description"], report.description.as_str());
    assert_eq!(json["reasons"].as_array().unwrap().len(), report.reasons.len());
}

#[test]
fn a_pm_decision_exits_zero_with_status_success() {
    let pr = Pr::new("decide-pm");
    let verdict = input(&pr, "verdict.json", &pr.verdict());

    let out = nxs(
        &pr,
        &["merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--verdict", &verdict],
    );

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let json = json_of(&out);
    assert_eq!(json["decision"], "pm");
    assert_eq!(json["status"], "success");
    assert_eq!(json["reasons"], json!([{ "code": "pm_rule", "rules": ["decided-design"] }]));
}

#[test]
fn a_comments_file_that_does_not_exist_is_no_valid_verdict() {
    let pr = Pr::new("decide-no-file");
    let missing = pr.path.join(".git/nothing.json");

    let out = nxs(
        &pr,
        &[
            "merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--comments",
            missing.to_str().unwrap(),
        ],
    );

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let json = json_of(&out);
    assert_eq!(json["decision"], "owner");
    assert_eq!(json["reasons"][0]["code"], "verdict_invalid");
    assert!(json["reasons"][0]["detail"].as_array().is_some_and(|d| !d.is_empty()));
}

#[test]
fn unreadable_rules_are_a_decision_and_exit_zero() {
    let pr = Pr::with_base("decide-unreadable", &[]);
    let verdict = input(&pr, "verdict.json", &pr.verdict());

    let out = nxs(
        &pr,
        &["merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--verdict", &verdict],
    );

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let json = json_of(&out);
    assert_eq!(json["decision"], "owner");
    assert_eq!(json["reasons"][0]["code"], "rules_unreadable");
    assert_eq!(
        json["reasons"][0]["errors"],
        json!([
            { "source": "factory/GENESIS.md", "code": "file_missing" },
            { "source": "factory/merge-gate/gate.toml", "code": "file_missing" }
        ])
    );
}

/// Asserts that `out` is the error object, with exit code 1 and nothing decided.
fn assert_cannot_decide(out: &Outcome) {
    assert_eq!(out.code, 1, "{}", stdout(out));
    let json = json_of(out);
    assert!(json["error"]["kind"].is_string(), "{json}");
    assert!(json.get("decision").is_none());
}

#[test]
fn a_missing_head_cannot_be_decided() {
    let pr = Pr::new("decide-no-head-flag");
    assert_cannot_decide(&nxs(&pr, &["merge-gate", "decide", "--base", &pr.base]));
}

#[test]
fn a_missing_base_cannot_be_decided() {
    let pr = Pr::new("decide-no-base-flag");
    assert_cannot_decide(&nxs(&pr, &["merge-gate", "decide", "--head", &pr.head]));
}

#[test]
fn a_head_that_is_no_commit_cannot_be_decided() {
    let pr = Pr::new("decide-head-no-commit");
    let out = nxs(&pr, &["merge-gate", "decide", "--base", &pr.base, "--head", "nowhere"]);
    assert_cannot_decide(&out);
    assert_eq!(json_of(&out)["error"]["kind"], "validation");
}

#[test]
fn comments_and_a_verdict_together_cannot_be_decided() {
    let pr = Pr::new("decide-both");
    let verdict = input(&pr, "verdict.json", &pr.verdict());
    let comments = input(&pr, "comments.json", &json!([]));
    let out = nxs(
        &pr,
        &[
            "merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--comments",
            &comments, "--verdict", &verdict,
        ],
    );
    assert_cannot_decide(&out);
}

#[test]
fn reviews_that_cannot_be_read_cannot_be_decided() {
    let pr = Pr::new("decide-bad-reviews");
    let verdict = input(&pr, "verdict.json", &pr.verdict());
    let reviews = input(&pr, "reviews.json", &json!({ "not": "a list" }));
    let missing = pr.path.join(".git/no-reviews.json");
    let base = ["merge-gate", "decide", "--base", &pr.base, "--head", &pr.head, "--verdict", &verdict];

    let bad: Vec<&str> = base.iter().copied().chain(["--reviews", &reviews]).collect();
    let gone: Vec<&str> = base
        .iter()
        .copied()
        .chain(["--reviews", missing.to_str().unwrap()])
        .collect();

    assert_cannot_decide(&nxs(&pr, &bad));
    assert_cannot_decide(&nxs(&pr, &gone));
}

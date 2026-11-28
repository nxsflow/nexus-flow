//! NEX-173: the owner's approval in `Engine::merge_gate_decide` (RFC 0010, "The owner's
//! approval"): it counts only on the head commit, only when it is an owner's latest deciding
//! review, and never for a login outside `owners`. An owner's latest request for changes, on any
//! commit, sends the pull request to the owner.

mod decide_common;

use decide_common::{assert_pm_merges, assert_waits_for_owner, codes, entry, Pr};
use nexus_flow_seam::merge_gate::Reason;
use nexus_flow_seam::Error;
use serde_json::{json, Value};

fn review(login: &str, state: &str, commit: &str, submitted_at: &str) -> Value {
    json!({ "user": { "login": login }, "state": state, "commit_id": commit, "submitted_at": submitted_at })
}

/// A pull request whose verdict sends it to the owner, decided with `reviews`.
fn owner_pr(test: &str) -> (Pr, Value) {
    let pr = Pr::new(test);
    let mut verdict = pr.verdict();
    entry(&mut verdict, "stored-format")["outcome"] = json!("applies");
    (pr, verdict)
}

#[test]
fn the_owners_approval_of_the_head_turns_the_status_green() {
    let (pr, verdict) = owner_pr("approved");
    let reviews = json!([review(
        "cabcookie",
        "APPROVED",
        &pr.head,
        "2026-11-27T10:00:00Z"
    )]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert_eq!(report.decision.as_str(), "owner");
    assert!(report.owner_approved);
    assert_eq!(report.status.as_str(), "success");
}

#[test]
fn the_owners_login_is_compared_without_regard_to_ascii_case() {
    let (pr, verdict) = owner_pr("approved-case");
    let reviews = json!([review(
        "CabCookie",
        "APPROVED",
        &pr.head,
        "2026-11-27T10:00:00Z"
    )]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert!(report.owner_approved);
}

#[test]
fn an_approval_of_an_earlier_commit_does_not_count() {
    let (pr, verdict) = owner_pr("approved-earlier");
    let reviews = json!([review(
        "cabcookie",
        "APPROVED",
        &pr.base,
        "2026-11-27T10:00:00Z"
    )]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert_waits_for_owner(&report);
}

#[test]
fn an_approval_by_a_login_outside_owners_does_not_count() {
    let (pr, verdict) = owner_pr("approved-stranger");
    let reviews = json!([review(
        "nexusflow-ug",
        "APPROVED",
        &pr.head,
        "2026-11-27T10:00:00Z"
    )]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert_waits_for_owner(&report);
}

#[test]
fn an_approval_later_dismissed_does_not_count() {
    let (pr, verdict) = owner_pr("approved-dismissed");
    let reviews = json!([
        review("cabcookie", "APPROVED", &pr.head, "2026-11-27T10:00:00Z"),
        review("cabcookie", "DISMISSED", &pr.head, "2026-11-27T11:00:00Z"),
    ]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert_waits_for_owner(&report);
}

#[test]
fn a_comment_after_an_approval_does_not_undo_it() {
    let (pr, verdict) = owner_pr("approved-commented");
    let reviews = json!([
        review("cabcookie", "APPROVED", &pr.head, "2026-11-27T10:00:00Z"),
        review("cabcookie", "COMMENTED", &pr.head, "2026-11-27T11:00:00Z"),
    ]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert!(report.owner_approved);
}

#[test]
fn the_latest_deciding_review_is_found_by_time_not_by_position() {
    let (pr, verdict) = owner_pr("approved-order");
    let reviews = json!([
        review(
            "cabcookie",
            "CHANGES_REQUESTED",
            &pr.head,
            "2026-11-27T11:00:00Z"
        ),
        review("cabcookie", "APPROVED", &pr.head, "2026-11-27T10:00:00Z"),
    ]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert_waits_for_owner(&report);
}

#[test]
fn at_the_same_time_the_later_position_is_the_latest() {
    let (pr, verdict) = owner_pr("approved-same-time");
    let reviews = json!([
        review(
            "cabcookie",
            "CHANGES_REQUESTED",
            &pr.head,
            "2026-11-27T10:00:00Z"
        ),
        review("cabcookie", "APPROVED", &pr.head, "2026-11-27T10:00:00Z"),
    ]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert!(report.owner_approved);
}

#[test]
fn an_owners_latest_request_for_changes_goes_to_the_owner_on_any_commit() {
    let pr = Pr::new("changes-requested");
    let reviews = json!([
        review("cabcookie", "APPROVED", &pr.base, "2026-11-27T10:00:00Z"),
        review(
            "cabcookie",
            "CHANGES_REQUESTED",
            &pr.base,
            "2026-11-27T11:00:00Z"
        ),
    ]);

    let report = pr
        .decide_with(None, Some(&pr.verdict()), Some(&reviews))
        .unwrap();

    assert_waits_for_owner(&report);
    assert_eq!(
        report.reasons,
        [Reason::ChangesRequested {
            by: "cabcookie".to_string()
        }]
    );
}

#[test]
fn a_request_for_changes_by_a_login_outside_owners_is_no_reason() {
    let pr = Pr::new("changes-requested-stranger");
    let reviews = json!([review(
        "someone",
        "CHANGES_REQUESTED",
        &pr.head,
        "2026-11-27T11:00:00Z"
    )]);

    let report = pr
        .decide_with(None, Some(&pr.verdict()), Some(&reviews))
        .unwrap();

    assert_pm_merges(&report);
}

#[test]
fn the_owners_approval_counts_without_a_valid_verdict() {
    let pr = Pr::new("approved-no-verdict");
    let reviews = json!([review(
        "cabcookie",
        "APPROVED",
        &pr.head,
        "2026-11-27T10:00:00Z"
    )]);

    let report = pr.decide_with(None, None, Some(&reviews)).unwrap();

    assert_eq!(codes(&report), ["verdict_invalid"]);
    assert!(report.owner_approved);
    assert_eq!(report.status.as_str(), "success");
}

#[test]
fn reviews_that_are_not_githubs_list_cannot_be_decided() {
    let pr = Pr::new("reviews-invalid");
    let reviews = json!([{ "user": { "login": "cabcookie" }, "state": "APPROVED" }]);

    let result = pr.decide_with(None, Some(&pr.verdict()), Some(&reviews));

    assert!(
        matches!(result, Err(Error::MergeGateInput(_))),
        "{result:?}"
    );
}

#[test]
fn a_pending_review_without_a_time_is_no_deciding_review() {
    let (pr, verdict) = owner_pr("review-pending");
    let reviews = json!([
        review("cabcookie", "APPROVED", &pr.head, "2026-11-27T10:00:00Z"),
        { "user": { "login": "cabcookie" }, "state": "PENDING", "commit_id": pr.head, "submitted_at": null },
        { "user": null, "state": "CHANGES_REQUESTED", "commit_id": pr.head, "submitted_at": "2026-11-27T12:00:00Z" },
    ]);

    let report = pr
        .decide_with(None, Some(&verdict), Some(&reviews))
        .unwrap();

    assert!(report.owner_approved);
}

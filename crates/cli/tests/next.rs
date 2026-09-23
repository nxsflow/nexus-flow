//! `nxf next` — the ready set ordered by the active plugin's declarative ranking (E2.11).

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

fn init(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

fn create(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["create", "--type", "bug", "--description", "d"];
    full.extend_from_slice(args);
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["id"].as_str().unwrap().to_string()
}

fn next_ids(dir: &Path) -> Vec<String> {
    let out = nxf()
        .args(["next", "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn claim(dir: &Path, id: &str) {
    nxf()
        .args(["claim", id])
        .current_dir(dir)
        .assert()
        .success();
}

fn set_plugin(dir: &Path, plugin: &str) {
    fs::write(
        dir.join(".nxs").join("config.toml"),
        format!("plugin = \"{plugin}\"\n"),
    )
    .unwrap();
}

#[test]
fn next_orders_by_priority_then_id() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), &["--title", "A", "--priority", "P3"]);
    let b = create(tmp.path(), &["--title", "B", "--priority", "P1"]);
    let c = create(tmp.path(), &["--title", "C", "--priority", "P1"]);
    // priority 1 first (b and c, tie broken by id), then priority 3 (a). Short ids
    // carry no timestamp, so the id tiebreak no longer matches creation order — derive
    // the expected pair order by sorting.
    let mut p1 = vec![b, c];
    p1.sort();
    let expected: Vec<String> = p1.into_iter().chain([a]).collect();
    assert_eq!(next_ids(tmp.path()), expected);
}

#[test]
fn ranking_is_config_driven() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // Same priority; differ only by due. Ranking must come from the plugin's keys,
    // not creation order (short ids carry no timestamp).
    let a = create(
        tmp.path(),
        &["--title", "A", "--priority", "P1", "--due", "2030-01-01"],
    );
    let b = create(
        tmp.path(),
        &["--title", "B", "--priority", "P1", "--due", "2020-01-01"],
    );

    // issue-tracker breaks the priority tie by due (earlier first): b, a.
    set_plugin(tmp.path(), "issue-tracker");
    assert_eq!(next_ids(tmp.path()), vec![b.clone(), a.clone()]);

    // personal-todo has no due key, so it falls through to the id tiebreaker. Short
    // ids aren't creation-ordered, so derive the expected order by sorting.
    set_plugin(tmp.path(), "personal-todo");
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(next_ids(tmp.path()), expected);
}

#[test]
fn in_progress_work_ranks_above_open_in_the_default_next() {
    // nexus-flow-qu2 + C4 (#916.3), as amended by the finish-first tiers: claimed work is part of
    // the DEFAULT `next` (no flag) and leads — a claimed, childless item is Tier 1 (finishable),
    // ahead of every open item regardless of priority.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let hi = create(tmp.path(), &["--title", "hi", "--priority", "P0"]); // highest open
    let claimed = create(tmp.path(), &["--title", "claimed", "--priority", "P4"]); // lowest
    claim(tmp.path(), &claimed);

    assert_eq!(next_ids(tmp.path()), vec![claimed, hi]);
}

#[test]
fn blocked_or_deferred_in_progress_work_is_excluded_from_next() {
    // A claimed item that is still blocked (or deferred) is not actionable, so it must NOT appear
    // in `next` at all — blocker/defer take precedence over the in_progress boost (qu2), and the
    // finish-first tiers change ordering only, never which items are actionable.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let blocker = create(tmp.path(), &["--title", "blocker", "--priority", "P2"]);
    let dependent = create(tmp.path(), &["--title", "dependent", "--priority", "P0"]);
    nxf()
        .args(["dep", "add", &dependent, &blocker])
        .current_dir(tmp.path())
        .assert()
        .success();
    claim(tmp.path(), &dependent); // claimed, but still blocked by the open blocker

    assert_eq!(
        next_ids(tmp.path()),
        vec![blocker],
        "a blocked in_progress item stays out of next entirely"
    );
}

// ---- `--limit` (6j6v.8pf2) -------------------------------------------------
//
// ONE truncation mechanism for the two callers that need one: `nxf next --limit <n>` and `prime`
// (which sets 15). The load-bearing requirement is not the cut — it is the DISCLOSURE: a list that
// shows 15 of 180 and looks like 15 of 15 makes a reader believe they are through.

/// The human `next` view, verbatim (framing blanks included).
fn next_human(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["next"];
    full.extend_from_slice(args);
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

/// The `next --json` payload, verbatim.
fn next_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = vec!["next"];
    full.extend_from_slice(args);
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

/// Five ready items, P0..P4 — a set big enough to cut and ranked deterministically.
fn five(dir: &Path) -> Vec<String> {
    (0..5)
        .map(|n| {
            create(
                dir,
                &["--title", &format!("t{n}"), "--priority", &format!("P{n}")],
            )
        })
        .collect()
}

#[test]
fn next_limit_truncates_the_human_list_and_names_the_untruncated_total() {
    // The DoD's central case: a cut list must not read like a complete one.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    five(tmp.path());

    let human = next_human(tmp.path(), &["--limit", "2"]);
    let rows = human.lines().filter(|l| l.contains("[bug]")).count();
    assert_eq!(rows, 2, "two rows shown: {human}");
    assert!(
        human.contains("showing 2 of 5"),
        "the human view names the untruncated total: {human}"
    );
}

#[test]
fn next_limit_reports_the_untruncated_total_as_its_own_json_field() {
    // A consumer must not have to infer truncation from the array's length.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    five(tmp.path());

    let v = next_json(tmp.path(), &["--limit", "2"]);
    assert_eq!(v["items"].as_array().unwrap().len(), 2, "two records: {v}");
    assert_eq!(v["total"].as_u64().unwrap(), 5, "true total named: {v}");
}

#[test]
fn next_without_limit_stays_a_bare_complete_array() {
    // The unchanged machine contract: no `--limit` ⇒ the same top-level array agents read today.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    five(tmp.path());

    let v = next_json(tmp.path(), &[]);
    assert_eq!(
        v.as_array().map(Vec::len),
        Some(5),
        "still a bare, complete array: {v}"
    );
}

#[test]
fn next_limit_wider_than_the_set_shows_everything_and_says_nothing() {
    // Nothing was cut, so there is nothing to disclose — the view stays exactly as it is without
    // `--limit`, and only the envelope (which the flag opts into) differs under `--json`.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    five(tmp.path());

    let human = next_human(tmp.path(), &["--limit", "50"]);
    assert_eq!(
        human,
        next_human(tmp.path(), &[]),
        "an uncut list is byte-identical to the unlimited view: {human}"
    );
    let v = next_json(tmp.path(), &["--limit", "50"]);
    assert_eq!(v["items"].as_array().unwrap().len(), 5, "all five: {v}");
    assert_eq!(v["total"].as_u64().unwrap(), 5, "total equals shown: {v}");
}

#[test]
fn next_limit_zero_is_a_count_only_answer_not_an_empty_board() {
    // PR #363 review, Test Quality #4: clap accepts `--limit 0`, so the behaviour is reachable and
    // belongs pinned. The honest reading of "show me none of them" is a disclosure with no rows —
    // never a bare empty list, which on this command reads as "nothing to work on".
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    five(tmp.path());

    let human = next_human(tmp.path(), &["--limit", "0"]);
    assert!(human.contains("showing 0 of 5"), "still discloses: {human}");
    let v = next_json(tmp.path(), &["--limit", "0"]);
    assert!(v["items"].as_array().unwrap().is_empty(), "no records: {v}");
    assert_eq!(v["total"].as_u64().unwrap(), 5, "total unaffected: {v}");
}

#[test]
fn next_limit_cuts_after_the_sort_order() {
    // `--sort id` is the flat, untiered order; the limit takes the head OF THAT order, so the
    // truncated set is the first n by id — not the first n by rank, then re-sorted.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let mut ids = five(tmp.path());
    ids.sort();

    let v = next_json(tmp.path(), &["--sort", "id", "--limit", "2"]);
    let shown: Vec<String> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(shown, ids[..2].to_vec(), "head of the id order: {v}");
    assert_eq!(
        v["total"].as_u64().unwrap(),
        5,
        "total is the whole set: {v}"
    );
}

#[test]
fn next_limit_cuts_after_the_label_filter() {
    // The total that gets disclosed is the FILTERED total (3 labelled of 5), never the board's —
    // a reader who filtered is told how much of their filter they are seeing.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let ids = five(tmp.path());
    for id in ids.iter().take(3) {
        nxf()
            .args(["label", "add", id, "ui"])
            .current_dir(tmp.path())
            .assert()
            .success();
    }

    let v = next_json(tmp.path(), &["--label", "ui", "--limit", "2"]);
    assert_eq!(v["items"].as_array().unwrap().len(), 2, "two shown: {v}");
    assert_eq!(
        v["total"].as_u64().unwrap(),
        3,
        "of the three labelled: {v}"
    );
}

// ---- legibility (6j6v.8bax) ------------------------------------------------

#[test]
fn next_emits_zero_escape_sequences_when_stdout_is_not_a_terminal() {
    // THE byte-stability guardrail at the seam that actually ships. The row styling is TTY-gated
    // inside `nxs_ui::style`, but the gate is only as good as the detection wired in front of it —
    // and a test harness, a pipe, CI and the trycmd goldens are all this same non-terminal case.
    // Asserted here on the real binary, because the unit tests inject their theme and cannot see
    // this wiring at all.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // An epic with a child, so the muted `↳` follow-up line is in the output too.
    let out = nxf()
        .args([
            "create",
            "--type",
            "epic",
            "--title",
            "E",
            "--priority",
            "P0",
            "--description",
            "d",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parent = serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    create(
        tmp.path(),
        &["--title", "child", "--priority", "P1", "--parent", &parent],
    );

    for args in [vec![], vec!["--limit", "1"]] {
        let out = next_human(tmp.path(), &args);
        assert!(
            !out.contains('\u{1b}'),
            "no escape byte with args {args:?}: {out:?}"
        );
    }
}

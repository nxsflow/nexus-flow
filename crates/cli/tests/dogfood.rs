//! Dogfood: a complete agent session run entirely through `nxf` (E2.18).
//!
//! This is the epic's acceptance gate — create → claim → fill design → close →
//! verify, with `--json` throughout, no fallback to any other tool. If this passes,
//! "≥ bd" is demonstrated rather than asserted. See docs/specs/E2-cli-mvp.md.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

/// Run an `nxf … --json` command, expect success, return parsed stdout.
fn run(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn full_agent_session_runs_entirely_on_nxf() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(dir, &["init", "--plugin", "issue-tracker"]);

    // 1. Bootstrap: an agent reads prime to learn the workflow.
    let primed = run(dir, &["prime"]);
    assert!(!primed["commands"].as_array().unwrap().is_empty());

    // 2. Plan the work: a project and a task that belongs to it.
    let project = run(
        dir,
        &[
            "create",
            "--type",
            "epic",
            "--title",
            "Ship E2",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let pid = project["id"].as_str().unwrap().to_string();
    let task = run(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Write the CLI",
            "--description",
            "d",
            "--priority",
            "P1",
            "--due",
            "2026-12-31",
            "--parent",
            &pid,
        ],
    );
    let tid = task["id"].as_str().unwrap().to_string();
    assert_eq!(task["belongs_to"], serde_json::json!(pid));

    // 3. A blocker task; the main task depends on it.
    let blocker = run(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Spec sign-off",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let bid = blocker["id"].as_str().unwrap().to_string();
    run(dir, &["dep", "add", &tid, &bid]);
    assert!(ids(&run(dir, &["blocked"])).contains(&tid));
    assert!(ids(&run(dir, &["next"])).contains(&bid));

    // 4. Finish the blocker; the unblocked task is ready again. Within the P1 band the `epic`
    //    container ranks ahead of the `bug` on type precedence (sp6.5: epic→bug→…), so the epic
    //    leads `next` and the now-actionable bug is the top leaf right behind it.
    run(dir, &["close", &bid, "--reason", "approved"]);
    let next_ids = ids(&run(dir, &["next"]));
    assert!(next_ids.contains(&tid), "the unblocked task is ready again");
    assert_eq!(next_ids[0], pid, "the P1 epic leads on type precedence");
    assert_eq!(
        next_ids[1], tid,
        "the unblocked bug is the next actionable item"
    );

    // 5. Work it: claim, record a worklog note, flesh out the design.
    let claimed = run(dir, &["claim", &tid, "--assignee", "dev"]);
    assert_eq!(claimed["status"], serde_json::json!("in_progress"));
    run(dir, &["note", "add", &tid, "started on the command layer"]);
    run(
        dir,
        &["update", &tid, "--set", "design=Thin CLI over the core."],
    );

    // 6. Search finds it by design content.
    assert!(ids(&run(dir, &["search", "thin cli"])).contains(&tid));

    // 7. Close it out and verify the final state via show.
    let closed = run(dir, &["close", &tid, "--reason", "done"]);
    assert_eq!(closed["status"], serde_json::json!("closed"));

    let shown = run(dir, &["show", &tid]);
    assert_eq!(shown["item"]["status"], serde_json::json!("closed"));
    assert_eq!(shown["item"]["closing_comment"], serde_json::json!("done"));
    assert_eq!(
        shown["item"]["design"],
        serde_json::json!("Thin CLI over the core.")
    );
    assert_eq!(shown["item"]["assignee"], serde_json::json!("dev"));
    // deps are structured {id, status} entries (nexus-flow-97b); the blocker was closed in step 4.
    assert_eq!(
        shown["deps"],
        serde_json::json!([{ "id": bid, "status": "closed" }])
    );
    assert_eq!(
        shown["notes"][0]["body"],
        serde_json::json!("started on the command layer")
    );

    // 8. The closed task has left the ready set.
    assert!(!ids(&run(dir, &["next"])).contains(&tid));
}

#[test]
fn json_output_is_deterministic_across_runs() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(dir, &["init", "--plugin", "issue-tracker"]);
    let v = run(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "stable",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let id = v["id"].as_str().unwrap().to_string();

    let a = nxf()
        .args(["show", &id, "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let b = nxf()
        .args(["show", &id, "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(a, b);
}

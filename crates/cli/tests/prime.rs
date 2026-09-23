//! `nxf prime` — the session bootstrap snapshot (E2.16), reshaped in the 0.2.1 batch:
//! plugin-determined purpose (6pu), a real next-top-7 instead of bare ready ids (e2z), a
//! dedicated create section with a priority recommendation (wt1), and a blocked section that
//! explains blockers and their leverage (bcj).

use assert_cmd::Command;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

fn init(dir: &Path) {
    init_with(dir, "issue-tracker");
}

fn init_with(dir: &Path, plugin: &str) {
    nxf()
        .args(["init", "--plugin", plugin])
        .current_dir(dir)
        .assert()
        .success();
}

fn create(dir: &Path, title: &str) -> String {
    create_args(dir, &["--title", title, "--priority", "P1"])
}

fn create_args(dir: &Path, args: &[&str]) -> String {
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

fn run(dir: &Path, args: &[&str]) {
    nxf().args(args).current_dir(dir).assert().success();
}

fn prime_json(dir: &Path) -> serde_json::Value {
    let out = nxf()
        .args(["prime", "--json"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn prime_human(dir: &Path) -> String {
    let out = nxf()
        .arg("prime")
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

fn next_ids(v: &serde_json::Value) -> Vec<String> {
    v["next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

/// Flatten prime's grouped `commands` (`[{group, items:[{name,summary}]}]`, nexus-flow-0b4)
/// into `(name, summary)` pairs in display order.
fn command_pairs(v: &serde_json::Value) -> Vec<(String, String)> {
    v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["items"].as_array().unwrap().iter())
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_string(),
                c["summary"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn string_array(v: &serde_json::Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

/// Close `id` with `reason`, stamping `closed_at` from a fixed instant so the recency order of the
/// "Recently Closed" section is deterministic (prime's `nxf()` does not pin the clock).
fn close_at(dir: &Path, id: &str, reason: &str, when: &str) {
    nxf()
        .args(["close", id, "--reason", reason])
        .env("NXF_NOW", when)
        .current_dir(dir)
        .assert()
        .success();
}

fn recently_closed(v: &serde_json::Value) -> &Vec<serde_json::Value> {
    v["recently_closed"].as_array().unwrap()
}

/// Set the `archived` cell directly (recap — and so prime's section — INCLUDES archived closed
/// items; the `archive` verb path is exercised elsewhere).
fn archive_cell(dir: &Path, id: &str, when: &str) {
    let mut store = workspace::discover(dir).unwrap().open_store().unwrap();
    store.set_field(id, "archived", Some(when.to_string()), "test");
}

// ---- r4kb: the "Recently Closed" recall section ----------------------------

#[test]
fn prime_json_recently_closed_carries_the_capped_record() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "Ship the thing");
    close_at(tmp.path(), &id, "shipped it", "2026-06-10T00:00:00Z");

    let v = prime_json(tmp.path());
    let rc = recently_closed(&v);
    assert_eq!(rc.len(), 1, "the one closed item appears");
    assert_eq!(rc[0]["id"].as_str().unwrap(), id);
    assert_eq!(rc[0]["title"].as_str().unwrap(), "Ship the thing");
    assert_eq!(rc[0]["closing_notes"].as_str().unwrap(), "shipped it");
    assert!(!rc[0]["notes_truncated"].as_bool().unwrap());
    assert_eq!(rc[0]["closed_at"].as_str().unwrap(), "2026-06-10T00:00:00Z");
    assert!(!rc[0]["archived"].as_bool().unwrap());
}

#[test]
fn prime_json_recently_closed_truncates_a_long_note_and_flags_it() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "Big writeup");
    let long = "x".repeat(400);
    close_at(tmp.path(), &id, &long, "2026-06-10T00:00:00Z");

    let v = prime_json(tmp.path());
    let notes = recently_closed(&v)[0]["closing_notes"].as_str().unwrap();
    assert_eq!(
        notes.chars().count(),
        281,
        "capped to 280 chars + the ellipsis"
    );
    assert!(notes.ends_with('…'));
    assert!(
        recently_closed(&v)[0]["notes_truncated"].as_bool().unwrap(),
        "the cap is reported in the record"
    );
}

#[test]
fn prime_recently_closed_is_top_3_newest_first() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "A");
    let b = create(tmp.path(), "B");
    let c = create(tmp.path(), "C");
    let d = create(tmp.path(), "D");
    close_at(tmp.path(), &a, "a", "2026-06-01T00:00:00Z");
    close_at(tmp.path(), &b, "b", "2026-06-02T00:00:00Z");
    close_at(tmp.path(), &c, "c", "2026-06-03T00:00:00Z");
    close_at(tmp.path(), &d, "d", "2026-06-04T00:00:00Z");

    let v = prime_json(tmp.path());
    let ids: Vec<&str> = recently_closed(&v)
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![d.as_str(), c.as_str(), b.as_str()],
        "Top-3, newest close first"
    );
}

#[test]
fn prime_human_renders_recently_closed_after_blocked_as_the_final_section() {
    // nxf xe2z retired `## Create` (and everything after it) from the human view, so Recently
    // Closed — still required to follow Blocked — is now the LAST section, not a middle one.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "Ship <v1>");
    close_at(tmp.path(), &id, "done & shipped", "2026-06-10T00:00:00Z");

    let out = prime_human(tmp.path());
    let sec = out.find("## Recently Closed").expect("section present");
    let blocked = out.find("## Blocked").expect("blocked present");
    assert!(blocked < sec, "the section follows Blocked");
    assert!(
        !out.contains("## Create"),
        "no section follows it any more: {out}"
    );
    // The line is the compact recall form; the title's markdown metacharacters are escaped.
    assert!(
        out.contains("Ship \\<v1\\> · done & shipped"),
        "compact `id · title · note` line, title escaped:\n{out}"
    );
}

#[test]
fn prime_recently_closed_empty_state() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "still open");

    let out = prime_human(tmp.path());
    assert!(
        out.contains("## Recently Closed (0)"),
        "count in the header"
    );
    assert!(
        out.contains("_Nothing closed recently._"),
        "empty-state line"
    );

    let v = prime_json(tmp.path());
    assert!(recently_closed(&v).is_empty(), "empty array in --json");
}

#[test]
fn prime_recently_closed_flags_an_archived_item() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "closed then archived");
    close_at(tmp.path(), &id, "done", "2026-06-10T00:00:00Z");
    archive_cell(tmp.path(), &id, "2026-06-20T00:00:00Z");

    // recap (and so the prime section) INCLUDES an archived closed item, and the record flags it.
    let v = prime_json(tmp.path());
    let rc = recently_closed(&v);
    assert_eq!(rc.len(), 1, "the archived-closed item still appears");
    assert!(
        rc[0]["archived"].as_bool().unwrap(),
        "archived: true for an actually-archived item"
    );
}

// ---- e2z: next replaces the bare ready id list -----------------------------

#[test]
fn prime_next_is_full_records_not_an_id_list() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "do me");

    let v = prime_json(tmp.path());
    assert!(v.get("ready").is_none(), "the bare `ready` id list is gone");
    let next = v["next"].as_array().expect("next is an array of records");
    let first = &next[0];
    // Canonical record fields, not a bare string.
    assert_eq!(first["id"].as_str().unwrap(), id);
    assert!(first["status"].is_string(), "full record carries status");
    assert!(first["type"].is_string(), "full record carries type");
    assert!(next_ids(&v).contains(&id));
}

#[test]
fn prime_next_truncates_to_fifteen_without_hiding_the_total() {
    // e2z + the finish-first tiers: the limit is 15 (raised from 7), so a started epic with ~10
    // children is not truncated mid-cluster and an agent never decides on a partial list. The
    // header still never hides the real total.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    for n in 0..17 {
        create(tmp.path(), &format!("task {n}"));
    }
    let v = prime_json(tmp.path());
    assert_eq!(v["next"].as_array().unwrap().len(), 15, "truncated to 15");
    assert_eq!(v["next_total"].as_u64().unwrap(), 17, "total not hidden");

    // Human header shows both the shown and the total count (as a Markdown heading).
    let human = prime_human(tmp.path());
    assert!(
        human.contains("## Next (showing 15 of 17)"),
        "human header shows truncation: {human}"
    );
}

#[test]
fn prime_next_shows_a_started_epics_whole_cluster_without_truncating_it() {
    // WHY the limit moved to 15 (the tiers' motivating case): an epic with 10 open children forms
    // one Tier-2 cluster of 11 rows. At the old limit of 7 the cluster was cut mid-way; now the
    // header and every one of its children survive the truncation together.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // issue-tracker only lets an `epic` be a parent, so the container is created as one.
    let out = nxf()
        .args([
            "create",
            "--type",
            "epic",
            "--title",
            "started epic",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let epic = serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut children = Vec::new();
    for n in 0..10 {
        let c = create(tmp.path(), &format!("child {n}"));
        nxf()
            .args(["update", &c, "--set", &format!("belongs_to={epic}")])
            .current_dir(tmp.path())
            .assert()
            .success();
        children.push(c);
    }
    nxf()
        .args(["claim", &epic])
        .current_dir(tmp.path())
        .assert()
        .success();

    let shown = next_ids(&prime_json(tmp.path()));
    assert_eq!(shown[0], epic, "the cluster header leads");
    for c in &children {
        assert!(
            shown.contains(c),
            "child {c} survives truncation: {shown:?}"
        );
    }
}

#[test]
fn prime_next_surfaces_in_progress_first() {
    // Inherits qu2: a claimed low-priority item leads prime's next over higher-priority open work.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let hi = create_args(tmp.path(), &["--title", "hi", "--priority", "P0"]);
    let claimed = create_args(tmp.path(), &["--title", "claimed", "--priority", "P4"]);
    run(tmp.path(), &["claim", &claimed]);

    let v = prime_json(tmp.path());
    assert_eq!(next_ids(&v), vec![claimed, hi]);
}

#[test]
fn prime_next_human_shows_bracketed_type() {
    // Inherits 9dy: the type column shows in prime's next block too.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "a bug");
    let human = prime_human(tmp.path());
    assert!(
        human.contains("[bug]"),
        "type bracketed in prime next: {human}"
    );
}

// ---- 6pu: plugin-determined purpose + sharpened workflow --------------------

#[test]
fn prime_purpose_is_plugin_determined() {
    let it = TempDir::new().unwrap();
    init_with(it.path(), "issue-tracker");
    let purpose = prime_json(it.path())["purpose"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        purpose.starts_with("nexus-flow is "),
        "leads with the identity: {purpose}"
    );
    assert!(
        purpose.contains("issue tracker"),
        "issue-tracker predicate present: {purpose}"
    );
    // The positioning prose and the redundant meta-sentence are gone (6pu).
    assert!(
        !purpose.contains("offline-first"),
        "no positioning prose: {purpose}"
    );
    assert!(
        !purpose.contains("maps this core onto its own vocabulary"),
        "redundant meta-sentence removed: {purpose}"
    );

    // A different plugin yields a different predicate (proves it reads the active config).
    let pt = TempDir::new().unwrap();
    init_with(pt.path(), "personal-todo");
    let purpose = prime_json(pt.path())["purpose"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        purpose.contains("to-do list"),
        "personal-todo predicate present: {purpose}"
    );
}

#[test]
fn prime_derivation_sentence_names_priority_and_keeps_invariant() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let purpose = prime_json(tmp.path())["purpose"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        purpose.contains("priority"),
        "derivation names priority: {purpose}"
    );
    assert!(
        purpose.contains("`next`") && purpose.contains("`blocked`"),
        "derivation begins from next/blocked: {purpose}"
    );
    assert!(
        purpose.contains("rather than stored"),
        "keeps the core invariant: {purpose}"
    );
}

#[test]
fn prime_workflow_starts_with_finding_next() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let rules = prime_json(tmp.path())["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    // 0b4: the tracker rule leads; finding next is the first *operational* rule (rules[1]).
    assert!(
        rules[0].contains("Track all work in nexus-flow"),
        "tracker rule leads: {rules:?}"
    );
    assert!(
        rules[1].contains("nxf next"),
        "first operational rule points at next: {rules:?}"
    );
    // bm0/§8: the mention-on-citation rule is still taught.
    assert!(
        rules
            .iter()
            .any(|r| r.contains("mention") && r.contains("short-id")),
        "mention rule retained: {rules:?}"
    );
    // 8qv.6: the stabilization convention — correct fields once after creation, then prefer
    // append-only notes over further field edits.
    assert!(
        rules
            .iter()
            .any(|r| r.contains("note add") && (r.contains("correct") || r.contains("stable"))),
        "stabilization convention taught: {rules:?}"
    );
    // 8qv.9: prime also carries the *why* — preserving the original intent builds the
    // intent-vs-outcome pair we learn from (the full principle lives in the guide).
    assert!(
        rules
            .iter()
            .any(|r| r.contains("intent-vs-outcome") && r.contains("preserved")),
        "intent-preservation rationale taught: {rules:?}"
    );
}

#[test]
fn prime_teaches_parent_vs_contributes_to_edge_choice() {
    // 07a.4: now that `parent` gates a child at its container's lifecycle (deferred/blocked/closed
    // propagate down), prime must teach the deliberate edge choice — `parent` rests the child when
    // its container rests, `contributes_to` is the non-gating association — so agents don't
    // over-constrain and silently hide work.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let rules = prime_json(tmp.path())["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let rule = rules
        .iter()
        .find(|r| r.contains("contributes_to"))
        .unwrap_or_else(|| panic!("a `parent` vs `contributes_to` rule is taught: {rules:?}"));
    assert!(
        rule.contains("parent"),
        "names the gating `parent` edge: {rule}"
    );
    assert!(
        rule.contains("rests") || rule.contains("gat"),
        "explains that the child rests when its container rests: {rule}"
    );
}

// ---- wt1: command order + dedicated create section -------------------------

#[test]
fn prime_command_reference_lists_next_before_ready() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let cmds: Vec<String> = command_pairs(&prime_json(tmp.path()))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let pos = |needle: &str| cmds.iter().position(|n| n.starts_with(needle)).unwrap();
    // `ready` was retired (C4 #916.3); `next` (the ready set) leads "Finding work", then `blocked`.
    assert!(
        pos("next") < pos("blocked"),
        "next before blocked: {cmds:?}"
    );
    assert!(
        cmds.iter().all(|n| !n.starts_with("ready")),
        "the retired `ready` entry is gone from the reference: {cmds:?}"
    );
    // create is only a short pointer in the list now (no full vocab example there).
    let create_entry = cmds.iter().find(|n| n.starts_with("create")).unwrap();
    assert!(
        !create_entry.contains("--type"),
        "create in the list is a pointer, not the full example: {create_entry}"
    );
}

#[test]
fn prime_has_a_create_section_recommending_priority() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = prime_json(tmp.path());
    let create = &v["create"];
    let example = create["example"].as_str().expect("create example string");
    assert!(
        example.contains("--type"),
        "example shows --type: {example}"
    );
    assert!(
        example.contains("--priority"),
        "example shows --priority: {example}"
    );
    let rec = create["recommendation"]
        .as_str()
        .expect("recommendation string");
    assert!(
        rec.to_lowercase().contains("priority"),
        "recommendation is about priority: {rec}"
    );

    // nxf xe2z: the human view no longer has a dedicated `## Create` section — the SAME
    // `create.example` string (with `--priority`) is folded into `## How work moves` instead, and
    // the recommendation sentence stays JSON-only (see the `create` field above).
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("## Create"),
        "the dedicated Create section is gone from the human view: {human}"
    );
    assert!(
        human.contains("## How work moves"),
        "How work moves replaces it: {human}"
    );
    assert!(
        human.contains(example),
        "the create example appears verbatim in How work moves: {human}"
    );
}

#[test]
fn prime_create_example_is_plugin_specific() {
    // issue-tracker: epic|bug|… vocab + P0–P4 range.
    let it = TempDir::new().unwrap();
    init_with(it.path(), "issue-tracker");
    let ex = prime_json(it.path())["create"]["example"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        ex.contains("epic") && ex.contains("bug"),
        "issue-tracker vocab in example: {ex}"
    );

    // personal-todo: project|todo vocab — a different active config yields a different example.
    let pt = TempDir::new().unwrap();
    init_with(pt.path(), "personal-todo");
    let ex = prime_json(pt.path())["create"]["example"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        ex.contains("project") && ex.contains("todo"),
        "personal-todo vocab in example: {ex}"
    );
}

// ---- bcj: blocked section explains blockers + leverage ----------------------

#[test]
fn prime_blocked_shows_open_blockers_with_leverage_count() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // C blocks both A and B (leverage 2); E blocks only D (leverage 1).
    let a = create(tmp.path(), "A");
    let b = create(tmp.path(), "B");
    let c = create(tmp.path(), "C");
    let d = create(tmp.path(), "D");
    let e = create(tmp.path(), "E");
    run(tmp.path(), &["dep", "add", &a, &c]);
    run(tmp.path(), &["dep", "add", &b, &c]);
    run(tmp.path(), &["dep", "add", &d, &e]);

    let v = prime_json(tmp.path());
    let blocked = v["blocked"].as_array().unwrap();
    // Each blocked entry carries structured blockers with a fan-out count.
    let blockers_of = |item: &str| -> serde_json::Value {
        blocked
            .iter()
            .find(|x| x["id"].as_str() == Some(item))
            .unwrap_or_else(|| panic!("{item} present in blocked {blocked:?}"))["blockers"]
            .clone()
    };
    let a_blockers = blockers_of(&a);
    assert_eq!(a_blockers[0]["id"].as_str().unwrap(), c);
    assert_eq!(a_blockers[0]["status"].as_str().unwrap(), "open");
    assert_eq!(
        a_blockers[0]["blocks_count"].as_u64().unwrap(),
        2,
        "C blocks two items"
    );
    assert_eq!(blockers_of(&d)[0]["blocks_count"].as_u64().unwrap(), 1);

    // Human view names the blocker and its leverage, and the high-leverage one is listed first.
    let human = prime_human(tmp.path());
    assert!(human.contains("blocks 2"), "leverage count shown: {human}");
    let high = human.find("blocks 2").unwrap();
    let low = human.find("blocks 1").unwrap();
    assert!(high < low, "high-leverage blocker surfaces first: {human}");
}

// ---- ykv: the prefix convention (nxf xe2z folded the dedicated `## IDs` heading away) ---------

/// A deterministic `nxf` (prefix `ab12`) so the rendered prefix value is assertable.
fn nxf_det(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1").current_dir(dir);
    c
}

#[test]
fn prime_names_the_local_prefix_and_the_id_convention() {
    // ykv: prime tells the agent its OWN prefix and the bare-suffix (local) / full-id (foreign)
    // convention — rendered at the CLI because the prefix value is workspace-specific. nxf xe2z:
    // the standalone `## IDs` heading is gone (folded into the plain paragraph right under the
    // title, task-2 brief's "was bleibt"), so this now checks for the paragraph's text instead of
    // a heading.
    let tmp = TempDir::new().unwrap();
    nxf_det(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let out = nxf_det(tmp.path())
        .arg("prime")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(out).unwrap();
    assert!(
        !human.contains("## IDs"),
        "the dedicated IDs heading is gone: {human}"
    );
    assert!(
        human.contains("this workspace's prefix is `ab12`"),
        "names the agent's own prefix: {human}"
    );
    assert!(
        human.contains("bare suffix") && human.contains("full id"),
        "states the local-suffix / foreign-full-id convention: {human}"
    );
    // Human-only by design: the prime --json keys are pinned elsewhere and must NOT gain an `ids`
    // key — so the JSON contract (and its tests) stay untouched.
    let v = nxf_det(tmp.path())
        .args(["prime", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&v).unwrap();
    assert!(
        v.get("ids").is_none() && v.get("prefix").is_none(),
        "the id hint is human-only; prime --json is unchanged: {v}"
    );
}

#[test]
fn prime_no_longer_teaches_the_id_capture_shortcut_in_the_human_view() {
    // 82h introduced the `-q`/`--jq` aside; nxf xe2z's task-2 brief retires it from the human
    // view outright ("die Feinheiten von `## Create`... und der `-q`-Hinweis" falls) — an agent
    // that wants it reads `nxf create --help`.
    let tmp = TempDir::new().unwrap();
    nxf_det(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    let out = nxf_det(tmp.path())
        .arg("prime")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(out).unwrap();
    assert!(
        !human.contains("-q") && !human.contains("--jq") && !human.contains("gh-ism"),
        "the -q/--jq aside is gone from the human view: {human}"
    );
}

// ---- unchanged guarantees --------------------------------------------------

#[test]
fn prime_is_deterministic() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "a");
    create(tmp.path(), "b");

    let once = prime_json(tmp.path());
    let twice = prime_json(tmp.path());
    assert_eq!(once, twice);
}

// ---- 7s6: the text output is valid, structured Markdown ---------------------

#[test]
fn prime_text_is_structured_markdown() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "do me");
    let human = prime_human(tmp.path());

    // Leads with an H1, and every surviving section is a Markdown heading — not a `key:` label.
    // (`trim_start` skips the deliberate output-framing blank line — nexus-flow-vwx.)
    assert!(
        human.trim_start().starts_with("# nexus-flow"),
        "leads with an H1 title: {human}"
    );
    for heading in [
        "## How work moves",
        "## Next",
        "## Blocked",
        "## Recently Closed",
    ] {
        assert!(
            human.contains(heading),
            "missing heading {heading}: {human}"
        );
    }
    // nxf xe2z shrank the human view: none of the pre-xe2z sections survive it (they are still in
    // `prime --json`, covered by `prime_json_top_level_keys_mirror_the_md` below), and none of the
    // old hand-set plain-text labels nor the pre-0b4 headings survive either.
    for legacy in [
        "workflow:",
        "create:",
        "commands:",
        "next (",
        "blocked (",
        "## Workflow",
        "> **Context Recovery",
        "## Core Rules",
        "## Create",
        "## IDs",
        "## Essential Commands",
        "## Common Workflows",
        "## Sync",
        "## Session close",
    ] {
        assert!(
            !human.contains(legacy),
            "legacy label {legacy:?} still present: {human}"
        );
    }
}

#[test]
fn prime_text_next_items_are_markdown_bullets_with_code_span_ids() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let id = create(tmp.path(), "do me");
    let human = prime_human(tmp.path());
    // The next row is a real Markdown bullet whose id is a code span (7s6), shown by its bare
    // local suffix (ykv).
    let bare = id.split_once('.').unwrap().1;
    assert!(
        human.lines().any(|l| l.starts_with(&format!("- `{bare}`"))),
        "next item rendered as a bullet with a code-span id: {human}"
    );
}

#[test]
fn prime_no_longer_surfaces_the_create_longtext_fine_print_in_the_human_view() {
    // 7s6 originally put the escaping-free long-text input (Epic 95d) right in the create
    // section so an agent that never reads --help still saw it. nxf xe2z's task-2 brief retires
    // it from the human view ("die Feinheiten von `## Create`... STDIN-Pipe, `--description-file`,
    // `create --json -`" falls) — it is still `create.long_text_hint` in `--json` (untouched),
    // an agent that wants it now reads `nxf create --help`.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("--description-file") && !human.contains("nxf create --json -"),
        "the create long-text fine print is gone from the human view: {human}"
    );
    let v = prime_json(tmp.path());
    let hint = v["create"]["long_text_hint"]
        .as_str()
        .expect("long_text_hint string");
    assert!(
        hint.contains("--description-file"),
        "the fine print survives in --json, unchanged: {hint}"
    );
}

#[test]
fn prime_json_top_level_keys_are_unchanged_by_the_human_view_shrink() {
    // vux: prime --json is the full, stable canonical record — the exhaustive key set, no more.
    // **Superseded by nxf xe2z:** the human view (Markdown) is no longer a 1:1 mirror of this —
    // it now renders only a curated subset (see the doc comment atop the `else` branch in
    // `crates/cli/src/commands/mod.rs::prime`) so the SessionStart hook clears its byte ceiling.
    // `--json` itself is untouched by that shrink: every key below still appears, unmoved, because
    // it is the contract the MCP server and embedding apps read. `sync` is present only when bound
    // (covered separately), so an unbound workspace omits it.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "x");
    let v = prime_json(tmp.path());
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "blocked",
            "commands",
            "context_recovery",
            "create",
            "next",
            "next_total",
            "purpose",
            "recently_closed",
            "rules",
            "session_close",
            "workflows",
        ],
        "unbound prime --json keys mirror the MD sections (no `sync` until bound): {v}"
    );
}

#[test]
fn prime_json_next_is_lean_not_full_records() {
    // vux: prime's `next` carries only the displayed fields in CANONICAL form, not every ticket
    // field — full records stay on `nxf next --json`. Keeps the bootstrap record small and
    // plugin-independent (ordinal priority / core tokens, not vocabulary labels).
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "do me"); // created with --priority P1
    let v = prime_json(tmp.path());
    let item = &v["next"][0];
    let mut keys: Vec<&str> = item
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["id", "parent", "priority", "status", "title", "type"],
        "prime next item is lean (displayed fields + the #916.7 parent join): {item}"
    );
    assert_eq!(
        item["priority"], "1",
        "canonical ordinal, not the `P1` label: {item}"
    );
    assert_eq!(
        item["type"], "bug",
        "the declared type, stored verbatim: {item}"
    );
    assert!(
        item.get("description").is_none() && item.get("design").is_none(),
        "no long-text bloat in the bootstrap record: {item}"
    );
}

#[test]
fn prime_context_recovery_survives_only_in_json() {
    // 0b4 originally rendered a bd-style context-recovery note, adapted to nxf, right under the
    // title. nxf xe2z's task-2 brief retires the line itself from the human view ("die 'Context
    // Recovery'-Zeile" falls) — `report.context_recovery` and its `--json` field are UNTOUCHED.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("Context Recovery"),
        "the blockquote line is gone from the human view: {human}"
    );
    // Mirrors into the JSON as a plain field, unchanged. The recovery hint points the agent at
    // the umbrella `nxs prime`, never the per-module `nxf prime` (nexus-flow-fyr).
    let v = prime_json(tmp.path());
    let cr = v["context_recovery"]
        .as_str()
        .expect("context_recovery string");
    assert!(cr.contains("nxs prime"), "names `nxs prime`: {cr}");
    assert!(!cr.contains("nxf prime"), "not the per-module verb: {cr}");
}

#[test]
fn prime_core_rules_lead_with_the_tracker_rule() {
    // 0b4: Core Rules lead with "track all work in nexus-flow" before the operational rules.
    // nxf xe2z retires the `## Core Rules` heading and eight of its nine entries from the human
    // view; the tracker-lead rule's GIST survives there instead, folded into the opening
    // paragraph's "single source of truth" sentence (task-2 brief's "was bleibt") — `report.rules`
    // and `--json`'s `rules` array are untouched and still lead with the rule verbatim.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("## Core Rules"),
        "the heading is gone from the human view: {human}"
    );
    assert!(
        human.contains("the single source of truth for this project's work"),
        "the tracker rule's gist survives in the opening paragraph: {human}"
    );
    let rules = string_array(&prime_json(tmp.path()), "rules");
    assert!(
        rules[0].contains("Track all work in nexus-flow"),
        "tracker rule is first in --json, unchanged: {rules:?}"
    );
    // The original find-next rule is preserved further down in --json.
    assert!(
        rules
            .iter()
            .any(|r| r.contains("Find what to work on next: `nxf next`.")),
        "operational rules preserved in --json: {rules:?}"
    );
}

#[test]
fn prime_essential_commands_survive_only_in_json() {
    // 0b4: the flat command table became grouped subsections. nxf xe2z retires the whole
    // `## Essential Commands` block from the human view (task-2 brief) — `report.commands` and
    // `--json`'s `commands` array are untouched.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = prime_json(tmp.path());
    let groups: Vec<String> = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["group"].as_str().unwrap().to_string())
        .collect();
    assert!(
        groups.iter().any(|g| g == "Finding work"),
        "commands are grouped in --json, unchanged: {groups:?}"
    );
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("## Essential Commands") && !human.contains("### Finding work"),
        "the section is gone from the human view: {human}"
    );
}

#[test]
fn prime_common_workflows_recipes_survive_only_in_json() {
    // 0b4: bd-style multi-step recipes, nxf-native (no git, no sync noise). nxf xe2z retires the
    // whole `## Common Workflows` block from the human view (task-2 brief) — `report.workflows`
    // and `--json`'s `workflows` array are untouched. The claim-step example an agent would have
    // found there now lives in `## How work moves` instead (a different literal line).
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let v = prime_json(tmp.path());
    let names: Vec<String> = v["workflows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "Starting work"),
        "workflows present in --json, unchanged: {names:?}"
    );
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("## Common Workflows"),
        "the section is gone from the human view: {human}"
    );
    assert!(
        human.contains("## How work moves") && human.contains("nxf claim <id>"),
        "the claim step lives on in the new cheatsheet instead: {human}"
    );
}

// ---- smz: sync hints + the lifecycle bracket, now JSON-only (nxf xe2z) -----

#[test]
fn prime_sync_and_session_close_survive_only_in_json() {
    // smz originally rendered `## Sync` (bound-only) and `## Session close` in the human view.
    // nxf xe2z's task-2 brief drops both from the human view unconditionally — `report.sync` and
    // `report.session_close`, and their `--json` fields, are untouched; the bound-gating behavior
    // itself is covered at the JSON level by `prime_session_close_sync_step_and_key_only_when_bound`
    // (this file) and `prime_sync_hint_is_present_only_when_bound` (`crates/facade/tests/read.rs`).
    let tmp = TempDir::new().unwrap();
    init(tmp.path());

    let unbound = prime_human(tmp.path());
    assert!(
        !unbound.contains("## Sync")
            && !unbound.contains("nxs sync run")
            && !unbound.contains("## Session close"),
        "no sync or session-close section unbound: {unbound}"
    );

    // Binding leaves `.nxs/sync.toml`; the human view still shows neither section.
    std::fs::write(
        tmp.path().join(".nxs").join("sync.toml"),
        "stream_id = \"s1\"\n",
    )
    .unwrap();
    let bound = prime_human(tmp.path());
    assert!(
        !bound.contains("## Sync")
            && !bound.contains("nxs sync run")
            && !bound.contains("## Session close"),
        "still no sync or session-close section once bound: {bound}"
    );

    // --json keeps both fields, gated on `bound`, exactly as before.
    let v = prime_json(tmp.path());
    assert!(
        v.get("sync").is_some(),
        "the sync key still appears bound, in --json: {v}"
    );
    assert!(
        string_array(&v, "session_close")
            .iter()
            .any(|s| s.contains("nxs sync run")),
        "the session_close sync step still appears bound, in --json: {v}"
    );
}

#[test]
fn prime_session_close_sync_step_and_key_only_when_bound() {
    // vux: the bound-only sync content mirrors in both views — the `session_close` sync step and
    // the top-level `sync` key appear only when `.nxs/sync.toml` exists.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());

    let unbound = prime_json(tmp.path());
    assert!(
        unbound.get("sync").is_none(),
        "no sync key unbound: {unbound}"
    );
    assert!(
        !string_array(&unbound, "session_close")
            .iter()
            .any(|s| s.contains("nxs sync run")),
        "no sync step in session_close unbound: {unbound}"
    );

    std::fs::write(
        tmp.path().join(".nxs").join("sync.toml"),
        "stream_id = \"s1\"\n",
    )
    .unwrap();
    let bound = prime_json(tmp.path());
    assert!(
        bound.get("sync").is_some(),
        "sync key appears bound: {bound}"
    );
    assert!(
        string_array(&bound, "session_close")
            .iter()
            .any(|s| s.contains("nxs sync run")),
        "sync step appears in session_close bound: {bound}"
    );
}

#[test]
fn prime_json_session_close_steps_are_adapted_to_nxf_not_bd() {
    // smz: the lifecycle bracket teaches noting unfinished work, closing with a reason, and a
    // non-prescriptive version-control reminder — adapted to nxf, NOT bd's literal git commands.
    // nxf xe2z moved this content's only reader from the human view to `--json` alone (the human
    // view no longer renders `## Session close` at all); the content itself is untouched, so this
    // now asserts on `report.session_close` directly instead of the retired heading's text.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let steps = string_array(&prime_json(tmp.path()), "session_close");
    let blob = steps.join(" ");
    assert!(
        !blob.to_lowercase().contains("git "),
        "no git steps copied from bd: {steps:?}"
    );
    assert!(
        blob.contains("nxf note add"),
        "teaches noting unfinished work: {steps:?}"
    );
    assert!(
        blob.contains("--reason"),
        "teaches closing with a reason: {steps:?}"
    );
    assert!(
        blob.to_lowercase().contains("version control"),
        "hybrid version-control reminder present: {steps:?}"
    );
}

#[test]
fn prime_text_reshapes_the_purpose_lede_for_xe2z() {
    // Our purpose lede (bd has none) survived the earlier Markdown rework (smz). nxf xe2z
    // reshapes ITS wording specifically: the human view now opens with `` `nxf` is … `` (the SAME
    // plugin-declared tagline `--json`'s "nexus-flow is …" `purpose` field carries — see
    // `read::prime_tagline`, shared by both), not the old full purpose+derivation sentence
    // verbatim. The find-next rule (and 4 of `report.rules`'s other 8 entries) no longer appears
    // in the human view at all — `--json`'s `rules` array still carries it, covered by
    // `prime_core_rules_lead_with_the_tracker_rule` above.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let human = prime_human(tmp.path());
    assert!(
        human.contains("`nxf` is a software issue tracker"),
        "reshaped purpose lede present: {human}"
    );
    assert!(
        !human.contains("nexus-flow is a software issue tracker"),
        "the old full sentence is gone from the human view: {human}"
    );
    assert!(
        !human.contains("Find what to work on next: `nxf next`."),
        "the find-next rule no longer appears verbatim in the human view: {human}"
    );
    // Same predicate string either way — the two views diverge only in grammatical subject.
    let v = prime_json(tmp.path());
    let purpose = v["purpose"].as_str().unwrap();
    assert!(
        purpose.starts_with("nexus-flow is a software issue tracker"),
        "--json's purpose field is untouched: {purpose}"
    );
}

#[test]
fn prime_dep_reference_explains_direction_unambiguously() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let pairs = command_pairs(&prime_json(tmp.path()));
    let (name, summary) = pairs
        .iter()
        .find(|(name, _)| name.starts_with("dep add"))
        .expect("a dep add entry exists");
    let blob = format!("{name} {summary}").to_lowercase();
    assert!(
        blob.contains("depend"),
        "names the depends-on relation: {blob}"
    );
    assert!(blob.contains("block"), "names which side blocks: {blob}");
}

#[test]
fn prime_human_empty_board_renders_zero_count_sections_with_placeholders() {
    // Test Quality #1: the empty-`next`/empty-`blocked` render branches are otherwise never hit
    // — every other fixture (and the golden) pre-populates the board, so a regression to the
    // zero-count headings or placeholder text would pass all gates.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let human = prime_human(tmp.path());
    assert!(
        human.contains("## Next (0)"),
        "zero-count next heading: {human}"
    );
    assert!(
        human.contains("_Nothing ready — create work, or unblock something below._"),
        "empty-next placeholder: {human}"
    );
    assert!(
        human.contains("## Blocked (0)"),
        "zero-count blocked heading: {human}"
    );
    assert!(
        human.contains("_Nothing blocked._"),
        "empty-blocked placeholder: {human}"
    );
}

#[test]
fn prime_md_and_json_render_the_same_record_for_the_parts_that_still_mirror() {
    // Test Quality #3 originally cross-checked a full MD↔JSON 1:1 mirror (vux), asserting every
    // JSON section (context_recovery, commands, workflows, session_close, sync presence) also
    // appeared in the human view. **Superseded by nxf xe2z:** the human view is now a curated
    // SUBSET of the record, not a 1:1 mirror — those fields stay JSON-only by design (see the
    // doc comment atop the `else` branch in `crates/cli/src/commands/mod.rs::prime`). Their
    // absence from the human view is covered by the dedicated `..._survive_only_in_json` tests
    // above; THIS test keeps checking the one thing that still IS a genuine cross-view drift risk
    // — `create.example`, which is spliced live into `## How work moves` — plus the three data
    // sections (`next`/`blocked`/`recently_closed`), which the task-2 brief requires unchanged.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "A");
    let b = create(tmp.path(), "B");
    run(tmp.path(), &["dep", "add", &a, &b]); // A depends on B ⇒ A blocked, B ready
    let v = prime_json(tmp.path());
    let human = prime_human(tmp.path());

    let example = v["create"]["example"].as_str().unwrap();
    assert!(
        human.contains(example),
        "create.example appears verbatim in How work moves: {human}"
    );
    for item in v["next"].as_array().unwrap() {
        let id = item["id"].as_str().unwrap();
        let bare = id.split_once('.').unwrap().1;
        assert!(
            human.contains(&format!("`{bare}`")),
            "next item {id} appears in the MD Next section: {human}"
        );
    }
    for entry in v["blocked"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        let bare = id.split_once('.').unwrap().1;
        assert!(
            human.contains(&format!("`{bare}`")),
            "blocked item {id} appears in the MD Blocked section: {human}"
        );
    }
}

// ---- oxmu: the defer-vs-WAIT-chore working convention ----------------------

#[test]
fn prime_teaches_defer_is_calendar_only_and_waiting_is_a_wait_chore() {
    // oxmu: DEFER is for a real calendar date only (a placeholder "someday, once X ships" date is
    // an anti-pattern); waiting on an external DELIVERY is modelled as an open WAIT chore the
    // dependents block on, and closing it (with the delivered version in the reason) releases the
    // chain. The convention rides `report.rules` (unchanged, --json still carries the full worked
    // example) — nxf xe2z is why it now rides the human view's `**Three rules...**` bullet list
    // instead of a `## Core Rules` heading, condensed to its essential shape with a `nxf guide`
    // pointer for the worked example.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let rules = string_array(&prime_json(tmp.path()), "rules");
    let rule = rules
        .iter()
        .find(|r| r.contains("WAIT") && r.to_lowercase().contains("defer"))
        .unwrap_or_else(|| panic!("a defer-vs-WAIT-chore rule is taught in prime: {rules:?}"));
    assert!(
        rule.to_lowercase().contains("calendar") || rule.to_lowercase().contains("date"),
        "defer is framed as a real calendar date: {rule}"
    );
    assert!(
        rule.contains("chore"),
        "waiting is modelled as a WAIT chore: {rule}"
    );
    let human = prime_human(tmp.path());
    assert!(
        !human.contains("## Core Rules"),
        "the old heading is gone: {human}"
    );
    assert!(
        human.contains("**Three rules `--help` will not teach you:**"),
        "the convention rides the new bold-intro bullet list instead: {human}"
    );
    assert!(
        human.contains("`nxf guide deferring-and-waiting`"),
        "the condensed rule points at the worked example: {human}"
    );
}

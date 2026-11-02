//! **The liveness report compares the policies** (nxf 6j6v.t7xp; design 6j6v.pszd section 6.4 and
//! test 11, as amended by the dispositions).
//!
//! Over a seeded `recovery_log` — three trial slots (answered after resume 2 under A, escalated
//! under B, noticed and answered after resume 1 under C), the boundary cases of the episode cut and
//! rows outside the window — `nxc liveness --json` is a golden document; the owner at the terminal
//! reads it, an agent session is refused; [`Engine::liveness`] returns the document the CLI printed
//! and refuses every reader but the owner; `--everywhere` reads every workspace the service home
//! attends and marks one it cannot open as `unreadable` while the others are complete.
//!
//! The boundary slots carry the letterless pair `2`/`false` on purpose: the episode cut does not
//! read the policy, and keeping them apart leaves the A, B and C summaries exactly the three trial
//! slots — rescues 1: 1, 2: 1, 3: 0 and one escalation between them. The same cut over rows the
//! ladder itself wrote under B is pinned in `the_ladder.rs`.

use assert_cmd::Command;
use nexus_chat::engine::Engine;
use nexus_chat::facade::{LivenessWindow, Reader};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nxs_test_support::PinHome;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const NOW: &str = "2026-10-26T12:00:00Z";
const FROM: &str = "2026-10-24T00:00:00Z";
const TO: &str = "2026-10-25T00:00:00Z";
const GOLDEN: &str = "tests/golden/liveness_report.json";

/// `(thread, role, at, action, attempt, outcome, dead_resumes, notice, silent_since, rest_secs)`.
type Seed = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u32,
    &'static str,
    u32,
    bool,
    Option<&'static str>,
    Option<i64>,
);

const A: (u32, bool) = (3, false);
const B: (u32, bool) = (1, false);
const C: (u32, bool) = (3, true);
const X: (u32, bool) = (2, false);

fn s(
    slot: (&'static str, &'static str),
    at: &'static str,
    action: &'static str,
    attempt: u32,
    outcome: &'static str,
    policy: (u32, bool),
    silent_since: Option<&'static str>,
) -> Seed {
    (
        slot.0,
        slot.1,
        at,
        action,
        attempt,
        outcome,
        policy.0,
        policy.1,
        silent_since,
        None,
    )
}

fn seeds() -> Vec<Seed> {
    let a = ("m-a", "ws/coder");
    let b = ("m-b", "ws/verifier");
    let c = ("m-c", "ws/coder");
    let d = ("m-d", "ws/coder");
    let e = ("m-e", "ws/coder");
    let f = ("m-f", "ws/coder");
    let g = ("m-g", "ws/coder");
    let h = ("m-h", "ws/coder");
    let z = ("m-z", "ws/coder");
    let sa = Some("2026-10-24T09:00:00Z");
    let sb = Some("2026-10-24T11:00:00Z");
    let sc = Some("2026-10-24T13:00:00Z");
    let mut confirmed = s(
        f,
        "2026-10-24T16:12:00Z",
        "confirmed",
        0,
        "granted",
        X,
        None,
    );
    confirmed.9 = Some(1500);
    vec![
        // Outside the window, before it: counted nowhere.
        s(
            z,
            "2026-10-23T23:59:59Z",
            "asked",
            0,
            "asked",
            A,
            Some("2026-10-23T23:49:59Z"),
        ),
        // A: answered after resume 2.
        s(a, "2026-10-24T09:10:00Z", "asked", 0, "asked", A, sa),
        s(
            a,
            "2026-10-24T09:15:00Z",
            "stopped",
            0,
            "stop_requested",
            A,
            sa,
        ),
        s(
            a,
            "2026-10-24T09:17:00Z",
            "continued",
            1,
            "triggered",
            A,
            None,
        ),
        s(
            a,
            "2026-10-24T09:32:00Z",
            "stopped",
            1,
            "stop_requested",
            A,
            sa,
        ),
        s(
            a,
            "2026-10-24T09:34:00Z",
            "continued",
            2,
            "triggered",
            A,
            None,
        ),
        s(a, "2026-10-24T09:40:00Z", "answered", 2, "replied", A, None),
        // B: escalated after its one resume.
        s(b, "2026-10-24T11:10:00Z", "asked", 0, "asked", B, sb),
        s(
            b,
            "2026-10-24T11:15:00Z",
            "stopped",
            0,
            "stop_requested",
            B,
            sb,
        ),
        s(
            b,
            "2026-10-24T11:17:00Z",
            "continued",
            1,
            "triggered",
            B,
            None,
        ),
        s(
            b,
            "2026-10-24T11:32:00Z",
            "stopped",
            1,
            "stop_requested",
            B,
            sb,
        ),
        s(
            b,
            "2026-10-24T11:32:00Z",
            "escalated",
            1,
            "escalated",
            B,
            sb,
        ),
        // C: noticed at the first stop, answered after resume 1.
        s(c, "2026-10-24T13:10:00Z", "asked", 0, "asked", C, sc),
        s(
            c,
            "2026-10-24T13:15:00Z",
            "stopped",
            0,
            "stop_requested",
            C,
            sc,
        ),
        s(c, "2026-10-24T13:15:00Z", "noticed", 0, "posted", C, None),
        s(
            c,
            "2026-10-24T13:17:00Z",
            "continued",
            1,
            "triggered",
            C,
            None,
        ),
        s(c, "2026-10-24T13:25:00Z", "answered", 1, "replied", C, None),
        // An overtaken nudge ending in `recovered` is closed; a later nudge is a second episode.
        s(
            d,
            "2026-10-24T14:10:00Z",
            "asked",
            0,
            "overtaken",
            X,
            Some("2026-10-24T14:00:00Z"),
        ),
        s(d, "2026-10-24T14:20:00Z", "recovered", 0, "life", X, None),
        s(
            d,
            "2026-10-24T14:50:00Z",
            "asked",
            0,
            "asked",
            X,
            Some("2026-10-24T14:40:00Z"),
        ),
        // Recovers after resume 1, goes silent again, escalates after 1 resume — never a resume 2.
        s(
            e,
            "2026-10-24T15:10:00Z",
            "asked",
            0,
            "asked",
            X,
            Some("2026-10-24T15:00:00Z"),
        ),
        s(
            e,
            "2026-10-24T15:15:00Z",
            "stopped",
            0,
            "stop_requested",
            X,
            Some("2026-10-24T15:00:00Z"),
        ),
        s(
            e,
            "2026-10-24T15:17:00Z",
            "continued",
            1,
            "triggered",
            X,
            None,
        ),
        s(e, "2026-10-24T15:19:00Z", "recovered", 1, "life", X, None),
        s(
            e,
            "2026-10-24T15:28:00Z",
            "asked",
            1,
            "asked",
            X,
            Some("2026-10-24T15:18:00Z"),
        ),
        s(
            e,
            "2026-10-24T15:33:00Z",
            "stopped",
            1,
            "stop_requested",
            X,
            Some("2026-10-24T15:18:00Z"),
        ),
        s(
            e,
            "2026-10-24T15:33:00Z",
            "escalated",
            1,
            "escalated",
            X,
            Some("2026-10-24T15:18:00Z"),
        ),
        // Confirm, then life: one episode ended by `recovered`.
        s(
            f,
            "2026-10-24T16:10:00Z",
            "asked",
            0,
            "asked",
            X,
            Some("2026-10-24T16:00:00Z"),
        ),
        confirmed,
        s(f, "2026-10-24T16:37:00Z", "recovered", 0, "life", X, None),
        // A reverted opener is an episode withdrawn; the nudge after it is a new one.
        s(
            g,
            "2026-10-24T17:05:00Z",
            "continued",
            0,
            "already_running",
            X,
            None,
        ),
        s(
            g,
            "2026-10-24T17:20:00Z",
            "asked",
            0,
            "asked",
            X,
            Some("2026-10-24T17:10:00Z"),
        ),
        // A `withdrawn` row ends the episode.
        s(
            h,
            "2026-10-24T18:10:00Z",
            "asked",
            0,
            "asked",
            X,
            Some("2026-10-24T18:00:00Z"),
        ),
        s(
            h,
            "2026-10-24T18:12:00Z",
            "withdrawn",
            0,
            "withdrawn",
            X,
            None,
        ),
        // Outside the window, at its end (the window is half-open): counted nowhere.
        s(
            z,
            "2026-10-25T00:00:00Z",
            "stopped",
            0,
            "stop_requested",
            A,
            Some("2026-10-23T23:49:59Z"),
        ),
    ]
}

fn workspace(config_extra: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    if !config_extra.is_empty() {
        let path = tmp.path().join(".nxs/config.toml");
        let mut cfg = std::fs::read_to_string(&path).unwrap_or_default();
        cfg.push('\n');
        cfg.push_str(config_extra);
        std::fs::write(&path, cfg).unwrap();
    }
    tmp
}

/// The workspace root as the CLI resolves it from its working directory (macOS hands `/var/…`
/// out as `/private/var/…`).
fn root(tmp: &TempDir) -> PathBuf {
    tmp.path().canonicalize().unwrap()
}

fn seed(tmp: &TempDir, rows: &[Seed]) {
    let store = Workspace::resolve(None, &root(tmp))
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    for (thread, role, at, action, attempt, outcome, dr, notice, silent, rest) in rows {
        store
            .connection()
            .execute(
                "INSERT INTO recovery_log(thread, role, at, action, attempt, session, outcome,
                     detail, policy_dead_resumes, policy_notice, silent_since, rest_secs)
                 VALUES (?1, ?2, ?3, ?4, ?5, 's-1', ?6, '', ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    thread,
                    role,
                    at,
                    action,
                    attempt,
                    outcome,
                    dr,
                    *notice as i64,
                    silent,
                    rest
                ],
            )
            .expect("seed a log row");
    }
}

/// The owner at the terminal: no agent session in the environment.
fn owner(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(root(tmp))
        .env_remove("NXC_SESSION")
        .env_remove("NXC_ACTOR")
        .env_remove("NXC_WORKER")
        .env_remove("NXC_DB")
        .env("NXC_NOW", NOW);
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    serde_json::from_str(stdout.trim()).expect("valid json")
}

fn report(tmp: &TempDir) -> Value {
    json_of(owner(tmp).args(["liveness", "--from", FROM, "--to", TO, "--json"]))
}

/// The document with the one machine-dependent value, the workspace's root, made stable.
fn stable(mut doc: Value) -> Value {
    for block in doc["workspaces"].as_array_mut().unwrap() {
        block["workspace"]["path"] = Value::from("<root>");
        block["workspace"]["name"] = Value::from("<name>");
    }
    doc
}

fn trial() -> TempDir {
    let tmp = workspace("[chat.liveness]\ndead_resume = 3\n");
    seed(&tmp, &seeds());
    tmp
}

fn summary_of<'v>(block: &'v Value, policy: (u32, bool)) -> &'v Value {
    block["summary"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["policy"] == serde_json::json!([policy.0, policy.1]))
        .unwrap_or_else(|| panic!("no summary for {policy:?} in {block}"))
}

#[test]
fn the_owners_report_is_the_golden_document() {
    let tmp = trial();
    let doc = stable(report(&tmp));
    if std::env::var_os("NXC_UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
    }
    let golden: Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("the golden document"))
            .expect("golden is json");
    assert_eq!(doc, golden);
}

#[test]
fn the_three_trial_slots_compare_as_designed() {
    let tmp = trial();
    let doc = report(&tmp);
    assert_eq!(
        (doc["from"].as_str(), doc["to"].as_str()),
        (Some(FROM), Some(TO))
    );
    let block = &doc["workspaces"][0];
    assert_eq!(block["state"], "read");

    let a = summary_of(block, A);
    assert_eq!(
        (a["letter"].as_str(), a["episodes"].as_u64()),
        (Some("A"), Some(1))
    );
    assert_eq!(a["rescued_by_resume"], serde_json::json!([0, 1, 0]));
    assert_eq!(a["max_silence_to_recovery_secs"], 2400);
    let b = summary_of(block, B);
    assert_eq!(
        (b["letter"].as_str(), b["escalated"].as_u64()),
        (Some("B"), Some(1))
    );
    assert_eq!(b["rescued_by_resume"], serde_json::json!([0, 0, 0]));
    assert_eq!(b["median_silence_to_escalation_secs"], 1920);
    let c = summary_of(block, C);
    assert_eq!(c["letter"], "C");
    assert_eq!(c["rescued_by_resume"], serde_json::json!([1, 0, 0]));
    assert_eq!(c["median_silence_to_recovery_secs"], 1500);
    let trial_rescues: Vec<u64> = (0..3)
        .map(|i| {
            [a, b, c]
                .iter()
                .map(|s| s["rescued_by_resume"][i].as_u64().unwrap())
                .sum()
        })
        .collect();
    assert_eq!(trial_rescues, vec![1, 1, 0]);

    // The resumes' outcomes, read from each slot's next row.
    let episodes = block["episodes"].as_array().unwrap();
    let of = |thread: &str| -> Vec<&Value> {
        episodes.iter().filter(|e| e["thread"] == thread).collect()
    };
    assert_eq!(
        of("m-a")[0]["resumes"],
        serde_json::json!([[1, "dead_again"], [2, "answered"]])
    );
    assert_eq!(of("m-a")[0]["end_attempt"], 2);
    assert_eq!(
        of("m-b")[0]["resumes"],
        serde_json::json!([[1, "dead_again"]])
    );
    assert_eq!(
        of("m-c")[0]["resumes"],
        serde_json::json!([[1, "answered"]])
    );
    assert_eq!(of("m-c")[0]["ended"], "answered");

    // The boundary cases.
    let ended = |thread: &str| -> Vec<String> {
        of(thread)
            .iter()
            .map(|e| e["ended"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ended("m-d"), vec!["recovered", "open"]);
    assert_eq!(ended("m-e"), vec!["recovered", "escalated"]);
    assert_eq!(
        of("m-e")[0]["resumes"],
        serde_json::json!([[1, "recovered"]])
    );
    assert_eq!(of("m-e")[1]["resumes"], serde_json::json!([]));
    assert_eq!(of("m-e")[1]["end_attempt"], 1);
    assert_eq!(ended("m-f"), vec!["recovered"]);
    assert_eq!(ended("m-g"), vec!["withdrawn", "open"]);
    assert_eq!(ended("m-h"), vec!["withdrawn"]);

    // Outside the window: neither counted nor an episode.
    assert!(of("m-z").is_empty(), "{block}");
    assert_eq!(block["counts"]["asked"], 10);
    assert_eq!(block["counts"]["stopped"], 7);
    assert_eq!(block["counts"]["noticed"], 1);
    assert_eq!(block["counts"]["withdrawn"], 1);
    assert_eq!(block["counts"]["recommissioned"], 0);
}

#[test]
fn the_config_findings_are_in_the_block() {
    let tmp = trial();
    let block = &report(&tmp)["workspaces"][0];
    assert_eq!(block["unknown_keys"], serde_json::json!(["dead_resume"]));
    let warnings = block["config_warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("dead_resume")),
        "{warnings:?}"
    );
    assert_eq!(block["letter_now"], "B");
}

#[test]
fn an_agent_session_is_refused_and_prints_no_report() {
    let tmp = trial();
    let out = owner(&tmp)
        .env("NXC_SESSION", "s-coder")
        .env("NXC_ACTOR", "coder")
        .args(["liveness", "--from", FROM, "--to", TO, "--json"])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let err: Value = serde_json::from_str(stdout.trim()).expect("an error envelope");
    assert_eq!(err["error"]["kind"], "forbidden", "{stdout}");
    assert!(err.get("workspaces").is_none(), "{stdout}");
}

#[test]
fn the_engine_refuses_every_reader_but_the_owner_and_answers_the_owner_with_the_clis_document() {
    let tmp = trial();
    let engine = Engine::open(None, &root(&tmp)).expect("open the engine");
    let window = Some(LivenessWindow { from: FROM, to: TO });
    for reader in [Reader::Handle("ws/coder"), Reader::NotOwner] {
        let err = engine.liveness(reader, NOW, window).unwrap_err();
        assert_eq!(err.kind.as_str(), "forbidden", "{reader:?}");
    }
    let at_the_seam = engine
        .liveness(Reader::Owner, NOW, window)
        .expect("the owner reads");
    assert_eq!(serde_json::to_value(&at_the_seam).unwrap(), report(&tmp));
}

#[test]
fn without_a_window_the_report_covers_the_last_seven_days() {
    let tmp = workspace("");
    let engine = Engine::open(None, &root(&tmp)).expect("open the engine");
    let doc = engine.liveness(Reader::Owner, NOW, None).unwrap();
    assert_eq!(
        (doc.from.as_str(), doc.to.as_str()),
        ("2026-10-19T12:00:00Z", NOW)
    );
}

fn register(home: &Path, entries: &[(&str, &Path)]) {
    let dir = home.join(".nexusflow");
    std::fs::create_dir_all(&dir).unwrap();
    let body: String = entries
        .iter()
        .map(|(name, path)| {
            format!(
                "[[workspace]]\nname = \"{name}\"\npath = \"{}\"\n\n",
                path.display()
            )
        })
        .collect();
    std::fs::write(dir.join("workspaces.toml"), body).unwrap();
}

fn everywhere(home: &TempDir, cwd: &TempDir) -> Value {
    let mut cmd = owner(cwd);
    cmd.pin_home(home.path());
    json_of(cmd.args([
        "liveness",
        "--everywhere",
        "--from",
        FROM,
        "--to",
        TO,
        "--json",
    ]))
}

#[test]
fn everywhere_reports_every_attended_workspace_with_its_own_policy() {
    let one = trial();
    let two = workspace("[chat.liveness]\ndead_resumes = 3\nnotice_at_first_stop = true\n");
    seed(&two, &seeds()[1..7]);
    let home = TempDir::new().unwrap();
    register(home.path(), &[("one", &root(&one)), ("two", &root(&two))]);

    let doc = everywhere(&home, &one);
    let blocks = doc["workspaces"].as_array().unwrap();
    assert_eq!(blocks.len(), 2, "{doc}");
    assert_eq!(blocks[0]["workspace"]["name"], "one");
    assert_eq!(blocks[1]["workspace"]["name"], "two");
    assert_eq!(
        (
            blocks[0]["letter_now"].as_str(),
            blocks[1]["letter_now"].as_str()
        ),
        (Some("B"), Some("C"))
    );
    assert_eq!(blocks[1]["episodes"].as_array().unwrap().len(), 1);
    // Each block is what that workspace's own report says, name aside.
    let mut alone = report(&one)["workspaces"][0].clone();
    alone["workspace"]["name"] = Value::from("one");
    assert_eq!(blocks[0], alone);
}

#[test]
fn everywhere_marks_a_store_it_cannot_open_unreadable_and_completes_the_others() {
    let one = trial();
    let two = trial();
    let home = TempDir::new().unwrap();
    register(home.path(), &[("one", &root(&one)), ("two", &root(&two))]);

    // A writer holds the second store: exclusive locking mode keeps the lock after the write.
    let holder = rusqlite::Connection::open(root(&two).join(".nxs/db.sqlite")).unwrap();
    holder
        .execute_batch(
            "PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE;
             UPDATE recovery_log SET detail = detail WHERE id = 1; COMMIT;",
        )
        .unwrap();

    let doc = everywhere(&home, &one);
    drop(holder);
    let blocks = doc["workspaces"].as_array().unwrap();
    assert_eq!(blocks.len(), 2, "{doc}");
    assert_eq!(blocks[0]["state"], "read");
    assert_eq!(blocks[0]["episodes"].as_array().unwrap().len(), 11, "{doc}");
    assert_eq!(blocks[1]["state"], "unreadable", "{doc}");
    assert_eq!(blocks[1]["workspace"]["name"], "two");
    assert!(!blocks[1]["error"].as_str().unwrap().is_empty());
}

#[test]
fn the_human_form_is_a_table_per_workspace() {
    let tmp = trial();
    let out = owner(&tmp)
        .args(["liveness", "--from", FROM, "--to", TO])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    for needle in [
        "policy now: B",
        "dead_resume",
        "A (3/false)",
        "rescued 0/1/0",
        "m-a",
    ] {
        assert!(text.contains(needle), "{needle:?} missing from:\n{text}");
    }
}

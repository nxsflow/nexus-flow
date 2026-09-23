//! The `nxm` classification + order acceptance suite (nexus-flow-6j6v.e0z6).
//!
//! A memory gained four things this ticket: a **category** (which section it belongs to), a
//! **reach** (how far it applies), **references** (which board items it is about) and an **order**
//! (where it reads within its section). This file pins the three claims the ticket is accepted on,
//! black-box over the built binary:
//!
//! 1. a new memory can be written with all of them, and filed afterwards without touching its text;
//! 2. a workspace written by the previous release migrates **silently** — the fields appear, and
//!    nothing a session sees changes;
//! 3. `nxm remember` needs no network, because nothing on the write path asks a model.
//!
//! The unit suites next door prove the CRDT half (each register converges on its own, insertion
//! order is a pure function of the log); this one proves the surface an agent actually touches.

use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-08-03T12:00:00Z";

/// An `nxm` invocation with actor/now/ids pinned, so records and stamps are byte-stable.
fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// A fresh memory workspace.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    tmp
}

fn stdout_of(dir: &Path, args: &[&str]) -> String {
    let out = nxm(dir).args(args).assert().success().get_output().clone();
    String::from_utf8(out.stdout).expect("utf8")
}

fn json_of(dir: &Path, args: &[&str]) -> Value {
    serde_json::from_str(stdout_of(dir, args).trim()).expect("valid json")
}

/// The workspace db the CLI writes.
fn db(dir: &Path) -> std::path::PathBuf {
    dir.join(".nxs/db.sqlite")
}

// ---- (1) writing and filing --------------------------------------------------------------------

#[test]
fn a_memory_can_be_written_with_a_category_a_reach_and_references() {
    let tmp = workspace();
    let dir = tmp.path();
    let rec = json_of(
        dir,
        &[
            "--json",
            "remember",
            "the two board levels are the load-bearing idea",
            "--key",
            "two-levels",
            "--category",
            "architecture",
            "--scope",
            "global",
            "--refs",
            "6j6v.e0z6,6j6v.5gvj",
            "--introduction",
            "in one line",
        ],
    );
    assert_eq!(rec["category"], "architecture");
    assert_eq!(rec["scope"], "global");
    assert_eq!(
        rec["refs"],
        serde_json::json!(["6j6v.5gvj", "6j6v.e0z6"]),
        "references are canonical — sorted and deduplicated — so two writers converge"
    );
    assert_eq!(
        rec["ordinal"],
        Value::Null,
        "writing never assigns a position"
    );
}

#[test]
fn classify_files_an_existing_memory_without_touching_its_text() {
    let tmp = workspace();
    let dir = tmp.path();
    nxm(dir)
        .args([
            "remember",
            "always run tests with -race",
            "--key",
            "race",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let out = stdout_of(dir, &["classify", "race", "--category", "rules"]);
    assert_eq!(
        out,
        "classified race\n  category: rules\n  scope:    project\n  refs:     none\n  \
         intro:    in one line\n",
        "the receipt reports the whole resulting filing, not just the change"
    );

    let rec = json_of(dir, &["--json", "recall", "race"]);
    assert_eq!(rec["category"], "rules");
    assert_eq!(rec["body"], "always run tests with -race", "text untouched");
    assert_eq!(
        rec["updated"], NOW,
        "`updated` dates the fact — filing it is not a change to the fact"
    );

    // An omitted flag is not a reset: setting the reach later leaves the category standing.
    nxm(dir)
        .args(["classify", "race", "--scope", "global"])
        .assert()
        .success();
    let rec = json_of(dir, &["--json", "recall", "race"]);
    assert_eq!(
        (&rec["category"], &rec["scope"]),
        (&json!("rules"), &json!("global"))
    );
}

#[test]
fn classify_needs_something_to_change_and_a_memory_to_change_it_on() {
    let tmp = workspace();
    let dir = tmp.path();
    nxm(dir)
        .args([
            "remember",
            "v",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    nxm(dir).args(["classify", "k"]).assert().failure();
    nxm(dir)
        .args(["classify", "ghost", "--category", "rules"])
        .assert()
        .failure();
    nxm(dir)
        .args(["classify", "k", "--category", "Two Words"])
        .assert()
        .failure();
    // Nothing stuck: the memory is still unfiled.
    assert_eq!(
        json_of(dir, &["--json", "recall", "k"])["category"],
        "unsorted"
    );
}

#[test]
fn reorder_stores_the_sequence_and_ordered_reads_serve_it() {
    let tmp = workspace();
    let dir = tmp.path();
    for key in ["rules", "intro", "arch"] {
        nxm(dir)
            .args([
                "remember",
                "body",
                "--key",
                key,
                "--category",
                "structure",
                "--introduction",
                "in one line",
            ])
            .assert()
            .success();
    }

    let out = stdout_of(dir, &["reorder", "intro", "arch", "rules"]);
    assert_eq!(
        out,
        "reordered 3 memories\n  1  intro\n  2  arch\n  3  rules\n"
    );

    let ordered = json_of(dir, &["--json", "memories", "--ordered"]);
    assert_eq!(
        ordered
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["intro", "arch", "rules"],
        "the stored order is what an ordered read serves"
    );
    // The default listing is untouched — still key order.
    let plain = json_of(dir, &["--json", "memories"]);
    assert_eq!(
        plain
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["arch", "intro", "rules"]
    );
}

#[test]
fn reads_narrow_by_category_and_by_reach() {
    let tmp = workspace();
    let dir = tmp.path();
    nxm(dir)
        .args([
            "remember",
            "what this is",
            "--key",
            "intro",
            "--category",
            "introduction",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(dir)
        .args([
            "remember",
            "always run tests with -race",
            "--key",
            "race",
            "--category",
            "rules",
            "--scope",
            "global",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(dir)
        .args([
            "remember",
            "loose note",
            "--key",
            "loose",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let keys = |args: &[&str]| -> Vec<String> {
        json_of(dir, args)
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["key"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        keys(&["--json", "memories", "--category", "rules"]),
        ["race"]
    );
    assert_eq!(
        keys(&["--json", "memories", "--scope", "project"]),
        ["intro", "loose"],
        "project is the reach every memory already had"
    );
    assert_eq!(
        keys(&["--json", "memories", "--category", "unsorted"]),
        ["loose"],
        "the reserved default is countable — the signal the judging migration works from"
    );
}

// ---- (2) the migration is not an event ---------------------------------------------------------

/// Roll a workspace back to the shape the previous release wrote: the pre-6j6v.e0z6 `memories`
/// columns and `PRAGMA user_version = 4`.
///
/// **Every column added after v4 has to be listed**, `introduction` (6j6v.xbnh, v6) included — the
/// stamp says v4, so `migrate` will re-add each of them, and a column left standing comes back as
/// `duplicate column name`. The op log is left exactly as it is: an older binary keeps a register op
/// it does not understand in the log and never folds it (§7), so a log carrying `introduction` ops
/// under a view that has no such column IS what the previous release would be holding.
fn downgrade_to_v4(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    for column in [
        "category",
        "category_v",
        "category_site",
        "scope",
        "scope_v",
        "scope_site",
        "refs",
        "refs_v",
        "refs_site",
        "ordinal",
        "ordinal_v",
        "ordinal_site",
        "created_v",
        "created_site",
        "introduction",
        "introduction_v",
        "introduction_site",
    ] {
        conn.execute_batch(&format!("ALTER TABLE memories DROP COLUMN {column};"))
            .unwrap_or_else(|e| panic!("dropping {column}: {e}"));
    }
    conn.pragma_update(None, "user_version", 4).unwrap();
}

#[test]
fn a_v4_workspace_migrates_without_a_session_noticing() {
    // The acceptance criterion, stated as a test: open a workspace from the previous release with
    // this binary and the SessionStart block it hands a session must be byte-identical. The fields
    // appear underneath; nothing above changes. That is what "the migration is deliberately not an
    // event" means.
    let tmp = workspace();
    let dir = tmp.path();
    for (key, body) in [
        ("auth-jwt", "auth uses JWT not sessions"),
        ("race-flag", "always run tests with the -race flag"),
        ("dolt", "phantom DBs hide in three places"),
    ] {
        nxm(dir)
            .args([
                "remember",
                body,
                "--key",
                key,
                "--introduction",
                "in one line",
            ])
            .assert()
            .success();
    }
    let expected_memories = stdout_of(dir, &["memories"]);

    downgrade_to_v4(&db(dir));

    assert_eq!(
        stdout_of(dir, &["memories"]),
        expected_memories,
        "what a v4 workspace stored still reads back unchanged after migrating"
    );

    // **And the introductions come BACK** (review of PR #381, Test Quality #2). The downgrade drops
    // the `introduction` column, so its values go with it — but the ops that wrote them stay in the
    // log, which is exactly the state a peer is in when it has received a foreign register op its
    // build could not fold yet. `MemoryStore::materialize_views` force-refolds when an open
    // migrated the schema, so the upgrade recovers every one of them from the log rather than
    // leaving three permanently empty lines behind.
    //
    // That is what makes 6j6v.e0z6's "the migration is not an event" promise still true here: the
    // registers underneath appear, the `memories` listing above is byte-identical, and the session
    // start says exactly what it said before. This is the guarantee observed end to end through the
    // real binary; the unit half is
    // `opening_a_pre_v6_workspace_force_refolds_a_deferred_introduction_op`.
    let block = stdout_of(dir, &["prime"]);
    for key in ["auth-jwt", "race-flag", "dolt"] {
        assert!(
            block.contains(&format!("- **{key}**: in one line")),
            "`{key}`'s introduction was recovered from the log:\n{block}"
        );
    }
    assert!(
        !block.contains("no introduction"),
        "…so the block names no gap at all — nothing was lost in the migration:\n{block}"
    );
}

#[test]
fn migrated_memories_carry_the_defaults_that_describe_what_they_already_did() {
    let tmp = workspace();
    let dir = tmp.path();
    nxm(dir)
        .args([
            "remember",
            "auth uses JWT",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    downgrade_to_v4(&db(dir));

    let rec = json_of(dir, &["--json", "recall", "auth-jwt"]);
    assert_eq!(
        rec["scope"], "project",
        "project is exactly what the memory did before: `nxs prime` replays it for this workspace"
    );
    assert_eq!(
        rec["category"], "unsorted",
        "reserved, and therefore countable — the judging migration's signal"
    );
    assert_eq!(rec["refs"], serde_json::json!([]));
    assert_eq!(rec["ordinal"], Value::Null);
    // And the migrated view is a real view: it takes writes and reads them back.
    nxm(dir)
        .args(["classify", "auth-jwt", "--category", "rules"])
        .assert()
        .success();
    assert_eq!(
        json_of(dir, &["--json", "recall", "auth-jwt"])["category"],
        "rules"
    );
}

#[test]
fn a_migrated_workspace_orders_exactly_as_a_refolded_one_would() {
    // The migration backfills insertion order from the op log instead of leaving it at the column
    // default. If it did not, a migrated workspace would read in one order until the next refold and
    // in another afterwards — the view has to be a pure function of the log at every moment.
    let tmp = workspace();
    let dir = tmp.path();
    for key in ["zeta", "alpha", "mid"] {
        nxm(dir)
            .args([
                "remember",
                "body",
                "--key",
                key,
                "--introduction",
                "in one line",
            ])
            .assert()
            .success();
    }
    downgrade_to_v4(&db(dir));

    let migrated = stdout_of(dir, &["memories", "--ordered"]);
    // Force a refold: wipe the view and let the next open rebuild it from the log.
    {
        let conn = rusqlite::Connection::open(db(dir)).unwrap();
        conn.execute_batch("DELETE FROM memories; DELETE FROM view_watermarks;")
            .unwrap();
    }
    assert_eq!(stdout_of(dir, &["memories", "--ordered"]), migrated);
    assert!(
        migrated.starts_with("zeta"),
        "insertion order, not key order: {migrated}"
    );
}

#[test]
fn the_backfill_takes_the_earliest_op_of_a_key_that_has_several() {
    // PR review, Test Quality #3. The migration backfills insertion order with its OWN SQL, which
    // has to agree with the reducer's MIN-fold — but the other migration tests only ever gave each
    // key a single pre-migration op, where "earliest" and "only" are the same thing. A workspace
    // with real history has keys that were updated in place, forgotten and revived; if the two
    // implementations of "earliest" ever disagreed (say one took the LAST op), nothing was red.
    //
    // Here `alpha` is written first but rewritten LAST, so first-op and last-op order differ: by
    // creation `alpha` leads, by latest write it trails.
    let tmp = workspace();
    let dir = tmp.path();
    nxm(dir)
        .args([
            "remember",
            "v1",
            "--key",
            "alpha",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(dir)
        .args([
            "remember",
            "v1",
            "--key",
            "beta",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(dir)
        .args([
            "remember",
            "v1",
            "--key",
            "gamma",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(dir).args(["forget", "alpha"]).assert().success();
    nxm(dir)
        .args([
            "remember",
            "v2",
            "--key",
            "alpha",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success(); // revived, newest op
    nxm(dir)
        .args([
            "remember",
            "v2",
            "--key",
            "beta",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let refolded = stdout_of(dir, &["memories", "--ordered"]);
    downgrade_to_v4(&db(dir));
    let migrated = stdout_of(dir, &["memories", "--ordered"]);

    assert_eq!(
        migrated, refolded,
        "the migration's own SQL must pick the same earliest op the reducer's MIN-fold does"
    );
    assert!(
        migrated.starts_with("alpha"),
        "insertion order, not last-touched order: {migrated}"
    );
}

// ---- (3) the write path is offline -------------------------------------------------------------

/// The crates the `nexus-memory` LIBRARY links (its own manifest plus the workspace crates it
/// depends on). Their `[dependencies]` sections are what the write path can possibly reach.
const LINKED_MANIFESTS: &[&str] = &[
    "crates/memory/Cargo.toml",
    "crates/foundation/Cargo.toml",
    "crates/nxs-init/Cargo.toml",
    "crates/nxs-ui/Cargo.toml",
];

/// Crates that would mean the write path can talk to a model provider.
const NETWORK_CRATES: &[&str] = &[
    "reqwest",
    "ureq",
    "hyper",
    "curl",
    "isahc",
    "attohttpc",
    "surf",
    "tungstenite",
    "native-tls",
    "rustls",
    "openssl",
    "async-openai",
    "anthropic",
];

#[test]
fn nothing_the_memory_library_links_can_reach_the_network() {
    // The structural half of the offline promise. Asking a model at `remember` time would make an
    // everyday write non-deterministic (two runs, two orderings) AND require a provider — so the
    // engine deliberately links no HTTP client at all. This fails by name the moment one is added.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for manifest in LINKED_MANIFESTS {
        let text = std::fs::read_to_string(root.join(manifest)).expect(manifest);
        // Only the real dependencies — a dev-dependency (the sync round-trip proof) is not linked
        // into the library and cannot be reached from `remember`.
        let deps = text
            .split("[dev-dependencies]")
            .next()
            .expect("a manifest has a body");
        for crate_name in NETWORK_CRATES {
            assert!(
                !deps.contains(&format!("\n{crate_name} "))
                    && !deps.contains(&format!("\n{crate_name}=")),
                "{manifest} depends on {crate_name}: the memory write path must stay offline"
            );
        }
    }
}

#[test]
fn remember_works_with_no_network_and_no_provider_credentials() {
    // The behavioural half. Every proxy points into a black hole and every provider credential is
    // stripped, so a write that reached out would have nothing to reach out with. It still succeeds,
    // still returns the canonical record, and still writes exactly ONE op — no ordering call, no
    // classification guess, nothing that only a model could have produced.
    let tmp = workspace();
    let dir = tmp.path();

    let mut cmd = nxm(dir);
    for proxy in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env(proxy, "http://127.0.0.1:1");
    }
    cmd.env("NO_PROXY", "").env("no_proxy", "");
    for key in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
    ] {
        cmd.env_remove(key);
    }
    let out = cmd
        .args([
            "--json",
            "remember",
            "auth uses JWT not sessions",
            "--key",
            "auth-jwt",
            "--category",
            "rules",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let rec: Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(rec["body"], "auth uses JWT not sessions");
    assert_eq!(rec["category"], "rules");

    // Three ops and no more: the body, the category and the introduction the caller asked for —
    // each one a register the CALLER named. Ordering is NOT decided here, and neither is the
    // introduction derived: that is what makes the write deterministic, and what `reorder` (and,
    // since 6j6v.xbnh, the mandatory `--introduction`) exist for.
    let conn = rusqlite::Connection::open(db(dir)).unwrap();
    let fields: Vec<String> = conn
        .prepare("SELECT field FROM ops WHERE domain='fact' ORDER BY lamport, site")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(fields, ["body", "category", "introduction"]);
}

#[test]
fn two_identical_writes_produce_identical_records() {
    // The other face of "no model in the write path": the same input twice is the same output twice.
    // A model asked at write time would make this flaky by construction — noise in exactly the git
    // history a generated memory document is supposed to win.
    let a = workspace();
    let b = workspace();
    let args = [
        "--json",
        "remember",
        "the engine stores; the product judges",
        "--key",
        "ip-line",
        "--category",
        "architecture",
        "--refs",
        "6j6v.5gvj",
        "--introduction",
        "in one line",
    ];
    assert_eq!(stdout_of(a.path(), &args), stdout_of(b.path(), &args));
}

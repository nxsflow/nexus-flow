//! E5m #3rx.3 — the App↔CLI parity differential, the acceptance test of the memory-embedding epic
//! (the mirror of flow's #9t7.6 `parity.rs`).
//!
//! Three seams sit over one memory core: the `nxm` CLI (the agent seam, a process per call), the
//! in-process [`Engine`] handle (the app seam), and — later — the MCP server (E9). The invariant the
//! epic guarantees is "drei Nähte, ein Core": they may never diverge. This pins that two ways:
//!
//!   1. **read parity** — over the SAME store, the embed surface's serialized record is byte-equal
//!      to `nxm <cmd> --json` (so the in-process records an app holds are exactly the CLI's bytes);
//!   2. **write parity** — driving the SAME mutation script through both seams against two stores
//!      seeded identically yields byte-equal op logs (once the per-op random `op_id` ULID is
//!      dropped — everything load-bearing matches: `(lamport, site)`, target, field, value, the
//!      pinned **author**, and **wall_clock**) and byte-equal post-write reads.
//!
//! `now` (`NXM_NOW`) and `actor` (`NXM_ACTOR`) are pinned on BOTH sides, and both workspaces are
//! initialized with `NXF_DETERMINISTIC_IDS=1` so they mint the SAME replica (`ab12`, site 1) — the
//! engine then ADOPTS that replica on open, so no env is set on this process (no test races).

use assert_cmd::Command;
use nexus_memory::engine::Engine;
use nexus_memory::facade::Classification;
use nexus_memory::model::Scope;
use nexus_memory::store::MemoryQuery;
use nexus_memory::workspace::{MemoryWorkspaceExt, Workspace};
use nxs_foundation::model::Op;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-20T10:00:00Z";
const ACTOR: &str = "alice";

/// An `nxm` invocation with `now`/`actor`/ids all pinned, so the subprocess seam is deterministic
/// and mints the SAME replica as the in-process handle's adopted one.
fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", ACTOR)
        .env("NXM_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// Initialize a memory workspace via the external `nxm` (so its replica is the pinned `ab12`/site 1).
fn init(dir: &Path) {
    nxm(dir).arg("init").assert().success();
}

/// Run an `nxm … --json` command to success and return its trimmed stdout (the exact bytes an agent
/// parses).
fn nxm_json(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = nxm(dir)
        .args(full)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf8").trim_end().to_string()
}

/// Serialize a value the SAME way the CLI does for `--json` (declaration-order `to_string`).
fn json_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("serialize")
}

/// The op log as comparable rows with the per-op random `op_id` (a ULID) dropped — everything else
/// is what parity is about. Ordered as `export` returns them: by `(lamport, site)`.
type OpRow = (
    i64,            // lamport
    i64,            // site
    String,         // domain
    String,         // target_kind
    String,         // target_id
    String,         // field
    String,         // op_type
    Option<String>, // value
    String,         // author
    String,         // wall_clock
);

fn op_rows(dir: &Path) -> Vec<OpRow> {
    let store = Workspace::resolve(None, dir)
        .unwrap()
        .open_memory_store()
        .unwrap();
    store
        .export()
        .into_iter()
        .map(|o: Op| {
            (
                o.lamport,
                o.site,
                o.domain,
                o.target_kind,
                o.target_id,
                o.field,
                o.op_type,
                o.value,
                o.author,
                o.wall_clock,
            )
        })
        .collect()
}

/// The introduction each write in the shared script states — `remember` requires one (6j6v.xbnh),
/// so it is part of the script both seams have to emit identically.
fn intro(line: &str) -> Classification {
    Classification {
        introduction: Some(line.to_string()),
        ..Classification::default()
    }
}

/// The shared mutation script through the in-process [`Engine`] (the app seam).
fn drive_engine(dir: &Path) {
    let e = Engine::open(None, dir).unwrap();
    e.remember(
        NOW,
        ACTOR,
        Some("auth-jwt"),
        "auth uses JWT",
        &intro("auth is JWT"),
    )
    .unwrap();
    e.remember(
        NOW,
        ACTOR,
        Some("auth-jwt"),
        "auth uses JWT (HS256)",
        &intro("auth is JWT, HS256"),
    )
    .unwrap(); // update in place (LWW)
    e.remember(
        NOW,
        ACTOR,
        None,
        "always run tests with -race",
        &intro("always run the tests with -race"),
    )
    .unwrap(); // auto-key
    e.remember(
        NOW,
        ACTOR,
        Some("revive-me"),
        "v1",
        &intro("the first version"),
    )
    .unwrap();
    e.forget(NOW, ACTOR, "revive-me").unwrap(); // tombstone
    e.remember(
        NOW,
        ACTOR,
        Some("revive-me"),
        "v2",
        &intro("the second version"),
    )
    .unwrap(); // revive
               // 6j6v.e0z6 — the classification half of the script. Filing while writing, filing afterwards,
               // and ordering are all writes like any other, so the differential has to see them too.
    e.remember(
        NOW,
        ACTOR,
        Some("two-levels"),
        "the two board levels are the load-bearing idea",
        &Classification {
            category: Some("architecture".into()),
            scope: Some(Scope::Global),
            refs: Some(vec!["6j6v.e0z6".into(), "6j6v.5gvj".into()]),
            ..intro("the board has two levels, and that is load-bearing")
        },
    )
    .unwrap();
    e.classify(
        NOW,
        ACTOR,
        "auth-jwt",
        &Classification {
            category: Some("rules".into()),
            ..Classification::default()
        },
    )
    .unwrap();
    e.reorder(
        NOW,
        ACTOR,
        &["two-levels".to_string(), "auth-jwt".to_string()],
    )
    .unwrap();
}

/// The identical script through `nxm` subprocesses (the agent seam).
fn drive_nxm(dir: &Path) {
    let remember = |args: &[&str]| nxm(dir).args(args).assert().success();
    remember(&[
        "remember",
        "auth uses JWT",
        "--key",
        "auth-jwt",
        "--introduction",
        "auth is JWT",
    ]);
    remember(&[
        "remember",
        "auth uses JWT (HS256)",
        "--key",
        "auth-jwt",
        "--introduction",
        "auth is JWT, HS256",
    ]);
    remember(&[
        "remember",
        "always run tests with -race",
        "--introduction",
        "always run the tests with -race",
    ]);
    remember(&[
        "remember",
        "v1",
        "--key",
        "revive-me",
        "--introduction",
        "the first version",
    ]);
    nxm(dir).args(["forget", "revive-me"]).assert().success();
    remember(&[
        "remember",
        "v2",
        "--key",
        "revive-me",
        "--introduction",
        "the second version",
    ]);
    // 6j6v.e0z6 — the same classification half, spelled as an agent types it.
    remember(&[
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
        "the board has two levels, and that is load-bearing",
    ]);
    nxm(dir)
        .args(["classify", "auth-jwt", "--category", "rules"])
        .assert()
        .success();
    nxm(dir)
        .args(["reorder", "two-levels", "auth-jwt"])
        .assert()
        .success();
}

#[test]
fn read_parity_embed_surface_equals_nxm_json_over_the_same_store() {
    // One store, seeded by `nxm`; both seams READ it. The embed surface's serialized records must be
    // byte-equal to `nxm <cmd> --json`.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init(dir);
    drive_nxm(dir);

    let engine = Engine::open(None, dir).unwrap();

    // `memories` (list) parity, including the deterministic key sort.
    assert_eq!(
        json_str(&engine.memories(&MemoryQuery::default()).unwrap()),
        nxm_json(dir, &["memories"]),
        "memories list parity"
    );
    // `memories <search>` parity (case-insensitive substring).
    assert_eq!(
        json_str(&engine.memories(&MemoryQuery::search("jwt")).unwrap()),
        nxm_json(dir, &["memories", "jwt"]),
        "memories search parity"
    );
    // `recall` parity for each surviving key — the per-record byte differential.
    for key in ["auth-jwt", "revive-me"] {
        assert_eq!(
            json_str(&engine.recall(key).unwrap()),
            nxm_json(dir, &["recall", key]),
            "recall parity for {key}"
        );
    }
    // A forgotten/never-remembered key is `not_found` on both seams.
    assert!(engine.recall("revive-me").is_ok());
    assert!(engine.recall("ghost").is_err());
    nxm(dir).args(["recall", "ghost"]).assert().failure();
}

#[test]
fn read_parity_prime_report_equals_nxm_prime_on_both_views() {
    // nxf 6j6v.wph0. `prime` used to exist on ONE seam only — a private function in `cli.rs` — and
    // this differential was blind to it by construction: it compares verbs that exist on both sides,
    // so an omission (as opposed to a divergence) slipped straight through. That gap is what forced
    // app-foundations to rebuild the SessionStart assembly in TypeScript. Now the handle serves the
    // same report, and BOTH of its views are pinned against the CLI: the `--json` projection and the
    // rendered human Markdown the SessionStart hook actually ships.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init(dir);
    drive_nxm(dir);

    let engine = Engine::open(None, dir).unwrap();
    let report = engine.prime().unwrap();

    assert_eq!(
        report.to_value().to_string(),
        nxm_json(dir, &["prime"]),
        "prime --json parity: one report, one projection"
    );
    // The human form is the SessionStart-hook contract, so it is the half that must be byte-equal:
    // `nxm prime` prints exactly `render_markdown()` plus the trailing newline `println!` adds.
    let human = nxm(dir)
        .arg("prime")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        format!("{}\n", report.render_markdown()),
        String::from_utf8(human).expect("utf8"),
        "prime human parity: the hook block an embedder renders IS the one the CLI prints"
    );
    // And the replay itself is the shared read, not a second one — the ORDERED read since
    // 6j6v.643z, which is the same records the plain listing serves, in the sequence they are meant
    // to be read in.
    assert_eq!(
        report.memories,
        engine
            .memories(&MemoryQuery {
                ordered: true,
                ..MemoryQuery::default()
            })
            .unwrap()
    );
}

#[test]
fn write_parity_engine_and_nxm_emit_identical_ops_and_reads() {
    // Two stores seeded with the IDENTICAL script through the two seams.
    let app = TempDir::new().unwrap();
    let cli = TempDir::new().unwrap();
    init(app.path());
    init(cli.path());

    drive_engine(app.path());
    drive_nxm(cli.path());

    // (1) Identical ops: every load-bearing field matches once the random op_id is dropped —
    // crucially the explicit `wall_clock` (now) and `author` (actor) on every op.
    let app_ops = op_rows(app.path());
    let cli_ops = op_rows(cli.path());
    assert_eq!(app_ops, cli_ops, "the two seams emit identical op logs");
    assert_eq!(
        app_ops.len(),
        19,
        "7 body ops + 6 introduction ops (one per remember — mandatory since 6j6v.xbnh) + 6 \
         classification ops (3 registers on the filed create, 1 classify, 2 ordinals)"
    );
    assert!(
        app_ops.iter().all(|r| r.8 == ACTOR && r.9 == NOW),
        "every op carries the pinned actor + now"
    );
    assert!(
        app_ops.iter().all(|r| r.2 == "fact"),
        "every op rides the fact domain (same reducer)"
    );
    // Each register is written by its own op, on its own field — the shape the fold depends on.
    let fields: Vec<&str> = app_ops.iter().map(|r| r.5.as_str()).collect();
    for field in ["body", "category", "scope", "refs", "ordinal"] {
        assert!(
            fields.contains(&field),
            "the script exercises the {field} register: {fields:?}"
        );
    }

    // (2) Identical post-write reads: the handle's records vs `nxm <cmd> --json`, byte for byte.
    let engine = Engine::open(None, app.path()).unwrap();
    assert_eq!(
        json_str(&engine.memories(&MemoryQuery::default()).unwrap()),
        nxm_json(cli.path(), &["memories"]),
        "post-write memories parity"
    );
    // …including the reads that exist only because of the classification registers: the ordered
    // listing and each filter. A divergence in how either seam sorts or narrows shows up here.
    assert_eq!(
        json_str(
            &engine
                .memories(&MemoryQuery {
                    ordered: true,
                    ..MemoryQuery::default()
                })
                .unwrap()
        ),
        nxm_json(cli.path(), &["memories", "--ordered"]),
        "post-write ordered-listing parity"
    );
    assert_eq!(
        json_str(
            &engine
                .memories(&MemoryQuery {
                    category: Some("rules".into()),
                    ..MemoryQuery::default()
                })
                .unwrap()
        ),
        nxm_json(cli.path(), &["memories", "--category", "rules"]),
        "post-write category-filter parity"
    );
    assert_eq!(
        json_str(
            &engine
                .memories(&MemoryQuery {
                    scope: Some(Scope::Global),
                    ..MemoryQuery::default()
                })
                .unwrap()
        ),
        nxm_json(cli.path(), &["memories", "--scope", "global"]),
        "post-write scope-filter parity"
    );
    for key in ["auth-jwt", "revive-me", "two-levels"] {
        assert_eq!(
            json_str(&engine.recall(key).unwrap()),
            nxm_json(cli.path(), &["recall", key]),
            "post-write recall parity for {key}"
        );
    }
    // The end state both seams must agree on: the LWW update won, the revive won, four active, and
    // the classification landed on the right memories without disturbing anyone's text.
    assert_eq!(
        engine.recall("auth-jwt").unwrap().body.as_deref(),
        Some("auth uses JWT (HS256)"),
        "later remember won the register"
    );
    assert_eq!(
        engine.recall("revive-me").unwrap().body.as_deref(),
        Some("v2"),
        "a remember after a forget revived"
    );
    let filed = engine.recall("auth-jwt").unwrap();
    assert_eq!(filed.category, "rules", "classify landed");
    assert_eq!(filed.ordinal, Some(2), "reorder landed");
    assert_eq!(
        engine.memories(&MemoryQuery::default()).unwrap().len(),
        4,
        "four active memories"
    );
}

#[test]
fn determinism_same_now_and_actor_yields_byte_equal_records() {
    // The same op against two fresh stores with the same `now`/`actor` produces byte-identical
    // records on BOTH seams — no race over the second-tick or the ambient `$USER`. The replica
    // pinning (`NXF_DETERMINISTIC_IDS`, inherited via `init`) is NOT load-bearing here: a
    // `MemoryRecord` carries no replica/site bytes, so ONLY `NXM_NOW`/`NXM_ACTOR` buy this equality
    // — the replica pinning matters for the op-log `site` comparison in `write_parity`, not here.
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    init(a.path());
    init(b.path());

    // CLI seam: two independent `nxm remember --json` are byte-equal.
    let args = [
        "remember",
        "auth uses JWT",
        "--key",
        "auth-jwt",
        "--introduction",
        "auth is JWT",
    ];
    let ra = nxm_json(a.path(), &args);
    let rb = nxm_json(b.path(), &args);
    assert_eq!(ra, rb, "nxm remember is deterministic across stores");

    // Embed seam: the engine's remember record is byte-equal to the CLI's over the same pinned
    // now/actor.
    let engine = Engine::open(None, a.path()).unwrap();
    let again = engine
        .remember(
            NOW,
            ACTOR,
            Some("auth-jwt"),
            "auth uses JWT",
            &intro("auth is JWT"),
        )
        .unwrap();
    assert_eq!(
        json_str(&again),
        ra,
        "the embed record is byte-equal to the CLI record"
    );
}

//! E5 #9t7.6 — the App↔CLI parity differential, the acceptance test of the embedding-API epic.
//!
//! Two seams sit over one core: the `nxf` CLI (the agent seam, a process per call) and the
//! in-process [`Engine`] handle (the app seam, held across the host's lifetime). The invariant
//! the epic must guarantee is "drei Nähte, ein Core" — they may never diverge. This drives the
//! SAME mutation script through both seams against two stores seeded identically, then asserts:
//!
//!   1. **identical ops** — the two op logs are byte-equal once the per-op random `op_id` (a
//!      ULID, never deterministic by design) is dropped; everything load-bearing — `(lamport,
//!      site)` order, target, field, value, **author**, and **wall_clock** — matches; and
//!   2. **identical derived output** — `ready`/`blocked`/`next`/`list`/`show` are byte-equal
//!      between the handle's records and `nxf <cmd> --json`.
//!
//! `now` and `actor` are pinned on BOTH sides (`NXF_NOW`/`NXF_ACTOR` for the subprocess, the same
//! constants for the handle). Without that, the two seams would race over the second-tick (a
//! different `wall_clock`) or the ambient `$USER` (a different `author`) and the op logs would
//! differ for reasons that are not divergence. Ids are pinned too (`NXF_DETERMINISTIC_IDS`) so
//! both seams mint the same `ab12.NNNN` sequence and the targets line up.

use assert_cmd::Command;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::workspace::WorkspaceExt;
use nexus_flow_facade::{read, workspace, write};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T08:30:00Z";
const ACTOR: &str = "alice";

/// An `nxf` invocation with `now`/`actor`/ids all pinned, so the subprocess seam is deterministic
/// and matches the handle seam exactly.
fn nxf(dir: &Path) -> Command {
    let mut cmd = nxs_test_support::cargo_bin("nxf");
    cmd.current_dir(dir)
        .env("NXF_NOW", NOW)
        .env("NXF_ACTOR", ACTOR)
        .env("NXF_DETERMINISTIC_IDS", "1");
    cmd
}

/// Run an `nxf … --json` command and return its parsed stdout.
fn nxf_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = args.to_vec();
    full.push("--json");
    let out = nxf(dir)
        .args(full)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json stdout")
}

/// The op log as comparable rows with the per-op random `op_id` dropped (a ULID — never matches
/// across independent runs by design; the rest is what parity is about). Ordered as `export`
/// returns them: by `(lamport, site)`, the canonical order.
type OpRow = (
    i64,            // lamport
    i64,            // site
    String,         // target_kind
    String,         // target_id
    String,         // field
    String,         // op_type
    Option<String>, // value
    String,         // author
    String,         // wall_clock
);

fn op_rows(dir: &Path) -> Vec<OpRow> {
    let store = workspace::discover(dir).unwrap().open_store().unwrap();
    store
        .export()
        .into_iter()
        .map(|o| {
            (
                o.lamport,
                o.site,
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

/// Replace each note's `op_id` (a per-replica random ULID) with a fixed placeholder in a `show`
/// record, so two seams' `show` JSON can be diffed on everything *except* that legitimately
/// non-deterministic id — the note BODY and order, and every other field, still compare exactly.
fn mask_note_ids(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(notes) = v.get_mut("notes").and_then(|n| n.as_array_mut()) {
        for note in notes {
            if let Some(obj) = note.as_object_mut() {
                obj.insert("id".into(), serde_json::Value::from("<note>"));
            }
        }
    }
    v
}

/// Drop the CLI-only additive presentation labels (ee2h: `priority_label`/`type_label`, which
/// `nxf show`/`list`/`next --json` decorate each record with) so the comparison is against the
/// SHARED app-seam contract. The Engine value methods (the seam an embedding host / the MCP tools
/// consume) deliberately omit these — the seam stays the plugin-independent canonical record, pinned
/// by the parity gate — while the CLI presents the plugin's labels alongside. Recurses so it also
/// reaches the record `show --json` nests under `item`.
fn without_presentation_labels(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Array(xs) => {
            serde_json::Value::Array(xs.iter().map(without_presentation_labels).collect())
        }
        serde_json::Value::Object(o) => serde_json::Value::Object(
            o.iter()
                .filter(|(k, _)| k.as_str() != "priority_label" && k.as_str() != "type_label")
                .map(|(k, val)| (k.clone(), without_presentation_labels(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The mandatory-only `create` payload (description + priority), all optionals empty.
fn item<'a>(description: &'a str, priority: &'a str) -> write::NewItem<'a> {
    write::NewItem {
        description,
        priority,
        design: None,
        dod: None,
        due: None,
        defer: None,
        parent: None,
        depends_on: &[],
        custom: &[],
    }
}

/// Apply the shared mutation script through the [`Engine`] handle (the app seam). Returns the
/// minted ids so the CLI side can assert it minted the same ones.
fn drive_app(dir: &Path) -> Vec<String> {
    workspace::init_flow(dir, "issue-tracker").unwrap();
    let engine = Engine::open(None, dir).unwrap();

    let a = engine
        .create(NOW, ACTOR, "bug", "Ship v1", item("cut the release", "P1"))
        .unwrap();
    let b = engine
        .create(
            NOW,
            ACTOR,
            "bug",
            "Write tests",
            item("cover the core", "P2"),
        )
        .unwrap();
    let c = engine
        .create(NOW, ACTOR, "bug", "Docs", item("write the guide", "P3"))
        .unwrap();

    engine.dep_add(NOW, ACTOR, &a.id, &b.id).unwrap(); // Ship v1 depends on Write tests
    engine
        .update(
            NOW,
            ACTOR,
            &c.id,
            &["title=Documentation".into(), "priority=P1".into()],
        )
        .unwrap();
    engine.claim(NOW, ACTOR, &b.id, Some("bob")).unwrap();
    engine.note_add(NOW, ACTOR, &b.id, "started").unwrap();
    engine.mention_add(NOW, ACTOR, &c.id, &a.id).unwrap();
    engine.contributes_add(NOW, ACTOR, &c.id, &a.id).unwrap();
    engine.close(NOW, ACTOR, &b.id, Some("green")).unwrap();

    vec![a.id, b.id, c.id]
}

/// Apply the identical script through `nxf` subprocesses (the agent seam). Returns the minted ids.
fn drive_cli(dir: &Path) -> Vec<String> {
    nxf(dir)
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();

    let create = |title: &str, description: &str, priority: &str| -> String {
        let v = nxf_json(
            dir,
            &[
                "create",
                "--type",
                "bug",
                "--title",
                title,
                "--description",
                description,
                "--priority",
                priority,
            ],
        );
        v["id"].as_str().unwrap().to_string()
    };
    let a = create("Ship v1", "cut the release", "P1");
    let b = create("Write tests", "cover the core", "P2");
    let c = create("Docs", "write the guide", "P3");

    nxf(dir).args(["dep", "add", &a, &b]).assert().success();
    nxf(dir)
        .args([
            "update",
            &c,
            "--set",
            "title=Documentation",
            "--set",
            "priority=P1",
        ])
        .assert()
        .success();
    nxf(dir)
        .args(["claim", &b, "--assignee", "bob"])
        .assert()
        .success();
    nxf(dir)
        .args(["note", "add", &b, "started"])
        .assert()
        .success();
    nxf(dir).args(["mention", "add", &c, &a]).assert().success();
    nxf(dir)
        .args(["contributes", "add", &c, &a])
        .assert()
        .success();
    nxf(dir)
        .args(["close", &b, "--reason", "green"])
        .assert()
        .success();

    vec![a, b, c]
}

#[test]
fn app_surface_and_cli_are_op_and_read_identical_over_the_same_script() {
    // The handle seam reads `NXF_DETERMINISTIC_IDS` from this process's env, so both seams mint
    // the same `ab12.NNNN` sequence. SAFETY: this is the only test in this binary, so no other
    // thread races on the environment.
    std::env::set_var("NXF_DETERMINISTIC_IDS", "1");

    let app_dir = TempDir::new().unwrap();
    let cli_dir = TempDir::new().unwrap();

    let app_ids = drive_app(app_dir.path());
    let cli_ids = drive_cli(cli_dir.path());

    // Pinned ids: both seams minted the identical sequence (so the op targets line up).
    assert_eq!(
        app_ids, cli_ids,
        "both seams mint the same deterministic ids"
    );
    assert_eq!(app_ids, ["ab12.0001", "ab12.0002", "ab12.0003"]);

    // (1) Identical ops: every load-bearing field matches once the random op_id is dropped —
    // crucially the explicit `wall_clock` (now) and `author` (actor) on every op.
    let app_ops = op_rows(app_dir.path());
    let cli_ops = op_rows(cli_dir.path());
    assert_eq!(app_ops, cli_ops, "the two seams emit identical op logs");
    assert!(!app_ops.is_empty(), "the script actually wrote ops");
    assert!(
        app_ops.iter().all(|r| r.7 == ACTOR && r.8 == NOW),
        "every op carries the pinned actor + now"
    );

    // (2) Identical derived output: the handle's records vs `nxf <cmd> --json`, byte for byte.
    let engine = Engine::open(None, app_dir.path()).unwrap();

    // yfwt: `blocked --json` now decorates each record with the CLI-only priority_label/type_label
    // (like next/list); strip them so the comparison stays against the shared app-seam record.
    // 6j6v.1hh7/2kjy: assert the Engine's PUBLIC `blocked_value` (the app seam) — it carries the same
    // sparse `custom` join AND op-log `created_at`/`updated_at` keys the CLI `--json` does, so the two
    // seams stay byte-identical. (The pure `blocked_to_value` builder omits both and would diverge.)
    assert_eq!(
        engine.blocked_value().unwrap(),
        without_presentation_labels(&nxf_json(cli_dir.path(), &["blocked"])),
        "blocked parity"
    );
    // C4 (#916.3): `next` is the tiered actionable set (ready + claimed), which must be
    // byte-identical across the two seams. Assert the Engine's PUBLIC value methods
    // (`next_value`/`list_value`/`show_value`) — the app seam an embedding host consumes, which
    // resolve the parent join AND, since 6j6v.ekf5, the sparse `labels`/`custom` read-layer joins —
    // against the CLI `--json`. Both surfaces route through the SAME `read::*_with_custom` facade
    // path with the same cfg, so custom fields (none here — issue-tracker declares no `[fields]`)
    // ride identically; this covers the `_with_custom` wrappers on both seams.
    // ee2h: the CLI reads (next/list/show) decorate each record with the additive
    // `priority_label`/`type_label`; strip them so the comparison is against the shared app seam,
    // which the parity gate keeps at the canonical record (labels are CLI presentation only).
    // One `next` surface now (the `--include-in-progress` flag is gone — claimed work is always
    // included), so one parity assertion: the embed seam and the CLI must agree on the SAME tiered
    // default order, not just on the same set.
    assert_eq!(
        engine.next_value(NOW).unwrap(),
        without_presentation_labels(&nxf_json(cli_dir.path(), &["next"])),
        "next parity"
    );
    assert_eq!(
        engine.list_value(None, None).unwrap(),
        without_presentation_labels(&nxf_json(cli_dir.path(), &["list"])),
        "list parity"
    );
    // C3 (#916.4): the lane verbs share one facade path, so each is byte-equal across the seams.
    // The script closes 0002 (→ closed lane); nothing is deferred or archived (both empty, still
    // byte-equal). 6j6v.1hh7/2kjy: use the Engine's PUBLIC lane `_value` methods (the app seam) —
    // they carry the sparse `custom` join AND the op-log `created_at`/`updated_at` keys the CLI
    // `--json` emits, so the two seams stay byte-identical (the pure `items_in_order_value` builder
    // omits both). `search` is the exception: it deliberately stays lean (no `custom`, no timestamps,
    // like the pure builder on both seams — bbq6/2kjy), so it is asserted through that pure path.
    assert_eq!(
        engine.closed_value().unwrap(),
        nxf_json(cli_dir.path(), &["closed"]),
        "closed parity"
    );
    assert_eq!(
        engine.deferred_value(NOW).unwrap(),
        nxf_json(cli_dir.path(), &["deferred"]),
        "deferred parity"
    );
    assert_eq!(
        engine.archived_value().unwrap(),
        nxf_json(cli_dir.path(), &["archived"]),
        "archived parity"
    );
    // C6 (#916.6): lane-ranked `search` is a facade method too — `s` matches every item, exercising
    // the next+ip → closed grouping across the seam. yfwt: `search --json` gains the CLI-only
    // priority_label/type_label; strip for the seam. It stays on the pure builder (lean: no custom,
    // no timestamps) on both sides, so the pure `items_in_order_value` is the right facade-side proxy.
    assert_eq!(
        read::items_in_order_value(&engine.search(NOW, "s", None, None, false, false).unwrap()),
        without_presentation_labels(&nxf_json(cli_dir.path(), &["search", "s"])),
        "search parity"
    );

    // `show` for the items WITHOUT notes is byte-equal. Uses `show_value` (the public app-seam read
    // that carries the sparse `labels`/`custom` joins), byte-identical to `nxf show --json`.
    for id in ["ab12.0001", "ab12.0003"] {
        assert_eq!(
            engine.show_value(id).unwrap(),
            without_presentation_labels(&nxf_json(cli_dir.path(), &["show", id])),
            "show parity for {id}"
        );
    }

    // The one note-bearing item (0002) is byte-equal once the note's `op_id` is masked — that id
    // is a per-replica random ULID (the same non-determinism the op compare drops), but everything
    // else the `show` record renders (note BODY order included) must still agree. This closes the
    // last derived surface not directly diffed above (Test Quality review #5).
    assert_eq!(
        mask_note_ids(engine.show_value("ab12.0002").unwrap()),
        mask_note_ids(without_presentation_labels(&nxf_json(
            cli_dir.path(),
            &["show", "ab12.0002"]
        ))),
        "show parity for the note-bearing item (note ids masked)"
    );

    // Sanity that the script reached a non-trivial end state both seams had to agree on: Write
    // tests (0002) was claimed then closed, which unblocks Ship v1 (0001).
    assert_eq!(
        engine.show("ab12.0002").unwrap().item.status.as_deref(),
        Some("closed"),
        "the script reached the expected end state"
    );
    let ready: Vec<_> = engine
        .next(NOW)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ready,
        ["ab12.0001", "ab12.0003"],
        "closing the blocker unblocked Ship v1"
    );
}

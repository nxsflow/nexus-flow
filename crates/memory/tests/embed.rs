//! E5m #3rx.2 — the embedding (Tauri-shaped) acceptance for the memory engine handle.
//!
//! A long-lived in-process [`Engine`] — what a Tauri backend holds in managed state — opens and
//! OWNS the shared `.nxs/` store for the test's lifetime, reading and writing the memory bestand
//! WITHOUT spawning `nxm`. The reactivity half mirrors flow's `embedding`: a genuinely external
//! `nxm` process mutates the same workspace and each write is delivered to the handle as a
//! [`Change`] (the app's re-render cue), and the SAME handle — never reopened — reads the new state
//! live. The deeper proof that the handle's writes are byte-identical to the CLI's lives in the
//! #3rx.3 differential; this file is the vertical "it embeds and it reacts" acceptance.

use assert_cmd::Command;
use nexus_memory::engine::Engine;
use nexus_memory::facade::Classification;
use nexus_memory::model::Scope;
use nexus_memory::project_doc::Drift;
use nexus_memory::store::MemoryQuery;
use nexus_memory::watch::Change;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tempfile::TempDir;

/// Short poll interval so the reactive loop closes within a sub-second assertion window. Production
/// uses the default cadence; this is the documented test seam (`open_with_poll_interval`).
const FAST: Duration = Duration::from_millis(40);

const NOW: &str = "2026-06-20T10:00:00Z";

/// A [`Classification`] carrying only the introduction `remember` requires (6j6v.xbnh).
fn intro(line: &str) -> Classification {
    Classification {
        introduction: Some(line.to_string()),
        ..Classification::default()
    }
}
const ACTOR: &str = "alice";

/// An `nxm` invocation with `now`/`actor` pinned, so the external seam is deterministic.
fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", ACTOR)
        .env("NXM_NOW", NOW);
    c
}

/// Block until the watcher delivers the next change (the app's re-render cue), then coalesce any
/// further queued events so a later wait observes the NEXT distinct external write.
fn await_change(rx: &Receiver<Change>) {
    rx.recv_timeout(Duration::from_secs(5))
        .expect("a Change after the external nxm write");
    while rx.try_recv().is_ok() {}
}

#[test]
fn engine_is_clone_send_sync() {
    // The handle must be `Clone + Send + Sync` to live in a Tauri backend's managed state and be
    // shared across async command tasks (M2 — "Handle hinter Aktor/Mutex für async-Host-Commands").
    // The foundation handle's `Mutex` over the non-`Sync` store is what buys `Sync`; this pins it.
    fn assert_clone_send_sync<T: Clone + Send + Sync + 'static>() {}
    assert_clone_send_sync::<Engine>();
}

#[test]
fn engine_reads_and_writes_in_process_without_spawning_nxm() {
    let tmp = TempDir::new().unwrap();
    // Set up a memory workspace in-process — no `nxm` subprocess anywhere in this test.
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();

    let engine = Engine::open(None, tmp.path()).unwrap();
    assert!(
        engine.memories(&MemoryQuery::default()).unwrap().is_empty(),
        "nothing remembered yet"
    );

    // In-process write through the handle.
    let rec = engine
        .remember(
            NOW,
            ACTOR,
            Some("auth-jwt"),
            "auth uses JWT",
            &intro("in one line"),
        )
        .unwrap();
    assert_eq!(rec.key, "auth-jwt");
    assert_eq!(rec.author, ACTOR, "the explicit actor is recorded");
    assert_eq!(rec.updated, NOW, "the explicit now is stamped");
    assert!(rec.active);

    // In-process read on the SAME handle, never reopened.
    assert_eq!(
        engine.recall("auth-jwt").unwrap().body.as_deref(),
        Some("auth uses JWT")
    );
    let keys: Vec<String> = engine
        .memories(&MemoryQuery::default())
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(keys, ["auth-jwt"]);

    // In-process forget → a tombstone, gone from the active reads.
    let tombstone = engine.forget(NOW, ACTOR, "auth-jwt").unwrap();
    assert!(!tombstone.active, "forget yields an inactive tombstone");
    assert!(
        engine.recall("auth-jwt").is_err(),
        "forgotten → recall not_found"
    );
    assert!(
        engine.memories(&MemoryQuery::default()).unwrap().is_empty(),
        "forgotten → not listed"
    );
}

#[test]
fn the_handle_hands_a_session_the_filed_argument_not_the_alphabet() {
    // 6j6v.643z on the seam an embedding app actually links: `Engine::prime` is what a host injects
    // as session context and `Engine::doc` is what it renders as the project's memory, so the claim
    // "the stored argument is finally the one that is read" has to hold HERE — CLI coverage proves
    // a path the products do not take.
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    // Written in an order that is neither the alphabet nor the argument, so both can be told apart
    // from the sequence of the writes.
    for (key, category) in [
        ("zebra-rule", Some("rules")),
        ("crate-map", Some("architecture")),
        ("open-question", None),
        ("what-this-is", Some("introduction")),
        ("alpha-cut", Some("release-process")),
        ("branching", Some("rules")),
    ] {
        engine
            .remember(
                NOW,
                ACTOR,
                Some(key),
                &format!("the {key} fact"),
                &Classification {
                    category: category.map(str::to_string),
                    ..intro("in one line")
                },
            )
            .unwrap();
    }
    // A position WITHIN `rules` — the half of the order a flat ordinal read got wrong, because
    // `branching` carries an ordinal while the two categories ahead of it carry none.
    engine
        .reorder(NOW, ACTOR, &["branching".to_string()])
        .unwrap();

    let expected = [
        "what-this-is",  // introduction
        "crate-map",     // architecture
        "branching",     // rules, placed
        "zebra-rule",    // rules, unplaced — behind the placed one
        "alpha-cut",     // release-process: the rest, by category name
        "open-question", // unsorted, last
    ];
    let replayed: Vec<String> = engine
        .prime()
        .unwrap()
        .memories
        .into_iter()
        .map(|m| m.key)
        .collect();
    assert_eq!(replayed, expected, "the session is handed the argument");

    // …and the generated document tells the same story, in the same sequence.
    let doc = engine.doc().unwrap();
    let positions: Vec<usize> = expected
        .iter()
        .map(|k| {
            doc.find(&format!("### `{k}`"))
                .unwrap_or_else(|| panic!("{k} is projected:\n{doc}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "the document reads front to back in the same order:\n{doc}"
    );

    // **The subset property was a verbatim substring until nxf 6j6v.waq9**, and 6j6v.xbnh finished
    // the move: the document is the whole of what this workspace knows, while `prime` is an INDEX
    // of it — one written line per memory. What still has to hold on the seam is that nothing the
    // document names is unreachable from the block, and that neither view invents an order.
    let block = engine.prime().unwrap().render_markdown();
    let at = |k: &str| {
        block
            .find(&format!("**{k}**"))
            .unwrap_or_else(|| panic!("{k} must still be reachable from prime:\n{block}"))
    };
    let positions: Vec<usize> = expected.iter().copied().map(at).collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "the block keeps the document's reading order:\n{block}"
    );
    assert!(
        !block.contains("### `branching`") && !block.contains("the branching fact"),
        "no memory is replayed whole any more — every one is a line and a key:\n{block}"
    );
}

#[test]
fn the_handle_serves_the_whole_index_the_bounded_session_block_points_at() {
    // nxf 6j6v.5jm3 on the seam an embedding app actually links. `Engine::prime` renders a block
    // bounded by `PRIME_BLOCK_BUDGET_BYTES` — a host whose own session start has room for more, or
    // that offers the "show me everything" the bounded block names, reads THIS instead. CLI
    // coverage proves a path manufakt.io and nexflow.it do not take.
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    // Enough memories, each at the maximum introduction a write can store, that the block MUST cut.
    for i in 0..80 {
        let head = format!("memory {i:03}: ");
        let line = format!(
            "{head}{}",
            "x".repeat(nexus_memory::model::INTRODUCTION_MAX_CHARS - head.chars().count())
        );
        engine
            .remember(
                NOW,
                ACTOR,
                Some(&format!("memory-{i:03}")),
                "a body no session start ever sees",
                &Classification {
                    category: Some("architecture".to_string()),
                    ..intro(&line)
                },
            )
            .unwrap();
    }

    let index = engine.index().unwrap();
    assert_eq!(
        index.memories.len(),
        80,
        "the whole index, unbounded in count"
    );

    // It is the same selection and the same reading order the session block draws from, which is
    // what makes the block's "N more not listed" arithmetic true.
    let prime = engine.prime().unwrap();
    assert_eq!(
        index.memories.iter().map(|m| &m.key).collect::<Vec<_>>(),
        prime.memories.iter().map(|m| &m.key).collect::<Vec<_>>(),
    );

    let block = prime.render_markdown();
    let listed = block
        .lines()
        .filter(|l| l.starts_with("- **memory-"))
        .count();
    assert!(
        listed < 80,
        "the block is bounded — this fixture exists to make it cut, and it listed all {listed}"
    );
    assert!(
        block.contains(&format!("## Memories (showing {listed} of 80)")),
        "and says so:\n{block}"
    );

    // The whole index carries every line the block left out, and still no bodies.
    let rendered = index.render_markdown();
    assert_eq!(
        rendered
            .lines()
            .filter(|l| l.starts_with("- **memory-"))
            .count(),
        80
    );
    assert!(!rendered.contains("a body no session start ever sees"));
    assert_eq!(index.to_value()["count"], 80);
}

#[test]
fn the_handle_files_and_orders_memories_in_process_too() {
    // PR review, Test Quality #1 (6j6v.e0z6): the classification verbs existed on the handle but no
    // test ever CALLED them — every proof went through `facade::*` directly or through an `nxm`
    // subprocess, so the embedding seam itself was unexercised. An app holding this handle is the
    // consumer these verbs were added for; this is that consumer.
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    // File while writing.
    let rec = engine
        .remember(
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
    assert_eq!(rec.category, "architecture");
    assert_eq!(rec.scope, "global");
    assert_eq!(rec.refs, ["6j6v.5gvj", "6j6v.e0z6"], "canonical: sorted");

    // File an existing memory afterwards — the text and its stamp stay put.
    engine
        .remember(
            NOW,
            ACTOR,
            Some("race"),
            "run tests with -race",
            &intro("in one line"),
        )
        .unwrap();
    let filed = engine
        .classify(
            "2026-08-03T12:00:00Z",
            "bob",
            "race",
            &Classification {
                category: Some("rules".into()),
                ..Classification::default()
            },
        )
        .unwrap();
    assert_eq!(filed.category, "rules");
    assert_eq!(filed.body.as_deref(), Some("run tests with -race"));
    assert_eq!(filed.updated, NOW, "`updated` still dates the fact");
    assert_eq!(filed.author, ACTOR, "the fact's author, not the filer");

    // Order, and read it back through the handle's own query.
    let ordered = engine
        .reorder(NOW, ACTOR, &["race".to_string(), "two-levels".to_string()])
        .unwrap();
    assert_eq!(
        ordered.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    let by_order: Vec<String> = engine
        .memories(&MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        })
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(
        by_order,
        ["two-levels", "race"],
        "6j6v.643z: an ordinal places a memory within its CATEGORY, so `architecture` still leads \
         `rules` even though `race` was given position 1 — the reading order ranks the section \
         first and the position inside it second"
    );
    // …while the default read is untouched — still key order.
    let by_key: Vec<String> = engine
        .memories(&MemoryQuery::default())
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(by_key, ["race", "two-levels"].map(String::from));

    // And the filters narrow on the same handle.
    let rules: Vec<String> = engine
        .memories(&MemoryQuery {
            category: Some("rules".into()),
            ..MemoryQuery::default()
        })
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(rules, ["race"]);
}

#[test]
fn the_handle_rejects_a_classification_it_cannot_store() {
    // The validation an app depends on reaching it: the same guards the CLI gets, on the handle.
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    engine
        .remember(NOW, ACTOR, Some("k"), "v", &intro("in one line"))
        .unwrap();

    assert!(
        engine
            .classify(NOW, ACTOR, "k", &Classification::default())
            .is_err(),
        "nothing to change is an error, not a silent no-op"
    );
    assert!(
        engine
            .classify(
                NOW,
                ACTOR,
                "ghost",
                &Classification {
                    category: Some("rules".into()),
                    ..Classification::default()
                }
            )
            .is_err(),
        "an unknown key is not_found"
    );
    assert!(
        engine
            .classify(
                NOW,
                ACTOR,
                "k",
                &Classification {
                    refs: Some(vec!["6j6v.e0z6,6j6v.5gvj".into()]),
                    ..Classification::default()
                }
            )
            .is_err(),
        "a reference carrying the separator would read back as two — rejected"
    );
    assert!(
        engine
            .reorder(NOW, ACTOR, &["k".to_string(), "ghost".to_string()])
            .is_err(),
        "an unknown key in the sequence is not_found"
    );
    assert_eq!(
        engine.recall("k").unwrap().ordinal,
        None,
        "and the rejected sequence wrote no position at all"
    );
}

#[test]
fn an_unknown_key_is_not_found_through_the_handle() {
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert!(engine.recall("ghost").is_err(), "missing key → not_found");
    assert!(
        engine.forget(NOW, ACTOR, "ghost").is_err(),
        "forgetting a missing key → not_found"
    );
}

#[test]
fn external_nxm_writes_appear_live_through_the_handle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // The agent seam initializes the workspace (real external process).
    nxm(dir).arg("init").assert().success();

    // The app opens ONE long-lived handle and subscribes — held for the whole test, never reopened.
    let engine = Engine::open_with_poll_interval(None, dir, FAST).unwrap();
    let rx = engine.subscribe().unwrap();
    assert!(
        engine.memories(&MemoryQuery::default()).unwrap().is_empty(),
        "no memories at start"
    );

    // 1) External `nxm remember` → live on the same handle, no reopen.
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
    await_change(&rx);
    let keys: Vec<String> = engine
        .memories(&MemoryQuery::default())
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert!(
        keys.contains(&"race".to_string()),
        "the externally-remembered fact is visible live: {keys:?}"
    );
    assert_eq!(
        engine.recall("race").unwrap().body.as_deref(),
        Some("always run tests with -race")
    );

    // 2) External `nxm forget` → the memory drops out of the handle's reads, observed live.
    nxm(dir).args(["forget", "race"]).assert().success();
    await_change(&rx);
    assert!(
        engine.recall("race").is_err(),
        "the external forget is visible live on the same handle"
    );
    assert!(engine.memories(&MemoryQuery::default()).unwrap().is_empty());
}

// ---- the project-memory document through the handle (6j6v.8q88) -------------------------------
//
// PR #287 review, Test Quality #1 (High): `doc`/`doc_check` and the write-time regeneration
// existed on the handle with NO coverage here. The verb-seam gate only asserts that a symbol named
// `doc` exists — it cannot see whether the handle projects, or whether it projects the same bytes
// the CLI writes. An embedding host had no regression protection at all.

/// The document a handle-owned workspace has on disk.
fn doc_on_disk(root: &Path) -> String {
    std::fs::read_to_string(root.join(nexus_memory::project_doc::FILE_NAME))
        .expect("the document exists")
}

#[test]
fn a_write_through_the_handle_projects_the_document_like_the_cli_does() {
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert!(
        !tmp.path()
            .join(nexus_memory::project_doc::FILE_NAME)
            .exists(),
        "an empty workspace gets no document planted in its root"
    );

    engine
        .remember(
            NOW,
            ACTOR,
            Some("auth-jwt"),
            "auth uses JWT",
            &intro("in one line"),
        )
        .unwrap();
    assert!(doc_on_disk(tmp.path()).contains("auth uses JWT"));

    // `doc()` renders the same bytes the write just put on disk — the property that lets a host
    // show the projection without reading the file, and the CLI's `nxm doc` print the file's own
    // content.
    assert_eq!(engine.doc().unwrap(), doc_on_disk(tmp.path()));
    assert_eq!(engine.doc_check().unwrap(), Drift::InSync);

    // Every other write verb on the handle projects too — the same omission risk the CLI's own
    // table-driven test covers, on the seam an embedding app actually links.
    engine
        .classify(
            NOW,
            ACTOR,
            "auth-jwt",
            &Classification {
                scope: Some(Scope::Item),
                refs: Some(vec!["6j6v.8q88".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        !doc_on_disk(tmp.path()).contains("auth uses JWT"),
        "an item-scoped memory naming a board item reads there, not here"
    );

    engine
        .remember(
            NOW,
            ACTOR,
            Some("race"),
            "run the tests with -race",
            &intro("in one line"),
        )
        .unwrap();
    engine.reorder(NOW, ACTOR, &["race".to_string()]).unwrap();
    assert_eq!(
        engine.doc_check().unwrap(),
        Drift::InSync,
        "reorder projects"
    );

    engine.forget(NOW, ACTOR, "race").unwrap();
    assert!(!doc_on_disk(tmp.path()).contains("run the tests with -race"));
    assert_eq!(
        engine.doc_check().unwrap(),
        Drift::InSync,
        "forget projects"
    );
}

#[test]
fn the_handle_reports_a_hand_edited_document_as_drifted() {
    // The guard, on the embedding seam: a host that ships its own "your context is stale" surface
    // reads this rather than re-deriving the comparison.
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        engine.doc_check().unwrap(),
        Drift::NotApplicable,
        "no file and nothing to project is trivially in sync, not a fault"
    );

    engine
        .remember(
            NOW,
            ACTOR,
            Some("auth-jwt"),
            "auth uses JWT",
            &intro("in one line"),
        )
        .unwrap();
    std::fs::write(
        tmp.path().join(nexus_memory::project_doc::FILE_NAME),
        "# Project memory\n\nsomething a human typed\n",
    )
    .unwrap();
    assert_eq!(engine.doc_check().unwrap(), Drift::Drifted);

    std::fs::remove_file(tmp.path().join(nexus_memory::project_doc::FILE_NAME)).unwrap();
    assert_eq!(
        engine.doc_check().unwrap(),
        Drift::Missing,
        "memories to project but no file — the shape a daemon that never ran here leaves"
    );
}

#[test]
fn a_write_still_succeeds_when_the_projection_cannot_be_written() {
    // PR #287 review, Test Quality #2 (Medium): the documented contract is that a write is NEVER
    // reported as failed because its projection lagged — the memory is already committed by then,
    // so failing would report a write that happened as one that did not. Forced here by making the
    // document a DIRECTORY, which no writer can rename a file over.
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    std::fs::create_dir(tmp.path().join(nexus_memory::project_doc::FILE_NAME)).unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();

    let rec = engine
        .remember(
            NOW,
            ACTOR,
            Some("auth-jwt"),
            "auth uses JWT",
            &intro("in one line"),
        )
        .expect("the write succeeds even though the projection cannot land");
    assert_eq!(rec.key, "auth-jwt");
    assert_eq!(
        engine.recall("auth-jwt").unwrap().body.as_deref(),
        Some("auth uses JWT"),
        "the memory really is stored"
    );

    // And the host is not left guessing: the failure surfaces on the very check that exists for it.
    assert!(
        engine.doc_check().is_err() || engine.doc_check().unwrap().is_drift(),
        "a projection that could not land shows up as drift, not as silence"
    );
}

// ---- the judging migration on the seam (6j6v.9yaj) ---------------------------------------------

/// A memory workspace with a hand-written `CLAUDE.md` and one unfiled memory — the shape the whole
/// migration exists for, on the seam an embedding app links.
fn workspace_awaiting_migration() -> (TempDir, Engine) {
    let tmp = TempDir::new().unwrap();
    nexus_memory::workspace::setup(tmp.path(), &nexus_memory::workspace::memory_config()).unwrap();
    std::fs::write(
        tmp.path().join("CLAUDE.md"),
        "# Project\n\nan orienting paragraph\n\n## Branching\n\nnever commit to main\n",
    )
    .unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    engine
        .remember(
            NOW,
            ACTOR,
            Some("auth-jwt"),
            "auth uses JWT",
            &intro("in one line"),
        )
        .unwrap();
    (tmp, engine)
}

#[test]
fn the_handle_plans_a_migration_over_its_own_workspace_root() {
    // The plan is the app's review material: an embedding host puts its own confirmation UI exactly
    // where the CLI puts a file, so it has to be able to obtain the plan WITHOUT shelling out to
    // `nxm` — the omission this seam rule exists to prevent.
    let (_tmp, engine) = workspace_awaiting_migration();

    let plan = engine.migrate_plan().unwrap();
    let keys: Vec<&str> = plan.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(
        keys,
        ["auth-jwt", "project", "branching"],
        "the unfiled memory, then the document's sections — the handle found the root itself"
    );
    assert!(
        plan.entries
            .iter()
            .all(|e| e.category == "unsorted" && e.action != nexus_memory::migration::Action::Skip),
        "and it proposes material, never a judgement"
    );
    assert!(plan.instructions.contains("skip"), "the task travels along");
}

#[test]
fn the_handle_applies_a_judged_plan_projects_it_and_marks_the_stream() {
    use nexus_memory::migration::{Action, ApplyOptions};
    let (tmp, engine) = workspace_awaiting_migration();
    assert!(engine.migrate_status().unwrap().is_open());

    let mut plan = engine.migrate_plan().unwrap();
    for entry in &mut plan.entries {
        entry.category = "rules".to_string();
        entry.introduction = Some(format!("what `{}` says, in one line", entry.key));
    }
    // One section is not durable knowledge — the judgement a mechanism could never make.
    plan.entries[1].action = Action::Skip;

    let report = engine
        .migrate_apply(NOW, ACTOR, &plan, &ApplyOptions::default())
        .unwrap();
    assert_eq!(report.classified, ["auth-jwt"]);
    assert_eq!(report.remembered, ["branching"]);
    assert_eq!(report.skipped, ["project"]);

    // The write projected, exactly as every other write through this handle does…
    assert_eq!(engine.doc_check().unwrap(), Drift::InSync);
    assert!(doc_on_disk(tmp.path()).contains("never commit to main"));
    // …the section really MOVED rather than being copied…
    let left = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
    assert!(!left.contains("never commit to main"), "{left}");
    assert!(
        left.contains("an orienting paragraph"),
        "the skipped one stays"
    );
    // …and the stream carries the mark, so a second device asks instead of filing it all again.
    let status = engine.migrate_status().unwrap();
    assert!(!status.is_open(), "nothing is unfiled now");
    let mark = status.mark.expect("the run left a mark");
    assert_eq!(mark.actor, ACTOR);
    assert_eq!((mark.classified, mark.remembered), (1, 1));
}

#[test]
fn the_handle_refuses_an_unjudged_plan_and_writes_nothing() {
    // "The human decides" has to hold on the seam too, or an app could file forty memories into the
    // category that means "not filed" and call it a migration.
    use nexus_memory::migration::ApplyOptions;
    let (tmp, engine) = workspace_awaiting_migration();
    let plan = engine.migrate_plan().unwrap();

    let err = engine
        .migrate_apply(NOW, ACTOR, &plan, &ApplyOptions::default())
        .expect_err("nobody judged it");
    assert!(err.msg.contains("unsorted"), "{}", err.msg);
    assert_eq!(
        engine.recall("auth-jwt").unwrap().category,
        "unsorted",
        "and the store is exactly as it was"
    );
    assert!(
        std::fs::read_to_string(tmp.path().join("CLAUDE.md"))
            .unwrap()
            .contains("never commit to main"),
        "the document too"
    );
}

#[test]
fn a_dry_run_through_the_handle_reports_without_writing_or_projecting() {
    use nexus_memory::migration::ApplyOptions;
    let (tmp, engine) = workspace_awaiting_migration();
    let mut plan = engine.migrate_plan().unwrap();
    for entry in &mut plan.entries {
        entry.category = "rules".to_string();
        entry.introduction = Some(format!("what `{}` says, in one line", entry.key));
    }
    let before = doc_on_disk(tmp.path());

    let report = engine
        .migrate_apply(
            NOW,
            ACTOR,
            &plan,
            &ApplyOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(report.dry_run);
    assert_eq!(report.classified, ["auth-jwt"]);
    assert!(report.mark.is_none());
    assert!(engine.migrate_status().unwrap().is_open());
    assert_eq!(
        doc_on_disk(tmp.path()),
        before,
        "a dry run does not even project — computing a report must not touch a file"
    );
    assert!(
        std::fs::read_to_string(tmp.path().join("CLAUDE.md"))
            .unwrap()
            .contains("never commit to main"),
        "nor move a section out of its document"
    );
}

#[test]
fn the_handle_refuses_a_second_migration_on_a_marked_stream() {
    // PR #289 review, Test Quality #1 (Critical). The once-per-stream rule used to be enforced in
    // the CLI's `migrate plan` alone — the one path that writes nothing — so an app on this seam
    // had NO protection at all against filing the same memories a second time and letting
    // last-writer-wins blend two judgements nobody made together. The guard is on the write now,
    // which is where every caller meets it.
    use nexus_memory::migration::ApplyOptions;
    let (_tmp, engine) = workspace_awaiting_migration();
    let mut plan = engine.migrate_plan().unwrap();
    for entry in &mut plan.entries {
        entry.category = "rules".to_string();
        entry.introduction = Some(format!("what `{}` says, in one line", entry.key));
    }
    engine
        .migrate_apply(NOW, ACTOR, &plan, &ApplyOptions::default())
        .expect("the first run");

    let err = engine
        .migrate_apply(NOW, "bob", &plan, &ApplyOptions::default())
        .expect_err("the same plan, applied again through the handle");
    assert!(err.msg.contains("already migrated"), "{}", err.msg);
    assert!(err.msg.contains("--again"), "{}", err.msg);

    // A dry run is refused too: previewing a run that would be refused previews nothing.
    let err = engine
        .migrate_apply(
            NOW,
            "bob",
            &plan,
            &ApplyOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .expect_err("a preview of a refused run");
    assert!(err.msg.contains("already migrated"), "{}", err.msg);

    // …and the deliberate second pass is still reachable, on the seam as on the command line.
    engine
        .migrate_apply(
            NOW,
            "bob",
            &plan,
            &ApplyOptions {
                again: true,
                ..Default::default()
            },
        )
        .expect("`again` is the explicit answer here too");
}

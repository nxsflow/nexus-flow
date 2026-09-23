//! memory's half of the actor-contract detector (nexus-flow-6j6v.d4dw): the 5 public write entry
//! points of `nexus-memory`, held against every boundary of the shared actor rule.
//!
//! The table, the boundaries and the reasoning live once, in
//! [`nxs_test_support::actor_contract`] — read that first. This file is only the wiring.
//!
//! Taken HERE rather than in `src/facade.rs`'s unit tests on purpose: `nexus-memory` is a surface
//! an embedding app pins by tag and links directly (project rule `engine-seam-test-rule`), so the
//! contract is pinned where a consumer stands, through the crate's public API.

use std::path::Path;

use nexus_memory::error::Result;
use nexus_memory::facade::{self, Classification};
use nexus_memory::migration::{
    Action, ApplyOptions, EntryKind, MigrationPlan, PlanEntry, PLAN_INSTRUCTIONS,
};
use nexus_memory::store::MemoryStore;
use nxs_test_support::actor_contract::{self, Boundary, Outcome};

const NOW: &str = "2026-08-09T09:00:00Z";
/// The identity that builds every fixture — never a boundary value.
const SEEDER: &str = "seeder";
const KEY: &str = "seeded-fact";

/// A fresh store holding one remembered fact, filed by [`SEEDER`].
fn fixture() -> MemoryStore {
    let mut s = MemoryStore::open_in_memory(1);
    facade::remember(
        &mut s,
        NOW,
        SEEDER,
        Some(KEY),
        "the seeded fact",
        &introduced(),
    )
    .unwrap();
    s
}

/// The introduction `remember` requires (6j6v.xbnh); this file is about the ACTOR, not the line.
fn introduced() -> Classification {
    Classification {
        introduction: Some("the seeded fact, in one line".to_string()),
        ..Classification::default()
    }
}

fn category(name: &str) -> Classification {
    Classification {
        category: Some(name.to_string()),
        ..introduced()
    }
}

/// A decided plan that files the seeded memory — the shape `migrate_apply` accepts (every entry
/// judged), built by hand so the fixture needs no context documents on disk.
fn decided_plan() -> MigrationPlan {
    MigrationPlan {
        version: 1,
        instructions: PLAN_INSTRUCTIONS.to_string(),
        categories: Vec::new(),
        scopes: Vec::new(),
        entries: vec![PlanEntry {
            kind: EntryKind::Memory,
            key: KEY.to_string(),
            action: Action::Classify,
            category: "rules".to_string(),
            scope: "project".to_string(),
            refs: Vec::new(),
            body: "the seeded fact".to_string(),
            introduction: Some("the seeded fact, in one line".to_string()),
            source: None,
        }],
        skipped_sources: Vec::new(),
    }
}

/// What the call did, plus the author of every op IT appended. Reported on BOTH paths: a refusal
/// that had already written is a defect the verdict alone would not show.
fn outcome<T>(s: &MemoryStore, before: usize, r: Result<T>) -> Outcome {
    // Collected, not lazy: `export()` hands back an owned Vec, so an iterator over a slice of
    // it would outlive the temporary.
    let appended = || -> Vec<String> {
        s.export()[before..]
            .iter()
            .map(|o| o.author.clone())
            .collect()
    };
    match r {
        Ok(_) => Outcome::accepted(appended()),
        Err(e) => Outcome::rejected(e.kind.as_str(), appended()),
    }
}

type Case = (&'static str, Box<dyn FnMut(&Boundary) -> Outcome>);

/// One closure per public write entry point — also the covered set the completeness gate holds
/// against the source.
fn cases(root: &Path) -> Vec<Case> {
    let root = root.to_path_buf();
    vec![
        (
            "remember",
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::remember(
                    &mut s,
                    NOW,
                    b.actor,
                    Some("under-test"),
                    "a new fact",
                    &introduced(),
                );
                outcome(&s, before, r)
            }),
        ),
        (
            "classify",
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::classify(&mut s, NOW, b.actor, KEY, &category("rules"));
                outcome(&s, before, r)
            }),
        ),
        (
            "reorder",
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::reorder(&mut s, NOW, b.actor, &[KEY.to_string()]);
                outcome(&s, before, r)
            }),
        ),
        (
            "forget",
            Box::new(|b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::forget(&mut s, NOW, b.actor, KEY);
                outcome(&s, before, r)
            }),
        ),
        (
            "migrate_apply",
            Box::new(move |b: &Boundary| {
                let mut s = fixture();
                let before = s.export().len();
                let r = facade::migrate_apply(
                    &mut s,
                    NOW,
                    b.actor,
                    &root,
                    &decided_plan(),
                    &ApplyOptions {
                        // The documents are not the subject here; the actor is.
                        keep_sources: true,
                        dry_run: false,
                        again: false,
                    },
                );
                outcome(&s, before, r)
            }),
        ),
    ]
}

#[test]
fn every_write_entry_point_honours_the_actor_contract() {
    let root = tempfile::tempdir().unwrap();
    for (entry, mut call) in cases(root.path()) {
        actor_contract::assert_actor_contract(
            &format!("nexus_memory::facade::{entry}"),
            str::to_owned,
            &mut call,
        );
    }
}

#[test]
fn the_pinned_set_is_every_entry_point_that_validates_an_actor() {
    let root = tempfile::tempdir().unwrap();
    let covered: Vec<&str> = cases(root.path()).iter().map(|(name, _)| *name).collect();
    actor_contract::assert_every_entry_point_is_covered(
        "nexus-memory",
        &nxs_test_support::verb_seam::crate_relative(env!("CARGO_MANIFEST_DIR"), "src"),
        &covered,
        &[],
    );
}

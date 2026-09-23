//! flow's half of the actor-contract detector (nexus-flow-6j6v.d4dw): all 17 public write entry
//! points of `nexus-flow-facade`, held against every boundary of the shared actor rule.
//!
//! The table, the boundaries and the reasoning live once, in
//! [`nxs_test_support::actor_contract`] — read that first. This file is only the wiring: one
//! closure per entry point, each driving it with a boundary's actor against a FRESH fixture and
//! reporting what came back plus the authors of exactly the ops it appended.
//!
//! This SUPERSEDES `write.rs`'s `every_write_entry_point_rejects_a_blank_actor_instead_of_panicking`
//! (PR #311, review finding Code Quality #1). That test enumerated the same 17 points with the same
//! blank actors, but pinned the REFUSAL side only — so PR #314, which tightened the same rule
//! further, ran straight through it. Its substrate claim (a refused write leaves the log exactly as
//! it found it) is carried over into the shared runner, where it now holds at every boundary of all
//! 26 entry points instead of these 17.

use nexus_flow_core::model::{LinkRelation, LinkWeight};
use nexus_flow_core::store::Store;
use nexus_flow_facade::error::Result;
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::write::{self, NewItem};
use nxs_test_support::actor_contract::{self, Boundary, Outcome};

const NOW: &str = "2026-08-09T09:00:00Z";
/// The identity that builds every fixture — never a boundary value, so a seeding op can never be
/// mistaken for the op under test.
const SEEDER: &str = "seeder";

fn cfg() -> PluginConfig {
    plugin::load("issue-tracker").unwrap()
}

fn minimal<'a>() -> NewItem<'a> {
    NewItem {
        description: "why",
        priority: "P1",
        design: None,
        dod: None,
        due: None,
        defer: None,
        parent: None,
        depends_on: &[],
        custom: &[],
    }
}

/// A fresh store holding `n` open issues, seeded by [`SEEDER`].
fn fixture(n: usize) -> (Store, PluginConfig, Vec<String>) {
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    let ids = (0..n)
        .map(|i| {
            write::create(
                &mut s,
                &cfg,
                "ab12",
                NOW,
                SEEDER,
                "bug",
                &format!("seed {i}"),
                minimal(),
            )
            .unwrap()
            .id
        })
        .collect();
    (s, cfg, ids)
}

/// A fresh store holding one CLOSED issue — the precondition `archive` needs to write anything.
fn closed_fixture() -> (Store, PluginConfig, String) {
    let (mut s, cfg, ids) = fixture(1);
    write::close(&mut s, NOW, SEEDER, &ids[0], Some("done")).unwrap();
    let id = ids[0].clone();
    (s, cfg, id)
}

/// What the call did, plus the author of every op IT appended (`before` is the op count taken
/// immediately before the call, so the fixture's own writes stay out). Reported on BOTH paths: a
/// refusal that had already written is a defect the verdict alone would not show.
fn outcome<T>(s: &Store, before: usize, r: Result<T>) -> Outcome {
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

/// One closure per public write entry point. The list is also what the completeness gate holds
/// against the source, so an entry point cannot be pinned without being run, or run without being
/// counted.
fn cases() -> Vec<Case> {
    vec![
        (
            "create",
            Box::new(|b: &Boundary| {
                let (mut s, cfg, _) = fixture(0);
                let before = s.export().len();
                let r = write::create(
                    &mut s,
                    &cfg,
                    "ab12",
                    NOW,
                    b.actor,
                    "bug",
                    "under test",
                    minimal(),
                );
                outcome(&s, before, r)
            }),
        ),
        (
            "update",
            Box::new(|b: &Boundary| {
                let (mut s, cfg, ids) = fixture(1);
                let before = s.export().len();
                let r = write::update(
                    &mut s,
                    &cfg,
                    NOW,
                    b.actor,
                    &ids[0],
                    &["title=renamed".to_string()],
                );
                outcome(&s, before, r)
            }),
        ),
        (
            "claim",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                let before = s.export().len();
                let r = write::claim(&mut s, NOW, b.actor, &ids[0], None);
                outcome(&s, before, r)
            }),
        ),
        (
            "close",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                let before = s.export().len();
                let r = write::close(&mut s, NOW, b.actor, &ids[0], Some("done"));
                outcome(&s, before, r)
            }),
        ),
        (
            "dep_add",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(2);
                let before = s.export().len();
                let r = write::dep_add(&mut s, NOW, b.actor, &ids[0], &ids[1]);
                outcome(&s, before, r)
            }),
        ),
        (
            "dep_remove",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(2);
                write::dep_add(&mut s, NOW, SEEDER, &ids[0], &ids[1]).unwrap();
                let before = s.export().len();
                let r = write::dep_remove(&mut s, NOW, b.actor, &ids[0], &ids[1]);
                outcome(&s, before, r)
            }),
        ),
        (
            "mention_add",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(2);
                let before = s.export().len();
                let r = write::mention_add(&mut s, NOW, b.actor, &ids[0], &ids[1]);
                outcome(&s, before, r)
            }),
        ),
        (
            "mention_remove",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(2);
                write::mention_add(&mut s, NOW, SEEDER, &ids[0], &ids[1]).unwrap();
                let before = s.export().len();
                let r = write::mention_remove(&mut s, NOW, b.actor, &ids[0], &ids[1]);
                outcome(&s, before, r)
            }),
        ),
        (
            "contributes_add",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(2);
                let before = s.export().len();
                let r = write::contributes_add(&mut s, NOW, b.actor, &ids[0], &ids[1]);
                outcome(&s, before, r)
            }),
        ),
        (
            "contributes_remove",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(2);
                write::contributes_add(&mut s, NOW, SEEDER, &ids[0], &ids[1]).unwrap();
                let before = s.export().len();
                let r = write::contributes_remove(&mut s, NOW, b.actor, &ids[0], &ids[1]);
                outcome(&s, before, r)
            }),
        ),
        (
            "label_add",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                let before = s.export().len();
                let r = write::label_add(&mut s, NOW, b.actor, &ids[0], "urgent");
                outcome(&s, before, r)
            }),
        ),
        (
            "label_remove",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                write::label_add(&mut s, NOW, SEEDER, &ids[0], "urgent").unwrap();
                let before = s.export().len();
                let r = write::label_remove(&mut s, NOW, b.actor, &ids[0], "urgent");
                outcome(&s, before, r)
            }),
        ),
        (
            "thread_link_add",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                let before = s.export().len();
                let r = write::thread_link_add(
                    &mut s,
                    NOW,
                    b.actor,
                    "t-1",
                    &ids[0],
                    LinkRelation::Cited,
                    LinkWeight::Passing,
                );
                outcome(&s, before, r)
            }),
        ),
        (
            "thread_link_remove",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                write::thread_link_add(
                    &mut s,
                    NOW,
                    SEEDER,
                    "t-1",
                    &ids[0],
                    LinkRelation::Cited,
                    LinkWeight::Passing,
                )
                .unwrap();
                let before = s.export().len();
                let r = write::thread_link_remove(&mut s, NOW, b.actor, "t-1", &ids[0]);
                outcome(&s, before, r)
            }),
        ),
        (
            "archive",
            Box::new(|b: &Boundary| {
                let (mut s, _, id) = closed_fixture();
                let before = s.export().len();
                let r = write::archive(&mut s, NOW, b.actor, &[id]);
                outcome(&s, before, r)
            }),
        ),
        (
            "unarchive",
            Box::new(|b: &Boundary| {
                let (mut s, _, id) = closed_fixture();
                write::archive(&mut s, NOW, SEEDER, std::slice::from_ref(&id)).unwrap();
                let before = s.export().len();
                let r = write::unarchive(&mut s, NOW, b.actor, &[id]);
                outcome(&s, before, r)
            }),
        ),
        (
            "note_add",
            Box::new(|b: &Boundary| {
                let (mut s, _, ids) = fixture(1);
                let before = s.export().len();
                let r = write::note_add(&mut s, NOW, b.actor, &ids[0], "worklog");
                outcome(&s, before, r)
            }),
        ),
    ]
}

#[test]
fn every_write_entry_point_honours_the_actor_contract() {
    for (entry, mut call) in cases() {
        actor_contract::assert_actor_contract(
            &format!("nexus_flow_facade::write::{entry}"),
            str::to_owned,
            &mut call,
        );
    }
}

#[test]
fn the_pinned_set_is_every_entry_point_that_validates_an_actor() {
    let covered: Vec<&str> = cases().iter().map(|(name, _)| *name).collect();
    actor_contract::assert_every_entry_point_is_covered(
        "nexus-flow-facade",
        &nxs_test_support::verb_seam::crate_relative(env!("CARGO_MANIFEST_DIR"), "src"),
        &covered,
        &[actor_contract::Delegator {
            name: "actor",
            reason: "`validate::actor` IS the rule under flow's own name — it forwards to \
                     `nxs_foundation::model::validate_author` and writes nothing itself; the 17 \
                     entry points above are its only callers",
        }],
    );
}

// The substrate claim carried over from PR #311 — a refused write leaves the log exactly as it
// found it — used to live here as its own test. It now runs inside the shared runner, on every
// boundary of every entry point of all THREE surfaces rather than the 17 of this one (PR #317
// review, Test Quality #1): `outcome` above reports the appended ops on the refusal path too, and
// `assert_actor_contract` requires them to be none.

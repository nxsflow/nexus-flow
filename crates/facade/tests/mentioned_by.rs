//! Mention edges read over the engine seam (6j6v.q29p): `Engine::mentions` (an item's outgoing
//! targets) and `Engine::mentioned_by` (the items that mention a target), the latter narrowed by
//! type and container and paged through the same machine-local snapshot `next` pages through
//! (6j6v.15ed). Tested at the seam the products take.

use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::{EdgeReadStats, Store};
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::read::{self, MentionedByQuery, NextFilter, NextQuery};
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";
const ACTOR: &str = "alice";
const TARGET: &str = "ab12.t001";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    tmp
}

/// Another writer on the workspace — what a second process, or a sync pull, is to the engine.
fn writer(dir: &Path) -> Store {
    workspace::discover(dir).unwrap().open_store().unwrap()
}

fn item(s: &mut Store, id: &str, ty: &str) {
    s.create_item(id, ty, id, "t");
    s.set_field(id, "priority", Some("2".into()), "t");
}

fn ids(items: &[nexus_flow_core::model::ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn by(engine: &Engine, q: MentionedByQuery) -> read::NextPage {
    engine.mentioned_by(TARGET, &q).unwrap()
}

/// A target, two containers, and mentioners of every kind:
///
/// - `0001` task, parent e001 — mentions the target
/// - `0002` bug, contributes to e002 — mentions the target
/// - `0003` task, parent e002 AND contributes to e001 — mentions the target
/// - `0004` bug, in nothing, closed — mentions the target
/// - `0005` task, deleted — mentions the target
/// - `0006` task — mentions only e001, not the target
/// - `0007` task — its mention of the target was removed
fn board(dir: &Path) {
    let mut s = writer(dir);
    item(&mut s, TARGET, "feature");
    item(&mut s, "ab12.e001", "epic");
    item(&mut s, "ab12.e002", "epic");
    for (id, ty) in [
        ("ab12.0001", "task"),
        ("ab12.0002", "bug"),
        ("ab12.0003", "task"),
        ("ab12.0004", "bug"),
        ("ab12.0005", "task"),
        ("ab12.0006", "task"),
        ("ab12.0007", "task"),
    ] {
        item(&mut s, id, ty);
    }
    s.set_parent("ab12.0001", "ab12.e001", "t").unwrap();
    s.set_parent("ab12.0003", "ab12.e002", "t").unwrap();
    s.add_edge("ab12.0002", "ab12.e002", EdgeKind::ContributesTo, "t");
    s.add_edge("ab12.0003", "ab12.e001", EdgeKind::ContributesTo, "t");
    for from in [
        "ab12.0003",
        "ab12.0001",
        "ab12.0002",
        "ab12.0004",
        "ab12.0005",
        "ab12.0007",
    ] {
        s.add_edge(from, TARGET, EdgeKind::Mentions, "t");
    }
    s.add_edge("ab12.0006", "ab12.e001", EdgeKind::Mentions, "t");
    s.remove_edge("ab12.0007", TARGET, EdgeKind::Mentions, "t");
    s.set_field("ab12.0004", "status", Some("closed".into()), "t");
    s.delete_item("ab12.0005", "t");
}

const LIVE: [&str; 4] = ["ab12.0001", "ab12.0002", "ab12.0003", "ab12.0004"];

// ---- Engine::mentions ----------------------------------------------------------

#[test]
fn mentions_reads_the_outgoing_targets_over_the_seam() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(engine.mentions("ab12.0006").unwrap(), ["ab12.e001"]);
    assert_eq!(engine.mentions("ab12.0001").unwrap(), [TARGET]);
    assert!(engine.mentions("ab12.0007").unwrap().is_empty(), "removed");
    // A write through the engine reads back through it.
    engine
        .mention_add(NOW, ACTOR, "ab12.0006", "ab12.e002")
        .unwrap();
    assert_eq!(
        engine.mentions("ab12.0006").unwrap(),
        ["ab12.e001", "ab12.e002"]
    );
    let err = engine.mentions("ab12.zzzz").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

// ---- Engine::mentioned_by: the set and its filters -------------------------------

#[test]
fn mentioned_by_lists_the_live_mentioners_in_id_order() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p = by(&engine, MentionedByQuery::new());
    // Closed counts (a reference is not work); deleted, removed and other targets' do not.
    assert_eq!(ids(&p.items), LIVE);
    assert_eq!((p.total, p.start, p.restarted), (4, 0, false));
    assert_eq!(p.next_token, None);
    // Nothing mentions e002.
    let none = engine
        .mentioned_by("ab12.e002", &MentionedByQuery::new())
        .unwrap();
    assert!(none.items.is_empty());
    assert_eq!(none.total, 0);
}

#[test]
fn an_archived_mentioner_still_counts_at_the_seam() {
    // Live is the one rule (spec §11): archived is not deleted, so `Engine::mentioned_by` keeps
    // it, as `mentioners_of` and `count = "mentioned_by"` do (review of PR #48, Test Quality #9).
    let tmp = workspace();
    board(tmp.path());
    {
        let mut s = writer(tmp.path());
        s.set_field("ab12.0002", "archived", Some(NOW.into()), "t");
    }
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(ids(&by(&engine, MentionedByQuery::new()).items), LIVE);
}

#[test]
fn mentioned_by_a_missing_or_deleted_target_is_not_found() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    for id in ["ab12.zzzz", "ab12.0005"] {
        let err = engine
            .mentioned_by(id, &MentionedByQuery::new())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound, "{id}");
    }
}

#[test]
fn mentioned_by_filters_by_type_and_by_container() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let q = |f: NextFilter| ids(&by(&engine, MentionedByQuery::new().filter(f)).items);

    assert_eq!(
        q(NextFilter::new().with_type("bug")),
        ["ab12.0002", "ab12.0004"]
    );
    assert_eq!(
        q(NextFilter::new().with_type("bug").with_type("task")),
        LIVE,
        "the types are alternatives"
    );
    // In e001: its child 0001 and its contributor 0003. In e002: contributor 0002, child 0003.
    assert_eq!(
        q(NextFilter::new().with_container("ab12.e001")),
        ["ab12.0001", "ab12.0003"]
    );
    assert_eq!(
        q(NextFilter::new().with_container("ab12.e002")),
        ["ab12.0002", "ab12.0003"]
    );
    assert_eq!(
        q(NextFilter::new()
            .with_container("ab12.e002")
            .with_type("task")),
        ["ab12.0003"]
    );
    assert!(q(NextFilter::new().with_type("epic")).is_empty());
}

// ---- Engine::mentioned_by: paging, as `next` pages ---------------------------------

#[test]
fn a_limit_alone_cuts_and_discloses_but_hands_out_no_token() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p = by(&engine, MentionedByQuery::new().limit(3));
    assert_eq!(
        (ids(&p.items), p.total),
        (LIVE[0..3].iter().map(|s| s.to_string()).collect(), 4)
    );
    assert!(p.truncated());
    assert_eq!(p.next_token, None);
}

#[test]
fn paginate_walks_the_whole_list_in_order() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = by(&engine, MentionedByQuery::new().limit(3).paginate());
    assert_eq!(ids(&p1.items), LIVE[0..3]);
    let t = p1.next_token.expect("a row remains");
    let p2 = by(&engine, MentionedByQuery::new().limit(3).token(&t));
    assert_eq!(
        (ids(&p2.items), p2.start, p2.total),
        (vec![LIVE[3].to_string()], 3, 4)
    );
    assert!(!p2.restarted);
    assert_eq!(p2.next_token, None, "the last page");
}

#[test]
fn removing_a_mention_of_a_cached_item_restarts_at_the_first_page() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = by(&engine, MentionedByQuery::new().limit(2).paginate());
    // The edge op names the mentioner on the page not yet served: it bears on the loaded list.
    engine
        .mention_remove(NOW, ACTOR, "ab12.0003", TARGET)
        .unwrap();
    let p = by(
        &engine,
        MentionedByQuery::new()
            .limit(2)
            .token(p1.next_token.unwrap()),
    );
    assert!(p.restarted, "the snapshot is stale");
    assert_eq!(
        (ids(&p.items), p.start, p.total),
        (vec!["ab12.0001".to_string(), "ab12.0002".to_string()], 0, 3)
    );
}

#[test]
fn an_outside_op_does_not_invalidate_and_a_new_mentioner_waits_for_a_fresh_query() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = by(&engine, MentionedByQuery::new().limit(2).paginate());
    // 0006 is not in the snapshot; its new mention of the target is not served by this pager.
    engine.mention_add(NOW, ACTOR, "ab12.0006", TARGET).unwrap();
    let p2 = by(
        &engine,
        MentionedByQuery::new()
            .limit(2)
            .token(p1.next_token.unwrap()),
    );
    assert!(!p2.restarted);
    assert_eq!(ids(&p2.items), ["ab12.0003", "ab12.0004"]);
    assert_eq!(p2.next_token, None);
    assert_eq!(
        by(&engine, MentionedByQuery::new()).total,
        5,
        "a fresh query sees it"
    );
}

#[test]
fn a_token_of_another_target_filter_or_read_restarts() {
    let tmp = workspace();
    board(tmp.path());
    let mut s = writer(tmp.path());
    item(&mut s, "ab12.t002", "feature");
    for from in ["ab12.0001", "ab12.0002", "ab12.0003"] {
        s.add_edge(from, "ab12.t002", EdgeKind::Mentions, "t");
    }
    let engine = Engine::open(None, tmp.path()).unwrap();
    let t = by(&engine, MentionedByQuery::new().limit(2).paginate())
        .next_token
        .unwrap();

    let other_target = engine
        .mentioned_by("ab12.t002", &MentionedByQuery::new().limit(2).token(&t))
        .unwrap();
    assert!(other_target.restarted, "another target");
    let other_filter = by(
        &engine,
        MentionedByQuery::new()
            .filter(NextFilter::new().with_type("task"))
            .limit(2)
            .token(&t),
    );
    assert!(other_filter.restarted, "another filter");
    let as_next = engine
        .next_query(&NextQuery::new(NOW).limit(2).token(&t))
        .unwrap();
    assert!(
        as_next.restarted,
        "a mentioned-by token is not a next token"
    );
    let garbage = by(&engine, MentionedByQuery::new().limit(2).token("nope"));
    assert!(garbage.restarted);
    assert_eq!(ids(&garbage.items), LIVE[0..2]);
}

#[test]
fn paginate_needs_a_limit_and_a_token_needs_paginate() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    for q in [
        MentionedByQuery::new().paginate(),
        MentionedByQuery::new().limit(0).paginate(),
    ] {
        let err = engine.mentioned_by(TARGET, &q).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
    }
    let mut q = MentionedByQuery::new().limit(2);
    q.token = Some("x.2".into());
    let err = engine.mentioned_by(TARGET, &q).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}

#[test]
fn mentioned_by_value_is_the_paging_envelope_of_list_records() {
    let tmp = workspace();
    board(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine
        .mentioned_by_value(TARGET, &MentionedByQuery::new().limit(2).paginate())
        .unwrap();
    assert_eq!(v["total"], 4);
    assert_eq!(v["restarted"], false);
    assert!(v["next_token"].is_string());
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], "ab12.0001");
    assert_eq!(items[0]["type"], "task");
    // Without paginate: no token keys, as for `next`.
    let plain = engine
        .mentioned_by_value(TARGET, &MentionedByQuery::new())
        .unwrap();
    assert_eq!(plain["total"], 4);
    assert!(plain.get("next_token").is_none());
}

// ---- the shared reverse lookup ------------------------------------------------------

#[test]
fn mentioners_of_answers_many_targets_in_one_read() {
    // THE reverse lookup `mentioned_by` and the derived ranking key `count = "mentioned_by"`
    // (6j6v.e354 §11) share.
    let tmp = workspace();
    board(tmp.path());
    let s = writer(tmp.path());
    let before = s.edge_read_stats();
    let m = read::mentioners_of(&s, &[TARGET, "ab12.e001", "ab12.e002", "ab12.zzzz"]).unwrap();
    assert_eq!(
        s.edge_read_stats().since(before),
        EdgeReadStats {
            mention_lookups: 1,
            ..EdgeReadStats::default()
        },
        "one reverse lookup for all four targets, and no per-item edge read"
    );
    assert_eq!(ids(&m[TARGET]), LIVE);
    assert_eq!(ids(&m["ab12.e001"]), ["ab12.0006"]);
    assert!(!m.contains_key("ab12.e002"), "nothing mentions it");
    assert!(!m.contains_key("ab12.zzzz"), "not an item, not an error");
}

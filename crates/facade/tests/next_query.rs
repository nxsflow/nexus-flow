//! `Engine::next_query` (6j6v.15ed): `next` narrowed by type and by container, cut by a limit, and
//! paged through a machine-local snapshot with a next-token. Tested at the seam the products take.

use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::read::{self, NextFilter, NextQuery};
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-06-17T00:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    tmp
}

/// Another writer on the workspace — what a second process, or a sync pull, is to the engine.
fn writer(dir: &Path) -> Store {
    workspace::discover(dir).unwrap().open_store().unwrap()
}

fn item(s: &mut Store, id: &str, ty: &str, priority: &str) {
    s.create_item(id, ty, id, "t");
    s.set_field(id, "priority", Some(priority.into()), "t");
}

/// Five ready tasks, priority 0..4 — ranked deterministically, in id order.
fn five(dir: &Path) -> Vec<String> {
    let mut s = writer(dir);
    let ids: Vec<String> = (0..5).map(|n| format!("ab12.000{n}")).collect();
    for (n, id) in ids.iter().enumerate() {
        item(&mut s, id, "task", &n.to_string());
    }
    ids
}

fn ids(items: &[nexus_flow_core::model::ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn page(engine: &Engine, q: NextQuery) -> read::NextPage {
    engine.next_query(&q).unwrap()
}

// ---- filters ---------------------------------------------------------------

/// Two containers and an item in both: it belongs to one (its parent) and contributes to the other.
fn containers(dir: &Path) {
    let mut s = writer(dir);
    item(&mut s, "ab12.e001", "epic", "2");
    item(&mut s, "ab12.e002", "epic", "2");
    item(&mut s, "ab12.0001", "task", "1"); // in e001 (parent) AND e002 (contributes)
    item(&mut s, "ab12.0002", "task", "1"); // in e002 (parent)
    item(&mut s, "ab12.0003", "bug", "0"); // in e001 (contributes)
    item(&mut s, "ab12.0004", "bug", "3"); // in nothing
    s.set_parent("ab12.0001", "ab12.e001", "t").unwrap();
    s.set_parent("ab12.0002", "ab12.e002", "t").unwrap();
    s.add_edge("ab12.0001", "ab12.e002", EdgeKind::ContributesTo, "t");
    s.add_edge("ab12.0003", "ab12.e001", EdgeKind::ContributesTo, "t");
}

/// The unfiltered ranked order, narrowed to `keep` — what a filter must answer: rows taken out,
/// nothing re-ranked.
fn ranked(engine: &Engine, keep: &[&str]) -> Vec<String> {
    ids(&engine.next(NOW).unwrap())
        .into_iter()
        .filter(|i| keep.contains(&i.as_str()))
        .collect()
}

#[test]
fn the_container_filter_takes_children_and_contributors_and_an_item_can_be_in_two() {
    let tmp = workspace();
    containers(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();

    let in_e1 = page(
        &engine,
        NextQuery::new(NOW).filter(NextFilter::new().with_container("ab12.e001")),
    );
    assert_eq!(
        ids(&in_e1.items),
        ranked(&engine, &["ab12.0001", "ab12.0003"]),
        "e001: its child and its contributor"
    );
    // The same, spelled out: priority 0 before priority 1 (review of #43, Test Quality #9).
    assert_eq!(ids(&in_e1.items), ["ab12.0003", "ab12.0001"]);
    let in_e2 = page(
        &engine,
        NextQuery::new(NOW).filter(NextFilter::new().with_container("ab12.e002")),
    );
    assert_eq!(
        ids(&in_e2.items),
        ranked(&engine, &["ab12.0001", "ab12.0002"]),
        "e002: its child and the item that belongs to e001 but contributes here"
    );
    assert_eq!(ids(&in_e2.items), ["ab12.0001", "ab12.0002"]);
    assert_eq!(in_e2.total, 2);
    assert!(in_e2.next_token.is_none() && !in_e2.restarted);
}

#[test]
fn the_type_filter_is_repeatable_and_keeps_the_ranked_order() {
    let tmp = workspace();
    containers(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();

    let bugs = page(
        &engine,
        NextQuery::new(NOW).filter(NextFilter::new().with_type("bug")),
    );
    assert_eq!(
        ids(&bugs.items),
        ranked(&engine, &["ab12.0003", "ab12.0004"])
    );
    assert_eq!(ids(&bugs.items), ["ab12.0003", "ab12.0004"]);
    let work = page(
        &engine,
        NextQuery::new(NOW).filter(NextFilter::new().with_type("task").with_type("bug")),
    );
    assert_eq!(
        ids(&work.items),
        ranked(
            &engine,
            &["ab12.0001", "ab12.0002", "ab12.0003", "ab12.0004"]
        )
    );
    assert_eq!(
        ids(&work.items),
        ["ab12.0003", "ab12.0001", "ab12.0002", "ab12.0004"]
    );
    // Both filters at once: the bugs in e001.
    let both = page(
        &engine,
        NextQuery::new(NOW).filter(
            NextFilter::new()
                .with_type("bug")
                .with_container("ab12.e001"),
        ),
    );
    assert_eq!(ids(&both.items), ["ab12.0003"]);
}

#[test]
fn the_store_free_path_filters_through_the_same_rule() {
    let tmp = workspace();
    containers(tmp.path());
    let s = writer(tmp.path());
    let cfg = nexus_flow_facade::plugin::load("issue-tracker").unwrap();
    let board = read::active_board(&s).unwrap();
    for filter in [
        NextFilter::new().with_container("ab12.e001"),
        NextFilter::new().with_container("ab12.e002"),
        NextFilter::new().with_type("bug"),
        NextFilter::new().with_type("epic").with_type("bug"),
    ] {
        assert_eq!(
            ids(&read::next_active_filtered(&cfg, &board, NOW, &filter).unwrap()),
            ids(&read::next_filtered(&cfg, &s, NOW, None, &filter).unwrap()),
            "{filter:?}"
        );
    }
    assert_eq!(
        ids(&read::next_active_filtered(
            &cfg,
            &board,
            NOW,
            &NextFilter::new().with_container("ab12.e002")
        )
        .unwrap())
        .len(),
        2
    );
}

// ---- paging ----------------------------------------------------------------

#[test]
fn a_limit_alone_cuts_and_discloses_but_hands_out_no_token() {
    let tmp = workspace();
    five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p = page(&engine, NextQuery::new(NOW).limit(2));
    assert_eq!(p.items.len(), 2);
    assert_eq!(p.total, 5);
    assert_eq!(p.next_token, None);
    assert!(!p.restarted);
}

#[test]
fn paginate_walks_the_whole_list_in_order() {
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();

    let p1 = page(&engine, NextQuery::new(NOW).limit(2).paginate());
    assert_eq!(
        (ids(&p1.items), p1.total, p1.start),
        (all[0..2].to_vec(), 5, 0)
    );
    let t1 = p1.next_token.expect("rows remain");
    let p2 = page(&engine, NextQuery::new(NOW).limit(2).token(&t1));
    assert_eq!(
        (ids(&p2.items), p2.total, p2.start),
        (all[2..4].to_vec(), 5, 2)
    );
    assert!(!p2.restarted);
    let t2 = p2.next_token.expect("rows remain");
    let p3 = page(&engine, NextQuery::new(NOW).limit(2).token(&t2));
    assert_eq!((ids(&p3.items), p3.start), (all[4..].to_vec(), 4));
    assert_eq!(p3.next_token, None, "the last page hands out no token");
    assert!(!p3.restarted);
    // A token can be asked again: the snapshot does not move on.
    assert_eq!(
        ids(&page(&engine, NextQuery::new(NOW).limit(2).token(&t1)).items),
        all[2..4]
    );
}

#[test]
fn an_op_on_an_item_in_the_snapshot_restarts_at_the_first_page() {
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = page(&engine, NextQuery::new(NOW).limit(2).paginate());
    let token = p1.next_token.unwrap();

    // A label on an item on a page not yet served: it bears on the loaded list.
    writer(tmp.path()).add_label(&all[3], "touched", "t");

    let p = page(&engine, NextQuery::new(NOW).limit(2).token(&token));
    assert!(p.restarted, "the snapshot is stale");
    assert_eq!(
        (ids(&p.items), p.start, p.total),
        (all[0..2].to_vec(), 0, 5)
    );
    let fresh = p.next_token.expect("a new token for the fresh list");
    assert_ne!(fresh, token);
    let p2 = page(&engine, NextQuery::new(NOW).limit(2).token(&fresh));
    assert!(!p2.restarted);
    assert_eq!(ids(&p2.items), all[2..4]);
}

#[test]
fn an_op_outside_the_snapshot_that_moves_an_item_in_it_restarts() {
    // An op whose TARGET is outside the snapshot but that changes the membership of an item in it:
    // ab12.0001 depends on a closed blocker; reopening the blocker blocks it.
    let tmp = workspace();
    let all = five(tmp.path());
    let mut s = writer(tmp.path());
    item(&mut s, "ab12.b001", "task", "4");
    s.add_edge(&all[1], "ab12.b001", EdgeKind::Dep, "t");
    s.set_field("ab12.b001", "status", Some("closed".into()), "t");
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = page(&engine, NextQuery::new(NOW).limit(2).paginate());
    assert_eq!(ids(&p1.items), all[0..2]);

    s.set_field("ab12.b001", "status", Some("open".into()), "t");

    let token = p1.next_token.unwrap();
    let p = page(&engine, NextQuery::new(NOW).limit(2).token(&token));
    assert!(p.restarted, "an item of the list left it");
    // The first page of the fresh list, cut at the limit, with a new token (Test Quality #8).
    // Fresh: the blocked item gone, the reopened blocker (priority 4) in.
    assert_eq!(
        (ids(&p.items), p.start, p.total),
        (vec![all[0].clone(), all[2].clone()], 0, 5)
    );
    let new_token = p.next_token.expect("a new token");
    assert_ne!(new_token, token);
}

#[test]
fn a_stale_token_restarts_even_when_the_fresh_list_fits_one_page() {
    // The restart at what would be the last page: the fresh first page holds everything, so the
    // restarted answer hands out no token.
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = page(&engine, NextQuery::new(NOW).limit(4).paginate());
    writer(tmp.path()).set_field(&all[4], "status", Some("closed".into()), "t");
    let p = page(
        &engine,
        NextQuery::new(NOW).limit(4).token(p1.next_token.unwrap()),
    );
    assert!(p.restarted);
    assert_eq!((ids(&p.items), p.total), (all[0..4].to_vec(), 4));
    assert_eq!(p.next_token, None);
}

#[test]
fn an_op_on_an_item_outside_the_snapshot_does_not_invalidate_it() {
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let p1 = page(&engine, NextQuery::new(NOW).limit(2).paginate());

    // A new item that would rank FIRST: it qualifies, but it is not in the loaded list.
    item(&mut writer(tmp.path()), "ab12.new0", "task", "0");

    let p2 = page(
        &engine,
        NextQuery::new(NOW).limit(2).token(p1.next_token.unwrap()),
    );
    assert!(!p2.restarted);
    assert_eq!((ids(&p2.items), p2.total), (all[2..4].to_vec(), 5));
    let p3 = page(
        &engine,
        NextQuery::new(NOW).limit(2).token(p2.next_token.unwrap()),
    );
    assert_eq!(ids(&p3.items), all[4..]);
    // It appears on the next fresh query.
    let fresh = page(&engine, NextQuery::new(NOW).limit(2).paginate());
    assert_eq!(fresh.total, 6);
    assert!(ids(&fresh.items).contains(&"ab12.new0".to_string()));
}

#[test]
fn a_malformed_or_unknown_token_restarts_at_the_first_page() {
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    for token in ["garbage", "", ".2", "01J00000000000000000000000.2", "x.y"] {
        let p = page(&engine, NextQuery::new(NOW).limit(2).token(token));
        assert!(p.restarted, "{token:?}");
        assert_eq!(
            (ids(&p.items), p.start),
            (all[0..2].to_vec(), 0),
            "{token:?}"
        );
        assert!(
            p.next_token.is_some(),
            "{token:?}: the limit stays in force"
        );
    }
    // A real token past the end of its list, or under another filter, is invalid too.
    let t = page(&engine, NextQuery::new(NOW).limit(2).paginate())
        .next_token
        .unwrap();
    let (id, _) = t.rsplit_once('.').unwrap();
    assert!(
        page(
            &engine,
            NextQuery::new(NOW).limit(2).token(format!("{id}.99"))
        )
        .restarted
    );
    let other = NextQuery::new(NOW)
        .filter(NextFilter::new().with_type("task"))
        .limit(2)
        .token(&t);
    assert!(page(&engine, other).restarted);
}

#[test]
fn paginate_needs_a_limit() {
    let tmp = workspace();
    five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    for q in [
        NextQuery::new(NOW).paginate(),
        NextQuery::new(NOW).limit(0).paginate(),
    ] {
        assert_eq!(
            engine.next_query(&q).unwrap_err().kind,
            ErrorKind::Validation
        );
    }
    assert_eq!(
        engine
            .next_query(&NextQuery::new("garbage"))
            .unwrap_err()
            .kind,
        ErrorKind::Validation
    );
}

#[test]
fn next_query_value_is_the_paging_envelope() {
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine
        .next_query_value(&NextQuery::new(NOW).limit(2).paginate())
        .unwrap();
    assert_eq!(v["total"], 5);
    assert_eq!(v["restarted"], false);
    assert!(v["next_token"].is_string());
    assert_eq!(v["items"][0]["id"], all[0].as_str());
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
}

// ---- review of #43 -----------------------------------------------------------

#[test]
fn a_huge_limit_with_a_valid_token_serves_the_rest_and_does_not_panic() {
    // Integrity #1: `pos + limit` overflowed on a valid token.
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let t = page(&engine, NextQuery::new(NOW).limit(2).paginate())
        .next_token
        .unwrap();
    let p = page(&engine, NextQuery::new(NOW).limit(usize::MAX).token(&t));
    assert!(!p.restarted);
    assert_eq!((ids(&p.items), p.start), (all[2..].to_vec(), 2));
    assert_eq!(p.next_token, None);
}

#[test]
fn tampered_tokens_restart_or_serve_from_the_position_they_name() {
    // Test Quality #5.
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let t = page(&engine, NextQuery::new(NOW).limit(2).paginate())
        .next_token
        .unwrap();
    let (id, _) = t.rsplit_once('.').unwrap();
    // Another sort: the snapshot was computed under the default one.
    let other_sort = NextQuery::new(NOW)
        .sort(read::SortKey::Id)
        .limit(2)
        .token(&t);
    assert!(page(&engine, other_sort).restarted, "another sort");
    // Position 0 is the first page, which a token never names.
    assert!(
        page(
            &engine,
            NextQuery::new(NOW).limit(2).token(format!("{id}.0"))
        )
        .restarted
    );
    // A position no answer minted, but inside the list: a token is an offset into the snapshot.
    let p = page(
        &engine,
        NextQuery::new(NOW).limit(2).token(format!("{id}.1")),
    );
    assert!(!p.restarted);
    assert_eq!((ids(&p.items), p.start), (all[1..3].to_vec(), 1));
    // A real token from another workspace names a snapshot this one does not hold.
    let other = workspace();
    five(other.path());
    let elsewhere = Engine::open(None, other.path()).unwrap();
    let foreign = page(&elsewhere, NextQuery::new(NOW).limit(2).paginate())
        .next_token
        .unwrap();
    assert!(page(&engine, NextQuery::new(NOW).limit(2).token(&foreign)).restarted);
}

#[test]
fn two_interleaved_pagers_each_walk_their_own_list() {
    // Test Quality #4.
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let by_id = |n: usize| NextQuery::new(NOW).sort(read::SortKey::Id).limit(n);
    let a1 = page(&engine, NextQuery::new(NOW).limit(2).paginate());
    let b1 = page(&engine, by_id(3).paginate());
    let a2 = page(
        &engine,
        NextQuery::new(NOW).limit(2).token(a1.next_token.unwrap()),
    );
    let b2 = page(&engine, by_id(3).token(b1.next_token.unwrap()));
    let a3 = page(
        &engine,
        NextQuery::new(NOW)
            .limit(2)
            .token(a2.next_token.clone().unwrap()),
    );
    assert!(!a2.restarted && !b2.restarted && !a3.restarted);
    assert_eq!(ids(&a2.items), all[2..4]);
    assert_eq!(ids(&a3.items), all[4..]);
    assert_eq!(ids(&b2.items), all[3..]);
}

#[test]
fn one_query_polled_past_its_cap_evicts_only_its_own_snapshots() {
    // Integrity #3 and Test Quality #3: the caps are wired through `next_page`, and a reader that
    // keeps asking for first pages of one query cannot restart another query's pager.
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let other = page(
        &engine,
        NextQuery::new(NOW)
            .sort(read::SortKey::Id)
            .limit(2)
            .paginate(),
    )
    .next_token
    .unwrap();
    let first = page(&engine, NextQuery::new(NOW).limit(2).paginate())
        .next_token
        .unwrap();
    for _ in 0..read::NEXT_CACHE_MAX_PER_QUERY {
        page(&engine, NextQuery::new(NOW).limit(2).paginate());
    }
    let p = page(&engine, NextQuery::new(NOW).limit(2).token(&first));
    assert!(p.restarted, "the poller's oldest snapshot was evicted");
    let q = page(
        &engine,
        NextQuery::new(NOW)
            .sort(read::SortKey::Id)
            .limit(2)
            .token(&other),
    );
    assert!(!q.restarted, "the other query's snapshot is untouched");
    assert_eq!(ids(&q.items), all[2..4]);
}

#[test]
fn a_cache_that_cannot_be_written_degrades_to_a_page_without_a_token() {
    // Integrity #2: a read must not fail because the cache cannot be written (a read-only db, a
    // lock held past the busy timeout). Here the table is gone, which fails the write the same way.
    let tmp = workspace();
    let all = five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    writer(tmp.path())
        .connection()
        .execute_batch("DROP TABLE next_page_cache;")
        .unwrap();
    let p = page(&engine, NextQuery::new(NOW).limit(2).paginate());
    assert_eq!((ids(&p.items), p.total), (all[0..2].to_vec(), 5));
    assert_eq!(p.next_token, None);
}

#[test]
fn next_query_value_carries_the_paging_keys_only_when_paginating() {
    // Code Quality #5: the Engine's envelope follows the query, as the CLI's follows the flag.
    let tmp = workspace();
    five(tmp.path());
    let engine = Engine::open(None, tmp.path()).unwrap();
    let v = engine
        .next_query_value(&NextQuery::new(NOW).limit(2))
        .unwrap();
    assert_eq!(v["total"], 5);
    assert!(
        v.get("next_token").is_none() && v.get("restarted").is_none(),
        "{v}"
    );
}

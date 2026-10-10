//! A server answers `next`, `blocked` and `deferred` as a replica does (6j6v.t1ym).
//!
//! The generated boards of the fold tests — tickets in every status, archived and deleted ones,
//! defer dates, dependencies and parents with removes, labels, thread links, custom fields — get
//! priorities, due dates, spread defer dates and `contributes_to` edges on top, so the plugin
//! ranking has something to order. Each board is
//! folded into a [`MemTable`] in a shuffled order with duplicates, as the relay hands it over. Then
//! the replica answers through its store (`read::next` + `next_to_value_with_custom`, what
//! `Engine::next_value` runs) and the server through its tables (`board::select` +
//! `records::active_board` + `read::next_active_value`), and the two must agree: the same rows in the
//! same order, and the same JSON, byte for byte. Likewise for `blocked` and `deferred`.

use nexus_flow_core::model::{EdgeKind, ItemRow};
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::read;
use nxs_fold_ddb::fold::{Folder, INDEX_DOMAIN};
use nxs_fold_ddb::layout::{row_key, WATERMARK_KEY};
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::table::Table;
use nxs_fold_ddb::write::{Cond, Write};
use nxs_fold_ddb::{board, records};
use nxs_foundation::change::Cell;

mod common;
use common::{delivered, id, merged, serve, Lcg, NOW};

/// A plugin that declares the generator's custom field, so the `custom` join is exercised, and
/// ranks by priority and due date.
fn cfg_with_fields() -> PluginConfig {
    toml::from_str(
        r#"
        name = "fields-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P0", "P1", "P2", "P3", "P4"]
        [types]
        list = ["task"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        order = [
            { field = "priority", dir = "asc" },
            { field = "due", dir = "asc", nulls = "last" },
        ]
        [presentation.list]
        columns = ["id"]

        [fields.estimate]
        type = "text"
        on = ["task"]
        "#,
    )
    .expect("the fixture parses")
}

/// A generated board with priorities, due dates and `contributes_to` edges on top. Its own Lcg, so
/// the shared generator's seed → board mapping, which the fold tests rely on, does not move.
fn gen_board(seed: u64) -> nexus_flow_core::store::Store {
    let mut s = merged(seed);
    let mut r = Lcg(seed ^ 0x7179);
    for i in 0..14 {
        match r.next(6) {
            0 => s.set_field(&id(i), "priority", Some(r.next(5).to_string()), "u"),
            1 => s.set_field(
                &id(i),
                "due",
                Some(format!("2026-11-{:02}", 1 + r.next(28))),
                "u",
            ),
            // `contributes_to` is neither a dependency nor a parent: it must hold nothing back,
            // promote nothing and name no parent, on either side.
            2 => s.add_edge(&id(i), &id(r.next(14)), EdgeKind::ContributesTo, "u"),
            // The shared generator defers to a single future date; these spread it, so the
            // deferred order is decided by the date and not only by the id tiebreak.
            3 => s.set_field(
                &id(i),
                "defer_until",
                Some(format!("2027-0{}-01T00:00:00Z", 1 + r.next(9))),
                "u",
            ),
            // Two dependencies at once, so a blocked ticket can list more than one blocker and
            // their order is compared.
            4 => {
                s.add_edge(&id(i), &id(r.next(14)), EdgeKind::Dep, "u");
                s.add_edge(&id(i), &id(r.next(14)), EdgeKind::Dep, "u");
            }
            _ => {}
        }
    }
    s
}

fn ids(items: &[ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn count(s: &nexus_flow_core::store::Store, sql: &str) -> usize {
    s.connection()
        .query_row(sql, [], |r| r.get::<_, i64>(0))
        .unwrap() as usize
}

/// What the generated boards held, so the comparison is known to have compared something worth
/// comparing — not only rows, but rows in the shapes an ordering or a join could get wrong.
#[derive(Default)]
struct Seen(std::collections::BTreeMap<&'static str, usize>);

impl Seen {
    fn add(&mut self, what: &'static str, n: usize) {
        *self.0.entry(what).or_default() += n;
    }

    fn assert_all(&self, expected: &[&'static str]) {
        for what in expected {
            let n = self.0.get(what).copied().unwrap_or(0);
            assert!(n > 0, "the generated boards have {what}: {:?}", self.0);
        }
    }
}

#[test]
fn the_server_answers_next_blocked_and_deferred_as_the_replica_does() {
    let folder = Folder::platform().unwrap();
    let cfgs = [plugin::load("issue-tracker").unwrap(), cfg_with_fields()];
    let mut seen = Seen::default();
    // Half the fold test's seeds: each board is read twice per plugin on top of the fold.
    for seed in 0..60 {
        let s = gen_board(seed);
        let t = serve(&s, seed, &folder);
        let ctx = format!("seed {seed}");

        let selection = ready(board::select(&t)).unwrap();
        let lanes = selection.board.lanes(NOW);
        let served = ready(records::active_board(&t, selection)).unwrap();
        // The input itself: every active ticket, with the same row and the same joins.
        let local = read::active_board(&s).unwrap();
        assert_eq!(served.tickets, local.tickets, "active tickets, {ctx}");
        for ticket in served.tickets.values() {
            seen.add("labelled tickets", usize::from(!ticket.labels.is_empty()));
            seen.add(
                "tickets with custom values",
                usize::from(!ticket.custom.is_empty()),
            );
            seen.add(
                "tickets with conversations",
                usize::from(ticket.conversations > 0),
            );
            seen.add(
                "tickets with a parent",
                usize::from(ticket.parent.is_some()),
            );
            seen.add(
                "tickets whose parent is not active",
                usize::from(
                    ticket
                        .parent
                        .as_ref()
                        .is_some_and(|p| !served.tickets.contains_key(&p.id)),
                ),
            );
            seen.add(
                "tickets whose parent is deleted or missing",
                usize::from(ticket.item.belongs_to.is_some() && ticket.parent.is_none()),
            );
        }
        seen.add(
            "present contributes_to edges",
            count(
                &s,
                "SELECT COUNT(*) FROM present_edges WHERE kind = 'contributes_to'",
            ),
        );
        // A next with a finishable started ticket AND a plain backlog row: tiers to order.
        seen.add(
            "boards with a tier-1 and a tier-3 candidate",
            usize::from(
                lanes
                    .candidates
                    .iter()
                    .any(|c| c.in_progress && c.finishable)
                    && lanes
                        .candidates
                        .iter()
                        .any(|c| !c.in_progress && c.promoter.is_none()),
            ),
        );
        // The closed-mask: open active tickets none of whose parents is active. They carry a
        // `parent_closed_reason` on the store path, and are kept out of `next` — which is why the
        // store-free records never carry that key.
        let masked: Vec<String> = served
            .tickets
            .values()
            .filter(|t| !read::parent_closed_reason(&s, &t.item).unwrap().is_empty())
            .map(|t| t.item.id.clone())
            .collect();
        seen.add("open tickets whose parents are all inactive", masked.len());

        for cfg in &cfgs {
            let ctx = format!("{ctx}, plugin {}", cfg.name);

            let want = read::next(cfg, &s, NOW, None).unwrap();
            assert_eq!(
                ids(&read::next_active(cfg, &served, NOW).unwrap()),
                ids(&want),
                "next order, {ctx}"
            );
            let value = read::next_active_value(cfg, &served, NOW).unwrap();
            assert_eq!(
                value,
                read::next_to_value_with_custom(cfg, &s, &want).unwrap(),
                "next JSON, {ctx}"
            );
            for m in &masked {
                assert!(!want.iter().any(|i| &i.id == m), "{m} is masked, {ctx}");
            }
            assert!(
                value
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r.get("parent_closed_reason").is_none()),
                "no next record is closed-masked, {ctx}"
            );
            seen.add("next rows", want.len());
            // Neighbours equal on every ranking field: only the id tiebreak orders them.
            seen.add(
                "next neighbours tied on priority and due",
                want.windows(2)
                    .filter(|w| {
                        w[0].status == w[1].status
                            && w[0].priority == w[1].priority
                            && w[0].due == w[1].due
                            && w[0].item_type == w[1].item_type
                    })
                    .count(),
            );

            let want = read::blocked(cfg, &s, None).unwrap();
            let got = read::blocked_active(cfg, &served);
            assert_eq!(
                got.iter()
                    .map(|r| (r.item.clone(), r.blockers.clone()))
                    .collect::<Vec<_>>(),
                want.iter()
                    .map(|r| (r.item.clone(), r.blockers.clone()))
                    .collect::<Vec<_>>(),
                "blocked rows and blockers, {ctx}"
            );
            assert_eq!(
                read::blocked_active_value(cfg, &served),
                read::blocked_to_value_with_custom(cfg, &s, &want).unwrap(),
                "blocked JSON, {ctx}"
            );
            seen.add("blocked rows", want.len());
            seen.add(
                "blocked rows with two or more blockers",
                want.iter().filter(|r| r.blockers.len() >= 2).count(),
            );

            let want = read::deferred(cfg, &s, NOW, None).unwrap();
            assert_eq!(
                ids(&read::deferred_active(cfg, &served, NOW).unwrap()),
                ids(&want),
                "deferred order, {ctx}"
            );
            assert_eq!(
                read::deferred_active_value(cfg, &served, NOW).unwrap(),
                read::items_in_order_value_with_custom(cfg, &s, &want).unwrap(),
                "deferred JSON, {ctx}"
            );
            seen.add("deferred rows", want.len());
            let dates: std::collections::BTreeSet<_> =
                want.iter().map(|i| i.defer_until.clone()).collect();
            seen.add(
                "deferred lanes with two distinct defer dates",
                usize::from(dates.len() >= 2),
            );
        }
    }
    seen.assert_all(&[
        "next rows",
        "blocked rows",
        "deferred rows",
        "labelled tickets",
        "tickets with custom values",
        "tickets with conversations",
        "tickets with a parent",
        "tickets whose parent is not active",
        "tickets whose parent is deleted or missing",
        "present contributes_to edges",
        "boards with a tier-1 and a tier-3 candidate",
        "open tickets whose parents are all inactive",
        "next neighbours tied on priority and due",
        "blocked rows with two or more blockers",
        "deferred lanes with two distinct defer dates",
    ]);
}

#[test]
fn a_ticket_reads_only_its_own_labels_custom_values_and_links() {
    // Each join is a per-ticket range: `ab12.0001` is a prefix of `ab12.00011`, and a `#` in an id
    // must not reach into the next key cell — neither may hand one ticket another's joins.
    use nexus_flow_core::model::{LinkRelation, LinkWeight, MergeStrategy};
    let folder = Folder::platform().unwrap();
    let mut s = nexus_flow_core::store::Store::open_in_memory(1);
    for i in ["ab12.0001", "ab12.00011", "ab12.0001#x"] {
        s.create_item(i, "task", "T", "u");
    }
    s.add_label("ab12.00011", "other", "u");
    s.add_label("ab12.0001#x", "hash", "u");
    s.add_label("ab12.0001", "mine", "u");
    s.add_label("ab12.0001", "gone", "u");
    s.remove_label("ab12.0001", "gone", "u");
    s.set_custom_field_merge(
        "ab12.00011",
        "estimate",
        Some("9".into()),
        "u",
        MergeStrategy::Lww,
    );
    s.set_custom_field_merge(
        "ab12.0001",
        "estimate",
        Some(String::new()),
        "u",
        MergeStrategy::Lww,
    );
    s.add_thread_link(
        "m-t1",
        "ab12.00011",
        LinkRelation::Cited,
        LinkWeight::Bearing,
        "u",
    );
    s.add_thread_link(
        "m-t1",
        "ab12.0001",
        LinkRelation::Cited,
        LinkWeight::Bearing,
        "u",
    );
    // The later link of the same thread wins, and it is passing.
    s.add_thread_link(
        "m-t1",
        "ab12.0001",
        LinkRelation::WorkedOn,
        LinkWeight::Passing,
        "u",
    );
    let t = serve(&s, 7, &folder);
    let served = ready(records::active_board(&t, ready(board::select(&t)).unwrap())).unwrap();
    let mine = &served.tickets["ab12.0001"];
    assert_eq!(mine.labels, ["mine"]);
    assert!(mine.custom.is_empty(), "an empty custom value is no value");
    assert_eq!(mine.conversations, 0);
    let other = &served.tickets["ab12.00011"];
    assert_eq!(other.labels, ["other"]);
    assert_eq!(other.custom.get("estimate").map(String::as_str), Some("9"));
    assert_eq!(other.conversations, 1);
    assert_eq!(served.tickets["ab12.0001#x"].labels, ["hash"]);
    assert_eq!(served.tickets, read::active_board(&s).unwrap().tickets);
}

#[test]
fn a_stream_folded_before_the_entries_is_refused_then_refolded_and_reads_its_labels() {
    // A stream an older release folded: the same rows, no entries under its tickets, and a
    // watermark that names no index revision.
    let folder = Folder::platform().unwrap();
    let mut seen_labels = 0;
    for seed in [3, 11, 27] {
        let s = gen_board(seed);
        let ops = delivered(&s, seed);
        let t = serve(&s, seed, &folder);
        for (sk, _) in t.rows() {
            if sk.starts_with(".ladj#") || sk.starts_with(".tadj#") {
                ready(t.delete(&sk)).unwrap();
            }
        }
        ready(t.write(&Write {
            sk: WATERMARK_KEY.into(),
            set: Vec::new(),
            remove: vec![format!("revision_{INDEX_DOMAIN}")],
            condition: Some(Cond::RowPresent),
        }))
        .unwrap();

        // Read as it is, it would answer without labels or conversations — so it is refused.
        let selection = ready(board::select(&t)).unwrap();
        match ready(records::active_board(&t, selection)) {
            Err(records::RecordsError::NotCurrent { found: None }) => {}
            other => panic!("seed {seed}: an old stream must be refused, got {other:?}"),
        }

        // The fold run's rule for a release with other rules, unchanged: not current, so clear the
        // stream and fold it again from position 0. No step of its own.
        let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
        assert!(!folder.is_current(&mark), "seed {seed}");
        ready(Folder::clear(&t)).unwrap();
        ready(folder.fold_batch(&t, &ops)).unwrap();
        let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
        assert!(folder.is_current(&mark), "seed {seed}");

        let served = ready(records::active_board(&t, ready(board::select(&t)).unwrap())).unwrap();
        assert_eq!(
            served.tickets,
            read::active_board(&s).unwrap().tickets,
            "seed {seed}"
        );
        seen_labels += served
            .tickets
            .values()
            .filter(|t| !t.labels.is_empty())
            .count();
    }
    assert!(
        seen_labels > 0,
        "the refolded streams have labelled tickets"
    );
}

#[test]
fn reading_the_records_costs_four_requests_per_ticket_and_one_per_outside_parent() {
    // Every label toggled many times and many thread links: the read does not grow with history.
    use nexus_flow_core::model::{LinkRelation, LinkWeight};
    let folder = Folder::platform().unwrap();
    let mut s = nexus_flow_core::store::Store::open_in_memory(1);
    let n = 12u64;
    s.create_item("ab12.p000", "task", "Closed parent", "u");
    s.set_field("ab12.p000", "status", Some("closed".into()), "u");
    for i in 0..n {
        let id = id(i);
        s.create_item(&id, "task", "T", "u");
        for round in 0..10 {
            s.add_label(&id, "churn", "u");
            if round < 9 {
                s.remove_label(&id, "churn", "u");
            }
            s.add_thread_link(
                &format!("m-t{round}"),
                &id,
                LinkRelation::Cited,
                LinkWeight::Bearing,
                "u",
            );
        }
        if i % 3 == 0 {
            s.set_field(&id, "status", Some("in_progress".into()), "u");
            s.add_edge(&id, "ab12.p000", EdgeKind::Parent, "u");
        }
    }
    let t = serve(&s, 1, &folder);
    let selection = ready(board::select(&t)).unwrap();
    let before = t.requests();
    let served = ready(records::active_board(&t, selection)).unwrap();
    let after = t.requests();
    assert_eq!(served.tickets.len(), n as usize);
    assert!(served.tickets.values().all(|t| t.labels == ["churn"]));
    assert!(served.tickets.values().all(|t| t.conversations == 10));
    // The watermark, a timestamps read per ticket, and the one parent outside the selection.
    assert_eq!(after.gets - before.gets, 1 + n + 1);
    // Custom values, labels and links: one Query each per ticket.
    assert_eq!(after.queries - before.queries, 3 * n);
    assert_eq!(after.index_queries, before.index_queries);
    assert_eq!(after.writes, before.writes);
    assert_eq!(served.tickets, read::active_board(&s).unwrap().tickets);
}

#[test]
fn the_parent_tiebreak_on_equal_coordinates_is_the_one_present_parent_picks() {
    // Two parent edges with the same (lamport, site) — ops that collided, which only a server folds
    // both of — and two without a coordinate (an older binary's rows). The winner is the highest
    // `to_id`, and a coordinate beats none: what `present_parent` picks from the same rows.
    let t = MemTable::new("stream-1");
    let put = |sk: String, cells: Vec<(&str, Cell)>| {
        ready(t.write(&Write::put(
            sk,
            cells.into_iter().map(|(c, v)| (c.to_string(), v)).collect(),
        )))
        .unwrap();
    };
    let txt = |v: &str| Cell::Text(v.to_string());
    for id in ["c", "d", "e", "p1", "p2", "q1", "q2", "r1", "r2"] {
        put(
            row_key("items", &[("id".into(), txt(id))]).unwrap(),
            vec![("id", txt(id)), ("status", txt("open"))],
        );
    }
    let edges: [(&str, &str, &str, Option<i64>, Option<i64>); 6] = [
        ("t1", "c", "p2", Some(5), Some(1)),
        ("t2", "c", "p1", Some(5), Some(1)),
        ("t3", "d", "q1", None, None),
        ("t4", "d", "q2", None, None),
        ("t5", "e", "r2", None, None),
        ("t6", "e", "r1", Some(0), Some(0)),
    ];
    let s = nexus_flow_core::store::Store::open_in_memory(1);
    for (tag, from, to, lamport, site) in edges {
        let int = |v: Option<i64>| v.map(Cell::Int).unwrap_or(Cell::Null);
        let mut cells = vec![
            ("tag", txt(tag)),
            ("from_id", txt(from)),
            ("to_id", txt(to)),
            ("kind", txt("parent")),
        ];
        cells.extend(lamport.map(|_| ("lamport", int(lamport))));
        cells.extend(site.map(|_| ("site", int(site))));
        put(
            row_key("edge_adds", &[("tag".into(), txt(tag))]).unwrap(),
            cells,
        );
        s.connection()
            .execute(
                "INSERT INTO edge_adds(from_id, to_id, kind, tag, lamport, site)
                 VALUES (?1, ?2, 'parent', ?3, ?4, ?5)",
                rusqlite::params![from, to, tag, lamport, site],
            )
            .unwrap();
    }
    ready(board::reindex(&t)).unwrap();
    let parents = ready(board::select(&t)).unwrap().parents;
    for child in ["c", "d", "e"] {
        let local: String = s
            .connection()
            .query_row(
                "SELECT parent_id FROM present_parent WHERE child_id = ?1",
                [child],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parents.get(child), Some(&local), "{child}");
    }
    assert_eq!(parents["c"], "p2");
    assert_eq!(parents["d"], "q2");
    assert_eq!(parents["e"], "r1");
}

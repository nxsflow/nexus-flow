//! A server answers `next`, `blocked` and `deferred` as a replica does (6j6v.t1ym).
//!
//! The generated boards of the fold tests — tickets in every status, archived and deleted ones,
//! defer dates, dependencies and parents with removes, labels, thread links, custom fields — get
//! priorities and due dates on top, so the plugin ranking has something to order. Each board is
//! folded into a [`MemTable`] in a shuffled order with duplicates, as the relay hands it over. Then
//! the replica answers through its store (`read::next` + `next_to_value_with_custom`, what
//! `Engine::next_value` runs) and the server through its tables (`board::select` +
//! `records::active_board` + `read::next_active_value`), and the two must agree: the same rows in the
//! same order, and the same JSON, byte for byte. Likewise for `blocked` and `deferred`.

use nexus_flow_core::model::EdgeKind;
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::read;
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::mem::ready;
use nxs_fold_ddb::{board, records};

mod common;
use common::{id, merged, serve, Lcg, NOW};

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

/// A generated board with priorities and due dates on some of its tickets.
fn board(seed: u64) -> nexus_flow_core::store::Store {
    let mut s = merged(seed);
    let mut r = Lcg(seed ^ 0x7179);
    for i in 0..14 {
        match r.next(3) {
            0 => s.set_field(&id(i), "priority", Some(r.next(5).to_string()), "u"),
            1 => s.set_field(
                &id(i),
                "due",
                Some(format!("2026-11-{:02}", 1 + r.next(28))),
                "u",
            ),
            _ => {}
        }
    }
    // Contributes-to edges (6j6v.15ed), some taken back: what the container filter reads beside
    // the parent. The fold tests' generator makes none.
    for i in 0..14 {
        if r.next(3) == 0 {
            let to = id(r.next(14));
            s.add_edge(&id(i), &to, EdgeKind::ContributesTo, "u");
            if r.next(4) == 0 {
                s.remove_edge(&id(i), &to, EdgeKind::ContributesTo, "u");
            }
        }
    }
    s
}

fn ids(items: &[nexus_flow_core::model::ItemRow]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

#[test]
fn the_server_answers_next_blocked_and_deferred_as_the_replica_does() {
    let folder = Folder::platform().unwrap();
    let cfgs = [plugin::load("issue-tracker").unwrap(), cfg_with_fields()];
    // What the boards held, so the comparison is known to have compared something.
    let (mut next_seen, mut blocked_seen, mut deferred_seen) = (0, 0, 0);
    let (mut labels_seen, mut custom_seen, mut conversations_seen, mut parents_seen) = (0, 0, 0, 0);
    let (mut outside_parents_seen, mut contributes_seen, mut filtered_seen) = (0, 0, 0);
    // Half the fold test's seeds: each board is read twice per plugin on top of the fold.
    for seed in 0..60 {
        let s = board(seed);
        let t = serve(&s, seed, &folder);
        let ctx = format!("seed {seed}");

        let served = ready(records::active_board(&t, ready(board::select(&t)).unwrap())).unwrap();
        // The input itself: every active ticket, with the same row and the same joins.
        let local = read::active_board(&s).unwrap();
        assert_eq!(served.tickets, local.tickets, "active tickets, {ctx}");
        for ticket in served.tickets.values() {
            labels_seen += usize::from(!ticket.labels.is_empty());
            custom_seen += usize::from(!ticket.custom.is_empty());
            conversations_seen += usize::from(ticket.conversations > 0);
            parents_seen += usize::from(ticket.parent.is_some());
            contributes_seen += usize::from(!ticket.contributes_to.is_empty());
            outside_parents_seen += usize::from(
                ticket
                    .parent
                    .as_ref()
                    .is_some_and(|p| !served.tickets.contains_key(&p.id)),
            );
        }

        for cfg in &cfgs {
            let ctx = format!("{ctx}, plugin {}", cfg.name);

            let want = read::next(cfg, &s, NOW, None).unwrap();
            assert_eq!(
                ids(&read::next_active(cfg, &served, NOW).unwrap()),
                ids(&want),
                "next order, {ctx}"
            );
            assert_eq!(
                read::next_active_value(cfg, &served, NOW).unwrap(),
                read::next_to_value_with_custom(cfg, &s, &want).unwrap(),
                "next JSON, {ctx}"
            );
            next_seen += want.len();

            // The container filter (6j6v.15ed): every ticket as a container, through both paths.
            for container in served.tickets.keys() {
                let filter = read::NextFilter::new().with_container(container.as_str());
                let want = read::next_filtered(cfg, &s, NOW, None, &filter).unwrap();
                assert_eq!(
                    ids(&read::next_active_filtered(cfg, &served, NOW, &filter).unwrap()),
                    ids(&want),
                    "next in {container}, {ctx}"
                );
                filtered_seen += want.len();
            }

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
            blocked_seen += want.len();

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
            deferred_seen += want.len();
        }
    }
    for (what, n) in [
        ("next rows", next_seen),
        ("blocked rows", blocked_seen),
        ("deferred rows", deferred_seen),
        ("labelled tickets", labels_seen),
        ("tickets with custom values", custom_seen),
        ("tickets with conversations", conversations_seen),
        ("tickets with a parent", parents_seen),
        ("tickets whose parent is not active", outside_parents_seen),
        ("tickets that contribute to another", contributes_seen),
        ("rows in a container", filtered_seen),
    ] {
        assert!(n > 0, "the generated boards have {what}");
    }
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

//! Plugin list fields (6j6v.4g6r): the entry OR-set and its merge semantics across two replicas.
//!
//! An entry is identified by its VALUE within its (item, field). Adds are tagged by their op id;
//! a remove tombstones the add tags its writer saw (observed-remove), so an add the remover did not
//! see survives — add wins. Several present adds of one value are one entry, labelled by the latest
//! add and placed by the earliest (`model::present_entries`).

use nexus_flow_core::model::{FieldEntry, Op};
use nexus_flow_core::store::Store;

const ITEM: &str = "a.p1";

fn entry(value: &str, label: Option<&str>) -> FieldEntry {
    FieldEntry {
        value: value.to_string(),
        label: label.map(str::to_string),
    }
}

fn entries(s: &Store, field: &str) -> Vec<FieldEntry> {
    s.field_entries_of(ITEM)
        .unwrap()
        .remove(field)
        .unwrap_or_default()
}

/// Two replicas holding the same item: A creates it, B receives it.
fn two_replicas() -> (Store, Store) {
    let mut a = Store::open_in_memory(1);
    let mut b = Store::open_in_memory(2);
    a.create_item(ITEM, "person", "Ada", "t");
    b.apply(&a.export());
    (a, b)
}

/// Exchange every op both ways.
fn sync(a: &mut Store, b: &mut Store) {
    let from_a = a.export();
    let from_b = b.export();
    a.apply(&from_b);
    b.apply(&from_a);
}

#[test]
fn entries_add_with_an_optional_label_in_insertion_order() {
    let mut s = Store::open_in_memory(1);
    s.create_item(ITEM, "person", "Ada", "t");
    s.add_field_entry(ITEM, "email", "ada@work.example", Some("work"), "t");
    s.add_field_entry(ITEM, "email", "ada@home.example", None, "t");
    s.add_field_entry(ITEM, "phone", "+49 30 1234", Some("private"), "t");
    assert_eq!(
        entries(&s, "email"),
        vec![
            entry("ada@work.example", Some("work")),
            entry("ada@home.example", None)
        ],
        "insertion order, not value order"
    );
    assert_eq!(
        entries(&s, "phone"),
        vec![entry("+49 30 1234", Some("private"))]
    );
    // The op shape: `field`-kind `add`, the label after U+001F.
    let adds: Vec<Op> = s
        .export()
        .into_iter()
        .filter(|o| o.target_kind == "field" && o.op_type == "add")
        .collect();
    assert_eq!(adds.len(), 3);
    assert_eq!(adds[0].target_id, ITEM);
    assert_eq!(adds[0].field, "email");
    assert_eq!(adds[0].value.as_deref(), Some("ada@work.example\u{1f}work"));
    assert_eq!(adds[1].value.as_deref(), Some("ada@home.example"));
}

#[test]
fn re_adding_a_value_relabels_it_in_place() {
    let mut s = Store::open_in_memory(1);
    s.create_item(ITEM, "person", "Ada", "t");
    s.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    s.add_field_entry(ITEM, "email", "b@x", None, "t");
    s.add_field_entry(ITEM, "email", "a@x", Some("private"), "t");
    assert_eq!(
        entries(&s, "email"),
        vec![entry("a@x", Some("private")), entry("b@x", None)],
        "one entry per value; the latest add's label; the first add's place"
    );
    s.add_field_entry(ITEM, "email", "a@x", None, "t");
    assert_eq!(entries(&s, "email")[0], entry("a@x", None), "unlabelled");
}

#[test]
fn remove_takes_the_value_whatever_its_label_and_clear_takes_all() {
    let mut s = Store::open_in_memory(1);
    s.create_item(ITEM, "person", "Ada", "t");
    s.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    s.add_field_entry(ITEM, "email", "a@x", Some("private"), "t");
    s.add_field_entry(ITEM, "email", "b@x", None, "t");
    s.add_field_entry(ITEM, "phone", "1", None, "t");
    s.remove_field_entry(ITEM, "email", "a@x", "t");
    assert_eq!(entries(&s, "email"), vec![entry("b@x", None)]);
    let before = s.op_count();
    s.remove_field_entry(ITEM, "email", "nobody@x", "t");
    assert_eq!(
        s.op_count(),
        before,
        "removing an absent value emits nothing"
    );

    s.clear_field_entries(ITEM, "email", "t");
    assert!(entries(&s, "email").is_empty());
    assert_eq!(
        entries(&s, "phone"),
        vec![entry("1", None)],
        "clear is per field"
    );
    assert!(
        !s.field_entries_of(ITEM).unwrap().contains_key("email"),
        "an emptied field is absent from the map"
    );
    // Re-adding after a remove is a new entry, placed last.
    s.add_field_entry(ITEM, "phone", "2", None, "t");
    s.add_field_entry(ITEM, "phone", "1", None, "t");
    assert_eq!(
        entries(&s, "phone"),
        vec![entry("1", None), entry("2", None)],
        "a still-present value keeps its place when re-added"
    );
}

#[test]
fn concurrent_adds_of_the_same_value_are_one_entry() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    b.add_field_entry(ITEM, "email", "a@x", Some("private"), "t");
    sync(&mut a, &mut b);
    let on_a = entries(&a, "email");
    assert_eq!(on_a, entries(&b, "email"), "both replicas converge");
    // Same lamport on both sides; the higher site (B = 2) wins the label.
    assert_eq!(on_a, vec![entry("a@x", Some("private"))]);
}

#[test]
fn remove_against_a_concurrent_re_add_add_wins() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    sync(&mut a, &mut b);
    // A removes the entry it sees; B, concurrently, adds the same value again.
    a.remove_field_entry(ITEM, "email", "a@x", "t");
    b.add_field_entry(ITEM, "email", "a@x", Some("private"), "t");
    sync(&mut a, &mut b);
    assert_eq!(entries(&a, "email"), entries(&b, "email"));
    assert_eq!(
        entries(&a, "email"),
        vec![entry("a@x", Some("private"))],
        "the add A did not observe survives its remove"
    );
}

#[test]
fn remove_against_a_concurrent_relabel_keeps_the_relabelled_entry() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    sync(&mut a, &mut b);
    a.remove_field_entry(ITEM, "email", "a@x", "t");
    b.add_field_entry(ITEM, "email", "a@x", Some("private"), "t");
    sync(&mut a, &mut b);
    assert_eq!(
        entries(&b, "email"),
        vec![entry("a@x", Some("private"))],
        "a relabel is an add, and add wins"
    );
}

#[test]
fn concurrent_add_and_remove_of_different_values_both_apply() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "email", "old@x", None, "t");
    sync(&mut a, &mut b);
    a.remove_field_entry(ITEM, "email", "old@x", "t");
    b.add_field_entry(ITEM, "email", "new@x", None, "t");
    sync(&mut a, &mut b);
    assert_eq!(entries(&a, "email"), vec![entry("new@x", None)]);
    assert_eq!(entries(&b, "email"), vec![entry("new@x", None)]);
}

#[test]
fn concurrent_removes_of_the_same_value_converge_to_absent() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "email", "a@x", None, "t");
    sync(&mut a, &mut b);
    a.remove_field_entry(ITEM, "email", "a@x", "t");
    b.remove_field_entry(ITEM, "email", "a@x", "t");
    sync(&mut a, &mut b);
    assert!(entries(&a, "email").is_empty());
    assert!(entries(&b, "email").is_empty());
}

#[test]
fn concurrent_relabels_converge_on_one_label() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "phone", "1", None, "t");
    a.add_field_entry(ITEM, "phone", "2", None, "t");
    sync(&mut a, &mut b);
    a.add_field_entry(ITEM, "phone", "1", Some("work"), "t");
    b.add_field_entry(ITEM, "phone", "1", Some("mobile"), "t");
    sync(&mut a, &mut b);
    let on_a = entries(&a, "phone");
    assert_eq!(on_a, entries(&b, "phone"));
    assert_eq!(
        on_a,
        vec![entry("1", Some("mobile")), entry("2", None)],
        "the higher (lamport, site) wins the label; the entry keeps its place"
    );
}

#[test]
fn clear_against_a_concurrent_add_keeps_only_the_unobserved_add() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(ITEM, "email", "a@x", None, "t");
    a.add_field_entry(ITEM, "email", "b@x", None, "t");
    sync(&mut a, &mut b);
    a.clear_field_entries(ITEM, "email", "t");
    b.add_field_entry(ITEM, "email", "c@x", None, "t");
    sync(&mut a, &mut b);
    assert_eq!(entries(&a, "email"), vec![entry("c@x", None)]);
    assert_eq!(entries(&b, "email"), vec![entry("c@x", None)]);
}

#[test]
fn a_remove_that_arrives_before_its_add_still_hides_it() {
    let mut a = Store::open_in_memory(1);
    a.create_item(ITEM, "person", "Ada", "t");
    a.add_field_entry(ITEM, "email", "a@x", None, "t");
    a.remove_field_entry(ITEM, "email", "a@x", "t");
    let mut ops = a.export();
    ops.reverse();
    let mut b = Store::open_in_memory(2);
    for op in ops {
        b.apply(&[op]);
    }
    assert!(entries(&b, "email").is_empty());
}

#[test]
fn malformed_entry_ops_are_stored_but_not_folded() {
    let mut s = Store::open_in_memory(1);
    s.create_item(ITEM, "person", "Ada", "t");
    let base = Op {
        op_id: "01JENTRY000000000000000001".into(),
        lamport: 50,
        site: 9,
        domain: "task".into(),
        target_kind: "field".into(),
        target_id: ITEM.into(),
        field: "email".into(),
        op_type: "add".into(),
        value: Some("ok@x".into()),
        author: "t".into(),
        wall_clock: String::new(),
        key_id: None,
        sig: None,
    };
    let bad = [
        Op {
            op_id: "01JENTRY000000000000000002".into(),
            lamport: 52,
            value: Some(String::new()),
            ..base.clone()
        },
        Op {
            op_id: "01JENTRY000000000000000003".into(),
            lamport: 53,
            value: Some("v\u{1f}".into()),
            ..base.clone()
        },
        Op {
            op_id: "01JENTRY000000000000000004".into(),
            lamport: 54,
            value: Some("v\u{1f}l\u{1f}x".into()),
            ..base.clone()
        },
        Op {
            op_id: "01JENTRY000000000000000005".into(),
            lamport: 55,
            value: None,
            ..base.clone()
        },
        Op {
            op_id: "01JENTRY000000000000000006".into(),
            lamport: 56,
            value: Some("x".repeat(nexus_flow_core::model::MAX_FIELD_ENTRY_BYTES + 1)),
            ..base.clone()
        },
        Op {
            op_id: "01JENTRY000000000000000007".into(),
            lamport: 57,
            target_id: "i".repeat(nexus_flow_core::model::MAX_CHUNK_ID_BYTES + 1),
            ..base.clone()
        },
        Op {
            op_id: "01JENTRY000000000000000008".into(),
            lamport: 58,
            op_type: "remove".into(),
            value: Some("t1\u{1f}\u{1f}t2".into()),
            ..base.clone()
        },
    ];
    s.apply(&bad);
    assert!(
        s.field_entries_of(ITEM).unwrap().is_empty(),
        "no malformed op folds an entry"
    );
    assert_eq!(
        s.connection()
            .query_row(
                "SELECT COUNT(*) FROM field_entries WHERE removed = 1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0,
        "nor a tombstone"
    );
    s.apply(std::slice::from_ref(&base));
    assert_eq!(entries(&s, "email"), vec![entry("ok@x", None)]);
}

#[test]
fn the_bulk_read_matches_the_single_read() {
    let mut s = Store::open_in_memory(1);
    s.create_item(ITEM, "person", "Ada", "t");
    s.create_item("a.p2", "person", "Bob", "t");
    s.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    s.add_field_entry("a.p2", "phone", "2", None, "t");
    let bulk = s.field_entries_of_bulk(&[ITEM, "a.p2", "a.none"]).unwrap();
    assert_eq!(bulk.get(ITEM), Some(&s.field_entries_of(ITEM).unwrap()));
    assert_eq!(bulk.get("a.p2"), Some(&s.field_entries_of("a.p2").unwrap()));
    assert!(!bulk.contains_key("a.none"));
}

/// Order independence over a pool of concurrent entry ops from three sites, as
/// `crdt_properties.rs` checks it for the other fold paths.
#[test]
fn any_apply_order_converges() {
    let mut a = Store::open_in_memory(1);
    let mut b = Store::open_in_memory(2);
    let mut c = Store::open_in_memory(3);
    a.create_item(ITEM, "person", "Ada", "t");
    a.add_field_entry(ITEM, "email", "a@x", Some("work"), "t");
    b.apply(&a.export());
    c.apply(&a.export());
    a.remove_field_entry(ITEM, "email", "a@x", "t");
    a.add_field_entry(ITEM, "email", "b@x", None, "t");
    b.add_field_entry(ITEM, "email", "a@x", Some("private"), "t");
    b.add_field_entry(ITEM, "email", "c@x", None, "t");
    c.clear_field_entries(ITEM, "email", "t");
    c.add_field_entry(ITEM, "email", "b@x", Some("home"), "t");
    let mut pool: Vec<Op> = a.export();
    for op in b.export().into_iter().chain(c.export()) {
        if !pool.iter().any(|p| p.op_id == op.op_id) {
            pool.push(op);
        }
    }
    let reference = {
        let mut r = Store::open_in_memory(9);
        r.apply(&pool);
        entries(&r, "email")
    };
    assert_eq!(
        reference
            .iter()
            .map(|e| e.value.as_str())
            .collect::<Vec<_>>(),
        vec!["a@x", "b@x", "c@x"]
    );
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for _ in 0..32 {
        let mut ops = pool.clone();
        for i in (1..ops.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ops.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        let mut r = Store::open_in_memory(9);
        for op in ops {
            r.apply(&[op]);
        }
        assert_eq!(entries(&r, "email"), reference);
    }
}

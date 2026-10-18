//! Typed references (6j6v.9tx3): a ref is a plain `set` / `add` whose value is an item id. The
//! store knows no declarations — it answers "which rows of field F hold value X" under one place
//! predicate (`model::ref_place_ok`); the facade decides which fields are refs. Nothing here moves
//! the task fold revision or adds a column: a based replica (6j6v.0cfa) must stay loadable.
use nexus_flow_core::model::{is_valid_ref_value, ref_place_ok, MergeStrategy, MAX_CHUNK_ID_BYTES};
use nexus_flow_core::store::Store;
use nexus_flow_core::task_reducer::TaskReducer;
use nxs_foundation::reducer::Reducer as _;

const M: &str = "a.m1"; // a meeting
const P1: &str = "a.p1"; // persons
const P2: &str = "a.p2";

fn two_replicas() -> (Store, Store) {
    let mut a = Store::open_in_memory(1);
    let mut b = Store::open_in_memory(2);
    for (id, ty) in [(M, "meeting"), (P1, "person"), (P2, "person")] {
        a.create_item(id, ty, id, "t");
    }
    b.apply(&a.export());
    (a, b)
}
fn sync(a: &mut Store, b: &mut Store) {
    let fa = a.export();
    let fb = b.export();
    a.apply(&fb);
    b.apply(&fa);
}
fn set(s: &mut Store, id: &str, field: &str, value: &str) {
    s.set_custom_field_merge(id, field, Some(value.to_string()), "t", MergeStrategy::Lww);
}
fn sources(s: &Store, field: &str, target: &str) -> Vec<String> {
    s.referenced_by_bulk(field, &[target])
        .unwrap()
        .remove(target)
        .unwrap_or_default()
}

#[test]
fn a_ref_value_is_id_shaped_and_a_place_is_bounded_like_a_list_entry() {
    assert!(is_valid_ref_value("a.p1"));
    assert!(
        is_valid_ref_value("nexus-flow-4oa.2"),
        "a foreign prefix may carry hyphens"
    );
    for bad in ["", "nodot", "a b.c", "a.b\u{1f}", "a.b\u{1e}"] {
        assert!(!is_valid_ref_value(bad), "{bad:?}");
    }
    let long = format!("a.{}", "x".repeat(MAX_CHUNK_ID_BYTES));
    assert!(!is_valid_ref_value(&long));
    assert!(ref_place_ok("a.m1", "organizer", "a.p1"));
    assert!(
        !ref_place_ok(&long, "organizer", "a.p1"),
        "a source id over the bound"
    );
    assert!(
        !ref_place_ok("a.m1", &"f".repeat(65), "a.p1"),
        "a field over the chunk-field grammar"
    );
    assert!(!ref_place_ok("a.m1", "organizer", "nodot"));
    assert!(
        !ref_place_ok("a.m\u{1f}1", "organizer", "a.p1"),
        "a source holding the entry separator"
    );
}

#[test]
fn referenced_by_bulk_unions_single_values_and_present_entries_by_equality() {
    let (mut a, _) = two_replicas();
    a.create_item("a.m2", "meeting", "m2", "t");
    a.create_item("a.m3", "meeting", "m3", "t");
    set(&mut a, M, "organizer", P1);
    a.add_field_entry("a.m2", "organizer", P1, None, "t"); // a list entry under the same field name
    set(&mut a, "a.m3", "organizer", "free text"); // not id-shaped: never a reference
    set(&mut a, "a.m3", "note", P1); // the store does not know declarations
    a.add_field_entry(M, "participants", P2, Some("host"), "t");
    a.remove_field_entry(M, "participants", P2, "t");
    let by = a.referenced_by_bulk("organizer", &[P1, P2]).unwrap();
    assert_eq!(by[P1], vec![M.to_string(), "a.m2".to_string()]);
    assert!(
        by.get(P2).is_none(),
        "a target nothing holds is absent, not an error"
    );
    assert_eq!(
        sources(&a, "note", P1),
        vec!["a.m3".to_string()],
        "equality on (field, value); the facade filters by declaration"
    );
    assert!(
        sources(&a, "participants", P2).is_empty(),
        "a removed entry is no reference"
    );
    assert!(sources(&a, "organizer", "free text").is_empty());
}

#[test]
fn a_source_whose_id_is_over_the_place_bound_is_not_a_referrer() {
    let (mut a, _) = two_replicas();
    let long = format!("a.{}", "m".repeat(MAX_CHUNK_ID_BYTES));
    a.create_item(&long, "meeting", "long", "t");
    set(&mut a, &long, "organizer", P1);
    assert_eq!(
        a.custom_fields_of(&long).unwrap()["organizer"],
        P1,
        "the value folds as any custom value"
    );
    assert!(
        sources(&a, "organizer", P1).is_empty(),
        "the place predicate excludes it, as entry_place_ok excludes its list entries"
    );
}

#[test]
fn a_concurrent_set_race_converges_and_the_lookup_follows_the_winner_on_both_replicas() {
    // Each replica folds its own op first and the other's second — both fold orders in one run.
    // Both ops carry the same Lamport (the logs were equal), so the register's (lamport, site)
    // rule makes B's value (site 2) the winner on both sides — asserted, not assumed.
    for rival in [P2, "free text"] {
        let (mut a, mut b) = two_replicas();
        set(&mut a, M, "organizer", P1);
        set(&mut b, M, "organizer", rival);
        sync(&mut a, &mut b);
        let va = a.custom_fields_of(M).unwrap()["organizer"].clone();
        let vb = b.custom_fields_of(M).unwrap()["organizer"].clone();
        assert_eq!(
            va, rival,
            "B's value wins: equal Lamport, higher site (rival {rival})"
        );
        assert_eq!(vb, rival, "and the same on B");
        for (s, name) in [(&a, "a"), (&b, "b")] {
            for candidate in [P1, P2, "free text"] {
                // An id winner is found under itself only. A non-id winner is found under NO
                // candidate — above all not under the loser P1 — because a non-id value is no
                // reference (`ref_place_ok`), whoever wrote it and whatever it replaced.
                let want: Vec<String> = if is_valid_ref_value(rival) && candidate == rival {
                    vec![M.to_string()]
                } else {
                    vec![]
                };
                assert_eq!(
                    sources(s, "organizer", candidate),
                    want,
                    "replica {name}, candidate {candidate}, rival {rival}"
                );
            }
        }
    }
}

#[test]
fn a_prefix_remap_rewrites_single_values_and_entry_values_and_keeps_the_label() {
    // CQ #1 of the 9tx3 review, pass 2: remap_prefix is Step 0 of a sync that got a
    // reassignment; a ref it did not rewrite would dangle after the refold.
    let mut s = Store::open_in_memory(1);
    s.create_item("old.m1", "meeting", "m", "t");
    s.create_item("old.p1", "person", "p", "t");
    s.create_item("old.p2", "person", "q", "t");
    set(&mut s, "old.m1", "organizer", "old.p1");
    s.add_field_entry("old.m1", "participants", "old.p2", Some("host"), "t");
    set(&mut s, "old.m1", "alias", "old.p1"); // a custom text field whose WHOLE value is an id: exact lookup remaps it
    set(&mut s, "old.m1", "note", "see old.p1 and old.p2"); // an id among other words: custom fields are not free text, unchanged
                                                            // A value the entry codec cannot split: left as it is, no panic (Integrity #3, pass 3).
    let mut raw = s
        .export()
        .into_iter()
        .find(|o| o.op_type == "add")
        .unwrap()
        .clone();
    raw.op_id = "x-raw-add".into();
    raw.lamport += 100;
    raw.site = 9;
    raw.value = Some("old.p1\u{1f}a\u{1f}b".into()); // two separators: the codec cannot split it; it HOLDS an id, so an arm that emptied or remapped it would show
    s.apply(&[raw]);
    s.remap_prefix("old", "new").unwrap();
    assert_eq!(s.custom_fields_of("new.m1").unwrap()["organizer"], "new.p1");
    assert_eq!(s.custom_fields_of("new.m1").unwrap()["alias"], "new.p1");
    assert_eq!(
        s.custom_fields_of("new.m1").unwrap()["note"],
        "see old.p1 and old.p2",
        "exact lookup, not the free-text rewrite"
    );
    assert_eq!(
        s.export()
            .iter()
            .find(|o| o.op_id == "x-raw-add")
            .unwrap()
            .value
            .as_deref(),
        Some("old.p1\u{1f}a\u{1f}b"),
        "unsplittable: byte-identical, neither emptied nor remapped"
    );
    let entries = s
        .field_entries_of("new.m1")
        .unwrap()
        .remove("participants")
        .unwrap();
    assert_eq!(
        entries,
        vec![nexus_flow_core::model::FieldEntry {
            value: "new.p2".into(),
            label: Some("host".into())
        }]
    );
    assert_eq!(
        sources(&s, "organizer", "new.p1"),
        vec!["new.m1".to_string()]
    );
    assert_eq!(
        sources(&s, "participants", "new.p2"),
        vec!["new.m1".to_string()]
    );
    assert!(
        sources(&s, "organizer", "old.p1").is_empty(),
        "nothing dangles"
    );
}

#[test]
fn the_lookup_uses_both_value_indexes() {
    // The precedent `per_item_read_indexes_exist_and_are_used` (core/src/schema.rs ~429): an
    // index that exists but is not planned is a scan. Run the lookup's own SQL through the planner.
    let s = Store::open_in_memory(1);
    let plan = |sql: &str| -> String {
        let mut st = s
            .connection()
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        st.query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ")
    };
    // The SQL the lookup really builds, not a hand copy of it (TQ #6, pass 3).
    let sql = Store::ref_lookup_sql(1)
        .replace("?1", "'f'")
        .replace('?', "'a.b'");
    let plan = plan(&sql);
    assert!(
        plan.contains("USING INDEX custom_fields_by_value"),
        "{plan}"
    );
    assert!(
        plan.contains("USING INDEX field_entries_by_value"),
        "{plan}"
    );
}

#[test]
fn a_file_backed_based_store_reopens_after_an_upgrade_without_a_rebuild() {
    // TQ #1 of the 9tx3 review, pass 2: the property this epic rests on is that a BASED store
    // written by the previous build OPENS on this one. A rebuild at open is refused on a based
    // store (NeedsHistory::Rebuild, foundation/store.rs bind_views) — so the open itself is the
    // proof, and a build that rebuilt when it found an index missing would turn this red.
    use nxs_foundation::image::ViewBase;
    let path = std::env::temp_dir().join(format!("nxf-based-{}.db", ulid::Ulid::new()));
    let p = path.to_str().unwrap();
    let (mut source, _) = two_replicas();
    set(&mut source, M, "organizer", P1);
    source.add_field_entry(M, "participants", P2, Some("host"), "t");
    let based = source
        .based_image(
            5,
            &[],
            vec![ViewBase {
                of: "flow".into(),
                domain: "task".into(),
                fold_revision: TaskReducer.fold_revision(),
            }],
        )
        .unwrap();
    {
        let mut s = Store::open(p, 2).unwrap();
        s.load_image(&based).unwrap();
        assert!(s.is_based());
        // The previous build had no value indexes.
        s.connection()
            .execute_batch("DROP INDEX custom_fields_by_value; DROP INDEX field_entries_by_value;")
            .unwrap();
    }
    let s = Store::open(p, 2)
        .expect("a based store opens on this build: nothing here rebuilds its views");
    assert!(s.is_based());
    let indexes: Vec<String> = {
        let mut st = s.connection()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name IN ('custom_fields_by_value', 'field_entries_by_value') ORDER BY name")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        indexes,
        ["custom_fields_by_value", "field_entries_by_value"]
    );
    assert_eq!(sources(&s, "organizer", P1), vec![M.to_string()]);
    assert_eq!(sources(&s, "participants", P2), vec![M.to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_concurrent_add_of_the_same_id_is_one_entry_and_one_source() {
    let (mut a, mut b) = two_replicas();
    a.add_field_entry(M, "participants", P1, Some("host"), "t");
    b.add_field_entry(M, "participants", P1, None, "t");
    sync(&mut a, &mut b);
    assert_eq!(a.field_entries_of(M).unwrap()["participants"].len(), 1);
    assert_eq!(sources(&a, "participants", P1), vec![M.to_string()]);
    assert_eq!(sources(&b, "participants", P1), vec![M.to_string()]);
}

#[test]
fn this_epic_moves_no_fold_revision_so_a_based_replica_stays_loadable() {
    // 6j6v.0cfa: a replica started from a based snapshot refuses a rebuild (NeedsHistory::Rebuild)
    // and an image at another revision (NeedsRevision). Typed references therefore ride the ops
    // and the columns that exist. If this number has to move, read epic 6j6v.9tx3 first.
    let s = Store::open_in_memory(1);
    assert_eq!(s.fold_revision_of("task"), Some(3));
    assert_eq!(TaskReducer.fold_revision(), 3);
}

#[test]
fn a_replica_bound_from_a_based_snapshot_holds_its_refs_and_answers_the_lookup() {
    use nxs_foundation::image::ViewBase;
    let (mut source, _) = two_replicas();
    set(&mut source, M, "organizer", P1);
    source.add_field_entry(M, "participants", P2, Some("host"), "t");
    let based = source
        .based_image(
            5,
            &[],
            vec![ViewBase {
                of: "flow".into(),
                domain: "task".into(),
                fold_revision: TaskReducer.fold_revision(),
            }],
        )
        .unwrap();
    let mut s = Store::open_in_memory(2);
    s.load_image(&based).unwrap();
    assert!(s.is_based());
    assert_eq!(s.custom_fields_of(M).unwrap()["organizer"], P1);
    assert_eq!(sources(&s, "organizer", P1), vec![M.to_string()]);
    assert_eq!(sources(&s, "participants", P2), vec![M.to_string()]);
    set(&mut s, M, "organizer", P2); // and it keeps writing after the bind
    assert_eq!(sources(&s, "organizer", P2), vec![M.to_string()]);
    assert!(sources(&s, "organizer", P1).is_empty());
}

#[test]
fn an_existing_workspace_gains_the_value_indexes_at_open_without_a_rebuild() {
    let path = std::env::temp_dir().join(format!("nxf-refs-{}.db", ulid::Ulid::new()));
    let p = path.to_str().unwrap();
    {
        // The build before this one: the views without the two indexes, an id-shaped value on a
        // field and a list entry, the watermark at the boundary at revision 3.
        let mut s = Store::open(p, 1).unwrap();
        s.create_item(M, "meeting", "m", "t");
        s.create_item(P1, "person", "p", "t");
        set(&mut s, M, "organizer", P1);
        s.add_field_entry(M, "participants", P1, None, "t");
        s.connection()
            .execute_batch("DROP INDEX custom_fields_by_value; DROP INDEX field_entries_by_value;")
            .unwrap();
    }
    let s = Store::open(p, 1).unwrap();
    let indexes: Vec<String> = {
        let mut st = s.connection()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name IN ('custom_fields_by_value', 'field_entries_by_value') ORDER BY name")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        indexes,
        ["custom_fields_by_value", "field_entries_by_value"]
    );
    // (The watermark's revision is 3 whether or not a rebuild ran on a full store — the
    // file-backed BASED test below is what proves no rebuild happens, because there it is refused.)
    assert_eq!(sources(&s, "organizer", P1), vec![M.to_string()]);
    assert_eq!(sources(&s, "participants", P1), vec![M.to_string()]);
    let _ = std::fs::remove_file(&path);
}

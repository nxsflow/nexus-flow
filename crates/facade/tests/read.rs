//! Direct coverage of the facade read-compute layer + canonical `to_value` parity (E5 #9t7.1 /
//! E9 #49g). The CLI integration suite proves byte-parity through the `nxf` binary; these prove
//! the same records and JSON shapes at the library boundary an embedding app consumes — without
//! a clap/CLI dependency in sight.

use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::{ItemReadStats, Store};
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::{plugin, read};

const NOW: &str = "2026-06-17T00:00:00Z";

fn cfg() -> plugin::PluginConfig {
    plugin::load("issue-tracker").unwrap()
}

fn open_task(s: &mut Store, id: &str, title: &str, priority: &str) {
    s.create_item(id, "task", title, "t");
    s.set_field(id, "priority", Some(priority.to_string()), "t");
}

#[test]
fn next_default_is_the_ready_set_and_excludes_blocked() {
    // 0fc: the removed `ready` verb folded into `next` (its default — in_progress off — IS the ready
    // set). With equal priority the rank tiebreak is id, so the order matches the old id-sort here.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0002", "B", "0");
    open_task(&mut s, "ab12.0001", "A", "0");
    let ids: Vec<_> = read::next(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ids, ["ab12.0001", "ab12.0002"]);

    // A depends on B ⇒ A is blocked, only B stays in the ready (next) set.
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t");
    let ready: Vec<_> = read::next(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ready, ["ab12.0002"]);
}

#[test]
fn blocked_carries_its_open_blockers() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    open_task(&mut s, "ab12.0002", "B", "0");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t");

    let rows = read::blocked(&cfg(), &s, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.id, "ab12.0001");
    assert_eq!(
        rows[0].blockers,
        vec![("ab12.0002".to_string(), "open".to_string())]
    );

    // JSON shape: the canonical record with an appended `blockers` list.
    let v = read::blocked_to_value(&rows);
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["id"], "ab12.0001");
    assert_eq!(arr[0]["blockers"][0]["id"], "ab12.0002");
    assert_eq!(arr[0]["blockers"][0]["status"], "open");
}

#[test]
fn next_always_includes_claimed_work_and_ranks_it_first() {
    // C4 (#916.3) as amended by the finish-first tiers: claimed work is ALWAYS in `next` (the
    // `include_in_progress` flag is gone — hiding started work is what let a finished-but-open epic
    // vanish). A claimed, childless item is Tier 1, ahead of the higher-priority open backlog.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "open-high", "0"); // open, highest priority
    open_task(&mut s, "ab12.0002", "claimed-low", "2"); // in_progress, lower priority
    s.set_field("ab12.0002", "status", Some("in_progress".into()), "t");

    let ids: Vec<_> = read::next(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0001"],
        "claimed work is present by default and finishes first"
    );

    // Ranked order is preserved in the JSON projection (NOT re-sorted by id).
    let v = read::next_to_value(&s, &read::next(&cfg(), &s, NOW, None).unwrap()).unwrap();
    let ids: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["ab12.0002", "ab12.0001"]);
}

// ---- sp6.5: status / type precedence in the next ranking -------------------

#[test]
fn next_breaks_priority_ties_by_type_precedence() {
    // sp6.5: within one priority band, issue-tracker ranks by its declared type precedence
    // (epic → bug → decision → feature → chore) — an enum-index sort, not lexicographic on the
    // type bytes. Seeded in a scrambled id order, all same priority, no due, so only type decides.
    let mut s = Store::open_in_memory(1);
    for (id, ty) in [
        ("ab12.0003", "feature"),
        ("ab12.0001", "epic"),
        ("ab12.0005", "chore"),
        ("ab12.0002", "bug"),
        ("ab12.0004", "decision"),
    ] {
        s.create_item(id, ty, ty, "t");
        s.set_field(id, "priority", Some("1".into()), "t");
    }
    let order: Vec<String> = read::next(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.item_type.unwrap())
        .collect();
    assert_eq!(
        order,
        ["epic", "bug", "decision", "feature", "chore"],
        "type precedence breaks the priority tie"
    );
}

#[test]
fn next_ranks_priority_above_type() {
    // The type tier is SUBORDINATE to priority (chain order status → priority → type → due → id): a
    // high-priority `chore` still outranks a low-priority `epic`, proving type does not jump ahead.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "chore", "hi chore", "t");
    s.set_field("ab12.0001", "priority", Some("0".into()), "t"); // P0
    s.create_item("ab12.0002", "epic", "lo epic", "t");
    s.set_field("ab12.0002", "priority", Some("4".into()), "t"); // P4
    let order: Vec<String> = read::next(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        order,
        ["ab12.0001", "ab12.0002"],
        "priority outranks type — the P0 chore leads the P4 epic"
    );
}

#[test]
fn personal_todo_does_not_rank_by_type() {
    // The contrast (sp6.5): personal-todo declares NO type precedence, so two same-priority items
    // of different types fall straight through to the id tiebreaker — type never reorders them.
    let pt = plugin::load("personal-todo").unwrap();
    let mut s = Store::open_in_memory(1);
    // `termin` would precede `todo` lexicographically-reversed, but with no type key the id wins.
    s.create_item("ab12.0002", "termin", "a termin", "t");
    s.set_field("ab12.0002", "priority", Some("1".into()), "t");
    s.create_item("ab12.0001", "todo", "a todo", "t");
    s.set_field("ab12.0001", "priority", Some("1".into()), "t");
    let order: Vec<String> = read::next(&pt, &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(order, ["ab12.0001", "ab12.0002"], "id tiebreak, not type");
}

// ---- #916.7: parent (belongs_to) resolved in next / prime ------------------

#[test]
fn next_json_resolves_a_live_parent_and_is_null_otherwise() {
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "project", "Ship v1", "t");
    open_task(&mut s, "ab12.0002", "child", "0");
    s.set_parent("ab12.0002", "ab12.0001", "t").unwrap();
    open_task(&mut s, "ab12.0003", "orphan", "0"); // no parent

    let items = read::next(&cfg(), &s, NOW, None).unwrap();
    let v = read::next_to_value(&s, &items).unwrap();
    let by_id = |id: &str| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()
            .clone()
    };

    let child = by_id("ab12.0002");
    assert_eq!(child["parent"]["id"], "ab12.0001");
    assert_eq!(child["parent"]["title"], "Ship v1");
    assert_eq!(
        child["parent"]["type"], "project",
        "parent.type is the RAW core type, not a plugin label"
    );
    // The raw belongs_to id is still on the record (parent is additive).
    assert_eq!(child["belongs_to"], "ab12.0001");

    assert_eq!(
        by_id("ab12.0003")["parent"],
        serde_json::Value::Null,
        "an item with no parent has parent: null"
    );
}

#[test]
fn next_json_parent_is_the_deterministic_projected_parent_for_a_multi_parent_item() {
    // ②c (sp6.7): with the n:m substrate a child can hold several parent edges, but the canonical
    // `--json` record keeps a SINGLE `parent` object (the E2 §7 contract is unchanged). It is the
    // `present_parent` projection — the deterministic LWW winner by (lamport, site, to_id) — so the
    // output is byte-stable regardless of how many parents the substrate holds.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "project", "First parent", "t");
    s.create_item("ab12.0002", "project", "Second parent", "t");
    open_task(&mut s, "ab12.0003", "multi-parent child", "0");
    // Two parent edges in the OR-set (the core is n:m-capable; the facade matrix would gate this,
    // but the store is meaning-free). 0002 is added last → higher (lamport,site) → the winner.
    s.add_parent("ab12.0003", "ab12.0001", "t");
    s.add_parent("ab12.0003", "ab12.0002", "t");
    assert_eq!(
        s.parents_of("ab12.0003").len(),
        2,
        "the substrate really holds two parent edges"
    );

    let items = read::next(&cfg(), &s, NOW, None).unwrap();
    let v = read::next_to_value(&s, &items).unwrap();
    let child = v
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "ab12.0003")
        .unwrap()
        .clone();
    assert_eq!(
        child["parent"]["id"], "ab12.0002",
        "the single projected parent is the deterministic winner"
    );
    assert_eq!(child["parent"]["title"], "Second parent");
    assert_eq!(
        child["belongs_to"], "ab12.0002",
        "belongs_to mirrors the projection"
    );
    assert!(
        child["parent"].is_object(),
        "still ONE parent object, not an array — contract held"
    );
}

#[test]
fn next_json_parent_is_null_for_a_deleted_parent_but_keeps_belongs_to() {
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "project", "gone", "t");
    open_task(&mut s, "ab12.0002", "child", "0");
    s.set_parent("ab12.0002", "ab12.0001", "t").unwrap();
    s.delete_item("ab12.0001", "t");

    let items = read::next(&cfg(), &s, NOW, None).unwrap();
    let v = read::next_to_value(&s, &items).unwrap();
    let child = v.as_array().unwrap()[0].clone();
    assert_eq!(
        child["parent"],
        serde_json::Value::Null,
        "a deleted parent resolves to null"
    );
    assert_eq!(
        child["belongs_to"], "ab12.0001",
        "the raw belongs_to id is preserved even when the parent is gone"
    );
}

#[test]
fn prime_next_block_carries_the_same_parent_shape() {
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "project", "Ship v1", "t");
    open_task(&mut s, "ab12.0002", "child", "0");
    s.set_parent("ab12.0002", "ab12.0001", "t").unwrap();

    let report = read::prime(&cfg(), &s, NOW, false).unwrap();
    let v = report.to_value();
    let child = v["next"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "ab12.0002")
        .unwrap();
    assert_eq!(child["parent"]["id"], "ab12.0001");
    assert_eq!(child["parent"]["title"], "Ship v1");
    assert_eq!(child["parent"]["type"], "project");
}

// ---- C2 (#916.2): the ordering axis — per-verb default + `--sort` override ----

#[test]
fn list_defaults_to_id_and_sort_rank_overrides() {
    let mut s = Store::open_in_memory(1);
    // Priority makes rank order (priority asc) differ from id order.
    open_task(&mut s, "ab12.0001", "low", "2"); // P2
    open_task(&mut s, "ab12.0002", "high", "0"); // P0
    open_task(&mut s, "ab12.0003", "mid", "1"); // P1

    let def: Vec<_> = read::list(&cfg(), &s, None, None, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        def,
        ["ab12.0001", "ab12.0002", "ab12.0003"],
        "list defaults to id (so list --json stays plugin-independent)"
    );

    let by_rank: Vec<_> = read::list(&cfg(), &s, None, None, Some(read::SortKey::Rank))
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        by_rank,
        ["ab12.0002", "ab12.0003", "ab12.0001"],
        "--sort rank overrides to the plugin's priority order"
    );
}

#[test]
fn blocked_defaults_to_rank_not_id() {
    // The C2 fix: `blocked` ranked by default (it was id-ordered before).
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "low blocked", "2"); // P2
    open_task(&mut s, "ab12.0002", "high blocked", "0"); // P0
    open_task(&mut s, "ab12.0009", "open blocker", "0");
    s.add_edge("ab12.0001", "ab12.0009", EdgeKind::Dep, "t");
    s.add_edge("ab12.0002", "ab12.0009", EdgeKind::Dep, "t");

    let def: Vec<_> = read::blocked(&cfg(), &s, None)
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect();
    assert_eq!(
        def,
        ["ab12.0002", "ab12.0001"],
        "blocked default is rank (high priority first), not id"
    );

    let by_id: Vec<_> = read::blocked(&cfg(), &s, Some(read::SortKey::Id))
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect();
    assert_eq!(by_id, ["ab12.0001", "ab12.0002"], "--sort id overrides");
}

#[test]
fn next_sort_id_overrides_the_rank_default() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0002", "high", "0"); // P0
    open_task(&mut s, "ab12.0001", "low", "2"); // P2

    let def: Vec<_> = read::next(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(def, ["ab12.0002", "ab12.0001"], "next default is rank");

    let by_id: Vec<_> = read::next(&cfg(), &s, NOW, Some(read::SortKey::Id))
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        by_id,
        ["ab12.0001", "ab12.0002"],
        "--sort id overrides next"
    );
}

#[test]
fn parse_sort_accepts_known_keys_and_rejects_others() {
    assert_eq!(read::parse_sort("rank").unwrap(), read::SortKey::Rank);
    assert_eq!(read::parse_sort("id").unwrap(), read::SortKey::Id);
    let err = read::parse_sort("bogus").unwrap_err();
    assert_eq!(err.kind, nexus_flow_facade::error::ErrorKind::Validation);
}

#[test]
fn show_gathers_deps_contributes_and_notes() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    open_task(&mut s, "ab12.0002", "B", "0");
    open_task(&mut s, "ab12.0003", "C", "0");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t"); // A depends on B
    s.add_edge("ab12.0001", "ab12.0003", EdgeKind::ContributesTo, "t"); // A contributes to C
    s.add_note("ab12.0001", "first note", "t");

    let rec = read::show(&s, "ab12.0001").unwrap();
    assert_eq!(rec.item.id, "ab12.0001");
    assert_eq!(
        rec.deps,
        vec![("ab12.0002".to_string(), Some("open".to_string()))]
    );
    assert_eq!(rec.contributes_to, vec!["ab12.0003".to_string()]);
    assert_eq!(rec.notes.len(), 1);
    assert_eq!(rec.notes[0].1, "first note");

    // The `show` JSON object: { item, deps, contributes_to, notes }.
    let v = rec.to_value();
    assert_eq!(v["item"]["id"], "ab12.0001");
    assert_eq!(v["deps"][0]["id"], "ab12.0002");
    assert_eq!(v["deps"][0]["status"], "open");
    assert_eq!(v["contributes_to"][0], "ab12.0003");
    assert_eq!(v["notes"][0]["body"], "first note");
}

#[test]
fn show_missing_item_is_not_found() {
    let s = Store::open_in_memory(1);
    let err = read::show(&s, "ab12.9999").unwrap_err();
    assert_eq!(err.kind, nexus_flow_facade::error::ErrorKind::NotFound);
}

// ---- 6j6v.zvd0: parent-pointer notice on show ------------------------------------------------

#[test]
fn show_of_a_parentless_item_omits_parents_and_notice() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "Standalone", "0");

    let rec = read::show(&s, "ab12.0001").unwrap();
    assert!(rec.parents.is_empty());
    assert!(rec.parents_notice.is_none());

    let v = rec.to_value();
    assert!(v.get("parents").is_none(), "no parents key when unset: {v}");
    assert!(
        v.get("parents_notice").is_none(),
        "no parents_notice key when unset: {v}"
    );
}

#[test]
fn show_of_an_item_with_one_parent_carries_parents_and_notice() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "Epic", "0");
    open_task(&mut s, "ab12.0002", "Child", "0");
    s.add_parent("ab12.0002", "ab12.0001", "t");

    let rec = read::show(&s, "ab12.0002").unwrap();
    assert_eq!(rec.parents, vec!["ab12.0001".to_string()]);
    assert_eq!(
        rec.parents_notice.as_deref(),
        Some(
            "This item has the following parents: ab12.0001. URGENT RECOMMENDATION: ALSO READ \
             THESE ITEMS TO GET THE COMPLETE PICTURE!!!"
        )
    );

    let v = rec.to_value();
    assert_eq!(v["parents"], serde_json::json!(["ab12.0001"]));
    assert_eq!(
        v["parents_notice"],
        "This item has the following parents: ab12.0001. URGENT RECOMMENDATION: ALSO READ \
         THESE ITEMS TO GET THE COMPLETE PICTURE!!!"
    );
}

#[test]
fn show_of_an_item_with_multiple_parents_lists_all_ids_comma_separated() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "Epic one", "0");
    open_task(&mut s, "ab12.0002", "Epic two", "0");
    open_task(&mut s, "ab12.0003", "Child", "0");
    s.add_parent("ab12.0003", "ab12.0002", "t");
    s.add_parent("ab12.0003", "ab12.0001", "t"); // added second, but ids are sorted

    let rec = read::show(&s, "ab12.0003").unwrap();
    assert_eq!(
        rec.parents,
        vec!["ab12.0001".to_string(), "ab12.0002".to_string()]
    );
    assert_eq!(
        rec.parents_notice.as_deref(),
        Some(
            "This item has the following parents: ab12.0001, ab12.0002. URGENT \
             RECOMMENDATION: ALSO READ THESE ITEMS TO GET THE COMPLETE PICTURE!!!"
        )
    );
}

#[test]
fn show_of_an_item_with_only_a_contributes_to_edge_omits_parents_and_notice() {
    // Test Quality #1 (PR #255 review): the load-bearing parent-vs-contributes_to distinction
    // wasn't pinned by a test — a `contributes_to` edge is NOT a parent, and must never trigger
    // the notice, even though both edges point "upward" from the item's perspective.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "Birthday plan", "0");
    open_task(&mut s, "ab12.0002", "Cream", "0");
    s.add_edge("ab12.0002", "ab12.0001", EdgeKind::ContributesTo, "t");

    let rec = read::show(&s, "ab12.0002").unwrap();
    assert!(
        rec.parents.is_empty(),
        "a contributes_to edge is not a parent"
    );
    assert!(rec.parents_notice.is_none());

    let v = rec.to_value();
    assert!(v.get("parents").is_none(), "no parents key: {v}");
    assert!(
        v.get("parents_notice").is_none(),
        "no parents_notice key: {v}"
    );
}

#[test]
fn show_omits_a_deleted_parent_from_parents_and_notice() {
    // Integrity #2 (PR #255 review): `parents`/`parents_notice` must be live-only, mirroring the
    // sibling `parent_closed_reason` join (which already skips deleted/missing parent ids) — a
    // tombstoned parent should never be surfaced as something the reader is told to go read.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "Deleted epic", "0");
    open_task(&mut s, "ab12.0002", "Live epic", "0");
    open_task(&mut s, "ab12.0003", "Child", "0");
    s.add_parent("ab12.0003", "ab12.0001", "t");
    s.add_parent("ab12.0003", "ab12.0002", "t");
    s.delete_item("ab12.0001", "t");

    let rec = read::show(&s, "ab12.0003").unwrap();
    assert_eq!(
        rec.parents,
        vec!["ab12.0002".to_string()],
        "the deleted parent is dropped, the live one remains"
    );
    assert_eq!(
        rec.parents_notice.as_deref(),
        Some(
            "This item has the following parents: ab12.0002. URGENT RECOMMENDATION: ALSO READ \
             THESE ITEMS TO GET THE COMPLETE PICTURE!!!"
        )
    );
}

#[test]
fn show_omits_the_notice_when_every_parent_is_deleted() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "Deleted epic", "0");
    open_task(&mut s, "ab12.0002", "Child", "0");
    s.add_parent("ab12.0002", "ab12.0001", "t");
    s.delete_item("ab12.0001", "t");

    let rec = read::show(&s, "ab12.0002").unwrap();
    assert!(rec.parents.is_empty());
    assert!(rec.parents_notice.is_none());
    assert!(rec.to_value().get("parents").is_none());
}

#[test]
fn notes_lists_the_worklog_oldest_first_and_json_is_id_body_pairs() {
    // `note list` compute: an item's non-tombstoned notes in canonical oldest-first order, and the
    // `--json` array of `{ id, body }` in that same order (the shape the CLI + MCP note_list share).
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    let n1 = s.add_note("ab12.0001", "first note", "t");
    let n2 = s.add_note("ab12.0001", "second note", "t");

    let notes = read::notes(&s, "ab12.0001").unwrap();
    assert_eq!(
        notes,
        vec![
            (n1.clone(), "first note".to_string()),
            (n2.clone(), "second note".to_string()),
        ],
        "notes are the item's worklog in canonical oldest-first order"
    );

    let v = read::notes_to_value(&notes);
    assert_eq!(v[0]["id"], n1);
    assert_eq!(v[0]["body"], "first note");
    assert_eq!(v[1]["id"], n2);
    assert_eq!(v[1]["body"], "second note");
}

#[test]
fn notes_on_a_missing_item_is_not_found() {
    // A note list on a non-existent item is `not_found`, matching `show` (explicit lookup, not
    // enumeration) — the existence guard belongs to the shared compute, not each caller.
    let s = Store::open_in_memory(1);
    let err = read::notes(&s, "ab12.9999").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn mentions_lists_the_cited_ids_sorted_and_json_is_a_plain_array() {
    // `mention list` compute: the short-ids the item cites in its free text, sorted/deterministic,
    // and the `--json` plain array of those ids (the shape the CLI + MCP mention_list share).
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    open_task(&mut s, "ab12.0002", "B", "0");
    open_task(&mut s, "ab12.0003", "C", "0");
    s.add_edge("ab12.0001", "ab12.0003", EdgeKind::Mentions, "t");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Mentions, "t");

    let mentions = read::mentions(&s, "ab12.0001").unwrap();
    assert_eq!(
        mentions,
        vec!["ab12.0002".to_string(), "ab12.0003".to_string()],
        "cited ids come back sorted, independent of add order"
    );

    let v = read::mentions_to_value(&mentions);
    assert_eq!(v, serde_json::json!(["ab12.0002", "ab12.0003"]));
}

#[test]
fn mentions_on_a_missing_item_is_not_found() {
    let s = Store::open_in_memory(1);
    let err = read::mentions(&s, "ab12.9999").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn list_filters_by_status_and_type_and_json_preserves_order() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    s.create_item("ab12.0002", "project", "P", "t");

    assert_eq!(
        read::list(&cfg(), &s, None, None, None).unwrap().len(),
        2,
        "both live items"
    );
    let tasks: Vec<_> = read::list(&cfg(), &s, None, Some("task"), None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(tasks, ["ab12.0001"], "type filter");
    assert_eq!(
        read::list(&cfg(), &s, Some("open"), None, None)
            .unwrap()
            .len(),
        2,
        "both default to open"
    );

    // The ordering-aware projection preserves the read order (list defaults to id).
    let v = read::items_in_order_value(&read::list(&cfg(), &s, None, None, None).unwrap());
    let ids: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids,
        ["ab12.0001", "ab12.0002"],
        "json array preserves id order"
    );
}

#[test]
fn prime_truncates_next_but_reports_the_true_total() {
    // PRIME_NEXT_LIMIT is 15 (raised from 7 with the finish-first tiers, so a started epic's
    // cluster is not cut mid-way); the total is never hidden by the truncation.
    let mut s = Store::open_in_memory(1);
    for i in 1..=17 {
        open_task(&mut s, &format!("ab12.{i:04}"), &format!("t{i}"), "0");
    }
    let report = read::prime(&cfg(), &s, NOW, false).unwrap();
    assert_eq!(report.next_total, 17, "all seventeen are ready candidates");
    assert_eq!(report.next.len(), 15, "truncated to the prime limit");

    let v = report.to_value();
    assert_eq!(v["next_total"], 17);
    assert_eq!(v["next"].as_array().unwrap().len(), 15);
    assert!(!v["rules"].as_array().unwrap().is_empty());
    assert!(!v["commands"].as_array().unwrap().is_empty());
    assert!(v["purpose"].as_str().unwrap().starts_with("nexus-flow is"));

    // vux: `next` is the lean projection — displayed fields only, no long-text bloat.
    let first = &v["next"][0];
    assert!(first["id"].is_string() && first["title"].is_string());
    assert!(
        first.get("description").is_none() && first.get("design").is_none(),
        "prime next item carries no long-text fields: {first}"
    );
}

#[test]
fn prime_finding_work_index_lists_the_deferred_lane() {
    // 6j6v.jpcj: the deferred lane lived only in the Core Rules prose, so an agent that scans the
    // command index (not the prose) never saw it. It must appear in the "Finding work" group.
    let s = Store::open_in_memory(1);
    let v = read::prime(&cfg(), &s, NOW, false).unwrap().to_value();
    let finding = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["group"] == serde_json::json!("Finding work"))
        .expect("prime has a Finding work group");
    let deferred = finding["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == serde_json::json!("deferred"))
        .expect("Finding work lists the deferred lane");
    // The summary also surfaces the event-vs-date practice + the guide pointer (owner note).
    let summary = deferred["summary"].as_str().unwrap();
    assert!(
        summary.contains("WAIT:") && summary.contains("nxf guide deferring-and-waiting"),
        "the deferred entry surfaces the WAIT-gate practice + guide: {summary}"
    );
}

#[test]
fn prime_sync_hint_is_present_only_when_bound() {
    // vux: the facade gates the sync hint + the session-close sync step on `bound`.
    let s = Store::open_in_memory(1);

    let unbound = read::prime(&cfg(), &s, NOW, false).unwrap();
    assert!(unbound.sync.is_none(), "no sync hint when unbound");
    assert!(
        unbound.to_value().get("sync").is_none(),
        "no sync key when unbound"
    );
    assert!(
        !unbound
            .session_close
            .iter()
            .any(|s| s.contains("nxs sync run")),
        "no sync step in session_close when unbound"
    );

    let bound = read::prime(&cfg(), &s, NOW, true).unwrap();
    assert!(bound.sync.is_some(), "sync hint when bound");
    assert!(
        bound.to_value().get("sync").is_some(),
        "sync key when bound"
    );
    assert!(
        bound
            .session_close
            .iter()
            .any(|s| s.contains("nxs sync run")),
        "sync step in session_close when bound"
    );
}

// ---- lane verbs + partition guarantee (C3 #916.4) --------------------------

/// Set an LWW cell directly (the C5 verbs don't exist yet in these unit tests).
fn set(s: &mut Store, id: &str, field: &str, value: &str) {
    s.set_field(id, field, Some(value.to_string()), "t");
}

#[test]
fn deferred_lane_orders_by_defer_date_ascending() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "later", "0");
    open_task(&mut s, "ab12.0002", "sooner", "0");
    set(&mut s, "ab12.0001", "defer_until", "2026-12-01T00:00:00Z");
    set(&mut s, "ab12.0002", "defer_until", "2026-09-01T00:00:00Z");

    // Default order is defer-date ascending (the soonest-returning item first), NOT id.
    let ids: Vec<_> = read::deferred(&cfg(), &s, NOW, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0001"],
        "deferred: soonest defer first"
    );

    // `--sort id` overrides the default.
    let by_id: Vec<_> = read::deferred(&cfg(), &s, NOW, Some(read::SortKey::Id))
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(by_id, ["ab12.0001", "ab12.0002"]);
}

#[test]
fn closed_lane_excludes_archived_and_orders_by_close_recency() {
    let mut s = Store::open_in_memory(1);
    for (id, t) in [("ab12.0001", "A"), ("ab12.0002", "B"), ("ab12.0003", "C")] {
        open_task(&mut s, id, t, "0");
    }
    // Two closed at different instants; one closed AND archived (must drop out — archived lane).
    set(&mut s, "ab12.0001", "status", "closed");
    set(&mut s, "ab12.0001", "closed_at", "2026-06-01T00:00:00Z"); // older close
    set(&mut s, "ab12.0002", "status", "closed");
    set(&mut s, "ab12.0002", "closed_at", "2026-06-10T00:00:00Z"); // newer close
    set(&mut s, "ab12.0003", "status", "closed");
    set(&mut s, "ab12.0003", "closed_at", "2026-06-05T00:00:00Z");
    set(&mut s, "ab12.0003", "archived", "2026-06-20T00:00:00Z"); // closed AND archived

    let ids: Vec<_> = read::closed(&cfg(), &s, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    // Recency: newest close first; the archived one is absent (it belongs to the archived lane).
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0001"],
        "closed: newest close first, archived excluded"
    );
}

#[test]
fn archived_lane_orders_by_archive_recency_regardless_of_status() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    open_task(&mut s, "ab12.0002", "B", "0");
    set(&mut s, "ab12.0001", "archived", "2026-06-01T00:00:00Z"); // older archive
    set(&mut s, "ab12.0002", "status", "closed");
    set(&mut s, "ab12.0002", "archived", "2026-06-10T00:00:00Z"); // newer archive (and closed)

    let ids: Vec<_> = read::archived(&cfg(), &s, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0001"],
        "archived: newest archive first, any status"
    );
}

#[test]
fn closed_lane_tolerates_a_missing_close_instant() {
    // A pre-C3 closed item carries no `closed_at`; the order must stay TOTAL (no panic) — nulls
    // sort to a deterministic position after dated ones, with id as the final tiebreak.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "legacy", "0");
    open_task(&mut s, "ab12.0002", "dated", "0");
    set(&mut s, "ab12.0001", "status", "closed"); // no closed_at (legacy)
    set(&mut s, "ab12.0002", "status", "closed");
    set(&mut s, "ab12.0002", "closed_at", "2026-06-10T00:00:00Z");

    let ids: Vec<_> = read::closed(&cfg(), &s, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0001"],
        "dated close before the null-instant one"
    );
}

// ---- recap (r4kb): the recency recall surface, distinct from the `closed` lane ----------------

#[test]
fn recap_includes_archived_closed_but_excludes_non_closed_and_deleted() {
    // recap is recency recall: every CLOSED item counts, archived or not — the ONE difference from
    // the `closed` lane, which drops archived (archived precedence). Non-closed (incl. reopened) and
    // deleted never appear.
    let mut s = Store::open_in_memory(1);
    for id in [
        "ab12.0001",
        "ab12.0002",
        "ab12.0003",
        "ab12.0004",
        "ab12.0005",
    ] {
        open_task(&mut s, id, id, "0");
    }
    set(&mut s, "ab12.0001", "status", "closed"); // closed, not archived → in
    set(&mut s, "ab12.0001", "closed_at", "2026-06-01T00:00:00Z");
    set(&mut s, "ab12.0002", "status", "closed"); // closed AND archived → IN (unlike `closed`)
    set(&mut s, "ab12.0002", "closed_at", "2026-06-02T00:00:00Z");
    set(&mut s, "ab12.0002", "archived", "2026-06-20T00:00:00Z");
    set(&mut s, "ab12.0003", "archived", "2026-06-20T00:00:00Z"); // archived, still open → out
                                                                  // ab12.0004 stays open (a reopened item is just status != closed) → out
    set(&mut s, "ab12.0005", "status", "closed"); // closed but deleted → out
    set(&mut s, "ab12.0005", "closed_at", "2026-06-03T00:00:00Z");
    s.delete_item("ab12.0005", "t");

    let ids: Vec<_> = read::recap(&cfg(), &s, None, None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0001"],
        "recap: both closed items (archived included), newest close first; non-closed + deleted out"
    );
}

#[test]
fn recap_limit_caps_to_the_most_recent() {
    let mut s = Store::open_in_memory(1);
    for (id, when) in [
        ("ab12.0001", "2026-06-01T00:00:00Z"),
        ("ab12.0002", "2026-06-02T00:00:00Z"),
        ("ab12.0003", "2026-06-03T00:00:00Z"),
    ] {
        open_task(&mut s, id, id, "0");
        set(&mut s, id, "status", "closed");
        set(&mut s, id, "closed_at", when);
    }
    let ids: Vec<_> = read::recap(&cfg(), &s, Some(2), None)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0003", "ab12.0002"],
        "limit keeps the N most-recent closes, in recency order"
    );
}

#[test]
fn recap_since_filters_closed_at_inclusive_of_the_boundary() {
    // The boundary is INCLUSIVE (`closed_at >= since`): a close exactly on the floor is kept, one
    // strictly before it drops.
    let mut s = Store::open_in_memory(1);
    for (id, when) in [
        ("ab12.0001", "2026-06-01T00:00:00Z"), // before the floor → out
        ("ab12.0002", "2026-06-10T00:00:00Z"), // exactly the floor → IN (inclusive)
        ("ab12.0003", "2026-06-15T00:00:00Z"), // after the floor → in
    ] {
        open_task(&mut s, id, id, "0");
        set(&mut s, id, "status", "closed");
        set(&mut s, id, "closed_at", when);
    }
    let ids: Vec<_> = read::recap(&cfg(), &s, None, Some("2026-06-10T00:00:00Z"))
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0003", "ab12.0002"],
        "since keeps closed_at >= boundary (boundary inclusive), newest first"
    );
}

#[test]
fn recap_since_compares_a_plain_date_floor_against_full_timestamps() {
    // The CLI hands recap a plain `YYYY-MM-DD` floor while `closed_at` is a full RFC3339 instant;
    // the ISO-8601 string order makes `2026-06-10T09:00:00Z >= 2026-06-10` hold, so a close ON the
    // floor day is kept and the day before drops.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "a", "0");
    set(&mut s, "ab12.0001", "status", "closed");
    set(&mut s, "ab12.0001", "closed_at", "2026-06-09T23:00:00Z"); // day before → out
    open_task(&mut s, "ab12.0002", "b", "0");
    set(&mut s, "ab12.0002", "status", "closed");
    set(&mut s, "ab12.0002", "closed_at", "2026-06-10T09:00:00Z"); // floor day → in
    let ids: Vec<_> = read::recap(&cfg(), &s, None, Some("2026-06-10"))
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        ids,
        ["ab12.0002"],
        "a plain-date floor keeps closes on that day (string order across formats)"
    );
}

#[test]
fn recap_rejects_a_malformed_since() {
    // The floor is validated INSIDE the facade fn (not only at the CLI seam), so an embedder calling
    // recap directly gets a loud error, never a silently-empty result.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "a", "0");
    set(&mut s, "ab12.0001", "status", "closed");
    set(&mut s, "ab12.0001", "closed_at", "2026-06-10T00:00:00Z");

    let err = read::recap(&cfg(), &s, None, Some("not-a-date")).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Validation,
        "a malformed since is a validation error, not an empty result"
    );
    // A well-formed floor (plain date or RFC3339) still succeeds.
    assert!(read::recap(&cfg(), &s, None, Some("2026-06-01")).is_ok());
    assert!(read::recap(&cfg(), &s, None, Some("2026-06-01T00:00:00Z")).is_ok());
}

#[test]
fn truncate_notes_keeps_exactly_the_cap_and_ellipsizes_past_it() {
    // The boundary the prime section + `nxf recap` share: a note exactly at the cap is kept
    // verbatim; one character over is capped to the first `max` chars plus a trailing `…`.
    let at = "x".repeat(280);
    let (out, trunc) = read::truncate_notes(&at, 280);
    assert_eq!(out, at, "exactly the cap is kept verbatim");
    assert!(!trunc, "…and is NOT marked truncated");

    let over = "x".repeat(281);
    let (out, trunc) = read::truncate_notes(&over, 280);
    assert!(out.ends_with('…'), "overflow gets an ellipsis");
    assert_eq!(out.chars().count(), 281, "280 kept chars + the ellipsis");
    assert!(trunc, "…and is marked truncated");
    assert_eq!(
        &out[..out.len() - '…'.len_utf8()],
        &"x".repeat(280),
        "the kept prefix is the first 280 chars"
    );
}

#[test]
fn truncate_notes_collapses_internal_whitespace_and_trims() {
    let (out, trunc) = read::truncate_notes("  a\n\nb\t c  ", 280);
    assert_eq!(
        out, "a b c",
        "newlines + whitespace runs collapse to single spaces, ends trimmed"
    );
    assert!(!trunc);
}

#[test]
fn truncate_notes_on_empty_or_blank_is_empty_and_untruncated() {
    assert_eq!(read::truncate_notes("", 280), (String::new(), false));
    assert_eq!(
        read::truncate_notes("   \n\t ", 280),
        (String::new(), false),
        "whitespace-only collapses to empty"
    );
}

#[test]
fn truncate_notes_caps_on_char_boundaries_not_bytes() {
    // The cap counts CHARS, and the kept prefix is whole chars — a multi-byte source must never
    // panic or split mid-char. Three 2-byte `é` capped to 2 → 2 chars kept + the ellipsis.
    let (out, trunc) = read::truncate_notes("ééé", 2);
    assert_eq!(out, "éé…");
    assert!(trunc);
    assert_eq!(out.chars().count(), 3);
    // A 4-byte emoji exactly at the cap is kept whole (no truncation).
    let (out, trunc) = read::truncate_notes("🚀🚀", 2);
    assert_eq!(out, "🚀🚀");
    assert!(!trunc);
}

#[test]
fn recap_note_uses_an_em_dash_for_a_missing_or_blank_comment() {
    // The placeholder the recap surfaces show when a close carried no comment (defensive — the
    // engine requires one, but a legacy/imported item may lack it).
    assert_eq!(read::recap_note(None, 280), ("—".to_string(), false));
    assert_eq!(read::recap_note(Some("   "), 280), ("—".to_string(), false));
    assert_eq!(
        read::recap_note(Some("done"), 280),
        ("done".to_string(), false)
    );
}

#[test]
fn lane_verbs_partition_the_live_item_space() {
    // The central C3 guarantee: archived ⊎ closed ⊎ blocked ⊎ deferred ⊎ ready ⊎ in_progress =
    // all live (deleted excluded). Every live item lands in EXACTLY one lane. The hard cases:
    // a claimed-but-blocked item stays in the in_progress lane (raw status), and a closed-AND-
    // archived item is in the archived lane only (archived precedence).
    use nexus_flow_core::derive;
    use std::collections::BTreeSet;

    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    let mk = |s: &mut Store, id: &str| open_task(s, id, id, "0");

    mk(&mut s, "ab12.0001"); // ready
    mk(&mut s, "ab12.0002"); // ready
    mk(&mut s, "ab12.0003"); // blocked by 0001
    s.add_edge("ab12.0003", "ab12.0001", EdgeKind::Dep, "t");
    mk(&mut s, "ab12.0004"); // cyclic ⇒ blocked
    mk(&mut s, "ab12.0005"); // cyclic ⇒ blocked
    s.add_edge("ab12.0004", "ab12.0005", EdgeKind::Dep, "t");
    s.add_edge("ab12.0005", "ab12.0004", EdgeKind::Dep, "t");
    mk(&mut s, "ab12.0006"); // deferred (future defer, no blocker)
    set(&mut s, "ab12.0006", "defer_until", "2026-12-01T00:00:00Z");
    mk(&mut s, "ab12.0007"); // in_progress, actionable
    set(&mut s, "ab12.0007", "status", "in_progress");
    mk(&mut s, "ab12.0008"); // in_progress BUT blocked by 0002 → still in_progress lane (raw status)
    set(&mut s, "ab12.0008", "status", "in_progress");
    s.add_edge("ab12.0008", "ab12.0002", EdgeKind::Dep, "t");
    mk(&mut s, "ab12.0009"); // in_progress AND deferred → still in_progress lane
    set(&mut s, "ab12.0009", "status", "in_progress");
    set(&mut s, "ab12.0009", "defer_until", "2026-12-01T00:00:00Z");
    mk(&mut s, "ab12.0010"); // closed, not archived → closed lane
    set(&mut s, "ab12.0010", "status", "closed");
    mk(&mut s, "ab12.0011"); // archived (open) → archived lane
    set(&mut s, "ab12.0011", "archived", "2026-06-15T00:00:00Z");
    mk(&mut s, "ab12.0012"); // closed AND archived → archived lane (archived precedence)
    set(&mut s, "ab12.0012", "status", "closed");
    set(&mut s, "ab12.0012", "archived", "2026-06-15T00:00:00Z");
    mk(&mut s, "ab12.0013"); // deleted → excluded from every lane
    s.delete_item("ab12.0013", "t");

    let lane = |ids: Vec<String>| ids.into_iter().collect::<BTreeSet<_>>();
    let ready = lane(derive::ready(s.connection(), NOW).unwrap());
    let blocked = lane(derive::blocked(s.connection()).unwrap());
    let deferred = lane(derive::deferred(s.connection(), NOW).unwrap());
    let in_progress = lane(
        read::list(&cfg, &s, Some("in_progress"), None, None)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect(),
    );
    let closed = lane(
        read::closed(&cfg, &s, None)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect(),
    );
    let archived = lane(
        read::archived(&cfg, &s, None)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect(),
    );

    let lanes = [
        &ready,
        &blocked,
        &deferred,
        &in_progress,
        &closed,
        &archived,
    ];
    // Pairwise disjoint.
    for (i, a) in lanes.iter().enumerate() {
        for b in &lanes[i + 1..] {
            let overlap: Vec<_> = a.intersection(b).collect();
            assert!(
                overlap.is_empty(),
                "lanes must be disjoint; overlap: {overlap:?}"
            );
        }
    }
    // Union == all live (non-deleted) items.
    let union: BTreeSet<String> = lanes.iter().flat_map(|l| l.iter().cloned()).collect();
    let live: BTreeSet<String> = s
        .list_items()
        .unwrap()
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .map(|i| i.id)
        .collect();
    assert_eq!(union, live, "every live item lands in exactly one lane");
    assert!(
        !union.contains("ab12.0013"),
        "the deleted item is in no lane"
    );
    // Spot-check the hard cases landed where intended.
    assert!(
        in_progress.contains("ab12.0008"),
        "claimed-but-blocked stays in_progress"
    );
    assert!(
        in_progress.contains("ab12.0009"),
        "claimed-but-deferred stays in_progress"
    );
    assert!(
        archived.contains("ab12.0012"),
        "closed-and-archived is archived, not closed"
    );
    assert!(!closed.contains("ab12.0012"));
}

// ---- lane-ranked search (C6 #916.6) ----------------------------------------

const NEEDLE: &str = "findme";

/// Create an open task whose description carries the search needle; returns nothing (id == given).
fn match_task(s: &mut Store, id: &str, priority: &str) {
    s.create_item(id, "task", "t", "t");
    s.set_field(id, "priority", Some(priority.to_string()), "t");
    s.set_field(id, "description", Some(format!("blah {NEEDLE} blah")), "t");
}

fn search_ids(cfg: &plugin::PluginConfig, s: &Store, args: read::SearchArgs) -> Vec<String> {
    read::search(cfg, s, NOW, NEEDLE, args)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect()
}

#[test]
fn search_groups_matches_by_lane_priority() {
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    // One matching item per lane (all same priority so the group order, not rank, is what shows).
    match_task(&mut s, "ab12.0001", "0"); // ready → next+ip group
    match_task(&mut s, "ab12.0002", "0"); // will be in_progress → next+ip group, ranked first
    s.set_field("ab12.0002", "status", Some("in_progress".into()), "t");
    match_task(&mut s, "ab12.0003", "0"); // blocked by 0001
    s.add_edge("ab12.0003", "ab12.0001", EdgeKind::Dep, "t");
    match_task(&mut s, "ab12.0004", "0"); // deferred
    s.set_field(
        "ab12.0004",
        "defer_until",
        Some("2026-12-01T00:00:00Z".into()),
        "t",
    );
    match_task(&mut s, "ab12.0005", "0"); // closed
    s.set_field("ab12.0005", "status", Some("closed".into()), "t");
    s.set_field(
        "ab12.0005",
        "closed_at",
        Some("2026-06-10T00:00:00Z".into()),
        "t",
    );

    let ids = search_ids(&cfg, &s, read::SearchArgs::default());
    // Group order: next+ip (in_progress 0002 first, then ready 0001) → blocked 0003 → deferred
    // 0004 → closed 0005.
    assert_eq!(
        ids,
        [
            "ab12.0002",
            "ab12.0001",
            "ab12.0003",
            "ab12.0004",
            "ab12.0005"
        ],
        "matches are grouped by lane priority, natural order within each group"
    );
}

#[test]
fn search_excludes_archived_by_default_and_include_archived_appends_it() {
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    match_task(&mut s, "ab12.0001", "0"); // ready
    match_task(&mut s, "ab12.0002", "0"); // archived
    s.set_field("ab12.0002", "status", Some("closed".into()), "t");
    s.set_field(
        "ab12.0002",
        "archived",
        Some("2026-06-20T00:00:00Z".into()),
        "t",
    );

    // Default: the archived match is excluded.
    assert_eq!(
        search_ids(&cfg, &s, read::SearchArgs::default()),
        ["ab12.0001"]
    );
    // --include-archived: the archived group is appended last (lowest priority).
    assert_eq!(
        search_ids(
            &cfg,
            &s,
            read::SearchArgs {
                include_archived: true,
                ..Default::default()
            }
        ),
        ["ab12.0001", "ab12.0002"]
    );
    // --archived-only: ONLY the archive is searched.
    assert_eq!(
        search_ids(
            &cfg,
            &s,
            read::SearchArgs {
                archived_only: true,
                ..Default::default()
            }
        ),
        ["ab12.0002"]
    );
}

#[test]
fn search_never_drops_a_non_actionable_in_progress_match() {
    // A claimed-but-blocked item is in_progress (not actionable, so `next` hides it) — but it is a
    // live, matching item, so search MUST still surface it, in the next+ip group (its lane).
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    match_task(&mut s, "ab12.0001", "0"); // open blocker (ready)
    match_task(&mut s, "ab12.0002", "0"); // in_progress AND blocked by 0001
    s.set_field("ab12.0002", "status", Some("in_progress".into()), "t");
    s.add_edge("ab12.0002", "ab12.0001", EdgeKind::Dep, "t");

    let ids = search_ids(&cfg, &s, read::SearchArgs::default());
    assert!(
        ids.contains(&"ab12.0002".to_string()),
        "the blocked in_progress match is not dropped"
    );
    // No duplicates — it appears exactly once.
    assert_eq!(ids.iter().filter(|i| *i == "ab12.0002").count(), 1);
}

#[test]
fn search_sort_override_flattens_the_grouping() {
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    match_task(&mut s, "ab12.0003", "0"); // ready
    match_task(&mut s, "ab12.0001", "0"); // closed
    s.set_field("ab12.0001", "status", Some("closed".into()), "t");
    match_task(&mut s, "ab12.0002", "0"); // blocked by 0003
    s.add_edge("ab12.0002", "ab12.0003", EdgeKind::Dep, "t");

    // `--sort id` overrides lane grouping with one flat id order across all matches.
    assert_eq!(
        search_ids(
            &cfg,
            &s,
            read::SearchArgs {
                sort: Some(read::SortKey::Id),
                ..Default::default()
            }
        ),
        ["ab12.0001", "ab12.0002", "ab12.0003"]
    );
}

#[test]
fn search_matches_title_description_design_dod_and_notes() {
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    s.create_item(
        "ab12.0001",
        "task",
        format!("a {NEEDLE} title").as_str(),
        "t",
    );
    s.create_item("ab12.0002", "task", "plain", "t");
    s.set_field(
        "ab12.0002",
        "design",
        Some(format!("the {NEEDLE} plan")),
        "t",
    );
    s.create_item("ab12.0003", "task", "noted", "t");
    s.add_note("ab12.0003", &format!("a worklog {NEEDLE}"), "t");
    s.create_item("ab12.0004", "task", "nope", "t"); // no match anywhere

    let ids = search_ids(&cfg, &s, read::SearchArgs::default());
    assert!(ids.contains(&"ab12.0001".to_string()));
    assert!(ids.contains(&"ab12.0002".to_string()));
    assert!(ids.contains(&"ab12.0003".to_string()));
    assert!(!ids.contains(&"ab12.0004".to_string()));
}

// ---- 07a.3 §8: search grouping over EFFECTIVE lanes -----------------------------------------------

#[test]
fn search_groups_a_suppressed_open_child_under_its_gating_ancestor_lane() {
    // The KNOWN GAP shipped in #146: an open child suppressed by a blocked/deferred ancestor used to
    // VANISH from search grouping (gone from ready/next, absent from the own-status blocked/deferred
    // lanes). It must now surface, grouped under its gating ancestor's EFFECTIVE lane — blocked-gated
    // → blocked, deferred-gated → deferred (owner decision).
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    // PB blocked by its own open dep XB → its open child CB groups under BLOCKED.
    match_task(&mut s, "ab12.0001", "0"); // PB
    match_task(&mut s, "ab12.0002", "0"); // XB (PB's blocker; itself ready)
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t");
    match_task(&mut s, "ab12.0003", "0"); // CB
    s.add_parent("ab12.0003", "ab12.0001", "t");
    // PD deferred → its open child CD groups under DEFERRED.
    match_task(&mut s, "ab12.0004", "0"); // PD
    s.set_field(
        "ab12.0004",
        "defer_until",
        Some("2026-12-01T00:00:00Z".into()),
        "t",
    );
    match_task(&mut s, "ab12.0005", "0"); // CD
    s.add_parent("ab12.0005", "ab12.0004", "t");

    let ids = search_ids(&cfg, &s, read::SearchArgs::default());
    assert_eq!(
        ids,
        [
            "ab12.0002", // next+ip: XB ready
            "ab12.0001", // blocked: PB (own dep)
            "ab12.0003", // blocked: CB suppressed by blocked PB
            "ab12.0004", // deferred: PD (own future defer)
            "ab12.0005", // deferred: CD suppressed by deferred PD (no own defer → sorts last)
        ],
        "suppressed children group under their gating ancestor's lane, none dropped"
    );
}

#[test]
fn search_groups_a_closed_masked_open_child_under_the_closed_lane() {
    // An open child whose every parent is closed is closed-masked: it groups under the CLOSED lane in
    // search (effective), never vanishing, while its own stored status stays `open`.
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    match_task(&mut s, "ab12.0001", "0"); // ready sibling, no parents
    match_task(&mut s, "ab12.0002", "0"); // P, will close
    match_task(&mut s, "ab12.0003", "0"); // C, open child of the closed P
    s.add_parent("ab12.0003", "ab12.0002", "t");
    s.set_field("ab12.0002", "status", Some("closed".into()), "t");
    s.set_field(
        "ab12.0002",
        "closed_at",
        Some("2026-06-10T00:00:00Z".into()),
        "t",
    );

    let items = read::search(&cfg, &s, NOW, NEEDLE, read::SearchArgs::default()).unwrap();
    let ids: Vec<_> = items.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        ids,
        ["ab12.0001", "ab12.0002", "ab12.0003"],
        "ready sibling, then the closed parent and its closed-masked child in the closed group"
    );
    let child = items.iter().find(|i| i.id == "ab12.0003").unwrap();
    assert_eq!(
        child.status.as_deref(),
        Some("open"),
        "the closed-masked child keeps its OWN status; only its lane is masked"
    );
}

#[test]
fn search_moves_a_suppressed_in_progress_child_out_of_next_into_the_blocked_lane() {
    // A claimed (in_progress) child of a blocked ancestor is suppressed: under effective lanes it
    // leaves the next+ip group and joins its gating ancestor's BLOCKED lane — exactly once.
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    match_task(&mut s, "ab12.0001", "0"); // PB
    match_task(&mut s, "ab12.0002", "0"); // XB blocker (ready)
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t");
    match_task(&mut s, "ab12.0003", "0"); // CI, in_progress child of blocked PB
    s.add_parent("ab12.0003", "ab12.0001", "t");
    s.set_field("ab12.0003", "status", Some("in_progress".into()), "t");

    let ids = search_ids(&cfg, &s, read::SearchArgs::default());
    assert_eq!(
        ids,
        ["ab12.0002", "ab12.0003", "ab12.0001"],
        "XB ready (next+ip), then the blocked group ranked: in_progress CI before the open PB"
    );
    assert_eq!(
        ids.iter().filter(|i| *i == "ab12.0003").count(),
        1,
        "the suppressed in_progress child is not duplicated"
    );
}

#[test]
fn search_groups_form_an_effective_lane_partition_of_the_matching_items() {
    // §8 restated over EFFECTIVE lanes: every matching live (non-archived) item lands in EXACTLY one
    // search group — own-lane items AND ancestor-coupled (suppressed / closed-masked) children alike.
    use std::collections::BTreeSet;
    let mut s = Store::open_in_memory(1);
    let cfg = cfg();
    match_task(&mut s, "ab12.0001", "0"); // ready (next+ip)
    match_task(&mut s, "ab12.0002", "0"); // in_progress (next+ip)
    s.set_field("ab12.0002", "status", Some("in_progress".into()), "t");
    match_task(&mut s, "ab12.0003", "0"); // own-blocked by 0001 (blocked)
    s.add_edge("ab12.0003", "ab12.0001", EdgeKind::Dep, "t");
    match_task(&mut s, "ab12.0004", "0"); // own-deferred (deferred)
    s.set_field(
        "ab12.0004",
        "defer_until",
        Some("2026-12-01T00:00:00Z".into()),
        "t",
    );
    match_task(&mut s, "ab12.0005", "0"); // closed
    s.set_field("ab12.0005", "status", Some("closed".into()), "t");
    match_task(&mut s, "ab12.0006", "0"); // CB: open child suppressed by blocked 0003
    s.add_parent("ab12.0006", "ab12.0003", "t");
    match_task(&mut s, "ab12.0007", "0"); // CC: open closed-masked child of closed 0005
    s.add_parent("ab12.0007", "ab12.0005", "t");

    let ids = search_ids(&cfg, &s, read::SearchArgs::default());
    let set: BTreeSet<_> = ids.iter().cloned().collect();
    assert_eq!(
        set.len(),
        ids.len(),
        "no item appears in two groups: {ids:?}"
    );
    let expected: BTreeSet<String> = (1..=7).map(|n| format!("ab12.000{n}")).collect();
    assert_eq!(
        set, expected,
        "every matching live item lands in exactly one effective lane"
    );
}

// ---- 07a.5: close(parent) sweep warning ----------------------------------------------------------

#[test]
fn swept_children_lists_open_children_left_with_no_open_parent() {
    // 07a.5: closing the LAST open parent of a child sweeps it into the effective-closed lane without
    // writing to it. `swept_children` surfaces exactly those — open, every live parent now closed —
    // each with its OTHER parents (id + status). A child that still has an open parent is NOT swept.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "P1", "t");
    s.create_item("ab12.0002", "task", "P2", "t");
    s.create_item("ab12.0003", "task", "C", "t"); // child of P1 + P2
    s.add_parent("ab12.0003", "ab12.0001", "t");
    s.add_parent("ab12.0003", "ab12.0002", "t");
    s.create_item("ab12.0004", "task", "D", "t"); // child of P1 + an always-open parent
    s.add_parent("ab12.0004", "ab12.0001", "t");
    s.create_item("ab12.0005", "task", "OpenP", "t");
    s.add_parent("ab12.0004", "ab12.0005", "t");

    // Close P2 first: C still has open P1 → nothing swept by closing P2.
    s.set_field("ab12.0002", "status", Some("closed".into()), "t");
    assert!(
        read::swept_children(&s, "ab12.0002").unwrap().is_empty(),
        "C still has open parent P1 → not swept"
    );
    // Close P1 (C's last open parent). C is now fully swept; D still has open P5, so D is not.
    s.set_field("ab12.0001", "status", Some("closed".into()), "t");
    let swept = read::swept_children(&s, "ab12.0001").unwrap();
    let ids: Vec<_> = swept.iter().map(|c| c.id.clone()).collect();
    assert_eq!(
        ids,
        ["ab12.0003"],
        "only C is swept (D keeps an open parent)"
    );
    assert_eq!(swept[0].title.as_deref(), Some("C"));
    assert_eq!(
        swept[0].other_parents,
        vec![("ab12.0002".to_string(), Some("closed".to_string()))],
        "C's other parent (besides the just-closed P1) is the closed P2"
    );
}

#[test]
fn close_receipt_value_carries_a_sparse_swept_children_join() {
    // The close receipt is the closed parent's canonical record plus a sparse `swept_children` join
    // — appended like parent_closed_reason, never a core field. A close that sweeps nothing carries
    // no field (so existing close receipts stay byte-stable).
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "P", "t");
    s.create_item("ab12.0002", "task", "C", "t");
    s.add_parent("ab12.0002", "ab12.0001", "t");
    s.set_field("ab12.0001", "status", Some("closed".into()), "t");

    let p = s.get_item("ab12.0001").unwrap().unwrap();
    let v = read::close_receipt_value(&s, &p).unwrap();
    assert_eq!(
        v["id"], "ab12.0001",
        "receipt is the closed parent's record"
    );
    let swept = v["swept_children"]
        .as_array()
        .expect("swept_children present when the close sweeps a child");
    assert_eq!(swept[0]["id"], "ab12.0002");
    assert_eq!(swept[0]["title"], "C");
    assert!(
        swept[0]["other_parents"].as_array().unwrap().is_empty(),
        "C has no other parents"
    );

    // A close that sweeps nothing carries no field (sparse).
    s.create_item("ab12.0003", "task", "lonely", "t");
    s.set_field("ab12.0003", "status", Some("closed".into()), "t");
    let lonely = s.get_item("ab12.0003").unwrap().unwrap();
    assert!(
        read::close_receipt_value(&s, &lonely)
            .unwrap()
            .get("swept_children")
            .is_none(),
        "no sweep → no swept_children field"
    );
}

// ---- #76u.13: a db error at a fallible store read surfaces as `io`, never a panic ----------------

#[test]
fn list_surfaces_a_db_read_failure_as_io_instead_of_panicking() {
    // The flow half of jo9: `read::list` reads `Store::list_items`, which is now fallible. Drop the
    // `items` table out from under it to force a real rusqlite error — the read must map it to the
    // structured `io` kind (so the MCP/embed seam reports a tool error) rather than `.unwrap()`
    // unwinding a long-lived handler. Symmetric with memory's `memories` hardening.
    let s = Store::open_in_memory(1);
    s.connection()
        .execute_batch("DROP TABLE items;")
        .expect("drop the items table to corrupt the read");
    let err =
        read::list(&cfg(), &s, None, None, None).expect_err("a dropped table is a read error");
    assert_eq!(
        err.kind,
        ErrorKind::Io,
        "a db read failure maps to io: {err}"
    );
}

#[test]
fn derivation_reads_surface_a_db_read_failure_as_io_not_a_panic() {
    // #76u.14: the `derive::*` layer is lifted to `Result`, so every read that runs a derivation on
    // the long-lived MCP/embed seam — `next` (the tool hosts call first), `deferred`, `blocked`,
    // `search`, `prime` — maps a db error to the structured `io` kind instead of `.unwrap()`-panicking
    // the handler. Each read's derivation `SELECT`s from `items`; dropping it forces a real rusqlite
    // error inside `derive::*` (`prepare` fails), so this exercises the derivation path, not a panic.
    let assert_io = |label: &str, r: Result<(), nexus_flow_facade::error::NxfError>| {
        let err = r.expect_err(label);
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "{label}: a db read failure maps to io: {err}"
        );
    };
    // A fresh store per read: `DROP TABLE items` is irreversible, so each read gets its own.
    let corrupt = || {
        let s = Store::open_in_memory(1);
        s.connection()
            .execute_batch("DROP TABLE items;")
            .expect("drop items to corrupt the derivation");
        s
    };
    assert_io(
        "next",
        read::next(&cfg(), &corrupt(), NOW, None).map(|_| ()),
    );
    assert_io(
        "deferred",
        read::deferred(&cfg(), &corrupt(), NOW, None).map(|_| ()),
    );
    assert_io(
        "blocked",
        read::blocked(&cfg(), &corrupt(), None).map(|_| ()),
    );
    assert_io(
        "search",
        read::search(&cfg(), &corrupt(), NOW, "x", read::SearchArgs::default()).map(|_| ()),
    );
    assert_io(
        "prime",
        read::prime(&cfg(), &corrupt(), NOW, false).map(|_| ()),
    );
}

#[test]
fn parent_read_seams_surface_a_db_read_failure_as_io_not_a_panic() {
    // 07a.7: the effective-lane read joins (`parent_closed_reason`, `swept_children`) and the
    // `next`/`list`/`close` value projections read the PARENT edge on the long-lived MCP/embed seam
    // via `parents_of_result`/`children_of_result` + the fallible `get_item`. The derivation-drop test
    // above hits `derive::*` FIRST and short-circuits before these seams run, so it cannot catch a
    // regression back to the infallible `parents_of`/`children_of` (`.expect()`-panic) or a `get_item`
    // swallow. This forces the fault INSIDE the seams: build a real parent → open-child edge, fetch the
    // rows, then drop `edge_adds` (the base table of the `present_edges`/`present_parent` VIEWS every
    // parent-edge read and `get_item` hit) — each seam must map that to `io`, never panic.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "parent", "t");
    s.create_item("ab12.0002", "task", "open child", "t");
    s.add_parent("ab12.0002", "ab12.0001", "t"); // child.belongs_to = parent; child stays open
    let child = s.get_item("ab12.0002").unwrap().unwrap();
    s.connection()
        .execute_batch("DROP TABLE edge_adds;")
        .expect("drop the parent-edge source to corrupt the seam reads");
    let assert_io = |label: &str, r: Result<(), nexus_flow_facade::error::NxfError>| {
        let err = r.expect_err(label);
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "{label}: a parent-read db failure maps to io: {err}"
        );
    };
    assert_io("parent_of", read::parent_of(&s, &child).map(|_| ()));
    assert_io(
        "parent_closed_reason",
        read::parent_closed_reason(&s, &child).map(|_| ()),
    );
    assert_io(
        "swept_children",
        read::swept_children(&s, "ab12.0001").map(|_| ()),
    );
    assert_io(
        "next_to_value",
        read::next_to_value(&s, std::slice::from_ref(&child)).map(|_| ()),
    );
    assert_io(
        "list_to_value",
        read::list_to_value(&s, std::slice::from_ref(&child)).map(|_| ()),
    );
}

// ---- 07a.3: parent_closed_reason read-layer join (show keeps own status) -------------------------

#[test]
fn show_of_a_closed_masked_child_keeps_own_status_and_carries_parent_closed_reason() {
    // §4/§5: a still-open child whose every parent is closed is closed-masked. `show` keeps the
    // child's OWN status (open) and surfaces parent_closed_reason — ALL closed parents as
    // [{parent_id, reason}], id-sorted — a read-layer join, never a rewritten status field.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "P1", "t");
    s.create_item("ab12.0002", "task", "P2", "t");
    s.create_item("ab12.0003", "task", "C", "t"); // open child
    s.add_parent("ab12.0003", "ab12.0001", "t");
    s.add_parent("ab12.0003", "ab12.0002", "t");
    for (p, reason) in [("ab12.0001", "done one"), ("ab12.0002", "done two")] {
        s.set_field(p, "status", Some("closed".into()), "t");
        s.set_field(p, "closing_comment", Some(reason.into()), "t");
    }

    let rec = read::show(&s, "ab12.0003").unwrap();
    assert_eq!(
        rec.item.status.as_deref(),
        Some("open"),
        "show keeps the child's OWN status (not masked to closed)"
    );
    let v = rec.to_value();
    let reasons = v["parent_closed_reason"]
        .as_array()
        .expect("closed-masked child carries parent_closed_reason");
    assert_eq!(reasons.len(), 2, "every closed parent is listed");
    assert_eq!(reasons[0]["parent_id"], "ab12.0001");
    assert_eq!(reasons[0]["reason"], "done one");
    assert_eq!(reasons[1]["parent_id"], "ab12.0002");
    assert_eq!(reasons[1]["reason"], "done two");
}

#[test]
fn next_envelope_carries_a_sparse_parent_closed_reason_join() {
    // §5: the closed-mask reason is a read-layer join on the `next` envelope too — uniform with
    // `show`/`list`, never a core field. `next_to_value` projects whatever item slice it is handed,
    // so feed it a closed-masked child directly (the normal `next` pipeline excludes such items via
    // the closed-mask; the envelope contract is what §5 fixes). A non-masked item carries no field.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "P", "t");
    s.create_item("ab12.0002", "task", "C", "t"); // open child of the closed P
    s.add_parent("ab12.0002", "ab12.0001", "t");
    s.set_field("ab12.0001", "status", Some("closed".into()), "t");
    s.set_field("ab12.0001", "closing_comment", Some("done".into()), "t");

    let masked = s.get_item("ab12.0002").unwrap().unwrap();
    let parent = s.get_item("ab12.0001").unwrap().unwrap(); // the closed parent itself is not masked
    let v = read::next_to_value(&s, &[masked, parent]).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["parent_closed_reason"][0]["parent_id"], "ab12.0001");
    assert_eq!(arr[0]["parent_closed_reason"][0]["reason"], "done");
    assert!(
        arr[1].get("parent_closed_reason").is_none(),
        "the closed parent itself is not closed-masked"
    );
}

#[test]
fn list_envelope_carries_a_sparse_parent_closed_reason_join() {
    // §5: `list` (which enumerates open children of closed parents) appends the same sparse join, so
    // an agent enumerating the board sees "closed only via parent, here's why" without a follow-up
    // `show`. A non-masked record is byte-identical to the plain projection (no extra field).
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "P", "t");
    s.create_item("ab12.0002", "task", "C", "t");
    s.add_parent("ab12.0002", "ab12.0001", "t");
    s.set_field("ab12.0001", "status", Some("closed".into()), "t");
    s.set_field("ab12.0001", "closing_comment", Some("done".into()), "t");

    let items = read::list(&cfg(), &s, None, None, None).unwrap();
    let v = read::list_to_value(&s, &items).unwrap();
    let arr = v.as_array().unwrap();
    let child = arr.iter().find(|r| r["id"] == "ab12.0002").unwrap();
    assert_eq!(child["parent_closed_reason"][0]["parent_id"], "ab12.0001");
    assert_eq!(child["parent_closed_reason"][0]["reason"], "done");
    let parent = arr.iter().find(|r| r["id"] == "ab12.0001").unwrap();
    assert!(
        parent.get("parent_closed_reason").is_none(),
        "a non-masked record carries no parent_closed_reason"
    );
}

#[test]
fn show_of_a_child_with_an_open_parent_has_no_parent_closed_reason() {
    // Not closed-masked (one parent still open) → no parent_closed_reason field (sparse join).
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "P1", "t");
    s.create_item("ab12.0002", "task", "P2", "t");
    s.create_item("ab12.0003", "task", "C", "t");
    s.add_parent("ab12.0003", "ab12.0001", "t");
    s.add_parent("ab12.0003", "ab12.0002", "t");
    s.set_field("ab12.0001", "status", Some("closed".into()), "t"); // only one closed

    let v = read::show(&s, "ab12.0003").unwrap().to_value();
    assert!(
        v.get("parent_closed_reason").is_none(),
        "an open parent remains → not closed-masked → no reason join"
    );
}

// ---- labels (h89s.2): read surface ----------------------------------------

#[test]
fn show_value_carries_the_items_labels_sorted() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    s.add_label("ab12.0001", "urgent", "t");
    s.add_label("ab12.0001", "backend", "t");
    let v = read::show_value(&s, "ab12.0001").unwrap();
    assert_eq!(
        v["labels"],
        serde_json::json!(["backend", "urgent"]),
        "show_value carries the sorted label set"
    );
}

#[test]
fn show_value_omits_labels_when_none() {
    // Sparse join: an unlabelled item's value carries no `labels` key, so it stays byte-identical
    // to the plain `ShowRecord::to_value` (additive — the facade struct itself is untouched).
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    let v = read::show_value(&s, "ab12.0001").unwrap();
    assert!(
        v.get("labels").is_none(),
        "no labels key when the item is unlabelled"
    );
    assert_eq!(
        v,
        read::show(&s, "ab12.0001").unwrap().to_value(),
        "unlabelled show_value == the plain ShowRecord value"
    );
}

#[test]
fn labels_read_returns_sorted_and_is_not_found_for_a_missing_item() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    s.add_label("ab12.0001", "z", "t");
    s.add_label("ab12.0001", "a", "t");
    assert_eq!(
        read::labels(&s, "ab12.0001").unwrap(),
        vec!["a".to_string(), "z".to_string()]
    );
    assert_eq!(
        read::labels(&s, "nope").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn with_label_filters_an_item_set() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    open_task(&mut s, "ab12.0002", "B", "0");
    s.add_label("ab12.0001", "urgent", "t");
    let all = read::list(&cfg(), &s, None, None, None).unwrap();
    let filtered = read::with_label(&s, all, "urgent").unwrap();
    let ids: Vec<_> = filtered.into_iter().map(|i| i.id).collect();
    assert_eq!(
        ids,
        ["ab12.0001"],
        "only the labelled item survives the filter"
    );
}

#[test]
fn list_and_next_json_projections_carry_labels() {
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    s.add_label("ab12.0001", "urgent", "t");
    let items = read::list(&cfg(), &s, None, None, None).unwrap();
    let lv = read::list_to_value(&s, &items).unwrap();
    assert_eq!(lv[0]["labels"], serde_json::json!(["urgent"]));

    let ready = read::next(&cfg(), &s, NOW, None).unwrap();
    let nv = read::next_to_value(&s, &ready).unwrap();
    assert_eq!(nv[0]["labels"], serde_json::json!(["urgent"]));
}

// ---- xn8s: read::next scaling benchmark + N+1 guard ------------------------

/// Seed a realistic ~156-item issue-tracker board into an in-memory store: `epics` container
/// projects + `tasks` parented under them, a quarter blocked by a dep, a seventh in-progress, a
/// third labelled — enough graph to exercise the ready CTE, the ready-set resolution, and ranking.
fn seed_board(epics: usize, tasks: usize) -> Store {
    let mut s = Store::open_in_memory(1);
    for e in 0..epics {
        let id = format!("ab12.e{e:03}");
        s.create_item(&id, "project", &format!("epic {e}"), "t");
        s.set_field(&id, "priority", Some("1".into()), "t");
    }
    for t in 0..tasks {
        let id = format!("ab12.t{t:04}");
        s.create_item(&id, "task", &format!("task {t}"), "t");
        s.set_field(&id, "priority", Some(((t % 5) as i64).to_string()), "t");
        s.add_edge(
            &id,
            &format!("ab12.e{:03}", t % epics),
            EdgeKind::Parent,
            "t",
        );
        if t % 4 == 0 && t > 0 {
            s.add_edge(&id, &format!("ab12.t{:04}", t - 1), EdgeKind::Dep, "t");
        }
        if t % 3 == 0 {
            s.add_label(&id, "area:core", "t");
        }
        if t % 7 == 0 {
            s.set_field(&id, "status", Some("in_progress".into()), "t");
        }
    }
    s
}

fn median_us(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// xn8s benchmark (run with `--ignored --nocapture` to document the numbers): read::next on a
/// ~156-item board. The dominant cost was the ready-set resolution — a per-id `get_item` each
/// re-materialises the `present_parent` view, so the lane was O(n·edges) (~0.7s release / ~1.6s
/// debug here). `resolve_items` now bulk-resolves via `Store::get_items` (one `present_parent`
/// materialisation), which is the load-bearing fix; the ready CTE (`derive::ready`) is the remainder.
/// Prints read::next, the OLD per-item resolve, the NEW bulk resolve, `derive::ready`, and `list`.
#[test]
#[ignore]
fn xn8s_bench_read_next() {
    use std::time::Instant;
    let s = seed_board(16, 140);
    let c = cfg();
    let n_items = read::list(&c, &s, None, None, None).unwrap().len();
    let ready_ids = nexus_flow_core::derive::ready(s.connection(), NOW).unwrap();
    let refs: Vec<&str> = ready_ids.iter().map(String::as_str).collect();
    for _ in 0..5 {
        let _ = read::next(&c, &s, NOW, None).unwrap();
    }
    let runs = 25;
    let (mut t_next, mut t_ready, mut t_old, mut t_bulk, mut t_list) =
        (vec![], vec![], vec![], vec![], vec![]);
    for _ in 0..runs {
        let a = Instant::now();
        let _ = read::next(&c, &s, NOW, None).unwrap();
        t_next.push(a.elapsed().as_micros());
        let a = Instant::now();
        let _ = nexus_flow_core::derive::ready(s.connection(), NOW).unwrap();
        t_ready.push(a.elapsed().as_micros());
        let a = Instant::now();
        for id in &refs {
            let _ = s.get_item(id).unwrap();
        }
        t_old.push(a.elapsed().as_micros());
        let a = Instant::now();
        let _ = s.get_items(&refs).unwrap();
        t_bulk.push(a.elapsed().as_micros());
        let a = Instant::now();
        let _ = read::list(&c, &s, None, None, None).unwrap();
        t_list.push(a.elapsed().as_micros());
    }
    eprintln!(
        "XN8S n={n_items} ready={} | read::next={}us | ready-CTE={}us | resolve OLD(Nx get_item)={}us | resolve NEW(bulk)={}us | list={}us",
        refs.len(),
        median_us(t_next),
        median_us(t_ready),
        median_us(t_old),
        median_us(t_bulk),
        median_us(t_list),
    );
}

/// xn8s regression guard (runs in CI): read::next on a large board must NOT resolve its ready set
/// with a per-item `get_item` (the O(n·edges) N+1 that dominated the lane). Asserts the QUERY SHAPE
/// — how many item reads the store ran — not the clock.
///
/// It used to guard the RATIO of read::next's runtime to list's, which was self-calibrating for
/// machine speed but not for machine *load*: a ratio of two medians only stays meaningful while
/// both samples see the same contention, and under a parallel build they do not. In PR #263 it went
/// red in five consecutive runs on a box at load average 317 while the measured read path was
/// byte-identical to main, blocking commits that never touched this crate (6j6v.g028). The property
/// it was reaching for is algorithmic, so the guard now states it directly: the ready set resolves
/// in ONE bulk query ([`Store::get_items`]) and zero per-item ones. Reverting `resolve_items` to the
/// old `filter_map(get_item)` loop turns `single: 0` into one read per ready item — a deterministic
/// failure on any machine, idle or hammered, in debug or release.
#[test]
fn xn8s_read_next_resolves_the_ready_set_in_bulk_not_n_plus_1() {
    let s = seed_board(16, 140);
    let c = cfg();
    let before = s.item_read_stats();
    let candidates = read::next(&c, &s, NOW, None).unwrap();
    let ran = s.item_read_stats().since(before);
    // A guard over a one-item ready set would pass with either resolution, so pin the fixture's
    // size: the N+1 must have something to fan out over for `single: 0` to mean anything.
    assert!(
        candidates.len() > 20,
        "fixture regressed: {} candidates is too small a ready set for the N+1 to show",
        candidates.len()
    );
    assert_eq!(
        ran,
        ItemReadStats { single: 0, bulk: 1 },
        "read::next resolved its {} candidates with {ran:?} — expected ONE bulk get_items and no \
         per-item get_item; a `single` count near the candidate count is the O(n·edges) N+1 back \
         (each get_item re-materializes the present_parent view)",
        candidates.len()
    );
}

// ---- plugin custom fields (6j6v.ekf5, T4): schema + the sparse `custom` record join ----------
//
// Bundled plugins declare NO `[fields]`, so custom output only appears here, under a
// fields-declaring fixture — the canonical goldens/oracle stay byte-identical.

/// A plugin config that DECLARES custom fields: a `file` type with a required `uri` text field and a
/// `project` with a `stage` enum. Built via `toml::from_str`, the same shape the T2 tests use.
fn cfg_with_fields() -> plugin::PluginConfig {
    toml::from_str(
        r#"
        name = "fields-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P1", "P2", "P3"]
        [types]
        list = ["file", "project"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        order = [ { field = "id", dir = "asc" } ]
        [presentation.list]
        columns = ["id"]

        [fields.uri]
        type = "text"
        on = ["file"]
        required = true
        label = "File URI"

        [fields.stage]
        type = "enum"
        values = ["backlog", "active", "done"]
        on = ["project"]
        "#,
    )
    .expect("cfg_with_fields parses")
}

/// Set a declared custom field's value directly through the core write helper (the T3 path the
/// facade drives), so the read tests can seed a folded `custom_fields` row.
fn set_custom(s: &mut Store, id: &str, field: &str, value: &str) {
    s.set_custom_field_merge(
        id,
        field,
        Some(value.to_string()),
        "t",
        nexus_flow_core::model::MergeStrategy::Lww,
    );
}

fn field_entry<'a>(schema: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    schema["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == name)
        .unwrap_or_else(|| panic!("no schema field entry for {name}"))
}

#[test]
fn schema_lists_declared_custom_fields_and_leaves_canonical_entries_untouched() {
    // §6: each declared custom field is appended to `fields[]`, marked `custom:true` and carrying its
    // declaration (`kind`/`on`/`required`/`label`, plus `values` for an enum). Canonical entries gain
    // NO new key, so their `schema --json` objects stay byte-identical (the `schema_*` goldens hold).
    let schema = read::schema(&cfg_with_fields()).to_value();

    let uri = field_entry(&schema, "uri");
    assert_eq!(uri["custom"], serde_json::json!(true));
    assert_eq!(uri["kind"], "text");
    assert_eq!(uri["on"], serde_json::json!(["file"]));
    assert_eq!(uri["required"], serde_json::json!(true));
    assert_eq!(uri["settable"], serde_json::json!(true));
    assert_eq!(uri["label"], "File URI");
    assert!(uri.get("values").is_none(), "a text field has no `values`");
    assert!(
        uri.get("create_flag").is_none() && uri.get("update_alias").is_none(),
        "a custom field carries no create flag / update alias"
    );

    let stage = field_entry(&schema, "stage");
    assert_eq!(stage["custom"], serde_json::json!(true));
    assert_eq!(stage["kind"], "enum");
    assert_eq!(stage["on"], serde_json::json!(["project"]));
    assert_eq!(
        stage["values"],
        serde_json::json!(["backlog", "active", "done"]),
        "an enum custom field spells out its members"
    );
    assert_eq!(
        stage["label"], "Stage",
        "an unlabelled field titleises its name"
    );

    // A CANONICAL entry gains no annotation keys — byte-identical to the pre-epic shape.
    let title = field_entry(&schema, "title");
    assert!(
        title.get("custom").is_none() && title.get("on").is_none() && title.get("values").is_none(),
        "canonical `title` entry is unchanged: {title}"
    );
}

#[test]
fn schema_omits_a_foreign_custom_value_it_does_not_declare() {
    // §7: a field the active plugin does not DECLARE never appears in `schema` (a schema is pure over
    // `cfg` — it does not read the store at all, so a foreign folded value cannot leak in).
    let schema = read::schema(&cfg_with_fields()).to_value();
    assert!(
        schema["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["field"] != "secret"),
        "an undeclared field is absent from schema"
    );
}

#[test]
fn record_carries_the_sparse_declared_custom_map_and_omits_foreign_and_empty() {
    // §2.3/§6/§7: `show`/`list`/`next --json` attach a sparse `custom` map with the item's DECLARED,
    // non-empty custom values only — a foreign (undeclared) value is carried in the view but NOT
    // shown, and a cleared (empty) value reads as unset.
    let cfg = cfg_with_fields();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "file", "A", "t");
    set_custom(&mut s, "ab12.0001", "uri", ".nxs/files/a.md");
    set_custom(&mut s, "ab12.0001", "secret", "leak"); // foreign — undeclared by cfg
    set_custom(&mut s, "ab12.0001", "stage", "backlog"); // declared but will be CLEARED below
    set_custom(&mut s, "ab12.0001", "stage", ""); // clear → unset

    let expected = serde_json::json!({ "uri": ".nxs/files/a.md" });

    // show
    let sv = read::show_value_with_custom(&cfg, &s, "ab12.0001").unwrap();
    assert_eq!(sv["custom"], expected, "show: declared+non-empty only");

    // list
    let items = read::list(&cfg, &s, None, None, None).unwrap();
    let lv = read::list_to_value_with_custom(&cfg, &s, &items).unwrap();
    assert_eq!(lv[0]["custom"], expected, "list: declared+non-empty only");

    // next
    let ready = read::next(&cfg, &s, NOW, None).unwrap();
    let nv = read::next_to_value_with_custom(&cfg, &s, &ready).unwrap();
    assert_eq!(nv[0]["custom"], expected, "next: declared+non-empty only");
}

#[test]
fn blocked_and_lane_builders_carry_the_sparse_declared_custom_map() {
    // 6j6v.bbq6: the `blocked`/`deferred`/`closed`/`archived` reads gain the SAME sparse `custom`
    // join `show`/`list`/`next` carry — declared, non-empty values only, sparse (no key when unset).
    let cfg = cfg_with_fields();
    let mut s = Store::open_in_memory(1);
    // A blocked `file` item with a declared custom `uri`, held up by an open blocker.
    s.create_item("ab12.0001", "file", "A", "t");
    set_custom(&mut s, "ab12.0001", "uri", ".nxs/files/a.md");
    set_custom(&mut s, "ab12.0001", "secret", "leak"); // foreign — undeclared, never shown
    s.create_item("ab12.0002", "file", "B", "t"); // no declared custom value
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t"); // ⇒ 0001 blocked by 0002

    let expected = serde_json::json!({ "uri": ".nxs/files/a.md" });

    // blocked_to_value_with_custom: the sparse custom rides the record alongside its `blockers` list.
    let rows = read::blocked(&cfg, &s, None).unwrap();
    let bv = read::blocked_to_value_with_custom(&cfg, &s, &rows).unwrap();
    assert_eq!(bv[0]["id"], "ab12.0001");
    assert_eq!(
        bv[0]["custom"], expected,
        "blocked: declared+non-empty only"
    );
    assert!(
        bv[0]["blockers"].as_array().is_some_and(|b| !b.is_empty()),
        "blocked still carries its blockers list"
    );

    // items_in_order_value_with_custom: the bare-lane builder (deferred/closed/archived share it).
    let items = read::list(&cfg, &s, None, None, None).unwrap();
    let iv = read::items_in_order_value_with_custom(&cfg, &s, &items).unwrap();
    let arr = iv.as_array().unwrap();
    let a = arr.iter().find(|r| r["id"] == "ab12.0001").unwrap();
    assert_eq!(a["custom"], expected, "bare-lane: declared+non-empty only");
    let b = arr.iter().find(|r| r["id"] == "ab12.0002").unwrap();
    assert!(
        b.get("custom").is_none(),
        "an item with no declared custom value omits the `custom` key entirely"
    );

    // declared_custom_by_id: the per-id map powering the `prime --json` attach.
    let map = read::declared_custom_by_id(&cfg, &s, &["ab12.0001", "ab12.0002"]).unwrap();
    assert_eq!(map.get("ab12.0001"), Some(&expected));
    assert!(
        !map.contains_key("ab12.0002"),
        "an id with no declared custom value is absent from the map"
    );
}

#[test]
fn blocked_and_lane_with_custom_are_byte_identical_without_declared_fields() {
    // Additive: a plugin declaring NO `[fields]` (issue-tracker/personal-todo) gets output
    // byte-identical to the base builders — the `_with_custom` skip-path (ky26) is a pure no-op.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");
    open_task(&mut s, "ab12.0002", "B", "0");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "t");

    let rows = read::blocked(&cfg(), &s, None).unwrap();
    assert_eq!(
        read::blocked_to_value_with_custom(&cfg(), &s, &rows).unwrap(),
        read::blocked_to_value(&rows),
        "blocked_to_value_with_custom == blocked_to_value with no declared fields"
    );
    let items = read::list(&cfg(), &s, None, None, None).unwrap();
    assert_eq!(
        read::items_in_order_value_with_custom(&cfg(), &s, &items).unwrap(),
        read::items_in_order_value(&items),
        "items_in_order_value_with_custom == items_in_order_value with no declared fields"
    );
    assert!(
        read::declared_custom_by_id(&cfg(), &s, &["ab12.0001"])
            .unwrap()
            .is_empty(),
        "declared_custom_by_id is empty with no declared fields"
    );
}

#[test]
fn record_omits_the_custom_key_entirely_when_nothing_declared_is_set() {
    // Sparse: an item with no declared, non-empty custom value carries NO `custom` key — byte-
    // identical to the label-less/custom-less base projection (additive, like the sparse `labels`).
    let cfg = cfg_with_fields();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "file", "A", "t");
    set_custom(&mut s, "ab12.0001", "secret", "leak"); // ONLY a foreign value

    let sv = read::show_value_with_custom(&cfg, &s, "ab12.0001").unwrap();
    assert!(
        sv.get("custom").is_none(),
        "no declared custom value ⇒ no `custom` key (foreign value not shown): {sv}"
    );
    assert_eq!(
        sv,
        read::show_value(&s, "ab12.0001").unwrap(),
        "byte-identical to the base show_value"
    );

    let items = read::list(&cfg, &s, None, None, None).unwrap();
    let lv = read::list_to_value_with_custom(&cfg, &s, &items).unwrap();
    assert!(
        lv[0].get("custom").is_none(),
        "list omits the empty custom map"
    );
    assert_eq!(
        lv,
        read::list_to_value(&s, &items).unwrap(),
        "list byte-identical to the base projection when nothing declared is set"
    );
}

#[test]
fn record_custom_map_is_per_row_across_a_lane() {
    // The lane join is a single bulk read (`custom_fields_of_bulk`) attached per row — each record
    // gets ITS OWN declared custom map (no cross-contamination), and a row with none omits the key.
    let cfg = cfg_with_fields();
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "file", "A", "t");
    s.create_item("ab12.0002", "project", "B", "t");
    s.create_item("ab12.0003", "file", "C", "t"); // no custom value
    set_custom(&mut s, "ab12.0001", "uri", "u1");
    set_custom(&mut s, "ab12.0002", "stage", "active");

    let items = read::list(&cfg, &s, None, None, None).unwrap();
    let lv = read::list_to_value_with_custom(&cfg, &s, &items).unwrap();
    let by_id = |id: &str| {
        lv.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        by_id("ab12.0001")["custom"],
        serde_json::json!({ "uri": "u1" })
    );
    assert_eq!(
        by_id("ab12.0002")["custom"],
        serde_json::json!({ "stage": "active" })
    );
    assert!(
        by_id("ab12.0003").get("custom").is_none(),
        "a row with no declared custom value omits the key"
    );
}

#[test]
fn no_fields_plugin_skips_the_custom_read_and_stays_byte_identical() {
    // T4-review efficiency (ky26): when the active plugin declares NO `[fields]` (issue-tracker), the
    // `_with_custom` render fns short-circuit — no `custom` key AND no `custom_fields_*` query. The
    // first half asserts behavior-preservation (byte-identical to the base projection); the second
    // half PROVES the query is skipped by dropping the `custom_fields` table and showing the no-fields
    // path still succeeds (it would error if it touched the missing table).
    let cfg = cfg(); // issue-tracker — no declared custom fields
    assert!(cfg.fields.is_empty(), "issue-tracker declares no [fields]");
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "0");

    // Behavior-preserving: the `_with_custom` wrappers equal their base projections and add no key.
    let sv = read::show_value_with_custom(&cfg, &s, "ab12.0001").unwrap();
    assert!(
        sv.get("custom").is_none(),
        "no-fields show has no custom key"
    );
    assert_eq!(sv, read::show_value(&s, "ab12.0001").unwrap());

    let items = read::list(&cfg, &s, None, None, None).unwrap();
    assert_eq!(
        read::list_to_value_with_custom(&cfg, &s, &items).unwrap(),
        read::list_to_value(&s, &items).unwrap(),
        "no-fields list is byte-identical to the base projection"
    );
    let ready = read::next(&cfg, &s, NOW, None).unwrap();
    assert_eq!(
        read::next_to_value_with_custom(&cfg, &s, &ready).unwrap(),
        read::next_to_value(&s, &ready).unwrap(),
        "no-fields next is byte-identical to the base projection"
    );

    // Short-circuit PROOF: with the `custom_fields` table gone, any custom read would error — the
    // no-fields path still returns Ok, so it demonstrably never issued the query.
    s.connection()
        .execute_batch("DROP TABLE custom_fields;")
        .unwrap();
    assert!(
        read::show_value_with_custom(&cfg, &s, "ab12.0001").is_ok(),
        "no-fields show must not query the (now-missing) custom_fields table"
    );
    assert!(
        read::list_to_value_with_custom(&cfg, &s, &items).is_ok(),
        "no-fields list must not query the (now-missing) custom_fields table"
    );
    assert!(
        read::next_to_value_with_custom(&cfg, &s, &ready).is_ok(),
        "no-fields next must not query the (now-missing) custom_fields table"
    );
}

// ---- op-log-derived read-surface timestamps (6j6v.2kjy) -----------------------
//
// created_at/updated_at (item) and created_at (note) are op-log derivations attached as ADDITIVE,
// SPARSE read-layer keys — like `labels`/`custom`, NOT canonical `CanonicalItem` fields (the "kanonische
// Keys unverändert" contract; the record struct + its byte-identity test stay put). Present iff the
// op-log stamped a `wall_clock`, so an un-stamped store is byte-identical to the pre-2kjy output.

/// Create an open task whose ops carry a stamped `wall_clock` (the write-path `stamp_now` a direct
/// store seed skips), so the timestamp derivation has a value to surface.
fn open_task_at(s: &mut Store, id: &str, title: &str, priority: &str, now: &str) {
    s.set_wall_clock(now);
    s.create_item(id, "task", title, "t");
    s.set_field(id, "priority", Some(priority.to_string()), "t");
}

#[test]
fn show_value_carries_item_created_and_updated_timestamps() {
    let mut s = Store::open_in_memory(1);
    open_task_at(&mut s, "ab12.0001", "A", "1", "2026-06-10T00:00:00Z");
    s.set_wall_clock("2026-06-14T12:00:00Z");
    s.set_field("ab12.0001", "status", Some("in_progress".into()), "t");

    let v = read::show_value(&s, "ab12.0001").unwrap();
    assert_eq!(
        v["item"]["created_at"],
        serde_json::json!("2026-06-10T00:00:00Z")
    );
    assert_eq!(
        v["item"]["updated_at"],
        serde_json::json!("2026-06-14T12:00:00Z")
    );
    // The canonical record keys are UNCHANGED — created_at/updated_at ride on top, not inside it.
    assert!(
        v["item"].get("created_at").is_some() && v["item"]["closed_at"].is_null(),
        "additive keys coexist with the untouched canonical fields: {v}"
    );
}

#[test]
fn show_value_notes_carry_their_created_at() {
    let mut s = Store::open_in_memory(1);
    open_task_at(&mut s, "ab12.0001", "A", "1", "2026-06-10T00:00:00Z");
    s.set_wall_clock("2026-06-11T09:00:00Z");
    s.add_note("ab12.0001", "first note", "t");

    let v = read::show_value(&s, "ab12.0001").unwrap();
    let notes = v["notes"].as_array().expect("notes array");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["body"], serde_json::json!("first note"));
    assert_eq!(
        notes[0]["created_at"],
        serde_json::json!("2026-06-11T09:00:00Z"),
        "a note carries the wall_clock its note-add op stamped: {v}"
    );
}

#[test]
fn lane_and_list_records_carry_timestamps() {
    let mut s = Store::open_in_memory(1);
    open_task_at(&mut s, "ab12.0001", "A", "1", "2026-06-10T00:00:00Z");
    open_task_at(&mut s, "ab12.0002", "B", "1", "2026-06-11T00:00:00Z");

    // list (via the CLI/Engine `_with_custom` seam) carries the timestamps on every record.
    let items = read::list(&cfg(), &s, None, None, None).unwrap();
    let lv = read::list_to_value_with_custom(&cfg(), &s, &items).unwrap();
    let by_id = |v: &serde_json::Value, id: &str| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == serde_json::json!(id))
            .unwrap()
            .clone()
    };
    assert_eq!(
        by_id(&lv, "ab12.0001")["created_at"],
        serde_json::json!("2026-06-10T00:00:00Z")
    );
    assert_eq!(
        by_id(&lv, "ab12.0002")["updated_at"],
        serde_json::json!("2026-06-11T00:00:00Z")
    );

    // next (ranked seam) carries them too.
    let ready = read::next(&cfg(), &s, NOW, None).unwrap();
    let nv = read::next_to_value_with_custom(&cfg(), &s, &ready).unwrap();
    assert_eq!(
        by_id(&nv, "ab12.0001")["created_at"],
        serde_json::json!("2026-06-10T00:00:00Z")
    );

    // A bare lane (closed) via items_in_order_value_with_custom carries them.
    s.set_wall_clock("2026-06-12T00:00:00Z");
    s.set_field("ab12.0001", "status", Some("closed".into()), "t");
    s.set_field(
        "ab12.0001",
        "closed_at",
        Some("2026-06-12T00:00:00Z".into()),
        "t",
    );
    let closed = read::closed(&cfg(), &s, None).unwrap();
    let cv = read::items_in_order_value_with_custom(&cfg(), &s, &closed).unwrap();
    assert_eq!(
        cv.as_array().unwrap()[0]["updated_at"],
        serde_json::json!("2026-06-12T00:00:00Z")
    );
}

#[test]
fn notes_with_created_projection_is_sparse_and_matches_show() {
    // `note list --json` carries the SAME sparse `created_at` per note as `show --json`, so the two
    // note projections agree. A note whose op carried no wall_clock keeps the bare {id, body} shape.
    let mut s = Store::open_in_memory(1);
    open_task_at(&mut s, "ab12.0001", "A", "1", "2026-06-10T00:00:00Z");
    s.set_wall_clock("2026-06-11T09:00:00Z");
    s.add_note("ab12.0001", "stamped note", "t");
    s.set_wall_clock(""); // an un-stamped note-add ⇒ no created_at
    s.add_note("ab12.0001", "bare note", "t");

    let notes = read::notes_with_created(&s, "ab12.0001").unwrap();
    let v = read::notes_with_created_to_value(&notes);
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["body"], serde_json::json!("stamped note"));
    assert_eq!(
        arr[0]["created_at"],
        serde_json::json!("2026-06-11T09:00:00Z")
    );
    assert_eq!(arr[1]["body"], serde_json::json!("bare note"));
    assert!(
        arr[1].get("created_at").is_none(),
        "an un-stamped note omits created_at: {v}"
    );
}

#[test]
fn search_stays_timestamp_free_even_for_stamped_items() {
    // `search --json` deliberately rides the LEAN pure `items_in_order_value` builder (no `custom`,
    // no `created_at`/`updated_at` — bbq6/2kjy), unlike the lanes. Pin that directly: even a fully
    // stamped item carries NO timestamp keys through the search projection, so a future refactor that
    // routed search through a `_with_custom`/timestamp path would fail here, not just drift a golden.
    let mut s = Store::open_in_memory(1);
    open_task_at(
        &mut s,
        "ab12.0001",
        "searchable",
        "1",
        "2026-06-10T00:00:00Z",
    );
    let hits = read::search(
        &cfg(),
        &s,
        NOW,
        "searchable",
        read::SearchArgs {
            status: None,
            item_type: None,
            include_archived: false,
            archived_only: false,
            sort: None,
        },
    )
    .unwrap();
    let v = read::items_in_order_value(&hits);
    let rec = &v.as_array().unwrap()[0];
    assert_eq!(
        rec["id"],
        serde_json::json!("ab12.0001"),
        "the item matched: {v}"
    );
    assert!(
        rec.get("created_at").is_none() && rec.get("updated_at").is_none(),
        "search stays timestamp-free (lean), even for a stamped item: {v}"
    );
}

#[test]
fn timestamp_keys_are_omitted_when_no_wall_clock_was_stamped() {
    // A direct un-stamped seed (as most read tests use) leaves the canonical --json byte-identical to
    // the pre-2kjy output — the sparse contract that keeps the differential oracle / goldens honest.
    let mut s = Store::open_in_memory(1);
    open_task(&mut s, "ab12.0001", "A", "1"); // no set_wall_clock
    let v = read::show_value(&s, "ab12.0001").unwrap();
    assert!(
        v["item"].get("created_at").is_none() && v["item"].get("updated_at").is_none(),
        "un-stamped item omits the timestamp keys: {v}"
    );
    let items = read::list(&cfg(), &s, None, None, None).unwrap();
    let lv = read::list_to_value_with_custom(&cfg(), &s, &items).unwrap();
    assert!(
        lv.as_array().unwrap()[0].get("created_at").is_none(),
        "un-stamped lane record omits the timestamp keys: {lv}"
    );
}

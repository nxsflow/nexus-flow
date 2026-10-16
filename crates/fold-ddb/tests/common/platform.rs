//! A generated PLATFORM stream — one op log holding all three products — and the store that folds
//! it (6j6v.0cfa).
//!
//! The based-snapshot differential needs a stream that looks like a real one: three writing sites,
//! every op shape the three reducers fold, and the handful of shapes none of them folds. It also
//! needs it by the tens of thousands, which rules out the file-backed stores a workspace really
//! uses — so the three products' write APIs each run over their own in-memory substrate, their logs
//! are merged, and ONE store with all three reducers registered folds the merged log. That store is
//! what a workspace's `db.sqlite` is: one log, three sets of views.
//!
//! What this is NOT: a test of the write APIs. The ops are generated for their SHAPES. What is
//! asserted about them lives in the test.

use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::store::ChatStore;
use nexus_flow_core::model::{EdgeKind, LinkRelation, LinkWeight, MergeStrategy};
use nexus_flow_core::store::Store;
use nexus_memory::model::Scope;
use nexus_memory::store::MemoryStore;
use nxs_foundation::model::Op;
use nxs_foundation::reducer::Reducer;
use std::collections::BTreeMap;

use super::Lcg;

/// Every view table of the three platform reducers, in a stable order.
pub fn platform_view_tables() -> Vec<&'static str> {
    let mut tables: Vec<&'static str> = nexus_flow_core::task_reducer::TaskReducer
        .view_tables()
        .to_vec();
    tables.extend(nexus_chat::message_reducer::MessageReducer.view_tables());
    tables.extend(nexus_memory::fact_reducer::FactReducer.view_tables());
    tables.sort();
    tables
}

/// A store shaped like a workspace: one op log, the three products' views, the three reducers.
///
/// The view DDL of all three runs over one connection, exactly as a workspace's does when `nxf`,
/// `nxc` and `nxm` have each opened it — which is the state a based snapshot is taken of and
/// loaded into.
pub fn platform_store(site: i64) -> Store {
    let mut store = Store::open_in_memory(site);
    nexus_chat::schema::apply_chat_views(store.connection());
    nexus_memory::schema::apply_memory_views(store.connection());
    store.register_reducer(Box::new(nexus_chat::message_reducer::MessageReducer));
    store.register_reducer(Box::new(nexus_memory::fact_reducer::FactReducer));
    store
}

/// The same, file-backed — for the places a test needs two handles on one database.
pub fn platform_store_at(path: &str, site: i64) -> Store {
    let mut store = Store::open(path, site).unwrap();
    nexus_chat::schema::try_apply_chat_views(store.connection()).unwrap();
    nexus_memory::schema::apply_memory_views(store.connection());
    store.register_reducer(Box::new(nexus_chat::message_reducer::MessageReducer));
    store.register_reducer(Box::new(nexus_memory::fact_reducer::FactReducer));
    store
}

fn id(i: u64) -> String {
    format!("ab12.{i:04}")
}

fn env(channel: &str, body: &str, thread: Option<String>) -> MessageEnvelope {
    MessageEnvelope {
        origin: "o".into(),
        channel_id: channel.into(),
        sender: "o/a".into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: thread,
        refs: Refs::default(),
        body: body.into(),
    }
}

/// One site's share of the board writing: every `task` op shape the reducer folds.
fn board(s: &mut Store, r: &mut Lcg, items: u64, steps: usize) {
    for step in 0..steps {
        s.set_wall_clock(&format!(
            "2026-10-{:02}T{:02}:00:00Z",
            1 + r.next(28),
            step % 24
        ));
        let (a, b) = (id(r.next(items)), id(r.next(items)));
        match step % 11 {
            // A retype, so the `typed` partition sees a type gained, lost and changed.
            4 => s.set_field(
                &a,
                "type",
                Some(["task", "epic", "note"][step % 3].into()),
                "u",
            ),
            // List-field entries (6j6v.4g6r): adds labelled and not, removes, and a clear — the
            // sites add and remove the same entries concurrently.
            1 | 3 => {
                let label = (step % 3 != 0).then_some(["work", "home"][step % 2]);
                s.add_field_entry(&a, "emails", &format!("e{}@x", step % 5), label, "u");
            }
            5 if step % 3 == 2 => s.clear_field_entries(&a, "emails", "u"),
            5 => s.remove_field_entry(&a, "emails", &format!("e{}@x", step % 5), "u"),
            // A datetime custom field (6j6v.vbp6): a value with a time of day, which the
            // date-only shape beside it must not be confused with.
            7 => s.set_custom_field_merge(
                &a,
                "due_at",
                Some(format!(
                    "2026-11-{:02}T{:02}:30:00Z",
                    1 + r.next(28),
                    r.next(24)
                )),
                "u",
                MergeStrategy::Lww,
            ),
            _ => {}
        }
        match r.next(16) {
            0 => s.create_item(&a, ["task", "epic"][r.next(2) as usize], "T", "u"),
            1 => s.set_field(&a, "status", Some("in_progress".into()), "u"),
            2 => {
                s.set_field(&a, "status", Some("closed".into()), "u");
                if r.next(2) == 0 {
                    s.set_field(
                        &a,
                        "closed_at",
                        Some(format!("2026-09-{:02}", 1 + r.next(28))),
                        "u",
                    );
                }
            }
            3 => s.set_field(&a, "status", Some("open".into()), "u"),
            4 => s.set_field(
                &a,
                "archived",
                Some(format!("2026-10-{:02}", 1 + r.next(28))),
                "u",
            ),
            5 => s.set_field(&a, "archived", None, "u"),
            6 if r.next(8) == 0 => s.delete_item(&a, "u"),
            7 => s.set_field(
                &a,
                "defer_until",
                Some(
                    ["2027-01-01T00:00:00Z", "2026-01-01T00:00:00Z", NOW][r.next(3) as usize]
                        .into(),
                ),
                "u",
            ),
            // Edges, and removes — including a remove of an edge this site never saw added, which
            // the other site adds concurrently.
            8 | 9 => s.add_edge(
                &a,
                &b,
                [EdgeKind::Dep, EdgeKind::Parent, EdgeKind::Mentions][r.next(3) as usize],
                "u",
            ),
            10 => s.remove_edge(
                &a,
                &b,
                [EdgeKind::Dep, EdgeKind::Parent][r.next(2) as usize],
                "u",
            ),
            11 => {
                if r.next(2) == 0 {
                    s.add_label(&a, &format!("l{}", r.next(4)), "u");
                } else {
                    s.remove_label(&a, &format!("l{}", r.next(4)), "u");
                }
            }
            12 => {
                let note = s.add_note(&a, &format!("body {step}"), "u");
                if r.next(3) == 0 {
                    s.redact_note(&note, "u");
                }
            }
            13 => {
                let thread = format!("m-t{}", r.next(5));
                if r.next(3) == 0 {
                    s.remove_thread_link(&thread, &a, "u");
                } else {
                    s.add_thread_link(
                        &thread,
                        &a,
                        [LinkRelation::WorkedOn, LinkRelation::Cited][r.next(2) as usize],
                        [LinkWeight::Bearing, LinkWeight::Passing][r.next(2) as usize],
                        "u",
                    );
                }
            }
            // Chunks (6j6v.c0kn): appends, and supersedes naming ids this site may not hold —
            // including, once merged, chunks the other site wrote.
            14 => {
                let (field, other) = [("doc", "doc2"), ("doc2", "doc")][r.next(2) as usize];
                let mut live: Vec<String> = s
                    .chunks_of(&a, field)
                    .unwrap()
                    .into_iter()
                    .map(|c| c.id)
                    .collect();
                if r.next(4) == 0 {
                    live.extend(s.chunks_of(&a, other).unwrap().into_iter().map(|c| c.id));
                    live.push(format!("nobody-{step}"));
                }
                let replaced: Vec<&str> = live
                    .iter()
                    .filter(|_| r.next(2) == 0)
                    .map(String::as_str)
                    .collect();
                if replaced.is_empty() || r.next(3) != 0 {
                    s.append_chunk(&a, field, &format!("c{step}"), "u");
                } else {
                    s.supersede_chunks(&a, field, &replaced, &format!("m{step}"), "u");
                }
            }
            _ => s.set_custom_field_merge(
                &a,
                "estimate",
                Some(format!("{}", r.next(9))),
                "u",
                MergeStrategy::Lww,
            ),
        }
    }
}

/// One site's share of the chat writing.
fn chat(s: &mut ChatStore, r: &mut Lcg, steps: usize) {
    let mut threads: Vec<String> = Vec::new();
    for step in 0..steps {
        match r.next(8) {
            0..=2 => {
                let thread = (!threads.is_empty() && r.next(2) == 0)
                    .then(|| threads[r.next(threads.len() as u64) as usize].clone());
                s.post_message(&env("c-1", &format!("b{step}"), thread));
            }
            3 => {
                let parent = (!threads.is_empty() && r.next(3) == 0)
                    .then(|| threads[r.next(threads.len() as u64) as usize].clone());
                threads.push(s.open_new_thread(
                    &ThreadRoot {
                        origin: "o".into(),
                        channel_id: "c-1".into(),
                        opener: "o/a".into(),
                        created: format!("2026-10-08T00:{:02}:{:02}Z", step / 60 % 60, step % 60),
                        parent,
                    },
                    "o/a",
                ));
            }
            4 if !threads.is_empty() => {
                let t = threads[r.next(threads.len() as u64) as usize].clone();
                s.set_thread_name(&t, &format!("n{}", r.next(5)), "o/a");
            }
            5 if !threads.is_empty() => {
                let t = threads[r.next(threads.len() as u64) as usize].clone();
                s.set_deadline(&t, &format!("2026-11-{:02}", 1 + r.next(28)), "o/a");
            }
            6 => s.set_channel_field("c-1", "name", &format!("chan{}", r.next(4)), "o/a"),
            _ => s.add_member("c-1", &format!("o/m{}", r.next(5)), "o/a"),
        }
    }
}

/// One site's share of the memory writing.
fn memory(s: &mut MemoryStore, r: &mut Lcg, steps: usize) {
    for _ in 0..steps {
        let key = format!("k{}", r.next(40));
        match r.next(7) {
            0 | 1 => s.remember(&key, &format!("body {}", r.next(99)), "u"),
            2 => s.forget(&key, "u"),
            3 => s.set_category(&key, ["rules", "gotchas", "howto"][r.next(3) as usize], "u"),
            4 => s.set_scope(&key, [Scope::Item, Scope::Project][r.next(2) as usize], "u"),
            _ => s.set_introduction(&key, &format!("intro {}", r.next(99)), "u"),
        }
    }
}

/// The RESIDUE shapes: ops no reducer of this build folds, so nothing in the views stands in for
/// them and a based snapshot has to carry them verbatim (spec L5).
///
/// Three kinds, each unfoldable for a different reason: a `task` op of a shape the task reducer
/// does not know, an op of a domain no reducer is registered for, and an op whose Lamport number is
/// past the bound every fold dispatch checks (6j6v.m19v).
pub fn residue_ops(site: i64, how_many: usize) -> Vec<Op> {
    (0..how_many)
        .map(|n| {
            let mut op = Op {
                op_id: format!("residue-{site}-{n}"),
                lamport: 7 + n as i64,
                site,
                domain: "task".into(),
                target_kind: "ab12.0001".into(),
                target_id: "ab12.0001".into(),
                field: "whatever".into(),
                op_type: "set".into(),
                value: Some(format!("v{n}")),
                author: "u".into(),
                wall_clock: String::new(),
                key_id: None,
                sig: None,
            };
            match n % 3 {
                0 => op.target_kind = "a-shape-this-build-does-not-know".into(),
                1 => op.domain = "no-reducer-here".into(),
                _ => op.lamport = nxs_foundation::model::MAX_LAMPORT + 1 + n as i64,
            }
            op
        })
        .collect()
}

pub const NOW: &str = "2026-10-20T12:00:00Z";

/// A generated platform stream: `sites` writing sites, each contributing board, chat and memory
/// ops, plus the residue — merged into one log in a deterministic order.
///
/// The order it returns is the order a relay would hold them in if each site had pushed its own log
/// in slices, interleaved with the others': per site the causal order is kept (a site's ops rise in
/// its own Lamport numbers), and across sites they are interleaved — which is exactly where arrival
/// order and causal order part company.
pub fn platform_log(seed: u64, sites: i64, steps_per_site: usize) -> Vec<Op> {
    let mut per_site: Vec<Vec<Op>> = Vec::new();
    for site in 1..=sites {
        let mut r = Lcg(seed ^ (site as u64).wrapping_mul(0x9e37_79b9));
        let mut board_store = Store::open_in_memory(site);
        let items = 40 + r.next(60);
        board(&mut board_store, &mut r, items, steps_per_site);
        // Each product writes from a store of its own, so each gets a site of its own: three logs
        // minted at one site would reuse each other's (lamport, site) coordinates, and a receiver
        // keeps only the first op at a coordinate — the rest would never reach any replica.
        let mut chat_store = ChatStore::open_in_memory(site + 100);
        chat(&mut chat_store, &mut r, steps_per_site / 3);
        let mut memory_store = MemoryStore::open_in_memory(site + 200);
        memory(&mut memory_store, &mut r, steps_per_site / 4);
        let mut ops = board_store.export();
        ops.extend(chat_store.export());
        ops.extend(memory_store.export());
        ops.extend(residue_ops(site + 300, 4));
        // Per site, causal order: the log as that site appended it.
        ops.sort_by_key(|o| (o.lamport, o.site, o.op_id.clone()));
        per_site.push(ops);
    }
    // Interleaved in slices, so the relay's order is neither one site's nor the causal one.
    let mut r = Lcg(seed ^ 0x5eed_5eed);
    let mut cursors = vec![0usize; per_site.len()];
    let mut merged = Vec::new();
    while cursors.iter().zip(&per_site).any(|(c, s)| *c < s.len()) {
        for (i, site_ops) in per_site.iter().enumerate() {
            let take = 1 + r.next(40) as usize;
            let end = (cursors[i] + take).min(site_ops.len());
            merged.extend_from_slice(&site_ops[cursors[i]..end]);
            cursors[i] = end;
        }
    }
    merged
}

/// Every view table of the three products as a sorted row set, with every column — the whole folded
/// state of a platform store, rendered so two stores can be compared without caring about order.
pub fn platform_rows(s: &Store) -> BTreeMap<String, Vec<Vec<(String, String)>>> {
    platform_view_tables()
        .into_iter()
        .map(|table| (table.to_string(), super::sqlite_rows(s, table)))
        .collect()
}

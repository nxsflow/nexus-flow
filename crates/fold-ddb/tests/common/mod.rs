//! What the fold tests share: a deterministic generator of merged boards, and the rows of a view
//! table as SQLite holds them and as the server holds them.
#![allow(dead_code)]

use nexus_flow_core::model::{EdgeKind, LinkRelation, LinkWeight, MergeStrategy};
use nexus_flow_core::store::Store;
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::layout::table_prefix;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::table::Row;
use nxs_foundation::change::Cell;
use nxs_foundation::model::Op;
use std::collections::BTreeMap;

pub const NOW: &str = "2026-10-08T12:00:00Z";

pub struct Lcg(pub u64);
impl Lcg {
    pub fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n.max(1)
    }
}

pub fn id(i: u64) -> String {
    format!("ab12.{i:04}")
}

/// One replica's share of the writing.
fn write(s: &mut Store, r: &mut Lcg, n: u64, steps: usize) {
    for step in 0..steps {
        s.set_wall_clock(&format!(
            "2026-10-0{}T{:02}:00:00Z",
            1 + r.next(7),
            step % 24
        ));
        let (a, b) = (id(r.next(n)), id(r.next(n + 1)));
        match r.next(16) {
            0 => s.create_item(&a, "task", "T", "u"),
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
                Some(format!("2026-10-0{}", 1 + r.next(7))),
                "u",
            ),
            5 => s.set_field(&a, "archived", None, "u"),
            6 if r.next(4) == 0 => s.delete_item(&a, "u"),
            7 => s.set_field(
                &a,
                "defer_until",
                Some(
                    ["2027-01-01T00:00:00Z", "2026-01-01T00:00:00Z", NOW][r.next(3) as usize]
                        .into(),
                ),
                "u",
            ),
            8 | 9 => {
                let kind =
                    [EdgeKind::Dep, EdgeKind::Parent, EdgeKind::Mentions][r.next(3) as usize];
                s.add_edge(&a, &b, kind, "u");
            }
            10 => {
                let kind = [EdgeKind::Dep, EdgeKind::Parent][r.next(2) as usize];
                s.remove_edge(&a, &b, kind, "u");
            }
            11 => {
                if r.next(2) == 0 {
                    s.add_label(&a, "l", "u");
                } else {
                    s.remove_label(&a, "l", "u");
                }
            }
            12 => {
                let note = s.add_note(&a, "body", "u");
                if r.next(3) == 0 {
                    s.redact_note(&note, "u");
                }
            }
            13 => {
                let thread = format!("m-t{}", r.next(3));
                if r.next(3) == 0 {
                    s.remove_thread_link(&thread, &a, "u");
                } else {
                    let relation =
                        [LinkRelation::WorkedOn, LinkRelation::Cited][r.next(2) as usize];
                    let weight = [LinkWeight::Bearing, LinkWeight::Passing][r.next(2) as usize];
                    s.add_thread_link(&thread, &a, relation, weight, "u");
                }
            }
            // Chunks (6j6v.c0kn): appends to two fields, and now and then a supersede of a random
            // part of what this replica sees — with a chunk the other replica wrote, once merged.
            14 => {
                // `doc` is a prefix of `doc2`: the server's per-field range must not mix them.
                let (field, other) = [("doc", "doc2"), ("doc2", "doc")][r.next(2) as usize];
                let mut live: Vec<String> = s
                    .chunks_of(&a, field)
                    .unwrap()
                    .into_iter()
                    .map(|c| c.id)
                    .collect();
                // Now and then also a chunk of the OTHER field and one nobody wrote: a supersede
                // names ids, it does not check them, and neither may hide anything it should not.
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

/// A merged board: two replicas that wrote apart, then exchanged everything.
pub fn merged(seed: u64) -> Store {
    let mut r = Lcg(seed);
    let n = 4 + r.next(8);
    let mut a = Store::open_in_memory(1);
    let mut b = Store::open_in_memory(2);
    for i in 0..n {
        a.create_item(&id(i), "task", "T", "u");
    }
    b.apply(&a.export());
    let steps = 10 + r.next(40) as usize;
    write(&mut a, &mut r, n, steps);
    write(&mut b, &mut r, n, steps);
    let from_b = b.export();
    a.apply(&from_b);
    a
}

/// The log of `s` as a relay without deduplication may hand it over: shuffled, and roughly one op
/// in five twice, numbered by relay position.
pub fn delivered(s: &Store, seed: u64) -> Vec<(i64, Op)> {
    let mut r = Lcg(seed ^ 0x5eed);
    let mut ops = s.export();
    for i in (1..ops.len()).rev() {
        ops.swap(i, r.next(i as u64 + 1) as usize);
    }
    let twice: Vec<_> = ops.iter().filter(|_| r.next(5) == 0).cloned().collect();
    ops.extend(twice);
    ops.into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect()
}

/// Every row of `table`, every column filled — as SQLite holds it.
pub fn sqlite_rows(s: &Store, table: &str) -> Vec<Vec<(String, String)>> {
    let conn = s.connection();
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
    let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let mut rows: Vec<Vec<(String, String)>> = stmt
        .query_map([], |r| {
            Ok(columns
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let cell = match r.get_ref(i).unwrap() {
                        rusqlite::types::ValueRef::Null => Cell::Null,
                        rusqlite::types::ValueRef::Integer(v) => Cell::Int(v),
                        rusqlite::types::ValueRef::Text(t) => {
                            Cell::Text(String::from_utf8(t.to_vec()).unwrap())
                        }
                        other => panic!("{other:?}"),
                    };
                    (c.clone(), format!("{cell:?}"))
                })
                .collect())
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    rows.sort();
    rows
}

/// Every row of `table` in the server's partition, an unwritten column read as its default.
pub fn served_rows(t: &MemTable, folder: &Folder, table: &str) -> Vec<Vec<(String, String)>> {
    let layout = folder.layout().table(table).unwrap();
    let prefix = table_prefix(table);
    let mut rows: Vec<Vec<(String, String)>> = t
        .rows()
        .into_iter()
        .filter(|(sk, _)| sk.starts_with(&prefix))
        .map(|(_, row)| {
            layout
                .columns
                .iter()
                .map(|c| {
                    let cell = row.get(c).cloned().unwrap_or_else(|| layout.default_of(c));
                    (c.clone(), format!("{cell:?}"))
                })
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

/// The server's fold of `s`'s log, as [`delivered`] hands it over.
pub fn serve(s: &Store, seed: u64, folder: &Folder) -> MemTable {
    let t = MemTable::new("stream-1");
    ready(folder.fold_batch(&t, &delivered(s, seed))).unwrap();
    t
}

/// The rows of a stored partition, by sort key.
pub fn by_key(rows: Vec<Row>) -> BTreeMap<String, Row> {
    rows.into_iter()
        .map(|r| match r.get("sk") {
            Some(Cell::Text(sk)) => (sk.clone(), r),
            other => panic!("a row without a sort key: {other:?}"),
        })
        .collect()
}

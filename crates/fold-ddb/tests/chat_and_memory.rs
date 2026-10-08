//! The applier is the same for every reducer (6j6v.vvw6 point 1): the chat's and the memory's
//! views, folded on the server in any order with ops delivered twice, hold exactly the rows the
//! local fold holds — the chat's lowest-wins message and thread registers and the memory's seeded
//! row among them.

use nexus_chat::message_reducer::MessageReducer;
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::store::ChatStore;
use nexus_memory::fact_reducer::FactReducer;
use nexus_memory::model::Scope;
use nexus_memory::store::MemoryStore;
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::layout::table_prefix;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_foundation::change::Cell;
use nxs_foundation::model::Op;
use nxs_foundation::reducer::Reducer;
use rusqlite::Connection;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n.max(1)
    }
}

fn rows_of(conn: &Connection, table: &str) -> Vec<Vec<(String, String)>> {
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

fn served(t: &MemTable, folder: &Folder, table: &str) -> Vec<Vec<(String, String)>> {
    let layout = folder.layout().table(table).unwrap();
    let mut rows: Vec<Vec<(String, String)>> = t
        .rows()
        .into_iter()
        .filter(|(sk, _)| sk.starts_with(&table_prefix(table)))
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

fn serve(mut ops: Vec<Op>, seed: u64, folder: &Folder) -> MemTable {
    let mut r = Lcg(seed ^ 0xc4a7);
    for i in (1..ops.len()).rev() {
        ops.swap(i, r.next(i as u64 + 1) as usize);
    }
    let twice: Vec<Op> = ops.iter().filter(|_| r.next(4) == 0).cloned().collect();
    ops.extend(twice);
    let t = MemTable::new("stream-1");
    let batch: Vec<(i64, Op)> = ops
        .into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();
    ready(folder.fold_batch(&t, &batch)).unwrap();
    t
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

fn chat(s: &mut ChatStore, r: &mut Lcg, threads: &mut Vec<String>) {
    for step in 0..30 {
        match r.next(7) {
            0 | 1 => {
                let thread = (!threads.is_empty() && r.next(2) == 0)
                    .then(|| threads[r.next(threads.len() as u64) as usize].clone());
                s.post_message(&env("c-1", &format!("b{step}"), thread));
            }
            2 => {
                let parent = (!threads.is_empty() && r.next(2) == 0)
                    .then(|| threads[r.next(threads.len() as u64) as usize].clone());
                let root = ThreadRoot {
                    origin: "o".into(),
                    channel_id: "c-1".into(),
                    opener: "o/a".into(),
                    created: format!("2026-10-08T00:00:{step:02}Z"),
                    parent,
                };
                threads.push(s.open_new_thread(&root, "o/a"));
            }
            3 if !threads.is_empty() => {
                let t = threads[r.next(threads.len() as u64) as usize].clone();
                s.set_thread_name(&t, &format!("n{}", r.next(3)), "o/a");
            }
            4 if !threads.is_empty() => {
                let t = threads[r.next(threads.len() as u64) as usize].clone();
                s.set_deadline(&t, &format!("2026-11-0{}", 1 + r.next(8)), "o/a");
            }
            5 => s.set_channel_field("c-1", "name", &format!("chan{}", r.next(3)), "o/a"),
            _ => s.add_member("c-1", &format!("o/m{}", r.next(3)), "o/a"),
        }
    }
}

#[test]
fn the_chat_views_fold_on_the_server_as_they_fold_locally() {
    let folder = Folder::platform().unwrap();
    let (mut messages, mut threads) = (0, 0);
    for seed in 0..30 {
        let mut r = Lcg(seed);
        let (mut a, mut b) = (ChatStore::open_in_memory(1), ChatStore::open_in_memory(2));
        let (mut ta, mut tb) = (Vec::new(), Vec::new());
        chat(&mut a, &mut r, &mut ta);
        chat(&mut b, &mut r, &mut tb);
        let from_b = b.export();
        a.apply(&from_b);
        let t = serve(a.export(), seed, &folder);
        messages += rows_of(a.connection(), "messages").len();
        threads += rows_of(a.connection(), "threads").len();
        for table in MessageReducer.view_tables() {
            assert_eq!(
                served(&t, &folder, table),
                rows_of(a.connection(), table),
                "{table}, seed {seed}"
            );
        }
    }
    assert!(
        messages > 0 && threads > 0,
        "{messages} messages, {threads} threads"
    );
}

#[test]
fn the_memory_views_fold_on_the_server_as_they_fold_locally() {
    let folder = Folder::platform().unwrap();
    let mut memories = 0;
    for seed in 0..30 {
        let mut r = Lcg(seed);
        let (mut a, mut b) = (
            MemoryStore::open_in_memory(1),
            MemoryStore::open_in_memory(2),
        );
        for s in [&mut a, &mut b] {
            for _ in 0..25 {
                let key = format!("k{}", r.next(4));
                match r.next(6) {
                    0 | 1 => s.remember(&key, &format!("body {}", r.next(9)), "u"),
                    2 => s.forget(&key, "u"),
                    3 => s.set_category(&key, ["rules", "gotchas"][r.next(2) as usize], "u"),
                    4 => s.set_scope(&key, [Scope::Item, Scope::Project][r.next(2) as usize], "u"),
                    _ => s.set_introduction(&key, &format!("intro {}", r.next(9)), "u"),
                }
            }
        }
        let from_b = b.export();
        a.apply(&from_b);
        let t = serve(a.export(), seed, &folder);
        memories += rows_of(a.connection(), "memories").len();
        for table in FactReducer.view_tables() {
            assert_eq!(
                served(&t, &folder, table),
                rows_of(a.connection(), table),
                "{table}, seed {seed}"
            );
        }
    }
    assert!(memories > 0);
}

#[test]
fn a_register_a_snapshot_carries_as_null_is_still_unwritten_after_the_import() {
    // A thread named before its open arrived: locally its root version is NULL — unwritten. Started
    // from a snapshot of that replica, the server must still let the open op take the root.
    let folder = Folder::platform().unwrap();
    let mut a = ChatStore::open_in_memory(1);
    let root = ThreadRoot {
        origin: "o".into(),
        channel_id: "c-1".into(),
        opener: "o/a".into(),
        created: "2026-10-08T00:00:00Z".into(),
        parent: None,
    };
    let thread = a.open_new_thread(&root, "o/a");
    a.set_thread_name(&thread, "named first", "o/a");
    let ops = a.export();
    let (open, rest): (Vec<Op>, Vec<Op>) = ops.into_iter().partition(|op| op.op_type == "open");
    assert_eq!(open.len(), 1);

    let mut b = ChatStore::open_in_memory(2);
    b.apply(&rest);
    let root_v: Option<i64> = b
        .connection()
        .query_row("SELECT root_v FROM threads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(root_v, None, "the local root is unwritten");
    let snap = nxs_fold_ddb::snapshot::from_sqlite(b.connection(), &folder, "stream-1", 1).unwrap();
    let t = MemTable::new("stream-1");
    ready(nxs_fold_ddb::snapshot::import(&t, &folder, &snap)).unwrap();
    ready(folder.fold_batch(&t, &[(2, open[0].clone())])).unwrap();
    assert_eq!(
        served(&t, &folder, "threads"),
        rows_of(a.connection(), "threads")
    );
}

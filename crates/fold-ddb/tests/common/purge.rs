//! The board-fold purge's fixture (6j6v.dbmd): one platform stream that leaves every kind of row a
//! board-fold partition can hold, with byte markers unique to the stream in every piece of content,
//! so a test can say "nothing of the target is left anywhere" by searching bytes.

use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::store::ChatStore;
use nexus_flow_core::model::{EdgeKind, LinkRelation, LinkWeight, MergeStrategy};
use nexus_flow_core::store::Store;
use nexus_memory::store::MemoryStore;
use nxs_foundation::model::Op;

/// An account's `<h>`: the target and its two neighbours share it, the fourth stream does not.
pub const H: &str = "00ff00ff00ff00ff00ff00ff00ff00ff";
/// Another account.
pub const H2: &str = "11ee11ee11ee11ee11ee11ee11ee11ee";

/// log-id v1 C9's key.
pub const C9_KEY: &str = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

/// The stream a purge deletes.
pub fn target() -> String {
    format!("n-{H}-1")
}

/// Same account, a key that `begins_with(target)` would reach: `n-<h>-10`.
pub fn neighbour() -> String {
    format!("n-{H}-10")
}

/// Same account, the target plus a suffix: `n-<h>-1-x`.
pub fn prefix_neighbour() -> String {
    format!("n-{H}-1-x")
}

/// Another account's stream.
pub fn other() -> String {
    format!("n-{H2}-1")
}

/// The byte markers of a stream's fixture: every piece of content it writes carries one.
pub fn markers(tag: &str) -> Vec<String> {
    (0..9).map(|n| format!("ZQ{tag}MARK{n}")).collect()
}

fn item(n: u64) -> String {
    format!("ab12.{n:04}")
}

/// The fixture ops minted so far, by tag.
type Minted = std::collections::HashMap<String, Vec<(i64, Op)>>;

/// The fixture's ops for `tag`, the SAME ops on every call in a process (the stores mint random op
/// ids, and sort keys carry them): so a control fold, a re-fold after a purge and the list of keys
/// no report may print all name exactly the keys the folded partition holds.
pub fn fixture_ops(tag: &str) -> Vec<(i64, Op)> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Minted>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(tag.to_string())
        .or_insert_with(|| mint_fixture_ops(tag))
        .clone()
}

/// The fixture's ops, numbered by relay position: items with custom fields, entries, labels, notes,
/// chunks and edges; a closed, an archived and a typed item; an item whose ref field holds another
/// item's id (so `.radj#` and `.rsrc#` rows exist); a deleted item with markers in its content; a
/// thread link; chat and memory ops. `tag` makes the markers this stream's own.
fn mint_fixture_ops(tag: &str) -> Vec<(i64, Op)> {
    let m = markers(tag);
    let mut s = Store::open_in_memory(1);
    s.set_wall_clock("2026-10-20T10:00:00Z");
    s.create_item(&item(1), "task", &format!("first {}", m[0]), "u");
    s.create_item(&item(2), "epic", "second", "u");
    s.create_item(&item(3), "task", "third", "u");
    s.create_item(&item(4), "note", "fourth", "u");
    s.create_item(&item(5), "task", &format!("to be deleted {}", m[1]), "u");
    s.set_custom_field_merge(
        &item(1),
        "estimate",
        Some("3".into()),
        "u",
        MergeStrategy::Lww,
    );
    // A ref field: an id-shaped value, so the fold keeps a `.radj#` hint and its `.rsrc#` mirror.
    s.set_custom_field_merge(&item(1), "owner", Some(item(2)), "u", MergeStrategy::Lww);
    s.add_field_entry(
        &item(1),
        "emails",
        &format!("{}@x", m[2]),
        Some("work"),
        "u",
    );
    s.add_label(&item(1), "l1", "u");
    s.add_note(&item(1), &format!("a note {}", m[3]), "u");
    s.append_chunk(&item(1), "doc", &format!("a chunk {}", m[4]), "u");
    s.add_edge(&item(1), &item(2), EdgeKind::Dep, "u");
    s.add_edge(&item(3), &item(2), EdgeKind::Parent, "u");
    s.add_edge(&item(1), &item(3), EdgeKind::ContributesTo, "u");
    s.add_thread_link(
        "m-t1",
        &item(1),
        LinkRelation::WorkedOn,
        LinkWeight::Bearing,
        "u",
    );
    s.set_field(&item(3), "status", Some("closed".into()), "u");
    s.set_field(&item(3), "closed_at", Some("2026-10-19".into()), "u");
    s.set_field(&item(4), "archived", Some("2026-10-18".into()), "u");
    // The deleted item keeps content the purge must remove too.
    s.add_note(&item(5), &format!("a deleted note {}", m[5]), "u");
    s.set_custom_field_merge(
        &item(5),
        "secret",
        Some(m[6].clone()),
        "u",
        MergeStrategy::Lww,
    );
    s.delete_item(&item(5), "u");

    let mut chat = ChatStore::open_in_memory(101);
    let thread = chat.open_new_thread(
        &ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/a".into(),
            created: "2026-10-20T10:00:00Z".into(),
            parent: None,
        },
        "o/a",
    );
    chat.post_message(&MessageEnvelope {
        origin: "o".into(),
        channel_id: "c-1".into(),
        sender: "o/a".into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some(thread),
        refs: Refs::default(),
        body: format!("a message {}", m[7]),
    });
    let mut memory = MemoryStore::open_in_memory(201);
    memory.remember("k1", &format!("a memory {}", m[8]), "u");

    let mut ops = s.export();
    ops.extend(chat.export());
    ops.extend(memory.export());
    ops.into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect()
}

/// One more op of a stream, as a late change-stream record would fold it: a new item.
pub fn late_ops(tag: &str, n: u64) -> Vec<(i64, Op)> {
    let mut s = Store::open_in_memory(50 + n as i64);
    s.set_wall_clock("2026-10-21T10:00:00Z");
    s.create_item(&format!("late.{n:04}"), "task", &format!("late {tag}"), "u");
    s.export()
        .into_iter()
        .enumerate()
        .map(|(i, op)| (10_000 + 10 * n as i64 + i as i64, op))
        .collect()
}

//! The snapshot cache behind `next --paginate` (6j6v.15ed): the ordered result a first page was
//! cut from, kept so the following pages come from the SAME list instead of a list that moved
//! underneath the reader.
//!
//! **Machine-local.** It lives in the `next_page_cache` table of the workspace db. The table is not
//! a view and is folded from nothing: no op writes it, no snapshot image carries it, no sync moves
//! it, and `clear_views` leaves it alone. Another replica cannot read a token minted here; it gets
//! the first page afresh, as for any token it does not hold.
//!
//! **Bounded.** Every insert first drops the snapshots older than the expiry the caller names, then
//! keeps only the newest `max` of what is left. The facade's numbers are in
//! `nexus_flow_facade::read::NEXT_CACHE_TTL_SECS` and `NEXT_CACHE_MAX_SNAPSHOTS`.
//!
//! What is cached and what it is checked against is the facade's business; this module stores rows
//! and answers one question about the log: which items did the ops after a watermark touch?

use crate::model::DOMAIN_TASK;
use crate::store::{Store, SEP};
use rusqlite::{params, OptionalExtension};
use std::collections::BTreeSet;

/// One cached ordered result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedResult {
    /// The snapshot's name, which the page token carries.
    pub id: String,
    /// When it was computed, in unix seconds. The expiry reads it.
    pub created: i64,
    /// The filters and sort it was computed under, as the facade spells them.
    pub query: String,
    /// The highest `ops.rowid` when it was computed ([`Store::ops_watermark`]).
    pub watermark: i64,
    /// The item ids, in the order handed out.
    pub ids: Vec<String>,
}

impl Store {
    /// The highest local `ops.rowid`, `0` for an empty log. `ops.rowid` is the local append order,
    /// so every op this replica takes in afterwards, its own or a peer's, lands above it.
    pub fn ops_watermark(&self) -> rusqlite::Result<i64> {
        self.connection()
            .query_row("SELECT COALESCE(MAX(rowid), 0) FROM ops", [], |r| r.get(0))
    }

    /// Store `snapshot`, after dropping every snapshot created before `expire_before` and all but
    /// the newest `max - 1` of the rest, so the table never holds more than `max` rows.
    pub fn next_cache_put(
        &self,
        snapshot: &CachedResult,
        expire_before: i64,
        max: usize,
    ) -> rusqlite::Result<()> {
        let tx = self.connection().unchecked_transaction()?;
        tx.execute(
            "DELETE FROM next_page_cache WHERE created < ?1",
            [expire_before],
        )?;
        let keep = i64::try_from(max.saturating_sub(1)).unwrap_or(i64::MAX);
        tx.execute(
            "DELETE FROM next_page_cache WHERE id NOT IN (
                 SELECT id FROM next_page_cache ORDER BY created DESC, id DESC LIMIT ?1)",
            [keep],
        )?;
        let ids = snapshot.ids.join(&SEP.to_string());
        tx.execute(
            "INSERT OR REPLACE INTO next_page_cache(id, created, query, watermark, ids)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                snapshot.id,
                snapshot.created,
                snapshot.query,
                snapshot.watermark,
                ids
            ],
        )?;
        tx.commit()
    }

    /// The snapshot named `id`, if this replica holds it.
    pub fn next_cache_get(&self, id: &str) -> rusqlite::Result<Option<CachedResult>> {
        self.connection()
            .query_row(
                "SELECT id, created, query, watermark, ids FROM next_page_cache WHERE id = ?1",
                [id],
                |r| {
                    let ids: String = r.get(4)?;
                    Ok(CachedResult {
                        id: r.get(0)?,
                        created: r.get(1)?,
                        query: r.get(2)?,
                        watermark: r.get(3)?,
                        ids: if ids.is_empty() {
                            Vec::new()
                        } else {
                            ids.split(SEP).map(str::to_string).collect()
                        },
                    })
                },
            )
            .optional()
    }

    /// The items the `task` ops above `watermark` target: an item's own cells and custom fields,
    /// its notes (a redaction through the note it redacts), its chunks, its labels, its thread
    /// links, and BOTH ends of an edge. An op whose item cannot be named (a redaction of a note
    /// this replica never saw) names none.
    pub fn items_touched_since(&self, watermark: i64) -> rusqlite::Result<BTreeSet<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT target_kind, target_id, op_type FROM ops
             WHERE rowid > ?1 AND domain = ?2",
        )?;
        let ops = stmt
            .query_map(params![watermark, DOMAIN_TASK], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = BTreeSet::new();
        for (kind, target, op_type) in ops {
            let mut parts = target.split(SEP);
            match (kind.as_str(), op_type.as_str()) {
                ("item" | "field" | "chunk", _) | ("note", "note_add") => {
                    out.insert(target);
                }
                ("note", _) => {
                    let item: Option<String> = conn
                        .query_row("SELECT item_id FROM notes WHERE id = ?1", [&target], |r| {
                            r.get(0)
                        })
                        .optional()?;
                    out.extend(item);
                }
                ("label", _) => out.extend(parts.next().map(str::to_string)),
                ("edge", _) => out.extend(parts.take(2).map(str::to_string)),
                ("thread_link", _) => out.extend(parts.nth(1).map(str::to_string)),
                _ => {}
            }
        }
        out.retain(|id| !id.is_empty());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EdgeKind;

    fn snap(id: &str, created: i64) -> CachedResult {
        CachedResult {
            id: id.into(),
            created,
            query: "q".into(),
            watermark: 0,
            ids: vec!["a".into(), "b".into()],
        }
    }

    #[test]
    fn a_snapshot_reads_back_as_it_was_stored() {
        let s = Store::open_in_memory(1);
        s.next_cache_put(&snap("s1", 100), 0, 8).unwrap();
        assert_eq!(s.next_cache_get("s1").unwrap(), Some(snap("s1", 100)));
        assert_eq!(s.next_cache_get("nope").unwrap(), None);
    }

    #[test]
    fn an_insert_drops_expired_snapshots_and_keeps_at_most_max() {
        let s = Store::open_in_memory(1);
        s.next_cache_put(&snap("old", 10), 0, 3).unwrap();
        s.next_cache_put(&snap("s2", 200), 0, 3).unwrap();
        s.next_cache_put(&snap("s3", 300), 0, 3).unwrap();
        // Expiry: everything created before 100 goes.
        s.next_cache_put(&snap("s4", 400), 100, 3).unwrap();
        assert_eq!(s.next_cache_get("old").unwrap(), None, "expired");
        // The cap: s2, s3, s4 are three; a fifth evicts the oldest.
        s.next_cache_put(&snap("s5", 500), 100, 3).unwrap();
        assert_eq!(
            s.next_cache_get("s2").unwrap(),
            None,
            "the oldest beyond the cap"
        );
        for id in ["s3", "s4", "s5"] {
            assert!(s.next_cache_get(id).unwrap().is_some(), "{id} kept");
        }
        let n: i64 = s
            .connection()
            .query_row("SELECT COUNT(*) FROM next_page_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn items_touched_since_names_every_item_an_op_bears_on() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.create_item("c1.B", "task", "B", "x");
        s.create_item("c1.C", "task", "C", "x");
        s.create_item("c1.D", "task", "D", "x");
        s.create_item("c1.E", "task", "E", "x");
        s.create_item("c1.F", "task", "F", "x");
        let note = s.add_note("c1.F", "n", "x");
        let w = s.ops_watermark().unwrap();
        assert!(s.items_touched_since(w).unwrap().is_empty());

        s.set_field("c1.A", "title", Some("A2".into()), "x");
        s.add_edge("c1.B", "c1.C", EdgeKind::Dep, "x");
        s.add_label("c1.D", "l", "x");
        s.redact_note(&note, "x");
        let touched: Vec<String> = s.items_touched_since(w).unwrap().into_iter().collect();
        assert_eq!(touched, ["c1.A", "c1.B", "c1.C", "c1.D", "c1.F"]);
    }
}

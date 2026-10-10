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
//! keeps only the newest `max_per_query` snapshots of the inserted snapshot's query and the newest
//! `max_total` overall. A reader polling one query churns its own slots, never another query's. The
//! facade's numbers are `nexus_flow_facade::read::NEXT_CACHE_TTL_SECS`,
//! `NEXT_CACHE_MAX_PER_QUERY` and `NEXT_CACHE_MAX_SNAPSHOTS`.
//!
//! What is cached and what it is checked against is the facade's business; this module stores rows
//! and answers two questions about the log: which op sits at a watermark, and which items did the
//! ops after it touch?

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
    /// The id of the op at `watermark` ([`Store::op_at`]), `None` for an empty log: the log's
    /// identity at that point. A log that holds another op there is another log.
    pub watermark_op: Option<String>,
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

    /// The id of the op at `rowid`, if the log holds one there.
    pub fn op_at(&self, rowid: i64) -> rusqlite::Result<Option<String>> {
        self.connection()
            .query_row("SELECT op_id FROM ops WHERE rowid = ?1", [rowid], |r| {
                r.get(0)
            })
            .optional()
    }

    /// Store `snapshot`, after dropping every snapshot created before `expire_before`, all but the
    /// newest `max_per_query - 1` of the snapshots of the same query, and all but the newest
    /// `max_total - 1` of the rest — so the table holds at most `max_total` rows, at most
    /// `max_per_query` of them for one query. A bound of `0` reads as `1`: the snapshot being
    /// stored is always kept.
    pub fn next_cache_put(
        &self,
        snapshot: &CachedResult,
        expire_before: i64,
        max_per_query: usize,
        max_total: usize,
    ) -> rusqlite::Result<()> {
        let keep = |max: usize| i64::try_from(max.max(1) - 1).unwrap_or(i64::MAX);
        let tx = self.connection().unchecked_transaction()?;
        tx.execute(
            "DELETE FROM next_page_cache WHERE created < ?1",
            [expire_before],
        )?;
        tx.execute(
            "DELETE FROM next_page_cache WHERE query = ?1 AND id NOT IN (
                 SELECT id FROM next_page_cache WHERE query = ?1
                 ORDER BY created DESC, id DESC LIMIT ?2)",
            params![snapshot.query, keep(max_per_query)],
        )?;
        tx.execute(
            "DELETE FROM next_page_cache WHERE id NOT IN (
                 SELECT id FROM next_page_cache ORDER BY created DESC, id DESC LIMIT ?1)",
            [keep(max_total)],
        )?;
        let ids = snapshot.ids.join(&SEP.to_string());
        tx.execute(
            "INSERT OR REPLACE INTO next_page_cache(id, created, query, watermark, watermark_op, ids)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.id,
                snapshot.created,
                snapshot.query,
                snapshot.watermark,
                snapshot.watermark_op,
                ids
            ],
        )?;
        tx.commit()
    }

    /// The snapshot named `id`, if this replica holds it.
    pub fn next_cache_get(&self, id: &str) -> rusqlite::Result<Option<CachedResult>> {
        self.connection()
            .query_row(
                "SELECT id, created, query, watermark, watermark_op, ids
                 FROM next_page_cache WHERE id = ?1",
                [id],
                |r| {
                    let ids: String = r.get(5)?;
                    Ok(CachedResult {
                        id: r.get(0)?,
                        created: r.get(1)?,
                        query: r.get(2)?,
                        watermark: r.get(3)?,
                        watermark_op: r.get(4)?,
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
    use crate::model::{EdgeKind, LinkRelation, LinkWeight, MergeStrategy};
    use crate::reducer::Reducer as _;
    use crate::task_reducer::TaskReducer;

    fn snap_of(id: &str, created: i64, query: &str) -> CachedResult {
        CachedResult {
            id: id.into(),
            created,
            query: query.into(),
            watermark: 0,
            watermark_op: None,
            ids: vec!["a".into(), "b".into()],
        }
    }

    fn snap(id: &str, created: i64) -> CachedResult {
        snap_of(id, created, "q")
    }

    fn rows(s: &Store) -> i64 {
        s.connection()
            .query_row("SELECT COUNT(*) FROM next_page_cache", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_snapshot_reads_back_as_it_was_stored() {
        let s = Store::open_in_memory(1);
        let mut one = snap("s1", 100);
        one.watermark = 7;
        one.watermark_op = Some("op-7".into());
        s.next_cache_put(&one, 0, 8, 8).unwrap();
        assert_eq!(s.next_cache_get("s1").unwrap(), Some(one));
        assert_eq!(s.next_cache_get("nope").unwrap(), None);
    }

    #[test]
    fn an_insert_drops_expired_snapshots_and_keeps_at_most_max() {
        let s = Store::open_in_memory(1);
        s.next_cache_put(&snap("old", 10), 0, 3, 3).unwrap();
        s.next_cache_put(&snap("s2", 200), 0, 3, 3).unwrap();
        s.next_cache_put(&snap("s3", 300), 0, 3, 3).unwrap();
        // Expiry: everything created before 100 goes.
        s.next_cache_put(&snap("s4", 400), 100, 3, 3).unwrap();
        assert_eq!(s.next_cache_get("old").unwrap(), None, "expired");
        // The cap: s2, s3, s4 are three; a fifth evicts the oldest.
        s.next_cache_put(&snap("s5", 500), 100, 3, 3).unwrap();
        assert_eq!(
            s.next_cache_get("s2").unwrap(),
            None,
            "the oldest beyond the cap"
        );
        for id in ["s3", "s4", "s5"] {
            assert!(s.next_cache_get(id).unwrap().is_some(), "{id} kept");
        }
        assert_eq!(rows(&s), 3);
    }

    #[test]
    fn the_per_query_cap_evicts_only_that_querys_snapshots() {
        // Review of #43, Integrity #3: one reader polling one query must not evict another's.
        let s = Store::open_in_memory(1);
        s.next_cache_put(&snap_of("other", 100, "B"), 0, 2, 10)
            .unwrap();
        for (n, id) in ["a1", "a2", "a3", "a4"].iter().enumerate() {
            s.next_cache_put(&snap_of(id, 200 + n as i64, "A"), 0, 2, 10)
                .unwrap();
        }
        assert!(
            s.next_cache_get("other").unwrap().is_some(),
            "query B untouched"
        );
        assert_eq!(s.next_cache_get("a1").unwrap(), None);
        assert_eq!(s.next_cache_get("a2").unwrap(), None);
        assert!(s.next_cache_get("a3").unwrap().is_some());
        assert!(s.next_cache_get("a4").unwrap().is_some());
        assert_eq!(rows(&s), 3);
    }

    #[test]
    fn a_bound_of_zero_or_one_keeps_exactly_the_snapshot_stored() {
        let s = Store::open_in_memory(1);
        for max in [0, 1] {
            s.next_cache_put(&snap(&format!("x{max}"), 100), 0, max, max)
                .unwrap();
            s.next_cache_put(&snap(&format!("y{max}"), 101), 0, max, max)
                .unwrap();
            assert_eq!(rows(&s), 1, "max {max}");
            assert!(s.next_cache_get(&format!("y{max}")).unwrap().is_some());
        }
    }

    #[test]
    fn the_expiry_bound_is_a_floor_not_a_ceiling() {
        // `expire_before` drops what is OLDER; a younger snapshot survives it.
        let s = Store::open_in_memory(1);
        s.next_cache_put(&snap("young", 500), 0, 8, 8).unwrap();
        s.next_cache_put(&snap("new", 600), 400, 8, 8).unwrap();
        assert!(s.next_cache_get("young").unwrap().is_some());
    }

    #[test]
    fn op_at_names_the_op_at_a_rowid() {
        let mut s = Store::open_in_memory(1);
        assert_eq!(s.op_at(s.ops_watermark().unwrap()).unwrap(), None);
        s.create_item("c1.A", "task", "A", "x");
        let w = s.ops_watermark().unwrap();
        let id = s.op_at(w).unwrap().expect("an op at the watermark");
        assert!(s.export().iter().any(|o| o.op_id == id));
    }

    #[test]
    fn items_touched_since_names_every_item_an_op_bears_on() {
        let mut s = Store::open_in_memory(1);
        for id in ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K"] {
            s.create_item(&format!("c1.{id}"), "task", id, "x");
        }
        let note = s.add_note("c1.F", "n", "x");
        let w = s.ops_watermark().unwrap();
        assert!(s.items_touched_since(w).unwrap().is_empty());

        s.set_field("c1.A", "title", Some("A2".into()), "x");
        s.add_edge("c1.B", "c1.C", EdgeKind::Dep, "x");
        s.add_label("c1.D", "l", "x");
        s.redact_note(&note, "x");
        // Review of #43, Test Quality #7: every other target shape.
        s.add_note("c1.G", "a new note", "x");
        s.append_chunk("c1.H", "doc", "c", "x");
        s.set_custom_field_merge(
            "c1.I",
            "estimate",
            Some("3".into()),
            "x",
            MergeStrategy::Lww,
        );
        s.add_thread_link(
            "t-1",
            "c1.J",
            LinkRelation::WorkedOn,
            LinkWeight::Bearing,
            "x",
        );
        s.remove_thread_link("t-1", "c1.J", "x");
        // A redaction of a note this replica never saw names no item.
        s.redact_note("n-unseen", "x");
        let touched: Vec<String> = s.items_touched_since(w).unwrap().into_iter().collect();
        assert_eq!(
            touched,
            ["c1.A", "c1.B", "c1.C", "c1.D", "c1.F", "c1.G", "c1.H", "c1.I", "c1.J"]
        );
    }

    #[test]
    fn a_snapshot_survives_a_refold_and_no_image_carries_it() {
        // Review of #43, Test Quality #6: the guarantee, not only the classification.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.next_cache_put(&snap("kept", 100), 0, 8, 8).unwrap();
        TaskReducer.clear_views(s.connection());
        s.refold();
        assert!(
            s.next_cache_get("kept").unwrap().is_some(),
            "clear_views and refold leave it"
        );
        let image = s.image().unwrap();
        assert!(image.views.iter().all(|t| t.name != "next_page_cache"));
        let mut fresh = Store::open_in_memory(2);
        fresh.load_image(&image).unwrap();
        assert_eq!(
            rows(&fresh),
            0,
            "a replica started from the image holds none"
        );
    }
}

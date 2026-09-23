//! memory's fact reducer (spec §3): folds `fact`-domain ops into the `memories` view. The
//! second registered reducer in the platform (after flow's task reducer) — the substrate dispatches
//! `fact` ops here. Stateless — the view lives in the substrate's db, so a unit struct suffices
//! (mirrors flow's `TaskReducer`).
//!
//! The **body** is one keep-if-beats LWW register: `remember` sets body + active=1, `forget` clears
//! body→NULL + active=0; the winning `(lamport, site)` decides, and `author`/`updated` move with it
//! (they date the FACT, so classifying a memory deliberately leaves them alone).
//!
//! Since 6j6v.e0z6 a memory also carries **classification and order** — `category`, `scope`, `refs`
//! and `ordinal` — and since 6j6v.xbnh its written `introduction`, each its own independently
//! versioned keep-if-beats register on the same row. That independence is the point: writing a category must not roll a concurrent body edit back, and a
//! rare `reorder` must not touch anyone's text. Alongside them the fold maintains
//! `created_v`/`created_site` as the **minimum** `(lamport, site)` over a key's ops — insertion
//! order, and a minimum is commutative/idempotent, so it is a pure function of the log no matter
//! what order ops arrive or refold in.

use crate::model::{
    FACT_FIELD, FACT_KIND, FIELD_CATEGORY, FIELD_INTRODUCTION, FIELD_ORDINAL, FIELD_REFS,
    FIELD_SCOPE, OP_FORGET, OP_SET,
};
use nxs_foundation::model::Op;
use nxs_foundation::reducer::Reducer;
use rusqlite::{params, Connection};

/// memory's reducer over the `fact` domain.
pub struct FactReducer;

/// The `memories` column each non-body register folds into. Their `_v`/`_site` companions are
/// derived by name, so a register is named exactly once.
fn register_column(field: &str) -> Option<&'static str> {
    match field {
        FIELD_CATEGORY => Some("category"),
        FIELD_SCOPE => Some("scope"),
        FIELD_REFS => Some("refs"),
        FIELD_ORDINAL => Some("ordinal"),
        FIELD_INTRODUCTION => Some("introduction"),
        _ => None,
    }
}

/// Whether `value` is a payload the register `field` can actually hold.
///
/// Two rules, both about a value that arrives from a FOREIGN writer — a sync peer this build did
/// not author the op for:
///
/// * a value-less op has nothing to write — and for `introduction` (6j6v.xbnh) the NULL column
///   already means "nobody wrote one", so folding a value-less op into it would erase a written
///   introduction by way of an op that says nothing;
/// * `ordinal` is the one column with INTEGER affinity, and SQLite would **silently keep** a
///   non-numeric string in it. That is not a per-memory problem: `MemoryStore::memories` decodes
///   every row in one `collect`, so a single bad cell fails the WHOLE read with
///   `InvalidColumnType` — and `nxs prime` walks that exact path at the start of every session. One
///   malformed op from one peer would take the workspace's memory replay down with it.
///
/// Rejecting here rather than repairing on read keeps it inside the model the substrate already
/// has: the op is **store-don't-fold** (§7) — never dropped from the log, refoldable by a build
/// that understands it, and invisible to the view until then.
fn is_well_formed(field: &str, value: Option<&str>) -> bool {
    match value {
        None => false,
        Some(v) => field != FIELD_ORDINAL || v.parse::<i64>().is_ok(),
    }
}

impl Reducer for FactReducer {
    fn domain(&self) -> &'static str {
        crate::model::DOMAIN_FACT
    }

    /// True iff `op` is a `remember` (`set`) / `forget` over a fact's `body`, or a value-carrying
    /// `set` over one of the classification registers. A foreign shape — including a fact-domain op
    /// the current build does not understand — is store-don't-fold (§7): kept in the log, never
    /// folded, refoldable once a later version understands it. That is exactly what an older binary
    /// does with the classification ops this one writes.
    ///
    /// A classification `set` whose value the register cannot hold — absent, or non-numeric for
    /// `ordinal` — is deliberately NOT foldable either; see [`is_well_formed`] for why that one
    /// bad op would otherwise cost the whole workspace its memory replay.
    fn is_foldable(&self, op: &Op) -> bool {
        op.target_kind == FACT_KIND
            && match op.field.as_str() {
                FACT_FIELD => matches!(op.op_type.as_str(), OP_SET | OP_FORGET),
                other => {
                    register_column(other).is_some()
                        && op.op_type == OP_SET
                        && is_well_formed(other, op.value.as_deref())
                }
            }
    }

    /// Fold one foldable op into `memories`. Three statements, each commutative or keep-if-beats, so
    /// the result is independent of delivery order (spec §3.2):
    ///
    /// 1. **ensure the row** — every register of an as-yet-unseen key starts at its status-quo
    ///    DEFAULT, and a classification op that overtakes its own body op materializes a row that
    ///    stays invisible (`active=0`) until the body lands. `author`/`updated` are seeded empty
    ///    explicitly rather than left to the column default: a view MIGRATED up to v5 kept the
    ///    original nullable `author TEXT` with no default, and a NULL there would break every read.
    /// 2. **insertion order** — `created_*` folds as a MINIMUM, so the earliest op for the key wins
    ///    no matter when it arrives.
    /// 3. **the register this op writes** — keep-if-beats on that register's own `(v, site)`. For
    ///    the body that is the whole `body`/`author`/`updated`/`active` state moving together, so
    ///    `body` is a pure function of the winning op (spec §3.2, why `forget` NULLs it).
    fn fold(&self, conn: &Connection, op: &Op) {
        conn.execute(
            "INSERT OR IGNORE INTO memories(key, author, updated, created_v, created_site)
             VALUES(?1, '', '', ?2, ?3)",
            params![op.target_id, op.lamport, op.site],
        )
        .unwrap();
        conn.execute(
            "UPDATE memories SET created_v=?2, created_site=?3
             WHERE key=?1 AND (?2, ?3) < (created_v, created_site)",
            params![op.target_id, op.lamport, op.site],
        )
        .unwrap();

        match op.field.as_str() {
            FACT_FIELD => {
                let (body, active): (Option<&str>, i64) = match op.op_type.as_str() {
                    OP_SET => (op.value.as_deref(), 1),
                    OP_FORGET => (None, 0),
                    other => unreachable!("non-foldable op reached FactReducer::fold(): {other}"),
                };
                conn.execute(
                    "UPDATE memories SET body=?2, author=?3, updated=?4, active=?5, v=?6, site=?7
                     WHERE key=?1 AND (?6, ?7) > (v, site)",
                    params![
                        op.target_id,
                        body,
                        op.author,
                        op.wall_clock,
                        active,
                        op.lamport,
                        op.site
                    ],
                )
                .unwrap();
            }
            field => {
                let column = register_column(field)
                    .unwrap_or_else(|| unreachable!("non-foldable field reached fold(): {field}"));
                // The column names come from `register_column`, never from the op — the op only
                // chooses WHICH of the fixed registers is written.
                conn.execute(
                    &format!(
                        "UPDATE memories SET {column}=?2, {column}_v=?3, {column}_site=?4
                         WHERE key=?1 AND (?3, ?4) > ({column}_v, {column}_site)"
                    ),
                    params![op.target_id, op.value, op.lamport, op.site],
                )
                .unwrap();
            }
        }
    }

    fn view_tables(&self) -> &'static [&'static str] {
        &["memories"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        DOMAIN_FACT, FACT_FIELD, FACT_KIND, FIELD_CATEGORY, FIELD_INTRODUCTION, FIELD_ORDINAL,
        FIELD_REFS, FIELD_SCOPE, OP_FORGET, OP_SET,
    };
    use crate::schema;
    use nxs_foundation::model::Op;
    use nxs_foundation::reducer::Reducer;
    use nxs_foundation::store::Store;

    fn op(target_id: &str, op_type: &str, value: Option<&str>, lamport: i64, site: i64) -> Op {
        Op {
            op_id: format!("op-{target_id}-{lamport}-{site}"),
            lamport,
            site,
            domain: DOMAIN_FACT.into(),
            target_kind: FACT_KIND.into(),
            target_id: target_id.into(),
            field: FACT_FIELD.into(),
            op_type: op_type.into(),
            value: value.map(str::to_string),
            author: "alice".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    /// A substrate store with the memories view + fact reducer wired — a minimal memory store.
    fn store() -> Store {
        let mut s = Store::open_in_memory(1);
        schema::apply_memory_views(s.connection());
        s.register_reducer(Box::new(FactReducer));
        s
    }

    fn row(s: &Store, key: &str) -> Option<(Option<String>, i64)> {
        s.connection()
            .query_row(
                "SELECT body, active FROM memories WHERE key=?1",
                [key],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)),
            )
            .ok()
    }

    #[test]
    fn domain_is_fact() {
        assert_eq!(FactReducer.domain(), "fact");
    }

    #[test]
    fn is_foldable_accepts_remember_and_forget_only() {
        let r = FactReducer;
        assert!(r.is_foldable(&op("k", OP_SET, Some("v"), 1, 1)));
        assert!(r.is_foldable(&op("k", OP_FORGET, None, 1, 1)));
        // Foreign shapes within the fact domain are store-don't-fold (§7), never panic.
        let mut weird = op("k", "frobnicate", None, 1, 1);
        assert!(!r.is_foldable(&weird));
        weird = op("k", OP_SET, Some("v"), 1, 1);
        weird.field = "headline".into();
        assert!(!r.is_foldable(&weird), "only the body field folds");
        weird = op("k", OP_SET, Some("v"), 1, 1);
        weird.target_kind = "note".into();
        assert!(!r.is_foldable(&weird), "only the fact kind folds");
    }

    #[test]
    fn folds_a_remember_into_an_active_memory() {
        let mut s = store();
        s.apply(&[op("auth", OP_SET, Some("uses JWT"), 1, 1)]);
        assert_eq!(row(&s, "auth"), Some((Some("uses JWT".into()), 1)));
    }

    #[test]
    fn forget_nulls_the_body_and_deactivates() {
        let mut s = store();
        s.apply(&[op("auth", OP_SET, Some("uses JWT"), 1, 1)]);
        s.apply(&[op("auth", OP_FORGET, None, 2, 1)]);
        assert_eq!(row(&s, "auth"), Some((None, 0)), "body NULLed, active 0");
    }

    #[test]
    fn clear_views_empties_memories() {
        let mut s = store();
        s.apply(&[op("auth", OP_SET, Some("v"), 1, 1)]);
        FactReducer.clear_views(s.connection());
        assert_eq!(row(&s, "auth"), None);
    }

    // ---- classification + order registers (6j6v.e0z6) --------------------------------------

    /// A classification op over one of the four registers.
    fn class_op(target_id: &str, field: &str, value: &str, lamport: i64, site: i64) -> Op {
        let mut o = op(target_id, OP_SET, Some(value), lamport, site);
        o.field = field.into();
        o.op_id = format!("op-{target_id}-{field}-{lamport}-{site}");
        o
    }

    /// The classification columns of a row, plus the written introduction (6j6v.xbnh).
    #[allow(clippy::type_complexity)]
    fn classification(
        s: &Store,
        key: &str,
    ) -> Option<(String, String, String, Option<i64>, Option<String>)> {
        s.connection()
            .query_row(
                "SELECT category, scope, refs, ordinal, introduction FROM memories WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok()
    }

    #[test]
    fn is_foldable_accepts_the_classification_registers() {
        let r = FactReducer;
        // Each register with a value it can actually hold — `ordinal` needs an integer, see
        // `a_non_integer_ordinal_is_store_dont_fold`.
        for (field, value) in [
            (FIELD_CATEGORY, "rules"),
            (FIELD_SCOPE, "global"),
            (FIELD_REFS, "6j6v.e0z6"),
            (FIELD_ORDINAL, "3"),
            (FIELD_INTRODUCTION, "never rewrite a published tag"),
        ] {
            assert!(
                r.is_foldable(&class_op("k", field, value, 1, 1)),
                "{field} folds"
            );
        }
        // A value-less classification set would write NULL into a NOT NULL column — store-don't-fold
        // rather than a failed write.
        let mut valueless = class_op("k", FIELD_CATEGORY, "x", 1, 1);
        valueless.value = None;
        assert!(!r.is_foldable(&valueless));
        // A forget only ever means the BODY register.
        let mut forget_category = class_op("k", FIELD_CATEGORY, "x", 1, 1);
        forget_category.op_type = OP_FORGET.into();
        assert!(!r.is_foldable(&forget_category));
        // An unknown register from a newer peer stays store-don't-fold (§7).
        assert!(!r.is_foldable(&class_op("k", "sentiment", "warm", 1, 1)));
        // A value-less `introduction` set must not fold either: the column's NULL already means
        // "nobody wrote one", so folding a value-less op into it would erase a written line by way
        // of an op that says nothing (6j6v.xbnh).
        let mut valueless_intro = class_op("k", FIELD_INTRODUCTION, "x", 1, 1);
        valueless_intro.value = None;
        assert!(!r.is_foldable(&valueless_intro));
    }

    #[test]
    fn a_non_integer_ordinal_is_store_dont_fold() {
        // PR review, Integrity & Robustness #1. `ordinal` is the one INTEGER column: SQLite's type
        // affinity would keep a non-numeric string there without complaint, and the read then fails
        // for the WHOLE view, not just that row (`memories` decodes every row in one collect). Since
        // `nxs prime` walks that path at every session start, one malformed op from one peer would
        // cost the workspace its memory replay. So it never folds in the first place.
        let r = FactReducer;
        for bad in ["not-a-number", "", "1.5", "3; DROP TABLE memories", " 4"] {
            assert!(
                !r.is_foldable(&class_op("k", FIELD_ORDINAL, bad, 1, 1)),
                "ordinal value {bad:?} must not fold"
            );
        }
        for good in ["0", "1", "-2", "9007199254740993"] {
            assert!(
                r.is_foldable(&class_op("k", FIELD_ORDINAL, good, 1, 1)),
                "ordinal value {good:?} must fold"
            );
        }
        // The text registers keep taking any value — only `ordinal` has the numeric column.
        assert!(r.is_foldable(&class_op("k", FIELD_CATEGORY, "anything at all", 1, 1)));
    }

    #[test]
    fn a_peers_malformed_ordinal_leaves_every_other_memory_readable() {
        // The behaviour the guard buys, stated as the symptom it prevents: a foreign op with a
        // junk ordinal lands in the log (never dropped, §7) and the view stays fully readable.
        let mut s = store();
        s.apply(&[op("good", OP_SET, Some("a fine memory"), 1, 1)]);
        let deferred = s.apply(&[class_op("good", FIELD_ORDINAL, "not-a-number", 9, 2)]);
        assert_eq!(
            deferred.len(),
            1,
            "the junk ordinal is deferred, not folded"
        );
        assert_eq!(s.op_count(), 2, "but it IS kept in the log");

        let ordinal: Option<i64> = s
            .connection()
            .query_row("SELECT ordinal FROM memories WHERE key='good'", [], |r| {
                r.get(0)
            })
            .expect("the row still decodes");
        assert_eq!(ordinal, None, "the junk never reached the numeric column");
    }

    #[test]
    fn an_unclassified_memory_carries_the_status_quo_defaults() {
        // The migration's defaults are the fold's defaults: a memory written with no classification
        // is `unsorted`, reaches the project, references nothing and holds no explicit position.
        let mut s = store();
        s.apply(&[op("auth", OP_SET, Some("uses JWT"), 1, 1)]);
        assert_eq!(
            classification(&s, "auth"),
            Some((
                "unsorted".into(),
                "project".into(),
                String::new(),
                None,
                // NULL, not the empty string: "nobody has written one" has to stay distinguishable
                // from "written as empty", or the bootstrap cannot name the gap (6j6v.xbnh).
                None
            ))
        );
    }

    #[test]
    fn classifying_leaves_the_body_register_alone_and_vice_versa() {
        // The reason each register is versioned on its own: filing a memory must not roll a
        // concurrent body edit back, and a body edit must not un-file it.
        let mut s = store();
        s.apply(&[op("auth", OP_SET, Some("v1"), 1, 1)]);
        s.apply(&[class_op("auth", FIELD_CATEGORY, "rules", 2, 1)]);
        s.apply(&[op("auth", OP_SET, Some("v2"), 3, 1)]);
        assert_eq!(row(&s, "auth"), Some((Some("v2".into()), 1)));
        assert_eq!(
            classification(&s, "auth").unwrap().0,
            "rules",
            "the body edit kept the category"
        );
    }

    #[test]
    fn a_classification_op_ahead_of_its_body_stays_invisible_until_the_body_lands() {
        // Out-of-order delivery: the category arrives first. It materializes a row (so the register
        // is not lost) that is inactive — nothing reads it — until the body op catches up.
        let mut s = store();
        s.apply(&[class_op("auth", FIELD_CATEGORY, "rules", 5, 1)]);
        assert_eq!(row(&s, "auth"), Some((None, 0)), "present but inactive");
        s.apply(&[op("auth", OP_SET, Some("uses JWT"), 1, 1)]);
        assert_eq!(row(&s, "auth"), Some((Some("uses JWT".into()), 1)));
        assert_eq!(classification(&s, "auth").unwrap().0, "rules");
    }

    #[test]
    fn every_register_converges_under_any_delivery_order() {
        // The CRDT invariant, per register: two devices classify and edit the same memory
        // concurrently. Whatever order the ops are delivered in, both sides land on the same row.
        let ops = vec![
            op("k", OP_SET, Some("v1"), 1, 1),
            class_op("k", FIELD_CATEGORY, "rules", 2, 2),
            class_op("k", FIELD_CATEGORY, "architecture", 4, 1),
            class_op("k", FIELD_SCOPE, "global", 3, 2),
            class_op("k", FIELD_ORDINAL, "7", 5, 1),
            op("k", OP_SET, Some("v2"), 6, 2),
            // 6j6v.xbnh: the introduction is a register like the others and folds through the
            // identical keep-if-beats mechanism, so it belongs in the ONE test that proves the
            // fold is order-independent. Two writes, the LOWER coordinate delivered last, so a
            // fold that took "most recently seen" instead of "highest (lamport, site)" fails here.
            class_op("k", FIELD_INTRODUCTION, "the first line anyone wrote", 7, 1),
            class_op("k", FIELD_INTRODUCTION, "the line that wins", 8, 2),
        ];
        let reversed: Vec<Op> = ops.iter().rev().cloned().collect();

        let mut forward = store();
        forward.apply(&ops);
        let mut backward = store();
        backward.apply(&reversed);
        assert_eq!(row(&forward, "k"), row(&backward, "k"));
        assert_eq!(
            classification(&forward, "k"),
            classification(&backward, "k"),
            "diverged across delivery order"
        );
        assert_eq!(
            classification(&forward, "k"),
            Some((
                "architecture".into(),
                "global".into(),
                String::new(),
                Some(7),
                Some("the line that wins".into())
            )),
            "the highest (lamport, site) wins each register independently"
        );
    }

    #[test]
    fn insertion_order_is_the_earliest_op_whatever_order_it_arrives_in() {
        // `created_*` folds as a MINIMUM, which is commutative — so a late-arriving early op still
        // decides insertion order, and a refold recomputes exactly the same pair.
        let created = |s: &Store, key: &str| -> (i64, i64) {
            s.connection()
                .query_row(
                    "SELECT created_v, created_site FROM memories WHERE key=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        };
        let mut late_first = store();
        late_first.apply(&[op("k", OP_SET, Some("v2"), 9, 1)]);
        late_first.apply(&[op("k", OP_SET, Some("v1"), 2, 1)]);
        assert_eq!(created(&late_first, "k"), (2, 1));

        let mut early_first = store();
        early_first.apply(&[op("k", OP_SET, Some("v1"), 2, 1)]);
        early_first.apply(&[op("k", OP_SET, Some("v2"), 9, 1)]);
        assert_eq!(created(&early_first, "k"), (2, 1), "order-independent");
    }
}

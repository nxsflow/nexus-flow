//! The reducer seam (spec §4.2). The substrate owns one append-only op-log and dispatches each
//! op to the reducer registered for its `domain`. A reducer folds the ops of ONE domain into that
//! domain's materialized views — the clean, independently-testable boundary between the
//! domain-agnostic substrate and a product. flow contributes the first one (its task reducer);
//! memory/chat will each add their own (`fact`/`message`).
//!
//! The reducer is the **data axis**, orthogonal to a product's presentation `plugin` (§4.4): one
//! reducer per domain (how ops fold into views) vs. n plugins per product (how the views are
//! presented).

use crate::model::Op;
use rusqlite::Connection;

/// Folds the ops of one domain into that domain's materialized views. Registered with a
/// [`Store`](crate::store::Store), which dispatches each op to the reducer whose [`domain`] matches
/// `op.domain`.
///
/// `Send` so a `Store` carrying a registry stays `Send` (an embedding engine holds the store
/// behind a `Mutex`). Reducers are stateless folders — the views live in the substrate's db, not
/// in the reducer — so this is a trivial bound in practice.
///
/// [`domain`]: Reducer::domain
pub trait Reducer: Send {
    /// The op domain this reducer folds (e.g. `task`). One reducer per domain.
    fn domain(&self) -> &'static str;

    /// True iff `op` has a shape this reducer knows how to fold. An op a reducer cannot fold — or
    /// one whose domain has no registered reducer — is stored-not-folded: kept in the log, never
    /// dropped, refolded once a later version understands it (§7).
    fn is_foldable(&self, op: &Op) -> bool;

    /// Fold `op` into this reducer's views, on `conn`, inside the substrate's open transaction.
    /// Only ever called for ops where [`is_foldable`](Reducer::is_foldable) returned true.
    fn fold(&self, conn: &Connection, op: &Op);

    /// The tables this reducer folds into — its materialized views, and nothing it keeps beside
    /// them. **The one list** (6j6v.mxt2): [`clear_views`](Reducer::clear_views) empties exactly
    /// these, and a snapshot carries exactly these, so the two cannot drift apart — a table missing
    /// here would be a view a snapshot silently leaves behind while its watermark claims it folded.
    ///
    /// A table the product keeps for itself — a transcript, a lease, a queue — is NOT a view even
    /// when it lives in the same database: it is not a fold of the log, and another replica must
    /// never receive it. The default is the empty list, so a reducer written before this existed
    /// carries nothing in a snapshot and its product simply folds itself from the log it receives.
    fn view_tables(&self) -> &'static [&'static str] {
        &[]
    }

    /// Drop this reducer's materialized views. The op-log is untouched — the views are a pure fold
    /// of it — so the substrate's rebuild paths (prefix remap) call this before a full refold.
    ///
    /// The default empties every [`view_tables`](Reducer::view_tables) table, which is what every
    /// reducer in this repository relies on; an override is for a reducer that predates the list.
    fn clear_views(&self, conn: &Connection) {
        for table in self.view_tables() {
            conn.execute(&format!("DELETE FROM {table}"), []).unwrap();
        }
    }
}

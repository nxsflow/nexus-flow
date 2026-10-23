//! [`PurgeSeams`]: the board-fold purge's test seam, one opaque value (design §3, as f6yb's).
//!
//! A shipped build can make exactly one value, [`PurgeSeams::shipped`]: nothing stopped, nothing
//! injected, plan pages of 1000. The constructor that reads `NXF_FOLD_TEST_PURGE_*` exists only
//! under the `test-seams` feature, which only this package's own tests turn on, and
//! [`PurgeSeams::actions_at`] answers nothing at every point in a build without it — so the seams
//! cannot act in a shipped `nxf-fold-purge` even if a value were somehow made with them.
//!
//! The variables (test builds only):
//! - `NXF_FOLD_TEST_PURGE_STOP_AFTER=<point>`: `std::process::abort()` right after the point;
//! - `NXF_FOLD_TEST_PURGE_INJECT_AT=<point>`: fold one more op of the target through
//!   [`crate::fold::Folder`] — one that CREATES a new item, so every row it writes has a key the
//!   plan does not hold ([`super::injected_ops`]). `NXF_FOLD_TEST_PURGE_INJECT_ENDPOINT` sends that
//!   fold to another endpoint than the purge's own (past a recording proxy);
//! - `NXF_FOLD_TEST_PURGE_PAGE=<n>`: the plan's Query `Limit`.
//!
//! A point is `RowsBatch:<k>` (after the k-th F1 batch, 1-based), `Rows`, `Watermark` or `V0`
//! (design §4). A seam value `RowsBatch` without a number acts after EVERY F1 batch.

/// What the seam does at one point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamAction {
    /// Fold one more op of the target (test builds only).
    Inject,
    /// Abort the process (test builds only).
    Stop,
}

/// The shipped plan page: the plan's Query `Limit`.
pub const SHIPPED_PAGE: usize = 1000;

/// The purge's test seam. Opaque: its fields are private and a shipped build can only make
/// [`PurgeSeams::shipped`].
#[derive(Debug, Clone)]
pub struct PurgeSeams {
    stop: Option<String>,
    inject: Option<String>,
    page: usize,
}

impl PurgeSeams {
    /// Nothing stopped or injected, plan page 1000.
    pub fn shipped() -> PurgeSeams {
        PurgeSeams {
            stop: None,
            inject: None,
            page: SHIPPED_PAGE,
        }
    }

    /// The seams from the process environment. A malformed value is a bug in the test that set it,
    /// so it panics with the variable's name rather than running with a seam half-applied.
    #[cfg(feature = "test-seams")]
    pub fn from_env() -> PurgeSeams {
        PurgeSeams::from_vars(|name| std::env::var(name).ok())
    }

    /// [`PurgeSeams::from_env`] over an explicit lookup, so the parsing is testable without touching
    /// process-global state.
    #[cfg(feature = "test-seams")]
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> PurgeSeams {
        const STOP: &str = "NXF_FOLD_TEST_PURGE_STOP_AFTER";
        const INJECT: &str = "NXF_FOLD_TEST_PURGE_INJECT_AT";
        const PAGE: &str = "NXF_FOLD_TEST_PURGE_PAGE";
        let point = |name: &str| -> Option<String> {
            let value = var(name)?;
            if !is_point_pattern(&value) {
                panic!("test seam {name}={value:?} is malformed (see purge::seams)")
            }
            Some(value)
        };
        let mut seams = PurgeSeams::shipped();
        seams.stop = point(STOP);
        seams.inject = point(INJECT);
        if let Some(value) = var(PAGE) {
            seams.page = match value.parse::<usize>() {
                Ok(n) if n > 0 => n,
                _ => panic!("test seam {PAGE}={value:?} is malformed (see purge::seams)"),
            };
        }
        seams
    }

    /// The plan's page size (DynamoDB Query `Limit`).
    pub fn page(&self) -> usize {
        self.page
    }

    /// What the seams do at `point`, in the order they run: the injection, then the stop. Always
    /// empty in a build without `test-seams`.
    pub fn actions_at(&self, point: &str) -> Vec<SeamAction> {
        if !cfg!(feature = "test-seams") {
            return Vec::new();
        }
        let mut actions = Vec::new();
        if self.inject.as_deref().is_some_and(|p| matches(p, point)) {
            actions.push(SeamAction::Inject);
        }
        if self.stop.as_deref().is_some_and(|p| matches(p, point)) {
            actions.push(SeamAction::Stop);
        }
        actions
    }
}

/// Whether a seam value names a point: `RowsBatch`, `RowsBatch:<k>` with k ≥ 1, `Rows`,
/// `Watermark` or `V0`.
#[cfg(feature = "test-seams")]
fn is_point_pattern(value: &str) -> bool {
    match value.strip_prefix("RowsBatch:") {
        Some(k) => k.parse::<u64>().is_ok_and(|k| k > 0),
        None => matches!(value, "RowsBatch" | "Rows" | "Watermark" | "V0"),
    }
}

/// Whether the seam value `pattern` acts at `point`: equal, or `RowsBatch` at any `RowsBatch:<k>`.
fn matches(pattern: &str, point: &str) -> bool {
    pattern == point || (pattern == "RowsBatch" && point.starts_with("RowsBatch:"))
}

#[cfg(all(test, feature = "test-seams"))]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn seams(vars: &[(&str, &str)]) -> PurgeSeams {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        PurgeSeams::from_vars(|name| vars.get(name).cloned())
    }

    #[test]
    fn nothing_set_is_the_shipped_value() {
        let s = seams(&[]);
        assert_eq!(s.page(), 1000);
        for point in ["RowsBatch:1", "Rows", "Watermark", "V0"] {
            assert!(s.actions_at(point).is_empty(), "{point}");
        }
    }

    #[test]
    fn each_variable_acts_at_exactly_its_point() {
        let s = seams(&[
            ("NXF_FOLD_TEST_PURGE_STOP_AFTER", "RowsBatch:3"),
            ("NXF_FOLD_TEST_PURGE_INJECT_AT", "Watermark"),
            ("NXF_FOLD_TEST_PURGE_PAGE", "7"),
        ]);
        assert_eq!(s.actions_at("RowsBatch:3"), vec![SeamAction::Stop]);
        assert!(s.actions_at("RowsBatch:30").is_empty());
        assert!(s.actions_at("RowsBatch:2").is_empty());
        assert!(s.actions_at("Rows").is_empty());
        assert_eq!(s.actions_at("Watermark"), vec![SeamAction::Inject]);
        assert_eq!(s.page(), 7);
    }

    #[test]
    fn rows_batch_without_a_number_acts_after_every_batch() {
        let s = seams(&[("NXF_FOLD_TEST_PURGE_INJECT_AT", "RowsBatch")]);
        for point in ["RowsBatch:1", "RowsBatch:2", "RowsBatch:40"] {
            assert_eq!(s.actions_at(point), vec![SeamAction::Inject], "{point}");
        }
        for point in ["Rows", "Watermark", "V0"] {
            assert!(s.actions_at(point).is_empty(), "{point}");
        }
    }

    #[test]
    fn the_injection_runs_before_the_stop() {
        let s = seams(&[
            ("NXF_FOLD_TEST_PURGE_STOP_AFTER", "V0"),
            ("NXF_FOLD_TEST_PURGE_INJECT_AT", "V0"),
        ]);
        assert_eq!(
            s.actions_at("V0"),
            vec![SeamAction::Inject, SeamAction::Stop]
        );
    }

    #[test]
    fn a_malformed_value_panics_naming_the_variable() {
        for (name, value) in [
            ("NXF_FOLD_TEST_PURGE_STOP_AFTER", "D1"),
            ("NXF_FOLD_TEST_PURGE_STOP_AFTER", "RowsBatch:0"),
            ("NXF_FOLD_TEST_PURGE_INJECT_AT", "RowsBatch:x"),
            ("NXF_FOLD_TEST_PURGE_INJECT_AT", "v0"),
            ("NXF_FOLD_TEST_PURGE_PAGE", "0"),
            ("NXF_FOLD_TEST_PURGE_PAGE", "x"),
        ] {
            let caught = std::panic::catch_unwind(|| seams(&[(name, value)]));
            let payload = caught.expect_err(&format!("{name}={value} must panic"));
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            assert!(message.contains(name), "{message}");
        }
    }
}

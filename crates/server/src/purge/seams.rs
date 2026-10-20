//! [`PurgeSeams`]: the purge's test seam, one opaque value (design §2 A, §4).
//!
//! A shipped build can make exactly one value, [`PurgeSeams::shipped`]: nothing injected, plan
//! pages of 1000, a stream lease of 900 s. The constructor that reads `NXF_RELAY_TEST_PURGE_*`
//! exists only under the `test-seams` feature, which only this package's own tests turn on, and
//! [`PurgeSeams::action_at`] answers [`SeamAction::None`] for every point in a build without it —
//! so the seams cannot act in the released binary even if a value were somehow made with them.
//!
//! The variables (test builds only):
//! - `NXF_RELAY_TEST_PURGE_STOP_AFTER=<point>`: `std::process::abort()` right after the point;
//! - `NXF_RELAY_TEST_PURGE_INJECT_AT=<point>:<write>`: one write of the target through the store's
//!   own path (`op`, `machine`, `prefix`, `steal`);
//! - `NXF_RELAY_TEST_PURGE_FAIL_AT=<point>`: a `Store` error at the point (`renew` names the lease
//!   renewal);
//! - `NXF_RELAY_TEST_PURGE_PAUSE_AT=<point>:<secs>`: sleep that long right after the point;
//! - `NXF_RELAY_TEST_PURGE_PAGE=<n>`: the plan's Query `Limit` (DynamoDB);
//! - `NXF_RELAY_TEST_PURGE_LEASE_SECS=<s>`: the stream lease's lifetime.
//!
//! A point is a label from design §4's table: `S0`, `S1-tx`, `S1`, `S3`, `P0`, `P1-tx`, `L0`,
//! `D1-batch:<k>` … `D5`, `V0`, `L9` — a batch point carries its 1-based batch number.

use std::time::Duration;

/// The write an injection makes, each with a NEW key so no later planned batch deletes it anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectWrite {
    /// A new op through `append`.
    Op,
    /// A new machine through `announce`.
    Machine,
    /// A new prefix and replica uuid through `register`.
    Prefix,
    /// DynamoDB: overwrite the stream lease with another holder.
    Steal,
}

/// What the seam does at one point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamAction {
    None,
    Stop,
    Inject(InjectWrite),
    Fail,
    Pause(Duration),
}

/// The shipped plan page: the plan's Query `Limit` on DynamoDB.
pub const SHIPPED_PAGE: usize = 1000;
/// The shipped stream lease lifetime (design §7a).
pub const SHIPPED_LEASE_SECS: u64 = 900;

/// The purge's test seam. Opaque: its fields are private and a shipped build can only make
/// [`PurgeSeams::shipped`].
#[derive(Debug, Clone)]
pub struct PurgeSeams {
    stop: Option<String>,
    inject: Option<(String, InjectWrite)>,
    fail: Option<String>,
    pause: Option<(String, Duration)>,
    page: usize,
    lease_secs: u64,
}

impl PurgeSeams {
    /// Nothing injected, plan page 1000, lease 900 s.
    pub fn shipped() -> PurgeSeams {
        PurgeSeams {
            stop: None,
            inject: None,
            fail: None,
            pause: None,
            page: SHIPPED_PAGE,
            lease_secs: SHIPPED_LEASE_SECS,
        }
    }

    /// The seams from the process environment. A malformed value is a bug in the test that set
    /// it, so it panics with the variable's name rather than running with a seam half-applied.
    #[cfg(feature = "test-seams")]
    pub fn from_env() -> PurgeSeams {
        PurgeSeams::from_vars(|name| std::env::var(name).ok())
    }

    /// [`PurgeSeams::from_env`] over an explicit lookup, so the parsing is testable without
    /// touching process-global state.
    #[cfg(feature = "test-seams")]
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> PurgeSeams {
        const STOP: &str = "NXF_RELAY_TEST_PURGE_STOP_AFTER";
        const INJECT: &str = "NXF_RELAY_TEST_PURGE_INJECT_AT";
        const FAIL: &str = "NXF_RELAY_TEST_PURGE_FAIL_AT";
        const PAUSE: &str = "NXF_RELAY_TEST_PURGE_PAUSE_AT";
        const PAGE: &str = "NXF_RELAY_TEST_PURGE_PAGE";
        const LEASE: &str = "NXF_RELAY_TEST_PURGE_LEASE_SECS";
        let malformed = |name: &str, value: &str| -> ! {
            panic!("test seam {name}={value:?} is malformed (see purge::seams)")
        };
        // `<point>:<last>`: the point may itself carry a `:<k>`, so split at the LAST colon.
        let split_last = |name: &str, value: &str| -> (String, String) {
            match value.rsplit_once(':') {
                Some((point, last)) if !point.is_empty() => (point.to_string(), last.to_string()),
                _ => malformed(name, value),
            }
        };
        let mut seams = PurgeSeams::shipped();
        seams.stop = var(STOP);
        seams.fail = var(FAIL);
        if let Some(value) = var(INJECT) {
            let (point, write) = split_last(INJECT, &value);
            let write = match write.as_str() {
                "op" => InjectWrite::Op,
                "machine" => InjectWrite::Machine,
                "prefix" => InjectWrite::Prefix,
                "steal" => InjectWrite::Steal,
                _ => malformed(INJECT, &value),
            };
            seams.inject = Some((point, write));
        }
        if let Some(value) = var(PAUSE) {
            let (point, secs) = split_last(PAUSE, &value);
            let secs: u64 = secs.parse().unwrap_or_else(|_| malformed(PAUSE, &value));
            seams.pause = Some((point, Duration::from_secs(secs)));
        }
        if let Some(value) = var(PAGE) {
            seams.page = match value.parse::<usize>() {
                Ok(n) if n > 0 => n,
                _ => malformed(PAGE, &value),
            };
        }
        if let Some(value) = var(LEASE) {
            seams.lease_secs = match value.parse::<u64>() {
                Ok(n) if n > 0 => n,
                _ => malformed(LEASE, &value),
            };
        }
        seams
    }

    /// The shipped value with another lease lifetime (unit tests of the lease rule).
    #[cfg(test)]
    pub(crate) fn with_lease_secs(secs: u64) -> PurgeSeams {
        PurgeSeams {
            lease_secs: secs,
            ..PurgeSeams::shipped()
        }
    }

    /// The plan's page size (DynamoDB Query `Limit`).
    pub fn page(&self) -> usize {
        self.page
    }

    /// The stream lease's lifetime.
    pub fn lease_ttl(&self) -> Duration {
        Duration::from_secs(self.lease_secs)
    }

    /// The renewal margin of design §7a: `min(60 s, ttl / 4)`.
    pub fn lease_margin(&self) -> Duration {
        Duration::from_secs(60).min(self.lease_ttl() / 4)
    }

    /// What the seam does at `point`. Always [`SeamAction::None`] in a build without `test-seams`.
    pub fn action_at(&self, point: &str) -> SeamAction {
        if !cfg!(feature = "test-seams") {
            return SeamAction::None;
        }
        if self.stop.as_deref() == Some(point) {
            SeamAction::Stop
        } else if let Some((_, write)) = self.inject.as_ref().filter(|(p, _)| p == point) {
            SeamAction::Inject(*write)
        } else if self.fail.as_deref() == Some(point) {
            SeamAction::Fail
        } else if let Some((_, d)) = self.pause.as_ref().filter(|(p, _)| p == point) {
            SeamAction::Pause(*d)
        } else {
            SeamAction::None
        }
    }
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
        assert_eq!(s.lease_ttl(), Duration::from_secs(900));
        assert_eq!(s.lease_margin(), Duration::from_secs(60));
        assert_eq!(s.action_at("D1-batch:1"), SeamAction::None);
    }

    #[test]
    fn each_variable_acts_at_exactly_its_point() {
        let s = seams(&[
            ("NXF_RELAY_TEST_PURGE_STOP_AFTER", "D1-batch:3"),
            ("NXF_RELAY_TEST_PURGE_INJECT_AT", "V0:machine"),
            ("NXF_RELAY_TEST_PURGE_FAIL_AT", "renew"),
            ("NXF_RELAY_TEST_PURGE_PAUSE_AT", "D1-batch:2:10"),
            ("NXF_RELAY_TEST_PURGE_PAGE", "7"),
            ("NXF_RELAY_TEST_PURGE_LEASE_SECS", "8"),
        ]);
        assert_eq!(s.action_at("D1-batch:3"), SeamAction::Stop);
        assert_eq!(s.action_at("D1-batch:30"), SeamAction::None);
        assert_eq!(s.action_at("D1"), SeamAction::None);
        assert_eq!(s.action_at("V0"), SeamAction::Inject(InjectWrite::Machine));
        assert_eq!(s.action_at("renew"), SeamAction::Fail);
        assert_eq!(
            s.action_at("D1-batch:2"),
            SeamAction::Pause(Duration::from_secs(10))
        );
        assert_eq!(s.page(), 7);
        assert_eq!(s.lease_ttl(), Duration::from_secs(8));
        assert_eq!(s.lease_margin(), Duration::from_secs(2));
    }

    #[test]
    fn every_write_kind_parses() {
        for (text, write) in [
            ("op", InjectWrite::Op),
            ("machine", InjectWrite::Machine),
            ("prefix", InjectWrite::Prefix),
            ("steal", InjectWrite::Steal),
        ] {
            let s = seams(&[("NXF_RELAY_TEST_PURGE_INJECT_AT", &format!("S0:{text}"))]);
            assert_eq!(s.action_at("S0"), SeamAction::Inject(write));
        }
    }

    #[test]
    fn a_five_second_lease_has_a_margin_of_a_quarter() {
        let s = seams(&[("NXF_RELAY_TEST_PURGE_LEASE_SECS", "5")]);
        assert_eq!(s.lease_margin(), Duration::from_millis(1250));
    }

    #[test]
    fn a_malformed_value_panics_naming_the_variable() {
        for (name, value) in [
            ("NXF_RELAY_TEST_PURGE_INJECT_AT", "S0:everything"),
            ("NXF_RELAY_TEST_PURGE_INJECT_AT", "S0"),
            ("NXF_RELAY_TEST_PURGE_PAUSE_AT", "D1:soon"),
            ("NXF_RELAY_TEST_PURGE_PAGE", "0"),
            ("NXF_RELAY_TEST_PURGE_LEASE_SECS", "x"),
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

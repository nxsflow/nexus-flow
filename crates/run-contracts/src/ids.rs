//! The small value types every contract shares: ids, sequence numbers and time.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::format_description::well_known::Rfc3339;
use time::format_description::BorrowedFormatItem;
use time::OffsetDateTime;
use ulid::Ulid;

/// A chat message id (`m-<ULID>`). The log carries these, never message bodies.
pub type MessageId = String;
/// A persona's bare handle.
pub type Handle = String;
/// A step id of a flow definition.
pub type StepId = String;
/// `<run>/<step>/v<visit>/<member>/a<attempt>`, deterministic, so a start is idempotent by key.
pub type SessionKey = String;
/// A named value in the store's compare-and-set space, or an inbox key.
pub type Key = String;
/// The per-run fencing token of the owner lease. It only ever rises.
pub type Epoch = u64;
/// A position in a run's log: 1-based and gap-free per run.
pub type Seq = u64;

/// A run id: a ULID with the prefix `r-`. Minting one needs a clock, so the host does that and hands
/// the ULID in; this crate only checks the form.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

impl RunId {
    pub fn new(ulid: Ulid) -> Self {
        Self(format!("r-{ulid}"))
    }

    /// `None` unless the text is `r-` followed by a valid ULID.
    pub fn parse(text: &str) -> Option<Self> {
        let ulid = text.strip_prefix("r-")?;
        Ulid::from_string(ulid).ok()?;
        Some(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for RunId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RunId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("not a run id: {text}")))
    }
}

/// One member slot of one step visit. It names a session without carrying the run, so the same value
/// reads the same in the run's own log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub step: StepId,
    pub visit: u32,
    pub member: Handle,
    pub attempt: u32,
}

impl Slot {
    /// The session key of this slot in `run`.
    pub fn session_key(&self, run: &RunId) -> SessionKey {
        format!(
            "{run}/{}/v{}/{}/a{}",
            self.step, self.visit, self.member, self.attempt
        )
    }
}

/// A span of whole seconds, on the wire as a plain integer. The policy pins these on a run, so a
/// replay folds with the values the run was started under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secs(pub u64);

impl Secs {
    pub const fn minutes(n: u64) -> Self {
        Self(n * 60)
    }

    pub const fn hours(n: u64) -> Self {
        Self(n * 3600)
    }
}

/// An instant, UTC, at millisecond precision, written with EXACTLY three fractional digits so golden
/// bytes cannot flap. Precision finer than a millisecond is truncated on the way in, so a value equals
/// its own round trip. The host writes these from its clock; nothing here reads one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_unix_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub const fn unix_millis(self) -> i64 {
        self.0
    }

    /// `None` unless the text is RFC 3339. Any offset and precision is read; UTC milliseconds are kept.
    pub fn parse(text: &str) -> Option<Self> {
        let at = OffsetDateTime::parse(text, &Rfc3339).ok()?;
        let millis = at.unix_timestamp_nanos().div_euclid(1_000_000);
        Some(Self(i64::try_from(millis).ok()?))
    }

    /// This instant moved forward by `span`, stopping at the last instant the wire form can write. A
    /// `Secs` read off the wire can be any `u64`, and a later instant could not be written back.
    pub const fn plus(self, span: Secs) -> Self {
        let millis = span.0.saturating_mul(1000);
        let millis = if millis > LAST_WRITABLE_MILLIS as u64 {
            LAST_WRITABLE_MILLIS
        } else {
            millis as i64
        };
        let moved = self.0.saturating_add(millis);
        Self(if moved > LAST_WRITABLE_MILLIS {
            LAST_WRITABLE_MILLIS
        } else {
            moved
        })
    }
}

/// `9999-12-31T23:59:59.999Z`: RFC 3339 has four digits for the year.
const LAST_WRITABLE_MILLIS: i64 = 253_402_300_799_999;

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.0) * 1_000_000)
            .map_err(|_| fmt::Error)?;
        let text = at.format(&millis_utc()).map_err(|_| fmt::Error)?;
        f.write_str(&text)
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ`. The value is always UTC, so the literal `Z` is true.
fn millis_utc() -> Vec<BorrowedFormatItem<'static>> {
    time::format_description::parse(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z",
    )
    .expect("the format description is a valid literal")
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("not an RFC 3339 instant: {text}")))
    }
}

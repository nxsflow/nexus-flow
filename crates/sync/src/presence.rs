//! Machine presence (nxf 6j6v.f0b5): which machines sync a stream, and which of them are online
//! right now.
//!
//! A machine's background service announces itself to the relay after every pass that synced
//! ([`MachineHello`], `POST /streams/{id}/machines`). The relay keeps, per stream and machine, the
//! name, when it last heard from it (on its own clock) and the cadence the service promised, and
//! hands that list back on `GET` ([`MachinesResponse`]). It judges nothing. The verdict is made
//! HERE, once, for every reader — `nxs sync machines`, an embedding host, a hosted relay reader —
//! so the rule cannot come out differently in two places.
//!
//! # What "online" means
//!
//! ```text
//! online  <=>  age <= 2 x cadence + ONLINE_SLACK_SECS
//! ```
//!
//! `cadence` is what the machine's service announced: its safety-net interval (300 s by default;
//! wake and local writes only make passes MORE frequent), but **never shorter than its retry
//! backoff** (300 s) and never longer than [`MAX_INTERVAL_SECS`] (`crates/nxs`'
//! `announced_cadence`). The window is built from three facts about that service, not guessed:
//!
//! - **One cadence** is the normal gap between two announcements of a healthy machine.
//! - **A second cadence** absorbs one failed pass. A pass that fails (a relay hiccup, a network
//!   change) announces nothing, and the service retries only after its backoff — a full 300 s,
//!   whatever the interval. So after one failure the next announcement comes `interval + 300 s`
//!   after the last, which `2 x max(interval, 300 s)` always covers. (The interval alone did not,
//!   below about 240 s — review of PR #485; a synthetic-clock test on the daemon pins it now.)
//! - **[`ONLINE_SLACK_SECS`]** absorbs the sweep itself. The service passes its workspaces one after
//!   another, each bounded by a 60-second wall clock (`PASS_WALL_CLOCK_BUDGET` in `crates/nxs`), so a
//!   workspace late in the sweep is announced up to that much later than the cadence alone says.
//!
//! That is **11 minutes** for every service whose interval is at most 300 s, the default among
//! them. The trade it makes, stated so nobody has to reverse-engineer it: a machine that stops (shut
//! down, asleep, service stopped) still reads as online for up to 11 minutes. A work order handed
//! to it in that window waits in the log until it is back — nothing is lost, it is late. The
//! opposite error, a live machine shown as absent, would make a person pick another machine for no
//! reason, so the window errs toward "online".
//!
//! The cadence travels with every announcement because the reader cannot know it otherwise:
//! `nxs sync daemon --interval` changes it, and a machine judged against somebody else's cadence
//! would flap.
//!
//! # What the relay sees, and what it proves
//!
//! Nothing here is authenticated (the relay authenticates no client at all, 6j6v.6aza). A sighting
//! is therefore a CLAIM: anybody who can reach the relay and knows a stream id can announce a
//! machine under any id and name, or re-announce a stopped machine's id to keep it looking alive —
//! with the longest cadence a relay stores, for **about two hours** (`2 x 3600 s + 60 s`), not the
//! 11 minutes an honest machine gets. That is acceptable for what presence is used for — showing a
//! person which machines exist and which are awake — and it is exactly why presence must never be
//! what AUTHORIZES a machine to act. [`check_hello`] bounds what one announcement can make the
//! relay store, [`RETENTION_SECS`] how long it keeps a machine nobody hears from, and
//! [`MAX_LISTED_MACHINES`] how much one answer carries ([`judge_all`] flags a list that is not
//! whole); none of it makes a claim true, and none of it stops a flood inside the window — only
//! relay authentication can.

use serde::{Deserialize, Serialize};

pub use crate::protocol::{MachineHello, MachineSeen, MachinesResponse};

/// The allowance for the sweep that carries an announcement — one full per-pass wall-clock budget.
/// See the module doc for the whole window.
pub const ONLINE_SLACK_SECS: u64 = 60;

/// The longest machine name the relay stores, in characters. A person reads it in a list on a
/// phone; anything longer is not a name. [`check_hello`] enforces it at the relay, and the service
/// refuses a longer rename before it ever reaches one.
pub const MAX_MACHINE_NAME_CHARS: usize = 64;

/// The longest machine id the relay stores, in bytes. A minted id is a 26-character ULID; the room
/// above that is for whatever E4 anchors an identity on later, not for arbitrary input.
pub const MAX_MACHINE_ID_BYTES: usize = 64;

/// The longest cadence a machine may announce: one hour. It is also the bound on the worst case the
/// relay's lack of authentication allows — anybody can re-announce a stopped machine's id with the
/// longest cadence and keep it reading "online" for `2 x 3600 s + 60 s`, about two hours (the
/// module doc). A day, the first choice, stretched that to two days for no real service: the
/// daemon's default is 300 s, and a longer one is announced as this.
pub const MAX_INTERVAL_SECS: u64 = 3_600;

/// How many machines one `GET` returns at most, most recently seen first. A bound on the answer of
/// an unauthenticated route, not a product limit — no real stream has a hundred machines. When the
/// relay knows more, the answer says so ([`MachinesResponse::truncated`]).
pub const MAX_LISTED_MACHINES: usize = 100;

/// How long the relay keeps a machine nobody has heard from: 30 days. After that it is forgotten
/// — no longer listed, and pruned where the backend can — and it reappears the moment its service
/// announces again. Without it every reinstall, every fresh service home and every throwaway CI
/// runner would leave a permanent "offline" ghost in the list a person chooses from.
pub const RETENTION_SECS: i64 = 30 * 86_400;

/// The window a machine announcing `interval_secs` is judged by. Saturating, so a hostile interval
/// cannot wrap it around to zero.
pub fn online_window_secs(interval_secs: u64) -> u64 {
    interval_secs
        .saturating_mul(2)
        .saturating_add(ONLINE_SLACK_SECS)
}

/// One machine, judged: everything the relay said about it, plus the window it was judged by and
/// the verdict. This is the shape `nxs sync machines --json` prints and an embedding host renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineStatus {
    pub machine_id: String,
    pub name: String,
    /// Unix seconds, on the relay's clock.
    pub last_seen: i64,
    /// Seconds since `last_seen`, measured by the relay when it answered.
    pub age_secs: u64,
    /// The cadence the machine's service announced.
    pub interval_secs: u64,
    /// The window `online` was judged by — [`online_window_secs`] of `interval_secs`.
    pub online_within_secs: u64,
    pub online: bool,
}

/// Judge one sighting. The whole rule is here; see the module doc for why it is this rule.
pub fn judge(seen: &MachineSeen) -> MachineStatus {
    let window = online_window_secs(seen.interval_secs);
    MachineStatus {
        machine_id: seen.machine_id.clone(),
        name: seen.name.clone(),
        last_seen: seen.last_seen,
        age_secs: seen.age_secs,
        interval_secs: seen.interval_secs,
        online_within_secs: window,
        online: seen.age_secs <= window,
    }
}

/// Judge a relay's whole answer: every sighting through [`judge`], after dropping what no honest
/// client can have sent. The relay authenticates nobody and a reader must not trust its list more
/// than the relay trusted the announcements: an entry whose id or name breaks the rules a relay of
/// this version enforces on the way in ([`check_hello`]) is left out, and so is anything past
/// [`MAX_LISTED_MACHINES`] — either makes the reading `truncated`, because it is no longer the
/// whole list the relay holds.
pub fn judge_all(resp: &MachinesResponse) -> Presence {
    let mut truncated = resp.truncated;
    let mut machines = Vec::new();
    for seen in &resp.machines {
        if check_id(&seen.machine_id).is_err() || check_machine_name(&seen.name).is_err() {
            truncated = true;
            continue;
        }
        if machines.len() == MAX_LISTED_MACHINES {
            truncated = true;
            break;
        }
        machines.push(judge(seen));
    }
    Presence::Reported {
        machines,
        truncated,
    }
}

/// What a relay said about a stream's machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// The relay records presence; these are the machines it knows for the stream, judged, most
    /// recently seen first. Empty is a real answer: nobody's service has announced itself yet.
    /// `truncated` means the list is not all the relay holds (see [`judge_all`]) — say so rather
    /// than present it as complete.
    Reported {
        machines: Vec<MachineStatus>,
        truncated: bool,
    },
    /// The relay has no presence route — it predates 6j6v.f0b5. Not an error: syncing is unaffected,
    /// there is simply no list to show until the relay is upgraded.
    Unsupported,
}

/// Whether the relay should store `hello` at all. The relay authenticates nobody, so what it
/// refuses here is what bounds how much any caller can make it keep — not whether the claim is true.
pub fn check_hello(hello: &MachineHello) -> Result<(), String> {
    check_id(&hello.machine_id)?;
    check_machine_name(&hello.name)?;
    if hello.interval_secs == 0 || hello.interval_secs > MAX_INTERVAL_SECS {
        return Err(format!(
            "interval_secs must be between 1 and {MAX_INTERVAL_SECS}"
        ));
    }
    Ok(())
}

/// The rule for a machine's id: 1 to [`MAX_MACHINE_ID_BYTES`] printable ASCII characters, no space.
pub fn check_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_MACHINE_ID_BYTES || !id.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(format!(
            "machine_id must be 1 to {MAX_MACHINE_ID_BYTES} printable ASCII characters without spaces"
        ));
    }
    Ok(())
}

/// The rule for a machine's name, on its own so the machine's owner hears it at rename time rather
/// than as a refusal from a relay later.
///
/// A person picks a machine by READING its name, so the name must render as what it is: 1 to
/// [`MAX_MACHINE_NAME_CHARS`] characters, at least one of them visible, no control character, no
/// whitespace but the plain space, and none of the characters that render as nothing or reorder
/// the display — the set the house rule for authors already names (`nxs_foundation::model::
/// is_attributable`, PR #314 review): zero-width characters, bidi marks, embeddings, overrides and
/// isolates, the BOM, the soft hyphen. `nxs-service` holds a rename to the same rule
/// (`nxs_service::machine::check_name`); `crates/nxs` pins the two together.
pub fn check_machine_name(name: &str) -> Result<(), String> {
    let renders_as_itself =
        |c: char| !c.is_control() && !is_format_invisible(c) && (c == ' ' || !c.is_whitespace());
    if name.trim().is_empty()
        || name.chars().count() > MAX_MACHINE_NAME_CHARS
        || !name.chars().all(renders_as_itself)
    {
        return Err(format!(
            "a machine name must be 1 to {MAX_MACHINE_NAME_CHARS} characters, not blank, with no \
             control, invisible or direction-changing characters and no whitespace but spaces"
        ));
    }
    Ok(())
}

/// The format characters that occupy no width or reorder text — the set of
/// `nxs_foundation::model::is_invisible` beyond whitespace and controls. Mirrored rather than
/// imported: this crate's always-on half is what the relay links, and it depends on nothing but
/// serde.
fn is_format_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}'                 // soft hyphen
        | '\u{061C}'               // arabic letter mark
        | '\u{180E}'               // mongolian vowel separator
        | '\u{200B}'..='\u{200F}'  // zero-width space/non-joiner/joiner, LRM, RLM
        | '\u{202A}'..='\u{202E}'  // bidi embedding + override
        | '\u{2060}'..='\u{2064}'  // word joiner, invisible operators
        | '\u{2066}'..='\u{2069}'  // bidi isolates
        | '\u{FEFF}'               // zero-width no-break space (BOM)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MachineSeen;

    fn seen(age_secs: u64, interval_secs: u64) -> MachineSeen {
        MachineSeen {
            machine_id: "m1".into(),
            name: "Mac mini".into(),
            last_seen: 1_790_000_000,
            age_secs,
            interval_secs,
        }
    }

    #[test]
    fn the_production_cadence_gives_an_eleven_minute_window() {
        // 2 x the 300 s safety net + one 60 s pass budget. The number the guide names.
        assert_eq!(online_window_secs(300), 660);
    }

    #[test]
    fn a_machine_is_online_up_to_and_including_its_window_and_not_one_second_after() {
        assert!(judge(&seen(0, 300)).online);
        assert!(judge(&seen(660, 300)).online, "the window is inclusive");
        assert!(!judge(&seen(661, 300)).online);
    }

    #[test]
    fn one_missed_pass_does_not_take_a_live_machine_offline() {
        // The pass after the last recorded one failed (a transient relay error), so the next
        // announcement lands two intervals after the last — plus the sweep that carried it.
        assert!(judge(&seen(2 * 300 + 30, 300)).online);
    }

    #[test]
    fn the_window_follows_the_cadence_the_machine_announced() {
        // A service run with `--interval 20` promises to be back within 20 s, so it is judged
        // against its own promise and not against the production default.
        assert_eq!(online_window_secs(20), 100);
        assert!(!judge(&seen(101, 20)).online);
        assert!(judge(&seen(101, 300)).online);
    }

    #[test]
    fn an_absurd_interval_cannot_overflow_the_window() {
        assert_eq!(online_window_secs(u64::MAX), u64::MAX);
    }

    #[test]
    fn the_verdict_carries_everything_the_sighting_said_and_the_window_it_was_judged_by() {
        let status = judge(&seen(12, 300));
        assert_eq!(
            status,
            MachineStatus {
                machine_id: "m1".into(),
                name: "Mac mini".into(),
                last_seen: 1_790_000_000,
                age_secs: 12,
                interval_secs: 300,
                online_within_secs: 660,
                online: true,
            }
        );
    }

    #[test]
    fn a_well_formed_hello_passes() {
        let hello = MachineHello {
            machine_id: "01j8machine000000000000000".into(),
            name: "Carsten’s Mac mini (dev)".into(),
            interval_secs: 300,
        };
        assert_eq!(check_hello(&hello), Ok(()));
    }

    #[test]
    fn a_hello_the_relay_would_have_to_store_unbounded_is_refused() {
        let ok = MachineHello {
            machine_id: "m1".into(),
            name: "Mac mini".into(),
            interval_secs: 300,
        };
        let refused = |f: &dyn Fn(&mut MachineHello)| {
            let mut h = ok.clone();
            f(&mut h);
            check_hello(&h).expect_err("must be refused")
        };
        assert!(refused(&|h| h.machine_id.clear()).contains("machine_id"));
        assert!(refused(&|h| h.machine_id = "x".repeat(65)).contains("machine_id"));
        assert!(refused(&|h| h.machine_id = "has space".into()).contains("machine_id"));
        assert!(refused(&|h| h.name = "   ".into()).contains("name"));
        assert!(refused(&|h| h.name = "a\u{7}bell".into()).contains("name"));
        assert!(refused(&|h| h.name = "é".repeat(65)).contains("name"));
        assert!(refused(&|h| h.interval_secs = 0).contains("interval"));
        assert!(refused(&|h| h.interval_secs = MAX_INTERVAL_SECS + 1).contains("interval"));
        // The bound is in CHARACTERS, not bytes: 64 two-byte characters are a legal name.
        let mut wide = ok.clone();
        wide.name = "é".repeat(64);
        assert_eq!(check_hello(&wide), Ok(()));
    }

    #[test]
    fn every_edge_a_hello_is_allowed_to_touch_is_accepted() {
        let edge = |machine_id: String, interval_secs| MachineHello {
            machine_id,
            name: "Mac mini".into(),
            interval_secs,
        };
        assert_eq!(
            check_hello(&edge("x".repeat(MAX_MACHINE_ID_BYTES), 300)),
            Ok(())
        );
        assert_eq!(check_hello(&edge("m1".into(), 1)), Ok(()));
        assert_eq!(check_hello(&edge("m1".into(), MAX_INTERVAL_SECS)), Ok(()));
    }

    #[test]
    fn the_longest_cadence_the_relay_stores_is_an_hour() {
        // It bounds how long anybody — including somebody who is not the machine — can keep a
        // stopped machine reading "online": 2 x 3600 s + 60 s, about two hours.
        assert_eq!(MAX_INTERVAL_SECS, 3_600);
        assert_eq!(online_window_secs(MAX_INTERVAL_SECS), 7_260);
    }

    #[test]
    fn a_name_that_renders_as_something_else_than_it_is_refused() {
        // The house rule (`nxs_foundation::model::is_attributable`, PR #314 review) plus what a
        // chooser needs on top: no character that is invisible or reorders the display anywhere in
        // the name, and no whitespace but the plain space — a person picks a machine by reading it.
        for bad in [
            "\u{200B}",            // zero-width space alone
            "Mac\u{200B}mini",     // … or hidden inside
            "Mac mini\u{202E}",    // right-to-left override
            "\u{2066}Mac\u{2069}", // bidi isolate
            "\u{FEFF}Mac",         // BOM
            "Mac\u{00AD}mini",     // soft hyphen
            "Mac\u{2028}mini",     // line separator
            "Mac\u{00A0}mini",     // no-break space
            "Mac\tmini",           // tab (a control)
        ] {
            assert!(check_machine_name(bad).is_err(), "{bad:?} must be refused");
        }
        for good in ["Mac mini", "Carsten’s MacBook (dev)", "studio", "工作站"] {
            assert_eq!(check_machine_name(good), Ok(()), "{good:?}");
        }
    }
}

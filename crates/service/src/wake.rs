//! **Keeping the machine awake while a run is working** (nxf 6j6v.7q3r).
//!
//! A run that is meant to work unattended for hours does not survive a sleep. Nothing held the
//! machine awake for it, and nothing in the engine could: an idle-sleep assertion must be held by a
//! LIVE PROCESS and dies with it, while `nxc` as a CLI is over the moment it has sent, and an
//! embedding app is gone exactly when the assertion is needed — when the user closes it. The
//! background service is the only thing that lives long enough, which is why this is here and not
//! in the engine.
//!
//! # The dangerous direction is the second one
//!
//! An assertion that is TAKEN and not given back means the machine never sleeps again. Nobody
//! notices for days, and when they do they diagnose a battery or a fan, not us. So releasing is the
//! property this module is built around, and it is guaranteed three deep:
//!
//! 1. [`WakeKeeper::want`]`(false)` — the ordinary path, the moment the last run ends.
//! 2. [`Drop`] on the keeper — every early return out of the service loop, including `?`.
//! 3. **The operating system, when the process dies without running either.** An `IOPMAssertion` is
//!    owned by the process that took it and the kernel reclaims it on exit — a `SIGKILL`, a panic,
//!    a power loss. That is the backstop no code can provide for itself, and
//!    `tests/wake_release.rs` proves it against a real `pmset` rather than assuming it.
//!
//! Taking one twice, by contrast, costs nothing but a duplicate: [`WakeKeeper`] holds at most one
//! anyway, so a repeated `want(true)` is a no-op.
//!
//! # What it asserts, and what it deliberately does not
//!
//! `kIOPMAssertionTypePreventUserIdleSystemSleep`: the machine does not fall asleep for being IDLE.
//! It does NOT keep the display awake, it does NOT override a closed lid, and it does not stop a
//! person choosing Sleep from the menu. A run is a background job; keeping somebody's screen lit
//! for it would be taking more than was asked for.

use nxs_foundation::error::{NxfError, Result};

/// The seam every assertion goes through — one so the whole decision (when to take, when to give
/// back, what a failure costs) is provable without touching the machine's real power management,
/// and a test can be told to fail an acquisition.
pub trait Wake {
    /// Take an assertion that keeps the machine awake, returning the handle that releases it.
    ///
    /// `reason` is shown to the user by `pmset -g assertions`, so it says who is holding it and
    /// why — a person looking at a Mac that will not sleep is entitled to find us there by name.
    fn acquire(&self, reason: &str) -> Result<u32>;

    /// Give one back. Infallible BY CONTRACT rather than by luck: there is nothing a caller could
    /// do about a failed release, and returning a `Result` here would invite somebody to `?` it out
    /// of a `Drop` — where the error cannot propagate and the assertion would be leaked by the very
    /// code written to prevent that.
    fn release(&self, handle: u32);
}

/// What one [`WakeKeeper::want`] did — returned rather than logged, so the keeper stays pure and
/// the caller keeps its own reporting.
#[derive(Debug)]
pub enum WakeChange {
    /// An assertion was just taken: the machine will not idle-sleep from here on.
    Held,
    /// The assertion was just given back: the machine may sleep again.
    Released,
    /// Nothing to do — already holding one and still wanted, or holding none and none wanted.
    Unchanged,
    /// The assertion could not be taken. Reported ONCE per episode (see [`WakeKeeper::want`]): a
    /// platform that has no assertion at all would otherwise say so on every tick, forever.
    Refused(NxfError),
}

/// Holds at most one wake assertion, and gives it back the moment it is not wanted.
///
/// The type IS the guarantee: there is no way to take a second one, and the only way to hold one is
/// through a value whose `Drop` releases it.
pub struct WakeKeeper<'a> {
    wake: &'a dyn Wake,
    reason: String,
    /// The live handle, and the whole of this type's state.
    held: Option<u32>,
    /// Whether the current run of refusals has already been reported. Reset on every successful
    /// hold and on every release, so a transient failure is announced again the next time runs
    /// start — but a platform that simply cannot do this says so once and then stops talking.
    announced_refusal: bool,
}

impl<'a> WakeKeeper<'a> {
    /// A keeper that has taken nothing yet. `reason` is what a person reading
    /// `pmset -g assertions` will see.
    pub fn new(wake: &'a dyn Wake, reason: impl Into<String>) -> WakeKeeper<'a> {
        WakeKeeper {
            wake,
            reason: reason.into(),
            held: None,
            announced_refusal: false,
        }
    }

    /// Bring the assertion into line with `awake`: take one if it is wanted and none is held, give
    /// the held one back if it is not.
    ///
    /// Idempotent in both directions, which is what lets a caller ask on every tick without keeping
    /// any state of its own — and keeping that state in ONE place is what makes a leaked assertion
    /// impossible to write.
    pub fn want(&mut self, awake: bool) -> WakeChange {
        match (awake, self.held) {
            (true, None) => match self.wake.acquire(&self.reason) {
                Ok(handle) => {
                    self.held = Some(handle);
                    self.announced_refusal = false;
                    WakeChange::Held
                }
                Err(e) => {
                    if std::mem::replace(&mut self.announced_refusal, true) {
                        WakeChange::Unchanged
                    } else {
                        WakeChange::Refused(e)
                    }
                }
            },
            (false, held) => {
                // The EPISODE is over either way, held or refused, so the refusal may be announced
                // again next time: a transient failure must not silence the service for good.
                self.announced_refusal = false;
                match held {
                    Some(handle) => {
                        self.wake.release(handle);
                        self.held = None;
                        WakeChange::Released
                    }
                    None => WakeChange::Unchanged,
                }
            }
            (true, Some(_)) => WakeChange::Unchanged,
        }
    }

    /// Whether an assertion is held right now.
    pub fn is_held(&self) -> bool {
        self.held.is_some()
    }
}

impl Drop for WakeKeeper<'_> {
    /// Guarantee 2 of the three in the module doc: every early return out of the loop that owns
    /// this — including a `?` — gives the assertion back on the way out.
    fn drop(&mut self) {
        if let Some(handle) = self.held.take() {
            self.wake.release(handle);
        }
    }
}

/// The real one: an `IOPMAssertion` on macOS, an honest refusal everywhere else.
pub struct SystemWake;

/// The assertion type, as `IOPMLib.h` spells it —
/// `kIOPMAssertionTypePreventUserIdleSystemSleep` is a `CFSTR` of exactly this literal, so building
/// the string here costs one FFI call and saves linking a CoreFoundation binding crate for two
/// constants.
#[cfg(target_os = "macos")]
const ASSERTION_TYPE: &str = "PreventUserIdleSystemSleep";

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::c_void;

    pub type CFTypeRef = *const c_void;
    pub type IOPMAssertionID = u32;
    pub type IOReturn = i32;

    /// `kCFStringEncodingUTF8`.
    pub const UTF8: u32 = 0x0800_0100;
    /// `kIOPMAssertionLevelOn`.
    pub const LEVEL_ON: u32 = 255;
    /// `kIOReturnSuccess`.
    pub const SUCCESS: IOReturn = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external_representation: u8,
        ) -> CFTypeRef;
        pub fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOPMAssertionCreateWithName(
            assertion_type: CFTypeRef,
            assertion_level: u32,
            assertion_name: CFTypeRef,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        pub fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }
}

/// A `CFString` owning its own copy of `s`, released by the caller.
///
/// `CFStringCreateWithBytes` COPIES the bytes (unlike `CFStringCreateWithBytesNoCopy`), which is
/// what makes it safe to hand a Rust `&str`'s pointer to it and then drop the `&str`.
#[cfg(target_os = "macos")]
fn cf_string(s: &str) -> Option<ffi::CFTypeRef> {
    // SAFETY: `s.as_ptr()`/`s.len()` describe a valid initialized byte range for the duration of
    // the call, and UTF-8 is what a `&str` is. A null return is a documented failure, not UB.
    let cf = unsafe {
        ffi::CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, ffi::UTF8, 0)
    };
    (!cf.is_null()).then_some(cf)
}

#[cfg(target_os = "macos")]
impl Wake for SystemWake {
    fn acquire(&self, reason: &str) -> Result<u32> {
        let kind = cf_string(ASSERTION_TYPE)
            .ok_or_else(|| NxfError::io("could not build the IOPM assertion type string"))?;
        let name = cf_string(reason).ok_or_else(|| {
            // SAFETY: `kind` came back non-null from `CFStringCreateWithBytes`, so this is the
            // matching release of an owned CoreFoundation object on the one path that leaves early.
            unsafe { ffi::CFRelease(kind) };
            NxfError::io("could not build the IOPM assertion name string")
        })?;
        let mut id: ffi::IOPMAssertionID = 0;
        // SAFETY: both arguments are non-null CFStrings this function owns, and `id` is a live
        // local the callee writes exactly once on success.
        let rc = unsafe { ffi::IOPMAssertionCreateWithName(kind, ffi::LEVEL_ON, name, &mut id) };
        // Released whatever the outcome: `IOPMAssertionCreateWithName` retains what it keeps, so
        // these two are this function's to give back either way.
        // SAFETY: both are owned, non-null, and not used again after this point.
        unsafe {
            ffi::CFRelease(kind);
            ffi::CFRelease(name);
        }
        if rc != ffi::SUCCESS {
            return Err(NxfError::io(format!(
                "the system refused an idle-sleep assertion (IOReturn {rc}); this machine may fall \
                 asleep while a run is working"
            )));
        }
        Ok(id)
    }

    fn release(&self, handle: u32) {
        // SAFETY: `handle` is an id this type produced and has not released before — `WakeKeeper`
        // takes it out of its own `Option` before calling, so a double release cannot be written.
        unsafe { ffi::IOPMAssertionRelease(handle) };
    }
}

/// Every other platform. An honest refusal rather than a silent no-op: a service that quietly did
/// not keep the machine awake would be discovered by a run that vanished overnight.
#[cfg(not(target_os = "macos"))]
impl Wake for SystemWake {
    fn acquire(&self, _reason: &str) -> Result<u32> {
        Err(NxfError::io(
            "this platform has no idle-sleep assertion the service can hold, so a long run may not \
             survive the machine going to sleep; configure your own inhibitor (systemd-inhibit, \
             a power profile) if that matters here",
        ))
    }

    fn release(&self, _handle: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records every take and give-back, so the property under test is the SEQUENCE of calls — the
    /// only thing that can be wrong in a way that costs somebody their battery.
    #[derive(Default)]
    struct SpyWake {
        log: RefCell<Vec<String>>,
        next: RefCell<u32>,
        refuse: bool,
    }

    impl SpyWake {
        fn refusing() -> SpyWake {
            SpyWake {
                refuse: true,
                ..SpyWake::default()
            }
        }
        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
        fn outstanding(&self) -> usize {
            let log = self.log.borrow();
            log.iter().filter(|e| e.starts_with("acquire")).count()
                - log.iter().filter(|e| e.starts_with("release")).count()
        }
    }

    impl Wake for SpyWake {
        fn acquire(&self, reason: &str) -> Result<u32> {
            if self.refuse {
                return Err(NxfError::io("stub: refused"));
            }
            let mut next = self.next.borrow_mut();
            *next += 1;
            self.log.borrow_mut().push(format!("acquire {reason}"));
            Ok(*next)
        }
        fn release(&self, handle: u32) {
            self.log.borrow_mut().push(format!("release {handle}"));
        }
    }

    /// **The dangerous direction, first.** An assertion that is not given back is a machine that
    /// never sleeps again, discovered days later as a battery problem.
    #[test]
    fn the_assertion_is_given_back_the_moment_the_last_run_ends() {
        let spy = SpyWake::default();
        let mut keeper = WakeKeeper::new(&spy, "nexus-flow: a run is working");
        assert!(matches!(keeper.want(true), WakeChange::Held));
        assert!(keeper.is_held());
        assert!(matches!(keeper.want(false), WakeChange::Released));
        assert!(!keeper.is_held());
        assert_eq!(
            spy.log(),
            vec![
                "acquire nexus-flow: a run is working".to_string(),
                "release 1".to_string()
            ]
        );
        assert_eq!(spy.outstanding(), 0);
    }

    /// The second half of that direction: the service exits (normally, or out of any `?` in its
    /// loop) while a run is still alive. Nothing calls `want(false)`, and the assertion must still
    /// go back.
    #[test]
    fn a_keeper_that_is_dropped_while_holding_gives_the_assertion_back() {
        let spy = SpyWake::default();
        {
            let mut keeper = WakeKeeper::new(&spy, "r");
            keeper.want(true);
            assert_eq!(spy.outstanding(), 1);
        }
        assert_eq!(
            spy.outstanding(),
            0,
            "a keeper that went out of scope holding one leaked it"
        );
        assert_eq!(spy.log().last().unwrap(), "release 1");
    }

    #[test]
    fn asking_for_it_again_while_holding_takes_nothing_extra() {
        let spy = SpyWake::default();
        let mut keeper = WakeKeeper::new(&spy, "r");
        for _ in 0..5 {
            keeper.want(true);
        }
        assert_eq!(
            spy.log()
                .iter()
                .filter(|e| e.starts_with("acquire"))
                .count(),
            1,
            "one keeper, one assertion, however often it is asked"
        );
        keeper.want(false);
        assert_eq!(spy.outstanding(), 0);
    }

    #[test]
    fn releasing_when_nothing_is_held_does_nothing_rather_than_releasing_a_stale_handle() {
        let spy = SpyWake::default();
        let mut keeper = WakeKeeper::new(&spy, "r");
        assert!(matches!(keeper.want(false), WakeChange::Unchanged));
        keeper.want(true);
        keeper.want(false);
        assert!(matches!(keeper.want(false), WakeChange::Unchanged));
        assert_eq!(
            spy.log()
                .iter()
                .filter(|e| e.starts_with("release"))
                .count(),
            1,
            "a second release of a handle already given back is a bug in somebody else's table"
        );
    }

    #[test]
    fn a_run_that_starts_again_after_a_quiet_spell_takes_a_fresh_assertion() {
        let spy = SpyWake::default();
        let mut keeper = WakeKeeper::new(&spy, "r");
        keeper.want(true);
        keeper.want(false);
        keeper.want(true);
        assert!(keeper.is_held());
        assert_eq!(
            spy.log(),
            vec![
                "acquire r".to_string(),
                "release 1".to_string(),
                "acquire r".to_string()
            ]
        );
        keeper.want(false);
        assert_eq!(spy.outstanding(), 0);
    }

    /// A machine that will not give one out must not turn the service's log into one line per
    /// second — and must not be reported as "held" either.
    #[test]
    fn a_refused_assertion_is_said_once_and_never_pretends_to_be_held() {
        let spy = SpyWake::refusing();
        let mut keeper = WakeKeeper::new(&spy, "r");
        assert!(matches!(keeper.want(true), WakeChange::Refused(_)));
        assert!(!keeper.is_held());
        for _ in 0..10 {
            assert!(matches!(keeper.want(true), WakeChange::Unchanged));
        }
        // …and the refusal is announced again for the next episode, because a transient failure
        // must not silence the service for the rest of its life.
        keeper.want(false);
        assert!(matches!(keeper.want(true), WakeChange::Refused(_)));
    }
}

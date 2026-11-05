//! A scripted panic, on an input's key or on a logged event's key. `decide` is pure and cannot be asked to
//! panic, so the host's fold asks here first (§13's poison cases).

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

static ON_INPUT: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static ON_REPLAY: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static CAS_FAILS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// The append of this input carries a comparison that cannot hold, however often it is re-read.
pub fn cas_fails_on(key: &str) {
    CAS_FAILS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.to_owned());
}

pub(crate) fn cas_fails(key: &str) -> bool {
    CAS_FAILS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(key)
}

/// Folding this input panics until [`clear`].
pub fn panic_on_input(key: &str) {
    ON_INPUT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.to_owned());
}

/// Replaying this logged event panics until [`clear`].
pub fn panic_on_replay(key: &str) {
    ON_REPLAY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.to_owned());
}

pub fn clear(key: &str) {
    ON_INPUT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(key);
    ON_REPLAY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(key);
}

pub(crate) fn on_input(key: &str) {
    let armed = ON_INPUT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(key);
    assert!(!armed, "a scripted panic on an input");
}

pub(crate) fn on_replay(key: &str) {
    let armed = ON_REPLAY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(key);
    assert!(!armed, "a scripted panic on a replay");
}

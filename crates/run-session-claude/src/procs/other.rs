//! A platform this adapter cannot read: every question answers "not established", which is `blind`, and nothing is
//! ever signalled.

use super::{ProcInfo, Signal};

pub(super) fn info(_pid: u32) -> Option<ProcInfo> {
    None
}

pub(super) fn list() -> Option<Vec<ProcInfo>> {
    None
}

pub(super) fn kill(_target: i32, _signal: Signal) -> bool {
    false
}

pub(super) fn group_is_gone(_pgid: u32) -> bool {
    false
}

pub(super) fn reap(_pid: u32) {}

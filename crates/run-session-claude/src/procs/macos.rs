//! `proc_pidinfo`, and `kill(2)`.

use super::{ProcInfo, Signal};

fn pidinfo<T>(pid: i32, flavor: libc::c_int) -> Option<T> {
    let size = std::mem::size_of::<T>() as libc::c_int;
    // SAFETY: `T` is one of libc's plain-old-data `proc_*info` structs, for which all-zero is a valid value.
    let mut buffer: T = unsafe { std::mem::zeroed() };
    // SAFETY: an owned buffer of exactly `size` bytes that outlives the call; `proc_pidinfo` writes at most that and
    // returns how many it wrote, so a short answer is no answer.
    let written = unsafe { libc::proc_pidinfo(pid, flavor, 0, (&raw mut buffer).cast(), size) };
    (written == size).then_some(buffer)
}

pub(super) fn info(pid: u32) -> Option<ProcInfo> {
    let pid_t = i32::try_from(pid).ok()?;
    let bsd: libc::proc_bsdinfo = pidinfo(pid_t, libc::PROC_PIDTBSDINFO)?;
    // A zombie has no task to read, and its CPU is nothing to compare.
    let cpu = pidinfo::<libc::proc_taskinfo>(pid_t, libc::PROC_PIDTASKINFO).map_or(0, |task| {
        task.pti_total_user.saturating_add(task.pti_total_system)
    });
    // SAFETY: `getsid` takes a pid by value and touches no caller memory; a pid that is gone answers -1.
    let sid = unsafe { libc::getsid(pid_t) };
    Some(ProcInfo {
        pid,
        ppid: bsd.pbi_ppid,
        pgid: bsd.pbi_pgid,
        sid: u32::try_from(sid).unwrap_or(0),
        start_ms: i64::try_from(bsd.pbi_start_tvsec).ok()?.checked_mul(1000)?
            + i64::try_from(bsd.pbi_start_tvusec / 1000).ok()?,
        cpu,
        zombie: bsd.pbi_status == libc::SZOMB,
    })
}

pub(super) fn list() -> Option<Vec<ProcInfo>> {
    // SAFETY: a null buffer asks for the count alone.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return None;
    }
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: an owned buffer of exactly `bytes` bytes that outlives the call; the answer is how many pids it wrote.
    let filled = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if filled <= 0 {
        return None;
    }
    Some(
        pids.iter()
            .take(filled as usize)
            .filter(|pid| **pid > 0)
            .filter_map(|pid| info(*pid as u32))
            .collect(),
    )
}

include!("unix_signals.rs");

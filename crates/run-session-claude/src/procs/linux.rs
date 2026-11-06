//! `/proc`, and `kill(2)`.

use super::{ProcInfo, Signal};

/// `(pid's state, ppid, pgrp, session, utime + stime, starttime in ticks)` of a `/proc/<pid>/stat` line, counted from
/// after the LAST `)`: a command name can hold anything, spaces and parentheses included.
fn fields(stat: &str) -> Option<(char, u32, u32, u32, u64, u64)> {
    let rest: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    Some((
        rest.first()?.chars().next()?,
        rest.get(1)?.parse().ok()?,
        rest.get(2)?.parse().ok()?,
        rest.get(3)?.parse().ok()?,
        rest.get(11)?
            .parse::<u64>()
            .ok()?
            .checked_add(rest.get(12)?.parse().ok()?)?,
        rest.get(19)?.parse().ok()?,
    ))
}

fn boot_ms() -> Option<i64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let secs: i64 = stat
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    secs.checked_mul(1000)
}

fn read(pid: u32, boot: i64, hz: i64) -> Option<ProcInfo> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (state, ppid, pgid, sid, cpu, ticks) = fields(&stat)?;
    Some(ProcInfo {
        pid,
        ppid,
        pgid,
        sid,
        start_ms: boot + i64::try_from(ticks).ok()?.checked_mul(1000)? / hz,
        cpu,
        zombie: matches!(state, 'Z' | 'X'),
    })
}

fn clock() -> Option<(i64, i64)> {
    // SAFETY: `sysconf` reads a static configuration value and touches no caller memory.
    let hz = i64::from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as i32);
    (hz > 0).then_some((boot_ms()?, hz))
}

pub(super) fn info(pid: u32) -> Option<ProcInfo> {
    let (boot, hz) = clock()?;
    read(pid, boot, hz)
}

pub(super) fn list() -> Option<Vec<ProcInfo>> {
    let (boot, hz) = clock()?;
    let entries = std::fs::read_dir("/proc").ok()?;
    Some(
        entries
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter_map(|pid| read(pid, boot, hz))
            .collect(),
    )
}

include!("unix_signals.rs");

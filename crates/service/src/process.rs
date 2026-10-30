//! **What the processes under one open tool call are doing** (nxf 6j6v.pszd cut 2, design 8nx4
//! section 1 (b)).
//!
//! A running subprocess counts as life only while it uses CPU; "writes output" is a system call and
//! costs CPU too, so the CPU counter carries both. The counter is the platform's RAW unit and is
//! compared for change only — converting mach units to ms would round a `gh run watch` (tens of ms
//! per minute) to zero, which is the case this exists to keep alive.
//!
//! **Which processes**: every descendant of a pid whose start lies at or after `since`, with the
//! slack [`born_together`](crate::heartbeat::born_together) concedes. That is what excludes `node`,
//! `claude`, a stdio MCP server started at init and a background job of an earlier call, without a
//! depth rule. A descendant whose start cannot be read is COUNTED — the generous side.
//!
//! **Races**: a pid that vanished between the listing and its read is skipped. A sample over a
//! partial tree still changes the tuple, which reads as progress — again the generous direction.

use std::time::SystemTime;

/// One reading of the processes started under an open call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SubprocessSample {
    /// Descendants started at or after `since` (with the start-time slack), the unreadable-start
    /// ones included.
    pub count: u32,
    /// Σ CPU counter of those processes in the platform's RAW unit — mach absolute time units on
    /// macOS, clock ticks on Linux. Compared for change only; never converted, never printed with
    /// a unit.
    pub cpu_units: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct StatFields {
    ppid: u32,
    cpu_ticks: u64,
}

/// ppid (field 4) and utime + stime (fields 14, 15) of a `/proc/<pid>/stat` line, counted from after
/// the LAST `)` — see `heartbeat::starttime_ticks` for the trap. Pure and not `cfg`-gated, so the
/// trap is tested from anywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stat_fields(stat_line: &str) -> Option<StatFields> {
    let rest: Vec<&str> = stat_line[stat_line.rfind(')')? + 1..]
        .split_whitespace()
        .collect();
    // After ')': index 0 = state (field 3), 1 = ppid (4), 11 = utime (14), 12 = stime (15).
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some(StatFields {
        ppid: rest.get(1)?.parse().ok()?,
        cpu_ticks: utime.checked_add(stime)?,
    })
}

/// Whether a process that started at `started` belongs under a call opened at `since`.
fn started_under(started: Option<SystemTime>, since: SystemTime) -> bool {
    match started {
        // Unreadable: counted — the generous side.
        None => true,
        Some(s) => s + crate::heartbeat::START_TIME_SLACK >= since,
    }
}

/// `(count, Σ CPU counter)` of the descendants of `pid` started at or after `since`.
///
/// `None` on a platform this cannot read, or when `pid` itself cannot be read — never a zero
/// sample, which would read as "nothing running" for a sidecar that is merely unreadable.
pub fn descendants_started_since(pid: u32, since: SystemTime) -> Option<SubprocessSample> {
    let tree = descendants(pid)?;
    let mut sample = SubprocessSample {
        count: 0,
        cpu_units: 0,
    };
    for child in tree {
        if !started_under(crate::heartbeat::process_started_at(child), since) {
            continue;
        }
        // A child that vanished between the listing and this read is skipped.
        let Some(cpu) = cpu_units(child) else {
            continue;
        };
        sample.count = sample.count.saturating_add(1);
        sample.cpu_units = sample.cpu_units.saturating_add(cpu);
    }
    Some(sample)
}

#[cfg(target_os = "macos")]
fn descendants(pid: u32) -> Option<Vec<u32>> {
    // `pid` itself must be readable, or the answer is None (not "no children").
    crate::heartbeat::process_started_at(pid)?;
    let mut out = Vec::new();
    let mut frontier = vec![pid];
    while let Some(p) = frontier.pop() {
        let Ok(p) = libc::pid_t::try_from(p) else {
            continue;
        };
        let mut buf = vec![0 as libc::pid_t; 1024];
        let bytes = (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: `buf` is an owned buffer of exactly `bytes` bytes that outlives the call; the call
        // writes at most that and returns the number of pids written (or <= 0 on failure).
        let n = unsafe { libc::proc_listchildpids(p, buf.as_mut_ptr().cast(), bytes) };
        if n <= 0 {
            continue;
        }
        for &c in buf.iter().take((n as usize).min(buf.len())) {
            if c > 0 {
                out.push(c as u32);
                frontier.push(c as u32);
            }
        }
    }
    Some(out)
}

#[cfg(target_os = "macos")]
fn cpu_units(pid: u32) -> Option<u64> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: `proc_taskinfo` is plain old data; all-zero is a valid value.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: as `process_started_at`'s macOS arm — an owned, zeroed buffer of exactly `size` bytes
    // that outlives the call; `proc_pidinfo` writes at most `size` bytes and returns how many.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        )
    };
    (written == size).then(|| info.pti_total_user.saturating_add(info.pti_total_system))
}

#[cfg(target_os = "linux")]
fn descendants(pid: u32) -> Option<Vec<u32>> {
    // `pid` itself must be readable, or the answer is None (not "no children").
    std::fs::metadata(format!("/proc/{pid}")).ok()?;
    let mut parent_of = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(p) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        // A process that vanished mid-scan has no stat: skipped.
        if let Some(f) = std::fs::read_to_string(format!("/proc/{p}/stat"))
            .ok()
            .as_deref()
            .and_then(stat_fields)
        {
            parent_of.push((p, f.ppid));
        }
    }
    let mut out = Vec::new();
    let mut frontier = vec![pid];
    while let Some(p) = frontier.pop() {
        for &(c, pp) in &parent_of {
            if pp == p {
                out.push(c);
                frontier.push(c);
            }
        }
    }
    Some(out)
}

#[cfg(target_os = "linux")]
fn cpu_units(pid: u32) -> Option<u64> {
    stat_fields(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?).map(|f| f.cpu_ticks)
}

/// The blind arm: this platform cannot list children or read their CPU, so the sample is `None`
/// rather than a guess.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn descendants(_pid: u32) -> Option<Vec<u32>> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn cpu_units(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_count_from_after_the_last_paren() {
        // pid (comm with ") (" in it) state ppid … utime(14) stime(15) … starttime(22)
        let line = "42 (a) b) S 7 1 1 0 -1 0 0 0 0 0 11 13 0 0 20 0 1 0 999 0 0";
        assert_eq!(
            stat_fields(line),
            Some(StatFields {
                ppid: 7,
                cpu_ticks: 24
            })
        );
    }
}

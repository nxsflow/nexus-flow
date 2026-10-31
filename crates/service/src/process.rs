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

/// **Every descendant of `pid` with its start instant** (nxf 6j6v.1r61) — what the ladder records
/// when it stops a session, so that the look after the end can name a descendant that outlived
/// the stop. The start is the operating system's (`None` when it cannot be read); a process is
/// told apart from a later one holding the same number by [`still_the_process`].
///
/// `None` on a platform this cannot read, or when `pid` itself cannot be read.
pub fn descendants_with_start(pid: u32) -> Option<Vec<(u32, Option<SystemTime>)>> {
    Some(
        descendants(pid)?
            .into_iter()
            .map(|child| (child, crate::heartbeat::process_started_at(child)))
            .collect(),
    )
}

/// **Whether `pid` is still the process that started at `started`** (RFC3339) — the start read
/// again and compared with the slack [`born_together`](crate::heartbeat::born_together) concedes.
/// A pid that is gone, or that the system handed to a later process, is not; nor is one whose start
/// cannot be read, since naming a stranger as a survivor would be worse than naming none.
pub fn still_the_process(pid: u32, started: &str) -> bool {
    crate::heartbeat::born_together(started, crate::heartbeat::process_started_at(pid))
        == Some(true)
}

#[cfg(target_os = "macos")]
fn descendants(pid: u32) -> Option<Vec<u32>> {
    // `pid` itself must be readable, or the answer is None (not "no children").
    crate::heartbeat::process_started_at(pid)?;
    let mut out = Vec::new();
    // A pid reused between two listings could reappear deeper in the tree; each is walked once.
    let mut seen = std::collections::HashSet::from([pid]);
    let mut frontier = vec![pid];
    while let Some(p) = frontier.pop() {
        let Ok(p) = libc::pid_t::try_from(p) else {
            continue;
        };
        // At most 1,024 children PER PARENT are listed; the rest are cut off silently. A cut-off
        // list only shrinks the sample, which is the generous direction.
        let mut buf = vec![0 as libc::pid_t; 1024];
        let bytes = (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: `buf` is an owned buffer of exactly `bytes` bytes that outlives the call; the call
        // writes at most that and returns the number of pids written (or <= 0 on failure).
        let n = unsafe { libc::proc_listchildpids(p, buf.as_mut_ptr().cast(), bytes) };
        if n <= 0 {
            continue;
        }
        for &c in buf.iter().take((n as usize).min(buf.len())) {
            if c > 0 && seen.insert(c as u32) {
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
    let mut children_of: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
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
            children_of.entry(f.ppid).or_default().push(p);
        }
    }
    let mut out = Vec::new();
    // One snapshot of the table cannot hold a cycle, but each pid is walked once regardless.
    let mut seen = std::collections::HashSet::from([pid]);
    let mut frontier = vec![pid];
    while let Some(p) = frontier.pop() {
        for &c in children_of.get(&p).into_iter().flatten() {
            if seen.insert(c) {
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

    /// A line that cannot be read is NO answer — never a zero counter or a ppid of 0, which would
    /// attach the process to the wrong parent or freeze its CPU.
    #[test]
    fn stat_fields_refuses_a_line_it_cannot_read() {
        for (line, why) in [
            ("42 a S 7 1 1 0 -1 0 0 0 0 0 11 13", "no ')' at all"),
            ("42 (a) S 7 1 1 0 -1 0 0 0 0 0 11", "truncated before stime"),
            ("42 (a) S 7 1 1 0 -1 0 0 0 0 0", "truncated before utime"),
            ("42 (a)", "nothing after the comm"),
            ("42 (a) S x 1 1 0 -1 0 0 0 0 0 11 13", "ppid not a number"),
            ("42 (a) S 7 1 1 0 -1 0 0 0 0 0 u 13", "utime not a number"),
            ("42 (a) S 7 1 1 0 -1 0 0 0 0 0 11 -1", "stime negative"),
            (
                "42 (a) S 7 1 1 0 -1 0 0 0 0 0 18446744073709551615 1",
                "utime + stime overflows",
            ),
        ] {
            assert_eq!(stat_fields(line), None, "{why}: {line}");
        }
    }

    /// The slack is the whole of the rule, so its edges are pinned: a process that started up to
    /// `START_TIME_SLACK` before the call is counted, one second more is not.
    #[test]
    fn started_under_admits_exactly_the_slack_before_since() {
        use crate::heartbeat::START_TIME_SLACK;
        use std::time::Duration;
        let since = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let second = Duration::from_secs(1);
        assert!(started_under(Some(since), since), "started at since");
        assert!(started_under(Some(since + second), since), "after since");
        assert!(
            started_under(Some(since - START_TIME_SLACK), since),
            "exactly the slack before"
        );
        assert!(
            started_under(Some(since - START_TIME_SLACK + second), since),
            "inside the slack"
        );
        assert!(
            !started_under(Some(since - START_TIME_SLACK - second), since),
            "one second past the slack"
        );
        assert_eq!(
            START_TIME_SLACK,
            Duration::from_secs(60),
            "the slack itself"
        );
    }

    /// A start that cannot be read is COUNTED, whatever `since` is — the generous side.
    #[test]
    fn started_under_counts_an_unreadable_start() {
        use std::time::Duration;
        for since in [
            SystemTime::UNIX_EPOCH,
            SystemTime::now(),
            SystemTime::now() + Duration::from_secs(86_400 * 365),
        ] {
            assert!(started_under(None, since), "{since:?}");
        }
    }
}

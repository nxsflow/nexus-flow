//! The heartbeat the running service publishes, and the liveness probe that decides whether the
//! process behind it is still there.
//!
//! There is no IPC. The service rewrites one small JSON file after every pass, and anybody asking
//! "is it running, and what did it last do?" reads that file — the same deliberately file-based
//! idiom as the write-nudge marker. A present heartbeat is NOT by itself proof of a live process
//! (a `SIGKILL`ed service leaves its last one behind forever, since nothing is left alive to
//! overwrite it), so the reading also probes the recorded pid.

use std::path::Path;
use std::time::{Duration, SystemTime};

use nxs_foundation::error::{NxfError, Result};
use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::atomic::write_atomic;

/// The service's last-known state, rewritten after each real pass (never on an idle tick).
/// `PartialEq`/`Eq` exist so a round-trip test can assert equality directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub pid: u32,
    pub started_at: String,
    pub last_pass_at: String,
    pub workspaces: Vec<WorkspaceHealth>,
    /// **Which binary this service is running** — the RESOLVED path, never the
    /// [`ServiceHome::program`](crate::ServiceHome::program) alias it was `exec`ed through (nxf
    /// 6j6v.dcpk (a)).
    ///
    /// The alias is a moving target: `nxs sync daemon install` re-points it, two installations do
    /// not collide, and the last one silently wins. So "what is the service running" was a question
    /// with no answer anywhere — measured on the owner's machine on 2026-08-29, where the running
    /// process and what the alias pointed at were already different builds and nothing said so.
    ///
    /// `Option` for one reason and it is not optionality of the fact: a heartbeat written by a
    /// service from before this field existed is still on disk on every machine that has one, and
    /// `status` must read it rather than refuse it. A service writing one today always fills both.
    #[serde(default)]
    pub program: Option<String>,
    /// The version of that binary, from its own `CARGO_PKG_VERSION`. Same `Option` and same reason
    /// as [`program`](Heartbeat::program).
    #[serde(default)]
    pub version: Option<String>,
    /// **Which service instance wrote this** — [`Instance::name`](crate::Instance::name) (nxf
    /// 6j6v.gd9p).
    ///
    /// A heartbeat is a file in an instance's own home, so a reader that opened it already knows;
    /// what this adds is that the FILE says so too. It is the one artefact of a service that gets
    /// copied, pasted into an issue and read out of context, and "which of the two services on this
    /// machine is this" is then unanswerable from the text. Same `Option` and same reason as the
    /// two fields above: a heartbeat written before this field existed is still on disk.
    #[serde(default)]
    pub instance: Option<String>,
    /// **The heartbeat writes that did NOT happen** since this service started (nxf 6j6v.kcan).
    ///
    /// A heartbeat that cannot be written is indistinguishable from a service that is not running:
    /// `status` derives its whole answer from this file, so a failed write makes a working service
    /// vanish, and the reason goes only into a log nobody opens. Measured on the owner's machine
    /// on 2026-08-31 — `No space left on device`, twice, while the service was syncing and holding
    /// the machine awake.
    ///
    /// It is carried IN the heartbeat and not beside it, which sounds circular and is the point:
    /// while the writes are failing nothing can be recorded anywhere, so what this delivers is the
    /// explanation AFTERWARDS — the next write that succeeds says the gap had a cause, instead of
    /// leaving a reader to conclude the service was down for an hour.
    ///
    /// Same `#[serde(default)]` and the same reason as the three fields above: a heartbeat written
    /// before this field existed is on disk on every machine that has one.
    #[serde(default)]
    pub write_failures: Option<WriteFailures>,
}

/// What [`Heartbeat::write_failures`] carries: how many heartbeat writes failed since this service
/// started, and what the most recent one said.
///
/// A count and one message rather than a list: the interesting facts are "there was a gap and it
/// was not a stopped service" and "here is why", and a full-disk episode produces one identical
/// message per pass for as long as it lasts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteFailures {
    /// How many heartbeat writes have failed since this service started.
    pub count: u64,
    /// When the most recent one failed, as this service records instants.
    pub last_at: String,
    /// What it said — the error's own message, verbatim.
    pub last_error: String,
}

/// The running service's own tally of failed heartbeat writes, carried in memory across the loop.
///
/// **The service keeps running when it cannot write its heartbeat, and that is a decision** (nxf
/// 6j6v.kcan, the open question the ticket left for whoever implemented it). Syncing and firing
/// deadlines are the job; the heartbeat is bookkeeping about the job. A service that exited on a
/// full disk would take the machine's only clock down with it — and a full disk that later frees
/// up heals by itself, which an exit would not. So the write is best-effort, and what changes here
/// is only that its failure stops being invisible.
#[derive(Debug, Clone, Default)]
pub struct WriteFailureLog {
    count: u64,
    last: Option<(String, String)>,
}

impl WriteFailureLog {
    /// Note one failed write: `at` as this service records instants, `error` verbatim.
    pub fn record(&mut self, at: String, error: String) {
        self.count = self.count.saturating_add(1);
        self.last = Some((at, error));
    }

    /// What to stamp into the heartbeat about to be written — `None` while nothing has failed, so
    /// a service that has never missed a beat carries no field about missing them.
    ///
    /// It is NOT cleared by a successful write. The tally is "since this service started", which
    /// is the question a reader looking at an unexplained gap is actually asking; clearing it on
    /// the next success would erase the explanation one pass after publishing it, and the reader
    /// arrives later than that.
    pub fn snapshot(&self) -> Option<WriteFailures> {
        let (last_at, last_error) = self.last.clone()?;
        Some(WriteFailures {
            count: self.count,
            last_at,
            last_error,
        })
    }
}

/// One workspace's last-known outcome inside a [`Heartbeat`]. A failing pass sets `last_error` and
/// leaves `last_ok`/`pushed`/`pulled` at whatever they were after the last SUCCESS (or `None`/`0`/`0`
/// if there has never been one) — a failure must not erase the record of the last time this
/// workspace actually synced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceHealth {
    pub path: String,
    pub last_ok: Option<String>,
    pub last_error: Option<String>,
    pub pushed: u64,
    pub pulled: u64,
}

/// What a reader can conclude about the service process itself.
///
/// `Unknown` is a first-class answer rather than a guess (6j6v.4t9p). The predecessor returned a
/// bare `bool` hardcoded to `true` on non-unix, and both call sites read that one bool: right for
/// the lock (conservative — never admit a second service on a guess), wrong for a human asking "is
/// my service up?", who was then told a long-dead process was running forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// A heartbeat exists and its process answers a liveness probe.
    Running,
    /// Either no heartbeat exists at all, or the recorded process is proven gone.
    NotRunning,
    /// **A process HOLDS the recorded pid, and it began AFTER this heartbeat says the service
    /// did** — so the service that wrote this is gone and the operating system handed its id on
    /// (nxf 6j6v.kcan, narrowed by 6j6v.d43g).
    ///
    /// It used to be reported as [`NotRunning`](ServiceState::NotRunning), which is a false map in
    /// the other direction: a person reads "the process is gone", reaches for the pid, and it is
    /// somebody else's. The service is not running; the process in front of them is unrelated to
    /// it, and starting the service is the move rather than killing that pid.
    ///
    /// **It carried a SECOND reading until nxf 6j6v.d43g**, and no longer does: a service that
    /// stamped its `started_at` far later than its own `execve` — 69 minutes of it on the owner's
    /// machine, behind a macOS permission dialog — used to land here too, and now reads as
    /// [`Running`](ServiceState::Running), which is what it was the whole time. See
    /// [`born_together`] for why the two are told apart by the DIRECTION of the disagreement.
    Unconfirmed,
    /// A heartbeat exists, but this platform has no way to probe whether its process is alive.
    Unknown,
}

impl ServiceState {
    /// The `--json` rendering of the RUNNING question: `true`/`false`/`null`.
    ///
    /// [`Unconfirmed`](ServiceState::Unconfirmed) is `null` and not `false`: it is not a "no", and
    /// spending the `false` on it is exactly the lie this state was split out of. The precise
    /// answer travels beside it as [`as_str`](ServiceState::as_str), so nothing is lost by keeping
    /// this key's meaning — and its shape — the one every existing reader already parses.
    pub fn as_json(self) -> Option<bool> {
        match self {
            ServiceState::Running => Some(true),
            ServiceState::NotRunning => Some(false),
            ServiceState::Unconfirmed | ServiceState::Unknown => None,
        }
    }

    /// The `--json` rendering of the STATE itself — the four answers by name, so a machine reader
    /// can tell the two `null`s of [`as_json`](ServiceState::as_json) apart.
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceState::Running => "running",
            ServiceState::NotRunning => "not_running",
            ServiceState::Unconfirmed => "unconfirmed",
            ServiceState::Unknown => "unknown",
        }
    }
}

/// What a pid probe concluded. Each platform's [`probe_pid`] constructs a SUBSET — unix:
/// `Alive`/`Dead`; everything else: `Unknown` — so on any ONE target the rest look dead to the
/// compiler. The enum is the full vocabulary of what a probe can conclude anywhere, and both
/// readings of it are pinned by tests that compile on every platform, which is why the lint is
/// silenced rather than the type split.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Liveness {
    /// The process exists (or exists and may not be signalled by us — `EPERM`).
    Alive,
    /// Proven absent: `kill(pid, 0)` reported `ESRCH`, or the pid cannot name a process at all.
    Dead,
    /// This platform has no probe here, so nothing was learned.
    Unknown,
}

/// Probe whether `pid` is currently alive. `cfg(unix)`: signal 0 sends nothing, it only probes
/// whether `kill` would succeed — `ESRCH` is the one errno that specifically means "no such
/// process"; anything else (most notably `EPERM`, a process that exists but we may not signal)
/// still counts as alive. `cfg(not(unix))`: [`Liveness::Unknown`], always — a real check needs a
/// platform API (`OpenProcess` on Windows) and belongs with Windows support (6j6v.rf4b).
///
/// Takes the `u32` its callers actually hold (a heartbeat's `pid`, a lock file's contents) and
/// narrows to `kill`'s `pid_t` here: a pid outside `i32` cannot be a real process, and an `as`
/// cast would hand `kill` a NEGATIVE value it reads as a process GROUP.
#[cfg(unix)]
pub(crate) fn probe_pid(pid: u32) -> Liveness {
    let Ok(pid) = i32::try_from(pid) else {
        return Liveness::Dead;
    };
    // SAFETY: `kill` with signal 0 sends nothing and touches no memory the caller owns.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Liveness::Alive;
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Liveness::Dead
    } else {
        Liveness::Alive
    }
}

#[cfg(not(unix))]
pub(crate) fn probe_pid(_pid: u32) -> Liveness {
    Liveness::Unknown
}

/// The LOCK's reading of a probe: anything but a PROVEN death means the recorded owner still holds
/// it. Conservative on purpose — a wrongly refused start is far cheaper than two services racing
/// the same watermark, and reclaiming a lock on a mere guess is what would admit the second one.
///
/// Only the `cfg(not(unix))` [`crate::ServiceLock`] fallback calls this — the unix lock takes its
/// exclusion from `flock` and never probes a pid at all — so on unix its only caller is its own
/// test. The `cfg_attr` is deliberately narrow: on the platform that DOES use it, an unused one is
/// still a lint error, which CI can see (6j6v.4t9p).
#[cfg_attr(unix, allow(dead_code))]
pub(crate) fn lock_owner_is_alive(liveness: Liveness) -> bool {
    !matches!(liveness, Liveness::Dead)
}

/// A reader's reading of the same probe, in the opposite direction: an unprobed pid reported as
/// running is the stale-heartbeat lie, and reported as stopped is a phantom crash — so it is
/// neither.
pub(crate) fn state_of(liveness: Liveness) -> ServiceState {
    match liveness {
        Liveness::Alive => ServiceState::Running,
        Liveness::Dead => ServiceState::NotRunning,
        Liveness::Unknown => ServiceState::Unknown,
    }
}

/// Read the heartbeat at `path`. A genuinely absent file reads as `None` — "not running" — not an
/// error, exactly like [`crate::registry::load_from`]'s "absent registry = empty" contract. A
/// present-but-malformed file (hand-edited, or torn by something outside this module's own atomic
/// writer) is a loud validation error naming it.
pub fn read_from(path: &Path) -> Result<Option<Heartbeat>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(NxfError::io(format!(
                "reading the service heartbeat {}: {e}",
                path.display()
            )))
        }
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let hb: Heartbeat = serde_json::from_str(&raw).map_err(|e| {
        NxfError::validation(format!(
            "the service heartbeat {} is not valid JSON: {e}",
            path.display()
        ))
    })?;
    Ok(Some(hb))
}

/// Write the heartbeat atomically, via the SAME [`write_atomic`] the registry uses, so a
/// concurrently-reading status never observes a half-written file.
pub fn write_to(path: &Path, hb: &Heartbeat) -> Result<()> {
    let raw = serde_json::to_vec(hb)
        .map_err(|e| NxfError::io(format!("serializing the service heartbeat: {e}")))?;
    write_atomic(path, &raw)
}

/// How far BEFORE its own process a service may claim to have started and still be that process
/// (6j6v.0wvp, narrowed to one direction by 6j6v.d43g).
///
/// It absorbs a clock adjustment, and nothing else. The two instants are not the same measurement —
/// the OS stamps `execve`, the service stamps its own start — but since 6j6v.d43g the service reads
/// that same `execve` instant back out of the kernel, so the two agree exactly wherever the platform
/// can say, and where it cannot the service's fallback stamp is LATER, never earlier. A recorded
/// start that is earlier at all is therefore already odd; a minute is far more room than a
/// sub-second correction landing in between needs.
///
/// **What the slack leaves open, stated rather than discovered:** a pid reused within a minute of
/// the service's OWN start still passes. That is not the failure this exists to fix — the reported
/// case is a service that died long ago and whose pid the system handed to something else, which
/// this catches however wide the gap.
const START_TIME_SLACK: Duration = Duration::from_secs(60);

/// How far AFTER its own process a service may claim to have started and still be that process
/// (nxf 6j6v.d43g, bounded after the review of PR #421).
///
/// **The forward direction is a compatibility affordance, and that is what sizes it.** A service
/// running today's code stamps [`recorded_start`], which IS the process's `execve`, so the two
/// instants agree exactly wherever the platform can answer — the forward gap is only ever produced
/// by a heartbeat an OLDER build left on disk, which stamped the moment of its first WRITE. The
/// question the ceiling has to answer is therefore not "how long may a service take to start" but
/// "how long could one of those older services have sat between `execve` and its first write" —
/// and the answer is: until a human clicked the permission dialog it was stuck behind. A day
/// covers one left until the next morning; the measured case was 69 minutes.
///
/// **What the bound buys, which is why it is a bound at all** (review of PR #421, Integrity #1).
/// The direction argument in [`born_together`] assumes the wall clock moved FORWARD between a dead
/// service's last stamp and the birth of whatever inherited its pid. A backward step breaks that,
/// and unbounded forward trust would then report a stranger as `Running` — worse than the
/// `Unconfirmed` it replaced, because it asserts liveness. The ceiling does not abolish that case;
/// it shrinks it to the same shape [`START_TIME_SLACK`] already concedes on the other side: the
/// misread now needs the clock step AND the pid handover to both land inside a day of each other,
/// rather than any clock step at all.
const LATE_STAMP_CEILING: Duration = Duration::from_secs(24 * 60 * 60);

/// Do the recorded start instant and the OS's own answer describe the SAME process?
///
/// `None` when there is no answer to compare against — an unreadable stamp, or a platform with no
/// start-time API. Pure, so the decision is provable without a process to point it at.
///
/// **The direction of the difference is the whole discriminator** (nxf 6j6v.d43g). The two cases
/// are not symmetrical, and treating them as if they were is what reported a working service as
/// somebody else's process for 69 minutes:
///
/// * **Recorded BEFORE the process began** is the handed-on pid. A process that inherits a dead
///   service's id must start after that service died, which is after it stamped its first
///   heartbeat — so this is exactly the shape 6j6v.0wvp exists to catch, and it is caught however
///   large the gap.
/// * **Recorded AFTER the process began** is the service stamping itself late — a binary sitting
///   behind a macOS permission dialog produced 69 minutes of it on the owner's machine — and it is
///   allowed up to [`LATE_STAMP_CEILING`], whose doc says what sizes that day.
///
/// **The argument holds for the ORDER of events and assumes the CLOCK agreed with it** (review of
/// PR #421, Integrity #1). "The heir started after the service died" is about real time; the two
/// instants being compared are wall-clock readings, and a backward step between them — an NTP
/// correction after a skewed boot, a VM resume, a hand-set clock — can put the heir's reading
/// below the dead service's. That is why the forward side has a ceiling rather than the unbounded
/// trust the argument alone would justify: past a day the reading is treated as the clock event it
/// almost certainly is. Inside a day it is not, and that residue is named rather than implied —
/// it is the same concession [`START_TIME_SLACK`] makes about a pid reused within a minute.
///
/// **What it also leaves open:** a heartbeat hand-edited to a start instant inside the ceiling, on
/// a pid that now belongs to something else, reads as running. A written stamp cannot land there
/// by itself, and the reading it replaces — "a live process holds this id and did not write this"
/// — was no more useful about a file somebody had edited.
///
/// **`pub` since nxf 6j6v.b9nf, and for the argument above rather than for the six lines below it.**
/// A second reader now has the same question about a different file: `nexus-chat`'s session claim
/// (`<root>/.nxs/agent-logs/<session>.pid`) records the pid of a sidecar that a withdrawal may
/// SIGTERM, and `kill(pid, 0)` cannot tell that pid from one the system handed to somebody else
/// after the sidecar crashed. Copying this function there would have copied the direction argument,
/// the two bounds and the clock-step residue with it — four things that then drift — which is the
/// defect this house's own docs warn about. It is pure and takes the recorded instant as a string,
/// so it needs nothing of a heartbeat: the caller brings its own record and its own
/// [`process_started_at`].
pub fn born_together(recorded: &str, os_start: Option<SystemTime>) -> Option<bool> {
    let os_start = os_start?;
    let recorded = OffsetDateTime::parse(recorded, &Rfc3339).ok()?;
    let recorded = SystemTime::from(recorded);
    Some(match recorded.duration_since(os_start) {
        Ok(late) => late <= LATE_STAMP_CEILING,
        Err(early) => early.duration() <= START_TIME_SLACK,
    })
}

/// The full liveness question about a heartbeat: is a process there, and is it THIS one?
///
/// The pid alone was never enough (6j6v.0wvp). A service dies, the operating system hands its pid
/// to something unrelated, and `kill(pid, 0)` answers yes forever — so `status` reported a service
/// that had been gone for weeks as running, and the non-unix lock would refuse a start on its
/// behalf. The heartbeat already carried `started_at`; comparing it against the process's real
/// start time is what makes the answer about a PROCESS rather than about a number.
///
/// No third answer was needed — [`Liveness`] already had the vocabulary. What changed is that
/// `Alive` now means "a process is there AND it is the one that wrote this", and a platform that
/// cannot read a start time says `Unknown` instead of guessing, exactly as it already did for the
/// existence question.
pub(crate) fn probe_heartbeat(hb: &Heartbeat) -> ServiceState {
    match probe_pid(hb.pid) {
        // Proven gone, or nothing learned: the identity question does not arise.
        answer @ (Liveness::Dead | Liveness::Unknown) => state_of(answer),
        Liveness::Alive => match born_together(&hb.started_at, process_started_at(hb.pid)) {
            Some(true) => ServiceState::Running,
            // A process is there and it did not write this. NOT "gone" (nxf 6j6v.kcan) — see
            // [`ServiceState::Unconfirmed`] for what produces it, and [`born_together`] for the
            // late-stamping service this deliberately no longer catches (nxf 6j6v.d43g).
            Some(false) => ServiceState::Unconfirmed,
            None => ServiceState::Unknown,
        },
    }
}

/// The instant a service records as its own start, for the heartbeat it is about to write: the
/// moment the operating system says `pid` began, and `fallback` only where it cannot say (nxf
/// 6j6v.d43g).
///
/// It used to be the moment of the WRITE, which is a different measurement wearing the same name.
/// The two normally differ by milliseconds — initialisation, argument parsing, taking the
/// single-instance lock — and on the owner's machine they differed by 69 minutes, because the
/// binary lived on a removable volume and macOS held the process in a permission dialog until
/// somebody clicked it. `started_at` now means what it says, and the reader comparing it against
/// the process's real start compares like with like.
///
/// `fallback` is the caller's own "now": a platform with no start-time API still has to stamp
/// something, and the write instant is what every service stamped before this. It is never a
/// silent lie either — a fallback stamp is LATER than the process's start, and
/// [`born_together`] allows exactly that direction without bound.
pub fn recorded_start(pid: u32, fallback: SystemTime) -> SystemTime {
    process_started_at(pid).unwrap_or(fallback)
}

/// When the process `pid` started, as an absolute instant — `None` where this platform has no way
/// to say.
///
/// Platform-bound by nature, and each answer is a different mechanism: macOS keeps it in
/// `proc_bsdinfo` behind `proc_pidinfo(PROC_PIDTBSDINFO)`, Linux in field 22 of `/proc/<pid>/stat`
/// (ticks since boot, so the boot instant from `/proc/stat` has to be added back), and Windows would
/// need `GetProcessTimes` — which belongs with Windows support (6j6v.rf4b), not here.
///
/// **The Linux answer is RECONSTRUCTED and may be a tick or two off** (project memory
/// `linux-process-start-is-reconstructed`): ticks since boot plus a boot instant read from a second
/// file, so two readings of the same process can differ by about a second. Nothing may compare it
/// for equality — [`born_together`] compares it with slack in both directions, which is the only
/// supported way to use this.
///
/// **`pub` since nxf 6j6v.b9nf**, with [`born_together`], which is the only thing that knows how to
/// read the answer: `nexus-chat` asks the same identity question about a session's sidecar before
/// it signals it. See that function's doc for why this is shared rather than copied.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn process_started_at(pid: u32) -> Option<SystemTime> {
    let pid = i32::try_from(pid).ok()?;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: `info` is a fully-owned, zeroed buffer of exactly `size` bytes that outlives the
    // call, and `proc_pidinfo` writes at most `size` bytes into it. It returns the number of bytes
    // written, or 0 on failure.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
            size as libc::c_int,
        )
    };
    // A short or failed answer is NO answer — never a start time of zero, which would make every
    // process look impossibly old and every heartbeat look like somebody else's.
    if written != size as libc::c_int {
        return None;
    }
    Some(
        SystemTime::UNIX_EPOCH
            + Duration::from_secs(info.pbi_start_tvsec)
            + Duration::from_micros(info.pbi_start_tvusec),
    )
}

/// Field 22 (`starttime`, in clock ticks since boot) of a `/proc/<pid>/stat` line.
///
/// **Pure, and not `cfg`-gated to Linux, so its own trap can be tested from anywhere** (review of
/// PR #369–#373, Test Quality #3). Field 2 is the executable name IN PARENTHESES and may itself
/// contain spaces and parentheses — `sh -c 'exec -a "a) b" sleep 60'` is enough to produce one — so
/// the fields are counted from after the LAST `)`, never by splitting the whole line. After that
/// paren, field 3 is the first, which puts `starttime` at index 19 of what remains. The trap was
/// documented and avoided from the start; what was missing was a test that would notice if the
/// avoidance were undone, and the only test there was ran against the test binary's own `comm`,
/// which contains neither.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn starttime_ticks(stat_line: &str) -> Option<u64> {
    let rest = &stat_line[stat_line.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// See the macOS arm above for the whole argument — and for why the tick reconstruction below may
/// never be compared for equality.
#[cfg(target_os = "linux")]
pub fn process_started_at(pid: u32) -> Option<SystemTime> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks: u64 = starttime_ticks(&stat)?;
    // SAFETY: `sysconf` reads a static configuration value and touches no caller memory.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz <= 0 {
        return None;
    }
    let since_boot = Duration::from_nanos(ticks.checked_mul(1_000_000_000)? / hz as u64);
    // `/proc/<pid>/stat` measures from BOOT, so the boot instant has to come from somewhere:
    // `/proc/stat`'s `btime` line, in epoch seconds.
    let boot: u64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(boot) + since_boot)
}

/// The blind arm: this platform cannot say when a process began, so every identity question over it
/// answers "not established" rather than guessing — see the macOS arm above.
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
pub fn process_started_at(_pid: u32) -> Option<SystemTime> {
    None
}

/// Render `t` the way a heartbeat records an instant: RFC3339.
///
/// Public and here, rather than a private copy in each writer, because the identity check above
/// COMPARES against what a writer produced — two spellings of "the same instant" that drifted apart
/// would make a live service look like somebody else's process. The `unwrap_or_else` fallback is
/// defensive, not expected: `OffsetDateTime::from(SystemTime)` is infallible, and `format` only
/// fails on a component outside RFC3339's representable range.
pub fn rfc3339(t: SystemTime) -> String {
    OffsetDateTime::from(t)
        .format(&Rfc3339)
        .unwrap_or_else(|_| format!("{t:?}"))
}

/// The state a heartbeat file implies: no heartbeat at all is [`ServiceState::NotRunning`] (nothing
/// even claims to be running), otherwise whatever [`probe_heartbeat`] concluded.
pub fn state(hb: Option<&Heartbeat>) -> ServiceState {
    match hb {
        None => ServiceState::NotRunning,
        Some(hb) => probe_heartbeat(hb),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> Heartbeat {
        Heartbeat {
            pid: 4242,
            started_at: "2026-08-25T10:00:00Z".into(),
            last_pass_at: "2026-08-25T10:05:00Z".into(),
            program: Some("/opt/nxs/bin/nxs".into()),
            version: Some("0.72.0".into()),
            instance: None,
            write_failures: None,
            workspaces: vec![WorkspaceHealth {
                path: "/proj/a".into(),
                last_ok: Some("2026-08-25T10:05:00Z".into()),
                last_error: None,
                pushed: 3,
                pulled: 4,
            }],
        }
    }

    #[test]
    fn the_heartbeat_round_trips_and_an_absent_one_reads_as_not_running() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sync-daemon.json");
        assert_eq!(read_from(&path).unwrap(), None);
        assert_eq!(state(None), ServiceState::NotRunning);
        write_to(&path, &sample()).unwrap();
        assert_eq!(read_from(&path).unwrap(), Some(sample()));
    }

    #[test]
    fn a_heartbeat_written_before_the_write_failure_field_existed_still_reads() {
        // Every machine with a service has one of these on disk. `status` must read it, not refuse
        // it — the same contract `program`/`version`/`instance` each arrived under.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sync-daemon.json");
        std::fs::write(
            &path,
            r#"{"pid":7,"started_at":"2026-08-31T15:00:00Z","last_pass_at":"2026-08-31T15:05:00Z",
               "workspaces":[]}"#,
        )
        .unwrap();
        let hb = read_from(&path).unwrap().expect("it parses");
        assert_eq!(hb.write_failures, None, "and says nothing it does not know");
    }

    #[test]
    fn a_failed_write_is_carried_to_the_next_one_that_succeeds() {
        // nxf 6j6v.kcan: the measured episode. While the disk is full nothing can be written at
        // all, so what a reader is owed is the explanation afterwards — that the gap they see had
        // a cause, and which one.
        let mut log = WriteFailureLog::default();
        assert_eq!(
            log.snapshot(),
            None,
            "a service that never missed says nothing"
        );

        log.record(
            "2026-08-31T15:06:00Z".into(),
            "writing /Users/u/.nexusflow/sync-daemon.json: No space left on device (os error 28)"
                .into(),
        );
        log.record(
            "2026-08-31T15:11:00Z".into(),
            "writing /Users/u/.nexusflow/sync-daemon.json: No space left on device (os error 28)"
                .into(),
        );
        let got = log.snapshot().expect("two failures are worth reporting");
        assert_eq!(got.count, 2);
        assert_eq!(got.last_at, "2026-08-31T15:11:00Z");
        assert!(got.last_error.contains("No space left on device"));
    }

    #[test]
    fn the_tally_survives_the_write_that_publishes_it() {
        // Clearing on success would erase the explanation one pass after it appeared, and a reader
        // looking at an unexplained gap arrives later than that.
        let mut log = WriteFailureLog::default();
        log.record("2026-08-31T15:06:00Z".into(), "disk full".into());
        let published = log.snapshot();
        assert_eq!(log.snapshot(), published, "asking twice reads the same");
        assert_eq!(published.unwrap().count, 1);
    }

    #[test]
    fn a_heartbeat_carrying_a_write_failure_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sync-daemon.json");
        let mut hb = sample();
        hb.write_failures = Some(WriteFailures {
            count: 2,
            last_at: "2026-08-31T15:11:00Z".into(),
            last_error: "No space left on device (os error 28)".into(),
        });
        write_to(&path, &hb).unwrap();
        assert_eq!(read_from(&path).unwrap(), Some(hb));
    }

    #[test]
    fn an_empty_heartbeat_file_reads_as_absent_not_as_a_parse_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sync-daemon.json");
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_from(&path).unwrap(), None);
    }

    #[test]
    fn a_malformed_heartbeat_is_a_loud_validation_error_naming_the_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sync-daemon.json");
        std::fs::write(&path, "{not json").unwrap();
        let err = read_from(&path).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(err.msg.contains("sync-daemon.json"), "{}", err.msg);
    }

    #[test]
    fn probe_pid_is_alive_for_this_very_process_and_dead_for_an_impossible_pid() {
        if cfg!(unix) {
            assert_eq!(probe_pid(std::process::id()), Liveness::Alive);
            // Above any plausible pid_max, and outside i32 entirely, so it cannot name a process.
            assert_eq!(probe_pid(u32::MAX), Liveness::Dead);
        } else {
            assert_eq!(probe_pid(std::process::id()), Liveness::Unknown);
        }
    }

    // ---- the pid is not the process (6j6v.0wvp) -------------------------------------------------

    #[test]
    fn this_process_has_a_start_time_and_it_is_in_the_recent_past() {
        let started = process_started_at(std::process::id());
        if cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "linux"
        )) {
            let started = started.expect("a shipped platform can read its own start time");
            let age = SystemTime::now()
                .duration_since(started)
                .expect("a process cannot have started in the future");
            assert!(
                age < Duration::from_secs(24 * 60 * 60),
                "a test process started within the day, not {age:?}"
            );
        } else {
            assert_eq!(started, None, "and a platform without the API says so");
        }
    }

    /// A `/proc/<pid>/stat` line, built the way the kernel builds one: pid, `(comm)`, then the
    /// remaining fields — of which `starttime` is the 22nd overall.
    fn stat_line(comm: &str, starttime: u64) -> String {
        // Fields 3..=21 are 19 values before `starttime`; their contents are irrelevant here, but
        // their COUNT is the whole point of the index.
        let filler: Vec<String> = (0..19).map(|i| i.to_string()).collect();
        format!("1234 ({comm}) {} {starttime} 0 0", filler.join(" "))
    }

    #[test]
    fn the_stat_parser_survives_a_comm_field_containing_a_space_and_a_paren() {
        // The documented trap: splitting the whole line, or cutting at the FIRST `)`, both land on
        // the wrong field here. A process can be given this name with one line of shell.
        assert_eq!(starttime_ticks(&stat_line("a) b", 987_654)), Some(987_654));
        assert_eq!(starttime_ticks(&stat_line("sleep", 42)), Some(42));
        assert_eq!(
            starttime_ticks(&stat_line("((( ) ) )", 7)),
            Some(7),
            "the LAST paren is what ends the name, however many it contains"
        );
    }

    #[test]
    fn a_stat_line_that_is_not_one_yields_nothing_rather_than_a_wrong_number() {
        assert_eq!(starttime_ticks(""), None);
        assert_eq!(starttime_ticks("1234 no-paren-here 0 0 0"), None);
        assert_eq!(starttime_ticks("1234 (sh) 1 2 3"), None, "truncated line");
        assert_eq!(
            starttime_ticks(&stat_line("sh", 0).replace(" 0 0", " x y")),
            None,
            "a non-numeric field is not a start time"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_service_records_the_start_of_its_process_not_the_instant_it_asks() {
        // nxf 6j6v.d43g, the writing half. A child process, then a measurable pause, then the
        // question: the answer must be the child's `execve`, which is now 300 ms in the past — not
        // the moment the question was asked, which is what a service stamping `now()` records and
        // what put 69 minutes between the two instants on the owner's machine.
        if !cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "linux"
        )) {
            return; // no start-time API; the fallback arm below is this platform's whole story
        }
        let before = SystemTime::now();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn a child to ask about");
        std::thread::sleep(Duration::from_millis(300));
        let stamp = recorded_start(child.id(), SystemTime::now());
        let _ = child.kill();
        let _ = child.wait();

        // `duration_since` rather than `expect`: Linux reconstructs this instant (see the
        // constant below), and an overshoot past `now` would panic on a message that blames the
        // process rather than the arithmetic (review of PR #421, Test Quality #4). Zero then fails
        // the assertion below on its own terms, which is what a reader needs to see.
        let age = SystemTime::now().duration_since(stamp).unwrap_or_default();
        assert!(
            age >= Duration::from_millis(200),
            "the child began before the pause, so its start is at least that old, not {age:?}"
        );
        // And it is that child's start rather than some other instant entirely — bounded from the
        // other side, with the room the PLATFORM needs rather than a guess about scheduling.
        //
        // Linux rebuilds this instant arithmetically: `/proc/<pid>/stat` counts clock ticks since
        // BOOT, and the boot instant comes from `/proc/stat`'s `btime`, which has whole-second
        // resolution — so the reconstruction can sit up to about a second either side of the real
        // `execve`. macOS reports the instant directly and needs none of this room. A bare
        // `stamp >= before` therefore passes on a Mac and fails on Linux, which is what it did
        // (CI on PR #421), and the code it was accusing was right both times.
        //
        // Production is unaffected by that second: [`born_together`] allows a stamp LATER than the
        // process start without bound, and gives an earlier one `START_TIME_SLACK` — sixty times
        // the room this reconstruction can be out by.
        const BOOT_RECONSTRUCTION_SLOP: Duration = Duration::from_secs(2);
        assert!(
            stamp + BOOT_RECONSTRUCTION_SLOP >= before,
            "the stamp is far older than the instant just before the child was spawned"
        );
    }

    #[test]
    fn a_pid_with_no_start_time_records_the_instant_it_was_given_instead() {
        // A platform with no start-time API, or a pid that names nothing: the service still has to
        // stamp something, and the instant of the write is the honest fallback — it is what every
        // service recorded before this, and the reading side allows a stamp later than the process
        // start precisely so the fallback cannot make a live service unreadable.
        let fallback = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert_eq!(recorded_start(u32::MAX, fallback), fallback);
    }

    #[test]
    fn a_pid_that_names_nothing_has_no_start_time() {
        assert_eq!(process_started_at(u32::MAX), None);
    }

    #[test]
    fn a_heartbeat_written_by_this_very_process_is_recognised_as_its_own() {
        let started = process_started_at(std::process::id());
        if started.is_none() {
            return; // a platform with no start-time API; the arm below covers it
        }
        let hb = Heartbeat {
            pid: std::process::id(),
            // What the service itself records: the instant it wrote its first heartbeat, which is
            // a few milliseconds after the process began.
            started_at: rfc3339(SystemTime::now()),
            last_pass_at: "2026-08-25T10:00:00Z".into(),
            workspaces: Vec::new(),
            program: None,
            version: None,
            instance: None,
            write_failures: None,
        };
        assert_eq!(probe_heartbeat(&hb), ServiceState::Running);
    }

    #[test]
    fn a_live_pid_whose_start_time_disagrees_is_not_reported_as_running() {
        // THE BUG, in one assertion: the service died, the system handed its pid to something else,
        // and `kill(pid, 0)` says yes forever. This process is alive and is emphatically not the one
        // that wrote a heartbeat claiming to have started in 2020.
        let hb = Heartbeat {
            pid: std::process::id(),
            started_at: "2020-01-01T00:00:00Z".into(),
            last_pass_at: "2020-01-01T00:00:00Z".into(),
            workspaces: Vec::new(),
            program: None,
            version: None,
            instance: None,
            write_failures: None,
        };
        assert_ne!(probe_heartbeat(&hb), ServiceState::Running);
    }

    #[test]
    fn status_no_longer_reports_a_reused_pid_as_running() {
        // The DoD, at the seam a human actually reads.
        let hb = Heartbeat {
            pid: std::process::id(),
            started_at: "2020-01-01T00:00:00Z".into(),
            last_pass_at: "2020-01-01T00:00:00Z".into(),
            workspaces: Vec::new(),
            program: None,
            version: None,
            instance: None,
            write_failures: None,
        };
        assert_ne!(
            state(Some(&hb)),
            ServiceState::Running,
            "a heartbeat whose pid now belongs to somebody else must never read as running"
        );
    }

    #[test]
    fn a_live_pid_that_did_not_write_this_heartbeat_is_neither_running_nor_gone() {
        // nxf 6j6v.kcan, and since 6j6v.d43g the ONE reading left: a pid handed on after a
        // long-dead service. A process HOLDS this pid — saying it is gone is a false map (somebody
        // reaches for a pid that is not theirs), and saying it is running is the older one. The
        // stamp here is EARLIER than this process's real start, which is the shape only a handover
        // produces.
        let hb = Heartbeat {
            pid: std::process::id(),
            started_at: "2020-01-01T00:00:00Z".into(),
            last_pass_at: "2020-01-01T00:00:00Z".into(),
            workspaces: Vec::new(),
            program: None,
            version: None,
            instance: None,
            write_failures: None,
        };
        let expected = if cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "linux"
        )) {
            ServiceState::Unconfirmed
        } else {
            // No start-time API at all: the identity question was never asked, so the honest
            // answer is the older "nothing was learned", not the sharper one.
            ServiceState::Unknown
        };
        assert_eq!(state(Some(&hb)), expected);
    }

    #[test]
    fn the_four_states_are_four_answers_on_the_wire() {
        // The `running` key keeps its meaning — a tri-state bool — and the new one is told apart
        // from the old `null` by name, not by guesswork.
        assert_eq!(ServiceState::Running.as_json(), Some(true));
        assert_eq!(ServiceState::NotRunning.as_json(), Some(false));
        assert_eq!(ServiceState::Unconfirmed.as_json(), None);
        assert_eq!(ServiceState::Unknown.as_json(), None);
        assert_eq!(ServiceState::Running.as_str(), "running");
        assert_eq!(ServiceState::NotRunning.as_str(), "not_running");
        assert_eq!(ServiceState::Unconfirmed.as_str(), "unconfirmed");
        assert_eq!(ServiceState::Unknown.as_str(), "unknown");
    }

    /// nxf 6j6v.d43g, at the probe: a service that hung before writing its first heartbeat.
    ///
    /// The stamp lies AFTER the process's own start — the one shape a handed-on pid can never
    /// produce, because the process that inherits a dead service's id must begin long after that
    /// service stamped itself. This process's real start is seconds ago, so the only way to place
    /// a stamp beyond the slack on the late side is to put it ahead of the clock; the measured
    /// case had all three instants in the past and is pinned exactly, on synthetic instants, by
    /// `a_start_recorded_long_after_the_process_began_is_still_the_same_process` below.
    #[test]
    fn a_service_that_stamped_itself_late_is_reported_as_running() {
        let hb = Heartbeat {
            pid: std::process::id(),
            started_at: rfc3339(SystemTime::now() + Duration::from_secs(2 * 60 * 60)),
            last_pass_at: rfc3339(SystemTime::now()),
            workspaces: Vec::new(),
            program: None,
            version: None,
            instance: None,
            write_failures: None,
        };
        let expected = if cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "linux"
        )) {
            ServiceState::Running
        } else {
            // No start-time API: the identity question was never asked at all.
            ServiceState::Unknown
        };
        assert_eq!(state(Some(&hb)), expected);
    }

    #[test]
    fn a_proven_dead_pid_never_reaches_the_start_time_question() {
        let hb = Heartbeat {
            pid: u32::MAX,
            started_at: rfc3339(SystemTime::now()),
            last_pass_at: "2026-08-25T10:00:00Z".into(),
            workspaces: Vec::new(),
            program: None,
            version: None,
            instance: None,
            write_failures: None,
        };
        let expected = if cfg!(unix) {
            ServiceState::NotRunning
        } else {
            ServiceState::Unknown
        };
        assert_eq!(probe_heartbeat(&hb), expected);
    }

    // ---- the comparison itself, without a process to point it at --------------------------------

    #[test]
    fn a_recorded_start_within_the_slack_is_the_same_process() {
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        // The service records the moment it wrote its first heartbeat — always a little after the
        // OS stamped `execve`.
        let recorded = rfc3339(os + Duration::from_millis(120));
        assert_eq!(born_together(&recorded, Some(os)), Some(true));
    }

    #[test]
    fn the_backward_slack_is_inclusive_at_its_own_boundary() {
        // The boundary itself, not just either side of it (review of PR #369–#373, Test Quality #4):
        // `<=` is what the comparison says, and a silent flip to `<` would pass both neighbours.
        //
        // Only the BACKWARD side has a boundary since nxf 6j6v.d43g — the forward side is
        // unbounded, and the test above says why.
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert_eq!(
            born_together(&rfc3339(os - START_TIME_SLACK), Some(os)),
            Some(true)
        );
    }

    #[test]
    fn a_start_recorded_absurdly_later_than_the_process_began_is_not_trusted() {
        // The gap the forward direction does NOT cover (review of PR #421, Integrity #1).
        //
        // The argument that a later stamp can only be the service itself assumes the wall clock
        // moved forward between the old service's death and the new process's birth. A backward
        // step — an NTP correction after a skewed boot, a VM resume, somebody fixing the clock by
        // hand — breaks that: the process holding the recycled id is stamped by the corrected,
        // LOWER clock, so it can read as earlier than a dead service's `started_at` even though it
        // began later in real time. Unbounded forward trust then reports the stranger as running,
        // which is worse than the `Unconfirmed` it replaced.
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let ten_days_late = rfc3339(os + Duration::from_secs(10 * 24 * 60 * 60));
        assert_eq!(
            born_together(&ten_days_late, Some(os)),
            Some(false),
            "a stamp days ahead of the process is a clock event, not a slow start"
        );
    }

    #[test]
    fn the_late_stamp_ceiling_is_inclusive_and_clears_the_measured_dialog() {
        // Both edges of the one bound this direction has, and the case it must keep admitting:
        // 69 minutes behind a macOS permission dialog (nxf 6j6v.d43g) sits far inside it.
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert_eq!(
            born_together(&rfc3339(os + LATE_STAMP_CEILING), Some(os)),
            Some(true),
            "the boundary itself is inside — a silent flip to `<` would pass both neighbours"
        );
        assert_eq!(
            born_together(
                &rfc3339(os + LATE_STAMP_CEILING + Duration::from_secs(1)),
                Some(os)
            ),
            Some(false)
        );
        assert_eq!(
            born_together(&rfc3339(os + Duration::from_secs(69 * 60 + 41)), Some(os)),
            Some(true),
            "and the measured dialog still reads as the service that stamped it"
        );
    }

    #[test]
    fn a_start_recorded_before_the_process_began_is_a_different_process() {
        // The 6j6v.0wvp shape, and the ONE direction that stays proof of a handed-on pid: the
        // process holding this id started AFTER the heartbeat claims the service did, which the
        // service that wrote it cannot have done.
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let earlier = rfc3339(os - START_TIME_SLACK - Duration::from_secs(1));
        assert_eq!(born_together(&earlier, Some(os)), Some(false));
        let long_dead = rfc3339(os - Duration::from_secs(30 * 24 * 60 * 60));
        assert_eq!(
            born_together(&long_dead, Some(os)),
            Some(false),
            "a month earlier is the reported case, and the gap's size changes nothing"
        );
    }

    #[test]
    fn a_start_recorded_long_after_the_process_began_is_still_the_same_process() {
        // nxf 6j6v.d43g, the measured shape: the operating system stamped `execve` at 07:33:46 and
        // the service stamped its first heartbeat at 08:43:27 — 69 minutes and 41 seconds later,
        // because the binary sat behind a macOS permission dialog with nobody there to click it.
        //
        // No pid handover can produce this, which is why the lateness is allowed without bound: a
        // process inheriting a dead service's id must START after that service died, and the
        // service stamped its first heartbeat long before dying — so a stamp later than the
        // process's own start can only have been written by that very process.
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let recorded = rfc3339(os + Duration::from_secs(69 * 60 + 41));
        assert_eq!(born_together(&recorded, Some(os)), Some(true));
    }

    #[test]
    fn an_unreadable_stamp_or_a_platform_with_no_answer_yields_no_verdict() {
        let os = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert_eq!(born_together("next tuesday", Some(os)), None);
        assert_eq!(born_together(&rfc3339(os), None), None);
    }

    #[test]
    fn the_lock_treats_an_unprobeable_owner_as_still_holding_it() {
        assert!(lock_owner_is_alive(Liveness::Alive));
        assert!(lock_owner_is_alive(Liveness::Unknown));
        assert!(!lock_owner_is_alive(Liveness::Dead));
    }

    #[test]
    fn a_reader_reports_an_unprobeable_pid_as_unknown_rather_than_as_running() {
        assert_eq!(state_of(Liveness::Alive), ServiceState::Running);
        assert_eq!(state_of(Liveness::Dead), ServiceState::NotRunning);
        assert_eq!(state_of(Liveness::Unknown), ServiceState::Unknown);
        assert_eq!(ServiceState::Unknown.as_json(), None);
        assert_eq!(ServiceState::Running.as_json(), Some(true));
        assert_eq!(ServiceState::NotRunning.as_json(), Some(false));
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_a_pre_placed_temp_symlink_and_leaves_the_target_untouched() {
        let tmp = TempDir::new().unwrap();
        let victim = tmp.path().join("precious");
        std::fs::write(&victim, b"PRECIOUS").unwrap();
        let path = tmp.path().join("sync-daemon.json");
        let temp_path = tmp
            .path()
            .join(format!(".sync-daemon.json.nxs.tmp.{}", std::process::id()));
        std::os::unix::fs::symlink(&victim, &temp_path).unwrap();

        let err = write_to(&path, &sample()).unwrap_err();
        assert_eq!(err.kind.as_str(), "io");
        assert_eq!(std::fs::read(&victim).unwrap(), b"PRECIOUS");
        assert!(!path.exists());
    }
}

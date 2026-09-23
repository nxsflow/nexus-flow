//! The macOS background agent (6j6v.8see): `launchd` starts a nexus-flow service at login and keeps
//! it alive, and that is the only thing this repo ever asks `launchd` for. One agent per
//! [`Instance`] (6j6v.gd9p), which for a machine that runs only the production service is the one
//! agent it always was.
//!
//! # What it replaces
//!
//! Two launchd paths grew here. This one — a singleton for sync — and a second, one self-removing
//! agent PER DEADLINE (6j6v.74c0). The owner looked at the macOS background items on 2026-08-24 and
//! found what the second one produces: *"sh — an item from an unidentified developer"*, one entry
//! per open board, `/bin/sh -c` with the workspace path spliced into the command string and a
//! `PATH` pointing at somebody's `target/debug`. The decision was one word: **one**.
//!
//! So the deadlines moved into the service ([`crate::timers`]) and the second path is gone. What is
//! left is this agent, and three things about it changed with the merge:
//!
//! 1. **It is named.** The label is `com.nxsflow.nexus-flow`, and the program it runs is
//!    [`ServiceHome::program`] — a link named `nexus-flow` next to the service's own state. macOS
//!    takes the process name from the path it `exec`s, so the background item reads `nexus-flow`
//!    rather than `nxs` and certainly rather than `sh`. The link is the whole mechanism: this repo
//!    already ships one binary under four names (`nxf`/`nxm`/`nxc` are `argv[0]` links to `nxs`),
//!    so a fifth costs nothing and needs no packaging change.
//!
//!    Since 6j6v.gd9p there can be more than one, and BOTH halves of that name come out of an
//!    [`Instance`]: the label is [`Instance::label`] and the alias is [`Instance::name`], so a
//!    `nexus-flow-dev` agent runs a `nexus-flow-dev` link out of `~/.nexusflow-dev`. "One macOS
//!    background agent" is now one PER INSTANCE, and the singleton it replaced is the production
//!    one.
//! 2. **It boots out the old label on install.** A machine that has the previous agent installed
//!    would otherwise end up running both. See [`RETIRED_LABELS`].
//! 3. **Its `ProgramArguments` is one element.** No shell, no arguments: the alias name IS the
//!    instruction.
//!
//! # What it reads back, and what it refuses (6j6v.0yrp, 6j6v.kvda)
//!
//! Two failures on 2026-09-04 said the same thing about this module from opposite ends: it TALKED
//! to launchd and never LISTENED to it.
//!
//! * `install` issued one `bootstrap` and believed its exit code. Run straight after
//!   `self-update` — which is what `self-update`'s own message asks for — that arrives while
//!   launchd is still disposing of the previous job, and the answer is `Bootstrap failed: 5:
//!   Input/output error` with nothing else said. So the bootstrap is now retried a bounded number
//!   of times, the result is READ BACK with `launchctl print`, and a failure names the
//!   precondition this installer itself established rather than the errno. See [`install_with`],
//!   [`BOOTSTRAP_ATTEMPTS`] and [`failed_preconditions`].
//! * Nothing ever asked launchd what it was holding. It was holding, for the production label, a
//!   registration bootstrapped from a `TempDir` a test run had deleted five days earlier — so the
//!   real plist could not load, and `status` could only report a stale heartbeat. The comparison
//!   is [`registration_verdict`], and the reason that test could do it at all is
//!   [`login_session_complaint`], now a precondition of ever reaching `launchctl`.
//!
//! The one thing it DOES carry is a `PATH`, and that needs saying because complaint #4 against the
//! retired agents was a `PATH` — see [`service_path`].

use std::path::{Path, PathBuf};

use nxs_foundation::error::{NxfError, Result};

#[cfg(any(unix, test))]
use crate::home::ServiceHome;
use crate::instance::Instance;

/// Labels this repo has installed before and must now remove wherever it finds them.
///
/// Renaming a launchd label does not rename what is already bootstrapped: without this, installing
/// the new agent on a machine that has an old one leaves BOTH loaded, and the single-instance lock
/// then does its job by refusing one of them to start — every ten seconds, forever, into an
/// unrotated log. Booting out a label that is not loaded is the ordinary case, not an error, so
/// this costs one swallowed `launchctl` call per install.
///
/// - `com.nxsflow.nxs.sync` — the sync daemon's own label before it became the service (6j6v.8see).
/// - `com.nxsflow.nxc.tick.*` — the per-deadline agents (6j6v.74c0). NOT a fixed label: each
///   carried a thread id, so they are removed by PREFIX, see [`retired_tick_plists`].
///
/// **A SISTER INSTANCE IS NOT A PREDECESSOR** (6j6v.gd9p). Everything on this list was installed by
/// the single unnamed service that existed before instances did, so only the PRODUCTION instance
/// has any of it to clear — [`install_with`] and [`uninstall_with`] sweep it for production and for
/// nothing else. Adding an instance's own label here would make `install` of one service tear down
/// another, which is the exact failure the list exists to prevent, pointed the wrong way; the
/// guard against that is `no_retired_label_is_an_instance_label`.
pub const RETIRED_LABELS: &[&str] = &["com.nxsflow.nxs.sync"];

/// The prefix every retired per-deadline agent's label carried.
pub const RETIRED_TICK_PREFIX: &str = "com.nxsflow.nxc.tick.";

/// Render the LaunchAgent plist XML. Pure and deterministic (`format!` only — no plist crate; this
/// is ~20 lines and does not need one) so that install's idempotency actually holds: two renders of
/// the SAME inputs are byte-identical, never depending on the clock, an env var, or a HashMap's
/// iteration order.
///
/// `label` is the instance's ([`Instance::label`]). `program` is the absolute path launchd should
/// `exec` — the alias, whose BASENAME is both what the user sees in their background items and how
/// the started service learns which instance it is. `log_dir` is where its stdout/stderr land.
///
/// Both interpolated values are XML-escaped before splicing: neither is attacker-reachable
/// (`program` and `log_dir` are both derived from this crate's own `~/.nexusflow` resolution), but
/// an unescaped `&`/`<`/`>` in either — an unusual but legal path component — would render a
/// MALFORMED plist that launchd silently refuses to load, a fragile failure mode a one-line escape
/// avoids entirely.
pub fn render_plist(label: &str, program: &Path, log_dir: &Path, path_env: &str) -> String {
    let label = xml_escape(label);
    let program = xml_escape(&program.display().to_string());
    let log_dir = xml_escape(&log_dir.display().to_string());
    let path_env = xml_escape(path_env);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path_env}</string>
    </dict>
    <key>StandardOutPath</key>
    <string>{log_dir}/service.log</string>
    <key>StandardErrorPath</key>
    <string>{log_dir}/service.err.log</string>
</dict>
</plist>
"#,
    )
}

/// What launchd would give an agent on its own, and this keeps as a floor.
const SYSTEM_PATH: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// The `PATH` the agent hands the service — and, through it, everything a due deadline starts.
///
/// **The service itself does not need one:** launchd `exec`s it by absolute path, and it re-invokes
/// itself by [`std::env::current_exe`]. What needs a `PATH` is what a tick GOES ON to start. A tick
/// that decides a round is settled may start an agent session, and that runs `node` and refuses
/// outright when `claude` is not on `PATH`. On a stock Mac both live in `/opt/homebrew/bin`,
/// `/usr/local/bin` or an nvm directory — none of which is in launchd's own four. Without this the
/// unattended job would fire, decide correctly, and then fail to start anything, with the reason
/// only in a log. That is the same argument the retired per-deadline agent made for carrying the
/// submitting shell's `PATH`, and it did not stop being true when the agent went.
///
/// **What it does NOT carry is a build tree.** Complaint #4 in 6j6v.8see is exactly that: the old
/// agent's `PATH` began with somebody's `target/debug`, so a `cargo clean` or a moved checkout left
/// the job unable to find the binary it was supposed to run — and it told nobody. Every component
/// under `target/debug` or `target/release` is dropped here, which is the difference between
/// inheriting a developer's environment and inheriting a developer's *build*.
///
/// **Nor a RELATIVE component** (review of PR #369–#373, Integrity #3). A relative `PATH` entry —
/// `.`, `node_modules/.bin`, `bin` — resolves against the process's working directory, and a job's
/// working directory is a WORKSPACE, which can be a clone of anything. Harmless in the shell that
/// exported it, and a way to have a repository decide which `node` a background service runs once it
/// is baked into an agent that starts at every login.
///
/// **What it still inherits, stated rather than left implicit:** every absolute directory that was
/// on the installing user's `PATH`. That is deliberate — a curated list would not find the `node`
/// and `claude` this exists for — and it is the same trust the user already extends to their own
/// shell. What it costs is that the set is frozen until the next `nxs sync daemon install`: a
/// Homebrew or nvm move is not picked up on its own.
pub fn service_path(inherited: Option<&str>) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in inherited.unwrap_or_default().split(':') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // A build tree, in either profile, in any checkout. Matched as a path segment so a
        // directory legitimately CALLED `target` somewhere else is not swept up with it.
        if part.contains("/target/debug") || part.contains("/target/release") {
            continue;
        }
        // A relative entry resolves against the JOB's working directory, which is a workspace.
        if !Path::new(part).is_absolute() {
            continue;
        }
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    for floor in SYSTEM_PATH {
        if !parts.contains(floor) {
            parts.push(floor);
        }
    }
    parts.join(":")
}

/// Escape the three XML metacharacters in a plist string value. `&` MUST go first — escaping
/// `<`/`>` before `&` would re-escape the `&` those replacements themselves introduce.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One `launchctl` invocation's result, whatever its exit status — the shape [`LaunchCtl::output`]
/// hands back for the READ verbs (`print`, `print-disabled`), where a non-zero status is an answer
/// rather than a failure.
///
/// `launchctl print` exits 113 with *"Could not find service ... in domain for user"* when nothing
/// is loaded under the label, and that is the single most useful thing this module can learn from
/// it. Collapsing it into `Err` — the way [`LaunchCtl::run`] rightly does for the WRITE verbs —
/// would make "no job is loaded" indistinguishable from "launchctl is broken", which is exactly the
/// distinction 6j6v.kvda is about.
#[derive(Debug, Clone)]
pub struct CtlOutput {
    /// The exit status, or `None` when the process was killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CtlOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Whether this is `print`'s "there is no such job", as opposed to any other failure.
    ///
    /// Read from the TEXT rather than from the code alone: 113 is `launchctl`'s general
    /// "bad request", and the sentence is what separates "no such service" from a malformed
    /// domain. Either half on its own would over-claim.
    pub fn is_no_such_service(&self) -> bool {
        !self.success()
            && (self.stderr.contains("Could not find service")
                || self.stderr.contains("No such process"))
    }
}

/// Abstracts one `launchctl` invocation. The ONLY seam through which this module ever shells out —
/// tests substitute a spy so no test run ever touches the developer's or CI's real launchd session.
///
/// Since 6j6v.kvda the seam carries three things rather than one, because "talking to launchd" is
/// three things: issuing a verb, READING what it holds, and WAITING for it to finish. All three are
/// injected together so that the retry-and-verify loop 6j6v.0yrp asks for lives ABOVE this trait,
/// where a spy can drive it, rather than inside [`RealCtl`], where no test could reach it.
pub trait LaunchCtl {
    /// A write verb: `bootstrap`, `bootout`, `kickstart`. Non-zero is an error.
    fn run(&self, args: &[&str]) -> Result<()>;

    /// A read verb: `print`, `print-disabled`. Non-zero is an ANSWER — see [`CtlOutput`].
    fn output(&self, args: &[&str]) -> Result<CtlOutput>;

    /// Wait for launchd to finish tearing the previous job down.
    ///
    /// Part of the seam rather than a bare `thread::sleep` for one reason: a unit test that drives
    /// the retry loop must not actually sleep. The default is the real wait, so only a spy has to
    /// say anything.
    fn nap(&self, d: std::time::Duration) {
        std::thread::sleep(d);
    }
}

/// The real `launchctl`. Constructed only by the macOS-gated verbs at the bottom of this file —
/// never by a test.
///
/// Output is CAPTURED, not inherited. Every install issues bootouts it EXPECTS to fail — one per
/// retired label plus its own, so that a re-install succeeds instead of erroring "already
/// bootstrapped" — and an inheriting `launchctl` prints `Boot-out failed: 3: No such process` for
/// each of them, straight into the terminal of somebody who just typed
/// `nxs sync daemon install` and whose install is going perfectly. Both streams belong in the
/// error message of the call that actually fails, not in the caller's output.
pub struct RealCtl {
    /// Not constructible from outside: the only door is [`RealCtl::for_login_session`], and that
    /// door is the gate of 6j6v.kvda. A bare `RealCtl` literal would walk straight past it.
    _earned: (),
}

/// What [`RealCtl::for_login_session`] refuses, as a pure function of the two directories — so the
/// refusal is provable without redirecting anybody's `$HOME`.
///
/// **The whole rule in one sentence:** `launchctl bootstrap gui/<uid>` registers into the LOGIN
/// SESSION of uid, and that session's home is whatever the user database says — never whatever
/// `$HOME` says. When the two disagree, the caller is looking at one home and writing into the
/// other, and every path it hands launchd (the plist, the program, the log files) is a path the
/// session cannot resolve.
///
/// That is not a hypothetical. On 2026-08-30 a black-box test with a pinned `$HOME` ran the real
/// install; launchd took the registration into the owner's real `gui/501` with the test's TempDir
/// paths in it, the test then deleted the TempDir, and the PRODUCTION service could not start for
/// five days behind a registration nothing could see (6j6v.kvda). The memory
/// `launchd-install-is-not-home-isolated` had said so in words since; this is the same sentence as
/// a precondition.
///
/// Returns the complaint, or `None` when the two agree. Both sides are canonicalised by the caller
/// where possible, because `/tmp` and `/private/tmp` are the same directory and must not read as
/// two.
///
/// **What canonicalising CANNOT do is the reason [`HomeRule`] exists.** It resolves two SPELLINGS
/// of one directory; it cannot turn two genuinely different directories into one. A Mac whose
/// `$HOME` is legitimately redirected away from the passwd home — MDM, a roaming profile, DLP
/// tooling — is exactly that shape, and for its owner this rule is a false accusation: their paths
/// are real and persistent, not a `TempDir` about to vanish. That case is honest and unblockable
/// from here, so it is unblocked from the COMMAND LINE instead. See [`HomeRule`].
pub fn login_session_complaint(
    process_home: &Path,
    session_home: &Path,
    uid: u32,
) -> Option<String> {
    if process_home == session_home {
        return None;
    }
    Some(format!(
        "refusing to talk to launchd: this process's home is {} but the gui/{uid} domain belongs \
         to {}. A pinned $HOME isolates files and NOT launchd — a bootstrap from here would \
         register {}/Library/LaunchAgents into the real login session, where it outlives the \
         directory and blocks the real service from ever loading (nxf 6j6v.kvda: exactly that \
         stopped this machine's production clock for five days). If this is a test, it must not \
         run the real installer: drive `nxs_service::launchd::install_with` against a fake \
         `LaunchCtl` instead. If this machine's home is REDIRECTED on purpose — MDM, a roaming \
         profile, security tooling — then both paths above are real and permanent and this refusal \
         is wrong about you: pass `--allow-redirected-home` to install anyway.",
        process_home.display(),
        session_home.display(),
        process_home.display(),
    ))
}

/// Whether a process whose home is NOT the login session's may still talk to launchd.
///
/// **Why this is a type and not a `bool`.** The two values are a security decision and a support
/// decision respectively, and at a call site `true` says neither. Spelled out, each one names what
/// the caller is asserting.
///
/// **Why the escape is a FLAG and not an environment variable.** An env var is inherited — by every
/// subprocess, from every shell, out of every `.envrc` — which is the precise mechanism by which
/// the gate would rot back into the hole it fills: one `NXS_...=1` in a test harness and every test
/// on the machine is unprotected again, silently. A flag is typed once, per invocation, on one
/// command line; it cannot be inherited, and it is greppable in a source tree. The guard test
/// `no_test_names_the_redirected_home_escape` asserts that no test in this repo ever passes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeRule {
    /// Refuse unless this process's home IS the login session's. Every path that is not an
    /// explicit `--allow-redirected-home` uses this, the bind-time autostart included.
    MustBeTheLoginSessions,
    /// Proceed even when it is not — the operator has said, on the command line, that this
    /// machine's home is redirected on purpose and the paths are permanent.
    RedirectedIsAllowed,
}

impl RealCtl {
    /// The only way to obtain a `RealCtl` — and under [`HomeRule::MustBeTheLoginSessions`] it is
    /// refused unless this process's home IS the home of the login session whose `gui/<uid>`
    /// domain every verb below addresses.
    ///
    /// See [`login_session_complaint`] for the rule and the incident it comes from. Constructed
    /// before anything is written, so a refused install leaves no plist, no alias and no log
    /// directory behind either.
    ///
    /// The uid lookup is NOT skipped under [`HomeRule::RedirectedIsAllowed`], and that is
    /// deliberate: a machine whose user database cannot be read is not a machine anybody has
    /// decided anything about, and the flag says "my home is redirected", not "ask nothing".
    #[cfg(unix)]
    pub fn for_login_session(rule: HomeRule) -> Result<RealCtl> {
        let process_home = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .ok_or_else(|| {
                NxfError::io("could not resolve a home directory for the launchd agent")
            })?;
        let uid = current_uid();
        let session_home = login_session_home(uid).ok_or_else(|| {
            NxfError::io(format!(
                "could not read the home directory of uid {uid} from the user database, so \
                 nothing could check that this process's home is the login session's \
                 (nxf 6j6v.kvda)"
            ))
        })?;
        match login_session_complaint(&canonical(&process_home), &canonical(&session_home), uid) {
            None => Ok(RealCtl { _earned: () }),
            Some(_) if rule == HomeRule::RedirectedIsAllowed => Ok(RealCtl { _earned: () }),
            Some(complaint) => Err(NxfError::forbidden(complaint)),
        }
    }
}

/// `realpath`, or the path itself when it does not resolve. Two spellings of one directory
/// (`/tmp` and `/private/tmp`, a home reached through a symlinked volume) must not read as two
/// different homes and turn every install on such a machine into a refusal.
#[cfg(unix)]
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The home directory the USER DATABASE holds for `uid` — the one launchd's `gui/<uid>` domain
/// actually belongs to, which `$HOME` can be pointed away from and this cannot.
#[cfg(unix)]
fn login_session_home(uid: u32) -> Option<PathBuf> {
    // SAFETY: `getpwuid` returns a pointer into a static buffer owned by libc, valid until the next
    // call to it from this thread; the string is copied out before returning. A null return (no
    // such user) is checked.
    unsafe {
        let pw = libc::getpwuid(uid as libc::uid_t);
        if pw.is_null() {
            return None;
        }
        let dir = (*pw).pw_dir;
        if dir.is_null() {
            return None;
        }
        let bytes = std::ffi::CStr::from_ptr(dir).to_bytes();
        if bytes.is_empty() {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsString::from(
            <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(bytes),
        )))
    }
}

impl LaunchCtl for RealCtl {
    fn run(&self, args: &[&str]) -> Result<()> {
        let out = std::process::Command::new("launchctl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| NxfError::io(format!("running launchctl {args:?}: {e}")))?;
        if out.status.success() {
            return Ok(());
        }
        Err(NxfError::io(format!(
            "launchctl {args:?} exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }

    fn output(&self, args: &[&str]) -> Result<CtlOutput> {
        let out = std::process::Command::new("launchctl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| NxfError::io(format!("running launchctl {args:?}: {e}")))?;
        Ok(CtlOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

// ---- Reading what launchd actually holds (6j6v.0yrp, 6j6v.kvda) --------------------------------

/// The fields of one `launchctl print gui/<uid>/<label>` this repo has any use for.
///
/// **`path` is the whole point.** It is the plist launchd was BOOTSTRAPPED FROM, and it is not the
/// plist sitting in `~/Library/LaunchAgents` — those are two facts, and on 2026-09-04 they were two
/// different files, one of which had been deleted with a TempDir five days earlier (6j6v.kvda).
/// Everything else here is context for the sentence that reports the mismatch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedJob {
    /// The plist launchd loaded this job from.
    pub path: Option<PathBuf>,
    /// The executable it was told to run.
    pub program: Option<PathBuf>,
    /// `running`, `waiting`, `spawn scheduled`, …
    pub state: Option<String>,
    /// Verbatim, because launchd's own spelling of "never" (`(never exited)`) is not a number.
    pub last_exit_code: Option<String>,
}

/// Parse `launchctl print`'s output into the four fields above.
///
/// **Depth-aware on purpose.** The output nests: `arguments = { … }`, `environment = { … }`,
/// `probabilistic guard malloc policy = { … }`. A line-wise `starts_with("path =")` over the whole
/// text would read whatever a nested block happened to call `path`; only the TOP-LEVEL block —
/// depth 1, inside the opening `<domain>/<label> = {` — describes the job itself. The first value
/// at that depth wins, so a repeated key cannot silently overwrite the real one.
///
/// **Only a BLOCK BOUNDARY moves the depth** (review of PR #428, Integrity #2). The first cut
/// counted every `{` and `}` character on the line, which a brace inside a path VALUE desynchronises
/// for every field after it in the same block — and a lost `program` then defaults to "matches"
/// rather than to "unknown", which is a false reassurance rather than a visible failure. A block
/// only ever opens with a line ending `= {` and only ever closes with a line that is nothing but
/// `}`, so those two shapes are the only ones that count. A brace in a path is now just a
/// character in a path.
pub fn parse_print(out: &str) -> LoadedJob {
    let mut job = LoadedJob::default();
    let mut depth: usize = 0;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed == "}" {
            depth = depth.saturating_sub(1);
            continue;
        }
        // `<key> = {` opens a block — and is never a value, because launchctl renders a value on
        // the same line as its key and a nested block's brace is the last thing on that line.
        if trimmed.ends_with("= {") {
            depth += 1;
            continue;
        }
        if depth != 1 {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(" = ") else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "path" if job.path.is_none() => job.path = Some(PathBuf::from(value)),
            "program" if job.program.is_none() => job.program = Some(PathBuf::from(value)),
            "state" if job.state.is_none() => job.state = Some(value.to_string()),
            "last exit code" if job.last_exit_code.is_none() => {
                job.last_exit_code = Some(value.to_string())
            }
            _ => {}
        }
    }
    job
}

/// What launchd holds under one label, before it is compared to anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// launchd has no job under this label. After a failed install this is the TRUE answer, and
    /// the one a heartbeat left over from the previous process cannot give (6j6v.0yrp, point 3).
    NotLoaded,
    /// launchd holds a job, and this is what it says about it.
    Loaded(LoadedJob),
    /// Nobody looked, and the reason. NEVER collapsed into `NotLoaded`: "there is no job" and
    /// "the question could not be asked" are the two answers this whole ticket exists to separate.
    Unreadable(String),
}

/// `launchctl print gui/<uid>/<label>`, read through the seam.
pub fn read_registration(label: &str, ctl: &dyn LaunchCtl) -> Registration {
    let target = format!("gui/{}/{label}", current_uid());
    match ctl.output(&["print", &target]) {
        Err(e) => Registration::Unreadable(e.msg),
        Ok(out) if out.success() => Registration::Loaded(parse_print(&out.stdout)),
        Ok(out) if out.is_no_such_service() => Registration::NotLoaded,
        Ok(out) => Registration::Unreadable(format!(
            "launchctl print {target} exited with {}: {}",
            out.code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "a signal".to_string()),
            out.stderr.trim()
        )),
    }
}

/// The comparison 6j6v.kvda asks `status` for: what launchd holds, held against the plist THIS
/// instance owns and the program that plist names.
///
/// Four answers, and the reason there are four rather than a boolean is the whole ticket: "no job"
/// and "somebody else's job" and "nobody looked" were all rendering as the same silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationVerdict {
    /// Nothing is loaded under the label — the true answer after a failed install, and the one a
    /// heartbeat left over from the previous process cannot give (6j6v.0yrp, point 3).
    NotLoaded,
    /// The loaded job came from this instance's own plist. `program_exists` and `program_is_ours`
    /// are carried rather than folded in: the registration IS ours either way, and what it runs is
    /// a second fact the reader needs separately.
    Ours {
        /// The plist launchd holds — reported even here, where it matches, because "derivable" is
        /// exactly what this whole comparison stopped accepting as an answer.
        loaded: PathBuf,
        state: Option<String>,
        program: Option<PathBuf>,
        /// Whether the program launchd holds is the one this instance's plist names. `false` means
        /// launchd is running an older reading of the same plist.
        program_is_ours: bool,
        /// Whether that program exists on disk at all.
        program_exists: bool,
    },
    /// A job is loaded under this instance's label, from a DIFFERENT plist. This is the poisoning:
    /// the instance's own plist cannot load while this registration stands, and `bootstrap` on the
    /// label answers "already bootstrapped" rather than replacing it.
    Foreign {
        loaded: PathBuf,
        own: PathBuf,
        /// Whether the plist launchd loaded from still exists at all. `false` is the TempDir case
        /// that cost this machine five days.
        loaded_exists: bool,
        program: Option<PathBuf>,
        state: Option<String>,
        last_exit_code: Option<String>,
    },
    /// launchd said something this cannot read, or was not asked. See [`Registration::Unreadable`].
    /// NEVER collapsed into [`RegistrationVerdict::NotLoaded`].
    Unknown(String),
}

/// The pure half of the comparison: everything the filesystem had to say is already in the two
/// booleans, so every verdict above is provable without a launchd, a `$HOME`, or a real plist.
pub fn compare_registration(
    reg: &Registration,
    own_plist: &Path,
    own_program: &Path,
    loaded_plist_exists: bool,
    loaded_program_exists: bool,
) -> RegistrationVerdict {
    match reg {
        Registration::NotLoaded => RegistrationVerdict::NotLoaded,
        Registration::Unreadable(why) => RegistrationVerdict::Unknown(why.clone()),
        Registration::Loaded(job) => {
            // No `path` at all is not "ours": launchd told us about a job and did not say where it
            // came from, and claiming a match on that is exactly the over-claim this replaces.
            let Some(loaded) = job.path.as_deref() else {
                return RegistrationVerdict::Unknown(
                    "launchd holds a job under this label but did not report the plist it was \
                     bootstrapped from"
                        .to_string(),
                );
            };
            if !same_path(loaded, own_plist) {
                return RegistrationVerdict::Foreign {
                    loaded: loaded.to_path_buf(),
                    own: own_plist.to_path_buf(),
                    loaded_exists: loaded_plist_exists,
                    program: job.program.clone(),
                    state: job.state.clone(),
                    last_exit_code: job.last_exit_code.clone(),
                };
            }
            RegistrationVerdict::Ours {
                loaded: loaded.to_path_buf(),
                state: job.state.clone(),
                program_is_ours: job
                    .program
                    .as_deref()
                    .is_none_or(|p| same_path(p, own_program)),
                program_exists: loaded_program_exists,
                program: job.program.clone(),
            }
        }
    }
}

/// Compare two paths that came from two different places — one from launchd's mouth, one from this
/// process's own resolution — without calling a symlinked spelling of one file two files.
///
/// Literal equality first (it is the common case and needs no syscall); `canonicalize` only as a
/// second opinion, and only when both sides resolve. `/var` vs `/private/var` is the pair that
/// makes this necessary on macOS and it is not exotic: it is what `TempDir` hands out.
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// [`compare_registration`] with the two filesystem questions answered here — the thin impure shell
/// around the pure comparison.
pub fn registration_verdict(
    label: &str,
    own_plist: &Path,
    own_program: &Path,
    ctl: &dyn LaunchCtl,
) -> RegistrationVerdict {
    let reg = read_registration(label, ctl);
    let (plist_exists, program_exists) = match &reg {
        Registration::Loaded(job) => (
            job.path.as_deref().is_some_and(|p| p.exists()),
            job.program.as_deref().is_none_or(|p| p.exists()),
        ),
        _ => (false, true),
    };
    compare_registration(&reg, own_plist, own_program, plist_exists, program_exists)
}

/// The current user's numeric uid, used to address the `gui/$UID` launchd domain — the target of
/// every `bootstrap`/`bootout` this module issues. `libc` is only linked on unix, so this is gated
/// the same way; the fallback keeps the pure functions compiling (and tested) everywhere.
#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: `getuid` cannot fail and touches no memory the caller owns.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Create (or refresh) the alias at [`ServiceHome::program`], pointing at `target`.
///
/// This is what puts a NAME in the user's background items. macOS derives a process's displayed
/// name from the last component of the path handed to `exec`, not from the resolved binary — so a
/// link named `nexus-flow` is enough, with no separate binary, no app bundle, and no change to how
/// this repo is packaged. `nxs` already answers to four names this way.
///
/// Since 6j6v.gd9p that name is the INSTANCE's, and it carries a second job with it: it is the only
/// channel a launchd-started service has for learning which instance it is, because
/// [`render_plist`] writes exactly one `ProgramArguments` element and there is no flag to add.
///
/// Replaced rather than left alone when it already exists: after an upgrade that moves the install
/// directory, a stale link points at a binary that is gone, and launchd's only report of that is a
/// service that never starts.
#[cfg(unix)]
pub fn link_program(home: &ServiceHome, target: &Path) -> Result<PathBuf> {
    let link = home.program();
    let dir = home.bin();
    std::fs::create_dir_all(&dir)
        .map_err(|e| NxfError::io(format!("creating {}: {e}", dir.display())))?;
    // `symlink_metadata`, not `exists`: a link pointing at a deleted binary must still be replaced,
    // and `exists` follows the link and answers `false` for exactly that case.
    if std::fs::symlink_metadata(&link).is_ok() {
        std::fs::remove_file(&link).map_err(|e| {
            NxfError::io(format!(
                "replacing the existing {} (remove it by hand if it is not a file): {e}",
                link.display()
            ))
        })?;
    }
    std::os::unix::fs::symlink(target, &link).map_err(|e| {
        NxfError::io(format!(
            "linking {} -> {}: {e}",
            link.display(),
            target.display()
        ))
    })?;
    Ok(link)
}

/// The labels `instance` supersedes: [`RETIRED_LABELS`] for production, NOTHING for anybody else.
///
/// The whole rule in one function so both `install` and `uninstall` cannot answer it differently.
/// A named instance came into existence with 6j6v.gd9p and has never installed anything under
/// another label, so there is nothing of its own to clear — and everything on that list belongs to
/// the one service that existed before, which is the production one.
pub fn retired_labels_for(instance: &Instance) -> &'static [&'static str] {
    if instance.is_production() {
        RETIRED_LABELS
    } else {
        &[]
    }
}

/// Write the plist to `plist_path`, boot out every retired label (swallowed), then `bootout` this
/// one (swallowed) before handing over to [`bootstrap_and_verify`].
///
/// The `bootout` BEFORE `bootstrap` is what makes a re-install succeed instead of erroring "already
/// bootstrapped": nothing is loaded on a first install, so `bootout` failing is the expected, common
/// case there — its result is deliberately swallowed, unlike `bootstrap`'s own, which propagates.
///
/// **The whole retry-and-verify loop lives on THIS side of the `ctl` seam** (6j6v.0yrp, 6j6v.kvda),
/// which is the practical consequence of making the seam the gate: a loop inside [`RealCtl`] would
/// be a loop no test could drive. Everything here is provable against a spy.
pub fn install_with(
    instance: &Instance,
    plist_path: &Path,
    program: &Path,
    log_dir: &Path,
    path_env: &str,
    ctl: &dyn LaunchCtl,
) -> Result<Bootstrapped> {
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NxfError::io(format!("creating {}: {e}", parent.display())))?;
    }
    std::fs::create_dir_all(log_dir)
        .map_err(|e| NxfError::io(format!("creating {}: {e}", log_dir.display())))?;

    let label = instance.label();
    let xml = render_plist(&label, program, log_dir, path_env);
    // Every sibling writer under `~/.nexusflow` goes through `write_atomic`'s temp+`O_EXCL`+rename
    // so a symlink pre-placed at the target path is refused rather than followed; the plist was the
    // one left on plain `fs::write` until a review caught it.
    crate::atomic::write_atomic(plist_path, xml.as_bytes())?;

    let domain = format!("gui/{}", current_uid());
    // **Predecessors, and only for the instance that HAS any.** A sister instance's agent is not an
    // old version of this one — see [`RETIRED_LABELS`].
    for retired in retired_labels_for(instance) {
        let _ = ctl.run(&["bootout", &format!("{domain}/{retired}")]);
    }
    let _ = ctl.run(&["bootout", &format!("{domain}/{label}")]);
    bootstrap_and_verify(&label, &domain, plist_path, program, log_dir, ctl)
}

/// How many times `bootstrap` is attempted before the install gives up.
///
/// A SINGLE bootstrap at this point is inherently a race (6j6v.0yrp), and the build creates the
/// race itself: the message `nxs self-update` prints asks the user to run `install` immediately
/// afterwards, which is the one moment when the binary has just changed and launchd is still tearing
/// the old job down. A `bootstrap` that arrives during that teardown is answered
/// `Bootstrap failed: 5: Input/output error` — measured on the owner's machine on 2026-09-04, with
/// every other candidate cause ruled out one by one, and the same command succeeding unchanged a
/// little later.
pub const BOOTSTRAP_ATTEMPTS: u32 = 5;

/// How long to wait before attempt `attempt + 1`. Pure, so the schedule is provable without
/// waiting: 150ms, 300ms, 600ms, 1200ms — 2.25s in total across four waits, which is short enough
/// that a person does not notice it and long enough to outlast a teardown that took under a second
/// when it was measured.
pub fn bootstrap_wait(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(150u64 << attempt.saturating_sub(1).min(3))
}

/// What a completed `bootstrap` is worth — which is not the same as what it CLAIMED.
///
/// `launchctl bootstrap` exiting 0 says the request was accepted, not that the job this install
/// wrote is the job launchd now holds. Reading it back is one cheap `print` (6j6v.0yrp, point 1),
/// and the third arm exists because a read-back that could not be done must not be reported as one
/// that succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bootstrapped {
    /// Read back from launchd: the job under this label was loaded from the plist just written.
    Confirmed { state: Option<String> },
    /// The bootstrap succeeded and the read-back did not happen — with the reason. An install is
    /// still an install; what it is not is verified, and the receipt says which.
    Unconfirmed(String),
}

/// `bootstrap` with bounded retry, then read the registration back.
///
/// Between two attempts the label is booted out again: the case being retried is "launchd was still
/// disposing of the previous job", and arriving at a half-torn-down label a second time with no
/// teardown of our own is how a retry loop turns one race into five.
fn bootstrap_and_verify(
    label: &str,
    domain: &str,
    plist_path: &Path,
    program: &Path,
    log_dir: &Path,
    ctl: &dyn LaunchCtl,
) -> Result<Bootstrapped> {
    let plist_str = plist_path.to_string_lossy().into_owned();
    let mut last = String::new();
    for attempt in 1..=BOOTSTRAP_ATTEMPTS {
        match ctl.run(&["bootstrap", domain, &plist_str]) {
            Ok(()) => match read_back(label, plist_path, program, ctl) {
                RegistrationVerdict::Ours { state, .. } => {
                    return Ok(Bootstrapped::Confirmed { state })
                }
                // Nobody could look, twice. Failing an install that may well have worked is worse
                // than saying it could not be confirmed — and re-bootstrapping over a job that may
                // be perfectly fine is worse than both.
                RegistrationVerdict::Unknown(why) => return Ok(Bootstrapped::Unconfirmed(why)),
                other => last = describe_unverified(&other),
            },
            Err(e) => last = e.msg,
        }
        if attempt < BOOTSTRAP_ATTEMPTS {
            ctl.nap(bootstrap_wait(attempt));
            let _ = ctl.run(&["bootout", &format!("{domain}/{label}")]);
        }
    }
    Err(install_failure(
        &last,
        &gather_preconditions(label, plist_path, program, log_dir, ctl),
    ))
}

/// Read the registration back, and give a read that could not be performed ONE second chance
/// before concluding anything from it (review of PR #428, Integrity #3).
///
/// The bootstrap has just returned success; the only thing in doubt is whether anybody could LOOK.
/// A single flaky `launchctl print` — a fork that failed under memory pressure, a launchd busy for
/// a moment — would otherwise turn a perfectly good install into `Unconfirmed`, which is honest but
/// gives up one step earlier than the retry loop around it does for every other transient.
///
/// A second read and no more: an unreadable `print` is nearly always structural (no launchctl, a
/// domain this process cannot address), and retrying a structural failure five times only makes the
/// user wait. Only `Unknown` is retried — every other verdict is an ANSWER, and answers are for the
/// loop above to act on.
fn read_back(
    label: &str,
    plist_path: &Path,
    program: &Path,
    ctl: &dyn LaunchCtl,
) -> RegistrationVerdict {
    match registration_verdict(label, plist_path, program, ctl) {
        RegistrationVerdict::Unknown(_) => {
            ctl.nap(bootstrap_wait(1));
            registration_verdict(label, plist_path, program, ctl)
        }
        answered => answered,
    }
}

/// The one-line "the bootstrap returned, and this is what launchd actually holds" for the arms of
/// [`RegistrationVerdict`] that are not a success. Pure.
fn describe_unverified(verdict: &RegistrationVerdict) -> String {
    match verdict {
        RegistrationVerdict::NotLoaded => {
            "launchctl bootstrap reported success, but launchd holds no job under this label"
                .to_string()
        }
        RegistrationVerdict::Foreign { loaded, .. } => format!(
            "launchctl bootstrap reported success, but the job launchd holds under this label was \
             loaded from {} — a different plist, which is why this one cannot take its place",
            loaded.display()
        ),
        RegistrationVerdict::Ours { .. } => "the job is loaded from this plist".to_string(),
        RegistrationVerdict::Unknown(why) => why.clone(),
    }
}

// ---- Naming the precondition instead of the errno (6j6v.0yrp, point 2) -------------------------

/// Below this, "the disk is full" is worth saying. Above it, saying anything about free space is
/// noise: the 2026-09-04 failure was investigated by hand with 62 GB free, and a diagnosis that
/// lists every fact it checked is as unreadable as one that lists none.
const LOW_DISK_FLOOR: u64 = 64 * 1024 * 1024;

/// Every precondition `install` established ITSELF, moments before the bootstrap it is about to
/// explain — as facts, so the judgement about them is pure.
///
/// The list is the one the owner walked by hand on 2026-09-04 while `Input/output error` sat on the
/// screen: label already loaded, label disabled, plist malformed, program missing, log directory
/// missing, disk full. Every one of them was answerable by the installer, which had just written
/// most of them.
#[derive(Debug, Clone)]
pub struct Preconditions {
    pub plist: PathBuf,
    /// `None` when the plist could not be read back at all.
    pub plist_bytes: Option<u64>,
    /// Whether what was read back still looks like the plist this module renders.
    pub plist_is_a_plist: bool,
    pub program_link: PathBuf,
    /// What the alias resolves to, `None` when it dangles or is not a link.
    pub program_target: Option<PathBuf>,
    pub program_exists: bool,
    pub log_dir: PathBuf,
    pub log_dir_present: bool,
    /// Free space on the volume holding the plist, when it could be measured.
    pub free_bytes: Option<u64>,
    /// Whether launchd has this label on its disabled list. `None` when that could not be read.
    pub disabled: Option<bool>,
    /// What launchd holds under the label right now.
    pub loaded: RegistrationVerdict,
}

/// The preconditions that are FALSE, each as the sentence a reader can act on. Pure.
///
/// Empty means every one of them held — which is itself the most useful thing the diagnosis can
/// say, because it turns "look for yourself" into "everything this installer controls was right,
/// so the fault is on launchd's side".
pub fn failed_preconditions(p: &Preconditions) -> Vec<String> {
    let mut out = Vec::new();
    match p.plist_bytes {
        None => out.push(format!(
            "the plist this install just wrote cannot be read back at {}",
            p.plist.display()
        )),
        Some(0) => out.push(format!("the plist at {} is empty", p.plist.display())),
        Some(_) if !p.plist_is_a_plist => out.push(format!(
            "the file at {} is not the plist this install writes (launchd refuses a malformed one \
             without saying so)",
            p.plist.display()
        )),
        Some(_) => {}
    }
    match &p.program_target {
        None => out.push(format!(
            "the program alias {} does not resolve to anything",
            p.program_link.display()
        )),
        Some(t) if !p.program_exists => out.push(format!(
            "the program alias {} points at {}, which does not exist",
            p.program_link.display(),
            t.display()
        )),
        Some(_) => {}
    }
    if !p.log_dir_present {
        out.push(format!(
            "the log directory {} is missing — launchd refuses to start a job whose \
             StandardOutPath it cannot create",
            p.log_dir.display()
        ));
    }
    if let Some(free) = p.free_bytes {
        if free < LOW_DISK_FLOOR {
            out.push(format!(
                "the volume holding {} has {} MiB free",
                p.plist.display(),
                free / (1024 * 1024)
            ));
        }
    }
    if p.disabled == Some(true) {
        out.push(
            "launchd has this label on its disabled list — `launchctl enable gui/<uid>/<label>` \
             clears it"
                .to_string(),
        );
    }
    if let RegistrationVerdict::Foreign {
        loaded,
        loaded_exists,
        ..
    } = &p.loaded
    {
        out.push(format!(
            "launchd already holds this label, loaded from {}{} — that registration takes \
             precedence over the plist this install wrote, and only a `launchctl bootout` of it \
             releases the label",
            loaded.display(),
            if *loaded_exists {
                ""
            } else {
                ", which no longer exists"
            }
        ));
    }
    out
}

/// The error `install` fails with: the preconditions in this installer's own vocabulary first, the
/// raw `launchctl` sentence last, and the half-state named either way.
///
/// **Why the half-state is in here** (6j6v.0yrp, point 3): when this error is returned, the plist
/// IS written and the alias DOES point at the new binary, while no job is loaded. Whoever reads
/// this message is about to run `status`, and until they do, the heartbeat left behind by the
/// previous process is the only thing describing a world that has ended.
pub fn install_failure(raw: &str, p: &Preconditions) -> NxfError {
    let failed = failed_preconditions(p);
    let head = if failed.is_empty() {
        format!(
            "could not register the launchd agent, and every precondition this installer controls \
             held: the plist at {} is written and well-formed, the program alias resolves to a \
             file that exists, the log directory is there, the volume has room, and the label is \
             neither disabled nor held by another registration. What is left is launchd itself, \
             after {BOOTSTRAP_ATTEMPTS} attempts",
            p.plist.display()
        )
    } else {
        format!(
            "could not register the launchd agent after {BOOTSTRAP_ATTEMPTS} attempts, because:\n  \
             - {}",
            failed.join("\n  - ")
        )
    };
    NxfError::io(format!(
        "{head}\nlaunchctl's own last words: {raw}\nNothing is loaded, and the install stopped \
         half-way: the plist is written and the program alias points at this binary. \
         `nxs sync daemon status` reports the missing registration rather than the previous \
         process's heartbeat; re-running `nxs sync daemon install` is safe and idempotent."
    ))
}

/// Answer every question in [`Preconditions`] — the impure shell around [`failed_preconditions`].
fn gather_preconditions(
    label: &str,
    plist_path: &Path,
    program: &Path,
    log_dir: &Path,
    ctl: &dyn LaunchCtl,
) -> Preconditions {
    let plist_text = std::fs::read_to_string(plist_path).ok();
    let program_target = std::fs::read_link(program)
        .ok()
        .or_else(|| program.exists().then(|| program.to_path_buf()));
    Preconditions {
        plist: plist_path.to_path_buf(),
        plist_bytes: plist_text.as_ref().map(|t| t.len() as u64),
        plist_is_a_plist: plist_text.as_deref().is_some_and(|t| {
            t.contains("<plist") && t.contains(&format!("<string>{label}</string>"))
        }),
        program_link: program.to_path_buf(),
        program_exists: program_target.as_deref().is_some_and(|t| t.exists()),
        program_target,
        log_dir: log_dir.to_path_buf(),
        log_dir_present: log_dir.is_dir(),
        free_bytes: free_space(plist_path.parent().unwrap_or(plist_path)),
        disabled: read_disabled(label, ctl),
        loaded: registration_verdict(label, plist_path, program, ctl),
    }
}

/// Whether `launchctl print-disabled gui/<uid>` lists this label as disabled — the one precondition
/// in the list that only launchd knows. `None` when the list could not be read.
fn read_disabled(label: &str, ctl: &dyn LaunchCtl) -> Option<bool> {
    let domain = format!("gui/{}", current_uid());
    let out = ctl.output(&["print-disabled", &domain]).ok()?;
    if !out.success() {
        return None;
    }
    // The lines read `"com.nxsflow.nexus-flow" => disabled` (or `=> enabled`).
    let needle = format!("\"{label}\"");
    Some(
        out.stdout
            .lines()
            .filter(|l| l.contains(&needle))
            .any(|l| l.contains("disabled") && !l.contains("not disabled")),
    )
}

/// Free bytes on the volume holding `path`. `std` has no API for this; `statvfs` is the same
/// unix-only dependency this crate already carries for `getuid` and `kill`.
#[cfg(unix)]
fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` outlives the call and is NUL-terminated; `st` is fully written by a successful
    // `statvfs` and only read afterwards.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
}

#[cfg(not(unix))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// `bootout` the agent (swallowed — nothing loaded is the common "already uninstalled" case, not a
/// real error) and remove the plist if present. Uninstalling when nothing was ever installed is a
/// success, not an error.
pub fn uninstall_with(instance: &Instance, plist_path: &Path, ctl: &dyn LaunchCtl) -> Result<()> {
    let domain = format!("gui/{}", current_uid());
    let own = instance.label();
    let labels = std::iter::once(own.as_str()).chain(retired_labels_for(instance).iter().copied());
    for label in labels {
        let _ = ctl.run(&["bootout", &format!("{domain}/{label}")]);
    }
    if plist_path.exists() {
        std::fs::remove_file(plist_path)
            .map_err(|e| NxfError::io(format!("removing {}: {e}", plist_path.display())))?;
    }
    Ok(())
}

/// Every retired per-deadline plist in `agents_dir` — the `com.nxsflow.nxc.tick.<thread>.plist`
/// files 6j6v.74c0's clock left behind.
///
/// They removed themselves after firing, but only after firing: a board closed before its window
/// ended left its agent in place, and it reloads at every login. They name a verb that still exists
/// (`nxc tick`), so they are not harmless leftovers — they are a second clock running beside the
/// service, in the very directory this ticket is about clearing.
///
/// A pure function over a directory listing, so the sweep is testable without a real
/// `~/Library/LaunchAgents`. A directory that cannot be read yields nothing: the cleanup is a
/// courtesy on top of an install, never a reason to fail one.
pub fn retired_tick_plists(agents_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(agents_dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(RETIRED_TICK_PREFIX) && n.ends_with(".plist"))
        })
        .collect();
    found.sort();
    found
}

/// Boot out and delete every retired per-deadline agent in `agents_dir`. Best-effort throughout —
/// see [`retired_tick_plists`].
pub fn sweep_retired_ticks(agents_dir: &Path, ctl: &dyn LaunchCtl) -> usize {
    let domain = format!("gui/{}", current_uid());
    let mut removed = 0;
    for plist in retired_tick_plists(agents_dir) {
        if let Some(label) = plist.file_stem().and_then(|s| s.to_str()) {
            let _ = ctl.run(&["bootout", &format!("{domain}/{label}")]);
        }
        if std::fs::remove_file(&plist).is_ok() {
            removed += 1;
        }
    }
    removed
}

// ---- macOS verbs ------------------------------------------------------------------------------

/// `~/Library/LaunchAgents` — the per-user agent directory launchd loads at login.
#[cfg(target_os = "macos")]
pub fn agents_dir() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new()
        .ok_or_else(|| NxfError::io("could not resolve a home directory for the launchd agent"))?;
    Ok(dirs.home_dir().join("Library").join("LaunchAgents"))
}

/// `~/Library/LaunchAgents/com.nxsflow.nexus-flow.plist` — the real plist path of one instance.
#[cfg(target_os = "macos")]
pub fn plist_path_for(instance: &Instance) -> Result<PathBuf> {
    Ok(agents_dir()?.join(format!("{}.plist", instance.label())))
}

/// [`plist_path_for`] against THIS process's instance.
#[cfg(target_os = "macos")]
pub fn plist_path() -> Result<PathBuf> {
    plist_path_for(&Instance::ambient()?)
}

/// Install (or re-install, idempotently) `home`'s instance as a launchd agent for the CURRENT
/// binary, returning the plist it wrote. Also refreshes that instance's alias and — for production
/// only — sweeps away the retired per-deadline agents.
#[cfg(target_os = "macos")]
pub fn install_for(home: &ServiceHome, rule: HomeRule) -> Result<Installed> {
    // **FIRST, before a single file is written** (6j6v.kvda): a process whose home is not the login
    // session's may not touch launchd at all, and a refused install must leave no plist, no alias
    // and no log directory behind either.
    let ctl = RealCtl::for_login_session(rule)?;
    let exe = std::env::current_exe()
        .map_err(|e| NxfError::io(format!("resolving the current executable: {e}")))?;
    let program = link_program(home, &exe)?;
    let plist = plist_path_for(home.instance())?;
    let path_env = service_path(std::env::var("PATH").ok().as_deref());
    let bootstrapped = install_with(
        home.instance(),
        &plist,
        &program,
        &home.logs(),
        &path_env,
        &ctl,
    )?;
    // The per-deadline agents are the production service's litter and nobody else's, exactly like
    // [`RETIRED_LABELS`] — a sister instance sweeping `~/Library/LaunchAgents` would be one service
    // deleting plists it did not write.
    if home.instance().is_production() {
        sweep_retired_ticks(&agents_dir()?, &ctl);
    }
    Ok(Installed {
        plist,
        bootstrapped,
    })
}

/// What a completed `install` has to report: where the plist went, and whether launchd was read
/// back to confirm it took (6j6v.0yrp).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct Installed {
    pub plist: PathBuf,
    pub bootstrapped: Bootstrapped,
}

/// [`install_for`] against THIS process's instance.
#[cfg(target_os = "macos")]
pub fn install() -> Result<Installed> {
    install_for(&ServiceHome::resolve()?, HomeRule::MustBeTheLoginSessions)
}

/// Stop and remove `instance`'s launchd agent (and, for production, the retired ones beside it).
/// Not an error if it was never installed.
#[cfg(target_os = "macos")]
pub fn uninstall_for(instance: &Instance, rule: HomeRule) -> Result<()> {
    // Same gate as `install_for`, and for the sharper half of the same reason: a `bootout` issued
    // from a pinned `$HOME` does not boot out a test's job — there is none — it boots out the
    // REAL one, and stops the production clock (6j6v.kvda).
    let ctl = RealCtl::for_login_session(rule)?;
    uninstall_with(instance, &plist_path_for(instance)?, &ctl)?;
    if instance.is_production() {
        sweep_retired_ticks(&agents_dir()?, &ctl);
    }
    Ok(())
}

/// [`uninstall_for`] against THIS process's instance.
#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    uninstall_for(&Instance::ambient()?, HomeRule::MustBeTheLoginSessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// A launchd small enough to fit in a test: a label-to-plist map, the four verbs that move it,
    /// and `print` answering FROM it.
    ///
    /// It became a state machine with 6j6v.0yrp. A spy that answered every call `Ok(())` could not
    /// tell a bootstrap that took from one that did not, which is precisely the distinction the
    /// retry-and-verify loop is built out of — so the loop would have been unprovable against it.
    #[derive(Default)]
    struct SpyCtl {
        calls: Mutex<Vec<Vec<String>>>,
        /// label -> the plist it was bootstrapped from. Seeded to model a machine that already
        /// holds a registration (the 6j6v.kvda poisoning) and mutated by the verbs.
        loaded: Mutex<Vec<(String, String)>>,
        /// Every wait the loop asked for, recorded rather than slept.
        naps: Mutex<Vec<std::time::Duration>>,
        /// Refuse EVERY bootstrap.
        fail_bootstrap: bool,
        /// Refuse the first N bootstraps with launchd's own teardown answer, then take.
        bootstrap_failures: Mutex<u32>,
        /// Answer `print` as if the label were unreadable, whatever is loaded.
        print_unreadable: bool,
        /// Refuse the first N `print` calls the way a fork that failed under memory pressure would,
        /// then answer normally — the transient the read-back's one retry exists for.
        print_failures: Mutex<u32>,
        /// A launchd that ACCEPTS every bootstrap and loads nothing — the shape a verification
        /// exists to catch: exit 0 is a receipt for the request, not for the job.
        bootstrap_lies: bool,
        /// A registration `bootout` cannot shift (another domain, a job in transition). Without
        /// this, every foreign registration in a test is cured by the install's own bootout, and
        /// the verification never gets anything to catch.
        sticky: bool,
        /// Labels `print-disabled` reports as disabled. Everything else it lists reads `=> enabled`
        /// — see [`SpyCtl::enabled`], because "the label appears in the list" is NOT the question.
        disabled: Vec<String>,
        /// Labels `print-disabled` lists as ENABLED. launchd lists both, so a check that only
        /// looked for the label's presence would report every service on the machine as disabled.
        enabled: Vec<String>,
    }

    impl SpyCtl {
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
        fn verbs(&self) -> Vec<String> {
            self.calls().into_iter().map(|c| c.join(" ")).collect()
        }
        fn naps(&self) -> Vec<std::time::Duration> {
            self.naps.lock().unwrap().clone()
        }
        /// Pre-load a label from a plist nobody wrote — the shape a test run left in the owner's
        /// real session on 2026-08-30.
        fn poisoned(label: &str, plist: &str) -> SpyCtl {
            SpyCtl {
                loaded: Mutex::new(vec![(label.to_string(), plist.to_string())]),
                ..SpyCtl::default()
            }
        }
        fn plist_of(&self, label: &str) -> Option<String> {
            self.loaded
                .lock()
                .unwrap()
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, p)| p.clone())
        }
        /// `gui/501/com.nxsflow.x` -> `com.nxsflow.x`
        fn label_of(target: &str) -> String {
            target.rsplit('/').next().unwrap_or(target).to_string()
        }
    }

    impl LaunchCtl for SpyCtl {
        fn run(&self, args: &[&str]) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|a| a.to_string()).collect());
            match args {
                ["bootstrap", _domain, plist] => {
                    if self.fail_bootstrap {
                        return Err(NxfError::io("stub: bootstrap refused"));
                    }
                    if self.bootstrap_lies {
                        return Ok(());
                    }
                    let mut left = self.bootstrap_failures.lock().unwrap();
                    if *left > 0 {
                        *left -= 1;
                        return Err(NxfError::io(
                            "launchctl [\"bootstrap\"] exited with exit status: 5: Bootstrap \
                             failed: 5: Input/output error",
                        ));
                    }
                    let label = Path::new(plist)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let mut loaded = self.loaded.lock().unwrap();
                    if loaded.iter().any(|(l, _)| *l == label) {
                        return Err(NxfError::io(
                            "Bootstrap failed: 37: Operation already in \
                                                 progress (already bootstrapped)",
                        ));
                    }
                    loaded.push((label, plist.to_string()));
                    Ok(())
                }
                ["bootout", target] => {
                    if self.sticky {
                        return Err(NxfError::io(
                            "Boot-out failed: 36: Operation now in progress",
                        ));
                    }
                    let label = SpyCtl::label_of(target);
                    let mut loaded = self.loaded.lock().unwrap();
                    let before = loaded.len();
                    loaded.retain(|(l, _)| *l != label);
                    if loaded.len() == before {
                        return Err(NxfError::io("Boot-out failed: 3: No such process"));
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        }

        fn output(&self, args: &[&str]) -> Result<CtlOutput> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|a| a.to_string()).collect());
            match args {
                ["print", target] => {
                    let label = SpyCtl::label_of(target);
                    let mut left = self.print_failures.lock().unwrap();
                    if *left > 0 {
                        *left -= 1;
                        return Ok(CtlOutput {
                            code: Some(1),
                            stdout: String::new(),
                            stderr: "launchctl: fork failed".to_string(),
                        });
                    }
                    drop(left);
                    if self.print_unreadable {
                        return Ok(CtlOutput {
                            code: Some(113),
                            stdout: String::new(),
                            stderr: "Bad request.\nUnknown domain.".to_string(),
                        });
                    }
                    match self.plist_of(&label) {
                        None => Ok(CtlOutput {
                            code: Some(113),
                            stdout: String::new(),
                            stderr: format!(
                                "Bad request.\nCould not find service \"{label}\" in domain for \
                                 user gui: 501"
                            ),
                        }),
                        Some(plist) => Ok(CtlOutput {
                            code: Some(0),
                            stdout: format!(
                                "{target} = {{\n\tactive count = 1\n\tpath = {plist}\n\ttype = \
                                 LaunchAgent\n\tstate = running\n\n\tprogram = /p/nexus-flow\n\
                                 \targuments = {{\n\t\t/p/nexus-flow\n\t}}\n}}\n"
                            ),
                            stderr: String::new(),
                        }),
                    }
                }
                ["print-disabled", _domain] => {
                    let mut stdout = String::from("\tdisabled services = {\n");
                    for l in &self.disabled {
                        stdout.push_str(&format!("\t\t\"{l}\" => disabled\n"));
                    }
                    for l in &self.enabled {
                        stdout.push_str(&format!("\t\t\"{l}\" => enabled\n"));
                    }
                    stdout.push_str("\t}\n");
                    Ok(CtlOutput {
                        code: Some(0),
                        stdout,
                        stderr: String::new(),
                    })
                }
                _ => Ok(CtlOutput {
                    code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                }),
            }
        }

        /// Recorded, never slept: a unit test that drove the real schedule would take 2.25 seconds
        /// per failing install.
        fn nap(&self, d: std::time::Duration) {
            self.naps.lock().unwrap().push(d);
        }
    }

    fn dev() -> Instance {
        Instance::named("nexus-flow-dev").expect("a legal instance name")
    }

    #[test]
    fn the_plist_names_the_service_and_runs_the_alias_with_no_shell() {
        let plist = render_plist(
            &Instance::production().label(),
            Path::new("/home/u/.nexusflow/bin/nexus-flow"),
            Path::new("/home/u/.nexusflow/logs"),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        );
        assert!(plist.contains("<string>com.nxsflow.nexus-flow</string>"));
        assert!(plist.contains("<string>/home/u/.nexusflow/bin/nexus-flow</string>"));
        assert!(
            !plist.contains("/bin/sh"),
            "no shell may appear in the agent this ticket exists to name: {plist}"
        );
        // ONE ProgramArguments element: the alias name is the whole instruction.
        let args = plist
            .split("<key>ProgramArguments</key>")
            .nth(1)
            .and_then(|s| s.split("</array>").next())
            .unwrap();
        assert_eq!(args.matches("<string>").count(), 1, "{args}");
    }

    /// The two conditions the owner attached to the service on 2026-08-25, as the plist states
    /// them. Both are one key, and one key is exactly the kind of thing that goes missing in a
    /// refactor without anything failing to compile.
    #[test]
    fn the_agent_starts_at_every_login_and_comes_back_from_a_crash() {
        let plist = render_plist(
            &Instance::production().label(),
            Path::new("/p/nexus-flow"),
            Path::new("/l"),
            "/usr/bin",
        );
        // `RunAtLoad`: launchd loads every plist in `~/Library/LaunchAgents` at each login — the
        // first of which, after a reboot, is the boot. Deliberately a per-user AGENT and not a
        // system-wide DAEMON: this service works in the user's own home, reads the user's
        // workspace registry, and starts sessions as the user. A root daemon starting before
        // anybody has logged in would have none of those things.
        assert!(
            plist.contains("<key>RunAtLoad</key>\n    <true/>"),
            "the service must start on its own, or it is not a service: {plist}"
        );
        // `KeepAlive`: the owner's second condition, "sich nach einem Absturz selbst neu starten".
        // Verified live as well as here — a `kill -9` on the running service was followed by a new
        // pid within seconds.
        assert!(
            plist.contains("<key>KeepAlive</key>\n    <true/>"),
            "a crash must not be the end of the clock: {plist}"
        );
    }

    #[test]
    fn the_plist_renders_byte_identically_twice_so_reinstalling_is_idempotent() {
        let label = Instance::production().label();
        let a = render_plist(
            &label,
            Path::new("/p/nexus-flow"),
            Path::new("/l"),
            "/usr/bin",
        );
        let b = render_plist(
            &label,
            Path::new("/p/nexus-flow"),
            Path::new("/l"),
            "/usr/bin",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn render_plist_escapes_xml_metacharacters_in_both_interpolated_paths() {
        let plist = render_plist(
            &Instance::production().label(),
            Path::new("/home/u&v/nexus-flow"),
            Path::new("/l<og>"),
            "/opt/a&b/bin",
        );
        assert!(plist.contains("/home/u&amp;v/nexus-flow"));
        assert!(!plist.contains("/home/u&v/nexus-flow"));
        assert!(plist.contains("/l&lt;og&gt;/service.log"));
        assert!(plist.contains("/opt/a&amp;b/bin"));
        assert!(!plist.contains("<string>/opt/a&b/bin</string>"));
    }

    #[test]
    fn install_boots_out_the_retired_label_before_bootstrapping_the_new_one() {
        let tmp = TempDir::new().unwrap();
        let ctl = SpyCtl::default();
        install_with(
            &Instance::production(),
            &tmp.path()
                .join("agents")
                .join("com.nxsflow.nexus-flow.plist"),
            Path::new("/p/nexus-flow"),
            &tmp.path().join("logs"),
            "/usr/bin",
            &ctl,
        )
        .unwrap();
        let verbs = ctl.verbs();
        let retired = verbs
            .iter()
            .position(|c| c.contains("com.nxsflow.nxs.sync"))
            .expect("the previous label is booted out: {verbs:?}");
        let bootstrap = verbs
            .iter()
            .position(|c| c.starts_with("bootstrap"))
            .expect("the new agent is bootstrapped");
        assert!(
            retired < bootstrap,
            "the old agent must go before the new one arrives, or both run: {verbs:?}"
        );
    }

    #[test]
    fn installing_twice_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let plist = tmp
            .path()
            .join("agents")
            .join("com.nxsflow.nexus-flow.plist");
        let log = tmp.path().join("logs");
        let ctl = SpyCtl::default();
        let prod = Instance::production();
        install_with(
            &prod,
            &plist,
            Path::new("/p/nexus-flow"),
            &log,
            "/usr/bin",
            &ctl,
        )
        .unwrap();
        let first = std::fs::read_to_string(&plist).unwrap();
        install_with(
            &prod,
            &plist,
            Path::new("/p/nexus-flow"),
            &log,
            "/usr/bin",
            &ctl,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&plist).unwrap(), first);
    }

    #[test]
    fn a_failing_bootstrap_propagates_rather_than_reporting_a_service_that_is_not_there() {
        let tmp = TempDir::new().unwrap();
        let ctl = SpyCtl {
            fail_bootstrap: true,
            ..SpyCtl::default()
        };
        let err = install_with(
            &Instance::production(),
            &tmp.path().join("com.nxsflow.nexus-flow.plist"),
            Path::new("/p/nexus-flow"),
            &tmp.path().join("logs"),
            "/usr/bin",
            &ctl,
        )
        .unwrap_err();
        assert_eq!(err.kind.as_str(), "io");
    }

    #[test]
    fn uninstall_boots_out_the_new_label_and_every_retired_one_then_removes_the_plist() {
        let tmp = TempDir::new().unwrap();
        let plist = tmp.path().join("com.nxsflow.nexus-flow.plist");
        std::fs::write(&plist, "x").unwrap();
        let ctl = SpyCtl::default();
        uninstall_with(&Instance::production(), &plist, &ctl).unwrap();
        let verbs = ctl.verbs();
        assert!(
            verbs.iter().any(|c| c.contains("com.nxsflow.nexus-flow")),
            "{verbs:?}"
        );
        assert!(
            verbs.iter().any(|c| c.contains("com.nxsflow.nxs.sync")),
            "{verbs:?}"
        );
        assert!(!plist.exists());
    }

    #[test]
    fn uninstalling_when_nothing_is_installed_is_not_an_error() {
        let tmp = TempDir::new().unwrap();
        uninstall_with(
            &Instance::production(),
            &tmp.path().join("absent.plist"),
            &SpyCtl::default(),
        )
        .unwrap();
    }

    #[test]
    fn the_retired_per_deadline_agents_are_found_by_prefix_and_nothing_else_is() {
        let tmp = TempDir::new().unwrap();
        for name in [
            "com.nxsflow.nxc.tick.m-01AAA.plist",
            "com.nxsflow.nxc.tick.m-01BBB.plist",
            "com.nxsflow.nexus-flow.plist",
            "com.nxsflow.aws-invoices.plist",
            "com.apple.something.plist",
            "com.nxsflow.nxc.tick.m-01CCC.txt",
        ] {
            std::fs::write(tmp.path().join(name), "x").unwrap();
        }
        let found: Vec<String> = retired_tick_plists(tmp.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            found,
            vec![
                "com.nxsflow.nxc.tick.m-01AAA.plist".to_string(),
                "com.nxsflow.nxc.tick.m-01BBB.plist".to_string()
            ],
            "the owner's own unrelated agents must never be touched"
        );
    }

    #[test]
    fn sweeping_removes_the_retired_agents_and_boots_each_of_them_out() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("com.nxsflow.nxc.tick.m-01AAA.plist"), "x").unwrap();
        std::fs::write(tmp.path().join("com.nxsflow.aws-invoices.plist"), "x").unwrap();
        let ctl = SpyCtl::default();
        assert_eq!(sweep_retired_ticks(tmp.path(), &ctl), 1);
        assert!(ctl
            .verbs()
            .iter()
            .any(|c| c.contains("com.nxsflow.nxc.tick.m-01AAA")));
        assert!(tmp.path().join("com.nxsflow.aws-invoices.plist").exists());
    }

    #[test]
    fn sweeping_a_directory_that_is_not_there_is_a_quiet_zero() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            sweep_retired_ticks(&tmp.path().join("nope"), &SpyCtl::default()),
            0
        );
    }

    #[test]
    fn the_agents_path_keeps_what_a_tick_needs_to_start_and_drops_the_build_tree() {
        let got = service_path(Some(
            "/Users/x/Development/nexus-flow/target/debug:/opt/homebrew/bin:/usr/bin",
        ));
        assert!(
            !got.contains("target/debug"),
            "an agent bound to a build tree is complaint #4 of 6j6v.8see: {got}"
        );
        assert!(
            got.starts_with("/opt/homebrew/bin"),
            "what a tick has to START (node, claude) lives there on a stock Mac: {got}"
        );
        for floor in SYSTEM_PATH {
            assert!(got.contains(floor), "{floor} missing from {got}");
        }
    }

    #[test]
    fn a_release_build_tree_is_dropped_too_and_an_absent_path_still_yields_the_floor() {
        assert!(!service_path(Some("/w/target/release:/usr/local/bin")).contains("target/release"));
        assert_eq!(service_path(None), SYSTEM_PATH.join(":"));
        assert_eq!(service_path(Some("")), SYSTEM_PATH.join(":"));
    }

    #[test]
    fn a_relative_path_component_never_reaches_the_agent() {
        // A job's working directory is a WORKSPACE, so a relative entry lets whatever was cloned
        // there decide which `node` a login-started service runs.
        let got = service_path(Some(".:node_modules/.bin:/opt/homebrew/bin"));
        for relative in [".", "node_modules/.bin"] {
            assert!(
                !got.split(':').any(|p| p == relative),
                "`{relative}` survived into {got}"
            );
        }
        assert!(got.contains("/opt/homebrew/bin"), "{got}");
    }

    #[test]
    fn the_agents_path_lists_each_directory_once() {
        let got = service_path(Some("/usr/bin:/opt/b:/usr/bin:/opt/b"));
        assert_eq!(got.matches("/opt/b").count(), 1, "{got}");
        assert_eq!(got.matches("/usr/bin").count(), 1, "{got}");
    }

    #[cfg(unix)]
    #[test]
    fn the_alias_is_named_after_its_instance_and_points_at_the_binary_it_was_given() {
        let tmp = TempDir::new().unwrap();
        let home = ServiceHome::at(tmp.path().join(".nexusflow"));
        let target = tmp.path().join("bin").join("nxs");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "#!/bin/sh\n").unwrap();

        let link = link_program(&home, &target).unwrap();
        assert_eq!(link.file_name().unwrap(), "nexus-flow");
        assert_eq!(std::fs::read_link(&link).unwrap(), target);

        // And a named instance's alias carries ITS name — which is not decoration: it is the only
        // thing the started service has to read its own instance out of.
        let dev_home = ServiceHome::at_instance(tmp.path().join(".nexusflow-dev"), dev());
        let dev_link = link_program(&dev_home, &target).unwrap();
        assert_eq!(dev_link.file_name().unwrap(), "nexus-flow-dev");
        assert_ne!(dev_link, link, "two instances, two aliases");
    }

    /// **The constraint that decides the design** (nxf 6j6v.gd9p), with its own name and its own
    /// test.
    ///
    /// `ProgramArguments` carries exactly one element — the alias path — for every instance. That
    /// is why a running service reads its instance off `argv[0]`: there is nowhere to put a flag,
    /// and the last component of that one element is the whole instruction. The assertion also
    /// lives inside `the_plist_names_the_service_and_runs_the_alias_with_no_shell`, but that test's
    /// NAME claims something else, so loosening the constraint could pass a review as an unrelated
    /// edit to a shell test. Here it fails by the name of the thing it breaks.
    #[test]
    fn the_program_arguments_stay_one_element_so_the_alias_name_is_the_whole_instruction() {
        for instance in [Instance::production(), dev()] {
            let alias = Path::new("/h")
                .join(instance.home_dir())
                .join("bin")
                .join(instance.name());
            let plist = render_plist(&instance.label(), &alias, Path::new("/l"), "/usr/bin");
            let args = plist
                .split("<key>ProgramArguments</key>")
                .nth(1)
                .and_then(|s| s.split("</array>").next())
                .expect("the plist has a ProgramArguments array");
            assert_eq!(
                args.matches("<string>").count(),
                1,
                "a second element would give the service a channel it must not have, and the \
                 alias name would stop being load-bearing: {args}"
            );
            assert!(
                args.contains(&format!("<string>{}</string>", alias.display())),
                "and the one element is the instance\'s own alias: {args}"
            );
            assert!(
                alias.file_name().unwrap() == instance.name().as_str(),
                "whose LAST COMPONENT is what the started service parses back into an instance"
            );
        }
    }

    /// The DoD's own sentence, as a run: installing one instance must not boot out another
    /// (6j6v.gd9p). Two launchd agents that tear each other down are the state `RETIRED_LABELS`
    /// documents — *"refusing one of them to start, every ten seconds, forever"* — arrived at from
    /// the other side.
    #[test]
    fn installing_one_instance_never_boots_out_a_sister() {
        let tmp = TempDir::new().unwrap();
        let ctl = SpyCtl::default();
        install_with(
            &dev(),
            &tmp.path().join("com.nxsflow.nexus-flow-dev.plist"),
            Path::new("/h/.nexusflow-dev/bin/nexus-flow-dev"),
            &tmp.path().join("logs"),
            "/usr/bin",
            &ctl,
        )
        .unwrap();

        let verbs = ctl.verbs();
        // Every label this install touched belongs to the instance being installed.
        for verb in &verbs {
            assert!(
                verb.contains("com.nxsflow.nexus-flow-dev") || verb.starts_with("bootstrap"),
                "installing nexus-flow-dev reached for something that is not its own: {verb}"
            );
        }
        assert!(
            !verbs.iter().any(
                |v| v.contains("gui/0/com.nxsflow.nexus-flow") && !v.contains("nexus-flow-dev")
            ),
            "the production agent must survive a sister's install: {verbs:?}"
        );
        // …and nothing retired either, because a named instance has no predecessors.
        assert!(
            !verbs.iter().any(|v| v.contains("com.nxsflow.nxs.sync")),
            "a sister instance has nothing of the old singleton to clear: {verbs:?}"
        );
        assert!(
            verbs.iter().any(|v| v.starts_with("bootstrap")),
            "{verbs:?}"
        );
    }

    #[test]
    fn uninstalling_one_instance_never_boots_out_a_sister_either() {
        let tmp = TempDir::new().unwrap();
        let plist = tmp.path().join("com.nxsflow.nexus-flow-dev.plist");
        std::fs::write(&plist, "x").unwrap();
        let ctl = SpyCtl::default();
        uninstall_with(&dev(), &plist, &ctl).unwrap();
        for verb in ctl.verbs() {
            assert!(
                verb.contains("com.nxsflow.nexus-flow-dev"),
                "uninstalling nexus-flow-dev booted out {verb}"
            );
        }
        assert!(!plist.exists());
    }

    /// The structural half of the rule above, and the one that survives a refactor: nothing on the
    /// retired list may be a name an instance could ever install under. Without this, adding
    /// `com.nxsflow.nexus-flow` to `RETIRED_LABELS` — a plausible thing to do the day production is
    /// renamed — would silently turn every named instance's install into a teardown of the others,
    /// and the two tests above would still pass, because they only exercise the labels they name.
    #[test]
    fn no_retired_label_is_an_instance_label() {
        for retired in RETIRED_LABELS {
            let tail = retired.strip_prefix("com.nxsflow.").unwrap_or(retired);
            assert!(
                Instance::named(tail).is_err(),
                "`{retired}` names a service INSTANCE — booting it out on install is one service \
                 tearing down another, not clearing a predecessor"
            );
        }
        assert!(
            Instance::named(RETIRED_TICK_PREFIX.trim_start_matches("com.nxsflow.")).is_err(),
            "the retired per-deadline prefix must not be reachable as an instance name either"
        );
    }

    /// Only production has predecessors — the rule both `install` and `uninstall` read out of one
    /// function so they cannot answer it differently.
    #[test]
    fn retired_labels_belong_to_the_production_instance_alone() {
        assert_eq!(retired_labels_for(&Instance::production()), RETIRED_LABELS);
        assert!(retired_labels_for(&dev()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_alias_pointing_at_a_binary_that_is_gone_is_replaced_not_kept() {
        let tmp = TempDir::new().unwrap();
        let home = ServiceHome::at(tmp.path().join(".nexusflow"));
        let gone = tmp.path().join("old").join("nxs");
        link_program(&home, &gone).unwrap();
        assert!(
            !home.program().exists(),
            "the link resolves to nothing, which is exactly the case `exists` cannot see"
        );

        let current = tmp.path().join("new").join("nxs");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, "#!/bin/sh\n").unwrap();
        link_program(&home, &current).unwrap();
        assert_eq!(std::fs::read_link(home.program()).unwrap(), current);
    }

    // ---- Reading what launchd holds (6j6v.0yrp verification, 6j6v.kvda detection) -------------

    /// The poisoned registration, VERBATIM from `launchctl print gui/501/com.nxsflow.nexus-flow`
    /// on the owner's machine on 2026-09-04 — the reading that ended five days of a stopped
    /// production clock. Kept whole rather than trimmed to the four interesting lines: the nested
    /// blocks are exactly what a naive line-wise parser gets wrong.
    const POISONED_PRINT: &str = "\
gui/501/com.nxsflow.nexus-flow = {
\tactive count = 1
\tpath = /private/var/folders/1k/zp26b2qx19xb7rr67crp3l1h0000gn/T/.tmpRblraz/Library/LaunchAgents/com.nxsflow.nexus-flow.plist
\ttype = LaunchAgent
\tstate = spawn scheduled

\tprogram = /var/folders/1k/zp26b2qx19xb7rr67crp3l1h0000gn/T/.tmpRblraz/.nexusflow/bin/nexus-flow
\targuments = {
\t\t/var/folders/1k/zp26b2qx19xb7rr67crp3l1h0000gn/T/.tmpRblraz/.nexusflow/bin/nexus-flow
\t}

\tstdout path = /var/folders/1k/T/.tmpRblraz/.nexusflow/logs/service.log
\tstderr path = /var/folders/1k/T/.tmpRblraz/.nexusflow/logs/service.err.log
\tdefault environment = {
\t\tPATH => /usr/bin:/bin:/usr/sbin:/sbin
\t}

\tdomain = gui/501 [100001]
\tlast exit code = 78: EX_CONFIG

\tproperties = keepalive | runatload | inferred program
}
";

    #[test]
    fn parse_print_reads_the_plist_the_job_was_loaded_from_and_not_a_nested_one() {
        let job = parse_print(POISONED_PRINT);
        assert_eq!(
            job.path.as_deref(),
            Some(Path::new(
                "/private/var/folders/1k/zp26b2qx19xb7rr67crp3l1h0000gn/T/.tmpRblraz/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"
            )),
            "the `path` at depth 1 is the plist launchd loaded from; `stdout path` is not it"
        );
        assert_eq!(job.state.as_deref(), Some("spawn scheduled"));
        assert_eq!(job.last_exit_code.as_deref(), Some("78: EX_CONFIG"));
        assert!(job
            .program
            .as_deref()
            .is_some_and(|p| p.ends_with(".nexusflow/bin/nexus-flow")));
    }

    /// The parser's own trap, stated as a test: a nested block may legally contain a key this
    /// module reads at depth 1. Only depth counts.
    #[test]
    fn a_key_inside_a_nested_block_is_never_mistaken_for_the_jobs_own() {
        let out = "gui/501/x = {\n\tstate = running\n\tenvironment = {\n\t\tpath = /nested/decoy\n\
                   \t\tstate = confused\n\t}\n\tpath = /real/plist.plist\n}\n";
        let job = parse_print(out);
        assert_eq!(job.path.as_deref(), Some(Path::new("/real/plist.plist")));
        assert_eq!(job.state.as_deref(), Some("running"));
    }

    /// `print` for a label nothing holds is an ANSWER, not a failure — and the one that separates
    /// "no job is loaded" from "launchctl is broken".
    #[test]
    fn a_label_launchd_does_not_hold_reads_as_not_loaded_rather_than_as_an_error() {
        let ctl = SpyCtl::default();
        assert_eq!(
            read_registration("com.nxsflow.nexus-flow", &ctl),
            Registration::NotLoaded
        );
    }

    #[test]
    fn a_print_that_fails_for_any_other_reason_is_unknown_and_never_not_loaded() {
        let ctl = SpyCtl {
            print_unreadable: true,
            ..SpyCtl::default()
        };
        assert!(
            matches!(
                read_registration("com.nxsflow.nexus-flow", &ctl),
                Registration::Unreadable(_)
            ),
            "an unreadable answer must not render as `there is no job`"
        );
    }

    /// **The 6j6v.kvda finding, end to end from launchd's own bytes.** The registration is loaded
    /// from a TempDir that no longer exists; the instance's real plist is untouched and was never
    /// loaded. `status` had nothing to say about this for five days.
    #[test]
    fn a_registration_loaded_from_a_deleted_tempdir_is_foreign_and_says_the_path_is_gone() {
        let reg = Registration::Loaded(parse_print(POISONED_PRINT));
        let own = Path::new("/Users/u/Library/LaunchAgents/com.nxsflow.nexus-flow.plist");
        let verdict = compare_registration(
            &reg,
            own,
            Path::new("/Users/u/.nexusflow/bin/nexus-flow"),
            false, // the TempDir is gone
            false,
        );
        match verdict {
            RegistrationVerdict::Foreign {
                loaded,
                own: reported_own,
                loaded_exists,
                last_exit_code,
                ..
            } => {
                assert!(loaded.to_string_lossy().contains(".tmpRblraz"));
                assert_eq!(reported_own, own);
                assert!(!loaded_exists, "the plist launchd holds is deleted");
                assert_eq!(last_exit_code.as_deref(), Some("78: EX_CONFIG"));
            }
            other => panic!("the poisoned registration must be FOREIGN, not {other:?}"),
        }
    }

    #[test]
    fn a_registration_loaded_from_our_own_plist_is_ours() {
        let reg = Registration::Loaded(LoadedJob {
            path: Some(PathBuf::from(
                "/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist",
            )),
            program: Some(PathBuf::from("/h/.nexusflow/bin/nexus-flow")),
            state: Some("running".to_string()),
            last_exit_code: None,
        });
        let verdict = compare_registration(
            &reg,
            Path::new("/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
            Path::new("/h/.nexusflow/bin/nexus-flow"),
            true,
            true,
        );
        assert_eq!(
            verdict,
            RegistrationVerdict::Ours {
                loaded: PathBuf::from("/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
                state: Some("running".to_string()),
                program: Some(PathBuf::from("/h/.nexusflow/bin/nexus-flow")),
                program_is_ours: true,
                program_exists: true,
            }
        );
    }

    /// Ours by plist, but launchd is running a program this instance no longer names — the shape a
    /// `self-update` that moved the binary leaves behind. Still OURS: the label is not stolen, and
    /// calling it foreign would send the reader hunting for a registration that is not there.
    #[test]
    fn a_job_from_our_plist_running_another_program_stays_ours_and_says_so() {
        let reg = Registration::Loaded(LoadedJob {
            path: Some(PathBuf::from("/h/LaunchAgents/x.plist")),
            program: Some(PathBuf::from("/old/nexus-flow")),
            state: None,
            last_exit_code: None,
        });
        let verdict = compare_registration(
            &reg,
            Path::new("/h/LaunchAgents/x.plist"),
            Path::new("/new/nexus-flow"),
            true,
            true,
        );
        assert!(matches!(
            verdict,
            RegistrationVerdict::Ours {
                program_is_ours: false,
                ..
            }
        ));
    }

    /// A job launchd reports without saying where it came from is UNKNOWN. Claiming a match on the
    /// strength of the label alone is the over-claim the whole comparison replaces.
    #[test]
    fn a_job_with_no_plist_path_is_unknown_rather_than_assumed_to_be_ours() {
        let reg = Registration::Loaded(LoadedJob::default());
        assert!(matches!(
            compare_registration(&reg, Path::new("/h/x.plist"), Path::new("/p"), false, true),
            RegistrationVerdict::Unknown(_)
        ));
    }

    // ---- The race, and the bounded retry that survives it (6j6v.0yrp) ------------------------

    /// A fixture install: a real plist path and a real program file under `tmp`, so the
    /// preconditions the diagnosis checks are all TRUE unless a test makes one false.
    fn fixture(tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        let plist = tmp
            .path()
            .join("agents")
            .join("com.nxsflow.nexus-flow.plist");
        let program = tmp.path().join("bin").join("nexus-flow");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        (plist, program, tmp.path().join("logs"))
    }

    /// **The 6j6v.0yrp race, made to happen.** launchd answers the first two bootstraps with the
    /// measured `Bootstrap failed: 5: Input/output error` and takes the third. Before the retry
    /// existed, this install failed and left a half-state behind.
    #[test]
    fn a_bootstrap_that_loses_the_race_twice_still_installs_on_the_third_attempt() {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            bootstrap_failures: Mutex::new(2),
            ..SpyCtl::default()
        };
        let got = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect("the third attempt takes");
        assert!(matches!(got, Bootstrapped::Confirmed { .. }));
        let verbs = ctl.verbs();
        assert_eq!(
            verbs.iter().filter(|c| c.starts_with("bootstrap")).count(),
            3,
            "one attempt per loss plus the one that took: {verbs:?}"
        );
        assert_eq!(
            ctl.naps().len(),
            2,
            "one wait before each retry and none after the success: {:?}",
            ctl.naps()
        );
        // Between two attempts the label is booted out again: arriving at a half-torn-down label a
        // second time with no teardown of our own is how one race becomes five.
        let bootouts_after_first_bootstrap = verbs
            .iter()
            .skip_while(|c| !c.starts_with("bootstrap"))
            .filter(|c| c.starts_with("bootout"))
            .count();
        assert_eq!(bootouts_after_first_bootstrap, 2, "{verbs:?}");
    }

    /// The waits are bounded and short: four of them, 2.25s in total, and the schedule is provable
    /// without waiting for any of it.
    #[test]
    fn the_retry_schedule_is_bounded_and_never_grows_past_its_cap() {
        let waits: Vec<u128> = (1..BOOTSTRAP_ATTEMPTS)
            .map(|a| bootstrap_wait(a).as_millis())
            .collect();
        assert_eq!(waits, vec![150, 300, 600, 1200]);
        assert_eq!(waits.iter().sum::<u128>(), 2250);
    }

    /// **The verification, made to matter** (6j6v.0yrp, point 1). launchd accepts every bootstrap
    /// and loads nothing. Exit 0 is a receipt for the REQUEST; without a read-back, this install
    /// reports a running service that does not exist — which is the family 6j6v.d43g and 6j6v.kcan
    /// belong to, arriving at the installer.
    #[test]
    fn a_bootstrap_that_returns_success_while_loading_nothing_is_not_reported_as_an_install() {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            bootstrap_lies: true,
            ..SpyCtl::default()
        };
        let err = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect_err("a bootstrap nobody read back is not an install");
        assert!(err.msg.contains("stopped half-way"), "{}", err.msg);
        let verbs = ctl.verbs();
        assert!(
            verbs.iter().filter(|c| c.starts_with("print ")).count() >= BOOTSTRAP_ATTEMPTS as usize,
            "every attempt was read back rather than believed: {verbs:?}"
        );
    }

    /// **The 6j6v.kvda poisoning, from the installer's side.** Another plist holds the label and
    /// `bootout` cannot shift it. `bootstrap` then reports success against a job that is not ours,
    /// and the install must say WHICH plist is in the way — the one sentence that would have
    /// ended the five days in a minute.
    #[test]
    fn an_install_blocked_by_a_foreign_registration_names_the_plist_that_holds_the_label() {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            sticky: true,
            ..SpyCtl::poisoned(
                "com.nxsflow.nexus-flow",
                "/private/var/folders/1k/T/.tmpRblraz/Library/LaunchAgents/com.nxsflow.nexus-flow.plist",
            )
        };
        let err = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect_err("a label held by somebody else is not an install");
        assert!(err.msg.contains(".tmpRblraz"), "{}", err.msg);
        assert!(
            err.msg.contains("no longer exists"),
            "and it says the plist launchd holds is gone: {}",
            err.msg
        );
        assert!(
            !err.msg.trim_end().ends_with("Input/output error"),
            "the raw errno is the LAST line, never the whole answer: {}",
            err.msg
        );
    }

    /// The install that cannot be verified is reported as unverified — never as verified, and never
    /// as failed. A read-back that could not be performed is not evidence either way.
    #[test]
    fn an_install_whose_registration_cannot_be_read_back_at_all_says_so_rather_than_claiming_success(
    ) {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            print_unreadable: true,
            ..SpyCtl::default()
        };
        let got = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect("an unreadable read-back does not fail an install that was accepted");
        assert!(matches!(got, Bootstrapped::Unconfirmed(_)), "{got:?}");
    }

    /// **A brace in a path is a character, not a block** (review of PR #428, Integrity #2). The
    /// first cut counted every `{`/`}` on the line, so one unbalanced brace in `path`'s VALUE
    /// desynchronised the depth for every field after it — and a lost `program` then defaulted to
    /// "matches" rather than to "unknown", which is a false reassurance and not a visible failure.
    #[test]
    fn a_brace_inside_a_path_value_does_not_swallow_the_fields_after_it() {
        let out = "gui/501/x = {\n\tpath = /Users/u/od{d/com.nxsflow.x.plist\n\tstate = running\n\
                   \tprogram = /Users/u/.nexusflow/bin/nexus-flow\n\targuments = {\n\
                   \t\t/Users/u/.nexusflow/bin/nexus-flow\n\t}\n\tlast exit code = 0\n}\n";
        let job = parse_print(out);
        assert_eq!(
            job.path.as_deref(),
            Some(Path::new("/Users/u/od{d/com.nxsflow.x.plist"))
        );
        assert_eq!(
            job.state.as_deref(),
            Some("running"),
            "the field AFTER the brace is the one that used to disappear"
        );
        assert_eq!(
            job.program.as_deref(),
            Some(Path::new("/Users/u/.nexusflow/bin/nexus-flow")),
            "and a lost `program` would have read as `program_is_ours: true` — a false all-clear"
        );
        assert_eq!(job.last_exit_code.as_deref(), Some("0"));
    }

    /// The nested block still nests. The brace fix must not buy field-safety by giving up the
    /// depth-awareness that stops a nested `path` being read as the job's own.
    #[test]
    fn a_nested_block_still_hides_its_own_keys_after_the_brace_fix() {
        let out = "gui/501/x = {\n\tenvironment = {\n\t\tpath = /nested/decoy\n\t}\n\
                   \tpath = /real/plist.plist\n}\n";
        assert_eq!(
            parse_print(out).path.as_deref(),
            Some(Path::new("/real/plist.plist"))
        );
    }

    /// **One flaky `print` must not demote a good install** (review of PR #428, Integrity #3).
    /// The bootstrap succeeded; the only thing in doubt was whether anybody could look.
    #[test]
    fn a_read_back_that_fails_once_is_retried_rather_than_reported_as_unconfirmed() {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            print_failures: Mutex::new(1),
            ..SpyCtl::default()
        };
        let got = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect("a transient read failure is not an install failure");
        assert!(
            matches!(got, Bootstrapped::Confirmed { .. }),
            "the second read answered, so the install IS confirmed: {got:?}"
        );
        assert_eq!(
            ctl.verbs()
                .iter()
                .filter(|c| c.starts_with("bootstrap"))
                .count(),
            1,
            "and nothing was re-bootstrapped over a job that was fine"
        );
    }

    /// The second chance is exactly one. A structurally unreadable `print` — no launchctl, a domain
    /// this process cannot address — must not make the user wait five times for the same answer.
    #[test]
    fn a_read_back_that_never_answers_is_tried_twice_and_no_more() {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            print_unreadable: true,
            ..SpyCtl::default()
        };
        let got = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect("an unreadable read-back does not fail an install that was accepted");
        assert!(matches!(got, Bootstrapped::Unconfirmed(_)), "{got:?}");
        assert_eq!(
            ctl.verbs()
                .iter()
                .filter(|c| c.starts_with("print "))
                .count(),
            2,
            "one read plus its one second chance: {:?}",
            ctl.verbs()
        );
    }

    /// The two sentences `describe_unverified` produces for a bootstrap that returned success over
    /// a launchd holding something else — asserted here because the FOREIGN string a black-box test
    /// checks comes from a DIFFERENT code path (`install_failure`'s own diagnosis), so these two
    /// arms were words no test had ever read (review of PR #428, Test Quality #2).
    #[test]
    fn the_unverified_sentences_name_what_launchd_holds_instead() {
        let nothing = describe_unverified(&RegistrationVerdict::NotLoaded);
        assert!(nothing.contains("reported success"), "{nothing}");
        assert!(nothing.contains("holds no job"), "{nothing}");

        let foreign = describe_unverified(&RegistrationVerdict::Foreign {
            loaded: PathBuf::from("/tmp/.tmpRblraz/com.nxsflow.nexus-flow.plist"),
            own: PathBuf::from("/h/LaunchAgents/com.nxsflow.nexus-flow.plist"),
            loaded_exists: false,
            program: None,
            state: None,
            last_exit_code: None,
        });
        assert!(foreign.contains(".tmpRblraz"), "{foreign}");
        assert!(
            foreign.contains("a different plist"),
            "and WHY this one cannot take its place: {foreign}"
        );
        assert_ne!(nothing, foreign, "two states must never read the same");
    }

    // ---- Naming the precondition instead of the errno (6j6v.0yrp, point 2) -------------------

    fn all_preconditions_hold(tmp: &TempDir) -> Preconditions {
        let (plist, program, logs) = fixture(tmp);
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(
            &plist,
            render_plist(&Instance::production().label(), &program, &logs, "/usr/bin"),
        )
        .unwrap();
        Preconditions {
            plist,
            plist_bytes: Some(900),
            plist_is_a_plist: true,
            program_link: program.clone(),
            program_target: Some(program),
            program_exists: true,
            log_dir: logs,
            log_dir_present: true,
            free_bytes: Some(60 * 1024 * 1024 * 1024),
            disabled: Some(false),
            loaded: RegistrationVerdict::NotLoaded,
        }
    }

    /// The 2026-09-04 investigation, as the product does it: with every precondition true, the
    /// diagnosis says exactly that — which is the sentence that ends the hunt rather than starting
    /// it.
    #[test]
    fn a_failure_with_every_precondition_true_says_so_rather_than_listing_nothing() {
        let tmp = TempDir::new().unwrap();
        let p = all_preconditions_hold(&tmp);
        assert!(failed_preconditions(&p).is_empty());
        let msg = install_failure("Bootstrap failed: 5: Input/output error", &p).msg;
        assert!(
            msg.contains("every precondition this installer controls held"),
            "{msg}"
        );
        assert!(
            msg.contains("Bootstrap failed: 5"),
            "launchctl's own words are kept as the LAST line, not thrown away: {msg}"
        );
    }

    /// Each of the checks the owner ran by hand, now run by the installer that wrote them.
    #[test]
    fn each_false_precondition_is_named_in_the_installers_own_vocabulary() {
        let tmp = TempDir::new().unwrap();

        let mut p = all_preconditions_hold(&tmp);
        p.log_dir_present = false;
        assert!(
            failed_preconditions(&p)[0].contains("log directory"),
            "{:?}",
            failed_preconditions(&p)
        );

        let mut p = all_preconditions_hold(&tmp);
        p.program_exists = false;
        assert!(failed_preconditions(&p)
            .iter()
            .any(|c| c.contains("does not exist")));

        let mut p = all_preconditions_hold(&tmp);
        p.program_target = None;
        assert!(failed_preconditions(&p)
            .iter()
            .any(|c| c.contains("does not resolve")));

        let mut p = all_preconditions_hold(&tmp);
        p.disabled = Some(true);
        assert!(failed_preconditions(&p)
            .iter()
            .any(|c| c.contains("disabled list")));

        let mut p = all_preconditions_hold(&tmp);
        p.plist_is_a_plist = false;
        assert!(failed_preconditions(&p)
            .iter()
            .any(|c| c.contains("is not the plist this install writes")));

        let mut p = all_preconditions_hold(&tmp);
        p.free_bytes = Some(1024);
        assert!(failed_preconditions(&p).iter().any(|c| c.contains("free")));

        // And 62 GB free is NOT a complaint — a diagnosis that lists every fact it checked is as
        // unreadable as one that lists none.
        let mut p = all_preconditions_hold(&tmp);
        p.free_bytes = Some(62 * 1024 * 1024 * 1024);
        assert!(failed_preconditions(&p).is_empty());
    }

    /// A foreign registration is the precondition an installer can see and a person cannot: it is
    /// why `bootstrap` refuses, and it is invisible in `~/Library/LaunchAgents`.
    #[test]
    fn a_label_held_by_another_plist_is_named_as_the_reason_the_bootstrap_had_no_chance() {
        let tmp = TempDir::new().unwrap();
        let mut p = all_preconditions_hold(&tmp);
        p.loaded = RegistrationVerdict::Foreign {
            loaded: PathBuf::from(
                "/tmp/.tmpRblraz/Library/LaunchAgents/com.nxsflow.nexus-flow.plist",
            ),
            own: p.plist.clone(),
            loaded_exists: false,
            program: None,
            state: Some("spawn scheduled".to_string()),
            last_exit_code: Some("78: EX_CONFIG".to_string()),
        };
        let complaints = failed_preconditions(&p);
        assert!(
            complaints.iter().any(|c| c.contains(".tmpRblraz")),
            "{complaints:?}"
        );
        assert!(
            complaints.iter().any(|c| c.contains("no longer exists")),
            "{complaints:?}"
        );
    }

    /// **The half-state, named at the moment it is created** (6j6v.0yrp, point 3). Whoever reads a
    /// failed install is looking at a written plist, a re-pointed alias and no loaded job, and the
    /// heartbeat beside it still describes the process that is gone.
    #[test]
    fn a_failed_install_names_the_half_state_it_leaves_behind() {
        let tmp = TempDir::new().unwrap();
        let (plist, program, logs) = fixture(&tmp);
        let ctl = SpyCtl {
            fail_bootstrap: true,
            ..SpyCtl::default()
        };
        let err = install_with(
            &Instance::production(),
            &plist,
            &program,
            &logs,
            "/usr/bin",
            &ctl,
        )
        .expect_err("every attempt was refused");
        assert!(err.msg.contains("stopped half-way"), "{}", err.msg);
        assert!(err.msg.contains("nxs sync daemon status"), "{}", err.msg);
        assert_eq!(
            ctl.verbs()
                .iter()
                .filter(|c| c.starts_with("bootstrap"))
                .count(),
            BOOTSTRAP_ATTEMPTS as usize,
            "it gives up, and it gives up after a BOUNDED number of tries"
        );
    }

    /// `print-disabled` lists EVERY label it knows, each with `=> disabled` or `=> enabled`, and
    /// a sibling instance's label is a superstring of the production one. Both are ways to answer
    /// "is this label disabled?" with a confident yes about a service that is perfectly enabled —
    /// which would send a reader to `launchctl enable` for a fault that is somewhere else.
    #[test]
    fn the_disabled_check_reads_the_verdict_and_matches_the_label_exactly() {
        let ctl = SpyCtl {
            disabled: vec!["com.nxsflow.nexus-flow-dev".to_string()],
            enabled: vec!["com.nxsflow.nexus-flow".to_string()],
            ..SpyCtl::default()
        };
        assert_eq!(read_disabled("com.nxsflow.nexus-flow", &ctl), Some(false));
        assert_eq!(
            read_disabled("com.nxsflow.nexus-flow-dev", &ctl),
            Some(true)
        );
        // A label launchd has never heard of is not disabled — and not unknown either: the list
        // was read, and it does not name it.
        assert_eq!(read_disabled("com.nxsflow.absent", &ctl), Some(false));
    }

    // ---- The gate (6j6v.kvda, half 1) --------------------------------------------------------

    /// The rule, as a pure function: a process whose home is not the login session's may not talk
    /// to launchd at all.
    #[test]
    fn a_home_that_is_not_the_login_sessions_is_refused_and_the_refusal_names_both() {
        let complaint = login_session_complaint(
            Path::new("/private/var/folders/1k/T/.tmp7Xq/"),
            Path::new("/Users/u"),
            501,
        )
        .expect("a redirected home must be refused");
        assert!(complaint.contains(".tmp7Xq"), "{complaint}");
        assert!(complaint.contains("/Users/u"), "{complaint}");
        assert!(complaint.contains("gui/501"), "{complaint}");
        assert!(
            complaint.contains("6j6v.kvda"),
            "the refusal points at the incident it prevents: {complaint}"
        );
    }

    /// **The one way out, named in the refusal itself** (review of PR #428, Integrity #1). A Mac
    /// whose `$HOME` is redirected on purpose — MDM, a roaming profile, security tooling — is a
    /// real, non-adversarial configuration, and for its owner this rule is a false accusation: the
    /// two paths are permanent, not a `TempDir` about to vanish. A refusal that leaves them no move
    /// is a regression dressed as a guard.
    #[test]
    fn the_refusal_names_the_flag_that_unblocks_a_legitimately_redirected_home() {
        let complaint =
            login_session_complaint(Path::new("/corp/homes/u"), Path::new("/Users/u"), 501)
                .expect("the mismatch is still refused by default");
        assert!(
            complaint.contains("--allow-redirected-home"),
            "an operator who cannot change their $HOME must be told the move that exists: \
             {complaint}"
        );
        assert!(
            complaint.contains("REDIRECTED on purpose"),
            "and when it applies to them: {complaint}"
        );
    }

    #[test]
    fn the_login_sessions_own_home_is_allowed() {
        assert!(
            login_session_complaint(Path::new("/Users/u"), Path::new("/Users/u"), 501).is_none()
        );
    }

    /// The real seam, exercised on the real `launchctl` — with a READ of a label that cannot exist,
    /// so nothing on this machine is touched. It closes the one link the fixtures above cannot:
    /// that `RealCtl::output` really does hand back launchd's "no such service" rather than an
    /// error, and that [`read_registration`] really does read it as [`Registration::NotLoaded`].
    #[test]
    #[cfg(target_os = "macos")]
    fn the_real_launchctl_reports_an_absent_label_as_not_loaded() {
        let Ok(ctl) = RealCtl::for_login_session(HomeRule::MustBeTheLoginSessions) else {
            // A test process whose home has been pointed elsewhere is exactly what the gate above
            // refuses; there is nothing to read and nothing to prove here.
            return;
        };
        assert_eq!(
            read_registration("com.nxsflow.no-such-label-6j6v-0yrp", &ctl),
            Registration::NotLoaded
        );
    }
}

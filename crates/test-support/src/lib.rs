//! The STALE multicall binary gate (nexus-flow-0yyw).
//!
//! Since the multicall collapse (5jz.7) `nxf`/`nxm`/`nxc` are no longer bin targets — they are
//! `argv[0]` SYMLINKS to the single `nxs` binary, created next to it by `crates/nxs/build.rs`.
//! `nxs` is its OWN package that depends on the persona crates, not the other way round. So a
//! package-scoped run (`cargo test -p nexus-flow-cli`) never rebuilds `nxs`, while `trycmd` and
//! `assert_cmd` still resolve `target/<profile>/nxf` by name, follow the symlink, and execute an
//! ARBITRARILY OLD binary. The run is green and asserts against foreign code.
//!
//! That is not hypothetical: in PR #241 goldens were regenerated after a rebase against an `nxs`
//! built BEFORE it, which silently deleted a command entry from the agent-facing docs.
//!
//! This gate turns the silent trap into a loud failure: compare the binary's mtime against the
//! newest compile input that links into it, and panic with the remedy before any assertion runs.
//!
//! The crate has since become the home of the repo's *cross-cutting* test gates — the ones whose
//! subject is a whole surface rather than one function. Its second tenant is [`verb_seam`]: the
//! omission gate that holds each persona's CLI verb list against the library seam an embedding app
//! links (nexus-flow-6j6v.vtvs). Its third is [`actor_contract`]: the detector that pins what the
//! public write entry points of the consumed surfaces do with the `actor` they are handed, so a
//! behavioural tightening reddens on its own instead of waiting to be declared (6j6v.d4dw). Its
//! fourth is [`seam_disposition`]: the other DIRECTION of the omission gate — for every entrance
//! taken off an agent surface, whether the corresponding read/write stays on the seam is decided,
//! recorded and held, rather than left to diligence (6j6v.dvyq). Its fifth is [`PinHome`]: the one
//! place a black-box invocation's `$HOME` is pinned, so that the variables which out-vote a pinned
//! home are pinned with it and the source gate beside it can be absolute (6j6v.9bjv).

/// The verb-seam omission gate (nexus-flow-6j6v.vtvs).
pub mod verb_seam;

/// The actor-contract detector (nexus-flow-6j6v.d4dw).
pub mod actor_contract;

/// The seam-disposition gate (nexus-flow-6j6v.dvyq, acceptance point 3).
pub mod seam_disposition;

use std::collections::{BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

pub use assert_cmd::Command;

/// Resolve a multicall persona binary (`nxf`/`nxm`/`nxc`/`nxs`) for a black-box test, refusing to
/// hand back a command that would run a STALE `nxs`.
///
/// This is the gated replacement for `assert_cmd::Command::cargo_bin(name)`. Going through it means
/// a test target is covered no matter how it is invoked — including a single-target run like
/// `cargo test -p nexus-flow-cli --test create`, which reaches neither the standalone
/// `binary_freshness` target nor the golden harness.
pub fn cargo_bin(name: &str) -> Command {
    assert_multicall_binary_fresh();
    let mut cmd = Command::cargo_bin(name).unwrap_or_else(|e| panic!("{name} binary: {e}"));
    // NEVER the host's real scheduler (nexus-flow-6j6v.74c0). Several `nxc` suites already say this
    // in prose — "a test suite must not shell out to the host's scheduler to find out what this
    // engine decided" — and set `NXC_TIMER=dry` call by call. It has to be structural now: an unset
    // `NXC_TIMER` used to mean `at`, which on macOS refuses before it submits anything (`atrun`
    // ships disabled), so a suite that forgot the line was harmless there by accident. It means
    // `launchd` on macOS since 74c0, and that one WORKS — a forgotten line would bootstrap a real
    // one-shot agent into the developer's own login session, once per channel opened, each of them
    // due to fire `nxc tick` against a `TempDir` that is long gone.
    //
    // A test that is ABOUT a real backend still says so: this is a default, and the `.env` a caller
    // adds afterwards wins.
    cmd.env("NXC_TIMER", "dry");
    // NEVER a real MODEL either (nxf 6j6v.e76c), and the argument is the one above, one seam over.
    // `nxc send` commissions a name for the thread it opens, and the shipped backend does that by
    // starting a detached run that asks Haiku what the conversation should be called — so a suite
    // that forgot the line would make one model call per `send`, on somebody's account, and let a
    // second process write into the very database the test is asserting on. `dry` derives the
    // message's own first words instead, so the whole path stays observable and nothing is spent.
    //
    // A test that is ABOUT the real backend still says so: this is a default, and the `.env` a
    // caller adds afterwards wins.
    cmd.env("NXC_NAMER", "dry");
    // NEVER the developer's real `~/.nexusflow` either (review of PR #393, Code Quality #1).
    //
    // Same argument as the line above, one ticket later. Since nxf 6j6v.dcpk EVERY invocation of
    // this binary — whatever the verb, `--version` included — resolves `$HOME` to look at the
    // launchd job and the service's program alias, and says so on stderr when that alias is dead.
    // That made the host's own service state an input to every black-box test in this repo: on a
    // machine whose alias is dangling (this one has been there twice in a week — it is what the
    // ticket was cut for) every test subprocess gains a stderr line it did not ask for. Nothing is
    // red today only because no suite asserts exact or empty stderr, which is incidental rather
    // than designed.
    //
    // The dir lives under `target/<profile>` so `cargo clean` takes it with everything else, and
    // carries the pid so two test binaries running in parallel cannot write over each other.
    //
    // It goes through [`PinHome`] rather than `.env` directly, because a pinned home is not enough
    // on its own to decide where the product looks — see that trait for the two variables that
    // out-vote it, one of which is set in this repo's own `.envrc` (nxf 6j6v.9bjv).
    cmd.pin_home(pinned_home());
    cmd
}

/// The name of the variable itself, spelled ONCE for the whole repository.
///
/// Every other place says [`PinHome::pin_home`] instead, which is what lets
/// `every_pinned_home_pins_the_instance.rs` check a LITERAL — nothing in the tree hands this name
/// to a call that sets it — rather than judge, file by file, whether a call site remembered
/// enough. That gate names the two spellings it reads and the two it cannot; read it there rather
/// than assuming the check is exhaustive (review of PR #413, Test Quality #1).
const HOME_ENV: &str = "HOME";

/// What a pinned `$HOME` has to be pinned TOGETHER WITH — see [`PinHome`] for why each is here.
const CLEARED_WITH_HOME: &[&str] = &["XDG_CONFIG_HOME", "XDG_DATA_HOME", "NXS_SERVICE_INSTANCE"];

/// Point a black-box invocation's `$HOME` at a directory the test owns — and with it everything
/// ELSE that decides where the product looks.
///
/// Pinning `$HOME` alone does not do that, and what it misses is invisible on the machine where
/// the miss happens (nxf 6j6v.9bjv):
///
///   * **`XDG_CONFIG_HOME` / `XDG_DATA_HOME`** — on Linux `directories` answers from these BEFORE
///     `$HOME`, so a pinned home moves nothing while they stand. Nearly every suite that pins a
///     home already cleared them by hand; this is that line, kept in one place.
///   * **`NXS_SERVICE_INSTANCE`** — a machine can run several named services (nxf 6j6v.gd9p), and
///     the shell that starts a test run names one: this repo's own `.envrc` exports
///     `nexus-flow-dev`, deliberately, so that a development build here is not the only clock on
///     the machine. Every subprocess inherits it, and the resolution then reads
///     `<the pinned home>/.nexusflow-dev` — a directory the suite never wrote to. Nothing leaks
///     into the developer's real `~`: the test reads an EMPTY registry under its OWN home and
///     reports the miss as a failure. Ten tests across four binaries were red on the owner's
///     machine and green in CI for exactly this, CI being the one environment where the variable
///     is not set — a gate that is fail-open against the environment the variable was built for.
///
/// A suite whose SUBJECT is one of these sets it itself afterwards and wins:
/// `crates/nxs/tests/instance_isolation.rs` and `service_stays_awake.rs` both do, and that is the
/// intended shape rather than a workaround.
pub trait PinHome {
    /// Set `$HOME` to `home` and clear every variable that could out-vote it.
    fn pin_home<P: AsRef<std::ffi::OsStr>>(&mut self, home: P) -> &mut Self;
}

impl PinHome for std::process::Command {
    fn pin_home<P: AsRef<std::ffi::OsStr>>(&mut self, home: P) -> &mut Self {
        self.env(HOME_ENV, home);
        for var in CLEARED_WITH_HOME {
            self.env_remove(var);
        }
        self
    }
}

/// The same isolation as [`PinHome::pin_home`], as VALUES a runner can only SET.
///
/// `trycmd::TestCases` has `env` and no `env_remove`, and a golden corpus needs the pinning most of
/// all: it asserts stdout AND stderr byte for byte, so one line the host adds unasked — the service
/// report every invocation makes when this machine's program alias is dangling (nxf 6j6v.dcpk) —
/// fails every case, and under `TRYCMD=overwrite` is BURNED INTO the committed goldens with the
/// developer's own paths in it. That is not hypothetical: it happened on the owner's machine while
/// nxf 6j6v.q6e3 was being built, and the regenerated corpus carried 240 such lines.
///
/// Each pair is the setting an unset variable resolves to anyway, so nothing is decided here that
/// the environment would not have decided on a clean machine:
///
///   * `HOME` — the same throwaway directory [`cargo_bin`] pins, so the corpus reads an empty
///     registry and heartbeat rather than the developer's.
///   * `XDG_CONFIG_HOME` / `XDG_DATA_HOME` — pointed INSIDE that home rather than cleared, because
///     on Linux `directories` answers from them first; the two spellings then agree.
///   * `NXS_SERVICE_INSTANCE` — the PRODUCTION name, which is exactly what
///     `nxs_service::Instance::resolve_from` falls back to when the variable is unset. A corpus
///     that inherited this repo's `.envrc` value would otherwise resolve `<home>/.nexusflow-dev`
///     (nxf 6j6v.9bjv).
pub fn pinned_home_env() -> Vec<(&'static str, String)> {
    let home = pinned_home();
    vec![
        (HOME_ENV, home.display().to_string()),
        (
            "XDG_CONFIG_HOME",
            home.join(".config").display().to_string(),
        ),
        (
            "XDG_DATA_HOME",
            home.join(".local/share").display().to_string(),
        ),
        ("NXS_SERVICE_INSTANCE", "nexus-flow".to_string()),
    ]
}

impl PinHome for Command {
    fn pin_home<P: AsRef<std::ffi::OsStr>>(&mut self, home: P) -> &mut Self {
        self.env(HOME_ENV, home);
        for var in CLEARED_WITH_HOME {
            self.env_remove(var);
        }
        self
    }
}

/// The throwaway `$HOME` [`cargo_bin`] hands every black-box invocation.
///
/// **Public since nxf 6j6v.npf9**, for the suites that deliberately do NOT go through
/// [`cargo_bin`] (they build their own command so as not to inherit its defaults) and still must
/// not reach the developer's real `~`. That stopped being a nicety the moment `nxs init` began
/// WRITING to `~/.nexusflow/workspaces.toml`: an unpinned suite rebuilds the 98 stale entries the
/// registry was once measured at, a few `TempDir`s per test run, and does it invisibly in CI —
/// where `$HOME` is a throwaway anyway.
///
/// One per test PROCESS, created on first use: a shared one would let two suites race on the same
/// `~/.nexusflow/workspaces.toml`, which is the accumulation the service registry's own history
/// records (98 stale entries written by tests that had nowhere else to reach).
///
/// A caller that needs its own — every suite that WRITES there and then reads it back — still sets
/// `HOME` itself afterwards and wins, exactly as it does for `NXC_TIMER`.
pub fn pinned_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = profile_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("test-home")
            .join(std::process::id().to_string());
        // Best-effort: a HOME that could not be created is still better pointed at a path that
        // does not exist than at the developer's own.
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

/// Fail the current test unless `target/<profile>/nxs` is at least as new as every source that
/// compiles into it.
///
/// Prefer [`cargo_bin`], which calls this for you. Call it directly only where no command is built
/// — notably at the top of a `trycmd` harness, before `TRYCMD=overwrite` can rewrite goldens from a
/// stale binary.
pub fn assert_multicall_binary_fresh() {
    panic_if_stale(stale_complaint().as_deref());
}

/// Turn a complaint into the abort. Split from [`assert_multicall_binary_fresh`] purely so a test
/// can drive the REAL panic path against a fixture — the cached real-workspace verdict cannot be
/// made stale on demand.
fn panic_if_stale(complaint: Option<&str>) {
    if let Some(complaint) = complaint {
        panic!("{complaint}");
    }
}

/// The real workspace's verdict: the failure text, or `None` when the binary is fresh.
///
/// Computed once per test process and cached: the answer cannot change mid-run (cargo has finished
/// building by the time tests execute), and [`cargo_bin`] is called from every black-box test — so
/// without the cache each call would re-walk the whole source tree.
fn stale_complaint() -> &'static Option<String> {
    static COMPLAINT: OnceLock<Option<String>> = OnceLock::new();
    COMPLAINT.get_or_init(|| {
        let profile = profile_dir().expect("test binary lives in target/<profile>/deps");
        complaint_in(&profile, &workspace_root())
    })
}

/// The whole decision, against an explicit `target/<profile>` and workspace root — so a fixture can
/// drive it end to end without a cargo build.
fn complaint_in(profile: &Path, root: &Path) -> Option<String> {
    let binary = profile.join(format!("nxs{}", std::env::consts::EXE_SUFFIX));
    let closure = nxs_closure(root);

    let detail = match freshness(mtime(&binary), newest_source(&closure)) {
        Freshness::Fresh => return None,
        Freshness::Missing => format!("{} does not exist.", binary.display()),
        Freshness::Stale { source } => format!(
            "{} is OLDER than {}.",
            binary.display(),
            source.strip_prefix(root).unwrap_or(&source).display()
        ),
        // Not a stale binary — the gate itself has gone blind, and says so in its own words rather
        // than borrowing the stale-binary wording and sending someone off to rebuild for nothing.
        Freshness::NoSources => {
            return Some(format!(
                "\n\nSTALE-BINARY GATE IS BLIND — it found no source to compare against.\n\n  \
                 Walked {} crate(s) from {}, and not one readable file under `src/`.\n\n\
                 This is NOT a stale binary, and `cargo build -p nxs` will not help: the gate can\n\
                 no longer see the sources it exists to watch, so it cannot detect staleness at\n\
                 all. Something moved — a renamed manifest, a relocated crate, a changed workspace\n\
                 layout. Fix the gate (crates/test-support); do not silence it. A gate that quietly\n\
                 finds nothing to compare reports `fresh` forever, which is precisely the silence\n\
                 nexus-flow-0yyw exists to abolish.\n",
                closure.len(),
                root.join("crates").join("nxs").display(),
            ))
        }
    };

    Some(format!(
        "\n\nSTALE MULTICALL BINARY — this run would have tested foreign code.\n\n  \
         {detail}\n\n\
         `nxf`/`nxm`/`nxc` are argv[0] symlinks to the single `nxs` binary (nexus-flow-5jz.7).\n\
         `nxs` is its own package, so a package-scoped run (`cargo test -p <crate>`) does NOT\n\
         rebuild it — trycmd/assert_cmd still resolve target/<profile>/nxf, follow the symlink,\n\
         and execute an arbitrarily old binary. The run goes green while asserting against code\n\
         that is not the code you edited.\n\n\
         Build the binary under test, then re-run:\n\n    \
         cargo build -p nxs\n\n\
         (`cargo test --all` does this for you. See nexus-flow-0yyw.)\n"
    ))
}

/// mtime of `p`, or `None` if it does not exist.
fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// The verdict. Split out as a pure decision so the comparison is unit-testable without a build.
#[derive(Debug, PartialEq, Eq)]
enum Freshness {
    Fresh,
    /// No binary at all — a package-scoped run that never built `nxs`.
    Missing,
    Stale {
        source: PathBuf,
    },
    /// The closure walk produced no source to compare against. Never a legitimate state: `nxs`
    /// always has sources, so this means the gate has gone blind (a moved crate, a renamed
    /// manifest). It is a distinct verdict rather than folded into `Fresh` because "I found nothing
    /// to check" and "I checked and it is fine" are opposite facts, and reporting the first as the
    /// second would disable the gate in perfect silence — the failure mode this crate exists to end.
    NoSources,
}

/// Decide freshness from the binary's mtime and the newest source's.
fn freshness(binary: Option<SystemTime>, newest: Option<(PathBuf, SystemTime)>) -> Freshness {
    let Some(binary) = binary else {
        return Freshness::Missing;
    };
    let Some((source, source_mtime)) = newest else {
        return Freshness::NoSources;
    };
    if source_mtime > binary {
        Freshness::Stale { source }
    } else {
        Freshness::Fresh
    }
}

/// The `path = "…"` targets declared in `[dependencies]`/`[build-dependencies]`, including their
/// platform-gated forms (`[target.'cfg(unix)'.dependencies]`).
///
/// The per-target sections are scanned rather than documented as a gap: a platform-gated path dep
/// compiles into the binary on that platform exactly like a plain one, so skipping it would shrink
/// the watched set with no sign — the gate would keep reporting `fresh` while missing real edits.
/// Scanning every cfg regardless of which one is active is deliberate: watching a source that does
/// not link on THIS host at worst asks for a rebuild that is a no-op, whereas missing one that does
/// link brings back the silent trap. (Today only `crates/nxs/Cargo.toml` has such a section and it
/// holds a registry dep, so this is future-proofing, not a live fix.)
///
/// Dev-dependencies are deliberately NOT followed: they do not link into the binary, so touching
/// one must never trip the gate.
fn path_deps(manifest: &str) -> Vec<String> {
    let Ok(doc) = manifest.parse::<toml::Table>() else {
        return Vec::new();
    };
    let per_target = doc
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(toml::Table::values)
        .filter_map(toml::Value::as_table);

    std::iter::once(&doc)
        .chain(per_target)
        .flat_map(|scope| {
            ["dependencies", "build-dependencies"]
                .into_iter()
                .filter_map(move |section| scope.get(section)?.as_table())
                .flat_map(toml::Table::values)
        })
        .filter_map(|dep| Some(dep.as_table()?.get("path")?.as_str()?.to_owned()))
        .collect()
}

/// The workspace crates compiled INTO `nxs`: `nxs` plus the transitive closure of its path deps.
fn nxs_closure(root: &Path) -> BTreeSet<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([root.join("crates").join("nxs")]);
    while let Some(krate) = queue.pop_front() {
        if !seen.insert(krate.clone()) {
            continue;
        }
        let Ok(manifest) = std::fs::read_to_string(krate.join("Cargo.toml")) else {
            continue;
        };
        queue.extend(path_deps(&manifest).iter().map(|d| lexical_join(&krate, d)));
    }
    seen
}

/// Resolve `base/rel` (`crates/nxs` + `../core` → `crates/core`) without touching the filesystem,
/// so paths stay readable in the failure message instead of becoming canonicalized noise.
fn lexical_join(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// The most recently modified compile input across `crates`: every file under `src/`, plus
/// `build.rs`, plus the guide docs embedded via `include_dir!` (`docs/guide/en/**`).
///
/// The inclusions and exclusions are load-bearing, not laziness. Every path here MUST be one that
/// cargo rebuilds on, or the gate could go red with no way to clear it — and an unfixable gate gets
/// deleted:
///   * `docs/guide/en/**` — baked into `nxs` by `crates/cli/src/commands/guide.rs`. Editing a file
///     is tracked (the macro expands to `include_bytes!`, recorded in dep-info), and since
///     `crates/cli/build.rs` emits `rerun-if-changed=docs/guide/en` (nexus-flow-e1qn) so is
///     adding/removing one — `cargo build -p nxs` clears a stale verdict, which is exactly what
///     lets us watch it. `docs/guide/de` is NOT watched: it is website-only, compiled into nothing,
///     so no rebuild could refresh it — watching it would strand the gate permanently red. (The
///     `files_under` walk is keyed on the embedded dir, so any crate without one contributes
///     nothing.)
///   * `Cargo.toml` — cargo fingerprints the PARSED manifest, so a comment-only edit rebuilds
///     nothing.
///   * `Cargo.lock` / the workspace manifest — a `cargo add` in an unrelated crate (`server`)
///     rewrites them without touching the nxs closure.
///   * `tests/**` — a test edit never relinks the binary.
fn newest_source(crates: &BTreeSet<PathBuf>) -> Option<(PathBuf, SystemTime)> {
    crates
        .iter()
        .flat_map(|krate| {
            let mut inputs = files_under(&krate.join("src"));
            inputs.push(krate.join("build.rs"));
            inputs.extend(files_under(&krate.join("docs/guide/en")));
            inputs
        })
        .filter_map(|f| Some((f.clone(), mtime(&f)?)))
        .max_by_key(|(_, t)| *t)
}

/// Every regular file below `dir`, recursively. A missing dir yields nothing — crates need no
/// `src/` to be listed, and `build.rs` is optional.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => found.push(entry.path()),
                _ => {}
            }
        }
    }
    found
}

/// `target/<profile>`, derived from the running test binary (`target/<profile>/deps/<test>-<hash>`)
/// rather than reconstructed — so it follows `CARGO_TARGET_DIR`, `--release` and `--target <triple>`
/// for free.
fn profile_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().nth(2).map(Path::to_path_buf)
}

/// The workspace root, fixed at COMPILE time from this crate's manifest dir
/// (`<root>/crates/test-support`), so it does not depend on the test's cwd.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits at <root>/crates/test-support")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn binary_newer_than_sources_is_fresh() {
        assert_eq!(
            freshness(Some(t(100)), Some(("a.rs".into(), t(50)))),
            Freshness::Fresh
        );
    }

    #[test]
    fn binary_older_than_a_source_is_stale() {
        assert_eq!(
            freshness(Some(t(50)), Some(("a.rs".into(), t(100)))),
            Freshness::Stale {
                source: "a.rs".into()
            }
        );
    }

    /// The binary is rebuilt AFTER the source it compiles, so equal mtimes are the boundary of
    /// fresh, not of stale — otherwise a fast build could flake red.
    #[test]
    fn equal_mtime_is_fresh() {
        assert_eq!(
            freshness(Some(t(100)), Some(("a.rs".into(), t(100)))),
            Freshness::Fresh
        );
    }

    #[test]
    fn absent_binary_is_missing() {
        assert_eq!(
            freshness(None, Some(("a.rs".into(), t(100)))),
            Freshness::Missing
        );
    }

    /// Finding nothing to compare is NOT a clean bill of health. Folding this into `Fresh` would
    /// switch the gate off without a word — the exact silence it exists to abolish.
    #[test]
    fn finding_no_source_at_all_is_not_fresh() {
        assert_eq!(freshness(Some(t(100)), None), Freshness::NoSources);
    }

    /// A throwaway workspace shaped like the real one — `crates/nxs` with a path dep, and a
    /// `target/debug/nxs` — so the gate can be driven end to end (manifest parse → closure walk →
    /// mtime compare → message) without a cargo build.
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        profile: PathBuf,
        dep_source: PathBuf,
        binary: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            let profile = root.join("target").join("debug");
            std::fs::create_dir_all(root.join("crates/nxs/src")).unwrap();
            std::fs::create_dir_all(root.join("crates/dep/src")).unwrap();
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(
                root.join("crates/nxs/Cargo.toml"),
                "[dependencies]\ndep = { path = \"../dep\" }\n",
            )
            .unwrap();
            std::fs::write(root.join("crates/nxs/src/main.rs"), "fn main() {}").unwrap();
            std::fs::write(
                root.join("crates/dep/Cargo.toml"),
                "[package]\nname=\"dep\"\n",
            )
            .unwrap();
            let dep_source = root.join("crates/dep/src/lib.rs");
            std::fs::write(&dep_source, "").unwrap();
            let binary = profile.join(format!("nxs{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(&binary, "").unwrap();
            Self {
                _dir: dir,
                root,
                profile,
                dep_source,
                binary,
            }
        }

        /// Every source the closure walk will see. Pinned explicitly in both states — a file left
        /// at its real creation time would tower over any epoch-based instant and decide the
        /// verdict by accident.
        fn sources(&self) -> [PathBuf; 2] {
            [
                self.root.join("crates/nxs/src/main.rs"),
                self.dep_source.clone(),
            ]
        }

        /// The binary predates a source it compiles — what a rebase leaves behind. Only
        /// `dep_source` is newer, so the verdict must name that file and no other.
        fn make_stale(&self) -> &Self {
            self.make_fresh();
            set_mtime(&self.binary, t(1_500));
            set_mtime(&self.dep_source, t(2_000));
            self
        }

        fn make_fresh(&self) -> &Self {
            for source in self.sources() {
                set_mtime(&source, t(1_000));
            }
            set_mtime(&self.binary, t(2_000));
            self
        }
    }

    /// The behaviour this whole crate exists to deliver: a stale binary ABORTS the test. Drives the
    /// real panic path, not a stand-in for it.
    #[test]
    #[should_panic(expected = "STALE MULTICALL BINARY")]
    fn a_stale_binary_panics_the_gate() {
        let fx = Fixture::new();
        panic_if_stale(complaint_in(&fx.make_stale().profile, &fx.root).as_deref());
    }

    #[test]
    fn a_fresh_binary_does_not_panic_the_gate() {
        let fx = Fixture::new();
        panic_if_stale(complaint_in(&fx.make_fresh().profile, &fx.root).as_deref());
    }

    /// The complaint has to name the file that outdated the binary and the command that fixes it —
    /// a gate that only says "stale" costs the reader the diagnosis it already did.
    #[test]
    fn the_complaint_names_the_newer_source_and_the_remedy() {
        let fx = Fixture::new();
        let complaint = complaint_in(&fx.make_stale().profile, &fx.root).expect("stale complains");
        assert!(complaint.contains("crates/dep/src/lib.rs"), "{complaint}");
        assert!(complaint.contains("cargo build -p nxs"), "{complaint}");
    }

    #[test]
    fn a_fresh_binary_yields_no_complaint() {
        let fx = Fixture::new();
        assert_eq!(complaint_in(&fx.make_fresh().profile, &fx.root), None);
    }

    /// The package-scoped run that never built `nxs` at all.
    #[test]
    fn an_absent_binary_complains_that_it_does_not_exist() {
        let fx = Fixture::new();
        fx.make_fresh();
        std::fs::remove_file(&fx.binary).unwrap();
        let complaint = complaint_in(&fx.profile, &fx.root).expect("a missing binary complains");
        assert!(complaint.contains("does not exist"), "{complaint}");
    }

    /// A blind gate must say so in its own words — not report "fresh", and not send the reader off
    /// to `cargo build -p nxs`, which cannot fix a gate that lost sight of the sources.
    #[test]
    fn a_closure_walk_that_finds_no_source_is_loud_not_silently_fresh() {
        let fx = Fixture::new();
        fx.make_fresh();
        std::fs::remove_dir_all(fx.root.join("crates/nxs/src")).unwrap();
        std::fs::remove_dir_all(fx.root.join("crates/dep/src")).unwrap();

        let complaint = complaint_in(&fx.profile, &fx.root).expect("a blind gate must complain");
        assert!(complaint.contains("GATE IS BLIND"), "{complaint}");
        assert!(
            !complaint.contains("Build the binary under test"),
            "a blind gate must not prescribe the rebuild — it cannot help: {complaint}"
        );
        assert!(
            complaint.contains("will not help"),
            "and it must say so outright, or the reader rebuilds anyway: {complaint}"
        );
    }

    #[test]
    fn path_deps_follows_platform_gated_dependencies() {
        let manifest = r#"
            [dependencies]
            plain = { path = "../plain" }
            [target.'cfg(unix)'.dependencies]
            only-on-unix = { path = "../only-on-unix" }
            libc = "0.2"
            [target.'cfg(windows)'.build-dependencies]
            winbuild = { path = "../winbuild" }
            [target.'cfg(unix)'.dev-dependencies]
            unix-only-dev = { path = "../unix-only-dev" }
        "#;
        let deps = path_deps(manifest);
        for linked in ["../plain", "../only-on-unix", "../winbuild"] {
            assert!(
                deps.contains(&linked.to_string()),
                "{linked} missing: {deps:?}"
            );
        }
        assert!(
            !deps.contains(&"../unix-only-dev".to_string()),
            "a platform-gated DEV dep still does not link in: {deps:?}"
        );
        assert_eq!(deps.len(), 3, "registry deps have no path: {deps:?}");
    }

    #[test]
    fn path_deps_follows_normal_and_build_deps_but_not_dev_deps() {
        let manifest = r#"
            [dependencies]
            nexus-flow-core = { path = "../core" }
            serde = "1"
            [build-dependencies]
            gen = { path = "../gen" }
            [dev-dependencies]
            nxs-test-support = { path = "../test-support" }
        "#;
        let deps = path_deps(manifest);
        assert!(deps.contains(&"../core".to_string()), "{deps:?}");
        assert!(deps.contains(&"../gen".to_string()), "{deps:?}");
        assert!(
            !deps.contains(&"../test-support".to_string()),
            "dev-dependency must not enter the closure: {deps:?}"
        );
        assert_eq!(deps.len(), 2, "registry deps have no path: {deps:?}");
    }

    /// Pins the real invariant against the real workspace: everything that links into `nxs` is
    /// watched, and everything that does not is NOT — a false positive there would be unfixable by
    /// rebuilding, since cargo would not relink `nxs`.
    #[test]
    fn closure_covers_the_personas_and_excludes_non_dependencies() {
        let root = workspace_root();
        let closure = nxs_closure(&root);
        let has = |c: &str| closure.contains(&root.join("crates").join(c));

        for linked in ["nxs", "cli", "memory", "chat", "core", "facade", "sync"] {
            assert!(has(linked), "{linked} links into nxs but is not watched");
        }
        for unlinked in ["server", "plugin-probe", "test-support"] {
            assert!(
                !has(unlinked),
                "{unlinked} does not link into nxs — watching it would go red with no rebuild that clears it"
            );
        }
    }

    #[test]
    fn newest_source_finds_the_latest_file_under_src_and_ignores_tests() {
        let dir = tempfile::tempdir().unwrap();
        let krate = dir.path().join("krate");
        std::fs::create_dir_all(krate.join("src/nested")).unwrap();
        std::fs::create_dir_all(krate.join("tests")).unwrap();
        std::fs::write(krate.join("src/lib.rs"), "").unwrap();
        std::fs::write(krate.join("src/nested/deep.rs"), "").unwrap();
        std::fs::write(krate.join("tests/it.rs"), "").unwrap();

        // Make `tests/it.rs` unambiguously the newest file on disk; the gate must still report the
        // deepest `src/` file, because a test edit does not relink the binary.
        let newer = SystemTime::now() + Duration::from_secs(3600);
        set_mtime(&krate.join("src/nested/deep.rs"), SystemTime::now());
        set_mtime(&krate.join("tests/it.rs"), newer);

        let found = newest_source(&BTreeSet::from([krate.clone()])).expect("a source");
        assert_eq!(found.0, krate.join("src/nested/deep.rs"));
    }

    /// The embedded guide docs are a watched build input — but only the compiled-in `en` tree
    /// (nexus-flow-e1qn). `crates/cli/src/commands/guide.rs` bakes `docs/guide/en/**` into `nxs`
    /// via `include_dir!`, and `crates/cli/build.rs` emits `rerun-if-changed=docs/guide/en`, so a
    /// guide edit (or add/remove) is refreshed by `cargo build -p nxs` — exactly what makes it safe
    /// to gate on. `docs/guide/de` is website-only, compiled into nothing: even as the newest file
    /// on disk it must NEVER decide the verdict, or the gate would go red with no rebuild that
    /// clears it.
    #[test]
    fn newest_source_watches_embedded_en_guide_docs_but_not_website_only_de() {
        let dir = tempfile::tempdir().unwrap();
        let krate = dir.path().join("cli");
        std::fs::create_dir_all(krate.join("src")).unwrap();
        std::fs::create_dir_all(krate.join("docs/guide/en")).unwrap();
        std::fs::create_dir_all(krate.join("docs/guide/de")).unwrap();
        std::fs::write(krate.join("src/lib.rs"), "").unwrap();
        let en = krate.join("docs/guide/en/getting-started.md");
        let de = krate.join("docs/guide/de/getting-started.md");
        std::fs::write(&en, "").unwrap();
        std::fs::write(&de, "").unwrap();

        set_mtime(&krate.join("src/lib.rs"), t(1_000));
        set_mtime(&en, t(2_000));
        // `de` is unambiguously the newest file on disk, yet it must be ignored: no rebuild
        // refreshes it, so watching it would strand the gate permanently red.
        set_mtime(&de, SystemTime::now() + Duration::from_secs(3_600));

        let found = newest_source(&BTreeSet::from([krate])).expect("a source");
        assert_eq!(
            found.0, en,
            "watch embedded en/**; ignore website-only de/**"
        );
    }

    fn set_mtime(p: &Path, t: SystemTime) {
        let f = std::fs::File::options().write(true).open(p).unwrap();
        f.set_modified(t).unwrap();
    }
}

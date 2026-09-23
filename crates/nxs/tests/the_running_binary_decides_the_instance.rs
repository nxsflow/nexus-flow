//! **Which service instance a process belongs to is decided by the running binary, not by the
//! directory it happens to stand in** (nxf 6j6v.cvpy).
//!
//! `crates/service` proves the rule at its seam: [`nxs_service::Instance::resolve_from`] is pure
//! over three inputs and every arm of it is a unit test. That is the mechanism. It is not the
//! defect.
//!
//! The defect was a WHOLE INVOCATION: `direnv` exports `NXS_SERVICE_INSTANCE` for a working copy,
//! so the `nxs`/`nxf`/`nxm`/`nxc` on the `PATH` — the INSTALLED files — became calls of a
//! development instance merely by being run there. Measured on the owner's machine on 2026-09-05:
//! all three registered services pointed at the same `~/.local/bin/nxs`, so the two development
//! services had never run a development build, while everyday commands in the checkout were
//! talking to a development registry.
//!
//! So every test here drives a REAL binary and reads what it says about itself, and the two halves
//! of the rule are run against ONE build placed in two places:
//!
//! - the built binary, in `target/<profile>`, is the development instance the variable names;
//! - the same bytes, standing where an installed binary stands, are the machine's own service —
//!   and no front door of theirs (`init`, `sync bind`) writes a development registry.
//!
//! **The fixture is never a symlink, and that is load-bearing.** `Origin` reads the CANONICAL path
//! of the running binary, so a symlink would resolve straight back into `target/<profile>` — which
//! is the truth for a service alias and exactly wrong for a stand-in for `~/.local/bin`. A hard
//! link is not resolved by `canonicalize`, so the fixture keeps its own path and costs one inode;
//! where the temporary directory is on another filesystem than the build (a checkout on an
//! external volume), it falls back to a copy, which has the same property.
//!
//! It lives in the system temporary directory and NOT under `target/`, which it did until the
//! review of PR #441: `Origin` now recognises a Cargo target directory by Cargo's own
//! `CACHEDIR.TAG` as well as by name, so anywhere under `target/` is a build tree by definition
//! and could no longer stand for an installed binary.
//!
//! **And it is a SUITE, not one file** — `nxs` plus the `nxf`/`nxm`/`nxc` links `install.sh` puts
//! beside it. The product resolves a suite binary as a SIBLING of the running one before it falls
//! back to the `PATH` (`nxs_init::spawn`), so a one-file fixture made `nxs init` reach for whatever
//! `nxc` happened to be on the developer's `PATH`: green on a machine with the suite installed, red
//! in CI, which is the fail-open shape this repository keeps closing. Found exactly that way, by CI.
//!
//! Unix-only for the same sentence: an installed suite IS `nxs` plus three symlinks, and a service
//! alias IS a symlink. The Windows lane runs `clippy`, not these tests.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use nxs_service::Origin;
use nxs_test_support::PinHome;
use tempfile::TempDir;

/// The instance name this repo's own `.envrc` exports — the one every invocation in this checkout
/// used to be dragged onto.
const DEV: &str = "nexus-flow-dev";

/// The freshly built `nxs`, in `target/<profile>` where Cargo put it.
fn built_nxs() -> PathBuf {
    nxs_test_support::assert_multicall_binary_fresh();
    cargo_bin("nxs")
}

/// The same binary, standing where an installed one stands. The `TempDir` is held so the fixture
/// is removed when the test ends.
struct FakeInstall {
    _dir: TempDir,
    nxs: PathBuf,
}

fn fake_install() -> FakeInstall {
    let built = built_nxs();
    let dir = TempDir::new().expect("a temporary directory");
    let nxs = dir.path().join("nxs");
    if std::fs::hard_link(&built, &nxs).is_err() {
        // Another filesystem — see the module doc. A copy keeps the one property that matters:
        // the fixture is its own file, so `canonicalize` cannot lead back to the build.
        std::fs::copy(&built, &nxs).expect("a copy of the built binary");
    }
    // The persona links, RELATIVE, exactly as `install.sh` writes them — see the module doc: the
    // suite resolves a sibling before the `PATH`, so without these the fixture is not an install.
    for persona in ["nxf", "nxm", "nxc"] {
        std::os::unix::fs::symlink("nxs", dir.path().join(persona)).expect("a persona link");
    }
    // The fixture checks ITSELF, because a fixture that quietly stopped standing for an installed
    // binary would turn every assertion below into a tautology about a build.
    let canonical = std::fs::canonicalize(&nxs).unwrap_or_else(|_| nxs.clone());
    assert_eq!(
        Origin::of(&canonical),
        Origin::Installed,
        "{} must stand where an installed binary stands",
        canonical.display()
    );
    assert_eq!(
        Origin::of(&std::fs::canonicalize(&built).unwrap_or(built)),
        Origin::Build,
        "and the binary it was linked from must still be a build"
    );
    FakeInstall { _dir: dir, nxs }
}

/// One invocation of `bin` under a fake `$HOME`, as a named instance (or with nothing named).
///
/// Built by hand rather than through `nxs_test_support::cargo_bin` for two reasons: that helper
/// deliberately CLEARS `NXS_SERVICE_INSTANCE`, which is exactly wrong for a suite whose subject is
/// the variable, and it always runs the BUILT binary, which is the other half of what is under
/// test here.
fn nxs_at(bin: &Path, home: &Path, cwd: &Path, instance: Option<&str>) -> Command {
    let mut c = Command::new(bin);
    c.current_dir(cwd).pin_home(home).env("NXC_TIMER", "dry");
    match instance {
        Some(name) => c.env("NXS_SERVICE_INSTANCE", name),
        None => c.env_remove("NXS_SERVICE_INSTANCE"),
    };
    c
}

/// `sync daemon status --json` — the reading that names the instance a process belongs to and the
/// home it would use, which is the whole question this ticket is about.
fn status_json(bin: &Path, home: &Path, instance: Option<&str>) -> serde_json::Value {
    let out = nxs_at(bin, home, home, instance)
        .args(["sync", "daemon", "status", "--json"])
        .output()
        .expect("nxs runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "status failed: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("--json emits JSON: {e}: {stdout}"))
}

/// **The everyday case, and the one that was wrong** (Definition of Done, first bullet): the
/// installed command line, run inside a working copy that exports a development instance, talks to
/// the INSTALLED service.
#[test]
fn the_installed_binary_answers_for_the_installed_service_though_the_checkout_names_another() {
    let home = TempDir::new().unwrap();
    let install = fake_install();

    let v = status_json(&install.nxs, home.path(), Some(DEV));

    assert_eq!(
        v["instance"], "nexus-flow",
        "an installed binary is the machine's own service: {v}"
    );
    assert_eq!(
        v["home"],
        home.path().join(".nexusflow").display().to_string(),
        "and it reads the production home, not the one the variable names: {v}"
    );
}

/// The other half of the same rule: the build in that same checkout IS the development instance —
/// the variable still says WHICH, it just no longer says WHETHER.
#[test]
fn the_build_in_the_checkout_is_the_development_instance_the_variable_names() {
    let home = TempDir::new().unwrap();

    let v = status_json(&built_nxs(), home.path(), Some(DEV));

    assert_eq!(v["instance"], DEV, "{v}");
    assert_eq!(
        v["home"],
        home.path().join(".nexusflow-dev").display().to_string(),
        "{v}"
    );
}

/// A build that names no instance is the developer's own machine — unchanged, and worth pinning
/// because it is what every `cargo run` and every test binary in this repository relies on.
#[test]
fn a_build_that_names_no_instance_is_still_the_production_one() {
    let home = TempDir::new().unwrap();
    let v = status_json(&built_nxs(), home.path(), None);
    assert_eq!(v["instance"], "nexus-flow", "{v}");
}

/// **A `bind` out of the installed binary lays down no development service** (Definition of Done,
/// third bullet).
///
/// This is the half that restored itself: `bind` registers the workspace with the service and
/// installs the agent, so one bind in a development working copy used to recreate the wrong
/// instance without anybody typing an install command. What is asserted is the REGISTRY — the file
/// a bind actually writes — because launchd is deliberately out of reach from a pinned `$HOME`
/// (nxf 6j6v.kvda), which is also why `--no-daemon` is passed rather than relied upon.
#[test]
fn no_front_door_of_the_installed_binary_writes_a_development_registry() {
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let install = fake_install();

    // `init` registers the workspace with the service too — both front doors, one test, because
    // the claim is about the binary and not about one verb.
    let out = nxs_at(&install.nxs, home.path(), ws.path(), Some(DEV))
        .args(["init", "--module", "chat", "--json"])
        .output()
        .expect("nxs runs");
    assert!(
        out.status.success(),
        "init failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let out = nxs_at(&install.nxs, home.path(), ws.path(), Some(DEV))
        .args(["sync", "bind", "--create", "--no-daemon", "--json"])
        .output()
        .expect("nxs runs");
    assert!(
        out.status.success(),
        "bind failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        home.path()
            .join(".nexusflow")
            .join("workspaces.toml")
            .is_file(),
        "the workspace must be registered with the installed service"
    );
    assert!(
        !home.path().join(".nexusflow-dev").exists(),
        "and nothing at all may be laid down for the development instance the checkout names"
    );
}

/// **`argv[0]` still beats everything, including an installed binary** — the third rule of the
/// owner's decision, and the reason a development instance that is already installed can still be
/// addressed at all.
///
/// It is what a launchd-started service depends on: `ProgramArguments` carries one element, so the
/// alias name is the only channel it has, and the alias may point at any binary anywhere.
///
#[test]
fn an_alias_names_its_own_instance_even_when_the_binary_behind_it_is_the_installed_one() {
    let home = TempDir::new().unwrap();
    let install = fake_install();
    let bin_dir = home.path().join(".nexusflow-dev").join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let alias = bin_dir.join(DEV);
    std::os::unix::fs::symlink(&install.nxs, &alias).expect("the alias links");

    // No variable at all: the NAME is the whole instruction, exactly as it is under launchd. The
    // alias stands for `nxs sync daemon`, so the verb is what follows it.
    let out = nxs_at(&alias, home.path(), home.path(), None)
        .args(["status", "--json"])
        .output()
        .expect("the alias runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("--json emits JSON");
    assert_eq!(
        v["instance"], DEV,
        "the alias it was started as decides, whatever the binary behind it is: {v}"
    );
}

/// **A job the service starts belongs to the service's instance** — the property
/// `Origin::ambient` canonicalises FOR, and the one this design's author named as its real risk.
///
/// The daemon runs its jobs by `exec`ing its own program — the alias — with `argv[0]` FORCED to
/// `nxs` so the verb routes (`RealSpawn::spawn` in `crates/nxs/src/sync/daemon.rs`), which throws
/// away the one name that could have identified the instance. What is left is a path that LIES
/// about where it lives (`~/.nexusflow-dev/bin/…`, no build tree in sight) and tells the truth
/// about what it is (`target/<profile>/nxs`). Judged unresolved, every such job would resolve to
/// production and write into the shared registry while looking isolated — "half-isolated, which is
/// worse than not isolated, because it looks right".
///
/// **Added by the review of PR #441 (Test Quality #1), which found the claim untested.** The unit
/// test that named this property only compared two hardcoded strings, and the black-box test above
/// invokes the alias under its OWN name, so `resolve_from` answers at step 1 and never consults
/// `Origin` at all.
///
/// On macOS this fails the moment `running_program` stops canonicalising, because `current_exe()`
/// there hands back the path it was `exec`ed with, symlink and all. On Linux `/proc/self/exe` is
/// already resolved, so the same assertion holds by the platform's own doing — the test is honest
/// on both and load-bearing on the one where the service actually runs.
#[test]
fn a_job_started_through_a_development_alias_belongs_to_that_instance_and_not_to_production() {
    use std::os::unix::process::CommandExt as _;

    let home = TempDir::new().unwrap();
    let bin_dir = home.path().join(".nexusflow-dev").join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let alias = bin_dir.join(DEV);
    std::os::unix::fs::symlink(built_nxs(), &alias).expect("the alias links to the BUILD");
    assert_eq!(
        Origin::of(&alias),
        Origin::Installed,
        "the alias's own path is not in a build tree — resolving it is the whole mechanism"
    );

    let mut cmd = nxs_at(&alias, home.path(), home.path(), Some(DEV));
    // Exactly what the daemon does to its children, and exactly what makes this the hard case.
    cmd.arg0("nxs").args(["sync", "daemon", "status", "--json"]);
    let out = cmd.output().expect("the alias runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("--json emits JSON");

    assert_eq!(
        v["instance"], DEV,
        "a child that lost the alias NAME must still be the alias's instance, by the file it \
         resolves to: {v}"
    );
    assert_eq!(
        v["home"],
        home.path().join(".nexusflow-dev").display().to_string(),
        "and it must read that instance's home, not the production one: {v}"
    );
}

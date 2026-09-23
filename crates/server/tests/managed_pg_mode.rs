//! The `managed-pg mode` command's **live wiring** (PR #321 review, Test Quality #1 + #2).
//!
//! `managed_pg::ProofMode` decides, and its decision table is unit-tested next to the code. What
//! that leaves untested is everything between the decision and the run page: writing `mode=` to
//! `$GITHUB_OUTPUT`, choosing `::warning::` over `::error::`, the exit code, and — the one the
//! review found — what happens when publishing the decision *fails*.
//!
//! That last one is not hypothetical. Every managed step in the job is guarded on
//! `steps.proof.outputs.mode == 'managed'`, so a run that decides `managed` and then cannot say
//! so skips **all** of them and reports **green**, having proved nothing. It is the exact trap the
//! whole ticket exists to close, reached through a broken runner instead of through a fork. Until
//! the review of PR #321 the failure was printed and swallowed.
//!
//! These drive the REAL example binary, because the seam under test is the process boundary: its
//! exit code, its stdout, and the files it appends to. An example's own `#[cfg(test)]` module
//! would never run — `cargo test` builds examples but does not run tests inside them — so this is
//! the only place this code can be exercised at all.

#![cfg(feature = "postgres")]

use std::path::PathBuf;
use std::process::{Command, Output};

/// A fork's pull request, in the shape GitHub writes to `GITHUB_EVENT_PATH`.
const FORK_EVENT: &str =
    r#"{"pull_request":{"head":{"repo":{"full_name":"someone/nexus-flow","fork":true}}}}"#;
/// The same, from a branch in this repository.
const SAME_REPO_EVENT: &str =
    r#"{"pull_request":{"head":{"repo":{"full_name":"nxsflow/nexus-flow","fork":false}}}}"#;

/// The compiled `managed-pg` example, refusing to hand back a stale one.
///
/// Derived from this package's own bin target rather than guessed, so it follows the profile
/// (`debug`/`release`) the test itself was built into — a hardcoded `target/debug` would silently
/// exercise the wrong binary under `cargo test --release`.
///
/// The freshness check is not belt-and-braces, it is the [`nexus-flow-0yyw`] trap in a new place,
/// and it bit while these very tests were being written: **`cargo test --test managed_pg_mode`
/// does not rebuild the example.** A single-target run compiles the test harness, finds a
/// `managed-pg` from some earlier build, and asserts against it — so the regression test for a
/// fix passes just as happily with the fix removed. Loud failure beats a green run that proves
/// nothing, which is, after all, this whole file's subject.
///
/// [`nexus-flow-0yyw`]: crates/test-support/src/lib.rs
fn managed_pg() -> PathBuf {
    let binary = assert_cmd::cargo::cargo_bin("nxf-relay")
        .parent()
        .expect("the bin target always has a parent directory")
        .join("examples")
        .join(format!("managed-pg{}", std::env::consts::EXE_SUFFIX));

    let built = std::fs::metadata(&binary)
        .and_then(|m| m.modified())
        .unwrap_or_else(|e| {
            panic!(
                "\n\n{} is not there: {e}\n\n\
                 Build it, then re-run:\n\n    \
                 cargo test -p nxs-server --features postgres\n\n\
                 (a `--test <name>` run does NOT build examples.)\n",
                binary.display()
            )
        });

    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(newer) =
        newest_source_after(&[crate_dir.join("src"), crate_dir.join("examples")], built)
    {
        panic!(
            "\n\nSTALE `managed-pg` EXAMPLE — this run would have tested foreign code.\n\n  \
             {} is OLDER than {}.\n\n\
             `cargo test --test <name>` builds the test harness but NOT the crate's examples, so \
             the binary these tests drive can be arbitrarily old. The run goes green while \
             asserting against code you did not write.\n\n\
             Rebuild it, then re-run:\n\n    \
             cargo test -p nxs-server --features postgres\n\n\
             (or `cargo build -p nxs-server --features postgres --example managed-pg`. \
             `cargo test --all` does it for you. Same class as nexus-flow-0yyw.)\n",
            binary.display(),
            newer.display(),
        );
    }
    binary
}

/// The first source under `roots` that is newer than `built`, if any.
fn newest_source_after(roots: &[PathBuf], built: std::time::SystemTime) -> Option<PathBuf> {
    let mut pending: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => pending.push(path),
                Ok(_) => {
                    if std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .is_ok_and(|m| m > built)
                    {
                        return Some(path);
                    }
                }
                Err(_) => continue,
            }
        }
    }
    None
}

/// One run of `managed-pg mode`, in a job-shaped environment.
///
/// Every variable the command reads is set explicitly — including the ones being *removed* — so a
/// test can never pass because the developer's own shell happened to export `DATABASE_URL`.
struct Run {
    output: Output,
    github_output: String,
    step_summary: String,
}

impl Run {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    fn failed(&self) -> bool {
        !self.output.status.success()
    }
}

struct Job {
    event_name: &'static str,
    event_payload: Option<&'static str>,
    database_url: Option<&'static str>,
    /// `${{ github.event.pull_request.head.repo.fork }}` as the runner renders it: `"true"`,
    /// `"false"`, or the empty string on an event that carries no pull request. `None` models the
    /// `env:` line having been removed from the job.
    head_is_fork: Option<&'static str>,
    /// Overrides `$GITHUB_OUTPUT`, so a test can point it somewhere unwritable.
    github_output: Option<PathBuf>,
}

impl Job {
    fn new(event_name: &'static str) -> Job {
        Job {
            event_name,
            event_payload: None,
            database_url: None,
            head_is_fork: Some(""),
            github_output: None,
        }
    }

    fn pull_request_from_a_fork() -> Job {
        Job {
            event_payload: Some(FORK_EVENT),
            head_is_fork: Some("true"),
            ..Job::new("pull_request")
        }
    }

    fn pull_request_from_this_repo() -> Job {
        Job {
            event_payload: Some(SAME_REPO_EVENT),
            head_is_fork: Some("false"),
            ..Job::new("pull_request")
        }
    }

    fn with_endpoint(mut self) -> Job {
        // Never dialled: `mode` decides from the environment and returns. A syntactically real URL
        // keeps the test honest about what `configured_url` accepts.
        self.database_url = Some("postgres://someone:hunter2@db.example.test:5432/postgres");
        self
    }

    fn run(self) -> Run {
        let dir = tempfile::tempdir().expect("tempdir");
        let out_path = self
            .github_output
            .unwrap_or_else(|| dir.path().join("github_output"));
        let summary_path = dir.path().join("step_summary");
        let event_path = dir.path().join("event.json");
        if let Some(payload) = self.event_payload {
            std::fs::write(&event_path, payload).expect("write event payload");
        }

        let mut cmd = Command::new(managed_pg());
        cmd.arg("mode")
            .env_remove("DATABASE_URL")
            .env_remove("NXF_CI_HEAD_IS_FORK")
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_EVENT_NAME", self.event_name)
            .env("GITHUB_OUTPUT", &out_path)
            .env("GITHUB_STEP_SUMMARY", &summary_path);
        if self.event_payload.is_some() {
            cmd.env("GITHUB_EVENT_PATH", &event_path);
        }
        if let Some(url) = self.database_url {
            cmd.env("DATABASE_URL", url);
        }
        if let Some(fork) = self.head_is_fork {
            cmd.env("NXF_CI_HEAD_IS_FORK", fork);
        }

        let output = cmd.output().expect("run managed-pg mode");
        Run {
            github_output: std::fs::read_to_string(&out_path).unwrap_or_default(),
            step_summary: std::fs::read_to_string(&summary_path).unwrap_or_default(),
            output,
        }
    }
}

/// THE regression the review found, and the reason this file exists.
///
/// A genuine `managed` run whose decision cannot be published must go RED. If it exits 0, every
/// guarded step below it is skipped and the job reports green having proved nothing about the
/// managed endpoint — indistinguishable, to a reader, from a run that proved everything.
#[test]
fn a_decision_that_cannot_be_published_fails_the_job_instead_of_reporting_green() {
    let run = Job {
        // A path whose parent does not exist: the append cannot be created, exactly as it would
        // fail on a runner with a broken or read-only workspace.
        github_output: Some(
            PathBuf::from("this-directory-does-not-exist-6j6v-waeb").join("github_output"),
        ),
        ..Job::pull_request_from_this_repo().with_endpoint()
    }
    .run();

    assert!(
        run.failed(),
        "a run that could not publish `mode` must be red, not green — stdout: {}, stderr: {}",
        run.stdout(),
        run.stderr()
    );
    assert!(
        run.stderr().contains("PROOF MODE COULD NOT BE PUBLISHED"),
        "the reason has to be readable without opening the diff: {}",
        run.stderr()
    );
    assert!(
        run.stdout().contains("::error::"),
        "and it belongs at the top of the run page: {}",
        run.stdout()
    );
}

/// The ordinary path: an endpoint is configured, so the suites below this step must run.
#[test]
fn a_configured_endpoint_publishes_managed_and_lets_the_suites_through() {
    let run = Job::pull_request_from_this_repo().with_endpoint().run();

    assert!(!run.failed(), "stderr: {}", run.stderr());
    assert!(
        run.github_output.contains("mode=managed"),
        "the guards below read this: {:?}",
        run.github_output
    );
    // Nothing was skipped, so there is no verdict to render and nothing to warn about.
    assert!(!run.stdout().contains("::warning::"), "{}", run.stdout());
    assert!(!run.stdout().contains("::error::"), "{}", run.stdout());
}

/// A fork's pull request: green, because the contributor has to be able to merge — but never
/// silent, because a skip that renders like a pass is what the blocking decision exists to close.
#[test]
fn a_fork_pull_request_stays_green_and_says_what_it_could_not_prove() {
    let run = Job::pull_request_from_a_fork().run();

    assert!(!run.failed(), "stderr: {}", run.stderr());
    assert!(
        run.github_output.contains("mode=fork-pull-request"),
        "{:?}",
        run.github_output
    );
    assert!(
        run.stdout().contains("::warning::"),
        "a declared skip has to reach the top of the run page: {}",
        run.stdout()
    );
    assert!(
        !run.stdout().contains("::error::"),
        "…as a warning, not an error — the job is green on purpose: {}",
        run.stdout()
    );
    assert!(
        run.step_summary.contains("NOT PROVEN"),
        "and the run page's summary has to say so too: {:?}",
        run.step_summary
    );
}

/// The door the fork branch must not become. A push to `main` — the event a release is cut from —
/// with no endpoint is a hard failure, not a declared skip.
#[test]
fn a_missing_endpoint_outside_a_fork_is_still_a_hard_failure() {
    let run = Job::new("push").run();

    assert!(run.failed(), "stdout: {}", run.stdout());
    assert!(
        run.github_output.contains("mode=not-configured"),
        "{:?}",
        run.github_output
    );
    assert!(
        run.stderr().contains("NO MANAGED POSTGRES CONFIGURED"),
        "{}",
        run.stderr()
    );
}

/// The tamper detector (review, Integrity #2). Contributor code compiles in this job before the
/// decision is made, so the event payload — a file on the runner — is reachable by a build script.
/// The runner's own context is not. When the two disagree, neither is believed.
#[test]
fn an_event_payload_that_contradicts_the_runner_fails_the_job() {
    let run = Job {
        // The payload claims a fork; the runner says this pull request is from the repository —
        // the shape a build script would forge to buy itself a skipped proof.
        head_is_fork: Some("false"),
        ..Job::pull_request_from_a_fork()
    }
    .run();

    assert!(
        run.failed(),
        "a forged fork claim must not be resolved in either side's favour: {}",
        run.stdout()
    );
    assert!(
        run.stderr().contains("FORK SIGNALS DISAGREE"),
        "{}",
        run.stderr()
    );
    assert!(
        !run.github_output.contains("mode=fork-pull-request"),
        "and it must never publish the mode it was trying to forge: {:?}",
        run.github_output
    );
}

/// A check that quietly stops checking is worth less than no check at all: if the `env:` line
/// carrying the runner's view is removed from the job, that is a failure, not a fallback.
#[test]
fn a_missing_cross_check_signal_fails_rather_than_falls_back() {
    let run = Job {
        head_is_fork: None,
        ..Job::pull_request_from_a_fork()
    }
    .run();

    assert!(run.failed(), "stdout: {}", run.stdout());
    assert!(
        run.stderr().contains("FORK SIGNALS DISAGREE"),
        "{}",
        run.stderr()
    );
}

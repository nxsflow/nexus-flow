//! The Supabase job's three moving parts, as one command (nexus-flow-6j6v.da42).
//!
//! The managed-Postgres job BLOCKS (owner decision, 2026-08-10), so a red run costs a human their
//! attention. What they get for it is this: a verdict that is already decided, in the run's own
//! output, instead of a stack of driver errors they have to interpret.
//!
//!   mode       — BEFORE anything else: which proof can this run even give? A pull request from a
//!                fork carries no secrets (GitHub's design, not a setting), so it declares that
//!                out loud instead of skipping quietly. See `managed_pg::ProofMode`.
//!   preflight  — before any suite: is the endpoint there, and usable the way the suites need?
//!                Exits non-zero if not, so the FAILING STEP'S NAME already carries the answer.
//!   classify   — after a suite failed: was the endpoint still there? If it was, the failure can
//!                only be the integration, which is the case that must never ship.
//!   sweep      — drop scratch schemas old enough that no run can still hold them, so a
//!                persistent project does not grow a schema per test forever.
//!
//! An example rather than a test binary on purpose: the output IS the product here, and a test
//! harness wraps it in panics and thread names. An example owns its stdout and its exit code.
//!
//! It reads the same `DATABASE_URL` the suites read, so preflight and suites can never disagree
//! about which database they are talking about.

use std::process::ExitCode;
use std::time::Duration;

use nxs_server::managed_pg::{
    self, ProofMode, Verdict, DEFAULT_PROBE_TIMEOUT, DEFAULT_SCRATCH_MAX_AGE, SCRATCH_SCHEMA_PREFIX,
};

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_default();
    // `mode` is the one command that must run when there is NO endpoint — deciding whether that
    // is forgivable is its entire job — so it is dispatched before the others' hard requirement.
    if command == "mode" {
        return proof_mode();
    }
    let Some(url) = database_url(&command) else {
        return ExitCode::FAILURE;
    };
    match command.as_str() {
        "preflight" => preflight(&url),
        "classify" => classify(&url),
        "sweep" => sweep(&url),
        other => {
            eprintln!("usage: managed-pg <mode|preflight|classify|sweep>  (got {other:?})");
            ExitCode::FAILURE
        }
    }
}

/// Which proof can this run give? (nexus-flow-6j6v.waeb)
///
/// Writes `mode=<token>` to `$GITHUB_OUTPUT`, which every managed step in the job is guarded on.
/// The three answers, and what each costs:
///
/// * **managed** — silent, and the suites follow. The verdict a reader wants is the preflight's.
/// * **fork-pull-request** — green, with a `::warning::` annotation and a summary block. Green
///   because a contributor cannot fix GitHub's secret policy and must be able to merge; annotated
///   because a skip that renders like a pass is the thing the blocking decision exists to prevent.
/// * **not-configured** — red, exactly as before. The fork branch forgives one named situation,
///   not "no endpoint" in general.
///
/// The reasoning, the words and the exit-code rule all live in `managed_pg::ProofMode`, where they
/// are unit-tested; what is left here is which stream they go to.
fn proof_mode() -> ExitCode {
    let configured = managed_pg::configured_url(std::env::var("DATABASE_URL").ok().as_deref());
    let event = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default();
    let fork = managed_pg::is_fork_pull_request(
        (!event.is_empty()).then_some(event.as_str()),
        github_event_payload().as_deref(),
    );
    if let Err(code) = cross_check_fork_signal(fork) {
        return code;
    }
    let mode = managed_pg::proof_mode(configured.as_deref(), fork);

    // BEFORE the verdict is rendered, and fatal on its own. Every managed step is guarded on this
    // value, so a run that computed `managed` and then failed to say so skips all of them and
    // reports GREEN — the exact trap this command exists to close, reached through a broken
    // runner instead of through a fork. Printing the error and carrying on (which is what this
    // did until the review of PR #321) leaves the job green with an annotation nobody has to act
    // on.
    if let Err(e) = emit_output("mode", mode.token()) {
        let banner = managed_pg::banner(
            "PROOF MODE COULD NOT BE PUBLISHED",
            "The decision was made, but writing it to $GITHUB_OUTPUT failed — so every step \
             guarded on it would be skipped and this job would report green having proved nothing.",
            "This is a runner or workspace fault, not a code failure: check $GITHUB_OUTPUT and \
             re-run. Nothing about the managed endpoint is proven either way by this run.",
            "detail",
            &format!("could not write mode={} : {e}", mode.token()),
        );
        eprintln!("{banner}");
        summary(&banner);
        annotate("::error", "PROOF MODE COULD NOT BE PUBLISHED", &e);
        return ExitCode::FAILURE;
    }

    // Which run it was, but only where it helps: the failure is the one a maintainer has to trace
    // back to an event, while a contributor reading the fork verdict already knows what they did.
    let context = if mode.is_failure() {
        let event = if event.is_empty() { "none" } else { &event };
        format!("no DATABASE_URL in the environment (event `{event}`)")
    } else {
        String::new()
    };

    let Some(banner) = mode.banner(&context) else {
        println!("managed endpoint configured — the suites below prove it or fail trying");
        return ExitCode::SUCCESS;
    };
    eprintln!("{banner}");
    summary(&banner);
    if mode.is_failure() {
        annotate("::error", mode.label(), &context);
        return ExitCode::FAILURE;
    }
    // A warning, not an error: the job is green on purpose. It still shows at the TOP of the run
    // page, which is the whole difference between a declared skip and a missing one.
    annotate(
        "::warning",
        mode.label(),
        "the managed-Postgres proof was skipped on purpose; it blocks on every push to main",
    );
    ExitCode::SUCCESS
}

/// The event GitHub wrote for this run, if there is one.
fn github_event_payload() -> Option<String> {
    std::fs::read_to_string(std::env::var_os("GITHUB_EVENT_PATH")?).ok()
}

/// `DATABASE_URL`, or a loud failure.
///
/// A missing endpoint is a **hard** failure, never a skip. This job exists to prove something; a
/// run that quietly proves nothing and reports green is the exact failure mode the whole ticket is
/// about.
///
/// Reached only when the `mode` guard above was bypassed or removed — the workflow never runs
/// these commands in a mode that has no endpoint. Kept anyway: it is the check that survives
/// someone editing the `if:` conditions out of the job.
fn database_url(command: &str) -> Option<String> {
    match managed_pg::configured_url(std::env::var("DATABASE_URL").ok().as_deref()) {
        Some(url) => Some(url),
        None => {
            let context = format!("no DATABASE_URL in the environment (running `{command}`)");
            let banner = ProofMode::NotConfigured
                .banner(&context)
                .expect("NotConfigured always renders a verdict block");
            eprintln!("{banner}");
            summary(&banner);
            annotate("::error", ProofMode::NotConfigured.label(), &context);
            None
        }
    }
}

/// The runner's own answer to the same question the event payload answered, or a hard failure.
///
/// Required whenever this runs inside GitHub Actions: an absent signal means the `env:` line that
/// carries it was removed from the job, and a check that quietly stops checking is worth less
/// than no check at all. Outside Actions (a local run) there is nothing to cross-check against,
/// so the payload stands alone.
fn cross_check_fork_signal(payload_fork: bool) -> Result<(), ExitCode> {
    if std::env::var_os("GITHUB_ACTIONS").is_none() {
        return Ok(());
    }
    let context = std::env::var(FORK_CONTEXT_ENV).ok();
    let detail = match context.as_deref() {
        Some(context) if managed_pg::fork_signals_agree(payload_fork, context) => return Ok(()),
        Some(context) => format!(
            "the event payload says fork={payload_fork}, the runner's context says {context:?} \
             ({FORK_CONTEXT_ENV})"
        ),
        None => format!(
            "{FORK_CONTEXT_ENV} is not set, so the runner's own view of fork-ness could not be \
             compared against the event payload"
        ),
    };
    let banner = managed_pg::banner(
        "FORK SIGNALS DISAGREE",
        "Two independent readings of the same fact — is this a pull request from a fork? — do not \
         match, so which proof this run owes cannot be decided.",
        "Do NOT re-run and hope. The event payload is a file on this runner, written before \
         contributor code compiled in this job; the context signal is not. A disagreement means \
         one of them was changed. Read the diff, starting with any build script or proc-macro.",
        "detail",
        &detail,
    );
    eprintln!("{banner}");
    summary(&banner);
    annotate("::error", "FORK SIGNALS DISAGREE", &detail);
    Err(ExitCode::FAILURE)
}

/// `${{ github.event.pull_request.head.repo.fork }}`, handed over in the step's own `env:` — the
/// one place a value written to `$GITHUB_ENV` by a build script cannot displace it.
const FORK_CONTEXT_ENV: &str = "NXF_CI_HEAD_IS_FORK";

/// A step output, so the workflow can branch on the decision this binary made rather than
/// re-deriving it in YAML where nothing tests it. Printed when there is no `$GITHUB_OUTPUT`, so a
/// local run still shows its answer.
///
/// The error is RETURNED, never just logged: the caller has to turn it into a failing exit code,
/// because a job that cannot publish its decision is a job whose every guarded step is skipped.
fn emit_output(key: &str, value: &str) -> Result<(), String> {
    let Some(path) = std::env::var_os("GITHUB_OUTPUT") else {
        println!("{key}={value}");
        return Ok(());
    };
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|e| format!("could not open {}: {e}", path.to_string_lossy()))?;
    writeln!(file, "{key}={value}")
        .map_err(|e| format!("could not write to {}: {e}", path.to_string_lossy()))
}

fn preflight(url: &str) -> ExitCode {
    let report = managed_pg::probe(url, probe_timeout());
    eprintln!("{}", report.banner());
    summary(&report.banner());
    if report.verdict.is_ok() {
        println!("managed Postgres reachable — any failure after this point is the integration");
        return ExitCode::SUCCESS;
    }
    annotate("::error", report.verdict.label(), &report.detail);
    ExitCode::FAILURE
}

/// A suite has already failed. The only question left is whether the endpoint was still there —
/// that is what separates "the provider had a bad minute" from "we broke it".
///
/// Always exits 0: the job is red already, from the step that actually failed. A second red step
/// here would only add noise to the thing this command exists to make clear.
fn classify(url: &str) -> ExitCode {
    let report = managed_pg::probe(url, probe_timeout());
    let (label, what, todo) = match report.verdict {
        Verdict::Reachable => (
            "INTEGRATION BROKEN",
            "A suite failed while the managed endpoint was reachable, before and after the run.",
            "This is a REAL failure: read the failing test above. Do not ship on the assumption \
             that the provider had a bad minute — it did not.",
        ),
        Verdict::Unreachable => (
            "MANAGED POSTGRES UNREACHABLE",
            "A suite failed and the managed endpoint is NOT reachable now.",
            "Check the provider's status page, then re-run. Nothing about the integration is \
             proven either way by this run.",
        ),
        Verdict::Misconfigured => (
            "MANAGED POSTGRES MISCONFIGURED",
            "A suite failed and the endpoint rejects us — the configuration, not the code.",
            "Fix the secret or the project (see the detail below), then re-run.",
        ),
        Verdict::Unclassified => (
            "UNCLASSIFIED FAILURE",
            "A suite failed and the endpoint failed in a way this classifier does not recognise.",
            "Classify it by hand from the detail below. Do NOT assume availability — an \
             unrecognised failure is the shape a real break takes.",
        ),
    };
    report_with_detail(label, what, todo, &report.detail);
    ExitCode::SUCCESS
}

fn sweep(url: &str) -> ExitCode {
    let max_age = scratch_max_age();
    match managed_pg::sweep_scratch_schemas(url, max_age, probe_timeout()) {
        Ok(dropped) if dropped.is_empty() => {
            println!(
                "no scratch schemas older than {}s to drop (prefix {SCRATCH_SCHEMA_PREFIX})",
                max_age.as_secs()
            );
            ExitCode::SUCCESS
        }
        Ok(dropped) => {
            println!(
                "dropped {} scratch schema(s) older than {}s:",
                dropped.len(),
                max_age.as_secs()
            );
            for name in &dropped {
                println!("  {name}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            // A sweep that cannot run is not a reason to hide it: the project grows silently
            // until someone notices the schema list, which is precisely the shape of leak this
            // exists to prevent.
            annotate("::error", "SCRATCH SCHEMA SWEEP FAILED", &e.0);
            eprintln!("scratch schema sweep failed: {}", e.0);
            ExitCode::FAILURE
        }
    }
}

fn probe_timeout() -> Duration {
    env_secs("NXF_PROBE_TIMEOUT_SECS").unwrap_or(DEFAULT_PROBE_TIMEOUT)
}

fn scratch_max_age() -> Duration {
    env_secs("NXF_SCRATCH_SCHEMA_MAX_AGE_SECS").unwrap_or(DEFAULT_SCRATCH_MAX_AGE)
}

fn env_secs(key: &str) -> Option<Duration> {
    std::env::var(key)
        .ok()?
        .parse()
        .ok()
        .map(Duration::from_secs)
}

fn report_with_detail(label: &str, what: &str, todo: &str, detail: &str) {
    // The module owns the LAYOUT (`managed_pg::banner`), this owns only the words. Two copies of a
    // block a human is meant to recognise at a glance is exactly what drifts.
    let banner = managed_pg::banner(label, what, todo, "detail", detail);
    eprintln!("{banner}");
    summary(&banner);
    annotate("::error", label, detail);
}

/// A GitHub Actions annotation — the line that shows at the TOP of the run page, above the logs.
/// The flattening lives in the module (`managed_pg::annotation_line`), where it is tested; an
/// example binary's own `#[cfg(test)]` would never be RUN, since `cargo test` builds examples but
/// does not run tests inside them.
///
/// `severity` is `::error` for everything that turns the job red and `::warning` for the one
/// outcome that does not — a fork's declared skip. Using `::error` there too would put a red
/// marker on a green job, and a marker that contradicts the job's own result is one readers learn
/// to ignore, which would cost exactly the attention this annotation exists to buy.
fn annotate(severity: &str, label: &str, detail: &str) {
    if std::env::var_os("GITHUB_ACTIONS").is_none() {
        return;
    }
    println!("{severity}::{}", managed_pg::annotation_line(label, detail));
}

/// The job summary — rendered as Markdown on the run page, which is where a human looks first.
///
/// A failure here is reported but not fatal, and the asymmetry with [`emit_output`] is the point:
/// nothing branches on the summary, so losing it costs a reader one of three places the verdict
/// appears (the log and the annotation still carry it). It is still said out loud — a verdict
/// that silently failed to reach the run page is how a reader concludes there was nothing to say.
fn summary(banner: &str) {
    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return;
    };
    use std::io::Write;
    let written = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "```\n{banner}\n```"));
    if let Err(e) = written {
        eprintln!(
            "could not write the verdict to GITHUB_STEP_SUMMARY ({}): {e} — it is in this log and \
             in the run's annotations either way",
            path.to_string_lossy()
        );
    }
}

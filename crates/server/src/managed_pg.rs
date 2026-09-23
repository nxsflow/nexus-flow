//! Reaching a **managed** Postgres — Supabase, Neon, RDS — and being honest about what went
//! wrong when we cannot (nexus-flow-6j6v.da42).
//!
//! ## Why this module exists at all
//!
//! The Supabase suites BLOCK (owner decision, 2026-08-10): a run that cannot prove the managed
//! endpoint works goes red, and a human looks at it before anything ships. That decision only
//! pays off if the human can tell, *in seconds*, which of two very different things happened:
//!
//! * **The provider was not there** — DNS, TCP, TLS, a timeout, a connection quota. Nothing is
//!   wrong with nexus-flow; check availability and re-run.
//! * **The integration is broken** — we connected fine and the behaviour is wrong. This is the
//!   one that must never ship.
//!
//! A failure that renders both the same way makes the rule unusable: by the third red run it
//! gets waved through, and the silent break is back, only more expensive. So the classification
//! is a first-class, tested artifact, not a comment in a workflow file.
//!
//! [`probe`] answers the first question on its own, *before* any suite runs. If it says
//! `REACHABLE`, then a subsequent test failure can only be the second case — which is precisely
//! what makes the second case identifiable without any cleverness in the suites themselves.
//!
//! It reports a third bucket the two-way split hides, and it is the likeliest first-run failure
//! of all: **`MISCONFIGURED`** — we reached the server and it rejected *us* (bad password, wrong
//! database, no permission, startup options swallowed by a transaction-mode pooler). Calling that
//! "unreachable" would send someone to a status page that says everything is fine.
//!
//! ## Scratch-schema hygiene
//!
//! The parity suite isolates every test in its own schema. Against a per-run service container
//! that is free; against a persistent managed project it is a leak — the project grows a schema
//! per test, forever. [`scratch_schema_name`] therefore stamps the creation time into the name
//! and [`sweep_scratch_schemas`] drops the ones that are old enough that no run can still be
//! using them. Age, not ownership, is what makes the sweep safe to run while other CI runs are
//! in flight.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::store::{StoreError, StoreResult};
use crate::store_pg::off_runtime;
use crate::tls_pg::make_tls;

/// What a human should do about it. The whole point of the module: three answers that lead to
/// three different actions, never one answer that leads to a shrug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Connected, authenticated, and usable the way the suites need it.
    Reachable,
    /// The endpoint was not there. Not a nexus-flow defect — check the provider, re-run.
    Unreachable,
    /// The endpoint was there and rejected us, or cannot serve us the way we need. The
    /// configuration (secret, project, pooler mode) is what has to change.
    Misconfigured,
    /// We could not tell. Never guessed: an unrecognised failure that gets filed under
    /// "provider was down" is exactly how a real break ships.
    Unclassified,
}

impl Verdict {
    /// The single word a reader scans for.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Reachable => "REACHABLE",
            Verdict::Unreachable => "MANAGED POSTGRES UNREACHABLE",
            Verdict::Misconfigured => "MANAGED POSTGRES MISCONFIGURED",
            Verdict::Unclassified => "MANAGED POSTGRES: UNCLASSIFIED FAILURE",
        }
    }

    /// Whether this verdict means the run proved nothing about the integration.
    pub fn is_ok(self) -> bool {
        self == Verdict::Reachable
    }
}

/// The specific thing that went wrong. Each maps to exactly one [`Verdict`], so a new cause
/// cannot quietly land in the wrong bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// Connected and usable.
    None,
    /// The connection string does not parse as a libpq URL.
    Url,
    /// The host name did not resolve.
    Dns,
    /// TCP never came up: refused, unreachable, reset.
    Connect,
    /// The attempt ran past the deadline — a black hole rather than a refusal.
    Timeout,
    /// TLS did not complete, or the server declined to speak it.
    Tls,
    /// The TLS connector could not even be BUILT — an unreadable `NXF_RELAY_PG_CA_FILE`, a file
    /// that is not PEM, a PEM with no certificate in it. No packet ever left this machine, so
    /// calling it a reachability problem would send a reader to a status page for a typo in their
    /// own environment.
    TlsSetup,
    /// The handshake got far enough for the server to present a certificate, and we did not trust
    /// it. Split out from [`Cause::Tls`] by the very first run against a real Supabase project:
    /// an outage cannot produce this. The server was demonstrably there and talking — what is
    /// missing is a trust anchor on our side.
    TlsTrust,
    /// The server answered and said it has no room: connection limit or quota.
    Capacity,
    /// The server answered and said it is not accepting connections yet.
    Starting,
    /// The credentials in the URL were rejected.
    Credentials,
    /// The named database or role does not exist.
    Database,
    /// Authenticated, but not allowed to do what the suites need.
    Permission,
    /// Connected, but libpq startup `options` were dropped, so per-connection `search_path`
    /// does not arrive — the transaction-mode-pooler trap.
    StartupOptions,
    /// Something the classifier does not know.
    Unknown,
}

impl Cause {
    fn verdict(self) -> Verdict {
        match self {
            Cause::None => Verdict::Reachable,
            Cause::Dns | Cause::Connect | Cause::Timeout | Cause::Tls => Verdict::Unreachable,
            Cause::Capacity | Cause::Starting => Verdict::Unreachable,
            Cause::Url
            | Cause::Credentials
            | Cause::Database
            | Cause::Permission
            | Cause::StartupOptions
            | Cause::TlsTrust
            | Cause::TlsSetup => Verdict::Misconfigured,
            Cause::Unknown => Verdict::Unclassified,
        }
    }

    /// One line naming the cause, in the words of the thing that failed.
    fn summary(self) -> &'static str {
        match self {
            Cause::None => "connected, authenticated, and usable",
            Cause::Url => "the connection string does not parse as a libpq URL",
            Cause::Dns => "the host name did not resolve",
            Cause::Connect => "no TCP connection could be established",
            Cause::Timeout => "the connection attempt ran past its deadline",
            Cause::Tls => "the TLS handshake did not complete",
            Cause::TlsTrust => "the server presented a certificate we do not trust",
            Cause::TlsSetup => "the local TLS setup could not be built — nothing was dialled",
            Cause::Capacity => "the server refused the connection for lack of capacity",
            Cause::Starting => "the server is not accepting connections yet",
            Cause::Credentials => "the credentials were rejected",
            Cause::Database => "the database or role named in the URL does not exist",
            Cause::Permission => "the role may connect but not create a schema",
            Cause::StartupOptions => "libpq startup `options` were not honoured",
            Cause::Unknown => "the driver failed in a way this classifier does not recognise",
        }
    }

    /// What to do next. A verdict a reader cannot act on has not saved anyone any time.
    fn advice(self) -> &'static str {
        match self {
            Cause::None => "Nothing to do.",
            Cause::Url => {
                "Fix NXF_SUPABASE_DATABASE_URL — it must be a libpq URL, e.g. \
                 postgresql://user:password@host:5432/postgres?sslmode=require"
            }
            Cause::Dns => {
                "Check the host name in the secret and the provider's status page. On Supabase, \
                 db.<ref>.supabase.co resolves to IPv6 only on the free tier and GitHub runners \
                 have no IPv6 — use the Session pooler host (aws-N-<region>.pooler.supabase.com)."
            }
            Cause::Connect | Cause::Timeout => {
                "Check the provider's status page and whether the project is paused. This is an \
                 AVAILABILITY failure: nothing about nexus-flow is proven either way. Re-run once \
                 the endpoint answers."
            }
            Cause::Tls => {
                "The endpoint refused or failed TLS. Check the provider's status page — the server \
                 never got as far as presenting a certificate."
            }
            Cause::TlsSetup => {
                "This never touched the network. NXF_RELAY_PG_CA_FILE points at something that is \
                 missing, unreadable, or not a PEM bundle with a CERTIFICATE in it. Fix the path \
                 or the file; the detail below names which of the three it is."
            }
            Cause::TlsTrust => {
                "The server IS there; we have no trust anchor for it. Supabase signs its Postgres \
                 and pooler certificates with its OWN root (Supabase Root 2021 CA), which the \
                 bundled Mozilla roots do not chain to. Point NXF_RELAY_PG_CA_FILE at that root — \
                 CI pins it at .github/certs/supabase-root-2021-ca.pem, and your own project's \
                 copy is under Project Settings -> Database -> SSL Configuration."
            }
            Cause::Capacity => {
                "The project is out of connections (free-tier limit, or another run holding them). \
                 This is an AVAILABILITY failure, not an integration break. Re-run, and if it \
                 persists lower NXF_RELAY_PG_POOL_MAX_SIZE or the test thread count."
            }
            Cause::Starting => {
                "The server is booting or restarting — a paused Supabase project wakes on demand. \
                 Re-run in a minute."
            }
            Cause::Credentials => {
                "The password in NXF_SUPABASE_DATABASE_URL is wrong or was rotated. Re-set the \
                 secret from the provider's connection dialog."
            }
            Cause::Database => {
                "The database or role in the URL does not exist. On Supabase's Session pooler the \
                 user is postgres.<project-ref> and the database is postgres."
            }
            Cause::Permission => {
                "The role connected but may not CREATE SCHEMA. The suites isolate every test in \
                 its own schema, so this role needs that right on this database."
            }
            Cause::StartupOptions => {
                "The endpoint dropped the startup `options` parameter, so per-connection \
                 search_path never arrives and test isolation is impossible. On Supabase this \
                 means the TRANSACTION pooler (port 6543); use the SESSION pooler (port 5432)."
            }
            Cause::Unknown => {
                "Read the detail below and classify it by hand. Do not assume availability — an \
                 unrecognised failure is exactly the shape a real integration break takes."
            }
        }
    }
}

/// The outcome of a [`probe`], with everything a human needs and nothing they have to infer.
#[derive(Debug, Clone)]
pub struct Report {
    pub verdict: Verdict,
    pub cause: Cause,
    /// The failing thing's own words, verbatim — never paraphrased. The classifier can be
    /// wrong; the driver's message is the appeal.
    pub detail: String,
}

impl Report {
    fn ok() -> Report {
        Report {
            verdict: Verdict::Reachable,
            cause: Cause::None,
            detail: String::new(),
        }
    }

    /// Every report is built here, which is what makes the redaction below unskippable: a caller
    /// cannot construct one that carries an unscrubbed password by forgetting a helper.
    fn from(cause: Cause, detail: impl Into<String>) -> Report {
        Report {
            verdict: cause.verdict(),
            cause,
            detail: redact_url_password(&detail.into()),
        }
    }

    /// The block a human reads. Deliberately shaped so the verdict is the first thing on the
    /// first line — this text is what a red CI run has to be decidable from.
    pub fn banner(&self) -> String {
        banner(
            self.verdict.label(),
            self.cause.summary(),
            self.cause.advice(),
            "driver said",
            &self.detail,
        )
    }
}

/// The one place the verdict block is FORMATTED, for every surface that shows one.
///
/// The `managed-pg` example renders verdicts this module never produces — "no endpoint
/// configured", "a suite failed and the endpoint is still there" — and it used to carry its own
/// copy of this `format!`. Two copies of a layout that a human is supposed to recognise at a
/// glance is exactly the thing that drifts: one gains a field, the other keeps the old padding,
/// and the shape stops being scannable. `detail_label` is a parameter rather than a constant
/// because the provenance genuinely differs — here it is the driver's own words, there it may be
/// an environment fact — and lying about which would defeat the point of showing it.
pub fn banner(label: &str, what: &str, todo: &str, detail_label: &str, detail: &str) -> String {
    format!(
        "==================== {label} ====================\n\
         what happened : {what}\n\
         what to do    : {todo}\n\
         {detail_label:<14}: {detail}\n\
         ================================================================",
        detail = if detail.is_empty() { "—" } else { detail },
    )
}

/// One line, safe for a GitHub Actions `::error::` annotation.
///
/// Annotations are single-line — a newline truncates everything after it, which would silently
/// drop the most useful half of a multi-line driver message.
pub fn annotation_line(label: &str, detail: &str) -> String {
    // Runs of whitespace collapse, not just newlines: a wrapped, indented driver message flattened
    // naively becomes a line of mostly gaps, and the annotation is the one surface with no room to
    // waste — it is what shows above the log.
    let flat = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        label.to_string()
    } else {
        format!("{label} — {flat}")
    }
}

/// Strip the password out of any `scheme://user:password@host` that appears in `text`.
///
/// Defence in depth, and cheap. GitHub masks a secret by exact-string match, so the moment a
/// driver's error message quotes a *fragment* of the connection URL — or the same URL with one
/// character normalised — the masking stops applying and a password reaches a public log. The
/// `Cause::Url` path is the realistic one: a parse error is about the URL, and parse errors are
/// exactly the kind of message that likes to echo its input.
///
/// The username survives on purpose: on Supabase it is `postgres.<project-ref>`, which is
/// diagnostic rather than secret, and a redaction that removes the useful half too tends to get
/// switched off.
pub fn redact_url_password(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme_at) = rest.find("://") {
        let after_scheme = scheme_at + 3;
        // Userinfo ends at the first `@`, and only counts if it comes before the authority does
        // (a `@` after the first `/`, `?` or whitespace belongs to a path or a sentence).
        let authority_end = rest[after_scheme..]
            .find(['/', '?', ' ', '\t', '\n', '"', '\''])
            .map_or(rest.len(), |i| after_scheme + i);
        let userinfo_end = rest[after_scheme..authority_end]
            .find('@')
            .map(|i| after_scheme + i);
        match userinfo_end {
            // A userinfo with a colon in it: everything after the colon is the password.
            Some(at) => match rest[after_scheme..at].find(':') {
                Some(colon) => {
                    out.push_str(&rest[..after_scheme + colon + 1]);
                    out.push_str("***REDACTED***");
                    rest = &rest[at..];
                }
                // No colon means no password to hide.
                None => {
                    out.push_str(&rest[..at]);
                    rest = &rest[at..];
                }
            },
            None => {
                out.push_str(&rest[..authority_end]);
                rest = &rest[authority_end..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A configured endpoint, or `None` — the one rule for "is there an endpoint at all", shared by
/// every caller that asks.
///
/// **Empty counts as unset.** That is not pedantry: a GitHub job that maps a secret which does not
/// exist — never set, or a pull request from a fork, where secrets are withheld by design —
/// produces exactly this, `DATABASE_URL` present and empty. Treating it as a URL replaces a loud
/// "not configured" with a baffling parse error.
///
/// The value is **trimmed**, because the other way a secret arrives malformed is with a trailing
/// newline from a paste, and a URL with a `\n` in it fails somewhere far from the cause.
///
/// Pure, so the rule is testable without touching process-global environment state.
pub fn configured_url(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Which proof **this** run can give — decided from the CI event before any suite runs
/// (nexus-flow-6j6v.waeb).
///
/// ## The problem this exists for
///
/// GitHub withholds repository secrets from a pull request opened from a **fork**, by design and
/// without exception (`GITHUB_TOKEN` aside). That is not a setting to turn off: the org-wide
/// "require approval for outside contributors" gate decides whether the run *starts*, never
/// whether it gets credentials. So once this repository is public, the blocking managed-Postgres
/// job is unpassable for every outside contributor — an open-source repository in which no
/// stranger can land a change.
///
/// ## Why the obvious fix is the wrong one
///
/// Skipping the job on forks with a job-level `if:` looks free and is not: **GitHub reports a
/// skipped required check as green.** That is precisely the silent pass the blocking decision
/// (2026-08-10) exists to close, and it would be silent exactly where unfamiliar code arrives.
///
/// Handing the credential to the fork instead — `pull_request_target`, or a `workflow_run` that
/// checks out the pull request's head — runs contributor-authored code with the CI database URL
/// in its environment. One `println!` in a test exfiltrates it.
///
/// ## What this does instead
///
/// The job always runs, under its own name, and *declares* which of two things it did. A fork's
/// pull request gets [`ProofMode::ForkPullRequest`]: green, because the contributor did nothing
/// wrong and must be able to merge — but carrying a verdict block and a run annotation that say,
/// in words, what went unproven and where it will be proven. A run that should have had an
/// endpoint and does not gets [`ProofMode::NotConfigured`] and stays a hard failure.
///
/// The distinction a reader must be able to make in seconds is therefore between a *deliberate*
/// skip and a *silent* one — never between a pass and a skip, because a skip never renders as a
/// pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofMode {
    /// There is an endpoint: run the managed suites, and let them block.
    Managed,
    /// A pull request from a fork, which by GitHub's design carries no secrets. The managed
    /// proof is skipped **out loud**; the run is not a pass and does not claim to be.
    ForkPullRequest,
    /// No endpoint, and no reason for there not to be one. A hard failure — this is the
    /// verdict that keeps the fork case from becoming a general escape hatch.
    NotConfigured,
}

impl ProofMode {
    /// The word the workflow branches on.
    ///
    /// Part of the contract, not an implementation detail: every managed step in the job is
    /// guarded by `steps.<id>.outputs.mode == 'managed'`, and a token that no longer matches
    /// turns each of those guards into a skip — leaving a job whose every step is skipped, which
    /// reports green. Pinned by a test for that reason.
    pub fn token(self) -> &'static str {
        match self {
            ProofMode::Managed => "managed",
            ProofMode::ForkPullRequest => "fork-pull-request",
            ProofMode::NotConfigured => "not-configured",
        }
    }

    /// The headline a reader scans for.
    pub fn label(self) -> &'static str {
        match self {
            ProofMode::Managed => "MANAGED POSTGRES CONFIGURED",
            ProofMode::ForkPullRequest => "MANAGED POSTGRES NOT PROVEN HERE — FORK PULL REQUEST",
            ProofMode::NotConfigured => "NO MANAGED POSTGRES CONFIGURED",
        }
    }

    /// Whether this mode should turn the job red.
    ///
    /// Only [`ProofMode::NotConfigured`] does. A fork's pull request must be mergeable — the
    /// contributor cannot fix GitHub's secret policy, and a check that is red for everyone
    /// outside is a closed door with extra steps.
    pub fn is_failure(self) -> bool {
        self == ProofMode::NotConfigured
    }

    /// The verdict block, or `None` for the mode that simply proceeds to the real proof.
    ///
    /// `context` is the run's own facts — which command asked, which event it was. It is shown
    /// *under* the fixed explanation rather than instead of it, so the words a contributor needs
    /// cannot be crowded out by a variable that happened to be long.
    pub fn banner(self, context: &str) -> Option<String> {
        let (what, todo, detail_label, fixed) = match self {
            ProofMode::Managed => return None,
            ProofMode::ForkPullRequest => (
                "This pull request comes from a FORK, and GitHub withholds repository secrets \
                 from a fork by design — so this run had no managed endpoint to reach. The \
                 managed proof was skipped ON PURPOSE. It is not a pass, and it is not something \
                 you did wrong.",
                "Nothing on your side. A maintainer gets the managed proof before this can ship: \
                 it blocks every push to `main` and every release line, and can be run on this \
                 code earlier by pushing the branch into this repository.",
                "still proven here",
                "postgres-parity (storage parity + the relay end-to-end test, against a service \
                 container) and postgres-tls (a real TLS handshake, and a rejected trust anchor) \
                 ran in this same workflow. NOT proven here: a TLS-only endpoint, a provider's \
                 connection pooler, a foreign network, and a database that outlives the run.",
            ),
            ProofMode::NotConfigured => (
                "DATABASE_URL is empty or unset, so this run proved nothing about the managed \
                 endpoint.",
                "Set the NXF_SUPABASE_DATABASE_URL repository secret (Settings -> Secrets and \
                 variables -> Actions). This is NOT the fork case: a pull request from a fork is \
                 recognised as one and declared as such, so reaching this verdict means the \
                 endpoint is missing where it was expected to be there.",
                "detail",
                "",
            ),
        };
        let detail = match (fixed, context) {
            ("", context) => context.to_string(),
            (fixed, "") => fixed.to_string(),
            (fixed, context) => format!("{fixed} ({context})"),
        };
        Some(banner(self.label(), what, todo, detail_label, &detail))
    }
}

/// Which proof this run can give, from the two facts that decide it.
///
/// **A configured endpoint always wins**, and the ordering is the safety property rather than a
/// style choice: were fork-ness consulted first, a mis-read event could replace a real managed
/// proof with a declaration that none was needed. This way the fork branch can only ever excuse
/// the exact situation GitHub creates — and if GitHub ever did pass the secret, the run would
/// quietly go back to proving the real thing.
pub fn proof_mode(configured: Option<&str>, fork_pull_request: bool) -> ProofMode {
    match (configured, fork_pull_request) {
        (Some(_), _) => ProofMode::Managed,
        (None, true) => ProofMode::ForkPullRequest,
        (None, false) => ProofMode::NotConfigured,
    }
}

/// Is this run a pull request opened from a fork?
///
/// Read out of GitHub's own event payload (`GITHUB_EVENT_PATH`) rather than from a flag the
/// workflow file computes and passes in — the workflow cannot then disagree with the event, and
/// the rule is testable against the payload GitHub really writes.
///
/// **Every uncertainty resolves to `false`**: an absent payload, one that does not parse, a
/// missing field, a `fork` that is a string rather than a boolean. `false` is the side that
/// fails loudly ([`ProofMode::NotConfigured`]), and a rule about who may skip a blocking check
/// must never fail toward the skip.
///
/// Only the `pull_request` event qualifies. A push to `main` or to a maintenance line — the only
/// commits a release is ever cut from — can never be a fork's pull request, which is what makes
/// the excuse impossible to ship through.
pub fn is_fork_pull_request(event_name: Option<&str>, event_payload: Option<&str>) -> bool {
    if event_name != Some("pull_request") {
        return false;
    }
    let Some(payload) = event_payload else {
        return false;
    };
    let Ok(event) = serde_json::from_str::<serde_json::Value>(payload) else {
        return false;
    };
    event
        .pointer("/pull_request/head/repo/fork")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Do the event payload and the runner's own context agree about fork-ness?
///
/// The payload is read from a **file on the runner** (`GITHUB_EVENT_PATH`), and by the time the
/// decision is made, contributor code has already compiled in this job — a `build.rs` or a
/// proc-macro could rewrite that file, or export a `GITHUB_EVENT_NAME` through `$GITHUB_ENV` for
/// the steps that follow. That takes commit access, so it is not the fork threat model; it is the
/// more-trusted contributor, and a hostile `build.rs` is far easier to miss in review than an
/// edit to the workflow file.
///
/// So the fact is read twice, from sources that a compiled artifact cannot both reach: the
/// payload, and `${{ github.event.pull_request.head.repo.fork }}`, which the runner evaluates
/// from its own copy of the event context and passes in a step-level `env:` (step-level `env:`
/// wins over anything written to `$GITHUB_ENV`). A disagreement is not resolved in favour of
/// either — it fails the job. Silently preferring one source is how a tamper becomes invisible.
///
/// Anything that is not exactly `true` reads as "not a fork", matching [`is_fork_pull_request`]:
/// on any event without a pull request the expression renders as the empty string, so a push must
/// never look like a disagreement.
pub fn fork_signals_agree(payload_fork: bool, context: &str) -> bool {
    payload_fork == (context.trim() == "true")
}

/// How long to wait for a connection before calling it a timeout. Short on purpose: a black-holed
/// endpoint should cost a CI job seconds, not the runner's whole timeout budget.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Can we reach this managed Postgres, and can we use it the way the suites need to?
///
/// Four questions, in the order that makes each answer meaningful: connect (does the endpoint
/// exist and accept us), read (is it actually serving), `CREATE SCHEMA` (may this role isolate a
/// test), and startup-`options` round-trip (does a per-connection `search_path` survive the
/// provider's pooler). The last one is not paranoia — a transaction-mode pooler silently drops it,
/// and the resulting parity failures look exactly like a broken backend.
///
/// Safe from any calling context: the synchronous driver is run through
/// [`off_runtime`](crate::store_pg), so this may be called from inside a Tokio runtime.
pub fn probe(url: &str, connect_timeout: Duration) -> Report {
    off_runtime(|| probe_blocking(url, connect_timeout))
}

fn probe_blocking(url: &str, connect_timeout: Duration) -> Report {
    let mut config: postgres::Config = match url.parse() {
        Ok(config) => config,
        Err(e) => return Report::from(Cause::Url, e.to_string()),
    };
    config.connect_timeout(connect_timeout);

    let tls = match make_tls() {
        Ok(tls) => tls,
        Err(e) => return Report::from(Cause::TlsSetup, e.to_string()),
    };
    let mut client = match config.connect(tls) {
        Ok(client) => client,
        Err(e) => return Report::from(classify_error(&e), error_chain(&e)),
    };

    if let Err(e) = client.simple_query("SELECT 1") {
        return Report::from(classify_error(&e), error_chain(&e));
    }

    // We were connected a moment ago, so the LIKELY failure here is a permission one — but
    // "likely" is not "certain", and hardcoding it was a real bug: a connection can die between
    // the `SELECT 1` above and this statement (idle timeout, network blip, pooler reset), and an
    // availability blip reported as "the role may not CREATE SCHEMA" points an operator at a
    // permissions bug that does not exist. So the classifier decides, exactly as it does for every
    // other fallible call in this module.
    let schema = scratch_schema_name("probe");
    if let Err(e) = client.batch_execute(&format!(
        "CREATE SCHEMA IF NOT EXISTS {}",
        quote_ident(&schema)
    )) {
        let cause = classify_ddl_failure(classify_error(&e), e.code().is_some());
        return Report::from(cause, error_chain(&e));
    }
    let report = check_startup_options(url, &schema, connect_timeout);
    // Best effort: the sweep is what actually guarantees the project does not grow, so a failure
    // to tidy up here is not worth turning a green probe red.
    let _ = client.batch_execute(&format!(
        "DROP SCHEMA IF EXISTS {} CASCADE",
        quote_ident(&schema)
    ));
    report
}

/// Open a SECOND connection carrying `options=-c search_path=<schema>` and ask the server what it
/// actually got. A pooler that swallows startup parameters answers with something else.
fn check_startup_options(url: &str, schema: &str, connect_timeout: Duration) -> Report {
    let scoped = url_with_search_path(url, schema);
    let mut config: postgres::Config = match scoped.parse() {
        Ok(config) => config,
        Err(e) => return Report::from(Cause::Url, e.to_string()),
    };
    config.connect_timeout(connect_timeout);
    let tls = match make_tls() {
        Ok(tls) => tls,
        Err(e) => return Report::from(Cause::TlsSetup, e.to_string()),
    };
    let mut client = match config.connect(tls) {
        Ok(client) => client,
        Err(e) => return Report::from(classify_error(&e), error_chain(&e)),
    };
    match client.query_one("SELECT current_schema()", &[]) {
        Ok(row) => match row.get::<_, Option<String>>(0) {
            Some(actual) if actual == schema => Report::ok(),
            other => Report::from(
                Cause::StartupOptions,
                format!(
                    "asked for search_path={schema}, the session reports current_schema()={}",
                    other.unwrap_or_else(|| "NULL".into())
                ),
            ),
        },
        Err(e) => Report::from(classify_error(&e), error_chain(&e)),
    }
}

/// Classify a driver error: the server's own SQLSTATE when there is one, the transport's words
/// when there is not.
///
/// SQLSTATE first, always. It is the server speaking in a stable code rather than a message that
/// changes with locale, version, and vendor — and it is the difference between "your password is
/// wrong" and "the provider is down", which are the two answers this whole module exists to keep
/// apart.
fn classify_error(e: &postgres::Error) -> Cause {
    if let Some(code) = e.code() {
        if let Some(cause) = classify_sqlstate(code.code()) {
            return cause;
        }
    }
    // No SQLSTATE means the failure happened below the protocol — DNS, TCP, TLS, a deadline.
    // `Display` on a driver error prints only the outermost layer ("error connecting to server"),
    // so the chain has to be walked or the actual reason is lost.
    classify_text(&error_chain(e))
}

/// Refine the classifier's verdict for a DDL statement issued on a connection we have *already*
/// used successfully.
///
/// The context adds one fact the generic classifier does not have: if the server answered at all —
/// `had_sqlstate` — then reachability is settled, and an unrecognised code is about what this role
/// may DO. Saying `Permission` there is more useful than `Unclassified`, and it is still honest.
///
/// What it must NOT do is the thing this replaced: assume. `CREATE SCHEMA` failing with no
/// SQLSTATE means the connection died between statements — an idle timeout, a network blip, a
/// pooler reset — and reporting that as "the role may not CREATE SCHEMA" sends an operator hunting
/// a permissions bug that does not exist.
fn classify_ddl_failure(cause: Cause, had_sqlstate: bool) -> Cause {
    match cause {
        Cause::Unknown if had_sqlstate => Cause::Permission,
        other => other,
    }
}

/// Every message in an error's source chain, joined — the text the classifier reads and the
/// operator is shown.
pub fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut source = e.source();
    while let Some(inner) = source {
        parts.push(inner.to_string());
        source = inner.source();
    }
    parts.join(": ")
}

/// The server answered with a SQLSTATE. Only codes whose meaning is unambiguous are mapped;
/// anything else falls through to the text classifier rather than being forced into a bucket.
fn classify_sqlstate(code: &str) -> Option<Cause> {
    match code {
        // 28xxx — the server rejected who we claim to be.
        "28000" | "28P01" => Some(Cause::Credentials),
        // 3D000 invalid_catalog_name, 3F000 invalid_schema_name.
        "3D000" | "3F000" => Some(Cause::Database),
        // 42501 insufficient_privilege.
        "42501" => Some(Cause::Permission),
        // 53300 too_many_connections, 53400 configuration_limit_exceeded.
        "53300" | "53400" => Some(Cause::Capacity),
        // 57P03 cannot_connect_now, 57P01/57P02 admin/crash shutdown — the server is there but
        // not serving. An availability answer, not a defect in us.
        "57P01" | "57P02" | "57P03" => Some(Cause::Starting),
        // 08xxx — the connection itself failed or was lost.
        "08000" | "08001" | "08003" | "08004" | "08006" | "08007" | "08P01" => Some(Cause::Connect),
        _ => None,
    }
}

/// The transport failed below the protocol. Ordered most-specific-first: a TLS failure mentions
/// "connection" too, and a DNS failure mentions "connect".
fn classify_text(text: &str) -> Cause {
    let t = text.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| t.contains(n));

    if has(&[
        "failed to lookup address information",
        "nodename nor servname",
        "name or service not known",
        "temporary failure in name resolution",
        "no such host",
        "dns error",
    ]) {
        return Cause::Dns;
    }
    // TRUST first, and it matters: rustls says "error performing TLS handshake: invalid peer
    // certificate: UnknownIssuer", which contains "handshake" too. Blaming the handshake would
    // file a missing trust anchor under "the provider is down" and send a reader to a status page
    // that says everything is fine — the first real Supabase run produced exactly this string.
    if has(&[
        "unknownissuer",
        "invalid peer certificate",
        "invalidcertificate",
        "notvalidforname",
        "certexpired",
        "certificate has expired",
        "self signed certificate",
        "self-signed certificate",
        "unable to get local issuer",
        "certificate verify failed",
    ]) {
        return Cause::TlsTrust;
    }
    // Ahead of the generic TLS bucket on purpose: "TLS handshake timed out" names both, and it is
    // a deadline, not a protocol failure. Same verdict either way, but the advice differs.
    if has(&[
        "timed out",
        "timeout expired",
        "deadline has elapsed",
        "operation timed out",
    ]) {
        return Cause::Timeout;
    }
    if has(&[
        "certificate",
        "handshake",
        "no tls implementation",
        "server does not support tls",
        "tls error",
        "corruptmessage",
    ]) {
        return Cause::Tls;
    }
    // Pooler capacity often arrives as prose rather than a SQLSTATE (Supabase's Supavisor says
    // "max client connections reached"), so it is matched here as well as in `classify_sqlstate`.
    if has(&[
        "max client connections reached",
        "too many clients",
        "too many connections",
        "remaining connection slots",
    ]) {
        return Cause::Capacity;
    }
    if has(&[
        "connection refused",
        "network is unreachable",
        "no route to host",
        "connection reset",
        "broken pipe",
        "connection closed",
        "unexpected eof",
        "os error 61",
    ]) {
        return Cause::Connect;
    }
    Cause::Unknown
}

// ----- scratch schemas ---------------------------------------------------------------------

/// The prefix every schema this repo's suites create carries. Nothing outside this prefix is ever
/// touched by [`sweep_scratch_schemas`].
pub const SCRATCH_SCHEMA_PREFIX: &str = "nxf_";

/// How long a scratch schema is left alone before the sweep may drop it. Generous by design: it
/// only has to exceed the longest a CI run can plausibly still be using one, and being wrong in
/// that direction deletes another run's database out from under it.
pub const DEFAULT_SCRATCH_MAX_AGE: Duration = Duration::from_secs(2 * 60 * 60);

/// A fresh, collision-free schema name that also records **when** it was made.
///
/// `nxf_<unix-seconds>_<pid>_<n>_<tag>`. The timestamp sits immediately after the fixed prefix so
/// [`sweep_scratch_schemas`] can read it without knowing anything about tags — which is what lets
/// the sweep run safely while other CI runs are mid-flight: it decides by age, never by guessing
/// whose schema it is looking at.
pub fn scratch_schema_name(tag: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{SCRATCH_SCHEMA_PREFIX}{}_{}_{}_{tag}",
        unix_seconds(),
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A libpq URL whose connections default `search_path` to `schema`, so every unqualified
/// `CREATE TABLE`/query in that session lands there.
///
/// One implementation, used by both the probe and the test harness on purpose: the encoding of
/// the space and `=` inside `options` is the kind of detail that is right in one copy and subtly
/// wrong in the second.
pub fn url_with_search_path(base: &str, schema: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    // Schema names minted here are `[a-z0-9_]`, so only the space and `=` need encoding.
    format!("{base}{sep}options=-c%20search_path%3D{schema}")
}

/// The age of a scratch schema, or `None` if `name` was not minted by [`scratch_schema_name`].
///
/// Returning `None` for anything unrecognised is the safety property: a database that happens to
/// hold an unrelated `nxf_`-prefixed schema keeps it.
fn scratch_schema_age(name: &str, now: u64) -> Option<Duration> {
    let rest = name.strip_prefix(SCRATCH_SCHEMA_PREFIX)?;
    let mut fields = rest.splitn(4, '_');
    let stamp: u64 = fields.next()?.parse().ok()?;
    // pid and counter must be numeric too, or this is some other schema that merely starts the
    // same way.
    fields.next()?.parse::<u64>().ok()?;
    fields.next()?.parse::<u64>().ok()?;
    // The tag is free-form, but it must exist — `nxf_1_2_3` alone is not one of ours.
    fields.next()?;
    Some(Duration::from_secs(now.saturating_sub(stamp)))
}

/// Whether the sweep may drop `name`: ours, and old enough that no run can still hold it.
///
/// Split out of the filter closure so the boundary is testable. `>=` is the deliberate edge —
/// a schema exactly `max_age` old is swept — and an off-by-one in the other direction would be
/// the kind of thing that only shows up as a project slowly filling with debris.
fn is_stale_scratch_schema(name: &str, now: u64, max_age: Duration) -> bool {
    scratch_schema_age(name, now).is_some_and(|age| age >= max_age)
}

/// A Postgres identifier, quoted so its content cannot be read as SQL.
///
/// Every schema name this module builds is `[a-z0-9_]` by construction, so this changes nothing
/// about what the statements do. It exists because one of them does NOT come from here: the sweep
/// reads names back out of `pg_namespace`, and anyone holding `CREATE` on that database can put a
/// perfectly legal quoted identifier there containing a quote and a semicolon. `batch_execute`
/// runs multiple statements, so splicing that raw into `DROP SCHEMA … CASCADE` would execute it.
/// The read side already escapes its `LIKE` pattern; the write side has no business being looser.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Drop every scratch schema older than `max_age`, returning the names dropped.
///
/// Run this **before** a suite, not after: a sweep that runs first can use a generous age cutoff
/// and is therefore safe next to a concurrent CI run, whereas an after-the-fact sweep that tried
/// to catch its own run's schemas would have to use a cutoff short enough to delete somebody
/// else's. The cost of the choice is that the most recent run's schemas outlive it — one run's
/// worth of debris, cleaned by the next — and the benefit is that the sweep can never destroy a
/// live run.
pub fn sweep_scratch_schemas(
    url: &str,
    max_age: Duration,
    connect_timeout: Duration,
) -> StoreResult<Vec<String>> {
    off_runtime(|| {
        // The same deadline `probe` uses, and for the same reason the module header gives: a
        // black-holed endpoint should cost a CI job seconds, not the runner's whole budget. This
        // connection had none, which mattered more here than anywhere else — the sweep is the
        // FIRST step of a job that is meant to block the merge gate.
        let mut config: postgres::Config = url.parse().map_err(pg_error)?;
        config.connect_timeout(connect_timeout);
        let mut client = config.connect(make_tls()?).map_err(pg_error)?;
        // `LIKE` narrows the scan; `scratch_schema_age` is what actually decides, so a schema
        // that merely shares the prefix is never dropped.
        let rows = client
            .query(
                "SELECT nspname FROM pg_namespace WHERE nspname LIKE 'nxf\\_%' ORDER BY nspname",
                &[],
            )
            .map_err(pg_error)?;
        let now = unix_seconds();
        let stale: Vec<String> = rows
            .iter()
            .map(|row| row.get::<_, String>(0))
            .filter(|name| is_stale_scratch_schema(name, now, max_age))
            .collect();
        for name in &stale {
            // Quoted, even though `is_stale_scratch_schema` has already established this is a name
            // we could have minted. That check constrains WHICH schemas are touched, not the
            // character set of the free-form tag field — and this string came back FROM the
            // database, where a deliberately hostile (but perfectly legal) quoted identifier could
            // have been created by anyone holding CREATE on it. The read side already escapes its
            // LIKE pattern; the write side has no business being the looser of the two.
            client
                .batch_execute(&format!(
                    "DROP SCHEMA IF EXISTS {} CASCADE",
                    quote_ident(name)
                ))
                .map_err(pg_error)?;
        }
        Ok(stale)
    })
}

fn pg_error<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The split the owner's rule stands on: a provider that is not there and a role that is
    /// rejected must never land in the same bucket.
    #[test]
    fn availability_and_configuration_are_different_verdicts() {
        for cause in [Cause::Dns, Cause::Connect, Cause::Timeout, Cause::Tls] {
            assert_eq!(cause.verdict(), Verdict::Unreachable, "{cause:?}");
        }
        for cause in [
            Cause::Credentials,
            Cause::Database,
            Cause::Permission,
            Cause::StartupOptions,
            Cause::TlsTrust,
            Cause::Url,
        ] {
            assert_eq!(cause.verdict(), Verdict::Misconfigured, "{cause:?}");
        }
    }

    /// A quota refusal is the provider saying "not now" — availability, not a defect in us.
    #[test]
    fn a_capacity_refusal_is_an_availability_answer() {
        assert_eq!(Cause::Capacity.verdict(), Verdict::Unreachable);
        assert_eq!(Cause::Starting.verdict(), Verdict::Unreachable);
    }

    /// The one that protects the rule from itself: an unrecognised failure must NOT be filed
    /// under "the provider was down", because that is how a real break gets waved through.
    #[test]
    fn an_unrecognised_failure_is_never_called_unreachable() {
        assert_eq!(Cause::Unknown.verdict(), Verdict::Unclassified);
        assert_eq!(
            classify_text("something nobody has seen before"),
            Cause::Unknown
        );
    }

    #[test]
    fn sqlstates_that_mean_wrong_credentials() {
        assert_eq!(classify_sqlstate("28P01"), Some(Cause::Credentials));
        assert_eq!(classify_sqlstate("28000"), Some(Cause::Credentials));
        assert_eq!(classify_sqlstate("3D000"), Some(Cause::Database));
        assert_eq!(classify_sqlstate("42501"), Some(Cause::Permission));
    }

    #[test]
    fn sqlstates_that_mean_the_server_has_no_room() {
        assert_eq!(classify_sqlstate("53300"), Some(Cause::Capacity));
        assert_eq!(classify_sqlstate("57P03"), Some(Cause::Starting));
    }

    /// An unmapped SQLSTATE falls through rather than being forced into a bucket.
    #[test]
    fn an_unmapped_sqlstate_is_not_guessed() {
        assert_eq!(classify_sqlstate("22012"), None);
    }

    /// The transport messages, in the words each platform actually uses.
    #[test]
    fn transport_failures_are_told_apart() {
        assert_eq!(
            classify_text("error connecting to server: failed to lookup address information: nodename nor servname provided"),
            Cause::Dns
        );
        assert_eq!(
            classify_text("error connecting to server: Connection refused (os error 61)"),
            Cause::Connect
        );
        assert_eq!(
            classify_text("error connecting to server: connection timed out"),
            Cause::Timeout
        );
        assert_eq!(
            classify_text("invalid peer certificate: UnknownIssuer"),
            Cause::TlsTrust
        );
        assert_eq!(
            classify_text("error performing TLS handshake: server does not support TLS"),
            Cause::Tls
        );
        assert_eq!(
            classify_text("db error: FATAL: Max client connections reached"),
            Cause::Capacity
        );
    }

    /// The ordering guarantee, stated as a test: a certificate complaint wrapped in the driver's
    /// "error connecting to server" prefix must still be read as a certificate complaint. Match
    /// the prefix first and every trust failure in the world becomes "the socket broke".
    #[test]
    fn a_certificate_complaint_is_never_reported_as_a_socket_failure() {
        assert_eq!(
            classify_text("error connecting to server: invalid peer certificate: UnknownIssuer"),
            Cause::TlsTrust
        );
    }

    /// THE regression this classifier learned the hard way — verbatim from the first run against
    /// a real Supabase project (CI run 31402670898). Supabase signs its Postgres and pooler
    /// certificates with its own root, so a client carrying only the bundled Mozilla roots gets
    /// this. It is NOT an availability failure: the server was there, talking, and presented a
    /// certificate. Filing it under "provider down" would send a reader to a status page that
    /// says everything is fine, and the missing trust anchor would never get fixed.
    #[test]
    fn supabases_own_root_is_a_trust_problem_not_an_outage() {
        let real = "error performing TLS handshake: invalid peer certificate: UnknownIssuer";
        assert_eq!(classify_text(real), Cause::TlsTrust);
        assert_eq!(Cause::TlsTrust.verdict(), Verdict::Misconfigured);
        assert!(
            Cause::TlsTrust.advice().contains("NXF_RELAY_PG_CA_FILE"),
            "the advice must name the lever that fixes it: {}",
            Cause::TlsTrust.advice()
        );
    }

    /// An expired or wrong-host certificate is the same KIND of answer: the endpoint answered.
    #[test]
    fn every_certificate_complaint_lands_in_the_trust_bucket() {
        for text in [
            "invalid peer certificate: CertExpired",
            "invalid peer certificate: NotValidForName",
            "SSL error: certificate verify failed",
            "self signed certificate in certificate chain",
        ] {
            assert_eq!(classify_text(text), Cause::TlsTrust, "{text}");
        }
    }

    /// The banner is the artifact a red run is decided from, so its shape is pinned: verdict
    /// first, then what happened, what to do, and the driver's own words.
    #[test]
    fn the_banner_leads_with_the_verdict_and_carries_the_evidence() {
        let report = Report::from(Cause::Connect, "Connection refused (os error 61)");
        let banner = report.banner();
        let first = banner.lines().next().unwrap();
        assert!(
            first.contains("MANAGED POSTGRES UNREACHABLE"),
            "the verdict must be on the first line: {first}"
        );
        assert!(banner.contains("Connection refused (os error 61)"));
        assert!(
            banner.contains("status page"),
            "must say what to do: {banner}"
        );
    }

    /// A reachable report says so and offers no remedy for a problem that does not exist.
    #[test]
    fn a_reachable_report_is_unambiguous() {
        let report = Report::ok();
        assert!(report.verdict.is_ok());
        assert!(report.banner().contains("REACHABLE"));
    }

    #[test]
    fn a_scratch_schema_name_is_unique_and_carries_its_birth_time() {
        let a = scratch_schema_name("parity");
        let b = scratch_schema_name("parity");
        assert_ne!(a, b);
        assert!(a.starts_with(SCRATCH_SCHEMA_PREFIX));
        assert!(a.ends_with("_parity"));
        let age = scratch_schema_age(&a, unix_seconds()).expect("our own name parses");
        assert!(age < Duration::from_secs(60), "just minted: {age:?}");
    }

    /// The sweep's safety property: it recognises only names it could have minted itself.
    #[test]
    fn a_foreign_schema_is_never_recognised_as_scratch() {
        let now = unix_seconds();
        for foreign in [
            "public",
            "nxf_data",
            "nxf_prod_tables",
            "nxf_1_2_3",
            "nxf_notanumber_1_0_parity",
            "supabase_migrations",
        ] {
            assert!(
                scratch_schema_age(foreign, now).is_none(),
                "{foreign} must not be treated as a scratch schema"
            );
        }
    }

    #[test]
    fn an_old_scratch_schema_is_recognised_as_old() {
        let now = 1_800_000_000;
        let old = format!(
            "{SCRATCH_SCHEMA_PREFIX}{}_{}_{}_{}",
            now - 7_200,
            42,
            0,
            "parity"
        );
        assert_eq!(
            scratch_schema_age(&old, now),
            Some(Duration::from_secs(7_200))
        );
    }

    /// PR #320 review, found independently by all three reviews: `CREATE SCHEMA` used to be
    /// hardcoded to `Permission`. A connection that dies between the `SELECT 1` and the DDL — idle
    /// timeout, network blip, pooler reset — carries NO SQLSTATE, and reporting that as "the role
    /// may not CREATE SCHEMA" sends an operator after a permissions bug that does not exist.
    #[test]
    fn a_dropped_connection_during_ddl_is_not_reported_as_a_permissions_problem() {
        assert_eq!(
            classify_ddl_failure(Cause::Connect, false),
            Cause::Connect,
            "no SQLSTATE means the server never answered — that is availability, not permissions"
        );
        assert_eq!(classify_ddl_failure(Cause::Timeout, false), Cause::Timeout);
        assert_eq!(classify_ddl_failure(Cause::Unknown, false), Cause::Unknown);
    }

    /// The other half: if the server DID answer, reachability is settled, so an unrecognised code
    /// is about what this role may do. Naming it beats shrugging.
    #[test]
    fn an_unmapped_server_error_during_ddl_is_a_permissions_answer() {
        assert_eq!(
            classify_ddl_failure(Cause::Unknown, true),
            Cause::Permission
        );
        // …but a code the classifier DOES understand is never overwritten.
        assert_eq!(
            classify_ddl_failure(Cause::Credentials, true),
            Cause::Credentials
        );
        assert_eq!(classify_ddl_failure(Cause::Capacity, true), Cause::Capacity);
    }

    /// A broken `NXF_RELAY_PG_CA_FILE` never reaches the network, so it must not be reported as
    /// the provider being down — the advice for that case sends a reader to a status page.
    #[test]
    fn a_local_tls_setup_failure_is_not_an_outage() {
        assert_eq!(Cause::TlsSetup.verdict(), Verdict::Misconfigured);
        assert!(
            Cause::TlsSetup.advice().contains("NXF_RELAY_PG_CA_FILE"),
            "must name the variable that caused it: {}",
            Cause::TlsSetup.advice()
        );
        assert!(
            !Cause::TlsSetup.advice().contains("status page"),
            "must NOT send anyone to a status page: {}",
            Cause::TlsSetup.advice()
        );
    }

    /// A deadline is a deadline even when the word "handshake" is in the message.
    #[test]
    fn a_tls_handshake_that_times_out_is_a_timeout() {
        assert_eq!(
            classify_text("error performing TLS handshake: operation timed out"),
            Cause::Timeout
        );
    }

    /// The sweep's boundary, stated exactly: `>=` means a schema of precisely `max_age` goes.
    #[test]
    fn the_sweep_age_cutoff_is_inclusive_at_the_boundary() {
        let now = 1_800_000_000;
        let max_age = Duration::from_secs(7_200);
        let name = |age: u64| format!("{SCRATCH_SCHEMA_PREFIX}{}_{}_{}_parity", now - age, 42, 0);

        assert!(
            is_stale_scratch_schema(&name(7_200), now, max_age),
            "exactly max_age old must be swept"
        );
        assert!(
            !is_stale_scratch_schema(&name(7_199), now, max_age),
            "one second younger than max_age must be spared"
        );
        assert!(is_stale_scratch_schema(&name(7_201), now, max_age));
        assert!(
            !is_stale_scratch_schema("public", now, max_age),
            "a foreign schema is never stale, at any age"
        );
    }

    /// Defence in depth for the one name in this module that does NOT come from this module: the
    /// sweep reads it back out of `pg_namespace`, where anyone with CREATE can put a legal quoted
    /// identifier carrying a quote and a semicolon.
    #[test]
    fn a_hostile_identifier_cannot_escape_into_ddl() {
        assert_eq!(quote_ident("nxf_1_2_3_parity"), "\"nxf_1_2_3_parity\"");
        let hostile = r#"nxf_1_2_3_x"; DROP TABLE items; --"#;
        let quoted = quote_ident(hostile);
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
        // Every inner quote is doubled, so the identifier cannot be terminated early — which is
        // what would let the rest of the string be read as SQL.
        assert_eq!(
            quoted, r#""nxf_1_2_3_x""; DROP TABLE items; --""#,
            "the embedded quote must be doubled, not passed through"
        );
    }

    /// A password must not reach a CI log. GitHub masks a secret by exact string, so the moment a
    /// driver echoes a fragment of the URL the masking stops applying.
    #[test]
    fn a_password_is_scrubbed_out_of_anything_we_print() {
        let text = "invalid connection string: \
                    postgresql://postgres.abcdef:s3cr3t-pw@aws-0-eu.pooler.supabase.com:5432/postgres";
        let safe = redact_url_password(text);
        assert!(!safe.contains("s3cr3t-pw"), "password survived: {safe}");
        assert!(
            safe.contains("postgres.abcdef"),
            "the username is diagnostic, not secret, and must survive: {safe}"
        );
        assert!(safe.contains("aws-0-eu.pooler.supabase.com"), "{safe}");
        assert!(safe.contains("***REDACTED***"), "{safe}");
    }

    /// …and it must not mangle the ordinary case, or it would be turned off.
    #[test]
    fn redaction_leaves_text_without_credentials_alone() {
        for text in [
            "error connecting to server: Connection refused (os error 61)",
            "postgres://localhost:5432/nxf_test",
            "see https://supabase.com/docs for the CA",
            "",
        ] {
            assert_eq!(redact_url_password(text), text, "{text}");
        }
    }

    /// Every `Report` is built through `from`, so redaction cannot be forgotten at a call site.
    #[test]
    fn redaction_is_not_optional_for_a_report() {
        let report = Report::from(Cause::Url, "bad url: postgres://u:hunter2@db.example.com/x");
        assert!(!report.detail.contains("hunter2"), "{}", report.detail);
        assert!(!report.banner().contains("hunter2"), "{}", report.banner());
    }

    /// The annotation is a single line by construction — a newline truncates a GitHub annotation,
    /// silently dropping the most useful half of a multi-line driver message.
    #[test]
    fn an_annotation_never_carries_a_newline() {
        let line = annotation_line("MANAGED POSTGRES UNREACHABLE", "one\ntwo\r\nthree");
        assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
        assert!(line.contains("one two three"), "{line}");
        assert_eq!(annotation_line("LABEL", ""), "LABEL", "no trailing dash");
        assert_eq!(annotation_line("LABEL", "  \n "), "LABEL");
    }

    /// Both verdict surfaces render through one formatter, so they cannot drift apart.
    #[test]
    fn the_shared_banner_puts_the_verdict_first_whatever_the_detail_label() {
        let b = banner("SOME VERDICT", "what", "todo", "detail", "");
        assert!(b.lines().next().unwrap().contains("SOME VERDICT"));
        assert!(
            b.contains("detail        : —"),
            "empty detail is dashed: {b}"
        );
        assert!(banner("V", "w", "t", "driver said", "x").contains("driver said   : x"));
    }

    /// The fork-PR contract: a secret that does not exist arrives as an empty string, and must
    /// never be mistaken for an endpoint.
    #[test]
    fn an_absent_or_blank_endpoint_is_not_an_endpoint() {
        assert_eq!(configured_url(None), None);
        assert_eq!(configured_url(Some("")), None);
        assert_eq!(configured_url(Some("   \n\t ")), None);
    }

    /// …and a pasted secret with a trailing newline is still a usable URL, not a parse error
    /// somewhere far from the cause.
    #[test]
    fn a_pasted_endpoint_is_trimmed_rather_than_passed_on_broken() {
        assert_eq!(
            configured_url(Some("  postgres://h/db?sslmode=require\n")).as_deref(),
            Some("postgres://h/db?sslmode=require")
        );
    }

    /// The rule the whole fork arrangement stands on: a run that CANNOT reach the endpoint says
    /// so, and a run that SHOULD have reached it and did not goes red. Both are visible; only one
    /// is forgiven.
    #[test]
    fn a_declared_fork_skip_and_a_missing_endpoint_are_different_modes() {
        assert_eq!(proof_mode(None, true), ProofMode::ForkPullRequest);
        assert_eq!(proof_mode(None, false), ProofMode::NotConfigured);
        assert!(!ProofMode::ForkPullRequest.is_failure());
        assert!(ProofMode::NotConfigured.is_failure());
    }

    /// The guarantee that makes the forgiveness safe to grant: the fork excuse is reachable ONLY
    /// from a fork's pull request. A push to `main` or a maintenance line — the only commits a
    /// release is ever cut from — cannot take it, so nothing ships through this door.
    #[test]
    fn nothing_that_ships_can_take_the_fork_excuse() {
        for event in ["push", "workflow_dispatch", "schedule", "release"] {
            assert!(
                !is_fork_pull_request(Some(event), Some(FORK_EVENT_PAYLOAD)),
                "{event} must never be read as a fork pull request"
            );
            assert_eq!(
                proof_mode(
                    None,
                    is_fork_pull_request(Some(event), Some(FORK_EVENT_PAYLOAD))
                ),
                ProofMode::NotConfigured,
                "{event} with no endpoint must be a hard failure"
            );
        }
    }

    /// A configured endpoint always wins. Stated as a test because the ordering is the whole
    /// safety property: were fork-ness checked first, a mis-set `fork` flag could silently
    /// replace a real managed proof with a declaration that none was needed.
    #[test]
    fn a_present_endpoint_is_never_displaced_by_the_fork_excuse() {
        assert_eq!(
            proof_mode(Some("postgres://h/db"), true),
            ProofMode::Managed
        );
        assert!(ProofMode::Managed.banner("").is_none());
    }

    /// Fork-ness is read from GitHub's own event payload, not from a flag the workflow file
    /// passes in — one less place for the two to disagree.
    #[test]
    fn a_fork_pull_request_is_read_out_of_the_event_itself() {
        assert!(is_fork_pull_request(
            Some("pull_request"),
            Some(FORK_EVENT_PAYLOAD)
        ));
        assert!(!is_fork_pull_request(
            Some("pull_request"),
            Some(r#"{"pull_request":{"head":{"repo":{"fork":false}}}}"#)
        ));
    }

    /// Every way the payload can be absent or unreadable resolves to "not a fork" — the side
    /// that fails LOUDLY. A parse error that defaulted to `true` would hand the excuse to a run
    /// that never asked for it.
    #[test]
    fn an_unreadable_event_is_never_read_as_a_fork() {
        for payload in [
            None,
            Some("not json at all"),
            Some("{}"),
            Some(r#"{"pull_request":{"head":{"repo":{}}}}"#),
            Some(r#"{"pull_request":{"head":{"repo":{"fork":"true"}}}}"#),
        ] {
            assert!(
                !is_fork_pull_request(Some("pull_request"), payload),
                "{payload:?} must not be read as a fork"
            );
        }
        assert!(!is_fork_pull_request(None, Some(FORK_EVENT_PAYLOAD)));
    }

    /// The declared skip has to READ like a skip. A banner that omitted what went unproven would
    /// be indistinguishable from a pass, which is the one outcome this mode exists to prevent.
    #[test]
    fn the_fork_banner_says_what_was_not_proven_and_where_it_will_be() {
        let banner = ProofMode::ForkPullRequest
            .banner("")
            .expect("a stopped proof always has a verdict block");
        assert!(banner.contains("NOT PROVEN"), "{banner}");
        assert!(banner.contains("fork"), "{banner}");
        assert!(banner.contains("main"), "{banner}");
        assert!(
            banner.contains("postgres-parity") && banner.contains("postgres-tls"),
            "the contributor has to be told what DID run: {banner}"
        );
    }

    /// The missing-endpoint verdict keeps the run's own facts, so two commands failing for the
    /// same reason are still told apart in the log.
    #[test]
    fn the_missing_endpoint_verdict_carries_the_context_it_was_asked_in() {
        let banner = ProofMode::NotConfigured
            .banner("no DATABASE_URL in the environment (running `preflight`)")
            .expect("a stopped proof always has a verdict block");
        assert!(
            banner.contains("NO MANAGED POSTGRES CONFIGURED"),
            "{banner}"
        );
        assert!(banner.contains("running `preflight`"), "{banner}");
    }

    /// The token is what the workflow branches on. Pinned, because renaming it silently turns
    /// every `if:` in the job into a no-op — and a job whose every step is skipped reports green.
    #[test]
    fn the_machine_readable_token_is_part_of_the_contract() {
        assert_eq!(ProofMode::Managed.token(), "managed");
        assert_eq!(ProofMode::ForkPullRequest.token(), "fork-pull-request");
        assert_eq!(ProofMode::NotConfigured.token(), "not-configured");
    }

    /// The runner's own view of fork-ness is the tamper detector, so the agreement rule has to
    /// read "the same fact, twice" — never "either source will do".
    #[test]
    fn the_two_fork_signals_agree_only_when_they_say_the_same_thing() {
        assert!(fork_signals_agree(true, "true"));
        assert!(fork_signals_agree(false, "false"));
        assert!(!fork_signals_agree(true, "false"));
        assert!(!fork_signals_agree(false, "true"));
    }

    /// A `pull_request` context on a non-pull-request event renders as the empty string, and every
    /// other unrecognised spelling is NOT a fork — matching `is_fork_pull_request`, so a push can
    /// never look like a disagreement.
    #[test]
    fn an_absent_or_unrecognised_context_signal_means_not_a_fork() {
        for context in ["", "  ", "null", "True", "1", "yes"] {
            assert!(
                fork_signals_agree(false, context),
                "{context:?} must read as not-a-fork"
            );
            assert!(
                !fork_signals_agree(true, context),
                "{context:?} must not confirm a fork the payload claims"
            );
        }
    }

    /// A fork's pull request, in the shape GitHub actually writes to `GITHUB_EVENT_PATH`
    /// (trimmed to the fields the rule reads).
    const FORK_EVENT_PAYLOAD: &str = r#"{
        "action": "opened",
        "number": 321,
        "pull_request": {
            "head": { "ref": "patch-1", "repo": { "full_name": "contributor/nexus-flow", "fork": true } },
            "base": { "ref": "main", "repo": { "full_name": "nxsflow/nexus-flow", "fork": false } }
        }
    }"#;

    #[test]
    fn the_search_path_option_is_appended_to_either_url_shape() {
        assert_eq!(
            url_with_search_path("postgres://h/db", "nxf_1_2_3_parity"),
            "postgres://h/db?options=-c%20search_path%3Dnxf_1_2_3_parity"
        );
        assert_eq!(
            url_with_search_path("postgres://h/db?sslmode=require", "s"),
            "postgres://h/db?sslmode=require&options=-c%20search_path%3Ds"
        );
    }
}

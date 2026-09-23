//! `cargo xtask runners check` (nexus-flow-6j6v.2sb1): keep the org's self-hosted Mac off every
//! job a `pull_request` can reach.
//!
//! Two workflows bill the org's own Mac today — `ci.yml`'s `platform-gates-macos` and
//! `release.yml`'s darwin build/sign/notarize legs. Both need write access to reach, and
//! `ci.yml`'s header explains at length why that is a SECURITY boundary and not a cost decision:
//! the machine is not ephemeral, it shares a login session with a human, and it holds the
//! Developer ID signing material (6j6v.px3d). What holds the `pull_request` surface shut is a
//! single `if:` line.
//!
//! That line is an assurance no gate checks. A later change adds a macOS step, leaves the `if:`
//! off or picks a trigger that includes `pull_request`, and from that merge on a fork's code runs
//! on the signing machine — nobody did anything wrong and nobody is warned. While the repo is
//! private the gap is inert (there are no outside contributors); from the public switch on it is
//! open. This is the half of that guard that lives in the repo. The other half — runner-group
//! scoping and fork-PR approval — is a server-side setting and belongs to the going-public
//! runbook (6j6v.w07d); neither replaces the other.
//!
//! WHAT THE RULE IS: no job that a `pull_request`/`pull_request_target` run can REACH may route
//! to a runner this repo cannot prove is GitHub-hosted.
//!
//! Three things make that sentence harder than it looks, and they are the actual content here:
//!
//! 1. REACHABLE, not merely present. `platform-gates-macos` sits in a workflow WITH a
//!    `pull_request` trigger and is fine, because its `if:` excludes the event. So the check reads
//!    the `if:` — with a three-valued evaluator that only ever calls a job excluded when it can
//!    PROVE the condition false for every event the workflow can be entered with. An `if:` it does
//!    not understand leaves the job reachable, which is the safe direction.
//!
//! 2. `workflow_run` and `workflow_call` are detours. A workflow with no `pull_request` trigger
//!    that hangs off one that has it is reachable all the same, and reachability propagates with
//!    the ENTRY EVENT: a workflow entered through `workflow_run` sees `github.event_name ==
//!    'workflow_run'`, so an `if: github.event_name != 'pull_request'` there excludes nothing. A
//!    called workflow, by contrast, keeps the CALLER's event, so the same line still bites there.
//!    There is no such chain in the tree today (6j6v.sgfr removed the last one), and the rule
//!    covers both anyway rather than resting on nobody building a new one.
//!
//! 3. PROVABLY HOSTED, not merely "does not say self-hosted". Since 2022 a job can target a
//!    self-hosted runner by its custom labels ALONE — `runs-on: [macOS, X64]` reaches this org's
//!    Mac without the word `self-hosted` appearing anywhere. And `vars.NXF_MACOS_RUNS_ON` holds a
//!    JSON label array whose value is a repository setting, invisible from here. So the check
//!    works from a whitelist: every label a reachable job can land on must be a GitHub-hosted
//!    image label (`ubuntu-*`, `windows-*`, `macos-*`, lowercase family + suffix). Anything else —
//!    a bare `self-hosted`, `macOS`, a `vars.` reference, a runner `group:`, an expression this
//!    module cannot resolve — fails, and the way out is a counted exemption below, not a weakened
//!    rule.
//!
//! WHAT IS DELIBERATELY NOT MODELLED: `needs:` propagation. A job whose every dependency is
//! skipped is skipped too, so in principle a `needs:` chain can exclude a job with no `if:` of its
//! own. Ignoring that only ever makes MORE jobs count as reachable, so it cannot open the hole
//! this check exists to close; no job in the tree relies on it. If one ever does, it takes an
//! exemption and this note gets revisited.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_yaml::Value;

use crate::truth::Truth;

/// Where the workflow files live, relative to the repo root.
const WORKFLOW_DIR: &str = ".github/workflows";

/// The two events that carry code from someone who does not have write access.
const PR_EVENTS: [&str; 2] = ["pull_request", "pull_request_target"];

// ---- the counted exemption list -----------------------------------------------------------

/// A job allowed to route somewhere this module cannot prove is GitHub-hosted, despite being
/// reachable from a pull request.
pub struct Exemption {
    /// Workflow file name inside `.github/workflows/` (e.g. `ci.yml`).
    pub workflow: &'static str,
    /// Job id — the key under `jobs:`, not the display `name:`.
    pub job: &'static str,
    /// Why this is safe. Read by humans; its length is asserted so an empty placeholder cannot
    /// pass for a justification.
    pub reason: &'static str,
}

/// Every exemption in force. EMPTY, and that is the point: nothing in this tree needs one — the
/// one job that bills the org's Mac from a PR-triggered workflow is excluded by its own `if:`,
/// which the check reads, not by a line here.
///
/// A list that grows quietly is the next broken assurance, so this one cannot: adding an entry
/// changes `EXEMPTIONS.len()`, and `the_exemption_list_has_not_grown_unnoticed` asserts that
/// number. An entry naming a job that does not exist, or one the rule would have let through
/// anyway, fails the check too — so a stale exemption cannot sit here looking like a live
/// decision.
pub const EXEMPTIONS: &[Exemption] = &[];

/// Shortest reason that counts as one. Long enough that `"ok"` or `"TODO"` cannot pass.
const MIN_REASON_LEN: usize = 24;

// ---- GitHub-hosted label whitelist --------------------------------------------------------

/// Is `label` provably one of GitHub's own hosted runner images?
///
/// The families are `ubuntu`, `windows`, `macos`, always lowercase, always followed by a version
/// or `latest` and optionally a size/arch suffix: `ubuntu-latest`, `ubuntu-22.04-arm`, `macos-14`,
/// `macos-latest-xlarge`, `windows-2025`. Everything else is unproven, INCLUDING the mixed-case
/// `macOS` a self-hosted mac carries automatically — which is exactly the label that would route
/// to this org's machine without the word `self-hosted` appearing.
pub fn is_github_hosted_label(label: &str) -> bool {
    let Some((family, rest)) = label.split_once('-') else {
        return false;
    };
    matches!(family, "ubuntu" | "windows" | "macos")
        && !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

// ---- three-valued evaluation of a job's `if:` ---------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Str(String),
    Bool(bool),
    Unknown,
}

impl Val {
    fn truth(&self) -> Truth {
        match self {
            // GitHub's truthiness: the empty string is falsy, any other string is truthy.
            Val::Str(s) if s.is_empty() => Truth::False,
            Val::Str(_) => Truth::True,
            Val::Bool(b) => {
                if *b {
                    Truth::True
                } else {
                    Truth::False
                }
            }
            Val::Unknown => Truth::Unknown,
        }
    }
    fn from_truth(t: Truth) -> Val {
        match t {
            Truth::True => Val::Bool(true),
            Truth::False => Val::Bool(false),
            Truth::Unknown => Val::Unknown,
        }
    }
}

/// Evaluate a job's `if:` under the hypothesis `github.event_name == event`.
///
/// Returns `Truth::False` ONLY when the expression is understood end to end and is definitely
/// false — the single case in which the caller may treat the job as unreachable for that event.
/// A parse failure, an unknown context, an unknown function: all `Unknown`.
pub fn eval_if(raw: &str, event: &str) -> Truth {
    let trimmed = raw.trim();
    // `if:` accepts both the bare expression and the `${{ … }}` form. Strip one whole wrapper;
    // anything that still carries `${{` afterwards is a mixed template we will not reason about.
    let inner = match trimmed
        .strip_prefix("${{")
        .and_then(|r| r.strip_suffix("}}"))
    {
        Some(inner) => inner,
        None => trimmed,
    };
    if inner.contains("${{") {
        return Truth::Unknown;
    }
    let mut p = ExprParser {
        src: inner.as_bytes(),
        pos: 0,
        event,
    };
    let Ok(v) = p.parse_expr() else {
        return Truth::Unknown;
    };
    p.skip_ws();
    if p.pos != p.src.len() {
        return Truth::Unknown; // trailing junk ⇒ we did not understand the whole thing
    }
    v.truth()
}

/// Recursive descent over the subset of GitHub's expression language that job conditions are
/// written in. Anything outside the subset resolves to `Val::Unknown` (or `Err`, which the caller
/// turns into `Unknown`) rather than being guessed.
///
/// `&&`/`||` are folded to truth values rather than GitHub's value-returning semantics. For an
/// `if:` the two agree — only truthiness is ever read — and `runs-on` is classified by
/// [`classify_runs_on`], which does not use this evaluator at all.
struct ExprParser<'a> {
    src: &'a [u8],
    pos: usize,
    event: &'a str,
}

impl ExprParser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && (self.src[self.pos] as char).is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self, tok: &str) -> bool {
        self.skip_ws();
        self.src[self.pos..].starts_with(tok.as_bytes())
    }

    fn eat(&mut self, tok: &str) -> bool {
        if self.peek(tok) {
            self.pos += tok.len();
            true
        } else {
            false
        }
    }

    fn parse_expr(&mut self) -> Result<Val, ()> {
        let mut v = self.parse_and()?;
        while self.eat("||") {
            let r = self.parse_and()?;
            v = Val::from_truth(v.truth().or(r.truth()));
        }
        Ok(v)
    }

    fn parse_and(&mut self) -> Result<Val, ()> {
        let mut v = self.parse_cmp()?;
        while self.eat("&&") {
            let r = self.parse_cmp()?;
            v = Val::from_truth(v.truth().and(r.truth()));
        }
        Ok(v)
    }

    fn parse_cmp(&mut self) -> Result<Val, ()> {
        let left = self.parse_unary()?;
        // `==`/`!=` are the only comparisons a workflow condition needs to be understood for;
        // the ordering ones are consumed and yield Unknown so the rest of the expression still
        // parses (and the job stays reachable).
        if self.eat("==") {
            let right = self.parse_unary()?;
            return Ok(Val::from_truth(equal(&left, &right)));
        }
        if self.eat("!=") {
            let right = self.parse_unary()?;
            return Ok(Val::from_truth(equal(&left, &right).not()));
        }
        for op in ["<=", ">=", "<", ">"] {
            if self.eat(op) {
                let _ = self.parse_unary()?;
                return Ok(Val::Unknown);
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Val, ()> {
        self.skip_ws();
        if self.peek("!") && !self.peek("!=") {
            self.pos += 1;
            let v = self.parse_unary()?;
            return Ok(Val::from_truth(v.truth().not()));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Val, ()> {
        self.skip_ws();
        if self.pos >= self.src.len() {
            return Err(());
        }
        if self.eat("(") {
            let v = self.parse_expr()?;
            if !self.eat(")") {
                return Err(());
            }
            return Ok(v);
        }
        let c = self.src[self.pos] as char;
        if c == '\'' {
            return self.parse_string();
        }
        if c.is_ascii_digit() {
            while self.pos < self.src.len()
                && ((self.src[self.pos] as char).is_ascii_digit() || self.src[self.pos] == b'.')
            {
                self.pos += 1;
            }
            return Ok(Val::Unknown); // numbers play no part in the conditions we decide
        }
        if c.is_ascii_alphabetic() || c == '_' {
            return self.parse_ident_or_call();
        }
        Err(())
    }

    fn parse_string(&mut self) -> Result<Val, ()> {
        debug_assert_eq!(self.src[self.pos], b'\'');
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.src.len() {
            if self.src[self.pos] == b'\'' {
                // `''` is GitHub's escape for a literal single quote.
                if self.src.get(self.pos + 1) == Some(&b'\'') {
                    out.push('\'');
                    self.pos += 2;
                    continue;
                }
                self.pos += 1;
                return Ok(Val::Str(out));
            }
            out.push(self.src[self.pos] as char);
            self.pos += 1;
        }
        Err(()) // unterminated
    }

    /// A dotted context path (`github.event_name`, `vars.X`, `needs.a.outputs.b`) or a function
    /// call. Index syntax (`github.event['x']`, `matrix.*`) is not in the subset and yields `Err`.
    fn parse_ident_or_call(&mut self) -> Result<Val, ()> {
        let start = self.pos;
        while self.pos < self.src.len() {
            let ch = self.src[self.pos] as char;
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(());
        }
        let path = std::str::from_utf8(&self.src[start..self.pos]).map_err(|_| ())?;
        if self.peek("(") {
            self.pos += 1; // consume `(`
            let mut args: Vec<Val> = Vec::new();
            if !self.eat(")") {
                loop {
                    args.push(self.parse_expr()?);
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat(")") {
                        break;
                    }
                    return Err(());
                }
            }
            return Ok(call(path, &args));
        }
        Ok(match path {
            "github.event_name" => Val::Str(self.event.to_string()),
            "true" => Val::Bool(true),
            "false" => Val::Bool(false),
            _ => Val::Unknown,
        })
    }
}

/// `==` over the value lattice. Only two known values of the same shape decide it.
fn equal(a: &Val, b: &Val) -> Truth {
    match (a, b) {
        (Val::Str(x), Val::Str(y)) => {
            if x == y {
                Truth::True
            } else {
                Truth::False
            }
        }
        (Val::Bool(x), Val::Bool(y)) => {
            if x == y {
                Truth::True
            } else {
                Truth::False
            }
        }
        _ => Truth::Unknown,
    }
}

/// The handful of built-ins whose result is decidable from known arguments. `success()`,
/// `failure()` and `cancelled()` depend on the run, not the file, so they stay `Unknown` — which
/// keeps their job reachable, the safe direction.
fn call(name: &str, args: &[Val]) -> Val {
    match (name, args) {
        ("always", []) => Val::Bool(true),
        ("contains", [Val::Str(hay), Val::Str(needle)]) => Val::Bool(hay.contains(needle.as_str())),
        ("startsWith", [Val::Str(s), Val::Str(p)]) => Val::Bool(s.starts_with(p.as_str())),
        ("endsWith", [Val::Str(s), Val::Str(p)]) => Val::Bool(s.ends_with(p.as_str())),
        _ => Val::Unknown,
    }
}

// ---- the workflow model --------------------------------------------------------------------

#[derive(Debug)]
struct Job {
    id: String,
    if_expr: Option<String>,
    runs_on: Option<Value>,
    matrix: Option<Value>,
    /// A reusable-workflow call (`uses: ./.github/workflows/x.yml`). Such a job has no `runs-on`
    /// of its own; the called workflow's jobs are checked in their own right.
    uses: Option<String>,
}

#[derive(Debug)]
struct Workflow {
    /// File name inside `.github/workflows/`, e.g. `ci.yml`.
    file: String,
    /// The `name:` a `workflow_run` trigger refers to; falls back to the file name, which is what
    /// GitHub displays when `name:` is absent.
    name: String,
    triggers: BTreeSet<String>,
    /// `on.workflow_run.workflows` — the display names this workflow hangs off.
    workflow_run_upstreams: Vec<String>,
    jobs: Vec<Job>,
}

fn map_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_mapping()?
        .iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, val)| val)
}

/// The `on:` block. YAML 1.1 readers fold a bare `on` key into the boolean `true`; serde_yaml
/// does not, but accepting both costs one line and makes the check independent of that.
fn on_block(doc: &Value) -> Option<&Value> {
    map_get(doc, "on").or_else(|| {
        doc.as_mapping()?
            .iter()
            .find(|(k, _)| matches!(k, Value::Bool(true)))
            .map(|(_, val)| val)
    })
}

/// Every trigger name in an `on:` block, in all three spellings GitHub allows
/// (`on: push`, `on: [push, pull_request]`, `on: {push: {...}}`).
fn trigger_names(on: &Value) -> BTreeSet<String> {
    match on {
        Value::String(s) => [s.clone()].into_iter().collect(),
        Value::Sequence(items) => items
            .iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect(),
        Value::Mapping(m) => m
            .iter()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn parse_workflow(file: &str, body: &str) -> Result<Workflow> {
    let doc: Value = serde_yaml::from_str(body)
        .with_context(|| format!("{WORKFLOW_DIR}/{file}: not parseable as YAML"))?;
    let on = on_block(&doc)
        .ok_or_else(|| anyhow::anyhow!("{WORKFLOW_DIR}/{file}: no `on:` block"))?
        .clone();
    let triggers = trigger_names(&on);
    let workflow_run_upstreams = map_get(&on, "workflow_run")
        .and_then(|wr| map_get(wr, "workflows"))
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut jobs = Vec::new();
    if let Some(Value::Mapping(m)) = map_get(&doc, "jobs") {
        for (k, v) in m {
            let Some(id) = k.as_str() else { continue };
            jobs.push(Job {
                id: id.to_string(),
                if_expr: map_get(v, "if").map(|c| match c {
                    Value::String(s) => s.clone(),
                    Value::Bool(b) => b.to_string(),
                    other => serde_yaml::to_string(other).unwrap_or_default(),
                }),
                runs_on: map_get(v, "runs-on").cloned(),
                matrix: map_get(v, "strategy")
                    .and_then(|s| map_get(s, "matrix"))
                    .cloned(),
                uses: map_get(v, "uses")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }

    Ok(Workflow {
        file: file.to_string(),
        name: map_get(&doc, "name")
            .and_then(Value::as_str)
            .unwrap_or(file)
            .to_string(),
        triggers,
        workflow_run_upstreams,
        jobs,
    })
}

fn load_workflows(root: &Path) -> Result<Vec<Workflow>> {
    let dir = root.join(WORKFLOW_DIR);
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("{}: no workflow files found", dir.display());
    }
    files
        .iter()
        .map(|f| {
            let body = std::fs::read_to_string(dir.join(f))
                .with_context(|| format!("reading {WORKFLOW_DIR}/{f}"))?;
            parse_workflow(f, &body)
        })
        .collect()
}

// ---- reachability ----------------------------------------------------------------------------

/// A job is reachable for an entry event unless its `if:` is PROVABLY false for that event.
fn job_reachable_for(job: &Job, event: &str) -> bool {
    match &job.if_expr {
        None => true,
        Some(expr) => eval_if(expr, event) != Truth::False,
    }
}

fn job_reachable(job: &Job, events: &BTreeSet<String>) -> bool {
    events.iter().any(|e| job_reachable_for(job, e))
}

/// The `uses:` target of a reusable-workflow call, as a workflow file name in THIS repo. A call
/// into another repository (`owner/repo/.github/workflows/x.yml@ref`) is not our tree and is
/// skipped — the labels it routes to live in that repo's file, and the runner group is what keeps
/// it off this org's Mac.
fn local_called_workflow(uses: &str) -> Option<String> {
    let path = uses.strip_prefix("./")?;
    let rest = path.strip_prefix(WORKFLOW_DIR)?.strip_prefix('/')?;
    (!rest.contains('/')).then(|| rest.to_string())
}

/// For each workflow file, the set of `github.event_name` values under which untrusted pull-request
/// code can reach it. Empty ⇒ out of reach of a pull request.
///
/// Propagation, to a fixpoint because a chain can be arbitrarily long:
/// - direct: the workflow declares `pull_request` and/or `pull_request_target`;
/// - `workflow_call`: a REACHABLE job of a reachable workflow calls it — and a called workflow
///   keeps the caller's event, so the caller's entry events carry over unchanged;
/// - `workflow_run`: it hangs off a workflow that is itself reachable — and there the event
///   becomes `workflow_run`, which is why it is added rather than inherited.
fn entry_events(workflows: &[Workflow]) -> BTreeMap<String, BTreeSet<String>> {
    let mut entry: BTreeMap<String, BTreeSet<String>> = workflows
        .iter()
        .map(|w| {
            let events: BTreeSet<String> = PR_EVENTS
                .iter()
                .filter(|e| w.triggers.contains(**e))
                .map(|e| e.to_string())
                .collect();
            (w.file.clone(), events)
        })
        .collect();

    let by_name: BTreeMap<&str, &str> = workflows
        .iter()
        .map(|w| (w.name.as_str(), w.file.as_str()))
        .collect();

    loop {
        let mut changed = false;
        let mut add = |entry: &mut BTreeMap<String, BTreeSet<String>>,
                       file: &str,
                       events: &BTreeSet<String>| {
            if let Some(slot) = entry.get_mut(file) {
                for e in events {
                    changed |= slot.insert(e.clone());
                }
            }
        };

        for w in workflows {
            let events = entry.get(&w.file).cloned().unwrap_or_default();
            if events.is_empty() {
                continue;
            }
            // workflow_call: a reachable caller job hands its own event to the called workflow.
            for job in &w.jobs {
                let Some(uses) = &job.uses else { continue };
                let Some(called) = local_called_workflow(uses) else {
                    continue;
                };
                let reachable_with: BTreeSet<String> = events
                    .iter()
                    .filter(|e| job_reachable_for(job, e))
                    .cloned()
                    .collect();
                if !reachable_with.is_empty() {
                    add(&mut entry, &called, &reachable_with);
                }
            }
            // workflow_run: anything hanging off this workflow is entered as `workflow_run`.
            for other in workflows {
                let hangs_off = other
                    .workflow_run_upstreams
                    .iter()
                    .any(|n| by_name.get(n.as_str()) == Some(&w.file.as_str()));
                if hangs_off {
                    add(
                        &mut entry,
                        &other.file,
                        &["workflow_run".to_string()].into_iter().collect(),
                    );
                }
            }
        }
        if !changed {
            return entry;
        }
    }
}

// ---- runner routing --------------------------------------------------------------------------

/// What a job's `runs-on` can be proven to be.
#[derive(Debug, PartialEq, Eq)]
enum Routing {
    /// Every label this job can land on is a GitHub-hosted image.
    ProvablyHosted,
    /// It is not; the string says which part could not be proven.
    Unproven(String),
}

/// Candidate values of `matrix.<key>`, from the job's own `strategy.matrix` — both the top-level
/// axis and any `include:` entry that sets the key. `None` means "cannot be resolved from the
/// file", which the caller treats as unproven.
fn matrix_candidates(matrix: Option<&Value>, key: &str) -> Option<Vec<String>> {
    let matrix = matrix?;
    let mut out: Vec<String> = Vec::new();
    if let Some(axis) = map_get(matrix, key) {
        let seq = axis.as_sequence()?;
        for item in seq {
            out.push(item.as_str()?.to_string());
        }
    }
    if let Some(include) = map_get(matrix, "include") {
        let seq = include.as_sequence()?;
        for entry in seq {
            if let Some(v) = map_get(entry, key) {
                out.push(v.as_str()?.to_string());
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Classify one `runs-on` value.
///
/// The bar is PROOF, not the absence of a keyword — see the module header for why a blacklist of
/// `self-hosted` would not hold. The one expression form accepted is a whole-value `${{ matrix.x }}`
/// whose candidates are all hosted labels, because that is the single legitimate pattern in this
/// tree's workflows; anything richer takes an exemption, which is visible and counted, rather than
/// an evaluator whose gaps would not be.
fn classify_runs_on(runs_on: Option<&Value>, matrix: Option<&Value>) -> Routing {
    let Some(runs_on) = runs_on else {
        return Routing::Unproven("the job declares no `runs-on`".into());
    };
    let labels: Vec<String> = match runs_on {
        Value::String(s) => vec![s.clone()],
        Value::Sequence(items) => {
            let mut out = Vec::new();
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Routing::Unproven(
                            "a `runs-on` list entry is not a plain label".into(),
                        )
                    }
                }
            }
            out
        }
        // `runs-on: {group: …, labels: […]}` selects a RUNNER GROUP, which exists only for
        // self-hosted and larger runners. Never provable from here.
        Value::Mapping(_) => {
            return Routing::Unproven("`runs-on` selects a runner group".into());
        }
        _ => return Routing::Unproven("`runs-on` is neither a label, a list, nor a group".into()),
    };

    for label in &labels {
        if !label.contains("${{") {
            if !is_github_hosted_label(label) {
                return Routing::Unproven(format!(
                    "label `{label}` is not a GitHub-hosted runner image"
                ));
            }
            continue;
        }
        // An expression. Only the whole-value `${{ matrix.<key> }}` form is resolvable here.
        let inner = label
            .trim()
            .strip_prefix("${{")
            .and_then(|r| r.strip_suffix("}}"))
            .map(str::trim);
        let resolved = inner
            .filter(|e| !e.contains("${{"))
            .and_then(|e| e.strip_prefix("matrix."))
            .filter(|k| {
                !k.is_empty()
                    && k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
            .and_then(|k| matrix_candidates(matrix, k));
        let Some(candidates) = resolved else {
            return Routing::Unproven(format!(
                "`runs-on` expression `{}` cannot be resolved to a fixed set of labels",
                label.trim()
            ));
        };
        for c in candidates {
            if !is_github_hosted_label(&c) {
                return Routing::Unproven(format!(
                    "`runs-on` expression `{}` can resolve to `{c}`, which is not a \
                     GitHub-hosted runner image",
                    label.trim()
                ));
            }
        }
    }
    Routing::ProvablyHosted
}

// ---- the check ---------------------------------------------------------------------------------

/// One job that breaks the rule.
#[derive(Debug)]
struct Violation {
    workflow: String,
    job: String,
    events: Vec<String>,
    why: String,
}

/// Run the guard against the repo at `root` with the exemptions in force. Reports EVERY offending
/// job at once, not the first — a reader who has just edited the workflows wants the whole verdict
/// from one run.
pub fn check(root: &Path) -> Result<()> {
    check_with(root, EXEMPTIONS)
}

/// The body of [`check`], with the exemption list as a parameter so the escape hatch itself is
/// testable — an untested one would be the very thing this module exists to stop.
fn check_with(root: &Path, exemptions: &[Exemption]) -> Result<()> {
    let workflows = load_workflows(root)?;
    check_exemptions_are_well_formed(exemptions)?;
    let entry = entry_events(&workflows);

    let mut violations: Vec<Violation> = Vec::new();
    for w in &workflows {
        let events = entry.get(&w.file).cloned().unwrap_or_default();
        if events.is_empty() {
            continue;
        }
        for job in &w.jobs {
            if job.uses.is_some() {
                continue; // a reusable-workflow call has no runner of its own
            }
            if !job_reachable(job, &events) {
                continue;
            }
            if let Routing::Unproven(why) =
                classify_runs_on(job.runs_on.as_ref(), job.matrix.as_ref())
            {
                violations.push(Violation {
                    workflow: w.file.clone(),
                    job: job.id.clone(),
                    events: events
                        .iter()
                        .filter(|e| job_reachable_for(job, e))
                        .cloned()
                        .collect(),
                    why,
                });
            }
        }
    }

    // An exemption is a live decision about a job the rule WOULD have flagged. One that names a
    // job the rule lets through anyway (or no job at all) is a leftover, and leftovers are how a
    // list starts growing quietly — so they fail here rather than accumulate.
    let mut stale: Vec<String> = Vec::new();
    for ex in exemptions {
        let matched = violations
            .iter()
            .any(|v| v.workflow == ex.workflow && v.job == ex.job);
        if !matched {
            let job_exists = workflows
                .iter()
                .any(|w| w.file == ex.workflow && w.jobs.iter().any(|j| j.id == ex.job));
            stale.push(format!(
                "  - {}/{}: {}",
                ex.workflow,
                ex.job,
                if job_exists {
                    "the rule does not flag this job — the exemption is not needed"
                } else {
                    "no such job in that workflow"
                }
            ));
        }
    }
    violations.retain(|v| {
        !exemptions
            .iter()
            .any(|ex| ex.workflow == v.workflow && ex.job == v.job)
    });

    if !stale.is_empty() {
        bail!(
            "stale self-hosted-runner exemption(s) in xtask/src/runners.rs:\n{}\n\
             Remove them; an exemption that guards nothing is a decision nobody is making.",
            stale.join("\n")
        );
    }
    if !violations.is_empty() {
        let detail = violations
            .iter()
            .map(|v| {
                format!(
                    "  - {WORKFLOW_DIR}/{}: job `{}` is reachable from {} and {}",
                    v.workflow,
                    v.job,
                    v.events.join(" / "),
                    v.why
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "a job reachable from a pull request may route to a runner this repo cannot prove is \
             GitHub-hosted (nexus-flow-6j6v.2sb1):\n{detail}\n\
             The org's self-hosted Mac is not ephemeral, shares a login session with a human and \
             holds the Developer ID signing material — untrusted pull-request code must never \
             reach it. Either guard the job (`if: github.event_name != 'pull_request'`), route it \
             to a GitHub-hosted image, or add a justified entry to EXEMPTIONS in \
             xtask/src/runners.rs (which is counted — see the test next to it)."
        );
    }
    Ok(())
}

fn check_exemptions_are_well_formed(exemptions: &[Exemption]) -> Result<()> {
    for ex in exemptions {
        if ex.reason.trim().len() < MIN_REASON_LEN {
            bail!(
                "the exemption for {}/{} carries no real justification (reasons must be at least \
                 {MIN_REASON_LEN} characters)",
                ex.workflow,
                ex.job
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace root, derived from this crate's manifest dir (`<root>/xtask`).
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// A throwaway repo holding exactly `files` under `.github/workflows/`.
    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(WORKFLOW_DIR);
        std::fs::create_dir_all(&wf).unwrap();
        for (name, body) in files {
            std::fs::write(wf.join(name), body).unwrap();
        }
        dir
    }

    /// A copy of the REAL `.github/workflows/` with one exact line replaced in one file — the
    /// FIRST `from` at or after the unique line `after`.
    ///
    /// Fixtures prove the rule; this proves it against the file it actually guards. A
    /// hand-written fixture can silently drift into "whatever shape my parser happens to read",
    /// and then the check is green on a tree it never looked at — which is the failure mode
    /// 6j6v.2sb1 names twice.
    ///
    /// The `after` anchor is what keeps the rewrite aimed: since 6j6v.fb1d added the `macos-tests`
    /// lane, `ci.yml` carries the SAME guard line on two jobs, and only one of them is the
    /// self-hosted one these counter-proofs are about. Both the anchor (exactly once) and the
    /// rewritten line must be found, so a workflow that moves on breaks the counter-proof loudly
    /// instead of quietly proving nothing.
    fn real_tree_with_line_replaced(
        file: &str,
        after: &str,
        from: &str,
        to: &str,
    ) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join(WORKFLOW_DIR);
        std::fs::create_dir_all(&dst).unwrap();
        let src = repo_root().join(WORKFLOW_DIR);
        for e in std::fs::read_dir(&src).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().into_string().unwrap();
            let body = std::fs::read_to_string(e.path()).unwrap();
            let body = if name == file {
                let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
                assert_eq!(
                    lines.iter().filter(|l| *l == after).count(),
                    1,
                    "the anchor this counter-proof aims by must appear exactly once in {name}; \
                     if the workflow moved on, move the counter-proof with it"
                );
                let anchor = lines.iter().position(|l| l == after).unwrap();
                let target = lines[anchor..]
                    .iter()
                    .position(|l| l == from)
                    .map(|i| i + anchor)
                    .unwrap_or_else(|| {
                        panic!(
                            "no `{from}` line after `{after}` in {name}; the counter-proof has \
                             stopped rewriting what it names"
                        )
                    });
                lines[target] = to.to_string();
                lines.join("\n") + "\n"
            } else {
                body
            };
            std::fs::write(dst.join(name), body).unwrap();
        }
        dir
    }

    /// The job-level guard on `ci.yml`'s `platform-gates-macos`, verbatim. Four spaces of indent —
    /// the eight-space `if:` on the `Tests (release)` STEP reads identically and must not be hit.
    const CI_GUARD: &str = "    if: github.event_name != 'pull_request'";

    /// The line that tells the guard above apart from the identical one on the `macos-tests` lane
    /// (6j6v.fb1d): the self-hosted job's own display name, immediately above it.
    const CI_MACOS_JOB: &str = "    name: Platform gates (macos)";

    // ---- the live assertion ---------------------------------------------------------------

    #[test]
    fn the_tree_today_keeps_every_pull_request_job_off_an_unproven_runner() {
        // Green direction, first spelling: the real tree. `platform-gates-macos` sits in a
        // workflow WITH a `pull_request` trigger and routes through `vars.NXF_MACOS_RUNS_ON`; it
        // passes only because the check reads its `if:`. A rule that merely looked for
        // "self-hosted in a PR workflow" would be red here and would have been deleted by now.
        check(&repo_root()).expect("no pull-request-reachable job routes to an unproven runner");
    }

    // ---- counter-proofs against the REAL workflow file, both directions, twice each --------

    #[test]
    fn dropping_the_real_guard_turns_the_check_red() {
        // Red, spelling 1: the guard line is simply gone — the exact shape of "a later change
        // adds a macOS step and leaves the `if:` off".
        let tree =
            real_tree_with_line_replaced("ci.yml", CI_MACOS_JOB, CI_GUARD, "    # guard removed");
        let err = check(tree.path()).expect_err("an unguarded platform-gates-macos must fail");
        let msg = err.to_string();
        assert!(msg.contains("platform-gates-macos"), "names the job: {msg}");
        assert!(msg.contains("pull_request"), "names the event: {msg}");
    }

    #[test]
    fn weakening_the_real_guard_to_another_event_turns_the_check_red() {
        // Red, spelling 2: an `if:` is still THERE — it just does not exclude the pull request.
        // This is the counter-proof the ticket asks for a second time in a different spelling:
        // a check that only asked "does this job have an `if:`?" would stay green right here.
        let tree = real_tree_with_line_replaced(
            "ci.yml",
            CI_MACOS_JOB,
            CI_GUARD,
            "    if: github.event_name != 'schedule'",
        );
        let err = check(tree.path()).expect_err("a guard on the wrong event must fail");
        assert!(
            err.to_string().contains("platform-gates-macos"),
            "names the job: {err}"
        );
    }

    #[test]
    fn a_differently_spelled_real_guard_stays_green() {
        // Green, spelling 2: the same exclusion written as a positive allow-list of events. The
        // evaluator has to actually decide the expression, not pattern-match one blessed string.
        let tree = real_tree_with_line_replaced(
            "ci.yml",
            CI_MACOS_JOB,
            CI_GUARD,
            "    if: ${{ github.event_name == 'push' || github.event_name == 'workflow_dispatch' }}",
        );
        check(tree.path()).expect("an equivalent guard, written the other way round, is accepted");
    }

    #[test]
    fn a_belt_and_braces_real_guard_stays_green() {
        // Green, spelling 3 — and the one that also covers `pull_request_target`, which `ci.yml`
        // does not declare today but a future edit might.
        let tree = real_tree_with_line_replaced(
            "ci.yml",
            CI_MACOS_JOB,
            CI_GUARD,
            "    if: github.event_name != 'pull_request' && github.event_name != 'pull_request_target'",
        );
        check(tree.path()).expect("excluding both pull-request events is accepted");
    }

    // ---- the routing rule, on fixtures ------------------------------------------------------

    const PR_WORKFLOW_HEAD: &str = "name: Fixture\non:\n  pull_request:\njobs:\n";

    #[test]
    fn a_bare_self_hosted_label_on_a_pull_request_job_fails() {
        let body = format!(
            "{PR_WORKFLOW_HEAD}  mac:\n    runs-on: [self-hosted, macOS, X64]\n    steps: []\n"
        );
        let tree = fixture(&[("fixture.yml", &body)]);
        assert!(
            check(tree.path()).is_err(),
            "a self-hosted label on a pull-request job must fail"
        );
    }

    #[test]
    fn label_only_targeting_without_the_word_self_hosted_fails() {
        // The reason the rule is a whitelist. Since 2022 a job reaches a self-hosted runner by its
        // custom labels alone — this routes to the org's Mac and never says `self-hosted`, so a
        // blacklist would wave it straight through.
        let body = format!("{PR_WORKFLOW_HEAD}  mac:\n    runs-on: [macOS, X64]\n    steps: []\n");
        let tree = fixture(&[("fixture.yml", &body)]);
        assert!(
            check(tree.path()).is_err(),
            "label-only self-hosted targeting must fail"
        );
    }

    #[test]
    fn the_repo_variable_detour_fails_when_the_job_is_reachable() {
        // `vars.*` is a repository setting — its value is not in the tree, so it can never be
        // proven hosted. This is `ci.yml`'s own `runs-on` expression, verbatim, minus the guard.
        let body = format!(
            "{PR_WORKFLOW_HEAD}  mac:\n    runs-on: ${{{{ vars.NXF_MACOS_RUNS_ON != '' && \
             fromJSON(vars.NXF_MACOS_RUNS_ON) || 'macos-latest' }}}}\n    steps: []\n"
        );
        let tree = fixture(&[("fixture.yml", &body)]);
        let err = check(tree.path()).expect_err("an unguarded vars.* runs-on must fail");
        assert!(err.to_string().contains("cannot be resolved"), "{err}");
    }

    #[test]
    fn pull_request_target_counts_as_a_pull_request_event() {
        let body = "name: Fixture\non:\n  pull_request_target:\njobs:\n  mac:\n    runs-on: [self-hosted]\n    steps: []\n";
        let tree = fixture(&[("fixture.yml", body)]);
        assert!(
            check(tree.path()).is_err(),
            "pull_request_target carries untrusted code too"
        );
    }

    #[test]
    fn a_runner_group_is_never_provably_hosted() {
        let body = format!(
            "{PR_WORKFLOW_HEAD}  mac:\n    runs-on:\n      group: macs\n      labels: [macos-14]\n    steps: []\n"
        );
        let tree = fixture(&[("fixture.yml", &body)]);
        assert!(check(tree.path()).is_err(), "a runner group must fail");
    }

    #[test]
    fn hosted_labels_and_a_hosted_matrix_pass() {
        let body = format!(
            "{PR_WORKFLOW_HEAD}\
             \x20 lin:\n    runs-on: ubuntu-latest\n    steps: []\n\
             \x20 mtx:\n    strategy:\n      matrix:\n        os: [ubuntu-latest, windows-latest]\n        include:\n          - os: macos-14\n    runs-on: ${{{{ matrix.os }}}}\n    steps: []\n"
        );
        let tree = fixture(&[("fixture.yml", &body)]);
        check(tree.path()).expect("GitHub-hosted images, spelled directly or through a matrix");
    }

    #[test]
    fn a_matrix_that_can_resolve_to_a_self_hosted_label_fails() {
        let body = format!(
            "{PR_WORKFLOW_HEAD}  mtx:\n    strategy:\n      matrix:\n        os: [ubuntu-latest]\n        include:\n          - os: self-hosted\n    runs-on: ${{{{ matrix.os }}}}\n    steps: []\n"
        );
        let tree = fixture(&[("fixture.yml", &body)]);
        let err = check(tree.path()).expect_err("a matrix leg that lands self-hosted must fail");
        assert!(err.to_string().contains("self-hosted"), "{err}");
    }

    // ---- the detours ------------------------------------------------------------------------

    #[test]
    fn a_workflow_run_detour_is_reachable() {
        // No `pull_request` anywhere in `downstream.yml` — it hangs off one that has it.
        let up = "name: Upstream\non:\n  pull_request:\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: []\n";
        let down = "name: Downstream\non:\n  workflow_run:\n    workflows: [Upstream]\n    types: [completed]\njobs:\n  mac:\n    runs-on: [self-hosted, macOS]\n    steps: []\n";
        let tree = fixture(&[("upstream.yml", up), ("downstream.yml", down)]);
        let err = check(tree.path()).expect_err("a workflow_run detour is still reachable");
        assert!(err.to_string().contains("downstream.yml"), "{err}");
    }

    #[test]
    fn a_workflow_run_job_is_not_excluded_by_a_pull_request_guard() {
        // The entry event downstream is `workflow_run`, so `github.event_name != 'pull_request'`
        // is TRUE there and excludes nothing. A model that propagated the caller's event would
        // wrongly call this job unreachable.
        let up = "name: Upstream\non:\n  pull_request:\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: []\n";
        let down = "name: Downstream\non:\n  workflow_run:\n    workflows: [Upstream]\njobs:\n  mac:\n    if: github.event_name != 'pull_request'\n    runs-on: [self-hosted]\n    steps: []\n";
        let tree = fixture(&[("upstream.yml", up), ("downstream.yml", down)]);
        assert!(
            check(tree.path()).is_err(),
            "a pull_request guard means nothing under a workflow_run entry"
        );
    }

    #[test]
    fn a_workflow_call_detour_is_reachable_and_keeps_the_callers_event() {
        let called = "name: Called\non:\n  workflow_call:\njobs:\n  mac:\n    runs-on: [self-hosted]\n    steps: []\n";
        let caller = "name: Caller\non:\n  pull_request:\njobs:\n  call:\n    uses: ./.github/workflows/called.yml\n";
        let tree = fixture(&[("called.yml", called), ("caller.yml", caller)]);
        assert!(
            check(tree.path()).is_err(),
            "a called workflow's jobs are reachable from the caller's pull request"
        );

        // …and the caller's guard carries over, because a called workflow keeps the caller's
        // `github.event_name`. Same two files, one `if:` on the calling job.
        let guarded_caller = "name: Caller\non:\n  pull_request:\njobs:\n  call:\n    if: github.event_name != 'pull_request'\n    uses: ./.github/workflows/called.yml\n";
        let tree = fixture(&[("called.yml", called), ("caller.yml", guarded_caller)]);
        check(tree.path()).expect("a guarded caller leaves the called workflow out of reach");
    }

    #[test]
    fn a_workflow_out_of_reach_of_a_pull_request_may_use_the_self_hosted_mac() {
        // The whole point of the ticket is that `release.yml` KEEPS doing this. A rule that
        // banned the label outright would be a different, wrong rule.
        let body = "name: Release\non:\n  push:\n    tags: ['v*']\njobs:\n  mac:\n    runs-on: [self-hosted, macOS, X64]\n    steps: []\n";
        let tree = fixture(&[("release.yml", body)]);
        check(tree.path()).expect("a tag-push workflow is not reachable from a pull request");
    }

    // ---- the exemption escape hatch ----------------------------------------------------------

    #[test]
    fn the_exemption_list_has_not_grown_unnoticed() {
        // A list that grows quietly is the next broken assurance. Adding an entry must change
        // this number — and that edit is what puts the decision in front of a reviewer.
        assert_eq!(
            EXEMPTIONS.len(),
            0,
            "an exemption was added to xtask/src/runners.rs. That is allowed, and it is not a \
             formality: it lets untrusted pull-request code onto a machine holding the Developer \
             ID signing material unless something else stops it. Update this number in the same \
             commit and say in the review why the runner-group scoping covers the case."
        );
    }

    #[test]
    fn an_exemption_suppresses_exactly_the_job_it_names() {
        let body = format!(
            "{PR_WORKFLOW_HEAD}  mac:\n    runs-on: [self-hosted]\n    steps: []\n  other:\n    runs-on: [self-hosted]\n    steps: []\n"
        );
        let tree = fixture(&[("fixture.yml", &body)]);
        let exemptions = [Exemption {
            workflow: "fixture.yml",
            job: "mac",
            reason: "fixture: the runner group keeps this off the org Mac",
        }];
        let err = check_with(tree.path(), &exemptions)
            .expect_err("the job that is NOT exempted still fails");
        let msg = err.to_string();
        assert!(
            msg.contains("`other`"),
            "the unexempted job is named: {msg}"
        );
        assert!(!msg.contains("`mac`"), "the exempted job is not: {msg}");
    }

    #[test]
    fn an_exemption_that_guards_nothing_fails() {
        // Both shapes of leftover: a job the rule already lets through, and a job that is gone.
        let body = format!("{PR_WORKFLOW_HEAD}  lin:\n    runs-on: ubuntu-latest\n    steps: []\n");
        let tree = fixture(&[("fixture.yml", &body)]);
        for (job, expected) in [("lin", "not needed"), ("vanished", "no such job")] {
            let exemptions = [Exemption {
                workflow: "fixture.yml",
                job,
                reason: "fixture: a reason long enough to count as one",
            }];
            let err = check_with(tree.path(), &exemptions).expect_err("a stale exemption fails");
            assert!(err.to_string().contains(expected), "{err}");
        }
    }

    #[test]
    fn an_exemption_without_a_real_reason_fails() {
        let tree = fixture(&[("fixture.yml", "name: F\non:\n  push:\njobs: {}\n")]);
        let exemptions = [Exemption {
            workflow: "fixture.yml",
            job: "mac",
            reason: "TODO",
        }];
        assert!(check_with(tree.path(), &exemptions).is_err());
    }

    // ---- the pieces ---------------------------------------------------------------------------

    #[test]
    fn hosted_labels_are_recognised_and_self_hosted_ones_are_not() {
        for hosted in [
            "ubuntu-latest",
            "ubuntu-22.04",
            "ubuntu-22.04-arm",
            "macos-14",
            "macos-latest",
            "macos-latest-xlarge",
            "windows-2025",
            "windows-11-arm",
        ] {
            assert!(is_github_hosted_label(hosted), "{hosted} is hosted");
        }
        for unproven in [
            "self-hosted",
            "macOS",      // the auto-label of a self-hosted mac
            "X64",        // ditto
            "Linux",      // ditto
            "macos",      // no version suffix ⇒ not an image label
            "MACOS-14",   // a runner may carry any label it likes, including this
            "nxf-mac-01", // an org-named runner
            "",
        ] {
            assert!(
                !is_github_hosted_label(unproven),
                "{unproven} is not hosted"
            );
        }
    }

    #[test]
    fn eval_if_decides_only_what_it_understands() {
        // Decided false ⇒ the job is excluded.
        for expr in [
            "github.event_name != 'pull_request'",
            "${{ github.event_name != 'pull_request' }}",
            "github.event_name == 'push'",
            "github.event_name == 'push' || github.event_name == 'workflow_dispatch'",
            "github.event_name != 'pull_request' && github.event_name != 'pull_request_target'",
            "!(github.event_name == 'pull_request' || github.event_name == 'push') && true",
            "contains('push workflow_dispatch', github.event_name)",
            "false",
        ] {
            assert_eq!(
                eval_if(expr, "pull_request"),
                Truth::False,
                "should exclude a pull request: {expr}"
            );
        }
        // Decided true ⇒ reachable.
        for expr in [
            "github.event_name == 'pull_request'",
            "always()",
            "github.event_name != 'schedule'",
            "startsWith(github.event_name, 'pull')",
        ] {
            assert_eq!(eval_if(expr, "pull_request"), Truth::True, "{expr}");
        }
        // Not understood ⇒ Unknown, which leaves the job reachable. The safe direction.
        for expr in [
            "success()",
            "github.repository == 'nxsflow/nexus-flow'",
            "vars.RUN_MAC == 'yes'",
            "github.event.pull_request.head.repo.full_name == github.repository",
            "needs.build.result == 'success'",
            "github.event_name != 'pull_request' && ${{ nested }}",
            "github.event_name !=", // malformed
            "github.event_name == 'x' trailing",
        ] {
            assert_eq!(eval_if(expr, "pull_request"), Truth::Unknown, "{expr}");
        }
        // The same expression decides differently per event — which is what makes the
        // entry-event model necessary rather than decorative.
        assert_eq!(
            eval_if("github.event_name != 'pull_request'", "workflow_run"),
            Truth::True
        );
    }

    #[test]
    fn a_matrix_axis_resolves_from_both_the_axis_and_include() {
        let matrix: Value = serde_yaml::from_str(
            "os: [ubuntu-latest, windows-latest]\ninclude:\n  - os: macos-14\n    extra: x\n",
        )
        .unwrap();
        assert_eq!(
            matrix_candidates(Some(&matrix), "os"),
            Some(vec![
                "ubuntu-latest".to_string(),
                "windows-latest".to_string(),
                "macos-14".to_string()
            ])
        );
        // A key nothing sets, and a matrix built at runtime, are both unresolvable.
        assert_eq!(matrix_candidates(Some(&matrix), "builder"), None);
        let dynamic: Value =
            serde_yaml::from_str("os: ${{ fromJSON(needs.plan.outputs.matrix) }}\n").unwrap();
        assert_eq!(matrix_candidates(Some(&dynamic), "os"), None);
        assert_eq!(matrix_candidates(None, "os"), None);
    }

    #[test]
    fn every_workflow_in_the_tree_parses() {
        // The check is worth nothing on a file it could not read, and `load_workflows` is
        // fail-closed about that — this is the assertion that the tree stays inside the subset.
        let workflows = load_workflows(&repo_root()).expect("every workflow file parses");
        assert!(workflows.len() >= 10, "found {} workflows", workflows.len());
        let ci = workflows.iter().find(|w| w.file == "ci.yml").unwrap();
        assert!(ci.triggers.contains("pull_request"), "{:?}", ci.triggers);
        assert!(
            ci.jobs.iter().any(|j| j.id == "platform-gates-macos"),
            "the guarded job is still there under that id"
        );
    }
}

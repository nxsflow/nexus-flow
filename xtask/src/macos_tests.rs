//! `cargo xtask macos-tests check` (nexus-flow-6j6v.fb1d): every `#[cfg(target_os = "macos")]`
//! TEST must live in a package the macOS lane actually runs.
//!
//! The hole this closes is one nobody would notice. Every test job in `ci.yml` runs on
//! `ubuntu-latest`, so a test the compiler removes off macOS is a test no gate ever executes —
//! and it goes on looking like coverage in the source tree. It is not a hypothesis: on 2026-09-03
//! the count was ~18 such tests across five files, two of them gated at MODULE level, and they
//! covered launchd installation, the wake assertion and the service's program resolution — the
//! machinery three consecutive work orders hung on.
//!
//! `macos-tests` (the job) is the answer to that. THIS is the answer to the next one: without a
//! gate, the lane is complete on the day it is written and incomplete from the next new test on.
//! Same fail-closed shape as `platforms check`, `channels check` and `docs check`, and it runs on
//! ubuntu because it only ever reads source text.
//!
//! WHAT THE RULE IS: if a package holds a test whose compilation is conditional on macOS, that
//! package must appear in the macOS lane's own `cargo test` command.
//!
//! Three decisions carry it:
//!
//! 1. THE LANE IS READ, NOT RESTATED. The package list comes out of `ci.yml`'s job — the same
//!    single-source shape as the drift gates next to it. Dropping `-p nexus-chat` from the
//!    workflow reddens this check instead of silently un-covering two tests. And the package list
//!    is only HALF of what decides coverage, so the lane's `cargo test` is read for scope too:
//!    `--lib` or a libtest filter keeps every `-p` while dropping what actually runs, so a
//!    narrowed lane fails here rather than reporting the coverage it no longer has.
//!
//! 2. MACOS-ONLY IS EVALUATED, NOT MATCHED. Every gated test in this tree has a
//!    `cfg(not(target_os = "macos"))` twin that DOES run on ubuntu, and
//!    `cfg(any(target_os = "macos", target_os = "linux"))` runs there too. A grep for the string
//!    would flag all of them. So each predicate is evaluated three-valued under two targets, and
//!    counts only when it can hold on macOS and provably cannot on linux (`Truth::Unknown` —
//!    a `feature = "x"` this module cannot resolve — never rules a site OUT).
//!
//! 3. TEST CODE, NOT ALL CODE, AND A GATE IS CARRIED DOWN. Everything under a package's `tests/`
//!    directory is test code by construction. Under `src/` the macOS gates are mostly PRODUCTION
//!    (`launchd.rs` is macOS-only in its entirety) and belong to `platform-gates-macos`'s clippy
//!    pass, not to this lane. So there a `#[test]` counts when its OWN attributes, any enclosing
//!    gated block, or the file's own `#![cfg(…)]` is macOS-only — see `sites_in_source`, which
//!    carries the whole reasoning and the four false negatives that produced it.
//!
//! WHAT IS DELIBERATELY NOT MODELLED: a macOS gate that reaches a test through a NAME it is not
//! written on — a `#[cfg]` on a `use`, or a gated helper the test calls. Missing those cannot
//! open the hole this exists to close: the test itself would still have to be gated to disappear
//! from an ubuntu run.
//!
//! That sentence used to excuse far more than it should. The first cut missed FIVE shapes that
//! gate a test directly, and two rounds of independent review of PR #423 found all five — four
//! that the scope stack now covers, and one it structurally cannot, because
//! `#[cfg(target_os = "macos")] mod mac_tests;` opens no block at all and the file it names
//! carries no gate of its own. That last one is followed across files by `inherited_gates`. All
//! five are fixed rather than disclaimed: a gate that quietly covers less than it claims is the
//! very failure this exists to prevent, and writing the shortfall into the header would only have
//! made it a documented one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_yaml::Value;

use crate::truth::Truth;

/// Where the CI workflow lives, relative to the repo root.
const CI_WORKFLOW: &str = ".github/workflows/ci.yml";

/// The job id (the key under `jobs:`, not the display `name:`) of the macOS test lane.
const LANE_JOB: &str = "macos-tests";

/// Where packages live, relative to the repo root.
const CRATES_DIR: &str = "crates";

// ---- three-valued evaluation of a cfg predicate --------------------------------------------

/// Evaluate a cfg predicate (the inside of `cfg(...)`) for `target_os`.
///
/// Only `macos` and `linux` are ever passed in — the two sides of the question "does an ubuntu
/// job see this test?" — so `unix`/`windows` are answered for those two and nothing else.
pub fn eval(pred: &str, target_os: &str) -> Truth {
    let p = pred.trim();
    if let Some(inner) = call_arg(p, "not") {
        return eval(inner, target_os).not();
    }
    if let Some(inner) = call_arg(p, "all") {
        return split_top(inner)
            .into_iter()
            .fold(Truth::True, |acc, x| acc.and(eval(x, target_os)));
    }
    if let Some(inner) = call_arg(p, "any") {
        return split_top(inner)
            .into_iter()
            .fold(Truth::False, |acc, x| acc.or(eval(x, target_os)));
    }
    let known_unix = matches!(target_os, "macos" | "linux");
    if let Some((key, value)) = p.split_once('=') {
        let value = value.trim().trim_matches('"');
        return match key.trim() {
            "target_os" => Truth::of(value == target_os),
            "target_family" if known_unix => Truth::of(value == "unix"),
            "target_vendor" if known_unix => {
                Truth::of((value == "apple") == (target_os == "macos"))
            }
            _ => Truth::Unknown,
        };
    }
    match p {
        "unix" if known_unix => Truth::True,
        "windows" if known_unix => Truth::False,
        // A cfg-test build is what every site here is read in.
        "test" => Truth::True,
        _ => Truth::Unknown,
    }
}

/// Can this predicate hold on macOS while provably not holding on linux — i.e. is the thing it
/// gates invisible to every ubuntu job in this repo?
///
/// `Unknown` on the macOS side still counts (a `feature = "x"` this module cannot resolve may
/// well be on); only a provable `False` on linux makes a gate this lane's business.
pub fn is_macos_only(pred: &str) -> bool {
    eval(pred, "macos") != Truth::False && eval(pred, "linux") == Truth::False
}

/// `name(...)` with nothing after the closing paren — the inside, or `None`.
fn call_arg<'a>(p: &'a str, name: &str) -> Option<&'a str> {
    let rest = p.strip_prefix(name)?;
    let (inner, after) = balanced_parens(rest)?;
    after.trim().is_empty().then_some(inner)
}

/// Split `s` at top-level commas, ignoring commas inside parentheses or string literals.
fn split_top(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut in_str, mut start) = (0i32, false, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// `s` must open with `(`: returns (inside, everything after the matching `)`).
fn balanced_parens(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if !s.starts_with('(') {
        return None;
    }
    let (mut depth, mut in_str) = (0i32, false);
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[1..i], &s[i + c.len_utf8()..]));
                }
            }
            _ => {}
        }
    }
    None
}

// ---- reading the gates out of one source file ----------------------------------------------

/// One macOS-gated test site inside a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSite {
    /// 1-based line of the attribute that carries the gate.
    pub line: usize,
    /// The predicate as written, e.g. `target_os = "macos"`.
    pub predicate: String,
}

/// Every macOS-gated TEST site in one Rust source file.
///
/// `in_test_target` says whether the file is an integration-test target (`<pkg>/tests/**`), where
/// every line is test code — under `src/` a gate counts only when it reaches a test.
///
/// REACHES, not "sits beside". The first cut of this read one attribute cluster at a time and
/// asked whether THAT cluster held both a `#[test]` and a macOS gate. The independent review of
/// PR #423 found four false negatives in that reading, and all four are shapes a person would
/// actually write:
///
///   1. a `src/` file opened with `#![cfg(target_os = "macos")]` — the whole file, tests included
///   2. `#[cfg(all(test, target_os = "macos"))] mod …` — one combined attribute, not two
///   3. `#[cfg(target_os = "macos")] mod mac { #[test] … }` — no `cfg(test)` beside the gate
///   4. …the same, nested inside the crate's ordinary `#[cfg(test)] mod tests`
///
/// A follow-up round found a fifth, `mod mac_tests;` with no body — see the note at the end.
///
/// So a gate is carried DOWN into the block it opens: the file's own inner attribute, and a stack
/// of gated blocks. A `#[test]` counts when its own cluster, any enclosing gated block, or the
/// file itself is macOS-only. That also fixes the other direction, which matters just as much on a
/// runner that bills at 10x: `#[cfg(target_os = "macos")] pub fn install(…) { … }` opens a block
/// too, and the ordinary tests AFTER it are outside it — `nxs-foundation` is the real instance,
/// and demanding it in the lane would be a bill for nothing.
///
/// A fifth shape is beyond this function by construction and is handled in `scan` instead:
/// `#[cfg(target_os = "macos")] mod mac_tests;` opens no block for a gate to be carried into, and
/// the file it names holds no gate to find. See `inherited_gates`.
///
/// WHAT IS STILL DELIBERATELY NOT MODELLED: a gate that reaches a test only through a NAME — a
/// `#[cfg]` on a `use`, or a gated helper the test calls. Missing those cannot open the hole this
/// exists to close: the test itself would still have to be gated to disappear from an ubuntu run.
pub fn sites_in_source(src: &str, in_test_target: bool) -> Vec<GateSite> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut cluster: Vec<(usize, String)> = Vec::new();
    // The gate on the file itself. It never closes.
    let mut file_gate: Option<(usize, String)> = None;
    // Enclosing macOS-gated blocks, innermost last: (the depth the block opened AT, its gate).
    let mut scopes: Vec<(usize, (usize, String))> = Vec::new();
    let mut depth = 0usize;
    let mut lexer = ScopeLexer::default();

    let mut i = 0usize;
    while i < lines.len() {
        let head = lines[i].trim_start();
        // Prose about a gate is not a gate — this module's own header names one repeatedly.
        if head.is_empty() || head.starts_with("//") {
            depth = lexer.depth_after(lines[i], depth);
            i += 1;
            continue;
        }
        if head.starts_with("#[") || head.starts_with("#![") {
            let (text, next) = read_attribute(&lines, i);
            if head.starts_with("#![") {
                // An inner attribute gates everything that follows it in this file.
                if let Some(predicate) = macos_gate(&text) {
                    let site = (i + 1, predicate);
                    if in_test_target {
                        out.push(GateSite {
                            line: site.0,
                            predicate: site.1.clone(),
                        });
                    }
                    file_gate.get_or_insert(site);
                }
            } else {
                cluster.push((i + 1, text));
            }
            for line in &lines[i..next] {
                depth = lexer.depth_after(line, depth);
            }
            i = next;
            continue;
        }

        // An item head (or any other line of code) closes the cluster and may open a block.
        let before = depth;
        depth = lexer.depth_after(lines[i], depth);
        while scopes
            .last()
            .is_some_and(|(opened_at, _)| depth <= *opened_at)
        {
            scopes.pop();
        }
        if !cluster.is_empty() {
            let own_gate = cluster
                .iter()
                .find_map(|(line, a)| macos_gate(a).map(|p| (*line, p)));
            if in_test_target {
                // Everything here is test code by construction, so every gate is a site.
                for (line, attribute) in &cluster {
                    if let Some(predicate) = macos_gate(attribute) {
                        out.push(GateSite {
                            line: *line,
                            predicate,
                        });
                    }
                }
            } else if cluster.iter().any(|(_, a)| is_test_attribute(a)) {
                // A test: report the nearest gate that hides it from every ubuntu job.
                if let Some((line, predicate)) = own_gate
                    .clone()
                    .or_else(|| scopes.last().map(|(_, gate)| gate.clone()))
                    .or_else(|| file_gate.clone())
                {
                    out.push(GateSite { line, predicate });
                }
            }
            if depth > before {
                if let Some(gate) = own_gate {
                    scopes.push((before, gate));
                }
            }
            cluster.clear();
        }
        i += 1;
    }
    out
}

/// Brace depth, with braces inside comments, strings, chars and raw strings not counted.
///
/// The scope stack above is only as good as this: a `"{"` in a string literal that moved the
/// depth would close a gated block early and quietly stop reporting the tests inside it. It
/// carries state across lines because block comments and raw strings do.
#[derive(Default)]
struct ScopeLexer {
    in_block_comment: bool,
    raw_string_hashes: Option<usize>,
}

impl ScopeLexer {
    fn depth_after(&mut self, line: &str, mut depth: usize) -> usize {
        let c: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < c.len() {
            if self.in_block_comment {
                if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    self.in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if let Some(hashes) = self.raw_string_hashes {
                if c[i] == '"'
                    && c[i + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|h| **h == '#')
                        .count()
                        == hashes
                {
                    self.raw_string_hashes = None;
                    i += 1 + hashes;
                } else {
                    i += 1;
                }
                continue;
            }
            match c[i] {
                '/' if c.get(i + 1) == Some(&'/') => break,
                '/' if c.get(i + 1) == Some(&'*') => {
                    self.in_block_comment = true;
                    i += 2;
                }
                'r' if matches!(c.get(i + 1), Some('"') | Some('#')) => {
                    let mut hashes = 0usize;
                    let mut j = i + 1;
                    while c.get(j) == Some(&'#') {
                        hashes += 1;
                        j += 1;
                    }
                    if c.get(j) == Some(&'"') {
                        self.raw_string_hashes = Some(hashes);
                        i = j + 1;
                    } else {
                        i += 1;
                    }
                }
                '"' => i = skip_string(&c, i + 1),
                // `'a` is a lifetime and `'}'` is a char literal; only the latter may hide a brace.
                '\'' => i = skip_char_literal(&c, i),
                '{' => {
                    depth += 1;
                    i += 1;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    i += 1;
                }
                _ => i += 1,
            }
        }
        depth
    }
}

/// Index just past the closing quote of a string that opened at `i - 1`.
fn skip_string(c: &[char], mut i: usize) -> usize {
    while i < c.len() {
        match c[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    c.len()
}

/// Index just past a char literal at `i`, or `i + 1` when the quote opens a lifetime.
fn skip_char_literal(c: &[char], i: usize) -> usize {
    if c.get(i + 1) == Some(&'\\') {
        let mut j = i + 2;
        while j < c.len() && c[j] != '\'' {
            j += 1;
        }
        return j + 1;
    }
    if c.get(i + 2) == Some(&'\'') {
        return i + 3;
    }
    i + 1
}

/// A line without its trailing `//` comment — a `//` inside a string literal is not one.
///
/// EVERY structural read in this module looks at the END of a line: an attribute has to close
/// with `]`, a `mod` declaration with `;`. A comment after either one silenced it — the gate
/// stopped being a gate and nothing said so. The follow-up review of PR #423 found this on a
/// declaration; it was the wider half that mattered, because `#[cfg(target_os = "macos")] // only
/// launchd lives here` is a line this codebase would write without a second thought, and it took
/// the gate with it.
fn strip_line_comment(line: &str) -> &str {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = |k: usize| chars.get(k).map(|(_, c)| *c);
    let mut k = 0usize;
    while k < chars.len() {
        match chars[k].1 {
            // A raw string ends only on the right number of hashes, and holds no escapes — so
            // counting quotes loses track of one that contains an odd number of them, and the
            // `//` after it would read as a comment that truncates the attribute.
            'r' if matches!(at(k + 1), Some('"') | Some('#')) => {
                let mut hashes = 0usize;
                let mut j = k + 1;
                while at(j) == Some('#') {
                    hashes += 1;
                    j += 1;
                }
                if at(j) != Some('"') {
                    k += 1;
                    continue;
                }
                j += 1;
                loop {
                    match at(j) {
                        None => return line.trim_end(),
                        Some('"') if (1..=hashes).all(|h| at(j + h) == Some('#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        _ => j += 1,
                    }
                }
                k = j;
            }
            '"' => {
                let mut j = k + 1;
                loop {
                    match at(j) {
                        None => return line.trim_end(),
                        Some('\\') => j += 2,
                        Some('"') => {
                            j += 1;
                            break;
                        }
                        _ => j += 1,
                    }
                }
                k = j;
            }
            '/' if at(k + 1) == Some('/') => return line[..chars[k].0].trim_end(),
            _ => k += 1,
        }
    }
    line.trim_end()
}

/// One attribute, joined across however many lines its brackets span.
fn read_attribute(lines: &[&str], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut depth = 0i32;
    let mut i = start;
    while i < lines.len() {
        // Stripped BEFORE the brackets are counted, so a `]` inside a comment cannot end an
        // attribute early either.
        let line = strip_line_comment(lines[i].trim());
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
        for c in line.chars() {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
        if depth <= 0 {
            break;
        }
    }
    (text, i)
}

/// The inside of a top-level `cfg(...)`, if this attribute is one. `cfg_attr` is deliberately not
/// one: it applies an attribute conditionally, it does not remove the item.
fn gate_predicate(attribute: &str) -> Option<String> {
    let body = attribute
        .strip_prefix("#![")
        .or_else(|| attribute.strip_prefix("#["))?
        .trim_end()
        .strip_suffix(']')?
        .trim();
    let (inner, after) = balanced_parens(body.strip_prefix("cfg")?)?;
    after.trim().is_empty().then(|| inner.trim().to_string())
}

/// The predicate of a `cfg(...)` that hides what it gates from every ubuntu job.
fn macos_gate(attribute: &str) -> Option<String> {
    gate_predicate(attribute).filter(|p| is_macos_only(p))
}

/// `#[test]`, `#[tokio::test]`, `#[serial_test::serial]`-shaped: the attribute PATH is `test` or
/// ends in `::test`.
fn is_test_attribute(attribute: &str) -> bool {
    let Some(body) = attribute
        .strip_prefix("#[")
        .and_then(|b| b.trim_end().strip_suffix(']'))
    else {
        return false;
    };
    let path = body.split('(').next().unwrap_or("").trim();
    path == "test" || path.ends_with("::test")
}

// ---- reading the lane out of the workflow ---------------------------------------------------

/// The packages `ci.yml`'s macOS lane runs, read from the job's own `cargo test` command.
///
/// Fail-closed on every way the lane could stop being one: a missing job, a `runs-on` this repo
/// cannot prove is a GitHub-hosted image (the org's own Mac holds signing material and must never
/// become the place tests run), or a job with no `cargo test` in it at all.
pub fn lane_packages(ci_yaml: &str) -> Result<BTreeSet<String>> {
    let doc: Value = serde_yaml::from_str(ci_yaml).context("parsing the CI workflow as YAML")?;
    let job = doc
        .get("jobs")
        .and_then(|j| j.get(LANE_JOB))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{CI_WORKFLOW} has no `{LANE_JOB}` job. That job IS the macOS lane — without it \
                 every `#[cfg(target_os = \"macos\")]` test in this repo runs in no CI lane at \
                 all (nxf 6j6v.fb1d). Restore it, or retire this gate deliberately."
            )
        })?;

    for label in runs_on_labels(job) {
        if !crate::runners::is_github_hosted_label(&label) {
            bail!(
                "the `{LANE_JOB}` job routes to `{label}`, which this repo cannot prove is a \
                 GitHub-hosted image. The macOS lane runs a whole test suite; the org's own Mac \
                 is not ephemeral and holds the Developer ID signing material (6j6v.px3d), so \
                 this lane is pinned to `macos-latest` on purpose — see `cargo xtask runners \
                 check` for the same boundary on the pull-request side."
            );
        }
    }

    let mut packages = BTreeSet::new();
    let mut saw_cargo_test = false;
    for step in job
        .get("steps")
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let Some(script) = step.get("run").and_then(Value::as_str) else {
            continue;
        };
        for command in shell_commands(script) {
            if !command.contains("cargo test") {
                continue;
            }
            saw_cargo_test = true;
            if let Some(flag) = narrowing_flag(&command) {
                bail!(
                    "the `{LANE_JOB}` job's `cargo test` carries `{flag}`, which narrows WHAT it \
                     runs. The package list is only half of what decides coverage: `--lib` keeps \
                     every `-p` and stops running the `tests/` targets, and a libtest filter \
                     silences tests by name — either one leaves this gate reporting full coverage \
                     for a lane that no longer has it, which is the exact failure it exists to \
                     prevent (nxf 6j6v.fb1d). Widen the lane, or teach this rule why the \
                     narrowing is safe."
                );
            }
            packages.extend(package_flags(&command));
        }
    }
    if !saw_cargo_test {
        bail!(
            "the `{LANE_JOB}` job runs no `cargo test`. The lane is defined by that command — a \
             job that only builds proves nothing about the tests this gate exists to place."
        );
    }
    Ok(packages)
}

/// `runs-on:` as a list of labels, whether it was written as a scalar or a sequence. Anything
/// else (a mapping with a runner `group:`, say) yields one unprovable label on purpose.
fn runs_on_labels(job: &Value) -> Vec<String> {
    match job.get("runs-on") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Sequence(items)) => items
            .iter()
            .map(|v| v.as_str().unwrap_or("<not a label>").to_string())
            .collect(),
        Some(_) => vec!["<not a label list>".to_string()],
        None => vec!["<no runs-on>".to_string()],
    }
}

/// A `run:` script split into logical commands: newlines end one, a trailing `\` does not.
fn shell_commands(script: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in script.lines() {
        let line = line.trim();
        if let Some(head) = line.strip_suffix('\\') {
            current.push_str(head);
            current.push(' ');
            continue;
        }
        current.push_str(line);
        out.push(std::mem::take(&mut current));
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Flags that narrow WHICH targets `cargo test` builds. Each one keeps the whole `-p` list while
/// dropping part of what runs — `--lib` is the sharpest: it would leave every package "covered"
/// here while `crates/nxs/tests/service_program.rs`, the costliest gated file in the tree, stopped
/// being compiled at all.
const NARROWING_TARGET_FLAGS: &[&str] = &[
    "--lib",
    "--bin",
    "--bins",
    "--test",
    "--tests",
    "--example",
    "--examples",
    "--bench",
    "--benches",
    "--doc",
    "--exclude",
];

/// The libtest arguments that cannot narrow what runs. Everything else after a bare `--` — a
/// `--skip`, an `--ignored`, or a bare word, which libtest reads as a name filter — does.
const HARMLESS_LIBTEST_ARGS: &[&str] = &[
    "--nocapture",
    "--show-output",
    "--include-ignored",
    "--test-threads",
    "--color",
    "--format",
];

/// The first flag in `command` that narrows what the lane runs, if any.
fn narrowing_flag(command: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut after_separator = false;
    for (i, token) in tokens.iter().enumerate() {
        let bare = token.split('=').next().unwrap_or(token);
        if !after_separator {
            if *token == "--" {
                after_separator = true;
                continue;
            }
            if NARROWING_TARGET_FLAGS.contains(&bare) {
                return Some((*token).to_string());
            }
            // A bare word where a package name is not expected is a libtest filter written
            // without the separator (`cargo test -p nxs service_program`).
            let follows_value_flag = i > 0 && matches!(tokens[i - 1], "-p" | "--package");
            if i > 1 && !token.starts_with('-') && !follows_value_flag && tokens[1] == "test" {
                return Some(format!("the test-name filter `{token}`"));
            }
            continue;
        }
        if token.starts_with('-') {
            if !HARMLESS_LIBTEST_ARGS.contains(&bare) {
                return Some((*token).to_string());
            }
        } else if !matches!(
            tokens
                .get(i.wrapping_sub(1))
                .map(|t| t.split('=').next().unwrap_or(t)),
            Some("--test-threads") | Some("--color") | Some("--format")
        ) {
            return Some(format!("the test-name filter `{token}`"));
        }
    }
    None
}

/// Every `-p <pkg>` / `--package <pkg>` (and the `=` spelling) in one command line.
fn package_flags(command: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let tokens: Vec<&str> = command.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        if let Some(name) = token.strip_prefix("--package=") {
            out.insert(name.to_string());
        } else if matches!(*token, "-p" | "--package") {
            if let Some(name) = tokens.get(i + 1) {
                out.insert((*name).to_string());
            }
        }
    }
    out
}

// ---- the verdict ----------------------------------------------------------------------------

/// One macOS-gated test site, with the package it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSite {
    pub package: String,
    /// Repo-relative path.
    pub path: String,
    pub line: usize,
    pub predicate: String,
}

/// Fail when any package holding a macOS-gated test is missing from the lane.
pub fn verdict(sites: &[TestSite], lane: &BTreeSet<String>) -> Result<()> {
    let mut uncovered: BTreeMap<&str, Vec<&TestSite>> = BTreeMap::new();
    for site in sites {
        if !lane.contains(&site.package) {
            uncovered
                .entry(site.package.as_str())
                .or_default()
                .push(site);
        }
    }
    if uncovered.is_empty() {
        return Ok(());
    }
    let mut report = String::from(
        "these tests are compiled only on macOS and live in a package the macOS lane does not \
         run, so no CI job executes them:\n",
    );
    for (package, sites) in &uncovered {
        report.push_str(&format!("\n  package `{package}`:\n"));
        for site in sites {
            report.push_str(&format!(
                "    {}:{}  cfg({})\n",
                site.path, site.line, site.predicate
            ));
        }
    }
    report.push_str(&format!(
        "\nTwo ways out, and they are not equal. Either add the package to the `{LANE_JOB}` job's \
         `cargo test` in {CI_WORKFLOW} — it bills at 10x while this repo is private, so it is a \
         real decision — or make the test platform-independent by injecting the platform instead \
         of gating on it, which is what most of these need (nxf 6j6v.fb1d)."
    ));
    bail!(report)
}

/// Every macOS-gated test site under `<root>/crates`.
pub fn scan(root: &Path) -> Result<Vec<TestSite>> {
    let crates = root.join(CRATES_DIR);
    let mut entries: Vec<_> = std::fs::read_dir(&crates)
        .with_context(|| format!("reading {}", crates.display()))?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path())
        .collect();
    entries.sort();

    let mut sites = Vec::new();
    for dir in entries {
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let package = package_name(&manifest)?;

        // `tests/`: every line is test code, so each file answers for itself.
        for file in rust_files(&dir.join("tests"))? {
            let src = read_source(&file)?;
            for gate in sites_in_source(&src, true) {
                sites.push(TestSite {
                    package: package.clone(),
                    path: repo_relative(root, &file),
                    line: gate.line,
                    predicate: gate.predicate,
                });
            }
        }

        // `src/`: a file answers for the gates written IN it — and for the one that may have
        // arrived from a parent's `mod` declaration, which lives in a different file entirely.
        let mut sources = BTreeMap::new();
        for file in rust_files(&dir.join("src"))? {
            sources.insert(file.clone(), read_source(&file)?);
        }
        for (file, src) in &sources {
            for gate in sites_in_source(src, false) {
                sites.push(TestSite {
                    package: package.clone(),
                    path: repo_relative(root, file),
                    line: gate.line,
                    predicate: gate.predicate,
                });
            }
        }
        for (file, origin) in inherited_gates(&sources) {
            if source_declares_a_test(&sources[&file]) {
                sites.push(TestSite {
                    package: package.clone(),
                    path: repo_relative(root, &origin.path),
                    line: origin.line,
                    predicate: origin.predicate,
                });
            }
        }
    }
    Ok(sites)
}

/// Where a gate that reaches a file from somewhere else was actually written. A site must point
/// HERE, not at the file it silenced — the declaration is what a fixer has to edit.
#[derive(Clone, Debug)]
struct GateOrigin {
    path: std::path::PathBuf,
    line: usize,
    predicate: String,
}

/// Which `src/` files are macOS-only because a `mod` DECLARATION says so — the fifth blind spot
/// the follow-up review of PR #423 found, and the one the scope stack structurally cannot see:
/// `#[cfg(target_os = "macos")] mod mac_tests;` opens no block, and `mac_tests.rs` read on its own
/// carries no gate at all. Both halves look ordinary while every test in the second one is gone.
///
/// This tree already gates six declarations exactly this way (`#[cfg(feature = "cli")] pub mod
/// cli;` and five siblings), so it is the shape this codebase reaches for — with a different
/// predicate, so far.
///
/// It propagates: a module declared BY a macOS-only file is macOS-only too, whatever its own
/// declaration says. First gate to reach a file wins, which keeps the walk finite.
fn inherited_gates(
    sources: &BTreeMap<std::path::PathBuf, String>,
) -> BTreeMap<std::path::PathBuf, GateOrigin> {
    let mut queue: std::collections::VecDeque<(std::path::PathBuf, GateOrigin)> =
        std::collections::VecDeque::new();
    for (file, src) in sources {
        for declaration in module_declarations(src) {
            let Some(predicate) = declaration.gate else {
                continue;
            };
            let origin = GateOrigin {
                path: file.clone(),
                line: declaration.line,
                predicate,
            };
            for child in child_module_paths(file, &declaration.name) {
                if sources.contains_key(&child) {
                    queue.push_back((child, origin.clone()));
                }
            }
        }
    }

    let mut reached: BTreeMap<std::path::PathBuf, GateOrigin> = BTreeMap::new();
    while let Some((file, origin)) = queue.pop_front() {
        if reached.contains_key(&file) {
            continue;
        }
        reached.insert(file.clone(), origin.clone());
        let Some(src) = sources.get(&file) else {
            continue;
        };
        for declaration in module_declarations(src) {
            for child in child_module_paths(&file, &declaration.name) {
                if sources.contains_key(&child) && !reached.contains_key(&child) {
                    queue.push_back((child, origin.clone()));
                }
            }
        }
    }
    reached
}

/// A `mod <name>;` declaration and the macOS gate on it, if any.
struct ModuleDeclaration {
    name: String,
    /// The gate's line when there is one, else the declaration's own.
    line: usize,
    gate: Option<String>,
}

fn module_declarations(src: &str) -> Vec<ModuleDeclaration> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut cluster: Vec<(usize, String)> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let head = lines[i].trim_start();
        if head.is_empty() || head.starts_with("//") {
            i += 1;
            continue;
        }
        if head.starts_with("#[") || head.starts_with("#![") {
            let (text, next) = read_attribute(&lines, i);
            if head.starts_with("#[") {
                cluster.push((i + 1, text));
            }
            i = next;
            continue;
        }
        if let Some(name) = module_declaration_name(head) {
            let gate = cluster
                .iter()
                .find_map(|(line, a)| macos_gate(a).map(|p| (*line, p)));
            out.push(ModuleDeclaration {
                name,
                line: gate.as_ref().map_or(i + 1, |(line, _)| *line),
                gate: gate.map(|(_, predicate)| predicate),
            });
        }
        cluster.clear();
        i += 1;
    }
    out
}

/// `mod foo;` — a declaration, not a block. `mod foo {` is the scope stack's business.
fn module_declaration_name(item_head: &str) -> Option<String> {
    let name = strip_line_comment(item_head)
        .trim_start_matches("pub(crate) ")
        .trim_start_matches("pub(super) ")
        .trim_start_matches("pub ")
        .strip_prefix("mod ")?
        .trim_end()
        .strip_suffix(';')?
        .trim();
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| name.to_string())
}

/// The two files `mod <name>;` in `file` can name, per the 2018 module layout.
fn child_module_paths(file: &Path, name: &str) -> Vec<std::path::PathBuf> {
    let Some(parent) = file.parent() else {
        return Vec::new();
    };
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let dir = match stem {
        "lib" | "main" | "mod" => parent.to_path_buf(),
        _ => parent.join(stem),
    };
    vec![
        dir.join(format!("{name}.rs")),
        dir.join(name).join("mod.rs"),
    ]
}

/// Does this file hold a test at all? A macOS-only module that holds none is not a test site.
fn source_declares_a_test(src: &str) -> bool {
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[") {
            let (text, next) = read_attribute(&lines, i);
            if is_test_attribute(&text) {
                return true;
            }
            i = next;
            continue;
        }
        i += 1;
    }
    false
}

fn read_source(file: &Path) -> Result<String> {
    std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))
}

fn repo_relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// `[package] name` out of a manifest.
fn package_name(manifest: &Path) -> Result<String> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", manifest.display()))?;
    doc["package"]["name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("{} has no [package] name", manifest.display()))
}

/// Every `.rs` file under `dir`, recursively. A missing directory is not an error — plenty of
/// packages have no `tests/`.
fn rust_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in
            std::fs::read_dir(&current).with_context(|| format!("reading {}", current.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Run the gate, and report WHAT it found — not just that it was happy.
///
/// The number is the point of the return value: a scanner that quietly stopped reading looks
/// exactly like a clean tree from the outside, and this gate exists precisely because a green
/// check that covers nothing is the failure mode nobody notices. `the_real_tree_is_covered…`
/// pins it from the test side; this pins it in the CI log, where a human might see it drop.
pub fn check(root: &Path) -> Result<Inventory> {
    let ci = std::fs::read_to_string(root.join(CI_WORKFLOW))
        .with_context(|| format!("reading {CI_WORKFLOW}"))?;
    let lane = lane_packages(&ci)?;
    let sites = scan(root)?;
    verdict(&sites, &lane)?;
    Ok(Inventory {
        sites: sites.len(),
        packages: sites
            .iter()
            .map(|s| s.package.clone())
            .collect::<BTreeSet<_>>(),
    })
}

/// What the gate saw, for the one line it prints.
pub struct Inventory {
    pub sites: usize,
    pub packages: BTreeSet<String>,
}

impl std::fmt::Display for Inventory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} macOS-gated test site{} in {}, every one inside the lane",
            self.sites,
            if self.sites == 1 { "" } else { "s" },
            if self.packages.is_empty() {
                "no package".to_string()
            } else {
                self.packages.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        )
    }
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

    // ---- the predicate evaluator ------------------------------------------------------------

    /// The one fact everything else rests on: the same predicate answers differently per target.
    #[test]
    fn a_target_os_equality_is_read_per_target() {
        assert_eq!(eval(r#"target_os = "macos""#, "macos"), Truth::True);
        assert_eq!(eval(r#"target_os = "macos""#, "linux"), Truth::False);
        assert_eq!(eval(r#"target_os = "linux""#, "linux"), Truth::True);
    }

    /// `not`/`all`/`any` compose, and `unix` is true on both targets the lane compares.
    #[test]
    fn the_connectives_compose_and_unix_holds_on_both_targets() {
        assert_eq!(eval(r#"not(target_os = "macos")"#, "macos"), Truth::False);
        assert_eq!(eval(r#"not(target_os = "macos")"#, "linux"), Truth::True);
        assert_eq!(
            eval(r#"any(target_os = "macos", target_os = "linux")"#, "linux"),
            Truth::True
        );
        assert_eq!(
            eval(r#"all(unix, target_os = "macos")"#, "macos"),
            Truth::True
        );
        assert_eq!(eval("unix", "linux"), Truth::True);
        assert_eq!(eval("windows", "macos"), Truth::False);
    }

    /// A predicate this module cannot resolve is `Unknown` — and `Unknown` must never be mistaken
    /// for "does not gate anything", which is the direction that would silently drop a test.
    #[test]
    fn an_unresolvable_predicate_is_unknown_and_still_counts_as_macos_only_when_paired() {
        assert_eq!(eval(r#"feature = "cli""#, "macos"), Truth::Unknown);
        assert!(is_macos_only(
            r#"all(target_os = "macos", feature = "cli")"#
        ));
    }

    /// The whole reason a grep would not do: every gated test in this tree has a twin that DOES
    /// run on ubuntu, and both spell `target_os = "macos"`.
    #[test]
    fn only_a_gate_that_hides_a_test_from_ubuntu_counts() {
        assert!(is_macos_only(r#"target_os = "macos""#));
        assert!(!is_macos_only(r#"not(target_os = "macos")"#));
        assert!(!is_macos_only(
            r#"any(target_os = "macos", target_os = "linux")"#
        ));
        assert!(!is_macos_only("unix"));
        // Windows-only code is invisible to the macOS lane too — but adding it there would not
        // make it visible, so it is not this gate's business.
        assert!(!is_macos_only("windows"));
    }

    // ---- reading one file -------------------------------------------------------------------

    /// The costliest shape in the tree: a whole test target switched off with one inner attribute.
    #[test]
    fn a_test_target_gated_at_module_level_is_one_site() {
        let src = "//! doc\n#![cfg(target_os = \"macos\")]\n\n#[test]\nfn t() {}\n";
        let sites = sites_in_source(src, true);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].line, 2);
        assert_eq!(sites[0].predicate, r#"target_os = "macos""#);
    }

    /// The in-source shape (`crates/chat/src/timer.rs`): the gate sits beside `#[test]`.
    #[test]
    fn a_gate_beside_a_test_attribute_in_src_is_a_site() {
        let src = "    #[test]\n    #[cfg(target_os = \"macos\")]\n    fn t() {}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].line, 2);
    }

    /// …and the same gate on PRODUCTION code is not this lane's business: `launchd.rs` is
    /// macOS-only in its entirety and `platform-gates-macos` already compiles and lints it.
    #[test]
    fn a_gate_on_production_code_in_src_is_not_a_test_site() {
        let src = "#[cfg(target_os = \"macos\")]\npub fn plist_path() -> PathBuf { todo!() }\n";
        assert!(sites_in_source(src, false).is_empty());
    }

    /// A `#[cfg(test)]` module gated as a whole — the `#[test]` inside is not adjacent to the
    /// gate, so the module head is what has to be read.
    #[test]
    fn a_module_gated_by_separate_cfg_test_and_target_os_is_a_site() {
        let src = "#[cfg(test)]\n#[cfg(target_os = \"macos\")]\nmod mac_tests {\n    #[test]\n    fn t() {}\n}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].line, 2);
    }

    // ---- the four blind spots the independent review of PR #423 found ----------------------
    //
    // Each one gates a test DIRECTLY, so none was covered by this module's own "deliberately not
    // modelled" note (which excuses only INDIRECT reach). Each was a real false negative: the
    // gate went green while the hole it exists to close stood open. They are the reason the
    // scanner now carries a notion of ENCLOSING SCOPE instead of reading one attribute cluster at
    // a time.

    /// Blind spot 1. The costliest shape this module's own docs praise handling — a whole file
    /// switched off by one inner attribute — was read only under `tests/`. A `src/` file can be
    /// macOS-only in exactly the same way, and every `#[test]` in it disappears with it.
    #[test]
    fn a_src_file_gated_at_module_level_carries_its_tests_with_it() {
        let src = "//! doc\n#![cfg(target_os = \"macos\")]\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].line, 2);
    }

    /// Blind spot 2. `#[cfg(all(test, target_os = "macos"))]` is one attribute, not two, and the
    /// old reader compared the predicate text to the literal string `"test"` — which is precisely
    /// the "match instead of evaluate" this module's own design note forbids.
    #[test]
    fn a_module_gated_by_one_combined_cfg_test_and_target_os_is_a_site() {
        let src = "#[cfg(all(test, target_os = \"macos\"))]\nmod mac_tests {\n    #[test]\n    fn t() {}\n}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].line, 1);
    }

    /// Blind spot 3. A module gated by `target_os` ALONE — no `cfg(test)` anywhere — still holds
    /// macOS-only tests. The old reader required `cfg(test)` beside the gate to believe it.
    #[test]
    fn a_module_gated_by_target_os_alone_is_a_site_through_the_tests_it_holds() {
        let src = "#[cfg(target_os = \"macos\")]\nmod mac_tests {\n    #[test]\n    fn t() {}\n}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].line, 1);
    }

    /// …and nested, which is how it would really be written: the macOS module sits inside the
    /// crate's ordinary `#[cfg(test)] mod tests`. Scope has to be carried DOWN, not looked for
    /// in one cluster.
    #[test]
    fn a_gated_module_nested_inside_the_ordinary_test_module_is_a_site() {
        let src = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn everywhere() {}\n\n    #[cfg(target_os = \"macos\")]\n    mod mac {\n        #[test]\n        fn only_here() {}\n    }\n}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(
            sites[0].line, 6,
            "the gate, not the test, is what to point at"
        );
    }

    /// The other side of carrying scope down: a macOS-gated PRODUCTION function opens a block
    /// too, and the ordinary tests that follow it are NOT inside that block. Getting this wrong
    /// would demand the 10x lane for every crate that owns a platform pair — `nxs-foundation`
    /// below is the real instance.
    #[test]
    fn a_gated_production_fn_does_not_swallow_the_tests_that_follow_it() {
        let src = "#[cfg(target_os = \"macos\")]\npub fn plist_path() -> PathBuf {\n    todo!()\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n";
        assert!(
            sites_in_source(src, false).is_empty(),
            "{:?}",
            sites_in_source(src, false)
        );
    }

    /// Braces inside strings, chars and comments must not move the scope depth — otherwise a
    /// gated block appears to close early (or never), and the two tests above stop meaning
    /// anything.
    #[test]
    fn braces_in_strings_and_comments_do_not_move_the_scope() {
        let src = "#[cfg(target_os = \"macos\")]\nfn noisy() {\n    let _ = \"{\";\n    let _ = '}';\n    // }\n    /* { */\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n";
        assert!(
            sites_in_source(src, false).is_empty(),
            "{:?}",
            sites_in_source(src, false)
        );
    }

    /// Blind spot 4. The lane is `cargo test -p …`, and the package list is only half of what
    /// decides coverage: `--lib` would keep every package name and stop running the `tests/`
    /// integration targets — exactly where the module-gated files live. A narrowed lane must
    /// fail, not report full coverage.
    #[test]
    fn a_lane_narrowed_to_one_target_kind_fails_instead_of_claiming_coverage() {
        let narrowed = LANE.replace(
            "cargo test -p nxs -p nexus-chat -p nxs-service",
            "cargo test --lib -p nxs -p nexus-chat -p nxs-service",
        );
        let err = lane_packages(&narrowed).unwrap_err().to_string();
        assert!(err.contains("--lib"), "{err}");
    }

    /// Same class, the other spelling: a libtest filter after `--` silences tests by name while
    /// every package stays listed.
    #[test]
    fn a_lane_carrying_a_test_name_filter_fails_too() {
        let filtered = LANE.replace(
            "cargo test -p nxs -p nexus-chat -p nxs-service",
            "cargo test -p nxs -p nexus-chat -p nxs-service -- --skip service_program",
        );
        let err = lane_packages(&filtered).unwrap_err().to_string();
        assert!(err.contains("--skip") || err.contains("filter"), "{err}");
    }

    /// …but the flags the lane legitimately carries must still pass, or the rule above would be
    /// a rule against writing the lane at all.
    #[test]
    fn the_flags_the_real_lane_carries_are_not_narrowing() {
        let real = std::fs::read_to_string(repo_root().join(CI_WORKFLOW)).unwrap();
        assert!(real.contains("--no-fail-fast"), "the lane still carries it");
        lane_packages(&real).expect("the real lane is not narrowed");
    }

    /// The false-positive direction, against real code: `nxs-foundation` owns a macOS-only `cfg`
    /// (`src/store.rs`'s `O_NOFOLLOW` constant) AND an ordinary `#[cfg(test)] mod tests`. It has
    /// no macOS-only TEST, so demanding it in the 10x lane would be a bill for nothing.
    #[test]
    fn a_crate_with_macos_production_code_and_ordinary_tests_is_not_reported() {
        let sites = scan(&repo_root()).unwrap();
        let packages: BTreeSet<&str> = sites.iter().map(|s| s.package.as_str()).collect();
        assert!(
            !packages.contains("nxs-foundation"),
            "scope tracking has drifted — foundation has macOS production code but no macOS \
             test: {sites:#?}"
        );
    }

    /// The twin that DOES run on ubuntu must not be reported — otherwise the gate would demand
    /// the macOS lane for every crate that owns a platform pair.
    #[test]
    fn the_ubuntu_twin_of_a_gated_test_is_not_a_site() {
        let src = "#[test]\n#[cfg(not(target_os = \"macos\"))]\nfn t() {}\n";
        assert!(sites_in_source(src, true).is_empty());
    }

    /// Prose about the gate is not the gate. A comment naming it — this module's own header does
    /// exactly that — must not redden anything.
    #[test]
    fn a_gate_named_in_a_comment_is_not_a_site() {
        let src = "// a #[cfg(target_os = \"macos\")] test runs in no CI lane\n/// #[cfg(target_os = \"macos\")]\nfn t() {}\n";
        assert!(sites_in_source(src, true).is_empty());
    }

    // ---- a trailing comment must not blind any of it -----------------------------------------

    /// The follow-up review found this on a `mod` declaration. It is a CLASS, not a spot: every
    /// structural read in this module looked at the end of a line, so a `// why` at the end of an
    /// ATTRIBUTE silenced the gate itself — the worse half, and on a codebase whose every other
    /// line explains itself.
    #[test]
    fn a_trailing_comment_on_the_gate_itself_does_not_hide_it() {
        let src = "    #[test]\n    #[cfg(target_os = \"macos\")] // launchd only exists here\n    fn t() {}\n";
        let sites = sites_in_source(src, false);
        assert_eq!(sites.len(), 1, "{sites:?}");
    }

    /// …and on the declaration, which is where it was found.
    #[test]
    fn a_trailing_comment_on_a_gated_declaration_does_not_hide_it() {
        let dir = package_fixture(
            "lonely",
            &[
                (
                    "lib.rs",
                    "#[cfg(target_os = \"macos\")]\nmod mac_only_tests; // the launchd shim\n",
                ),
                ("mac_only_tests.rs", "#[test]\nfn t() {}\n"),
            ],
            &["somebody-else"],
        );
        assert_eq!(scan(dir.path()).unwrap().len(), 1);
    }

    /// The last residual the review named and chose not to escalate: a RAW string holds an odd
    /// number of quotes, so counting quotes loses track and the `//` after it reads as a comment.
    /// The attribute is then truncated before its `]`, `read_attribute` never closes it, and it
    /// swallows the gate on the next line whole. Remote — this tree has no raw string in any
    /// attribute — but it is the same class as the three rounds before it, in a function written
    /// this session, so it is closed rather than written down.
    #[test]
    fn a_raw_string_with_an_odd_quote_does_not_swallow_the_gate_below_it() {
        let src = "    #[doc = r#\"the \"why // here\"#]\n    #[test]\n    #[cfg(target_os = \"macos\")]\n    fn t() {}\n";
        assert_eq!(
            sites_in_source(src, false).len(),
            1,
            "{:?}",
            sites_in_source(src, false)
        );
    }

    /// The over-correction that would follow from stripping naively: a `//` inside a string is not
    /// a comment, and cutting there would truncate the attribute and lose the gate a second way.
    #[test]
    fn a_double_slash_inside_a_string_is_not_a_comment() {
        let src = "    #[doc = \"see https://example.com for why\"]\n    #[test]\n    #[cfg(target_os = \"macos\")]\n    fn t() {}\n";
        assert_eq!(sites_in_source(src, false).len(), 1);
    }

    // ---- blind spot 5: the gate is on a module DECLARATION, its tests are in another file ----

    /// A throwaway package: `crates/<pkg>/` with a manifest and the given `src/` files, plus a
    /// `ci.yml` whose lane runs `lane_runs`.
    fn package_fixture(pkg: &str, files: &[(&str, &str)], lane_runs: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(format!("crates/{pkg}/src"))).unwrap();
        std::fs::write(
            root.join(format!("crates/{pkg}/Cargo.toml")),
            format!("[package]\nname = \"{pkg}\"\nversion = \"0.0.0\"\n"),
        )
        .unwrap();
        for (rel, body) in files {
            let path = root.join(format!("crates/{pkg}/src/{rel}"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        let lane = lane_runs
            .iter()
            .map(|p| format!("-p {p}"))
            .collect::<Vec<_>>()
            .join(" ");
        std::fs::write(
            root.join(CI_WORKFLOW),
            format!(
                "jobs:\n  {LANE_JOB}:\n    runs-on: macos-latest\n    steps:\n      - run: cargo test {lane}\n"
            ),
        )
        .unwrap();
        dir
    }

    /// Blind spot 5, found by the follow-up review of PR #423. `#[cfg(target_os = "macos")] mod
    /// foo;` opens NO block, so carrying a gate down into a block cannot see it — and `foo.rs`,
    /// read on its own, carries no gate at all. Both halves look ordinary; the tests are gone.
    ///
    /// Not theoretical here: this tree already gates six module DECLARATIONS this way
    /// (`#[cfg(feature = "cli")] pub mod cli;` in `crates/chat/src/lib.rs` and five siblings), so
    /// it is the shape this codebase reaches for, just with a different predicate.
    #[test]
    fn a_gate_on_a_module_declaration_reaches_the_file_it_names() {
        let dir = package_fixture(
            "lonely",
            &[
                (
                    "lib.rs",
                    "#[cfg(target_os = \"macos\")]\nmod mac_only_tests;\n",
                ),
                ("mac_only_tests.rs", "#[test]\nfn t() {}\n"),
            ],
            &["somebody-else"],
        );
        let sites = scan(dir.path()).unwrap();
        assert_eq!(sites.len(), 1, "{sites:#?}");
        assert_eq!(sites[0].package, "lonely");
        assert!(
            sites[0].path.ends_with("lib.rs") && sites[0].line == 1,
            "the site must point at the DECLARATION, which is where the fix goes: {:?}",
            sites[0]
        );
        assert!(
            check(dir.path()).is_err(),
            "and the lane does not run `lonely`"
        );
    }

    /// The `foo/mod.rs` spelling of the same declaration.
    #[test]
    fn a_gated_declaration_also_reaches_a_mod_rs_directory_module() {
        let dir = package_fixture(
            "lonely",
            &[
                ("lib.rs", "#[cfg(target_os = \"macos\")]\nmod mac;\n"),
                ("mac/mod.rs", "#[test]\nfn t() {}\n"),
            ],
            &["somebody-else"],
        );
        assert_eq!(scan(dir.path()).unwrap().len(), 1);
    }

    /// …and it does not stop at the first hop: a module declared BY a gated file is gated too,
    /// even though its own declaration says nothing.
    #[test]
    fn a_gate_on_a_declaration_carries_through_to_the_modules_below_it() {
        let dir = package_fixture(
            "lonely",
            &[
                ("lib.rs", "#[cfg(target_os = \"macos\")]\nmod mac;\n"),
                ("mac.rs", "mod deeper;\n"),
                ("mac/deeper.rs", "#[test]\nfn t() {}\n"),
            ],
            &["somebody-else"],
        );
        let sites = scan(dir.path()).unwrap();
        assert_eq!(sites.len(), 1, "{sites:#?}");
        assert!(sites[0].path.ends_with("lib.rs"), "{:?}", sites[0]);
    }

    /// The false-positive direction again, and it is the one that costs money: an UNGATED
    /// declaration must reach nothing, or every crate with a `mod tests;` would demand the lane.
    #[test]
    fn an_ungated_declaration_reaches_nothing() {
        let dir = package_fixture(
            "lonely",
            &[
                (
                    "lib.rs",
                    "mod ordinary;\n#[cfg(feature = \"x\")]\nmod featured;\n",
                ),
                ("ordinary.rs", "#[test]\nfn t() {}\n"),
                ("featured.rs", "#[test]\nfn t() {}\n"),
            ],
            &["somebody-else"],
        );
        assert!(scan(dir.path()).unwrap().is_empty());
    }

    /// A gated module with no test in it is not a test site — the gate has to reach a TEST.
    #[test]
    fn a_gated_declaration_of_a_module_holding_no_test_is_not_a_site() {
        let dir = package_fixture(
            "lonely",
            &[
                ("lib.rs", "#[cfg(target_os = \"macos\")]\nmod launchd;\n"),
                ("launchd.rs", "pub fn install() {}\n"),
            ],
            &["somebody-else"],
        );
        assert!(scan(dir.path()).unwrap().is_empty());
    }

    // ---- reading the lane -------------------------------------------------------------------

    const LANE: &str = r#"
jobs:
  macos-tests:
    runs-on: macos-latest
    steps:
      - run: cargo build -p nxs
      - name: Tests
        run: |
          cargo test -p nxs -p nexus-chat -p nxs-service
"#;

    #[test]
    fn the_lane_packages_come_out_of_the_jobs_own_cargo_test_command() {
        let got = lane_packages(LANE).unwrap();
        assert_eq!(
            got,
            ["nexus-chat", "nxs", "nxs-service"]
                .into_iter()
                .map(String::from)
                .collect::<BTreeSet<_>>()
        );
    }

    /// `cargo build -p nxs` is a build step, not the lane — reading it as one would let the lane
    /// claim a package it never tests.
    #[test]
    fn only_cargo_test_defines_the_lane() {
        let only_build = "jobs:\n  macos-tests:\n    runs-on: macos-latest\n    steps:\n      - run: cargo build -p nxs\n";
        let err = lane_packages(only_build).unwrap_err().to_string();
        assert!(err.contains("cargo test"), "{err}");
    }

    /// Fail-closed: no job, no lane, and the check must say so rather than pass on an empty set.
    #[test]
    fn a_missing_lane_fails_closed() {
        let err = lane_packages("jobs:\n  quality-gates:\n    runs-on: ubuntu-latest\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains(LANE_JOB), "{err}");
    }

    /// THE SECURITY HALF. The org's own Mac holds the Developer ID signing material; a lane that
    /// runs the whole test suite there is a far bigger surface than the clippy pass that runs
    /// there today. `runs-on` must be a label this repo can prove is a GitHub-hosted image.
    #[test]
    fn a_lane_routed_off_a_github_hosted_image_fails() {
        let self_hosted = LANE.replace("runs-on: macos-latest", "runs-on: [self-hosted, macOS]");
        let err = lane_packages(&self_hosted).unwrap_err().to_string();
        assert!(err.contains("hosted"), "{err}");
    }

    // ---- the verdict ------------------------------------------------------------------------

    fn site(package: &str) -> TestSite {
        TestSite {
            package: package.to_string(),
            path: format!("crates/{package}/tests/mac.rs"),
            line: 1,
            predicate: r#"target_os = "macos""#.to_string(),
        }
    }

    #[test]
    fn a_package_the_lane_runs_is_covered() {
        let lane = ["nxs".to_string()].into_iter().collect();
        assert!(verdict(&[site("nxs")], &lane).is_ok());
    }

    /// The whole point: a macOS-gated test in a package nobody runs on macOS is the hole.
    #[test]
    fn a_macos_test_outside_the_lane_is_named_and_fails() {
        let lane = ["nxs".to_string()].into_iter().collect();
        let err = verdict(&[site("nexus-chat")], &lane)
            .unwrap_err()
            .to_string();
        assert!(err.contains("nexus-chat"), "{err}");
        assert!(err.contains("crates/nexus-chat/tests/mac.rs"), "{err}");
    }

    // ---- against the real tree --------------------------------------------------------------

    /// The gate is green on the repository it guards — and BOTH of the scanner's paths really
    /// found something there, so a scanner that quietly reads nothing cannot pass for a clean
    /// tree. (Sites, not tests: one module-level gate switches off nine tests at once.)
    #[test]
    fn the_real_tree_is_covered_and_the_scanner_really_read_it() {
        let root = repo_root();
        let sites = scan(&root).unwrap();
        // A whole test target switched off by one inner attribute — the costliest shape…
        assert!(
            sites
                .iter()
                .any(|s| s.path.ends_with("crates/nxs/tests/service_program.rs")),
            "{sites:#?}"
        );
        // …and a single test gated beside its own `#[test]`, inside a `src` file.
        assert!(
            sites
                .iter()
                .any(|s| s.path.ends_with("crates/chat/src/timer.rs")),
            "{sites:#?}"
        );
        let inventory = check(&root).unwrap();
        assert!(inventory.sites >= 7, "{inventory}");
    }

    /// The counter-proof the ticket's DoD asks for, in the cheap direction: drop a package from
    /// the lane in a copy of the REAL workflow and the gate must name what that un-covers.
    #[test]
    fn dropping_a_package_from_the_real_lane_reddens_the_gate() {
        let root = repo_root();
        let ci = std::fs::read_to_string(root.join(CI_WORKFLOW)).unwrap();
        let narrowed = ci.replace(" -p nexus-chat", "");
        assert_ne!(narrowed, ci, "the lane no longer names -p nexus-chat");
        let lane = lane_packages(&narrowed).unwrap();
        let sites = scan(&root).unwrap();
        let err = verdict(&sites, &lane).unwrap_err().to_string();
        assert!(err.contains("nexus-chat"), "{err}");
    }
}

//! `cargo xtask layering check`: the fail-closed gate of the nxc rebuild (spec 6j6v.zjyv §11). The new
//! engine is built parallel to the old one and nothing new imports the old, so the crates' edges are
//! a rule here, read from `cargo metadata`, and not a convention:
//!
//! - every `nexus-run-*` crate has exactly the workspace and external dependencies its row allows;
//! - a new crate is imported only by the crates its row names;
//! - `nexus-run-host`'s `test-support` feature is reachable from dev-dependencies only, and no other
//!   new crate declares a feature;
//! - every source file under `crates/run-*/src`, and this file, holds at most 700 lines and no inline
//!   test module;
//! - `nexus-run-engine`'s sources touch no clock, environment, disk, thread or randomness, and neither
//!   they nor the contract types hold a collection whose iteration order differs per process.
//!
//! A dependency's kind does not matter for the edge rules: a dev-dependency on the old engine would
//! still be the old engine under a new crate's tests. It stays a metadata and source lint, so it needs
//! no engine build. The determinism of `decide` itself is proved by its own step in the `lint` job,
//! `.github/scripts/run-engine-determinism.sh`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const GATE_FILE: &str = "xtask/src/layering.rs";
#[cfg(test)]
const DETERMINISM_SCRIPT: &str = ".github/scripts/run-engine-determinism.sh";

const MAX_LINES: usize = 700;
const RUN_PREFIX: &str = "nexus-run-";
const CONTRACTS: &str = "nexus-run-contracts";
const HOST: &str = "nexus-run-host";
const TEST_SUPPORT: &str = "test-support";

/// What one new crate may depend on. Editing a row is the deliberate act §11 asks for.
struct Row {
    name: &'static str,
    workspace: &'static [&'static str],
    external: &'static [&'static str],
    dev_external: &'static [&'static str],
}

/// The crate table of §5, and the external lists of §11 (ii).
const ROWS: &[Row] = &[
    Row {
        name: CONTRACTS,
        workspace: &[],
        external: &["serde", "serde_json", "time", "sha2", "ulid"],
        dev_external: &[],
    },
    Row {
        name: "nexus-run-engine",
        workspace: &[CONTRACTS],
        external: &["serde", "serde_json", "sha2"],
        dev_external: &[],
    },
    Row {
        name: "nexus-run-store-sqlite",
        workspace: &[CONTRACTS],
        external: &["rusqlite", "serde", "serde_json", "time"],
        dev_external: &["tempfile"],
    },
    Row {
        name: "nexus-run-session-claude",
        workspace: &[CONTRACTS],
        external: &["libc", "serde", "serde_json", "time"],
        dev_external: &["tempfile"],
    },
    Row {
        name: HOST,
        workspace: &[
            CONTRACTS,
            "nexus-run-engine",
            "nexus-run-store-sqlite",
            "nexus-run-session-claude",
        ],
        external: &["serde", "serde_json", "time", "ulid"],
        dev_external: &["tempfile"],
    },
];

/// The only existing crates that may import the host.
const HOST_IMPORTERS: &[&str] = &["nexus-chat", "nxs"];

/// The part of `cargo metadata --no-deps` the gate reads; with `--no-deps` every package is a
/// workspace member.
#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct Dependency {
    pub name: String,
    /// `dev`, `build`, or absent for a normal dependency.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub rename: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

impl Dependency {
    fn is_dev(&self) -> bool {
        self.kind.as_deref() == Some("dev")
    }

    /// The name a feature entry of the depending package calls this dependency by.
    fn local_name(&self) -> &str {
        self.rename.as_deref().unwrap_or(&self.name)
    }
}

/// One source file, its path relative to the repository root with forward slashes.
pub struct Source {
    pub path: String,
    pub text: String,
}

/// Runs the whole gate against the repository at `root`, naming every problem it finds.
pub fn run(root: &Path) -> Result<()> {
    let metadata = read_metadata(root)?;
    let sources = collect_sources(root)?;
    match (check(&metadata), check_sources(&sources)) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(problem), Ok(())) | (Ok(()), Err(problem)) => Err(problem),
        (Err(edges), Err(files)) => bail!("{edges:#}\n{files:#}"),
    }
}

/// The rules over dependency edges and features.
pub fn check(metadata: &Metadata) -> Result<()> {
    report(edge_problems(metadata))
}

/// The rules over source text.
pub fn check_sources(sources: &[Source]) -> Result<()> {
    report(source_problems(sources))
}

fn report(problems: Vec<String>) -> Result<()> {
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "the layering gate found {} problem(s):\n  {}",
        problems.len(),
        problems.join("\n  ")
    )
}

pub fn read_metadata(root: &Path) -> Result<Metadata> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("reading cargo metadata")
}

/// Every `*.rs` under `crates/run-*/src`, and this file.
pub fn collect_sources(root: &Path) -> Result<Vec<Source>> {
    let mut files = Vec::new();
    let crates = root.join("crates");
    for entry in std::fs::read_dir(&crates).with_context(|| format!("{}", crates.display()))? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("run-") {
            walk(&entry.path().join("src"), &mut files)?;
        }
    }
    files.push(root.join(GATE_FILE));
    files
        .into_iter()
        .map(|file| {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let relative = file.strip_prefix(root).unwrap_or(&file);
            Ok(Source {
                path: relative.to_string_lossy().replace('\\', "/"),
                text,
            })
        })
        .collect()
}

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("{}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn edge_problems(metadata: &Metadata) -> Vec<String> {
    let workspace: BTreeSet<&str> = metadata.packages.iter().map(|p| p.name.as_str()).collect();
    let mut problems = Vec::new();
    for package in &metadata.packages {
        if package.name.starts_with(RUN_PREFIX) {
            own_edges(package, &workspace, &mut problems);
            own_features(package, &mut problems);
        }
        for dep in &package.dependencies {
            importer_rule(package, dep, &mut problems);
            test_support_edge(package, dep, &mut problems);
        }
        test_support_features(package, &mut problems);
    }
    problems
}

/// What a new crate depends on, against its row.
fn own_edges(package: &Package, workspace: &BTreeSet<&str>, problems: &mut Vec<String>) {
    let Some(row) = ROWS.iter().find(|row| row.name == package.name) else {
        problems.push(format!(
            "{} has no row in the layering gate: add one, with its allowed dependencies",
            package.name
        ));
        return;
    };
    for dep in &package.dependencies {
        let name = dep.name.as_str();
        let edge = format!("{} -> {name}", package.name);
        if workspace.contains(name) {
            if !row.workspace.contains(&name) {
                problems.push(format!(
                    "{edge}: outside the crate table of the specification (§5); a new crate never \
                     imports an existing one"
                ));
            }
        } else if !(row.external.contains(&name)
            || dep.is_dev() && row.dev_external.contains(&name))
        {
            problems.push(format!(
                "{edge}: an external dependency outside this crate's list in the layering gate"
            ));
        }
    }
}

/// `nexus-run-*` crates have no feature flags, with the one exception §11 names.
fn own_features(package: &Package, problems: &mut Vec<String>) {
    for feature in package.features.keys() {
        if !(package.name == HOST && feature == TEST_SUPPORT) {
            problems.push(format!(
                "{} declares the feature `{feature}`: the only feature of a new crate is {HOST}'s \
                 `{TEST_SUPPORT}`",
                package.name
            ));
        }
    }
}

/// Who may import a new crate. A new crate's own imports are its row's business above.
fn importer_rule(package: &Package, dep: &Dependency, problems: &mut Vec<String>) {
    let (importer, imported) = (package.name.as_str(), dep.name.as_str());
    let guarded = imported.starts_with(RUN_PREFIX) && imported != CONTRACTS;
    if !guarded || importer.starts_with(RUN_PREFIX) {
        return;
    }
    let edge = format!("{importer} -> {imported}");
    if imported == HOST {
        if !HOST_IMPORTERS.contains(&importer) {
            problems.push(format!(
                "{edge}: {HOST} is imported only by {}",
                HOST_IMPORTERS.join(" and ")
            ));
        }
    } else {
        problems.push(format!(
            "{edge}: {imported} is imported only by {RUN_PREFIX}* crates"
        ));
    }
}

/// The feature on an edge: the shipped service must not be able to reach the crash harness.
fn test_support_edge(package: &Package, dep: &Dependency, problems: &mut Vec<String>) {
    if dep.name == HOST && !dep.is_dev() && dep.features.iter().any(|f| f == TEST_SUPPORT) {
        problems.push(format!(
            "{} -> {HOST} enables `{TEST_SUPPORT}` on a non-dev edge: only dev-dependencies may",
            package.name
        ));
    }
}

/// The feature through a feature table, where the lean build (or any other) could turn it on.
fn test_support_features(package: &Package, problems: &mut Vec<String>) {
    let aliases: Vec<&str> = package
        .dependencies
        .iter()
        .filter(|dep| dep.name == HOST)
        .map(Dependency::local_name)
        .chain([HOST])
        .collect();
    for (feature, enables) in &package.features {
        for entry in enables {
            let reaches = aliases.iter().any(|alias| {
                *entry == format!("{alias}/{TEST_SUPPORT}")
                    || *entry == format!("{alias}?/{TEST_SUPPORT}")
            }) || (package.name == HOST && entry == TEST_SUPPORT);
            if reaches {
                problems.push(format!(
                    "{}'s feature `{feature}` enables `{TEST_SUPPORT}` of {HOST}: only \
                     dev-dependencies may, or the lean build could reach it",
                    package.name
                ));
            }
        }
    }
}

fn source_problems(sources: &[Source]) -> Vec<String> {
    let mut problems = Vec::new();
    for source in sources {
        let path = source.path.as_str();
        let in_new_crate = path.starts_with("crates/run-") && path.contains("/src/");
        if !(in_new_crate || path == GATE_FILE) {
            continue;
        }
        let lines = source.text.lines().count();
        if lines > MAX_LINES {
            problems.push(format!(
                "{path} has {lines} lines: the budget is {MAX_LINES} per file"
            ));
        }
        let tokens = tokenize(&source.text);
        inline_tests(path, &tokens, &mut problems);
        if path.starts_with("crates/run-engine/src/") {
            purity(path, &tokens, &mut problems);
        }
        if path.starts_with("crates/run-engine/src/")
            || path.starts_with("crates/run-contracts/src/")
        {
            hash_collections(path, &tokens, &mut problems);
        }
    }
    problems
}

/// A token of Rust source: an identifier, or one character of punctuation, on its line.
#[derive(Debug, PartialEq)]
struct Token {
    text: String,
    line: usize,
}

/// Splits source into tokens, leaving out comments and the contents of string and char literals, so a
/// sentence or a test fixture about a rule never trips it.
fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => i = skip_block_comment(&chars, i, &mut line),
            '"' => i = skip_string(&chars, i, &mut line),
            '\'' => i = skip_char_or_lifetime(&chars, i),
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if let Some(hashes) = raw_string_prefix(&word, &chars, i) {
                    i = skip_raw_string(&chars, i + hashes, hashes, &mut line);
                } else {
                    tokens.push(Token { text: word, line });
                }
            }
            c if c.is_whitespace() => i += 1,
            c => {
                tokens.push(Token {
                    text: c.to_string(),
                    line,
                });
                i += 1;
            }
        }
    }
    tokens
}

/// The number of `#`s when `word` just read is the prefix of a raw string starting at `at`.
fn raw_string_prefix(word: &str, chars: &[char], at: usize) -> Option<usize> {
    if word != "r" && word != "br" && word != "cr" {
        return None;
    }
    let hashes = chars[at..].iter().take_while(|c| **c == '#').count();
    (chars.get(at + hashes) == Some(&'"')).then_some(hashes)
}

fn skip_block_comment(chars: &[char], start: usize, line: &mut usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            depth += 1;
            i += 2;
        } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            if chars[i] == '\n' {
                *line += 1;
            }
            i += 1;
        }
    }
    i
}

fn skip_string(chars: &[char], start: usize, line: &mut usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if chars.get(i + 1) == Some(&'\n') {
                    *line += 1;
                }
                i += 2;
            }
            '"' => return i + 1,
            '\n' => {
                *line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    i
}

/// `at` is the opening quote of a raw string closed by a quote and `hashes` `#`s.
fn skip_raw_string(chars: &[char], at: usize, hashes: usize, line: &mut usize) -> usize {
    let mut i = at + 1;
    while i < chars.len() {
        if chars[i] == '\n' {
            *line += 1;
        }
        let closes =
            chars[i] == '"' && chars[i + 1..].iter().take_while(|c| **c == '#').count() >= hashes;
        if closes {
            return i + 1 + hashes;
        }
        i += 1;
    }
    i
}

/// A char literal is skipped whole. A lifetime leaves only its tick, and its name reads as a word.
fn skip_char_or_lifetime(chars: &[char], start: usize) -> usize {
    if chars.get(start + 1) == Some(&'\\') {
        let mut i = start + 2;
        // Past the escaped character, which may itself be a quote.
        i += 1;
        while i < chars.len() && chars[i] != '\'' {
            i += 1;
        }
        return i + 1;
    }
    if chars.get(start + 2) == Some(&'\'') {
        return start + 3;
    }
    start + 1
}

fn is(tokens: &[Token], at: usize, text: &str) -> bool {
    tokens.get(at).is_some_and(|t| t.text == text)
}

/// The index just past the `]` closing the attribute whose `#` is at `at`, or `None` if it is no
/// attribute.
fn attribute_end(tokens: &[Token], at: usize) -> Option<usize> {
    if !is(tokens, at, "#") || !is(tokens, at + 1, "[") {
        return None;
    }
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate().skip(at + 1) {
        match token.text.as_str() {
            "[" => depth += 1,
            "]" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// `#[cfg(test)]` before `mod <name> {`, and any `#[test]` outside a file of unit tests. The out-of-line
/// `#[cfg(test)] mod tests;` is what is allowed.
fn inline_tests(path: &str, tokens: &[Token], problems: &mut Vec<String>) {
    let is_test_file = path.ends_with("/tests.rs") || path.contains("/tests/");
    for at in 0..tokens.len() {
        let Some(end) = attribute_end(tokens, at) else {
            continue;
        };
        let attribute = &tokens[at + 2..end - 1];
        let line = tokens[at].line;
        if !is_test_file && attribute.len() == 1 && attribute[0].text == "test" {
            problems.push(format!(
                "{path}:{line}: an inline test: unit tests live in <module>/tests.rs or tests/"
            ));
        }
        let names = |word: &str| attribute.iter().any(|t| t.text == word);
        if is(attribute, 0, "cfg") && names("test") && !names("not") && opens_a_module(tokens, end)
        {
            problems.push(format!(
                "{path}:{line}: an inline test module: `#[cfg(test)] mod tests;` and \
                 <module>/tests.rs is the form"
            ));
        }
    }
}

/// Whether the item starting at `at`, past any further attributes and a visibility, is `mod <name> {`.
fn opens_a_module(tokens: &[Token], mut at: usize) -> bool {
    while let Some(end) = attribute_end(tokens, at) {
        at = end;
    }
    if is(tokens, at, "pub") {
        at += 1;
        if is(tokens, at, "(") {
            while at < tokens.len() && !is(tokens, at, ")") {
                at += 1;
            }
            at += 1;
        }
    }
    is(tokens, at, "mod") && is(tokens, at + 2, "{")
}

const IMPURE_STD_MODULES: &[&str] = &["time", "env", "fs", "thread"];
const IMPURE_WORDS: &[&str] = &[
    "now_utc",
    "now_local",
    "SystemTime",
    "Instant",
    "RandomState",
];

/// The clock, the environment, the file system, threads and randomness.
fn purity(path: &str, tokens: &[Token], problems: &mut Vec<String>) {
    let mut refuse = |line: usize, what: String| {
        problems.push(format!(
            "{path}:{line}: `{what}`: the engine reads no clock, environment, disk, thread or \
             randomness; its inputs are events"
        ));
    };
    for (at, token) in tokens.iter().enumerate() {
        let text = token.text.as_str();
        if IMPURE_WORDS.contains(&text) {
            refuse(token.line, text.to_owned());
        }
        if text == "Ulid" && path_step(tokens, at) && is(tokens, at + 3, "new") {
            refuse(token.line, "Ulid::new".to_owned());
        }
        if text == "std" && path_step(tokens, at) {
            for (module, line) in std_modules(tokens, at + 3) {
                if IMPURE_STD_MODULES.contains(&module) {
                    refuse(line, format!("std::{module}"));
                }
            }
        }
    }
}

/// Whether `::` follows the token at `at`.
fn path_step(tokens: &[Token], at: usize) -> bool {
    is(tokens, at + 1, ":") && is(tokens, at + 2, ":")
}

/// The modules named directly under `std::`, whose next token is at `at`: one, or a `{ .. }` group's.
fn std_modules(tokens: &[Token], at: usize) -> Vec<(&str, usize)> {
    if !is(tokens, at, "{") {
        return tokens
            .get(at)
            .map(|t| (t.text.as_str(), t.line))
            .into_iter()
            .collect();
    }
    let mut modules = Vec::new();
    let mut depth = 0;
    let mut starts_a_name = false;
    for token in &tokens[at..] {
        match token.text.as_str() {
            "{" => {
                depth += 1;
                starts_a_name = depth == 1;
            }
            "}" => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            "," => starts_a_name = depth == 1,
            word => {
                if starts_a_name {
                    modules.push((word, token.line));
                }
                starts_a_name = false;
            }
        }
    }
    modules
}

/// `decide` must give the same list in every process, and `HashMap` and `HashSet` iterate in an order
/// seeded per process. A name is enough to refuse: a map reached through state iterates with no
/// `HashMap` in sight of the loop, so the type itself stays out of the engine and the contracts.
fn hash_collections(path: &str, tokens: &[Token], problems: &mut Vec<String>) {
    for token in tokens {
        if token.text == "HashMap" || token.text == "HashSet" {
            problems.push(format!(
                "{path}:{}: `{}`: its iteration order differs per process, which `decide` cannot \
                 have; use BTreeMap or BTreeSet",
                token.line, token.text
            ));
        }
    }
}

#[cfg(test)]
mod tests;

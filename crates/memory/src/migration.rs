//! The **judging** migration (6j6v.9yaj) — the step that finishes the arc `NEXUS_MEMORY.md`
//! started: file the memories this workspace already holds, move the hand-written context document
//! into them section by section, and let the projection take over.
//!
//! The structural migration before it (6j6v.e0z6) set *standards*: every memory got a reach
//! (`project`) and a category (`unsorted`) describing what it already did. It is deterministic, it
//! runs itself, and nobody notices. What it cannot do is decide that a memory is a **rule** and not
//! an architecture note, or that it holds **everywhere** and not just here. That is a judgement, and
//! this module is the mechanism a judgement rides on — never the judgement itself.
//!
//! Three properties shape everything below.
//!
//! **The human decides; the machine proposes.** Every entry is a proposal somebody confirms or
//! overwrites, which is why the migration is two verbs and not one: [`plan`](build_plan) reads and
//! proposes, `apply` writes. Nothing is filed silently and nothing is filed in bulk without a
//! document a person can read, diff and edit in between. There is deliberately **no heuristic
//! guess**: an unjudged plan carries the status quo, and `apply` refuses to file an entry nobody
//! judged (see [`validate_plan`]) rather than laundering "unsorted" into a decision.
//!
//! **The command may ask a model, and that does not break the offline promise.** What is forbidden
//! is a model call in the WRITE path — it would make every `nxm remember` indeterminate and
//! network-bound, and `crates/memory/tests/classification.rs` pins that structurally: the library
//! links no HTTP client at all, by name. A one-off, human-invoked command is exactly the sanctioned
//! exception, so the judge here is an **external process** ([`Judge`]) the command pipes the plan
//! through — the engine keeps linking nothing, and the assistant that judges stays a name in one
//! table rather than a hard-wired vendor.
//!
//! **Only hand-written text is imported, and the rule hangs on the CONTENT, not the file name.**
//! `AGENTS.md` is a generated document in this house; importing it back would declare a projection
//! to be the source — the very loop the drift guard exists against. Elsewhere `AGENTS.md` is
//! hand-maintained. So a file whose head is a generated header is skipped whole
//! ([`is_generated`]), a section overlapping an assembler-managed block is skipped
//! ([`managed_regions`]), and everything else is offered.

use crate::error::{NxfError, Result};
use crate::facade::{self, Classification};
use crate::model::{self, Scope};
use crate::project_doc;
use crate::store::{MemoryQuery, MemoryStore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

// ---- the mark: once per stream, not once per machine -------------------------------------------

/// The reserved op target the migration mark rides on. It is NOT a memory key: the mark is written
/// on the [`FIELD_MIGRATION`](model::FIELD_MIGRATION) field, which no register column answers to, so
/// the fact reducer treats it as **store-don't-fold** — it lives in the log, crosses the sync wire
/// like every other op, and never materializes a row in `memories`. That is what lets a second
/// device see the run without the mark showing up as a memory in `prime` or in the projection.
pub const MARK_TARGET: &str = "nxm:migration";

/// What a completed run left behind. Serialized into the mark op's value, so a second device reads
/// back who ran it, when, and how much it moved — enough to ask a real question instead of a bare
/// "already done".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationMark {
    /// The actor that ran the migration.
    pub actor: String,
    /// The `now` the run was stamped with (RFC3339).
    pub at: String,
    /// How many existing memories the run filed.
    pub classified: usize,
    /// How many document sections the run turned into memories.
    pub remembered: usize,
}

// ---- what gets offered for import --------------------------------------------------------------

/// The context documents a workspace root is scanned for, in offer order. Names only — whether a
/// named file is actually importable is decided by its CONTENT ([`is_generated`]), which is why
/// `NEXUS_MEMORY.md` is deliberately ON this list: it is the one file guaranteed to carry a
/// generated header, so the content rule is exercised by the everyday case rather than by a
/// hypothetical one.
pub const CONTEXT_DOCUMENTS: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    "GEMINI.md",
    project_doc::FILE_NAME,
];

/// The heading depth a section is cut at. Level 2 (`##`) is the size rule from the `nxm` ticket
/// (6j6v.e0z6) made concrete: **one section is one memory**, and a `###` subsection stays with the
/// section it argues inside. A coherent introduction has to survive as ONE thought — the need for an
/// ordering only appears once forty fragments lie around without one.
const SPLIT_LEVEL: usize = 2;

/// One importable section of a hand-written context document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The document this came from, relative to the workspace root.
    pub path: String,
    /// The section's own heading text, without the `#` marks (empty for a leading preamble).
    pub heading: String,
    /// The section text WITHOUT its heading line, trimmed. The heading becomes the memory's key, so
    /// repeating it in the body would render a heading inside its own block.
    pub body: String,
    /// The byte range the section occupies in the ORIGINAL document — what pruning removes.
    pub range: std::ops::Range<usize>,
}

/// A context document, or a section of one, that was NOT offered — and why. Reported in the plan so
/// a skip is a statement rather than a silence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSource {
    /// The document, relative to the workspace root.
    pub path: String,
    /// `generated` (the whole file carries a generated header) or `managed` (the section overlaps an
    /// assembler-managed block).
    pub reason: String,
}

/// Whether `content` opens with a generated header — the content rule that decides importability.
///
/// The test is the FIRST non-empty line: an HTML comment saying the file is generated. That is the
/// shape `NEXUS_MEMORY.md` carries (`<!-- generated by nexus-flow — do not edit … -->`) and the
/// shape other generators use; a marker further down the file is a note about a section, not a claim
/// about the document.
pub fn is_generated(content: &str) -> bool {
    let Some(first) = content.lines().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let first = first.trim();
    if !first.starts_with("<!--") {
        return false;
    }
    let lowered = first.to_ascii_lowercase();
    lowered.contains("generated") || lowered.contains("autogenerated")
}

/// The byte ranges of assembler-managed blocks in `content`: from a line beginning `<!-- BEGIN `
/// (column 0) through its matching `<!-- END <NAME> -->` line.
///
/// This mirrors `nxs_init::assembler`'s own marker convention rather than sharing its code, which is
/// private there. The duplication is held honest by a test that runs the REAL assembler over
/// memory's own manifest and asserts this function finds exactly the block it wrote — a stronger
/// pin than a shared helper, because it compares against produced output instead of an intention.
///
/// An unbalanced BEGIN (no matching END) claims the rest of the file: a corrupted managed region is
/// exactly the case where importing "the remainder" would import a machine's text as a human's.
pub fn managed_regions(content: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (start, line) in line_offsets(content) {
        let end = start + line.len();
        match &open {
            None => {
                if let Some(rest) = line.strip_prefix("<!-- BEGIN ") {
                    let name = rest
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    open = Some((start, format!("<!-- END {name} -->")));
                }
            }
            Some((begin, closer)) => {
                if line.trim_end() == closer {
                    out.push(*begin..end);
                    open = None;
                }
            }
        }
    }
    if let Some((begin, _)) = open {
        out.push(begin..content.len());
    }
    out
}

/// `(byte offset, line without its newline)` for every line of `content`.
fn line_offsets(content: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        out.push((offset, line.trim_end_matches('\n').trim_end_matches('\r')));
        offset += line.len();
    }
    out
}

/// The importable sections of one context document.
///
/// A file with a generated header yields nothing (and one [`SkippedSource`]). Otherwise the document
/// is cut at every heading of depth ≤ [`SPLIT_LEVEL`], **fenced code is not scanned for headings**
/// (a `# comment` inside a shell block is not a section), and any section overlapping a managed
/// block is dropped with its own [`SkippedSource`] — never merged into a neighbour, which is how a
/// machine-written block would otherwise be imported as somebody's prose.
pub fn sections_of(path: &str, content: &str) -> (Vec<Section>, Vec<SkippedSource>) {
    if is_generated(content) {
        return (
            Vec::new(),
            vec![SkippedSource {
                path: path.to_string(),
                reason: "generated".to_string(),
            }],
        );
    }
    let managed = managed_regions(content);
    let lines = line_offsets(content);

    // Heading starts, in document order: (byte offset of the heading line, heading text).
    let mut starts: Vec<(usize, String)> = Vec::new();
    let mut fence: Option<String> = None;
    for (offset, line) in &lines {
        let trimmed = line.trim_start();
        match &fence {
            Some(marker) => {
                if trimmed.starts_with(marker.as_str()) {
                    fence = None;
                }
                continue;
            }
            None => {
                if let Some(marker) = fence_marker(trimmed) {
                    fence = Some(marker);
                    continue;
                }
            }
        }
        if let Some((level, text)) = heading_of(line) {
            if level <= SPLIT_LEVEL {
                starts.push((*offset, text));
            }
        }
    }

    let mut sections = Vec::new();
    let mut skipped = Vec::new();
    for (i, (start, heading)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(content.len(), |(next, _)| *next);
        if managed.iter().any(|m| m.start < end && *start < m.end) {
            skipped.push(SkippedSource {
                path: path.to_string(),
                reason: "managed".to_string(),
            });
            continue;
        }
        // The body starts after the heading LINE; `content[start..end]` opens with it.
        let after_heading = content[*start..end]
            .split_once('\n')
            .map_or("", |(_, rest)| rest)
            .trim();
        if after_heading.is_empty() {
            continue;
        }
        sections.push(Section {
            path: path.to_string(),
            heading: heading.clone(),
            body: after_heading.to_string(),
            range: *start..end,
        });
    }
    (sections, skipped)
}

/// The fence marker a line opens (```` ``` ````/`~~~`, three or more), or `None`.
fn fence_marker(trimmed: &str) -> Option<String> {
    for ch in ['`', '~'] {
        let run = trimmed.chars().take_while(|c| *c == ch).count();
        if run >= 3 {
            return Some(std::iter::repeat_n(ch, run).collect());
        }
    }
    None
}

/// `(depth, text)` if `line` is an ATX heading (`## Title`), else `None`.
fn heading_of(line: &str) -> Option<(usize, String)> {
    let depth = line.chars().take_while(|c| *c == '#').count();
    if depth == 0 || depth > 6 {
        return None;
    }
    let rest = &line[depth..];
    if !rest.starts_with(' ') && !rest.is_empty() {
        return None; // `#hashtag`, not a heading
    }
    Some((depth, rest.trim().to_string()))
}

/// Scan `root` for importable sections of every [`CONTEXT_DOCUMENTS`] entry that exists.
pub fn scan_documents(root: &Path) -> (Vec<Section>, Vec<SkippedSource>) {
    let mut sections = Vec::new();
    let mut skipped = Vec::new();
    for name in CONTEXT_DOCUMENTS {
        let Ok(content) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        let (found, dropped) = sections_of(name, &content);
        sections.extend(found);
        skipped.extend(dropped);
    }
    (sections, skipped)
}

/// The memory key proposed for a section: a slug of its heading, else of its file stem. `taken`
/// carries the keys already spoken for (existing memories plus earlier sections of the same plan) so
/// a proposal can never overwrite a memory nobody asked it to touch.
fn section_key(section: &Section, taken: &BTreeSet<String>) -> String {
    let base = slug(&section.heading)
        .or_else(|| slug(section.path.trim_end_matches(".md")))
        .unwrap_or_else(|| "section".to_string());
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded sequence has a free key")
}

/// A lower-case slug of `text` (the category grammar): alphanumerics kept, everything else a single
/// `-`. `None` when nothing survives.
fn slug(text: &str) -> Option<String> {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

// ---- the plan document -------------------------------------------------------------------------

/// The plan format version. `apply` refuses a document it does not understand rather than guessing
/// at fields that may have changed meaning.
pub const PLAN_VERSION: u32 = 1;

/// What `apply` does with one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// File an existing memory: write its category/reach/references, leave its text alone.
    Classify,
    /// Write a document section as a new memory under `key`.
    Remember,
    /// Leave it alone. The honest answer for a section that is not durable knowledge — an import
    /// directive, a table of contents — and the way to keep a memory `unsorted` on purpose.
    Skip,
}

/// Where an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A memory that already exists in the store; only its filing is at stake.
    Memory,
    /// A section of a hand-written context document, not yet a memory.
    Section,
}

/// The document section an entry was cut from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySource {
    /// The document, relative to the workspace root. Checked against [`CONTEXT_DOCUMENTS`] before
    /// `apply` rewrites anything ([`validate_plan`]): the `--plan <file>` entry point is the one
    /// that never passes through [`reconcile`], so it is the one that needs the allow-list.
    pub path: String,
    /// The section's heading.
    pub heading: String,
    /// **Which** section of that document — its position among the document's offered sections.
    ///
    /// The heading alone does not identify a section (PR #289 review, Code Quality #1): two `##`
    /// blocks can carry the same text, `section_key` gives them distinct keys (`notes`, `notes-2`)
    /// but the same heading, and pruning by heading membership then deleted BOTH ranges when only
    /// one was remembered — destroying the skipped one's text, which existed nowhere else. The
    /// position disambiguates; the heading is still checked at that position, so a document that
    /// moved on between `plan` and `apply` fails to match and is left alone rather than mis-cut.
    pub index: usize,
}

/// One entry of the plan: the material, and the decision about it. The decision fields are
/// pre-filled with the **status quo**, never with a guess — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub kind: EntryKind,
    /// The memory key: the existing one for a `memory`, the proposed one for a `section`.
    pub key: String,
    /// What to do with it.
    pub action: Action,
    /// The section to file it under — a slug ([`model::is_valid_category`]).
    pub category: String,
    /// How far it reaches (`item` / `project` / `global`).
    pub scope: String,
    /// The board items it is about; only meaningful with reach `item`.
    #[serde(default)]
    pub refs: Vec<String>,
    /// The text. For a `section` this is what will be written; for a `memory` it is the stored text,
    /// carried so the judge can read what it is filing — **`apply` never writes it back**, because
    /// filing a memory must not reword it.
    pub body: String,
    /// The one line the session start replays for this memory (6j6v.xbnh), at most
    /// [`model::INTRODUCTION_MAX_CHARS`] characters. `#[serde(default)]` so a plan written before
    /// the register existed still parses — and then fails validation by name for the entries that
    /// actually need one, which is the loud half of forward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub introduction: Option<String>,
    /// The document section this was cut from (`section` entries only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EntrySource>,
}

/// One vocabulary entry the judge is handed: the word, and what it means here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guide {
    pub name: String,
    pub meaning: String,
}

/// The whole migration plan — the document `plan` emits, a judge revises, a human confirms, and
/// `apply` executes. One shape for all four, so the thing reviewed IS the thing applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub version: u32,
    /// The judgement task, in prose — so an agent handed nothing but this document knows what is
    /// being asked of it.
    pub instructions: String,
    /// The category vocabulary, with meanings.
    pub categories: Vec<Guide>,
    /// The reach vocabulary, with meanings.
    pub scopes: Vec<Guide>,
    /// Every entry awaiting a decision.
    pub entries: Vec<PlanEntry>,
    /// Documents (or sections) deliberately not offered, and why.
    #[serde(default)]
    pub skipped_sources: Vec<SkippedSource>,
}

/// The judgement task the plan carries. Written to be read by a model or a person with equal
/// success: what the two registers mean, what the size rule is, and — the part a mechanism can never
/// supply — that a section which cannot be phrased as a durable project fact was probably never
/// knowledge to begin with.
pub const PLAN_INSTRUCTIONS: &str = "\
Decide, for every entry, where it is filed, how far it reaches, and the one line that stands for \
it. Return THIS SAME JSON document with `category`, `scope`, `refs`, `introduction` and `action` \
decided — do not add entries, do not remove entries, do not change `key`, `kind`, `source`, or the \
`body` of a `memory` entry.\n\n\
- `category` is which section of the project's memory document the entry belongs to; use one of the \
categories below, or your own lower-case slug when none fits. Leaving an entry `unsorted` is \
refused: judge it, or set `action` to `skip`.\n\
- `scope` is how far it reaches. Most project knowledge is `project`. Use `global` only for what \
holds wherever this agent works, and `item` (with `refs`) only for a fact about named board items.\n\
- `action`: `classify` files an existing memory, `remember` writes a document section as a new \
memory, `skip` leaves it alone. Set `skip` on a section that is not durable knowledge — an import \
directive, a table of contents, a pointer to another file. A section that cannot be phrased as a \
durable fact about this project probably never was one.\n\
- `introduction` is the ONE line a session start is handed for this entry: one line, at most 200 \
characters, no line breaks. It must let a reader decide whether to run `nxm recall <key>`, so it \
says what the memory SAYS, not what it is about. For `category: rules` it SPEAKS the rule — \
\"Never X, always Y\", not \"Rules about X\" — because nobody looks a prohibition up before \
breaking it. Required for every entry whose `action` is `remember`.\n\
- One section is one memory. Do not split a coherent argument into fragments.\n\
- The ORDER of `entries` becomes the reading order of the project's memory document, so put it in \
the order somebody should read it: what the project IS, then how it is built and why, then the hard \
rules. You may reorder entries freely; you may not add or remove any.\n\n\
Reply with the JSON document and nothing else.";

/// The category vocabulary handed to the judge: the conventional ones a generated document leads
/// with, each with what it means.
fn category_guide() -> Vec<Guide> {
    [
        (
            "introduction",
            "what this workspace IS — the orienting paragraphs a newcomer needs first",
        ),
        (
            "architecture",
            "how it is built and why — the load-bearing ideas and their reasons",
        ),
        (
            "rules",
            "what must and must not be done here — conventions, gates, hard constraints",
        ),
    ]
    .iter()
    .map(|(name, meaning)| Guide {
        name: (*name).to_string(),
        meaning: (*meaning).to_string(),
    })
    .collect()
}

/// The reach vocabulary handed to the judge, in widening order.
fn scope_guide() -> Vec<Guide> {
    [
        (
            "item",
            "about the board items named in `refs`; reads on `nxf show <id>`, not at session start",
        ),
        (
            "project",
            "holds for this workspace; replayed at every session start (the default)",
        ),
        ("global", "holds wherever the agent works, not just here"),
    ]
    .iter()
    .map(|(name, meaning)| Guide {
        name: (*name).to_string(),
        meaning: (*meaning).to_string(),
    })
    .collect()
}

/// Build the plan for `store` + the context documents under `root`.
///
/// Two halves, in the order a reader meets them: the memories that are still `unsorted` (there is
/// nothing to decide about one already filed), then every offered document section. The decision
/// fields carry the status quo — `plan` proposes material and vocabulary, a judge proposes
/// decisions.
pub fn build_plan(store: &MemoryStore, root: &Path) -> Result<MigrationPlan> {
    let unsorted = facade::memories(
        store,
        &MemoryQuery {
            category: Some(model::CATEGORY_UNSORTED.to_string()),
            ..MemoryQuery::default()
        },
    )?;
    let all = facade::memories(store, &MemoryQuery::default())?;
    let mut taken: BTreeSet<String> = all.iter().map(|m| m.key.clone()).collect();

    let mut entries: Vec<PlanEntry> = unsorted
        .iter()
        .map(|m| PlanEntry {
            kind: EntryKind::Memory,
            key: m.key.clone(),
            action: Action::Classify,
            category: m.category.clone(),
            scope: m.scope.clone(),
            refs: m.refs.clone(),
            body: m.body.clone().unwrap_or_default(),
            // Carried, not proposed: a memory that already has an introduction keeps it, and one
            // that has none is a `None` the judge is asked to fill (6j6v.xbnh). Nothing here
            // derives a line from the body — that mechanism is exactly what the register replaces.
            introduction: m.introduction.clone(),
            source: None,
        })
        .collect();

    let (sections, skipped_sources) = scan_documents(root);
    // The position is per DOCUMENT, matching what `sections_of` returns for that file — `apply`
    // re-cuts one document at a time, so the index it looks up has to be that document's own.
    let mut position: std::collections::BTreeMap<&str, usize> = Default::default();
    for section in &sections {
        let key = section_key(section, &taken);
        taken.insert(key.clone());
        let index = position.entry(section.path.as_str()).or_default();
        entries.push(PlanEntry {
            kind: EntryKind::Section,
            key,
            action: Action::Remember,
            category: model::CATEGORY_UNSORTED.to_string(),
            scope: Scope::DEFAULT.as_str().to_string(),
            refs: Vec::new(),
            body: section.body.clone(),
            introduction: None,
            source: Some(EntrySource {
                path: section.path.clone(),
                heading: section.heading.clone(),
                index: *index,
            }),
        });
        *index += 1;
    }

    Ok(MigrationPlan {
        version: PLAN_VERSION,
        instructions: PLAN_INSTRUCTIONS.to_string(),
        categories: category_guide(),
        scopes: scope_guide(),
        entries,
        skipped_sources,
    })
}

// ---- the judge ---------------------------------------------------------------------------------

/// The env seam that replaces the judge's argv — the deterministic **test** hook (mirrors `NXM_NOW`).
/// Production always resolves the argv from [`Judge`], so the assistant a user names stays a name in
/// one table rather than a string anyone can inject a command through.
pub const JUDGE_CMD_ENV: &str = "NXM_JUDGE_CMD";

/// The coding assistants the migration knows how to ask.
///
/// A NAMED judge, not a free-form command: the invocation recipe belongs in one table in the engine
/// — the same discipline `nexus_chat::role::Model` follows for model aliases — so adding a second
/// assistant is one entry here and `--with-judge <name>` for the user, and an unknown name fails by
/// name instead of spawning whatever it was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Judge {
    /// Claude Code's headless mode (`claude -p`), reading the plan on stdin.
    #[default]
    Claude,
}

impl Judge {
    /// Every judge this build knows, in `--help` order.
    pub const ALL: &'static [Judge] = &[Judge::Claude];

    /// The name a user types after `--with-judge`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Judge::Claude => "claude",
        }
    }

    /// Parse a judge name, or `None` when this build knows no such assistant.
    pub fn parse(s: &str) -> Option<Judge> {
        Judge::ALL.iter().copied().find(|j| j.as_str() == s)
    }

    /// The argv this judge is invoked with — program first. The plan goes in on stdin and the
    /// revised plan is expected on stdout.
    pub fn argv(&self) -> &'static [&'static str] {
        match self {
            Judge::Claude => &["claude", "-p"],
        }
    }
}

impl std::fmt::Display for Judge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How long [`ask_judge`] waits for an answer when the caller does not say otherwise (6j6v.wbwe).
///
/// **Generous on purpose.** A judge reading a plan this size legitimately thinks for minutes — this
/// repository's own run is a 77 KB request and about three of them — and a limit tight enough to
/// interrupt that would throw away a whole judgement, which is worse than the hang it prevents. Half
/// an hour is an order of magnitude past the real work and still bounds the failure this exists for:
/// a judge that is alive and simply never answers (`claude -p` on a hanging login), which used to
/// leave the command standing until somebody pressed Ctrl-C.
pub const DEFAULT_JUDGE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// [`DEFAULT_JUDGE_TIMEOUT`] in the grammar a user types — the `[default: …]` `--help` shows. Two
/// constants for one fact because clap needs a string and the wait needs a [`Duration`];
/// `tests/judge.rs` pins them to each other so they cannot drift apart.
pub const DEFAULT_JUDGE_TIMEOUT_SPELLING: &str = "30m";

/// How [`ask_judge`] runs — the sibling of [`ApplyOptions`], and for the same reason: the knobs of a
/// long-lived call belong in one struct a caller can extend without every call site changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JudgeOptions {
    /// How long to wait for the answer; `None` waits as long as it takes (what this call did before
    /// 6j6v.wbwe, and what `--judge-timeout 0` asks for). [`Default`] is
    /// [`DEFAULT_JUDGE_TIMEOUT`].
    ///
    /// A **zero** duration means the same as `None` — no limit. It is the one value where the two
    /// spellings of "off" could otherwise disagree: `--judge-timeout 0` parses to `None`, while a
    /// host writing `Some(Duration::ZERO)` on this seam would get a judge killed before it could
    /// answer. One reading of zero, whichever way it is written (PR #293 review, Integrity #1).
    pub timeout: Option<Duration>,
}

impl JudgeOptions {
    /// The deadline as [`ask_judge`] applies it: `None` — and a zero duration — wait as long as it
    /// takes. See [`timeout`](JudgeOptions::timeout).
    fn deadline(&self) -> Option<Duration> {
        self.timeout.filter(|limit| !limit.is_zero())
    }
}

impl Default for JudgeOptions {
    fn default() -> JudgeOptions {
        JudgeOptions {
            timeout: Some(DEFAULT_JUDGE_TIMEOUT),
        }
    }
}

/// Read a `--judge-timeout` value: `<n>` plus `s`, `m` or `h` (`45s`, `30m`, `2h`), or `0` for no
/// limit at all. The `<n><unit>` grammar is the house one (`nxc`'s deadlines use the same shape), and
/// a bare number is refused rather than guessed at — "30" is thirty of something the writer knew and
/// the reader does not.
///
/// Zero is **no limit**, not "give up immediately": a zero-second deadline is a command that can
/// never succeed, and the one thing a user actually wants to express here is "wait as long as it
/// takes", the behaviour this flag replaced.
pub fn parse_judge_timeout(s: &str) -> Result<Option<Duration>> {
    let bad = || {
        NxfError::validation(format!(
            "--judge-timeout `{s}` is not a <n><unit> duration (units: s/m/h, e.g. 90s, 30m, 2h; \
             0 waits as long as it takes)"
        ))
    };
    let (digits, unit) = match s.find(|c: char| !c.is_ascii_digit()) {
        // All digits: only `0` is a meaningful unit-less value.
        None if !s.is_empty() => (s, ""),
        Some(split) if split > 0 => s.split_at(split),
        _ => return Err(bad()),
    };
    let n: u64 = digits.parse().map_err(|_| bad())?;
    let per_unit = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        // A unit-less number is only allowed to be the "no limit" zero.
        "" if n == 0 => 1,
        _ => return Err(bad()),
    };
    // Checked: a `<n>` large enough to overflow is a validation error, never a panic.
    let secs = n.checked_mul(per_unit).ok_or_else(bad)?;
    Ok((secs > 0).then(|| Duration::from_secs(secs)))
}

/// Render a deadline back into the grammar it was written in — the inverse of
/// [`parse_judge_timeout`], for the message that reports the wait and for a host that announces it
/// the way the `nxm` CLI does.
pub fn spell_timeout(d: Duration) -> String {
    let secs = d.as_secs();
    if secs == 0 {
        format!("{}ms", d.as_millis())
    } else if secs.is_multiple_of(3_600) {
        format!("{}h", secs / 3_600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// The floor under the wait for an answer the judge has ALREADY written (see [`ask_judge`]). A grace
/// period, not a second deadline: by the time it applies the child has exited, so draining what it
/// wrote takes microseconds — the floor exists only so that a deadline running out in the same
/// instant as a perfectly good answer does not throw that answer away.
const REPLY_DRAIN_GRACE: Duration = Duration::from_secs(5);

/// How long to keep reading the answer of a judge that has already exited, `deadline` into a call
/// that started `elapsed` ago. `None` waits for the answer however long the pipe takes, matching a
/// call that was given no deadline at all.
///
/// Split out from [`ask_judge`] because it is the one piece of the timeout arithmetic worth pinning
/// on its own (PR #293 review, Test Quality #3): the interesting case — a deadline that expires in
/// the same instant the judge answers — cannot be provoked reliably through a subprocess.
fn drain_budget(deadline: Option<Duration>, elapsed: Duration) -> Option<Duration> {
    deadline.map(|limit| limit.saturating_sub(elapsed).max(REPLY_DRAIN_GRACE))
}

/// Ask `judge` to decide `plan`, and return the plan it answered with.
///
/// This is the sanctioned model call: one process, started because a human asked for it, outside
/// every write path. The reply is **validated against the request** ([`reconcile`]) before it is
/// believed — a judge that dropped entries would silently lose knowledge, which is the one failure
/// this whole ticket exists to prevent.
///
/// The wait is bounded by `options` (6j6v.wbwe): a judge that neither answers nor exits is stopped
/// at the deadline and reported, rather than leaving the command standing forever.
pub fn ask_judge(
    judge: Judge,
    plan: &MigrationPlan,
    options: &JudgeOptions,
) -> Result<MigrationPlan> {
    let argv = resolved_argv(judge);
    let (program, args) = argv.split_first().ok_or_else(|| {
        NxfError::validation(format!("the {JUDGE_CMD_ENV} override names no program"))
    })?;
    // The task goes FIRST, as prose, and the document after it. It also travels inside the document
    // (an agent handed nothing but the plan file needs it there), but a prompt whose instructions are
    // buried in a JSON field is a worse prompt — and delivering worse proposals than we could is the
    // failure mode the ticket names by name.
    let request = format!(
        "{PLAN_INSTRUCTIONS}\n\n{}\n",
        serde_json::to_string_pretty(plan)
            .map_err(|e| NxfError::io(format!("serializing the plan for the judge: {e}")))?
    );

    use std::io::Write as _;
    use std::process::{Command, Stdio};
    // Held across the whole call — and across the writing thread's whole life, which is why it is an
    // `Arc` and not a local (see the timeout path below): a judge that answers without reading must
    // not kill the command.
    let sigpipe = std::sync::Arc::new(SigPipeIgnored::install());
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| {
            NxfError::io(format!(
                "starting the judge `{}`: {e}. Install it, or run `nxm migrate plan` without \
                 --with-judge and decide the plan yourself.",
                argv.join(" ")
            ))
        })?;

    // The plan is written from its OWN thread, so that nothing here blocks on it (PR #289 review,
    // Integrity #3). Writing it inline would block once the request outgrows the OS pipe buffer —
    // ~64 KiB — while nothing drains the child's stdout, and the child blocks writing its answer: a
    // two-pipe deadlock with no timeout to break it. That is not a hypothetical size. THIS
    // repository's own plan is 77 KB, because it carries the CLAUDE.md it is migrating; the
    // migration only survived its first real run because `claude -p` happens to read its input to
    // the end before answering. A judge that answers as it reads would hang.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let writing = std::sync::Arc::clone(&sigpipe);
    let writer = std::thread::spawn(move || {
        let written = stdin.write_all(request.as_bytes());
        // The disposition stays ignored until this thread is done with the pipe, however this call
        // returns. On the timeout path below nothing joins this thread — it can outlive the call —
        // and restoring `SIG_DFL` underneath a write that is about to fail with `EPIPE` would kill
        // the whole process with a signal instead of the error this code handles.
        drop(writing);
        written
    });

    // The answer is read by its OWN thread too (6j6v.wbwe). Bounding the wait means POLLING for the
    // child's exit — `wait_with_output` blocks unconditionally and cannot be given up on — so the
    // waiting thread is no longer free to drain stdout while the child writes. A reply that outgrows
    // the ~64 KiB pipe buffer would then wedge the child mid-write and the deadline would fire on a
    // deadlock of our own making. A whole plan is exactly that size: this repository's is 77 KB.
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let (replies, reply_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read as _;
        let mut buf = Vec::new();
        let _ = replies.send(stdout.read_to_end(&mut buf).map(|_| buf));
    });

    let deadline = options.deadline();
    let started = Instant::now();
    let Some(status) = wait_bounded(&mut child, deadline, started)
        .map_err(|e| NxfError::io(format!("waiting for the judge: {e}")))?
    else {
        // Deliberately WITHOUT joining the two helper threads. The child is dead, but a grandchild
        // it left behind can hold both pipes open, and blocking on either would restore exactly the
        // unbounded wait the deadline exists to end. They are detached instead — each ends on its
        // own EOF, and the `Arc` above keeps SIGPIPE ignored for as long as the writer needs it.
        return Err(NxfError::io(format!(
            "the judge `{}` did not answer within {} — it was stopped, and nothing was written. \
             The plan is rebuilt from the store, so re-running `nxm migrate plan --with-judge` \
             loses nothing; give it longer with `--judge-timeout <n>[smh]`, or wait as long as it \
             takes with `--judge-timeout 0`.",
            argv.join(" "),
            deadline.map(spell_timeout).unwrap_or_default()
        )));
    };
    // Reading the answer is bounded too ([`drain_budget`]): a judge that exits while something it
    // spawned still holds its stdout would otherwise hang here instead, one step past the deadline.
    let reply = match drain_budget(deadline, started.elapsed()) {
        None => reply_rx.recv().map_err(|_| ()),
        Some(budget) => reply_rx.recv_timeout(budget).map_err(|_| ()),
    }
    .map_err(|()| {
        NxfError::io(format!(
            "the judge `{}` exited but its answer never arrived",
            argv.join(" ")
        ))
    })?
    .map_err(|e| NxfError::io(format!("reading the judge's answer: {e}")))?;
    // Join AFTER the child is reaped, and read the write failure for what it means rather than as a
    // failure by default:
    //
    // * a BROKEN PIPE means the judge stopped reading — because it exited, or because it had seen
    //   enough. Neither is this side's error: what the judge ANSWERED is the thing that matters, and
    //   `reconcile` below refuses an answer that does not cover every entry. Reporting it would turn
    //   a perfectly good judgement into a failed command.
    // * any other write error is a genuine fault on this side, and is reported — unless the child
    //   also failed, in which case its own status is the more useful message.
    match writer.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) if e.kind() != std::io::ErrorKind::BrokenPipe && status.success() => {
            return Err(NxfError::io(format!("handing the plan to the judge: {e}")))
        }
        Ok(Err(_)) => {}
        Err(_) => {
            return Err(NxfError::io(
                "the thread handing the plan to the judge panicked",
            ))
        }
    }
    if !status.success() {
        return Err(NxfError::io(format!(
            "the judge `{}` exited with {status}",
            argv.join(" ")
        )));
    }
    let judged = parse_reply(&String::from_utf8_lossy(&reply))?;
    reconcile(plan, judged)
}

/// How often the bounded wait looks at the child. Short enough that a deadline of a few hundred
/// milliseconds is honoured to the tick (the tests set exactly that), and negligible against the
/// minutes a real judge takes.
const JUDGE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Wait for `child` to exit, giving up `timeout` after `started`. `Ok(None)` means the deadline
/// passed — the child has then been killed and reaped.
///
/// Polling [`try_wait`](std::process::Child::try_wait) rather than blocking on `wait_with_output` is
/// the only way to bound the wait at all: `wait_with_output` takes the child by value and blocks
/// unconditionally, `std` has no timed variant, and moving the child into a watcher thread would
/// give up the `kill()` this needs. `nexus_chat::timer` reached the same conclusion for `at`
/// (nxf 6j6v.81cq) — the difference here is that the judge's answer is large, so it is drained by a
/// separate thread instead of after the fact.
fn wait_bounded(
    child: &mut std::process::Child,
    timeout: Option<Duration>,
    started: Instant,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if matches!(timeout, Some(limit) if started.elapsed() >= limit) {
            let _ = child.kill();
            let _ = child.wait(); // reap — never leave a zombie behind.
            return Ok(None);
        }
        std::thread::sleep(JUDGE_POLL_INTERVAL);
    }
}

/// Makes a write to a judge that stopped reading surface as the `EPIPE` [`ask_judge`] handles,
/// instead of killing the process — and puts the previous disposition back when it drops.
///
/// Why this is needed at all: Rust's runtime ignores `SIGPIPE` before `main`, which would be enough,
/// but the `nxs` binary deliberately restores `SIG_DFL` (see its `sigpipe` module) so
/// `nxf list | head` dies quietly like any Unix filter. A judge that answers without draining its
/// input then takes the whole command down with a signal.
///
/// Why it is the process disposition and not a thread mask, which is what the textbook prescribes:
/// **measured, not assumed.** `pthread_sigmask(SIG_BLOCK, SIGPIPE)` on the writing thread returns 0
/// and the process still dies with 141 — on this platform the signal reaches a thread that has not
/// blocked it. Blocking it everywhere means blocking it on threads this library does not own, which
/// is worse than a brief, restored change of one disposition.
///
/// The window is exactly the judge call, and [`Drop`] closes it on every path, including the `?`
/// early returns.
#[cfg(unix)]
struct SigPipeIgnored(libc::sighandler_t);

#[cfg(unix)]
impl SigPipeIgnored {
    fn install() -> SigPipeIgnored {
        // SAFETY: one `signal` call; the previous handler is kept and restored in `drop`.
        SigPipeIgnored(unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) })
    }
}

#[cfg(unix)]
impl Drop for SigPipeIgnored {
    fn drop(&mut self) {
        // SAFETY: restores exactly the disposition `install` found.
        unsafe {
            libc::signal(libc::SIGPIPE, self.0);
        }
    }
}

#[cfg(not(unix))]
struct SigPipeIgnored;

#[cfg(not(unix))]
impl SigPipeIgnored {
    fn install() -> SigPipeIgnored {
        SigPipeIgnored
    }
}

/// The argv to spawn: the [`JUDGE_CMD_ENV`] override (whitespace-split) if set, else the judge's own.
fn resolved_argv(judge: Judge) -> Vec<String> {
    match std::env::var(JUDGE_CMD_ENV).ok().filter(|s| !s.is_empty()) {
        Some(cmd) => cmd.split_whitespace().map(str::to_string).collect(),
        None => judge.argv().iter().map(|s| (*s).to_string()).collect(),
    }
}

/// Pull the plan out of a judge's reply. Assistants wrap JSON in prose and code fences however they
/// like, so the outermost `{…}` span is taken rather than the whole stdout — the alternative is a
/// migration that fails on a stray "Here you go:".
pub fn parse_reply(reply: &str) -> Result<MigrationPlan> {
    let start = reply.find('{');
    let end = reply.rfind('}');
    let json = match (start, end) {
        (Some(s), Some(e)) if s < e => &reply[s..=e],
        _ => {
            return Err(NxfError::validation(
                "the judge's reply carries no JSON document".to_string(),
            ))
        }
    };
    serde_json::from_str(json).map_err(|e| {
        NxfError::validation(format!("the judge's reply is not a migration plan: {e}"))
    })
}

/// Take the judge's DECISIONS and nothing else.
///
/// The request is the frame: each entry's key, kind, source and (for a memory) its text come from
/// the store and the documents, and a judge cannot change them — it can only decide `action`,
/// `category`, `scope`, `refs`, the **sequence**, and the body of a section it is rewording.
///
/// The sequence is a decision, not a formality: the plan's order becomes the stored reading order, so
/// "introduction before rules" is exactly the kind of judgement being asked for. Hence the output is
/// built in the JUDGE's order with the request's frames — an earlier version rebuilt it in the
/// REQUEST's order and silently threw the ordering away, which made the claim `apply` rests on
/// false.
///
/// The entry SET, though, is fixed. A dropped entry is reported rather than quietly filed as
/// "unjudged", because lost knowledge is the one failure that would never be noticed here; an
/// invented one is refused for the same reason in the other direction.
pub fn reconcile(request: &MigrationPlan, judged: MigrationPlan) -> Result<MigrationPlan> {
    let mut out = request.clone();
    let mut entries = Vec::with_capacity(request.entries.len());
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for decided in &judged.entries {
        let Some(frame) = request.entries.iter().find(|e| e.key == decided.key) else {
            return Err(NxfError::validation(format!(
                "the judge's reply invents the entry '{}' — nothing was applied",
                decided.key
            )));
        };
        if !seen.insert(frame.key.as_str()) {
            return Err(NxfError::validation(format!(
                "the judge's reply names '{}' twice — nothing was applied",
                decided.key
            )));
        }
        let mut entry = frame.clone();
        entry.action = decided.action;
        entry.category = decided.category.clone();
        entry.scope = decided.scope.clone();
        entry.refs = decided.refs.clone();
        // A section's text may be reworded into a durable sentence; a memory's may not — filing a
        // memory is not permission to rewrite what somebody chose to remember.
        if entry.kind == EntryKind::Section && !decided.body.trim().is_empty() {
            entry.body = decided.body.clone();
        }
        entries.push(entry);
    }
    if let Some(lost) = request
        .entries
        .iter()
        .find(|e| !seen.contains(e.key.as_str()))
    {
        return Err(NxfError::validation(format!(
            "the judge's reply is missing the entry '{}' — nothing was applied",
            lost.key
        )));
    }
    out.entries = entries;
    Ok(out)
}

// ---- applying ----------------------------------------------------------------------------------

/// How `apply` runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Leave the source documents untouched. Off by default: the migration is a MOVE, and a copy
    /// would leave two sources for one truth — the exact state this work ends.
    pub keep_sources: bool,
    /// Compute the report and write nothing at all.
    pub dry_run: bool,
    /// Apply even though this stream already carries a completed migration.
    ///
    /// The once-per-stream rule is enforced HERE, at the write, and not only in the CLI verb that
    /// happens to run first (PR #289 review, Test Quality #1 — Critical). It used to live in
    /// `nxm migrate plan` alone, which left both an embedding app on the Engine seam and
    /// `nxm migrate apply --plan <an old file>` completely unguarded — and applying a stale plan
    /// after a mark exists IS the "two independent filings blend by last-writer-wins" case the mark
    /// was introduced to prevent. `plan --again` and `apply --again` are the same deliberate answer,
    /// asked once where it is only a courtesy and once where it actually decides.
    pub again: bool,
}

/// What a run did — a value, so the CLI renders and a test asserts on the same thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    /// Keys of the memories that were filed.
    pub classified: Vec<String>,
    /// Keys of the memories written from document sections.
    pub remembered: Vec<String>,
    /// Keys deliberately left alone.
    pub skipped: Vec<String>,
    /// Documents the migrated sections were removed from.
    pub pruned: Vec<String>,
    /// Whether this was a dry run (nothing was written).
    pub dry_run: bool,
    /// The mark the run left behind (absent on a dry run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<MigrationMark>,
}

/// Check a plan before a single op is emitted: a run is all-or-nothing, so every reason to refuse is
/// found first.
///
/// The load-bearing one is the **unjudged** check. A plan whose entries still say `unsorted` is a
/// plan nobody judged, and applying it would file forty memories into the category that means "not
/// filed" — a mass filing with the appearance of a decision, which is precisely what "the human
/// decides" rules out. `action: skip` is the way to say "leave this one unsorted" on purpose.
///
/// "Before a single op is emitted" is a promise, and it is only kept if EVERY check the write path
/// would run also happens here. Three of them were missing until the PR #289 review found them, and
/// all three had the same shape — a plan arriving through `--plan <file>`, the one entry point that
/// never passes through [`reconcile`], carrying something `build_plan` would never have produced:
///
/// * a malformed `refs` value, which `Classification::checked` would have rejected inside the loop
///   — after the earlier entries were already written (Integrity #1);
/// * a `source.path` outside [`CONTEXT_DOCUMENTS`], which decides which file gets rewritten
///   (Integrity #2);
/// * a `remember` key already held by a DIFFERENT memory, which last-writer-wins would overwrite in
///   silence (Integrity #4).
pub fn validate_plan(store: &MemoryStore, plan: &MigrationPlan) -> Result<()> {
    if plan.version != PLAN_VERSION {
        return Err(NxfError::validation(format!(
            "this plan is version {} and this build applies version {PLAN_VERSION}",
            plan.version
        )));
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for entry in &plan.entries {
        if !seen.insert(entry.key.as_str()) {
            return Err(NxfError::validation(format!(
                "the plan names '{}' twice",
                entry.key
            )));
        }
        if entry.action == Action::Skip {
            continue;
        }
        if entry.category == model::CATEGORY_UNSORTED {
            return Err(NxfError::validation(format!(
                "'{}' is still '{}' — nobody judged it. Run `nxm migrate plan --with-judge`, give \
                 it a category by hand, or set its action to `skip` to leave it as it is.",
                entry.key,
                model::CATEGORY_UNSORTED
            )));
        }
        if !model::is_valid_category(&entry.category) {
            return Err(NxfError::validation(format!(
                "'{}' names the invalid category '{}': use a lower-case slug",
                entry.key, entry.category
            )));
        }
        if Scope::parse(&entry.scope).is_none() {
            return Err(NxfError::validation(format!(
                "'{}' names the unknown reach '{}': use item, project or global",
                entry.key, entry.scope
            )));
        }
        // The SAME validation the per-entry write would run — reached here so it cannot fire
        // halfway through the loop with earlier entries already committed.
        Classification {
            category: Some(entry.category.clone()),
            scope: Scope::parse(&entry.scope),
            refs: Some(entry.refs.clone()),
            introduction: entry.introduction.clone(),
        }
        .checked()
        .map_err(|e| NxfError::validation(format!("'{}': {}", entry.key, e.msg)))?;

        match entry.action {
            Action::Classify => {
                if store.recall(&entry.key)?.is_none() {
                    return Err(NxfError::not_found(format!(
                        "the plan files '{}', but this workspace has no such memory",
                        entry.key
                    )));
                }
            }
            Action::Remember => {
                if entry.body.trim().is_empty() {
                    return Err(NxfError::validation(format!(
                        "'{}' would be remembered with an empty body",
                        entry.key
                    )));
                }
                // A new memory needs the line the session start replays for it (6j6v.xbnh) —
                // the same rule `facade::remember` enforces, reached HERE so it cannot fire
                // halfway through the loop with earlier entries already committed.
                if entry.introduction.is_none() {
                    return Err(NxfError::validation(format!(
                        "'{}' would be remembered with no introduction. Give the entry an \
                         `introduction` — one line, at most {} characters, the only thing a \
                         session start reads about this memory.",
                        entry.key,
                        model::INTRODUCTION_MAX_CHARS
                    )));
                }
                // A key `build_plan` proposed is free by construction, but a plan is a file: the
                // store can have moved on since, or the key can have been edited by hand. Writing
                // it anyway would overwrite an unrelated memory through last-writer-wins, in
                // silence. Re-applying the SAME plan is still fine — identical text is not a
                // collision — so an interrupted run stays resumable, which is what `apply` promises.
                if let Some(existing) = store.recall(&entry.key)? {
                    if existing.body.as_deref().map(str::trim) != Some(entry.body.trim()) {
                        return Err(NxfError::validation(format!(
                            "'{}' would be remembered over a memory that already exists here and \
                             says something else. Give the entry another key, or forget the \
                             existing memory first if replacing it is what you mean.",
                            entry.key
                        )));
                    }
                }
                // The path decides which FILE a successful apply rewrites, and `--plan <file>` is
                // the entry point that never passed through `reconcile`. Only the documents the
                // scan itself offers are acceptable targets.
                if let Some(source) = &entry.source {
                    if !CONTEXT_DOCUMENTS.contains(&source.path.as_str()) {
                        return Err(NxfError::validation(format!(
                            "'{}' names the source document '{}', which is not one this migration \
                             reads ({})",
                            entry.key,
                            source.path,
                            CONTEXT_DOCUMENTS.join(", ")
                        )));
                    }
                }
            }
            Action::Skip => unreachable!("skip returns above"),
        }
    }

    Ok(())
}

/// Execute `plan`: file the memories, write the sections, store the plan's sequence as the reading
/// order, move the migrated sections out of their documents, and leave the mark.
///
/// The order matters. The mark guard and the validation first (all-or-nothing), then the writes,
/// then the order (its keys have to exist), then the source prune — a document is only stripped of a
/// section once that section is a memory. The mark goes last, so an interrupted run is resumable
/// rather than sealed.
///
/// **The once-per-stream guard lives here**, at the write, so every caller is held to it: the `nxm`
/// CLI, `Engine::migrate_apply`, and an old `plan.json` re-applied a week later. It used to live in
/// the CLI's `migrate plan` alone, which guarded the one path that writes nothing (PR #289 review,
/// Test Quality #1). [`ApplyOptions::again`] is the deliberate way past it.
pub fn apply(
    store: &mut MemoryStore,
    now: &str,
    actor: &str,
    root: &Path,
    plan: &MigrationPlan,
    options: &ApplyOptions,
) -> Result<MigrationReport> {
    if !options.again {
        if let Some(mark) = read_mark(store)? {
            return Err(NxfError::validation(already_migrated(&mark)));
        }
    }
    validate_plan(store, plan)?;

    let mut report = MigrationReport {
        classified: Vec::new(),
        remembered: Vec::new(),
        skipped: Vec::new(),
        pruned: Vec::new(),
        dry_run: options.dry_run,
        mark: None,
    };
    for entry in &plan.entries {
        match entry.action {
            Action::Skip => report.skipped.push(entry.key.clone()),
            Action::Classify => report.classified.push(entry.key.clone()),
            Action::Remember => report.remembered.push(entry.key.clone()),
        }
    }
    // A dry run answers the SAME question the real one does, computed the same way and written
    // nowhere — a preview derived from a different rule would be a preview of a different run.
    let rewrites = if options.keep_sources {
        Vec::new()
    } else {
        planned_rewrites(root, plan)
    };
    if options.dry_run {
        report.pruned = rewrites.into_iter().map(|(path, _)| path).collect();
        return Ok(report);
    }

    for entry in &plan.entries {
        let class = Classification {
            category: Some(entry.category.clone()),
            scope: Scope::parse(&entry.scope),
            refs: Some(entry.refs.clone()),
            introduction: entry.introduction.clone(),
        };
        match entry.action {
            Action::Skip => {}
            Action::Classify => {
                facade::classify(store, now, actor, &entry.key, &class)?;
            }
            Action::Remember => {
                facade::remember(store, now, actor, Some(&entry.key), &entry.body, &class)?;
            }
        }
    }

    // The plan's sequence IS the reading order — the judge already put the introduction before the
    // rules, and throwing that away would mean asking for the same judgement twice.
    let order: Vec<String> = plan
        .entries
        .iter()
        .filter(|e| e.action != Action::Skip)
        .map(|e| e.key.clone())
        .collect();
    if !order.is_empty() {
        facade::reorder(store, now, actor, &order)?;
    }

    for (path, content) in rewrites {
        let file = root.join(&path);
        std::fs::write(&file, &content)
            .map_err(|e| NxfError::io(format!("rewriting {}: {e}", file.display())))?;
        report.pruned.push(path);
    }

    let mark = MigrationMark {
        actor: actor.to_string(),
        at: now.to_string(),
        classified: report.classified.len(),
        remembered: report.remembered.len(),
    };
    write_mark(store, &mark, actor)?;
    report.mark = Some(mark);
    Ok(report)
}

/// The document sections this plan actually turns into a memory, as `(document, source)`.
///
/// Keyed by the SECTION, not by its heading text: two blocks can carry the same words, and
/// identifying them by text is what let a `Skip`ped section be deleted alongside its `Remember`ed
/// twin (PR #289 review, Code Quality #1).
fn migrated_sections(plan: &MigrationPlan) -> Vec<(String, EntrySource)> {
    plan.entries
        .iter()
        .filter(|e| e.action == Action::Remember)
        .filter_map(|e| e.source.as_ref().map(|s| (s.path.clone(), s.clone())))
        .collect()
}

/// `(document, new content)` for every source document this plan takes sections out of — the half
/// that makes the migration a **move** rather than a copy, computed without writing anything so the
/// dry run and the real run answer the same question the same way.
///
/// Sections are located by re-reading the document and re-cutting it, not by trusting byte offsets
/// recorded when the plan was built: a plan is meant to be read and edited by a person, and the file
/// may well have moved on in between. A heading that is no longer there is simply not removed — the
/// memory already carries its text, so the cost is a duplicate somebody can see, never a deletion of
/// something the plan did not name.
fn planned_rewrites(root: &Path, plan: &MigrationPlan) -> Vec<(String, String)> {
    let mut by_document: std::collections::BTreeMap<String, Vec<EntrySource>> = Default::default();
    for (path, source) in migrated_sections(plan) {
        by_document.entry(path).or_default().push(source);
    }
    let mut out = Vec::new();
    for (path, sources) in by_document {
        let Ok(content) = std::fs::read_to_string(root.join(&path)) else {
            continue;
        };
        let (sections, _) = sections_of(&path, &content);
        // Back to front, so an earlier removal cannot shift a later section's range. A section is
        // matched by POSITION and then confirmed by its heading: the position is what tells two
        // identically-headed blocks apart, and the heading is what keeps a document that moved on
        // between `plan` and `apply` from having an unrelated section cut out of it.
        let mut ranges: Vec<std::ops::Range<usize>> = sources
            .iter()
            .filter_map(|source| {
                sections
                    .get(source.index)
                    .filter(|s| s.heading == source.heading)
                    .map(|s| s.range.clone())
            })
            .collect();
        if ranges.is_empty() {
            continue;
        }
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        let mut rewritten = content;
        for range in ranges {
            rewritten.replace_range(range, "");
        }
        out.push((path, tidy_ending(&rewritten)));
    }
    out
}

/// End the document with exactly one newline (or leave it empty).
///
/// The ONLY reformatting a prune does, and deliberately so. A section's range runs from its heading
/// to the start of the next one, so the blank line separating them is removed along with it and the
/// interior needs no repair — while a general blank-run collapse would silently reflow parts of the
/// document nobody touched, including deliberate blank lines inside a fenced code block.
fn tidy_ending(s: &str) -> String {
    let trimmed = s.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

// ---- the mark, and what `prime` reads ----------------------------------------------------------

/// Write the migration mark into the op log.
fn write_mark(store: &mut MemoryStore, mark: &MigrationMark, actor: &str) -> Result<()> {
    let value = serde_json::to_string(mark)
        .map_err(|e| NxfError::io(format!("serializing the migration mark: {e}")))?;
    store.mark_migration(&value, actor);
    Ok(())
}

/// The refusal a marked stream answers with — one sentence, one wording, wherever it is raised.
///
/// Shared by the seam's write guard ([`apply`]) and the CLI's read-side courtesy stop, because two
/// copies of an explanation drift and the reader deserves the same one either way.
pub fn already_migrated(mark: &MigrationMark) -> String {
    format!(
        "this workspace was already migrated by {} on {} ({} memories filed, {} sections \
         imported). Running it again would file the same memories a second time and let \
         last-writer-wins blend the two judgements. Pass --again if that is really what you want.",
        if mark.actor.is_empty() {
            "another device"
        } else {
            &mark.actor
        },
        if mark.at.is_empty() {
            "an earlier run"
        } else {
            &mark.at
        },
        mark.classified,
        mark.remembered
    )
}

/// The migration state of a workspace: what is still unfiled, and whether a run already happened
/// **anywhere on this stream**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationStatus {
    /// How many active memories are still filed as `unsorted`.
    pub unsorted: usize,
    /// The mark a completed run left, if there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<MigrationMark>,
}

impl MigrationStatus {
    /// Whether the MOVE is still open — what `prime` reports on.
    ///
    /// Two conditions, and the second is the one that took a real run to find: something is still
    /// unfiled **and** no run has happened on this stream. A run that deliberately `skip`ped an entry
    /// leaves it `unsorted` on purpose — that is what `skip` means — so counting alone would nag
    /// forever about a decision somebody already made, and a line that cannot be satisfied is a line
    /// people learn to ignore. Once the mark is there the migration is not open; `--again` re-opens
    /// it for whoever wants another pass.
    ///
    /// Deliberately a STORE-only question either way: the session bootstrap reads no filesystem, so
    /// it cannot (and must not) claim anything about the documents on disk.
    pub fn is_open(&self) -> bool {
        self.unsorted > 0 && self.mark.is_none()
    }
}

/// Read the migration state of `store`.
pub fn status(store: &MemoryStore) -> Result<MigrationStatus> {
    let unsorted = facade::memories(
        store,
        &MemoryQuery {
            category: Some(model::CATEGORY_UNSORTED.to_string()),
            ..MemoryQuery::default()
        },
    )?
    .len();
    Ok(MigrationStatus {
        unsorted,
        mark: read_mark(store)?,
    })
}

/// The mark the latest run left on this stream, or `None` if none ever ran.
pub fn read_mark(store: &MemoryStore) -> Result<Option<MigrationMark>> {
    let raw = store
        .migration_mark()
        .map_err(|e| NxfError::io(format!("reading the migration mark: {e}")))?;
    match raw {
        None => Ok(None),
        // A mark written by a NEWER peer may carry fields this build cannot read. Losing the mark
        // would mean re-running the whole judgement on this device, so an unparseable one still
        // counts as "somebody ran it" rather than as nothing at all.
        Some(value) => Ok(Some(serde_json::from_str(&value).unwrap_or(
            MigrationMark {
                actor: String::new(),
                at: String::new(),
                classified: 0,
                remembered: 0,
            },
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::Classification;
    use tempfile::TempDir;

    const NOW: &str = "2026-08-04T12:00:00Z";

    fn store() -> MemoryStore {
        MemoryStore::open_in_memory(1)
    }

    /// `remember` with the introduction these tests are not about — `remember` requires one
    /// (6j6v.xbnh), so the fixtures say it once here rather than at every call.
    fn remember(s: &mut MemoryStore, key: &str, body: &str) {
        let class = Classification {
            introduction: Some(format!("what `{key}` says, in one line")),
            ..Classification::default()
        };
        facade::remember(s, NOW, "alice", Some(key), body, &class).unwrap();
    }

    // ---- what is offered for import ------------------------------------------------------------

    #[test]
    fn a_generated_document_is_skipped_whole_by_its_header_not_its_name() {
        // The rule hangs on CONTENT: importing a projection back into the source is the loop the
        // drift guard exists against, and the file that proves it is the one this very feature
        // writes.
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        let doc = project_doc::render(&s).unwrap();
        assert!(is_generated(&doc), "the projection announces itself");

        let (sections, skipped) = sections_of(project_doc::FILE_NAME, &doc);
        assert!(sections.is_empty(), "{sections:?}");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].reason, "generated");

        // …and a hand-written file with the same NAME would be offered, which is the whole point of
        // keying on content.
        let (sections, _) = sections_of(project_doc::FILE_NAME, "# Notes\n\nsomething I typed\n");
        assert_eq!(sections.len(), 1);
    }

    #[test]
    fn a_section_overlapping_a_managed_block_is_skipped_and_the_rest_is_offered() {
        let content = "# Agent Instructions\n\n\
                       <!-- BEGIN NEXUS v:1 — managed, do not edit -->\n\
                       ## nexus-flow tools for agents\n\n\
                       run `nxs prime`\n\
                       <!-- END NEXUS -->\n\n\
                       ## Shell Commands\n\n\
                       always pass -f\n";
        let (sections, skipped) = sections_of("AGENTS.md", content);
        assert_eq!(
            sections
                .iter()
                .map(|s| s.heading.as_str())
                .collect::<Vec<_>>(),
            ["Shell Commands"],
            "the hand-written section survives; the managed one and its container do not"
        );
        assert_eq!(skipped.len(), 2, "both skips are stated, not silent");
        assert!(skipped.iter().all(|s| s.reason == "managed"));
    }

    #[test]
    fn the_managed_region_scanner_matches_what_the_real_assembler_writes() {
        // The one duplication this design cannot avoid — the assembler's marker handling is private
        // — held honest against its actual OUTPUT rather than against an intention.
        let tmp = TempDir::new().unwrap();
        let module = nxs_init::assembler::ModuleManifest {
            module: "memory".to_string(),
            binary: "nxm".to_string(),
            manifest: crate::onboarding::manifest(),
        };
        nxs_init::assembler::assemble(tmp.path(), std::slice::from_ref(&module)).unwrap();
        let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();

        let regions = managed_regions(&agents);
        assert_eq!(regions.len(), 1, "exactly the block the assembler wrote");
        let block = &agents[regions[0].clone()];
        assert!(block.starts_with("<!-- BEGIN "), "{block}");
        assert!(block.trim_end().ends_with("-->"), "{block}");
        assert!(
            !agents[..regions[0].start].contains("<!-- BEGIN"),
            "the region starts at the marker"
        );
    }

    #[test]
    fn an_unbalanced_managed_marker_claims_the_rest_of_the_file() {
        let content = "# Title\n\n<!-- BEGIN NEXUS -->\n## Managed\n\nbody\n";
        let (sections, skipped) = sections_of("AGENTS.md", content);
        assert!(sections.is_empty(), "{sections:?}");
        assert!(!skipped.is_empty(), "a corrupted region is not imported");
    }

    #[test]
    fn a_hash_inside_a_fenced_block_is_not_a_section() {
        // CLAUDE.md is full of shell blocks whose comments start with `#`. Cutting there would
        // shred a section into nonsense.
        let content =
            "## Build\n\n```bash\n# not a heading\ncargo test\n```\n\n## Ship\n\ntag it\n";
        let (sections, _) = sections_of("CLAUDE.md", content);
        assert_eq!(
            sections
                .iter()
                .map(|s| s.heading.as_str())
                .collect::<Vec<_>>(),
            ["Build", "Ship"]
        );
        assert!(sections[0].body.contains("# not a heading"));
    }

    #[test]
    fn a_subsection_stays_with_the_section_it_argues_inside() {
        // The size rule (6j6v.e0z6): one section is one memory, and a `###` is part of its argument.
        let content = "## Build\n\nrun it\n\n### The trap\n\nmind the symlink\n\n## Ship\n\ntag\n";
        let (sections, _) = sections_of("CLAUDE.md", content);
        assert_eq!(sections.len(), 2);
        assert!(sections[0].body.contains("### The trap"));
        assert!(sections[0].body.contains("mind the symlink"));
    }

    #[test]
    fn a_heading_with_no_body_is_not_an_entry() {
        let (sections, _) = sections_of("CLAUDE.md", "# Title\n\n## Empty\n\n## Real\n\nbody\n");
        assert_eq!(
            sections
                .iter()
                .map(|s| s.heading.as_str())
                .collect::<Vec<_>>(),
            ["Real"]
        );
    }

    // ---- the plan ------------------------------------------------------------------------------

    fn workspace_with(root: &Path, name: &str, content: &str) {
        std::fs::write(root.join(name), content).unwrap();
    }

    #[test]
    fn the_plan_offers_unsorted_memories_and_document_sections_and_judges_neither() {
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        facade::classify(
            &mut s,
            NOW,
            "alice",
            "auth",
            &Classification {
                category: Some("rules".into()),
                ..Default::default()
            },
        )
        .unwrap();
        remember(&mut s, "dolt", "phantom DBs hide in three places");

        let plan = build_plan(&s, tmp.path()).unwrap();
        assert_eq!(
            plan.entries
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            ["dolt", "branching"],
            "the already-filed memory has nothing to decide"
        );
        assert_eq!(plan.entries[0].kind, EntryKind::Memory);
        assert_eq!(plan.entries[1].kind, EntryKind::Section);
        // Nothing is guessed: the decision fields carry the status quo, and the vocabulary plus the
        // task come along so a judge knows what is being asked.
        assert!(plan
            .entries
            .iter()
            .all(|e| e.category == model::CATEGORY_UNSORTED));
        assert!(plan.instructions.contains("skip"));
        assert_eq!(plan.categories.len(), 3);
        assert_eq!(plan.scopes.len(), 3);
    }

    #[test]
    fn a_section_key_never_lands_on_a_memory_that_already_exists() {
        // Silently overwriting an unrelated memory is the one way an import can DESTROY knowledge.
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        facade::remember(
            &mut s,
            NOW,
            "alice",
            Some("branching"),
            "an unrelated memory that happens to own the key",
            &Classification {
                category: Some("rules".into()),
                introduction: Some("branching, as an unrelated memory".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let plan = build_plan(&s, tmp.path()).unwrap();
        let section = plan
            .entries
            .iter()
            .find(|e| e.kind == EntryKind::Section)
            .unwrap();
        assert_eq!(section.key, "branching-2");
    }

    // ---- the judge -----------------------------------------------------------------------------

    #[test]
    fn a_judge_is_a_name_this_build_knows_or_it_is_refused() {
        assert_eq!(Judge::parse("claude"), Some(Judge::Claude));
        assert_eq!(
            Judge::parse("gpt-9"),
            None,
            "an unknown name is not spawned"
        );
        assert_eq!(Judge::default().as_str(), "claude");
        assert_eq!(Judge::Claude.argv()[0], "claude");
    }

    #[test]
    fn an_answer_that_lands_as_the_deadline_runs_out_still_gets_read() {
        // PR #293 review, Test Quality #3. Once the judge has EXITED, what is left to do is drain a
        // pipe that already holds the answer — microseconds of work. Bounding that by the literal
        // remainder of the deadline would throw away a perfectly good judgement whenever the two
        // coincide, which is exactly the moment a slow judge finishes. The floor is the fix, and it
        // is pinned here rather than through a subprocess because the interesting case is a race
        // that cannot be provoked on demand.
        let limit = Duration::from_secs(30 * 60);
        assert_eq!(
            drain_budget(Some(limit), limit),
            Some(REPLY_DRAIN_GRACE),
            "a deadline that ran out in the same instant still gets the grace"
        );
        assert_eq!(
            drain_budget(Some(limit), limit + Duration::from_secs(90)),
            Some(REPLY_DRAIN_GRACE),
            "and it does not go negative when the wait overshot"
        );
        assert_eq!(
            drain_budget(Some(limit), Duration::from_secs(60)),
            Some(limit - Duration::from_secs(60)),
            "with time to spare, the rest of the deadline is the budget"
        );
        assert_eq!(
            drain_budget(None, Duration::from_secs(60)),
            None,
            "no deadline reads the answer for as long as the pipe takes"
        );
    }

    #[test]
    fn a_zero_deadline_means_no_deadline_whichever_way_it_is_written() {
        // PR #293 review, Integrity #1. `--judge-timeout 0` parses to `None`; a host on the library
        // seam writing the same intention as a zero DURATION must not get a judge killed before it
        // can answer.
        assert_eq!(
            JudgeOptions {
                timeout: Some(Duration::ZERO)
            }
            .deadline(),
            None
        );
        assert_eq!(JudgeOptions { timeout: None }.deadline(), None);
        assert_eq!(
            JudgeOptions::default().deadline(),
            Some(DEFAULT_JUDGE_TIMEOUT)
        );
    }

    #[test]
    fn a_reply_is_read_out_of_whatever_prose_the_assistant_wrapped_it_in() {
        let plan = MigrationPlan {
            version: PLAN_VERSION,
            instructions: String::new(),
            categories: vec![],
            scopes: vec![],
            entries: vec![],
            skipped_sources: vec![],
        };
        let json = serde_json::to_string(&plan).unwrap();
        let parsed = parse_reply(&format!("Sure! Here you go:\n\n```json\n{json}\n```\n")).unwrap();
        assert_eq!(parsed.version, PLAN_VERSION);
        assert!(parse_reply("I could not do that").is_err());
    }

    fn entry(key: &str, kind: EntryKind, body: &str) -> PlanEntry {
        PlanEntry {
            kind,
            key: key.to_string(),
            action: if kind == EntryKind::Memory {
                Action::Classify
            } else {
                Action::Remember
            },
            category: model::CATEGORY_UNSORTED.to_string(),
            scope: Scope::DEFAULT.as_str().to_string(),
            refs: vec![],
            body: body.to_string(),
            introduction: Some(format!("what `{key}` says, in one line")),
            source: None,
        }
    }

    fn request() -> MigrationPlan {
        MigrationPlan {
            version: PLAN_VERSION,
            instructions: PLAN_INSTRUCTIONS.to_string(),
            categories: category_guide(),
            scopes: scope_guide(),
            entries: vec![
                entry("auth", EntryKind::Memory, "auth uses JWT"),
                entry("branching", EntryKind::Section, "never commit to main"),
            ],
            skipped_sources: vec![],
        }
    }

    #[test]
    fn reconcile_takes_the_decisions_and_refuses_a_reply_that_lost_an_entry() {
        let req = request();
        let mut judged = req.clone();
        judged.entries[0].category = "architecture".into();
        judged.entries[0].scope = "global".into();
        judged.entries[1].category = "rules".into();
        let out = reconcile(&req, judged.clone()).unwrap();
        assert_eq!(out.entries[0].category, "architecture");
        assert_eq!(out.entries[0].scope, "global");
        assert_eq!(out.entries[1].category, "rules");

        judged.entries.remove(1);
        let err = reconcile(&req, judged).expect_err("a dropped entry is lost knowledge");
        assert!(err.msg.contains("branching"), "{}", err.msg);
    }

    #[test]
    fn the_sequence_the_judge_returns_is_kept_because_it_is_a_decision() {
        // The plan's order becomes the stored reading order, so "introduction before rules" IS the
        // judgement being asked for. Rebuilding the output in the REQUEST's order would silently
        // discard it and make `apply`'s claim false.
        let req = request();
        let mut judged = req.clone();
        judged.entries.reverse();
        for entry in &mut judged.entries {
            entry.category = "rules".into();
        }
        let out = reconcile(&req, judged).unwrap();
        assert_eq!(
            out.entries
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            ["branching", "auth"],
            "the judge's sequence survives"
        );
        assert_eq!(
            out.entries[0].kind,
            EntryKind::Section,
            "and each entry still carries the frame the STORE gave it, not the judge's"
        );
        assert!(
            PLAN_INSTRUCTIONS.contains("reading order"),
            "a capability the task never mentions is one no judge will use"
        );
    }

    #[test]
    fn an_invented_or_duplicated_entry_is_refused() {
        let req = request();
        let mut judged = req.clone();
        let mut ghost = judged.entries[0].clone();
        ghost.key = "never-asked-about".into();
        judged.entries.push(ghost);
        let err = reconcile(&req, judged.clone()).expect_err("an invented entry");
        assert!(err.msg.contains("invents"), "{}", err.msg);

        judged.entries.pop();
        judged.entries.push(judged.entries[0].clone());
        let err = reconcile(&req, judged).expect_err("a duplicated entry");
        assert!(err.msg.contains("twice"), "{}", err.msg);
    }

    #[test]
    fn a_judge_may_reword_a_section_but_never_a_memory() {
        // Filing a memory is not permission to rewrite what somebody chose to remember; a section
        // is still raw document text and may well need to become a sentence.
        let req = request();
        let mut judged = req.clone();
        judged.entries[0].category = "rules".into();
        judged.entries[0].body = "auth uses SAML".into();
        judged.entries[1].category = "rules".into();
        judged.entries[1].body = "Never commit to main; branch first.".into();

        let out = reconcile(&req, judged).unwrap();
        assert_eq!(
            out.entries[0].body, "auth uses JWT",
            "the memory's own words"
        );
        assert_eq!(out.entries[1].body, "Never commit to main; branch first.");
    }

    // ---- applying ------------------------------------------------------------------------------

    /// Give every `remember` entry the introduction a judge would have written (6j6v.xbnh). The
    /// fixtures that call it are about source handling and key collisions, not about the line, so
    /// they say it once here rather than at every entry.
    fn introduce(plan: &mut MigrationPlan) {
        for e in &mut plan.entries {
            if e.action == Action::Remember && e.introduction.is_none() {
                e.introduction = Some(format!("what `{}` says, in one line", e.key));
            }
        }
    }

    /// A judged plan over a store + workspace: `auth` filed under rules, the CLAUDE.md section
    /// remembered under architecture.
    fn judged_plan(root: &Path) -> MigrationPlan {
        let mut plan = request();
        plan.entries[0].category = "rules".into();
        plan.entries[1].category = "architecture".into();
        plan.entries[1].source = Some(EntrySource {
            path: "CLAUDE.md".into(),
            heading: "Branching".into(),
            index: 1,
        });
        let _ = root;
        plan
    }

    #[test]
    fn apply_files_the_memories_writes_the_sections_and_stores_the_order() {
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "# Title\n\nkeep me\n\n## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");

        let plan = judged_plan(tmp.path());
        let report = apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();

        assert_eq!(report.classified, ["auth"]);
        assert_eq!(report.remembered, ["branching"]);
        assert_eq!(facade::recall(&s, "auth").unwrap().category, "rules");
        let written = facade::recall(&s, "branching").unwrap();
        assert_eq!(written.body.as_deref(), Some("never commit to main"));
        assert_eq!(written.category, "architecture");
        // The judge's sequence is kept as the reading order rather than asked for twice.
        assert_eq!(facade::recall(&s, "auth").unwrap().ordinal, Some(1));
        assert_eq!(written.ordinal, Some(2));
    }

    #[test]
    fn apply_moves_the_section_out_of_its_document_instead_of_copying_it() {
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "# Title\n\nkeep me\n\n## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        let report = apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &judged_plan(tmp.path()),
            &ApplyOptions::default(),
        )
        .unwrap();

        assert_eq!(report.pruned, ["CLAUDE.md"]);
        let left = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
        assert!(left.contains("keep me"), "{left}");
        assert!(
            !left.contains("never commit to main"),
            "two sources for one truth is the state this ends: {left}"
        );
    }

    #[test]
    fn two_sections_with_the_same_heading_are_told_apart_by_position() {
        // PR #289 review, Code Quality #1 — the finding that actually destroys text. Two `##`
        // blocks can carry identical words; `section_key` gives them distinct keys but the same
        // heading, so pruning by heading MEMBERSHIP deleted both ranges when only one was
        // remembered. The skipped one's text existed nowhere else — it was never written as a
        // memory — so it was simply gone.
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "## Notes\n\nthe first note, which moves\n\n## Notes\n\nthe second note, which stays\n",
        );
        let mut s = store();

        let mut plan = build_plan(&s, tmp.path()).unwrap();
        assert_eq!(
            plan.entries
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            ["notes", "notes-2"],
            "distinct keys…"
        );
        let sources: Vec<(&str, usize)> = plan
            .entries
            .iter()
            .filter_map(|e| e.source.as_ref().map(|s| (s.heading.as_str(), s.index)))
            .collect();
        assert_eq!(
            sources,
            [("Notes", 0), ("Notes", 1)],
            "…the same heading, and the position is what separates them"
        );

        plan.entries[0].category = "rules".into();
        plan.entries[1].action = Action::Skip;
        introduce(&mut plan);
        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();

        let left = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
        assert!(
            !left.contains("the first note, which moves"),
            "the remembered one moved out: {left}"
        );
        assert!(
            left.contains("the second note, which stays"),
            "and the skipped one is untouched — it exists nowhere else: {left}"
        );
    }

    #[test]
    fn a_document_that_moved_on_between_plan_and_apply_is_left_alone() {
        // The other half of matching by position: the index alone would cut whatever now sits at
        // that spot. The heading is re-checked there, so an edited document loses nothing.
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        let mut plan = build_plan(&s, tmp.path()).unwrap();
        plan.entries[0].category = "rules".into();
        introduce(&mut plan);

        // Somebody rewrote the document in the meantime.
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "## Something Else Entirely\n\nunrelated prose\n",
        );
        let report = apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();

        assert!(report.pruned.is_empty(), "nothing was cut");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap(),
            "## Something Else Entirely\n\nunrelated prose\n"
        );
        assert!(
            facade::recall(&s, "branching").is_ok(),
            "and the memory was still written — the plan's text is what it carries"
        );
    }

    #[test]
    fn pruning_edits_only_what_it_removes() {
        // A prune is a removal, not a reformat. An earlier attempt collapsed blank runs across the
        // whole file to tidy the seam, which silently reflowed sections nobody touched — including
        // deliberate blank lines inside a fenced code block, where they are content.
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "# Title\n\nkeep me\n\n```text\nline\n\n\nstill one block\n```\n\n\
             ## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &judged_plan(tmp.path()),
            &ApplyOptions::default(),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap(),
            "# Title\n\nkeep me\n\n```text\nline\n\n\nstill one block\n```\n",
            "everything that stayed is byte-identical, ending in exactly one newline"
        );
    }

    #[test]
    fn keep_sources_and_dry_run_both_leave_the_document_standing() {
        for options in [
            ApplyOptions {
                keep_sources: true,
                ..Default::default()
            },
            ApplyOptions {
                dry_run: true,
                ..Default::default()
            },
        ] {
            let tmp = TempDir::new().unwrap();
            workspace_with(
                tmp.path(),
                "CLAUDE.md",
                "# Title\n\nkeep me\n\n## Branching\n\nnever commit to main\n",
            );
            let mut s = store();
            remember(&mut s, "auth", "auth uses JWT");
            let report = apply(
                &mut s,
                NOW,
                "alice",
                tmp.path(),
                &judged_plan(tmp.path()),
                &options,
            )
            .unwrap();
            let left = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
            assert!(left.contains("never commit to main"), "{options:?}: {left}");
            if options.dry_run {
                assert!(report.mark.is_none(), "a dry run leaves no mark");
                assert!(
                    facade::recall(&s, "branching").is_err(),
                    "a dry run writes nothing at all"
                );
                assert_eq!(report.pruned, ["CLAUDE.md"], "but it says what it would do");
            }
        }
    }

    #[test]
    fn an_unjudged_plan_is_refused_and_nothing_is_written() {
        // The rule that keeps "the human decides" from degrading into a silent mass filing.
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        let mut plan = request();
        plan.entries.truncate(1); // just the memory, still `unsorted`

        let err = apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .expect_err("nobody judged it");
        assert!(err.msg.contains("unsorted"), "{}", err.msg);
        assert_eq!(
            facade::recall(&s, "auth").unwrap().category,
            model::CATEGORY_UNSORTED,
            "and the refusal left the store exactly as it was"
        );
    }

    #[test]
    fn skip_is_how_a_plan_says_leave_this_one_alone() {
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        let mut plan = request();
        plan.entries.truncate(1);
        plan.entries[0].action = Action::Skip;

        let report = apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();
        assert_eq!(report.skipped, ["auth"]);
        assert_eq!(
            facade::recall(&s, "auth").unwrap().category,
            model::CATEGORY_UNSORTED
        );
    }

    #[test]
    fn a_plan_that_files_a_memory_this_workspace_does_not_have_is_refused() {
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        let mut plan = request();
        plan.entries.truncate(1);
        plan.entries[0].category = "rules".into();
        let err = apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .expect_err("no such memory");
        assert_eq!(err.kind.as_str(), "not_found");
    }

    // ---- the mark ------------------------------------------------------------------------------

    #[test]
    fn the_mark_is_in_the_log_and_is_not_a_memory() {
        // Once per STREAM, not once per machine — and invisible where memories are read, or the
        // bookkeeping would show up in the very document it exists to produce.
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        assert!(status(&s).unwrap().is_open());
        assert!(read_mark(&s).unwrap().is_none());

        let mut plan = request();
        plan.entries.truncate(1);
        plan.entries[0].category = "rules".into();
        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();

        let mark = read_mark(&s).unwrap().expect("the run left a mark");
        assert_eq!(mark.actor, "alice");
        assert_eq!(mark.at, NOW);
        assert_eq!(mark.classified, 1);
        assert!(
            facade::memories(&s, &MemoryQuery::default())
                .unwrap()
                .iter()
                .all(|m| m.key != MARK_TARGET),
            "the mark never materializes as a memory"
        );
        assert!(!status(&s).unwrap().is_open(), "nothing is unsorted now");
    }

    #[test]
    fn a_second_apply_is_refused_by_the_seam_itself_not_by_the_cli_that_ran_first() {
        // PR #289 review, Test Quality #1 (Critical). The guard used to sit in `nxm migrate plan`
        // alone — the one path that writes nothing — which left an embedding app on the Engine seam
        // and `apply --plan <an old file>` completely unguarded. Applying a stale plan after a mark
        // exists IS the "two independent filings blend by last-writer-wins" case the mark prevents.
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        let mut plan = request();
        plan.entries.truncate(1);
        plan.entries[0].category = "rules".into();

        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .expect("the first run");

        let err = apply(
            &mut s,
            NOW,
            "bob",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .expect_err("the same plan, applied again");
        assert!(err.msg.contains("already migrated by alice"), "{}", err.msg);
        assert!(err.msg.contains("--again"), "and it names the way through");

        // …and `again` is that way through, so a deliberate second pass is still possible.
        apply(
            &mut s,
            NOW,
            "bob",
            tmp.path(),
            &plan,
            &ApplyOptions {
                again: true,
                ..Default::default()
            },
        )
        .expect("--again is the explicit answer");
    }

    #[test]
    fn a_dry_run_is_refused_on_a_marked_stream_too() {
        // The guard sits BEFORE the dry-run branch on purpose: a preview of a run that would be
        // refused is a preview of nothing, and reporting it as a plausible outcome is the kind of
        // half-truth this whole ticket is against.
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        let mut plan = request();
        plan.entries.truncate(1);
        plan.entries[0].category = "rules".into();
        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();

        let err = apply(
            &mut s,
            NOW,
            "bob",
            tmp.path(),
            &plan,
            &ApplyOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .expect_err("a preview of a refused run");
        assert!(err.msg.contains("already migrated"), "{}", err.msg);
    }

    #[test]
    fn validate_plan_refuses_every_malformed_plan_before_a_single_op() {
        // PR #289 review, Test Quality #2 + Integrity #1/#2/#4. `--plan <file>` is the one entry
        // point that never passes through `reconcile`, so each of these is reachable — and each one
        // must be caught in the up-front pass, or the promise "all-or-nothing" is only a comment.
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        remember(
            &mut s,
            "occupied",
            "a memory that already lives under this key",
        );

        /// A one-entry plan built from `request()` with `mutate` applied.
        fn plan_with(mutate: impl FnOnce(&mut PlanEntry)) -> MigrationPlan {
            let mut plan = request();
            plan.entries.truncate(1);
            plan.entries[0].category = "rules".into();
            mutate(&mut plan.entries[0]);
            plan
        }

        let cases: Vec<(&str, MigrationPlan, &str)> = vec![
            (
                "version",
                {
                    let mut p = plan_with(|_| {});
                    p.version = PLAN_VERSION + 1;
                    p
                },
                "version",
            ),
            (
                "duplicate key",
                {
                    let mut p = plan_with(|_| {});
                    p.entries.push(p.entries[0].clone());
                    p
                },
                "twice",
            ),
            (
                "invalid category",
                plan_with(|e| e.category = "Two Words".into()),
                "invalid category",
            ),
            (
                "unknown reach",
                plan_with(|e| e.scope = "galaxy".into()),
                "unknown reach",
            ),
            (
                "malformed reference",
                plan_with(|e| e.refs = vec!["a,b".into()]),
                "comma",
            ),
            (
                "empty body",
                plan_with(|e| {
                    e.action = Action::Remember;
                    e.key = "fresh".into();
                    e.body = "   ".into();
                }),
                "empty body",
            ),
            (
                "a key another memory holds",
                plan_with(|e| {
                    e.action = Action::Remember;
                    e.key = "occupied".into();
                    e.body = "something else entirely".into();
                }),
                "already exists",
            ),
            (
                "a source outside the offered documents",
                plan_with(|e| {
                    e.action = Action::Remember;
                    e.key = "fresh".into();
                    e.body = "text".into();
                    e.source = Some(EntrySource {
                        path: "../../etc/passwd".into(),
                        heading: "x".into(),
                        index: 0,
                    });
                }),
                "not one this migration reads",
            ),
            (
                "a memory this workspace does not have",
                plan_with(|e| e.key = "ghost".into()),
                "no such memory",
            ),
            // 6j6v.xbnh (review of PR #381, Test Quality #3). `facade::remember` refuses a write
            // with no introduction, and `validate_plan` mirrors that refusal up front — for
            // exactly the reason every other row here exists: without it the refusal fires inside
            // the write loop, with the entries before it already committed and nothing to roll
            // back into. The row that follows it proves the OTHER half: an over-long line is
            // caught by the same up-front pass, through `Classification::checked()`.
            (
                "a remembered entry with no introduction",
                plan_with(|e| {
                    e.action = Action::Remember;
                    e.key = "fresh".into();
                    e.body = "text".into();
                    e.introduction = None;
                }),
                "no introduction",
            ),
            (
                "an over-long introduction",
                plan_with(|e| e.introduction = Some("x".repeat(201))),
                "201 characters",
            ),
        ];

        for (name, plan, needle) in cases {
            let err = apply(
                &mut s,
                NOW,
                "alice",
                tmp.path(),
                &plan,
                &ApplyOptions::default(),
            )
            .err()
            .unwrap_or_else(|| panic!("{name} was applied instead of refused"));
            assert!(
                err.msg.contains(needle),
                "{name}: expected a message naming {needle:?}, got {:?}",
                err.msg
            );
            // …and every refusal left the store exactly as it was: no ops, no mark.
            assert_eq!(
                facade::recall(&s, "auth").unwrap().category,
                model::CATEGORY_UNSORTED,
                "{name}: nothing was written"
            );
            assert!(read_mark(&s).unwrap().is_none(), "{name}: and no mark");
        }
    }

    #[test]
    fn re_applying_the_same_plan_is_still_allowed_because_identical_text_is_not_a_collision() {
        // The key-collision guard must not cost the resumability `apply` promises: an interrupted
        // run has to be re-runnable, and re-writing the same sentence under the same key is not an
        // overwrite of anybody's memory.
        let tmp = TempDir::new().unwrap();
        workspace_with(
            tmp.path(),
            "CLAUDE.md",
            "## Branching\n\nnever commit to main\n",
        );
        let mut s = store();
        let mut plan = build_plan(&s, tmp.path()).unwrap();
        plan.entries[0].category = "rules".into();
        introduce(&mut plan);

        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();
        validate_plan(&s, &plan).expect("the identical plan still validates");

        // …but the same key over DIFFERENT text is exactly the silent overwrite the guard is for.
        plan.entries[0].body = "a different sentence".into();
        let err = validate_plan(&s, &plan).expect_err("different text under a taken key");
        assert!(err.msg.contains("already exists"), "{}", err.msg);
    }

    #[test]
    fn a_deliberately_skipped_memory_does_not_nag_forever() {
        // Found by running the real thing: `skip` means "leave this one unsorted on purpose", so a
        // count-only openness test would keep `prime` asking about a decision somebody already made
        // — and a line that cannot be satisfied is a line people learn to ignore.
        let tmp = TempDir::new().unwrap();
        let mut s = store();
        remember(&mut s, "auth", "auth uses JWT");
        remember(&mut s, "obsolete", "a memory nobody wants filed");
        let mut plan = request();
        plan.entries.truncate(1);
        plan.entries[0].category = "rules".into();
        plan.entries.push(entry("obsolete", EntryKind::Memory, "…"));
        plan.entries[1].action = Action::Skip;

        apply(
            &mut s,
            NOW,
            "alice",
            tmp.path(),
            &plan,
            &ApplyOptions::default(),
        )
        .unwrap();

        let status = status(&s).unwrap();
        assert_eq!(status.unsorted, 1, "the skipped one is still unfiled");
        assert!(!status.is_open(), "…and the migration is nonetheless done");
    }

    #[test]
    fn a_mark_a_newer_peer_wrote_still_counts_as_a_run() {
        // Losing an unreadable mark would mean re-running the whole judgement on this device — the
        // exact double-filing the mark exists to prevent.
        let mut s = store();
        s.mark_migration("{\"shape\":\"from a later build\"}", "device-b");
        assert!(read_mark(&s).unwrap().is_some());
    }
}

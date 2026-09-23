//! memory's facade — the compute→render seam (E5m/#3rx.1, the mirror of flow's `nexus-flow-facade`).
//!
//! The `nxm` verbs used to print the record straight to stdout and return `Result<()>`; this layer
//! is the **compute** half of a compute→render split: each op returns the canonical
//! [`MemoryRecord`] **as a value** — no `println!`, no clap, no presentation. Each consumer renders
//! over it: the `nxm` CLI serializes the record under `--json` (byte-identical to before) or draws
//! the human line; the in-process [`Engine`](crate::engine::Engine) hands the typed record to an
//! embedding app; the MCP server (E9/#76u) maps it to a tool result.
//!
//! `now` and `actor` are **explicit parameters** on every write (like flow's facade), never read
//! from the ambient clock/env here — the seam is deterministic and the caller owns time/identity.
//! There is NO new semantics: the same `fact` reducer, the same canonical record, the same store
//! helpers the CLI always used (seam invariant, proven by the #3rx.3 differential).
//!
//! The session bootstrap ([`prime`]) joined this layer in nxf 6j6v.wph0. It used to be a private
//! function in `cli.rs` that held the memory rule, the recovery hint and the command list as
//! hard-wired `println!` prose, so an embedding app could not obtain the SessionStart block from the
//! library at all — and app-foundations rebuilt the whole assembly in TypeScript instead. Now
//! [`PrimeReport`] carries every part as data and renders itself; the CLI only prints.

use crate::error::{NxfError, Result};
use crate::key::auto_key;
use crate::migration::{ApplyOptions, MigrationPlan, MigrationReport, MigrationStatus};
use crate::model::{self, MemoryRow, Scope};
use crate::store::{MemoryQuery, MemoryStore};
use serde::Serialize;
use serde_json::{json, Value};

/// The canonical memory record — the presentation-independent value every seam returns. The
/// **declared field order IS the contract**: `serde_json::to_string` preserves it (no
/// `preserve_order` in the build graph), and the `nxm --json` goldens pin the exact bytes
/// `{"key":…,"body":…,"author":…,"updated":…,"active":…,"category":…,"scope":…,"refs":…,"ordinal":…,
/// "introduction":…}`. Owned (not borrowed) so it outlives the store read it came from — an
/// embedding app holds it past the lock. The classification fields (6j6v.e0z6) and `introduction`
/// (6j6v.xbnh) are APPENDED, so every byte a consumer already parsed keeps its position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryRecord {
    /// The stable entity key (the op `target_id`).
    pub key: String,
    /// The memory's text, or `None` when it was forgotten (a tombstone record).
    pub body: Option<String>,
    /// The author of the memory's latest mutation.
    pub author: String,
    /// The display-only timestamp of the latest mutation (may be empty). It dates the FACT: filing
    /// or reordering a memory deliberately leaves it alone.
    pub updated: String,
    /// Whether the memory is currently remembered (`true`) or forgotten (`false`).
    pub active: bool,
    /// The section this memory is filed under — [`model::CATEGORY_UNSORTED`] until classified.
    pub category: String,
    /// How far the memory reaches ([`Scope`]), as stored — `project` unless classified otherwise.
    pub scope: String,
    /// The board items this memory is about; canonical (sorted, deduplicated), possibly empty.
    pub refs: Vec<String>,
    /// The explicit reading position, or `None` when the memory was never reordered.
    pub ordinal: Option<i64>,
    /// The written introduction (6j6v.xbnh) — the ONE line the session bootstrap replays for this
    /// memory — or `None` when nobody has written one yet (every memory that predates the register
    /// is in that state, and the bootstrap names the gap rather than deriving a stand-in).
    pub introduction: Option<String>,
}

impl From<MemoryRow> for MemoryRecord {
    fn from(m: MemoryRow) -> MemoryRecord {
        MemoryRecord {
            key: m.key,
            body: m.body,
            author: m.author,
            updated: m.updated,
            active: m.active,
            category: m.category,
            scope: m.scope,
            refs: m.refs,
            ordinal: m.ordinal,
            introduction: m.introduction,
        }
    }
}

/// The classification a write may set (6j6v.e0z6). Every field is optional and a `None` leaves that
/// register **untouched** — so `remember` can classify while creating, and `classify` can change one
/// register without disturbing the other two (or the memory's text).
///
/// The engine only stores and addresses. WHICH category, reach or reference a memory deserves is
/// the product's judgement, so nothing here decides — it only validates the shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classification {
    /// The section to file the memory under — a slug ([`model::is_valid_category`]).
    pub category: Option<String>,
    /// How far the memory reaches.
    pub scope: Option<Scope>,
    /// The board items the memory is about. An empty slice clears the references.
    pub refs: Option<Vec<String>>,
    /// The written introduction (6j6v.xbnh) — the one line the session bootstrap replays. Optional
    /// HERE like every other register, because `classify` moves one register at a time; that it is
    /// nevertheless MANDATORY on [`remember`] is a rule of that verb, stated there.
    pub introduction: Option<String>,
}

impl Classification {
    /// Whether this asks for any change at all.
    pub fn is_empty(&self) -> bool {
        self.category.is_none()
            && self.scope.is_none()
            && self.refs.is_none()
            && self.introduction.is_none()
    }

    /// Validate the shape and canonicalize the references, or fail with `validation` before any op
    /// is emitted — a write is all-or-nothing, never half-classified.
    ///
    /// `pub(crate)` since the PR #289 review (Integrity #1): the migration's own up-front check has
    /// to reach the SAME validation the per-entry write would eventually run, or a plan whose LAST
    /// entry carries a malformed reference aborts mid-loop after the earlier ones are already
    /// written — the exact opposite of the all-or-nothing this function exists for.
    pub(crate) fn checked(&self) -> Result<Classification> {
        if let Some(category) = &self.category {
            if !model::is_valid_category(category) {
                return Err(NxfError::validation(format!(
                    "invalid category '{category}': use a lower-case slug like \
                     '{}' (letters, digits, '-' and '_')",
                    model::CONVENTIONAL_CATEGORIES[0]
                )));
            }
        }
        let refs = match &self.refs {
            None => None,
            Some(refs) => {
                // A reference is an opaque handle here: memory knows no flow vocabulary (spec
                // §4.4), so it validates the SHAPE and never the existence of the item.
                //
                // The comma is not cosmetic. `refs` is stored as ONE comma-joined register value,
                // so an entry containing a comma is written as one reference and read back as two
                // — silently, with no error anywhere. The `nxm` CLI never produces that (it splits
                // on comma before it gets here), but any other caller on this seam can, which is
                // exactly why the guard belongs at the seam and not in the CLI.
                if let Some(bad) = refs
                    .iter()
                    .find(|r| r.contains(',') || r.trim().contains(char::is_whitespace))
                {
                    return Err(NxfError::validation(format!(
                        "invalid reference '{bad}': an item id carries no whitespace and no comma \
                         (the comma separates references)"
                    )));
                }
                Some(model::normalize_refs(refs))
            }
        };
        if let Some(introduction) = &self.introduction {
            // The length rule is enforced AT THE WRITE and nowhere else (6j6v.xbnh): the render is a
            // total function of what is stored, so a line that is too long has to be refused while
            // somebody is still deciding what it says.
            model::check_introduction(introduction).map_err(NxfError::validation)?;
        }
        Ok(Classification {
            category: self.category.clone(),
            scope: self.scope,
            refs,
            // Stored trimmed: the introduction is a rendered line, and leading/trailing space in
            // an LWW register is two spellings of one value that would never converge.
            introduction: self
                .introduction
                .as_deref()
                .map(str::trim)
                .map(str::to_string),
        })
    }

    /// Emit this classification's registers for `key`. Only the fields the caller actually gave are
    /// written, so an untouched register keeps its version — and its value.
    fn apply(&self, store: &mut MemoryStore, key: &str, actor: &str) {
        if let Some(category) = &self.category {
            store.set_category(key, category, actor);
        }
        if let Some(scope) = self.scope {
            store.set_scope(key, scope, actor);
        }
        if let Some(refs) = &self.refs {
            store.set_refs(key, refs, actor);
        }
        if let Some(introduction) = &self.introduction {
            store.set_introduction(key, introduction, actor);
        }
    }
}

impl From<&MemoryRow> for MemoryRecord {
    fn from(m: &MemoryRow) -> MemoryRecord {
        MemoryRecord::from(m.clone())
    }
}

// ---- reads -----------------------------------------------------------------

/// Recall one active memory by key as a record, or `not_found` (forgotten / never-remembered). A
/// real db error surfaces as `io` (#76u.13) — distinct from the `not_found` of an absent key — now
/// the store read no longer swallows it with `.ok()` (jo9).
pub fn recall(store: &MemoryStore, key: &str) -> Result<MemoryRecord> {
    store
        .recall(key)?
        .map(MemoryRecord::from)
        .ok_or_else(|| NxfError::not_found(format!("no memory '{key}'")))
}

/// List the active memories a [`MemoryQuery`] selects, as records (the same read the store
/// performs; no new semantics). The default query is the one this seam always served: no filter,
/// key-sorted. Fallible (#76u.13): a db error from the underlying `memories` read maps to the
/// structured `io` kind, so the long-lived `nxs mcp serve` stdio server (where
/// `memory_list`/`memory_search` reach this) reports a tool error instead of a handler panic
/// unwinding the connection. The symmetric flow half is `read::list`.
pub fn memories(store: &MemoryStore, query: &MemoryQuery) -> Result<Vec<MemoryRecord>> {
    Ok(store
        .memories(query)?
        .into_iter()
        .map(MemoryRecord::from)
        .collect())
}

/// The memories each of `ids` carries — the retrieval rule's board-item half (6j6v.srpg): the
/// **item**-scoped memories whose `refs` name that id, key-sorted, bucketed by id. An id with none
/// is ABSENT from the map (never an empty vector), so a caller can ask "does this item carry any?"
/// with a single lookup — the shape flow's `next` hint and `show` section both read.
///
/// One store read for the whole batch, not one per id: `nxf next` asks about a lane of items at
/// once, and the item-scoped set is small enough that narrowing it in SQL by reach and bucketing in
/// Rust beats N round-trips (the same trade the substring search makes next door).
///
/// This is the ONE place the rule's item half is applied to stored rows, so `show`, the `next` hint
/// and any embedding host cannot each grow their own reading of it.
pub fn memories_about(
    store: &MemoryStore,
    ids: &[&str],
) -> Result<std::collections::BTreeMap<String, Vec<MemoryRecord>>> {
    let mut out: std::collections::BTreeMap<String, Vec<MemoryRecord>> = Default::default();
    if ids.is_empty() {
        return Ok(out);
    }
    let item_scoped = memories(
        store,
        &MemoryQuery {
            scope: Some(Scope::Item),
            ..MemoryQuery::default()
        },
    )?;
    for id in ids {
        for rec in &item_scoped {
            if model::reaches_item(&rec.scope, &rec.refs, id) {
                out.entry((*id).to_string()).or_default().push(rec.clone());
            }
        }
    }
    Ok(out)
}

// ---- prime: the session bootstrap (nxf 6j6v.wph0) --------------------------

/// The context-recovery hint the bootstrap opens with — the sentence, without the blockquote
/// framing the renderer adds (mirrors flow's `PRIME_CONTEXT_RECOVERY`).
///
/// **Superseded 2026-08-28 (nxf q065, task 4).** The human view no longer renders this hint at
/// all — see [`PrimeReport::render_markdown`]'s doc comment for the full account of what the
/// task-4 cut kept and dropped. This constant and the field it feeds are UNCHANGED: `to_value()`'s
/// `"context_recovery"` key still serializes the string below verbatim.
pub const PRIME_CONTEXT_RECOVERY: &str =
    "re-run `nxs prime` after a context compaction to reload these memories.";

/// The memory rule, stated emphatically (nexus-flow-0f7): agents kept writing a `MEMORY.md`, so it
/// carries CRITICAL emphasis, a NEVER-style imperative, and the explicit consequence. One paragraph
/// of Markdown, rendered verbatim — the whole point of the rule is its exact wording.
///
/// **Superseded 2026-08-28 (nxf q065, task 4).** "Rendered verbatim" stopped being true of the
/// human view: it now opens with [`PRIME_INTRO`] below, in different, shorter wording (the owner's
/// own edited target draft, task-4 brief — not a rewording of THIS constant in place). This
/// constant and the `memory_rule` field are UNCHANGED: `to_value()` still serializes the string
/// below verbatim, unaffected by the human view's cut.
pub const PRIME_MEMORY_RULE: &str =
    "**CRITICAL — Memory rule:** store durable project knowledge (conventions, gotchas, \
     decisions) **only** with `nxm remember`. **NEVER create a `MEMORY.md`** or any other ad-hoc \
     memory file: nxs never reads it, so it is **never replayed** at session start and that \
     knowledge is silently lost. `nxm remember` is the one durable channel (reuse a stable `--key` \
     to evolve a fact in place).";

/// The intro paragraph `nxm prime`'s human view opens with, right after the title (nxf q065, task
/// 4): what `nxm` is for, with the `MEMORY.md` prohibition folded in as a `**Never**` clause rather
/// than a separately labelled, CRITICAL-emphasis rule of its own — the one thing no
/// `--help`/`nxm guide` page teaches, which is exactly why the task-4 brief kept it and cut the
/// rest. Rendered verbatim.
///
/// A deliberately DIFFERENT string from [`PRIME_MEMORY_RULE`], not that constant reworded in
/// place: the owner's own edited target draft (`nxm-static-target.md`) is the specification for
/// this text, and the machine contract (`memory_rule` in `--json`) is untouched — see that
/// constant's own doc comment.
pub const PRIME_INTRO: &str =
    "`nxm` carries durable project knowledge — conventions, gotchas, decisions — and replays it \
     into every session. **Never write a `MEMORY.md`** or any other ad-hoc memory file: nothing \
     reads it, so it is never replayed and the knowledge is silently lost. `nxm remember` is the \
     one durable channel, and reusing a stable `--key` evolves a fact in place. `nxm --help` lists \
     the commands, `nxm guide` the topics behind them.";

/// Why the project's `CLAUDE.md` does not import the generated context document (nxf 6j6v.sebs).
///
/// A SessionStart hook runs this bootstrap — `nxm prime` directly since nxf n2m6 + a2a1, the
/// umbrella's `nxs prime` fanning out to it before that — so by the time anyone reads a line of it,
/// every memory is ALREADY in the session's context. `NEXUS_MEMORY.md` is a
/// projection of the same store ([`crate::project_doc`]), so a `CLAUDE.md` importing it would put
/// each memory in the context a second time. The absent import is therefore the design.
///
/// It is stated HERE, in the bootstrap, because this is the one text every session and every
/// reviewer is handed — and the question it answers is otherwise asked by each of them in turn: a
/// three-way independent review of PR #302 raised the missing import as a High/Medium defect, all
/// three times, because nothing on the path from `CLAUDE.md` to the store says why it is missing.
/// Answering it once, next to the memories it is about, is cheaper than answering it forever.
///
/// **Superseded 2026-08-28 (nxf q065, task 4): "stated HERE" stopped being true of the human
/// view.** The task-4 brief pulled this paragraph out of `nxm prime` for the session-start budget
/// (see [`PrimeReport::render_markdown`]'s doc comment for the full accounting) — but the CLAIM
/// above did not become false, only unstated in the one place every session used to read it. It
/// remains true independently: `crates/nxs-init/src/assembler.rs`'s `HOOK_FALLBACK` doc comment
/// reasons about the exact same subset relationship ("`NEXUS_MEMORY.md` is a projection of the
/// same memories `prime` replays … so the fallback can only ever deliver less than the main path")
/// for its own, unrelated reason (bounding what a hook-less contributor's fallback `cat` can ever
/// show) — checked 2026-08-28 and it names neither this constant nor `nxm prime`, so nothing there
/// depended on this paragraph and nothing there needed to change. *(That reasoning moved from the
/// retired `HOOK_COMMAND` constant to `HOOK_FALLBACK` when nxf n2m6 + a2a1 split the wiring; the
/// sentence quoted above travelled with it unchanged.)* What DOES still depend on this
/// reasoning is `--json`: this constant and the `context_document` field are UNCHANGED, so an
/// embedding host reading `nxm prime --json` still gets the full explanation, and the three-review
/// incident above (an agent with no other way to learn this) cannot recur on that seam.
///
/// **The claim was true of the design and false of the tool until 2026-08-30 (nxf 6j6v.q6e3).**
/// "The absent import is therefore the design" was written as a statement of fact, and it described
/// what a reader would find in a hand-written `CLAUDE.md`. It did not describe what `nxs init`
/// wrote: the assembler put its managed block into `CLAUDE.md` as well as `AGENTS.md`, and that
/// block carries `@NEXUS_MEMORY.md` — so the shipped bootstrap told every session the import was
/// missing on purpose while the tool beside it added one, at 95–120 KB a session in the product
/// repos. Neither this constant nor its reasoning changed; the ASSEMBLER did, and the sentence is
/// now true of both. Recorded here rather than quietly left alone because the gap ran the other way
/// round from the usual one: the prose was right and the code was wrong.
///
/// **Corrected 2026-08-28 (nxf n2m6 + a2a1): the string below names a different hook.** It said the
/// block "arrived through this project's SessionStart hook (`nxs prime`)", which was exact until
/// the wiring became one hook per active module; a memory block now arrives through `nxm prime`.
/// This is a change to a SHIPPED `--json` string and is made rather than deferred because the
/// sentence's whole job is to tell a reader HOW the block reached them — a wrong answer there is
/// worse than a changed byte, and `NEXUS_MEMORY.md`/`CLAUDE.md`, the two things the rest of the
/// paragraph is about, are untouched.
pub const PRIME_CONTEXT_DOCUMENT: &str =
    "**This block IS how the project's memory reaches you.** It arrived through this project's \
     SessionStart hook (`nxm prime`). The generated `NEXUS_MEMORY.md` repeats the index below and \
     adds every body beneath it, so importing that file from `CLAUDE.md` would hand you all of it \
     again. The missing import is deliberate, not an oversight: nothing needs to be added to \
     `CLAUDE.md` for this project's memory to reach you.";

/// The line the bootstrap prints when the workspace holds no memory yet — an invitation to capture
/// one, not a bare "none".
pub const PRIME_NO_MEMORIES: &str =
    "_No memories yet — capture durable project knowledge with `nxm remember`._";

/// The sentence that heads the index, so a reader knows this is a POINTER and not the whole of what
/// the workspace knows (nxf 6j6v.xbnh).
///
/// It replaces the two-class sentence nxf 6j6v.waq9 introduced ("rules here in full, everything
/// else one line"). That cut went the right way and did not clear the host's cut-off: measured on
/// 2026-08-27 a host passes 25.893 bytes of hook output through unchanged and files 32.000 away
/// behind a 2 KB preview — so above the cliff the yield of `prime` is not smaller but NIL, and the
/// full-text class was what kept four workspaces above it. There is no full-text channel any more;
/// what carries the prohibition class instead is the obligation on the introduction itself
/// ([`PRIME_INTRODUCTION_RULE`]): for `rules`, the line SPEAKS the prohibition rather than
/// announcing it. A rule that cannot be said in 200 characters is an essay with a rule inside it.
pub const PRIME_MEMORY_INDEX_RULE: &str =
    "**One line per memory — this is an index, not the memories.** Each line is the introduction \
     its author wrote for it. Read one in full with `nxm recall <key>`, or search their bodies \
     with `nxm memories <text>`.";

/// **The byte budget `nxm prime`'s human block is rendered within** (nxf 6j6v.5jm3).
///
/// The index was the one part of the block whose size nothing bounded: one line per memory, growing
/// linearly with what a workspace has remembered. The fixed prose was cut to its target by nxf q065
/// (1.346 B) and stayed there; the index went on growing past it, and in a worst-case fixture — 55
/// memories each at the 200-character maximum a write can store — the whole block measured 13.528 B
/// against a host that delivers **nothing at all** above 10.240 B per hook output. Not a smaller
/// block: nothing. So an unbounded index is not a block that gets long, it is a module that goes
/// silent, and the more a workspace remembers the sooner it does.
///
/// **9 KiB, and the number is the host's cut-off less a margin.** The cut-off itself is
/// `nxs::prime::SESSION_START_CEILING_BYTES` — 10 KiB, from 1.114 real hook events — and it is
/// deliberately NOT reachable from here: `nxs` depends on this crate, not the other way round, so
/// naming it in code would invert the dependency. What holds the two together instead is a gate
/// that sees both (`crates/nxs/tests/the_session_start_ceiling.rs`), which asserts this constant
/// stays under that one AND measures the real `nxm prime` output against it.
///
/// The margin is 10 % — the same fraction `nxs::prime::CEILING_WARNING_FRACTION` uses for its
/// tripwire, so the block stops filling exactly where the runtime warning would start firing about
/// it. It buys two things worth having: the host's constant was INFERRED from a 39-byte band rather
/// than read off a spec, and a block rendered flush against an inferred edge is a block that sits
/// on it; and the hook's output is this block plus whatever the host puts around it, which is not
/// ours to measure.
///
/// **It bounds the human block, not the record.** [`PrimeReport::to_value`] carries every memory
/// whatever this is set to — the cut is a rendering decision, exactly as the bodies' removal was
/// (6j6v.xbnh), so an embedding host reading `--json` renders its own bootstrap from the whole set.
/// [`index`] serves the whole index to a session that wants it, and a cut block names that verb.
pub const PRIME_BLOCK_BUDGET_BYTES: usize = 9 * 1024;

/// The heading over the index: the plain count when every memory is listed, and the excerpt's own
/// arithmetic when it is not (nxf 6j6v.5jm3).
///
/// `## Memories (showing 34 of 80)` is deliberately the shape `nxf prime` already uses for the
/// board (`## Next (showing 15 of 153)`) — the same move, and a reader who has learnt one has
/// learnt the other. It carries two of the three things a cut has to say: that this is a selection,
/// and how much there is. [`more_memories_line`] carries the third.
fn memories_heading(shown: usize, total: usize) -> String {
    if shown == total {
        format!("## Memories ({total})")
    } else {
        format!("## Memories (showing {shown} of {total})")
    }
}

/// The note under a cut index: how much was left out, why, and the verb that serves it
/// (nxf 6j6v.5jm3).
///
/// It names [`index`]'s verb rather than `nxm memories`, because they answer different questions: a
/// search needs a term, and a reader who has just been told a tail is missing does not have one
/// yet. Present ONLY when something was actually cut — a block that fits announces nothing, so the
/// two states are told apart by reading rather than by counting bullets.
fn more_memories_line(hidden: usize) -> String {
    format!(
        "> **{hidden} more not listed here.** The index is cut to what a session start can \
         deliver. Run `nxm index` for the whole of it, one line per memory."
    )
}

/// **The obligation on a `rules` introduction** (nxf 6j6v.xbnh), stated where an agent reads it
/// before it writes: at session start, in `nxm remember --help`, and in the memory guide.
///
/// It is what keeps the prohibition class alive without a second mechanism. A prohibition is only
/// worth something when it is present BEFORE the mistake, and nobody goes looking for one — so
/// under a pure index the line itself has to do the forbidding. "Never X, always Y" is present;
/// "Rules about X" is a filing label that reaches nobody in time.
///
/// **Superseded 2026-08-28 (nxf q065, task 4).** The human view still states this obligation — it
/// is one of the two things the task-4 brief named as staying, because no `--help`/`nxm guide` page
/// teaches it either — but no longer with THIS exact wording: [`PRIME_INTRODUCTION_HINT`] below
/// carries the shorter, "Writing one:"-less phrasing of the owner's edited target draft
/// (`nxm-static-target.md`). This constant and the `introduction_rule` field are UNCHANGED:
/// `to_value()` still serializes the string below verbatim.
pub const PRIME_INTRODUCTION_RULE: &str =
    "**Writing one:** `--introduction` is ONE line, at most 200 characters, and it says what the \
     memory SAYS. Under `--category rules` it speaks the rule — \"Never X, always Y\", not \
     \"Rules about X\" — because nobody looks a prohibition up before breaking it.";

/// The paragraph `nxm prime`'s human view prints after the "## How a memory is written" cheatsheet
/// (nxf q065, task 4): the same obligation [`PRIME_INTRODUCTION_RULE`] states, in the owner's
/// shorter target-draft wording — no `--help`/`nxm guide` page teaches it, which is why the task-4
/// brief kept it and only reworded and repositioned it (after the code block, not before
/// [`PRIME_CONTEXT_DOCUMENT`], which the same brief dropped outright).
///
/// A deliberately DIFFERENT string from [`PRIME_INTRODUCTION_RULE`], not that constant reworded in
/// place, for the same reason [`PRIME_INTRO`] is a different string from [`PRIME_MEMORY_RULE`]: the
/// target draft is the specification for this text, and the `introduction_rule` field in `--json`
/// stays byte-identical to what shipped before this task.
pub const PRIME_INTRODUCTION_HINT: &str =
    "`--introduction` is ONE line, at most 200 characters, and it says what the memory SAYS — \
     under `--category rules` it speaks the rule (\"Never X, always Y\", not \"Rules about X\"), \
     because nobody looks a prohibition up before breaking it.";

/// How a memory whose introduction nobody has written yet renders in the index (nxf 6j6v.xbnh).
///
/// **The gap is NAMED, not filled.** The mechanism this register replaces derived the line from the
/// body's first line and cut it at 110 characters; run over real data it produced entries reading
/// `- \`update-canary-mhjg\` — -…`, because an agent-written body starts with a dash. A fallback
/// that renders empty is worse than one that renders a gap: it looks like an answer. So this says
/// plainly that the line is missing, and [`PrimeReport::introduction_gap`] says how many are and
/// what to type.
pub const PRIME_NO_INTRODUCTION: &str = "_(no introduction written yet)_";

/// The correction rule (6j6v.9yaj) — the first, deliberately cheap answer to "how does a memory stay
/// true?".
///
/// A memory that was right once goes wrong eventually: the same disease as the stale ticket, only
/// slower. The best-informed corrector this fact will ever have is the agent that just applied it and
/// found the opposite in the code — what it lacks is the permission and the handle. Every memory
/// below is already headed by its key, so the handle exists; this sentence supplies the permission.
///
/// Deliberately not decay, not a contradiction check, not confirm-on-read: those are earned once
/// this demonstrably falls short.
///
/// **Superseded 2026-08-28 (nxf q065, task 4).** The human view still states this rule verbatim in
/// substance — it is the other of the two things the task-4 brief named as staying — but as
/// [`PRIME_KEEP_THEM_TRUE`] below, a byte-distinct string (one punctuation mark: a colon, not an
/// em dash, before "correct it in place"), because the owner's edited target draft
/// (`nxm-static-target.md`) is the specification for the rendered text and this constant's own
/// `--json` contract stays untouched. This constant and the `correction_rule` field are UNCHANGED:
/// `to_value()` still serializes the string below verbatim.
pub const PRIME_CORRECTION_RULE: &str =
    "**Keep them true.** Each memory below is named by its key. If you apply one and the project \
     says otherwise, do not read past it — correct it in place with \
     `nxm remember \"<the corrected fact>\" --key <key>`, or `nxm forget <key>` if it no longer \
     holds at all. You are the best-informed corrector this memory will ever have.";

/// The rendered form of [`PRIME_CORRECTION_RULE`] `nxm prime`'s human view actually prints (nxf
/// q065, task 4) — byte-identical in substance, differing only in the one punctuation mark the
/// owner's edited target draft (`nxm-static-target.md`) uses: a colon, not an em dash, ahead of
/// "correct it in place". See [`PRIME_CORRECTION_RULE`]'s own doc comment for why this is a
/// separate constant rather than that one reworded in place.
pub const PRIME_KEEP_THEM_TRUE: &str =
    "**Keep them true.** Each memory below is named by its key. If you apply one and the project \
     says otherwise, do not read past it: correct it in place with \
     `nxm remember \"<the corrected fact>\" --key <key>`, or `nxm forget <key>` if it no longer \
     holds at all. You are the best-informed corrector this memory will ever have.";

/// One entry of the bootstrap's command reference: the invocation(s) as an agent types them, and
/// what they do. Rendered `- `{invocation}` — {summary}`; the halves are separate so a consumer that
/// is not rendering Markdown (a palette, a tool list) can use them apart.
///
/// `invocations` is a slice so this is the SAME shape chat's bootstrap uses, where one entry may
/// name a pair of closely related verbs sharing a summary — an embedder reading both modules'
/// reports sees one command-reference shape, not two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimeCommand {
    pub invocations: &'static [&'static str],
    pub summary: &'static str,
}

impl PrimeCommand {
    /// The Markdown bullet: every invocation in its own code span, joined by ` / `, then the summary.
    ///
    /// **Superseded 2026-08-28 (nxf q065, task 4).** `PrimeReport::render_markdown` no longer calls
    /// this — the human view's "## How a memory is written" section is
    /// [`PRIME_HOW_A_MEMORY_IS_WRITTEN`], a fixed block, not a loop over [`PRIME_COMMANDS`]. This
    /// method stays public for a caller that wants ONE command bullet rendered this way from the
    /// full list `--json`'s `commands` field still carries — the same reasoning chat's task 3 kept
    /// its own `PrimeCommand::render_markdown` public for — and is covered directly by this file's
    /// own `#[cfg(test)] mod tests`, since the golden that used to exercise it indirectly through
    /// "## Memory Commands" is gone.
    pub fn render_markdown(&self) -> String {
        let invocations: Vec<String> = self.invocations.iter().map(|i| format!("`{i}`")).collect();
        format!("- {} — {}", invocations.join(" / "), self.summary)
    }
}

/// The "## How a memory is written" cheatsheet `nxm prime`'s human view renders instead of looping
/// over [`PRIME_COMMANDS`] (nxf q065, task 4): the three verbs the task-4 brief kept —
/// `nxm remember`, `nxm recall`, `nxm memories` — as literal shell examples with a trailing `#`
/// comment on two of them, mirroring `nxf prime`'s own "## How work moves" cheatsheet
/// (`crates/cli/src/commands/mod.rs`) and `nxc prime`'s "## How a conversation moves"
/// (`crates/chat/src/facade.rs::PRIME_HOW_A_CONVERSATION_MOVES`). The owner-edited target draft
/// (`nxm-static-target.md`) is the specification for the exact wording and column alignment;
/// rendered verbatim, a raw string so neither the embedded `"..."` bodies nor the newlines need
/// escaping.
///
/// Deliberately NOT derived from [`PRIME_COMMANDS`]: `nxm classify` (both entries) and
/// `nxm forget` are absent here — `forget` survives instead in [`PRIME_KEEP_THEM_TRUE`]'s prose,
/// and `classify` is left to `--help`/`nxm guide` (a budget-cut rewrite, not a subset render).
/// `PRIME_COMMANDS` and the `commands` field it feeds are UNCHANGED and still carry all six
/// entries verbatim in `--json`.
pub const PRIME_HOW_A_MEMORY_IS_WRITTEN: &str = r#"    nxm remember "<fact>" --introduction "<one line>" --key <key>
    nxm recall <key>                     # read one memory in full
    nxm memories [<search>]              # list or search them"#;

/// The memory verbs the bootstrap names, in the order it lists them.
///
/// **Superseded 2026-08-28 (nxf q065, task 4).** "The bootstrap names" stopped being true of the
/// RENDERED human view: task 4 cut it to three entries rendered as
/// [`PRIME_HOW_A_MEMORY_IS_WRITTEN`], not as a loop over this list (see that constant's doc comment
/// for why). It is still true of the DATA: this array is UNCHANGED, all six entries still ride
/// `to_value()`'s `commands` field in `--json`.
pub const PRIME_COMMANDS: &[PrimeCommand] = &[
    // 6j6v.xbnh: `--introduction` is REQUIRED, so it belongs in the invocation an agent copies —
    // a reference that shows a form the write path refuses is a reference that costs a round trip.
    PrimeCommand {
        invocations: &["nxm remember \"<fact>\" --introduction \"<one line>\" [--key <key>]"],
        summary: "add or update a fact; the introduction is the one line the index below \
                  replays, and reusing a stable `--key` evolves the fact in place",
    },
    PrimeCommand {
        invocations: &["nxm recall <key>"],
        summary: "read one memory's full text",
    },
    PrimeCommand {
        invocations: &["nxm memories [<search>]"],
        summary: "list or search memories",
    },
    // 6j6v.srpg: the reach is what decides WHERE a memory surfaces, so the verb that sets it has to
    // be in the reference an agent is handed at session start — otherwise the rule is a mechanism
    // nobody can reach, and every memory stays workspace-wide by default forever. The summary names
    // the consequence, not the flag list: what an agent needs to know is which surface it is
    // choosing between.
    PrimeCommand {
        invocations: &["nxm classify <key> --scope item --refs <item-id>"],
        summary: "file a memory against board items; it then reads on `nxf show <item-id>` \
                  instead of here (reach `project`/`global` keeps reading here)",
    },
    PrimeCommand {
        invocations: &["nxm classify <key> --introduction \"<one line>\""],
        summary: "rewrite a memory's line below, leaving its text alone",
    },
    PrimeCommand {
        invocations: &["nxm forget <key>"],
        summary: "remove a memory (reversible)",
    },
];

/// The full `nxm prime` session-bootstrap record (nxf 6j6v.wph0, epic 6j6v.fjrc) — the mirror of
/// flow's `PrimeReport`. It carries the ENTIRE block as data: the context-recovery hint, the memory
/// rule, the command reference, and every active memory.
/// [`render_markdown`](PrimeReport::render_markdown) draws the human form and
/// [`to_value`](PrimeReport::to_value) the `--json` one, so the `nxm` CLI, the in-process
/// [`Engine`](crate::engine::Engine) and any embedding host all serve the SAME bytes from the SAME
/// assembly.
///
/// Deliberately NOT `Serialize`: two ways to serialize one record is exactly how the two views drift
/// apart. [`to_value`](PrimeReport::to_value) is the single JSON projection (flow's `PrimeReport`
/// makes the same choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimeReport {
    /// The context-recovery hint ([`PRIME_CONTEXT_RECOVERY`]), without its blockquote framing.
    pub context_recovery: &'static str,
    /// The memory rule ([`PRIME_MEMORY_RULE`]), one paragraph of Markdown.
    pub memory_rule: &'static str,
    /// The correction rule ([`PRIME_CORRECTION_RULE`]) — how a memory stays true (6j6v.9yaj).
    pub correction_rule: &'static str,
    /// Why `CLAUDE.md` does not import the generated context document
    /// ([`PRIME_CONTEXT_DOCUMENT`], 6j6v.sebs).
    pub context_document: &'static str,
    /// The obligation on the one line a memory is replayed by ([`PRIME_INTRODUCTION_RULE`]) — here
    /// because an agent has to read it BEFORE it writes, and this block is what it reads first.
    pub introduction_rule: &'static str,
    /// The command reference ([`PRIME_COMMANDS`]), in display order.
    pub commands: &'static [PrimeCommand],
    /// The workspace's migration state (6j6v.9yaj), present ONLY while something is still unfiled —
    /// `prime` names an open move and stays silent when none is open. Store-only by construction:
    /// the bootstrap reads no filesystem, so it says nothing about the documents on disk.
    pub migration: Option<crate::migration::MigrationStatus>,
    /// The active memories the retrieval rule replays here, in the store's
    /// [reading order](crate::store::MemoryQuery::ordered) — category first, then the position
    /// within it (6j6v.643z) — and everything except the `item`-scoped ones, which read on their
    /// board items instead (6j6v.srpg, [`model::replayed_at_session_start`]). Both halves of that
    /// order are stored facts, so the sequence is byte-stable, which is what makes this a
    /// SessionStart-hook output.
    pub memories: Vec<MemoryRecord>,
}

impl PrimeReport {
    /// The `prime` `--json` object: a 1:1 structured view of the human layout. `count` is
    /// `memories.len()`, carried explicitly so a consumer need not count the array.
    ///
    /// Keys land in [`Value`]'s own (alphabetical) order — no `preserve_order` in the build graph —
    /// so the bytes are stable without anyone maintaining an order by hand.
    pub fn to_value(&self) -> Value {
        json!({
            "context_recovery": self.context_recovery,
            "memory_rule": self.memory_rule,
            "correction_rule": self.correction_rule,
            "introduction_rule": self.introduction_rule,
            "context_document": self.context_document,
            "migration": self.migration,
            "commands": self.commands
                .iter()
                .map(|c| json!({ "invocations": c.invocations, "summary": c.summary }))
                .collect::<Vec<_>>(),
            "memories": self.memories,
            "count": self.memories.len(),
        })
    }

    /// The blockquote naming an open migration, or nothing when none is open — the carrier the
    /// design picks over `self-update`, because this is the one place that knows whether THIS
    /// workspace still has unfiled memories. No coercion: whoever never migrates keeps working
    /// memories with reach `project`.
    ///
    /// **Moved 2026-08-28 (nxf q065, task 4): from the head of the block to right after the
    /// memories.** The wording is unchanged — only where [`render_markdown`](Self::render_markdown)
    /// places the call moved. It names a one-time, situational move (a workspace that has finished
    /// migrating never sees it at all), not a standing fact every session needs before it reads the
    /// memories themselves, so it no longer has to lead them.
    fn migration_line(&self) -> Option<String> {
        let status = self.migration.as_ref().filter(|m| m.is_open())?;
        Some(format!(
            "> **Open migration:** {} {} still filed as `unsorted`, so this project's memory \
             document cannot read as an argument yet. Run \
             `nxm migrate plan --with-judge --json > migration.json`, read the plan, then \
             `nxm migrate apply --plan migration.json`.",
            status.unsorted,
            if status.unsorted == 1 {
                "memory is"
            } else {
                "memories are"
            }
        ))
    }

    /// How many of the replayed memories carry no written introduction, and the line that says so
    /// — or `None` when every one of them has one (nxf 6j6v.xbnh).
    ///
    /// A COUNT and not just a per-entry placeholder, because the per-entry placeholder answers
    /// "which one is missing" and this answers "how much of what you are reading is missing", which
    /// is the question during the one-off run that writes the introductions for memories older than
    /// the register. It names the command, so seeing the gap and closing it are the same step.
    pub fn introduction_gap(&self) -> Option<String> {
        let missing = self
            .memories
            .iter()
            .filter(|m| introduction_of(m).is_none())
            .count();
        (missing > 0).then(|| {
            format!(
                "> **{missing} of these {} {} no introduction yet** and read as a placeholder \
                 below. Write one with `nxm classify <key> --introduction \"<one line, at most \
                 {} characters>\"`.",
                self.memories.len(),
                if missing == 1 {
                    "memories has"
                } else {
                    "memories have"
                },
                model::INTRODUCTION_MAX_CHARS,
            )
        })
    }

    /// The human session-start block as valid, structured Markdown — the SessionStart-hook output a
    /// host injects verbatim. No trailing newline (the caller's `println!` supplies it), so this
    /// composes cleanly into a larger document too.
    ///
    /// **One line per memory since nxf 6j6v.xbnh**, and no full-text channel at all. The measurement
    /// that decides it is a CLIFF and not a slope: a host passes 25.893 bytes of hook output through
    /// and files 32.000 away behind a 2 KB preview, so above the line the whole bootstrap — board,
    /// core rules, command vocabulary, none of which is written down anywhere else — is replaced by
    /// a file path. Memories were 90 % of the payload and displaced the other 10 %. The two-class
    /// split of nxf 6j6v.waq9 (rules whole, the rest indexed) went the right way and measurably did
    /// not clear the cliff: nexus-flow projected to ~61 KB, manufakt-io to ~47 KB, both fully within
    /// the budget that split declared, because that budget was calibrated against token cost rather
    /// than against the host's threshold.
    ///
    /// So the line each memory gets is WRITTEN, not derived. The derivation it replaces took the
    /// body's first line and cut it at 110 characters, which on real data rendered empty entries for
    /// exactly the memories an agent had written — and agents write nearly all of them.
    ///
    /// `nxf` was doing this all along, in this same text: `## Next (showing 15 of 153)` is a ranked,
    /// bounded projection that leaves the rest to `nxf show <id>`. This is the same move.
    ///
    /// **Superseded 2026-08-28 (nxf q065, task 4, q3fh session-start-budget).** Everything above
    /// this paragraph is still true — one line per memory, nothing derived — but it stopped being
    /// the WHOLE story: the fixed prose, MEASURED (`nxm init && nxm prime` in a bare, empty
    /// workspace — the fixed prose is the whole output there, since nothing is remembered), was
    /// **2.285 B** before this task. The host drops any single hook output above 10.240 B entirely
    /// rather than delivering it smaller, so this fixed prose is a cost paid before a workspace has
    /// remembered anything — and in `watch-bundestag`, the real project workspace this branch is
    /// measured against (9 memories), the SAME fixed prose sat inside a 3.629 B total (task-4
    /// brief).
    ///
    /// The owner's own edited target draft (`nxm-static-target.md`) cuts the fixed prose to
    /// **1.346 B**, MEASURED the same way (`nxm init && nxm prime` in a bare workspace, on this
    /// task's own build): the title gains a subtitle, the `MEMORY.md` prohibition survives as one
    /// clause inside an intro paragraph ([`PRIME_INTRO`]) instead of a CRITICAL-emphasis rule of
    /// its own, "## How a memory is written" replaces "## Memory Commands" with three verbs
    /// instead of six as literal shell examples ([`PRIME_HOW_A_MEMORY_IS_WRITTEN`], not a loop
    /// over [`PRIME_COMMANDS`]), and the introduction obligation moves after that cheatsheet in
    /// shorter wording ([`PRIME_INTRODUCTION_HINT`]). `watch-bundestag`'s own composed `nxm prime`
    /// is now **2.690 B**, MEASURED against the same 9-memory fixture — byte-identical to
    /// `NXM_PRIME-full-example.md` up to that file's own trailing blank line (2.691 B on disk).
    ///
    /// **Dropped outright, not shrunk:** the Context Recovery blockquote
    /// ([`PRIME_CONTEXT_RECOVERY`] — re-running `nxs prime` is not a fact a session needs before it
    /// acts) and the paragraph explaining why `CLAUDE.md` does not import the generated
    /// `NEXUS_MEMORY.md` ([`PRIME_CONTEXT_DOCUMENT`] — the question a session asks about the
    /// document only once it goes looking for it, not before it can act at all; the explanation is
    /// not lost, see that constant's own doc comment for where it still lives). `nxm classify`
    /// leaves the fixed verb list with it (both entries) — `--help`/`nxm guide` teach it on demand
    /// — and still appears, unaffected by this cut, in its OWN conditional spot:
    /// [`introduction_gap`](Self::introduction_gap)'s situational note, which names it only while a
    /// memory has no written line yet.
    ///
    /// **Moved, not dropped:** the open-migration note ([`migration_line`](Self::migration_line))
    /// now prints AFTER the memories instead of before "## Memory Commands" — it names a one-time,
    /// situational move, not a standing fact every session needs before the memories themselves.
    ///
    /// **Every field this used to render stays exactly as it was — this is a rendering cut, not a
    /// data cut.** `to_value()` still serializes `context_recovery`, `memory_rule`,
    /// `introduction_rule`, `context_document`, and all six `commands` verbatim, and
    /// [`PrimeCommand::render_markdown`] stays public for a caller that wants the command reference
    /// rendered that way even though this method no longer calls it — see each constant's/method's
    /// own doc comment for the full account.
    ///
    /// **Superseded 2026-08-29 (nxf 6j6v.5jm3): the index is BOUNDED now, and everything above
    /// stays true of what it renders.** Task 4 cut the fixed prose and left the one part that grows
    /// — one line per memory, bounded by nothing — so a workspace that remembered enough went from
    /// a long block to no block at all. The whole block is now rendered within
    /// [`PRIME_BLOCK_BUDGET_BYTES`]; past it the index carries its leading entries and says what it
    /// left out and where the rest is ([`more_memories_line`], [`index`]). Nothing else about the
    /// block changed, and a workspace whose index fits — every real one today — renders exactly the
    /// bytes it rendered before. See [`render_within`](Self::render_within) for how the fill works.
    pub fn render_markdown(&self) -> String {
        self.render_within(PRIME_BLOCK_BUDGET_BYTES)
    }

    /// [`render_markdown`](Self::render_markdown) against an explicit budget — the testable core,
    /// and the whole of what nxf 6j6v.5jm3 added.
    ///
    /// **The block is filled and then stopped, not sampled.** It renders the whole thing first,
    /// because that is the state every real workspace is in today (nexus-flow's own 50 memories
    /// measure 8.763 B), and only when that does not fit does it add index bullets one at a time
    /// until the next one would not. What a session loses is therefore the TAIL of the reading
    /// order — the argument's later, more particular entries — never a hole in the middle.
    ///
    /// It grows the block rather than computing a remainder and slicing to it, because the pieces
    /// around the index are not a constant: the open-migration note, the introduction gap and the
    /// heading itself all change size with the workspace, and a reserve computed from them is a
    /// second arithmetic that can disagree with the render. Measuring the assembled candidate
    /// cannot.
    ///
    /// **What that costs, stated exactly** (review of PR #389, Code Quality #4 / Integrity #1 —
    /// this paragraph replaces one that claimed "a workspace with ten thousand memories costs the
    /// same as one with a hundred", which was true of the loop and false of the call). Each
    /// memory's own line is formatted ONCE, into `lines` — that part is proportional to the record
    /// this method was handed, and nothing can be below it. Everything after it is bounded by the
    /// BUDGET: the loop stops at the first candidate that does not fit, and each candidate it
    /// assembles is at most a budget's worth of bytes.
    ///
    /// The whole-index candidate is the one thing that could break that bound, because it is
    /// assembled before anything has been compared — so it is assembled only when the bullets
    /// ALONE fit. They are a subset of the block, so bullets that already exceed the budget prove
    /// the block does, and summing their lengths costs no allocation at all. That guard never
    /// decides the answer; it only declines to build a string it would have thrown away. Every
    /// candidate that IS considered is still measured as assembled.
    ///
    /// A workspace whose fixed prose alone exceeds the budget still gets its block: the cut is
    /// there to keep the module speaking, and rendering nothing would be the very outcome it exists
    /// to prevent.
    ///
    /// **Public because the budget is a property of the HOST, and an embedding app is a different
    /// host.** [`PRIME_BLOCK_BUDGET_BYTES`] is derived from the cut-off of the one host this repo
    /// wires a SessionStart hook for; an app composing its own context window has its own number,
    /// and without this it would inherit a limit that has nothing to do with its surface. That is
    /// not a speculative knob: this method's default IS a behaviour change for such a consumer
    /// (`render_markdown` used to render every line, at any size), so the parameter is what keeps
    /// the change something a host can answer rather than only absorb. A host that wants the whole
    /// index and no block around it wants [`index`] instead.
    pub fn render_within(&self, budget: usize) -> String {
        let total = self.memories.len();
        let lines: Vec<String> = self.memories.iter().map(index_line).collect();
        // `len + 1` per line for the newline that joins it to the previous one — one too many for
        // the first, which only makes the guard more conservative, never less.
        let bullet_bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
        if total == 0 || bullet_bytes <= budget {
            let whole = self.assemble(&lines, total);
            if total == 0 || whole.len() <= budget {
                return whole;
            }
        }
        let mut best = self.assemble(&lines, 0);
        for shown in 1..total {
            let candidate = self.assemble(&lines, shown);
            if candidate.len() > budget {
                break;
            }
            best = candidate;
        }
        best
    }

    /// The block with the first `shown` of `lines` in its index, and everything else sized to
    /// match: the plain heading when `shown` is all of them, the excerpt's heading plus
    /// [`more_memories_line`] when it is not.
    ///
    /// `lines` is [`index_line`] over `self.memories`, formatted by the caller so the fill loop
    /// formats each memory once rather than once per candidate.
    fn assemble(&self, lines: &[String], shown: usize) -> String {
        let total = self.memories.len();
        let memories = if total == 0 {
            PRIME_NO_MEMORIES.to_string()
        } else {
            let mut sections: Vec<String> = vec![PRIME_MEMORY_INDEX_RULE.to_string()];
            sections.extend(self.introduction_gap());
            if shown > 0 {
                sections.push(lines[..shown].join("\n"));
            }
            if shown < total {
                sections.push(more_memories_line(total - shown));
            }
            sections.join("\n\n")
        };
        // The migration note is the LAST thing in the block (nxf q065, task 4) — assembled here
        // rather than interpolated as a possibly-empty `{}`, so the closed case leaves no trailing
        // blank gap behind. See `migration_line`'s own doc comment for why it moved.
        let migration = self
            .migration_line()
            .map(|line| format!("\n\n{line}"))
            .unwrap_or_default();
        format!(
            "# nexus-memory — what this project knows\n\n\
             {PRIME_INTRO}\n\n\
             {PRIME_KEEP_THEM_TRUE}\n\n\
             ## How a memory is written\n\n\
             {PRIME_HOW_A_MEMORY_IS_WRITTEN}\n\n\
             {PRIME_INTRODUCTION_HINT}\n\n\
             {}\n\n\
             {memories}{migration}",
            memories_heading(shown, total),
        )
    }
}

/// Render active memories as standalone, headed blocks (nexus-flow-7ca): each memory gets a
/// ``### `<key>` `` heading and its body as its OWN block beneath, with a `---` rule between
/// entries. A flat bullet (the bd-style one-liner format this replaced) collapsed a multi-paragraph
/// body onto the top level and left no boundary between memories; the headed block keeps a long
/// body's internal Markdown intact, and the `---` marks each memory's edge independent of any
/// body-internal heading levels. The order is the caller's. A forgotten/body-less row renders as
/// just its head.
///
/// **The session bootstrap no longer calls this** (nxf 6j6v.xbnh): `prime` renders an index and
/// nothing else. Its readers are the surfaces that WANT the full text — the lower half of
/// `NEXUS_MEMORY.md` ([`crate::project_doc`]), which is the versioned record of what the memories
/// say, and a host composing its own context document (app-foundations).
pub fn render_memory_blocks(rows: &[MemoryRecord]) -> String {
    rows.iter()
        .map(
            |m| match m.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
                Some(body) => format!("### `{}`\n\n{body}", m.key),
                None => format!("### `{}`", m.key),
            },
        )
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// One memory's written introduction, trimmed, or `None` when nobody has written one (6j6v.xbnh).
///
/// One place, because "has an introduction" is asked by the renderer AND by the count above it, and
/// two spellings of it would disagree about a register holding whitespace.
fn introduction_of(m: &MemoryRecord) -> Option<&str> {
    m.introduction
        .as_deref()
        .map(str::trim)
        .filter(|i| !i.is_empty())
}

/// One index line's text, bounded (review of PR #381, Integrity #3).
///
/// **The write path is not the only way a value reaches this register.** `check_introduction`
/// refuses an empty, multi-line or over-long introduction at every LOCAL seam — but a `fact` op
/// arriving from a sync peer is folded on its shape alone (`fact_reducer::is_well_formed` only
/// special-cases `ordinal`, and has since 6j6v.e0z6 for every other register). An older, buggier or
/// corrupted peer can therefore store a 4-KB, three-line "introduction" here, and without this the
/// index would render it verbatim: one memory becoming several bullets, and the size of the
/// session start becoming unbounded again — the two things the whole item exists to prevent.
///
/// So the render bounds what the write refuses: the first line, cut at
/// [`model::INTRODUCTION_MAX_CHARS`]. A locally written introduction can never reach either branch
/// (it was validated), which is what keeps this a defensive floor rather than a second, quieter
/// rule about what an introduction may be. It deliberately does NOT drop the value — a bounded
/// foreign line still says what the memory is, and a peer whose text is merely malformed should not
/// have it silently replaced by "nobody wrote one".
fn bounded(introduction: &str) -> std::borrow::Cow<'_, str> {
    let first = introduction
        .split(['\n', '\r'])
        .next()
        .unwrap_or_default()
        .trim_end();
    if first.chars().count() <= model::INTRODUCTION_MAX_CHARS && first.len() == introduction.len() {
        return std::borrow::Cow::Borrowed(first);
    }
    let cut: String = first
        .chars()
        .take(model::INTRODUCTION_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string();
    std::borrow::Cow::Owned(
        if cut.chars().count() < first.chars().count() || first.len() < introduction.len() {
            format!("{cut}…")
        } else {
            cut
        },
    )
}

/// Render the memories as THE index the session start replays: one bullet per memory, its key and
/// the introduction its author wrote for it, and nothing else (nxf 6j6v.xbnh).
///
/// **Nothing is derived here.** The line is a stored register, validated at the write to one line of
/// at most [`model::INTRODUCTION_MAX_CHARS`] characters, so this renderer has no cut-off, no
/// ellipsis and no rule about where to cut — which is exactly the mechanism it replaces. That one
/// took the body's first line and cut it at 110 characters; on this project's own workspaces it
/// rendered ``- `update-canary-mhjg` — -…`` for 2 of 22 entries, because an agent-written body opens
/// with a bullet. It failed precisely for the memories an agent wrote, and agents write nearly all
/// of them.
///
/// A memory with no introduction yet renders [`PRIME_NO_INTRODUCTION`] — the gap NAMED, because a
/// fallback that renders empty looks like an answer. A body-less (forgotten) row is not special: the
/// index shows introductions, and a tombstone that still carries one still says what it was.
///
/// A value that reached the register from a SYNC PEER rather than through this build's write path
/// is bounded here ([`bounded`]) — the one thing this renderer does decide, and it decides it so
/// that "one memory is one line, and the index is `memories × 200`" stays true of the stored data
/// and not merely of the data this binary wrote.
pub fn render_memory_index(rows: &[MemoryRecord]) -> String {
    rows.iter().map(index_line).collect::<Vec<_>>().join("\n")
}

/// One memory's index bullet — the ONE place the line's shape is decided (nxf 6j6v.5jm3, review of
/// PR #389).
///
/// It exists because [`PrimeReport::render_within`] needs each line's LENGTH before it knows how
/// many of them it will keep, and it must not get that from a second formatter: two spellings of
/// "what a bullet looks like" is precisely how a budget comes to be measured against something
/// other than what is rendered. So the fill loop formats each line exactly once through this, and
/// [`render_memory_index`] — the unbounded form, which `NEXUS_MEMORY.md` and [`index`] use — is a
/// join over the same function.
fn index_line(m: &MemoryRecord) -> String {
    let line = introduction_of(m)
        .map(bounded)
        .unwrap_or(std::borrow::Cow::Borrowed(PRIME_NO_INTRODUCTION));
    format!("- **{}**: {line}", m.key)
}

/// Compute the `nxm prime` session-bootstrap record (spec §5.2) — the bd-prime equivalent: state the
/// memory rule, name the key commands, and replay the memories that hold for this whole workspace.
/// Wired as the SessionStart hook (via the `nxs prime` fan-out), so it loads automatically each
/// session.
///
/// **The retrieval rule applies here** (6j6v.srpg): an `item`-scoped memory that NAMES board items
/// is left OUT, because it reads where a reader is already looking — on those items (`nxf show`,
/// with a hint in `nxf next`). Without that, the session start would grow every ticket-local note
/// ever written and the bootstrap would drown in detail that concerns one item. Everything else is
/// replayed, including a reach a newer peer wrote and an `item`-scoped memory that names nothing
/// (which has no board item to read on), so nothing goes silently missing everywhere.
///
/// Pure over the store: the prose parts are constants, the memories come from the same
/// [`memories`] read every other seam uses. No env, no clock, no filesystem — the caller renders.
pub fn prime(store: &MemoryStore) -> Result<PrimeReport> {
    let memories = replayed_memories(store)?;
    // The migration state is carried only while a move is OPEN (6j6v.9yaj) — a closed one is
    // silence, not a `null` a renderer has to remember to suppress.
    let migration = crate::migration::status(store)?;
    Ok(PrimeReport {
        context_recovery: PRIME_CONTEXT_RECOVERY,
        memory_rule: PRIME_MEMORY_RULE,
        correction_rule: PRIME_CORRECTION_RULE,
        introduction_rule: PRIME_INTRODUCTION_RULE,
        context_document: PRIME_CONTEXT_DOCUMENT,
        commands: PRIME_COMMANDS,
        migration: migration.is_open().then_some(migration),
        memories,
    })
}

/// The memories a session start replays, in the store's reading order — the ONE selection both
/// [`prime`] and [`index`] serve (nxf 6j6v.5jm3).
///
/// It is one function and not two reads because the block's own arithmetic depends on the two
/// agreeing: a cut block says "34 of 80, run `nxm index` for the rest", and that sentence is only
/// true while the verb it names answers over the same 80. Two spellings of "what a session start
/// replays" would drift the moment the retrieval rule gains a case.
///
/// **In reading order, not by key** (6j6v.643z): a memory document is an argument — what the
/// workspace is, then the idea that carries it, then the hard rules — and that argument is exactly
/// what the stored category + position say. Reading it by key would keep the shape stored and
/// unshown, which is what filing was introduced to end.
///
/// **The retrieval rule applies** (6j6v.srpg): an `item`-scoped memory that NAMES board items is
/// left out, because it reads where a reader is already looking — on those items (`nxf show`, with
/// a hint in `nxf next`). Everything else is replayed, including a reach a newer peer wrote and an
/// `item`-scoped memory that names nothing (which has no board item to read on), so nothing goes
/// silently missing everywhere.
fn replayed_memories(store: &MemoryStore) -> Result<Vec<MemoryRecord>> {
    let ordered = MemoryQuery {
        ordered: true,
        ..MemoryQuery::default()
    };
    let mut rows = memories(store, &ordered)?;
    rows.retain(|m| model::replayed_at_session_start(&m.scope, &m.refs));
    Ok(rows)
}

/// The WHOLE index `nxm index` serves (nxf 6j6v.5jm3): every memory a session start replays, one
/// written line each, unbounded in count.
///
/// It is the full form of the index [`PrimeReport`] renders an excerpt of — same memories, same
/// reading order — which is what makes a cut block's "N more not listed" arithmetic true and what
/// that block sends a reader to. Deliberately not a second, wider read: `nxm memories` already
/// lists everything there is, including what reads on a board item, and a verb whose count
/// disagreed with the block's would answer a question nobody asked.
///
/// The intended sequence is index first, then one body: read the lines, decide which memory is
/// worth the tokens, and open THAT one with `nxm recall <key>`. So this is the complete version of
/// the session start's own index, not an emergency exit from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexReport {
    /// The memories, in the store's [reading order](crate::store::MemoryQuery::ordered) — the same
    /// selection and the same sequence [`PrimeReport::memories`] carries.
    pub memories: Vec<MemoryRecord>,
}

impl IndexReport {
    /// The `index` `--json` object: the records themselves plus `count`, mirroring
    /// [`PrimeReport::to_value`] so a host that reads one can read the other.
    pub fn to_value(&self) -> Value {
        json!({ "memories": self.memories, "count": self.memories.len() })
    }

    /// The human view: the index rule, then one line per memory. No trailing newline (the caller's
    /// `println!` supplies it).
    ///
    /// It reuses [`PRIME_MEMORY_INDEX_RULE`] verbatim rather than restating it, because the two
    /// surfaces are the same index at two lengths and the sentence that explains one explains the
    /// other — including where a full text is read (`nxm recall <key>`), which is the whole point
    /// of arriving here.
    pub fn render_markdown(&self) -> String {
        let body = if self.memories.is_empty() {
            PRIME_NO_MEMORIES.to_string()
        } else {
            format!(
                "{PRIME_MEMORY_INDEX_RULE}\n\n{}",
                render_memory_index(&self.memories)
            )
        };
        format!(
            "# nexus-memory — the whole index ({})\n\n{body}",
            self.memories.len()
        )
    }
}

/// Compute the whole memory index (nxf 6j6v.5jm3) — see [`IndexReport`] for what it is for.
///
/// Pure over the store, like [`prime`]: no env, no clock, no filesystem. The caller renders.
pub fn index(store: &MemoryStore) -> Result<IndexReport> {
    Ok(IndexReport {
        memories: replayed_memories(store)?,
    })
}

// ---- the judging migration (6j6v.9yaj) -------------------------------------

/// Build the migration plan for this workspace: the memories still filed as `unsorted`, plus every
/// section of the hand-written context documents under `root`, each carrying the status quo as its
/// decision and the vocabulary as its guide.
///
/// **This proposes material, not judgement.** Nothing here guesses a category — a heuristic dressed
/// as a proposal is exactly the worse-because-artificially-offline outcome the ticket warns against.
/// [`crate::migration::ask_judge`] is where a model is asked, and it runs as a separate process, so
/// the write path this seam sits on keeps linking no network client at all.
pub fn migrate_plan(store: &MemoryStore, root: &std::path::Path) -> Result<MigrationPlan> {
    crate::migration::build_plan(store, root)
}

/// Execute a decided plan: file the memories, write the document sections as memories, keep the
/// plan's sequence as the reading order, move the migrated sections out of their documents, and
/// leave the once-per-stream mark.
///
/// All-or-nothing on the validation: a plan nobody judged (entries still `unsorted`) is refused
/// before a single op is emitted, so "the human decides" cannot decay into a silent mass filing.
pub fn migrate_apply(
    store: &mut MemoryStore,
    now: &str,
    actor: &str,
    root: &std::path::Path,
    plan: &MigrationPlan,
    options: &ApplyOptions,
) -> Result<MigrationReport> {
    let actor = nxs_foundation::model::validate_author(actor)?;
    crate::migration::apply(store, now, actor, root, plan, options)
}

/// This workspace's migration state: how much is still unfiled, and the mark of the run that already
/// happened on this stream (which is what lets a second device ask instead of starting over).
pub fn migrate_status(store: &MemoryStore) -> Result<MigrationStatus> {
    crate::migration::status(store)
}

// ---- writes (now + actor explicit) -----------------------------------------

/// Remember a fact and return its materialized record. `key` is the caller's explicit key, or
/// `None` to mint the content-hash auto-key ([`crate::key::auto_key`]). An empty body or an empty
/// explicit key is rejected (`validation`) before any write — the shared guard every seam enforces.
///
/// **`class.introduction` is MANDATORY here** (nxf 6j6v.xbnh), and it is the one field of a
/// [`Classification`] this verb refuses to leave alone. The session start replays exactly one line
/// per memory and that line is the introduction, so a memory written without one is a memory that
/// reaches every future session as a placeholder. Deriving it from the body was the previous
/// mechanism and it demonstrably produced empty lines; asking for it at the write is the only
/// moment somebody is deciding what the memory says. Enforced HERE rather than in the CLI so the
/// embedding [`Engine`](crate::engine::Engine) and the MCP tools cannot each be a way around it.
///
/// An UPDATE is not exempt: `remember --key <existing>` restates the introduction like any other
/// write. The body changed, so whether the line still describes it is exactly the question, and
/// carrying the old one forward silently is how an index stops matching what it indexes.
///
/// The rest of `class` optionally files the memory as it is written (6j6v.e0z6), leaving the
/// status-quo defaults (`unsorted`, reach `project`, no references) in place. **No model is
/// consulted here.** The write path stays deterministic and offline by construction — the same body
/// and classification always produce the same ops — which is the whole reason ordering is a
/// separate, deliberate verb ([`reorder`]).
pub fn remember(
    store: &mut MemoryStore,
    now: &str,
    actor: &str,
    key: Option<&str>,
    text: &str,
    class: &Classification,
) -> Result<MemoryRecord> {
    let actor = nxs_foundation::model::validate_author(actor)?;
    if text.is_empty() {
        return Err(NxfError::validation("a memory body must not be empty"));
    }
    // An explicit `--key` must be non-empty: an empty key (e.g. an unset shell var) would become an
    // unaddressable `memories` primary key. Reject it rather than silently fall back to an auto-key
    // the caller did not ask for (symmetric with the empty-body guard above).
    if key == Some("") {
        return Err(NxfError::validation("a memory --key must not be empty"));
    }
    if class.introduction.is_none() {
        return Err(NxfError::validation(format!(
            "a memory needs an introduction: pass --introduction \"<one line, at most {} \
             characters>\". It is the ONE line replayed to every session for this memory — the \
             body is read on demand with `nxm recall <key>`. For --category rules the \
             introduction SPEAKS the rule (\"Never X, always Y\"), because nobody looks a \
             prohibition up before breaking it.",
            model::INTRODUCTION_MAX_CHARS
        )));
    }
    let class = class.checked()?;
    let resolved = key.map(str::to_string).unwrap_or_else(|| auto_key(text));
    store.set_wall_clock(now);
    store.remember(&resolved, text, actor);
    class.apply(store, &resolved, actor);
    store
        .recall(&resolved)?
        .map(MemoryRecord::from)
        .ok_or_else(|| NxfError::io("remember did not materialize the memory"))
}

/// File an existing memory: set its category, reach, references and/or introduction **without
/// touching its text** (6j6v.e0z6, extended by 6j6v.xbnh). The written sentence is what a memory IS
/// and stays untouched here; where it is filed, what it points at and the one line that stands for
/// it are separate registers, so `author`/`updated` keep dating the fact.
///
/// This is also the verb that closes the introduction gap on a memory older than the register: it
/// writes the line without rewriting — or even re-reading — the body.
///
/// A `None` field leaves that register alone, so a caller can move one memory into a category
/// without asserting anything about its reach. An empty [`Classification`] is a `validation` error
/// rather than a silent no-op — a caller that meant to change something deserves to hear that it
/// did not. An unknown/forgotten key is `not_found`.
pub fn classify(
    store: &mut MemoryStore,
    now: &str,
    actor: &str,
    key: &str,
    class: &Classification,
) -> Result<MemoryRecord> {
    let actor = nxs_foundation::model::validate_author(actor)?;
    if class.is_empty() {
        return Err(NxfError::validation(
            "classify needs at least one of --category, --scope, --refs or --introduction",
        ));
    }
    let class = class.checked()?;
    if store.recall(key)?.is_none() {
        return Err(NxfError::not_found(format!("no memory '{key}'")));
    }
    store.set_wall_clock(now);
    class.apply(store, key, actor);
    store
        .recall(key)?
        .map(MemoryRecord::from)
        .ok_or_else(|| NxfError::io("classify did not materialize the memory"))
}

/// Store `keys` as the reading order: the first key gets position 1, the second 2, and so on
/// (6j6v.e0z6). Returns the records in the order just written.
///
/// This is the ONE verb that changes order, and it is meant to run rarely and deliberately. Ordering
/// is a **reading** concern while writing happens constantly, so asking anything — a human, a model,
/// a heuristic — at `remember` time would make an everyday write non-deterministic and drag the
/// engine's offline promise along with it. Here the caller has already decided; the engine only
/// records the decision. Whoever produced the sequence (a person, a product, a model the product
/// asked) is outside the engine by design.
///
/// Keys not listed keep whatever position they had, and a memory that has never been reordered
/// sorts after every placed one, in insertion order.
pub fn reorder(
    store: &mut MemoryStore,
    now: &str,
    actor: &str,
    keys: &[String],
) -> Result<Vec<MemoryRecord>> {
    let actor = nxs_foundation::model::validate_author(actor)?;
    if keys.is_empty() {
        return Err(NxfError::validation("reorder needs at least one key"));
    }
    // Reject a repeated key rather than let the last position silently win: two positions for one
    // memory is a mistake in the caller's sequence, not an ordering.
    let mut seen = std::collections::BTreeSet::new();
    if let Some(dup) = keys.iter().find(|k| !seen.insert(k.as_str())) {
        return Err(NxfError::validation(format!(
            "duplicate key '{dup}' in the order"
        )));
    }
    // Check every key BEFORE writing any position — a rejected sequence must leave the order
    // exactly as it was, not partially applied.
    for key in keys {
        if store.recall(key)?.is_none() {
            return Err(NxfError::not_found(format!("no memory '{key}'")));
        }
    }
    store.set_wall_clock(now);
    for (position, key) in keys.iter().enumerate() {
        store.set_ordinal(key, position as i64 + 1, actor);
    }
    keys.iter()
        .map(|key| {
            store
                .recall(key)?
                .map(MemoryRecord::from)
                .ok_or_else(|| NxfError::io("reorder did not materialize the memory"))
        })
        .collect()
}

/// Forget a memory by key (a reversible tombstone) and return the tombstone record (`active=false`,
/// `body=None`). A missing/already-forgotten key is a friendly `not_found` — the existence check is
/// the facade's, the store-level forget stays pure.
pub fn forget(store: &mut MemoryStore, now: &str, actor: &str, key: &str) -> Result<MemoryRecord> {
    let actor = nxs_foundation::model::validate_author(actor)?;
    if store.recall(key)?.is_none() {
        return Err(NxfError::not_found(format!("no memory '{key}'")));
    }
    store.set_wall_clock(now);
    store.forget(key, actor);
    // `get` (not `recall`) so the inactive tombstone row is returned as the record value.
    store
        .get(key)?
        .map(MemoryRecord::from)
        .ok_or_else(|| NxfError::io("forget did not materialize the tombstone"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    fn store() -> MemoryStore {
        MemoryStore::open_in_memory(1)
    }

    /// The stand-in introduction the classification-free cases below carry. `remember` REQUIRES one
    /// (6j6v.xbnh), so "nothing classified" is no longer "an empty [`Classification`]" — the tests
    /// that are about something else say so once, here.
    const INTRO: &str = "a fact one of these tests remembers";

    /// A [`Classification`] carrying only the mandatory introduction.
    fn introduced() -> Classification {
        Classification {
            introduction: Some(INTRO.to_string()),
            ..Classification::default()
        }
    }

    /// `remember` with nothing classified beyond the mandatory introduction — the shape most of
    /// these tests exercise. It shadows [`super::remember`] deliberately, so the
    /// classification-free cases stay readable; the tests that DO classify call `super::remember`
    /// with an explicit [`Classification`].
    fn remember(
        store: &mut MemoryStore,
        now: &str,
        actor: &str,
        key: Option<&str>,
        text: &str,
    ) -> Result<MemoryRecord> {
        super::remember(store, now, actor, key, text, &introduced())
    }

    /// A classification naming just a category (plus the mandatory introduction).
    fn category(name: &str) -> Classification {
        Classification {
            category: Some(name.to_string()),
            ..introduced()
        }
    }

    #[test]
    fn every_write_entry_point_rejects_a_blank_actor_instead_of_panicking() {
        // Review finding Code Quality #1 (PR #311): `Store::emit` asserts a non-blank author, but
        // an assert is the substrate's backstop against an unattributed op — not an input check.
        // A blank actor from an embedding app has to come back as a `validation` Err, never as a
        // panic out of a library call.
        let mut s = store();
        super::remember(&mut s, NOW, "alice", Some("k"), "v", &introduced()).unwrap();

        for blank in ["", "   ", "\t\n"] {
            for (name, kind) in [
                (
                    "remember",
                    super::remember(&mut s, NOW, blank, Some("k2"), "v", &introduced())
                        .unwrap_err()
                        .kind,
                ),
                (
                    "classify",
                    super::classify(&mut s, NOW, blank, "k", &category("c"))
                        .unwrap_err()
                        .kind,
                ),
                (
                    "reorder",
                    super::reorder(&mut s, NOW, blank, &["k".to_string()])
                        .unwrap_err()
                        .kind,
                ),
                (
                    "forget",
                    super::forget(&mut s, NOW, blank, "k").unwrap_err().kind,
                ),
            ] {
                assert_eq!(
                    kind,
                    ErrorKind::Validation,
                    "{name} must reject the blank actor {blank:?}"
                );
            }
        }

        let blank_authored: i64 = s
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM ops WHERE author IS NULL OR trim(author) = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(blank_authored, 0, "no op reached the log unattributed");
    }

    #[test]
    fn recall_returns_the_canonical_record_as_a_value() {
        let mut s = store();
        remember(
            &mut s,
            "2026-06-20T10:00:00Z",
            "alice",
            Some("auth-jwt"),
            "auth uses JWT",
        )
        .unwrap();
        let rec = recall(&s, "auth-jwt").expect("recall finds it");
        assert_eq!(
            rec,
            MemoryRecord {
                key: "auth-jwt".into(),
                body: Some("auth uses JWT".into()),
                author: "alice".into(),
                updated: "2026-06-20T10:00:00Z".into(),
                active: true,
                // Unclassified: the status quo, spelled out (6j6v.e0z6).
                category: model::CATEGORY_UNSORTED.into(),
                scope: Scope::DEFAULT.as_str().into(),
                refs: vec![],
                ordinal: None,
                introduction: Some(INTRO.into()),
            }
        );
    }

    #[test]
    fn recall_of_a_missing_key_is_not_found() {
        let err = recall(&store(), "ghost").expect_err("missing → error");
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert_eq!(err.msg, "no memory 'ghost'");
    }

    #[test]
    fn memories_returns_records_key_sorted_and_searchable() {
        let mut s = store();
        let now = "2026-06-20T10:00:00Z";
        remember(
            &mut s,
            now,
            "alice",
            Some("dolt-phantoms"),
            "Dolt phantom DBs",
        )
        .unwrap();
        remember(&mut s, now, "alice", Some("auth-jwt"), "auth uses JWT").unwrap();

        let keys: Vec<String> = memories(&s, &MemoryQuery::default())
            .unwrap()
            .into_iter()
            .map(|r| r.key)
            .collect();
        assert_eq!(keys, ["auth-jwt", "dolt-phantoms"], "key-sorted records");

        let hits: Vec<String> = memories(&s, &MemoryQuery::search("DOLT"))
            .unwrap()
            .into_iter()
            .map(|r| r.key)
            .collect();
        assert_eq!(
            hits,
            ["dolt-phantoms"],
            "case-insensitive substring over key+body"
        );
    }

    #[test]
    fn remember_returns_the_record_and_stamps_now_and_actor() {
        let mut s = store();
        let rec = remember(&mut s, "2026-06-21T09:00:00Z", "bob", Some("k"), "v").unwrap();
        assert_eq!(rec.key, "k");
        assert_eq!(rec.body.as_deref(), Some("v"));
        assert_eq!(rec.author, "bob", "the explicit actor is recorded");
        assert_eq!(
            rec.updated, "2026-06-21T09:00:00Z",
            "the explicit now is stamped"
        );
        assert!(rec.active);
    }

    #[test]
    fn remember_without_a_key_mints_the_content_hash_auto_key() {
        let mut s = store();
        let rec = remember(
            &mut s,
            "2026-06-20T10:00:00Z",
            "alice",
            None,
            "always run tests with -race",
        )
        .unwrap();
        assert_eq!(rec.key, auto_key("always run tests with -race"));
        assert!(rec.key.starts_with("f-"));
    }

    #[test]
    fn remember_rejects_an_empty_body() {
        let err = remember(&mut store(), "2026-06-20T10:00:00Z", "alice", None, "")
            .expect_err("empty body");
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn remember_rejects_an_empty_explicit_key() {
        let err = remember(
            &mut store(),
            "2026-06-20T10:00:00Z",
            "alice",
            Some(""),
            "some fact",
        )
        .expect_err("empty key");
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn forget_returns_the_tombstone_record() {
        let mut s = store();
        remember(&mut s, "2026-06-20T10:00:00Z", "alice", Some("k"), "secret").unwrap();
        let rec =
            forget(&mut s, "2026-06-22T08:00:00Z", "alice", "k").expect("forget the live memory");
        assert_eq!(rec.key, "k");
        assert_eq!(rec.body, None, "the tombstone has no body");
        assert!(!rec.active, "the tombstone is inactive");
        // And it is gone from the active reads.
        assert!(recall(&s, "k").is_err(), "forgotten → recall is not_found");
        assert!(
            memories(&s, &MemoryQuery::default()).unwrap().is_empty(),
            "forgotten → not listed"
        );
    }

    // ---- classification + order (6j6v.e0z6) ----------------------------------------------------

    const NOW: &str = "2026-08-03T12:00:00Z";

    #[test]
    fn remember_files_the_memory_in_the_same_step() {
        let mut s = store();
        let rec = super::remember(
            &mut s,
            NOW,
            "alice",
            Some("two-levels"),
            "the two board levels are the load-bearing idea",
            &Classification {
                category: Some("architecture".into()),
                scope: Some(Scope::Global),
                refs: Some(vec!["6j6v.e0z6".into(), "6j6v.5gvj".into()]),
                ..introduced()
            },
        )
        .unwrap();
        assert_eq!(rec.category, "architecture");
        assert_eq!(rec.scope, "global");
        assert_eq!(
            rec.refs,
            ["6j6v.5gvj", "6j6v.e0z6"],
            "references are canonical: sorted, deduplicated"
        );
        assert_eq!(rec.ordinal, None, "writing never assigns a position");
    }

    #[test]
    fn an_unclassified_memory_reports_the_status_quo() {
        // What every memory written before 6j6v.e0z6 does — and what the migration therefore sets.
        let rec = remember(&mut store(), NOW, "alice", Some("k"), "v").unwrap();
        assert_eq!(rec.category, model::CATEGORY_UNSORTED);
        assert_eq!(rec.scope, Scope::DEFAULT.as_str());
        assert!(rec.refs.is_empty());
        assert_eq!(rec.ordinal, None);
    }

    #[test]
    fn classify_changes_one_register_and_leaves_the_rest_standing() {
        let mut s = store();
        super::remember(
            &mut s,
            NOW,
            "alice",
            Some("k"),
            "v",
            &Classification {
                category: Some("rules".into()),
                scope: Some(Scope::Global),
                refs: Some(vec!["6j6v.e0z6".into()]),
                ..introduced()
            },
        )
        .unwrap();
        let rec = classify(&mut s, NOW, "bob", "k", &category("architecture")).unwrap();
        assert_eq!(rec.category, "architecture");
        assert_eq!(rec.scope, "global", "an omitted flag is not a reset");
        assert_eq!(rec.refs, ["6j6v.e0z6"]);
        assert_eq!(rec.body.as_deref(), Some("v"), "the text is untouched");
    }

    #[test]
    fn classify_can_clear_the_references() {
        let mut s = store();
        super::remember(
            &mut s,
            NOW,
            "alice",
            Some("k"),
            "v",
            &Classification {
                refs: Some(vec!["6j6v.e0z6".into()]),
                ..introduced()
            },
        )
        .unwrap();
        let rec = classify(
            &mut s,
            NOW,
            "alice",
            "k",
            &Classification {
                refs: Some(vec![]),
                ..Classification::default()
            },
        )
        .unwrap();
        assert!(rec.refs.is_empty(), "an explicit empty set clears them");
    }

    #[test]
    fn a_reference_containing_the_separator_is_rejected_not_silently_split() {
        // PR review, Integrity & Robustness #2. `refs` is stored as ONE comma-joined value, so an
        // entry carrying a comma would be written as one reference and read back as two — silently.
        // The `nxm` CLI cannot produce it (it splits on comma first); any other caller on this seam
        // can, which is why the guard lives here.
        let mut s = store();
        remember(&mut s, NOW, "alice", Some("k"), "v").unwrap();
        let err = classify(
            &mut s,
            NOW,
            "alice",
            "k",
            &Classification {
                refs: Some(vec!["6j6v.e0z6,6j6v.5gvj".into()]),
                ..Classification::default()
            },
        )
        .expect_err("a comma inside one reference");
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("comma"), "{}", err.msg);
        assert!(
            recall(&s, "k").unwrap().refs.is_empty(),
            "and nothing was written"
        );
    }

    #[test]
    fn classify_rejects_a_no_op_an_unknown_key_and_a_malformed_value() {
        let mut s = store();
        remember(&mut s, NOW, "alice", Some("k"), "v").unwrap();

        // Nothing asked for: say so rather than report a successful no-change.
        let err =
            classify(&mut s, NOW, "alice", "k", &Classification::default()).expect_err("empty");
        assert_eq!(err.kind, ErrorKind::Validation);

        let err = classify(&mut s, NOW, "alice", "ghost", &category("rules")).expect_err("unknown");
        assert_eq!(err.kind, ErrorKind::NotFound);

        let err = classify(&mut s, NOW, "alice", "k", &category("Two Words")).expect_err("slug");
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("invalid category"), "{}", err.msg);

        let err = classify(
            &mut s,
            NOW,
            "alice",
            "k",
            &Classification {
                refs: Some(vec!["not an id".into()]),
                ..Classification::default()
            },
        )
        .expect_err("reference shape");
        assert_eq!(err.kind, ErrorKind::Validation);

        // And none of the rejections left a partial write behind.
        let rec = recall(&s, "k").unwrap();
        assert_eq!(rec.category, model::CATEGORY_UNSORTED);
        assert!(rec.refs.is_empty());
    }

    #[test]
    fn a_rejected_classification_on_remember_writes_nothing_at_all() {
        // All-or-nothing: a bad category must not leave the memory itself behind half-written.
        let mut s = store();
        let err = super::remember(&mut s, NOW, "alice", Some("k"), "v", &category("Nope"))
            .expect_err("bad slug");
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(recall(&s, "k").is_err(), "no memory was created");
    }

    #[test]
    fn reorder_stores_the_given_sequence_as_positions() {
        let mut s = store();
        for key in ["rules", "intro", "arch"] {
            remember(&mut s, NOW, "alice", Some(key), "v").unwrap();
        }
        let records = reorder(
            &mut s,
            NOW,
            "alice",
            &["intro".to_string(), "arch".to_string(), "rules".to_string()],
        )
        .unwrap();
        assert_eq!(
            records.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            ["intro", "arch", "rules"],
            "returned in the order just written"
        );
        assert_eq!(
            records.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3)]
        );
        // And it is stored, not merely reported.
        assert_eq!(recall(&s, "arch").unwrap().ordinal, Some(2));
    }

    #[test]
    fn reorder_refuses_an_ill_defined_sequence_without_writing_anything() {
        let mut s = store();
        for key in ["a", "b"] {
            remember(&mut s, NOW, "alice", Some(key), "v").unwrap();
        }
        let err = reorder(&mut s, NOW, "alice", &[]).expect_err("empty");
        assert_eq!(err.kind, ErrorKind::Validation);

        // A repeated key would give one memory two positions — a mistake, not an ordering.
        let err = reorder(&mut s, NOW, "alice", &["a".into(), "a".into()]).expect_err("duplicate");
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("duplicate key 'a'"), "{}", err.msg);

        // An unknown key is checked BEFORE anything is written, so the order is left as it was.
        let err =
            reorder(&mut s, NOW, "alice", &["b".into(), "ghost".into()]).expect_err("unknown");
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert_eq!(
            recall(&s, "b").unwrap().ordinal,
            None,
            "the valid prefix was NOT applied"
        );
    }

    #[test]
    fn reading_order_is_opt_in_and_the_plain_listing_keeps_key_order() {
        let mut s = store();
        for key in ["zeta", "alpha"] {
            remember(&mut s, NOW, "alice", Some(key), "v").unwrap();
        }
        reorder(&mut s, NOW, "alice", &["zeta".to_string()]).unwrap();

        let ordered = MemoryQuery {
            ordered: true,
            ..MemoryQuery::default()
        };
        assert_eq!(
            memories(&s, &ordered)
                .unwrap()
                .iter()
                .map(|r| r.key.clone())
                .collect::<Vec<_>>(),
            ["zeta", "alpha"]
        );
        assert_eq!(
            memories(&s, &MemoryQuery::default())
                .unwrap()
                .iter()
                .map(|r| r.key.clone())
                .collect::<Vec<_>>(),
            ["alpha", "zeta"],
            "the plain listing keeps serving key order — `nxm memories` is a directory, not a \
             document (6j6v.643z moved `prime` onto the reading order, not this)"
        );
    }

    #[test]
    fn classification_moves_the_blocks_in_prime_and_keeps_the_replayed_set_whole() {
        // The claim this test makes was INVERTED by 6j6v.643z, and deliberately so. 6j6v.e0z6 first
        // pinned it as "filing changes NOTHING in prime" — classification added fields, not events
        // — and 6j6v.9yaj narrowed it by the migration line. That was always a description of an
        // unfinished feature: the whole reason categories exist is that a memory document is an
        // argument, and an argument has an ORDER. So filing now moves a memory to where it is read,
        // and what must stay untouched is the SET (nothing appears, nothing vanishes) and the
        // surrounding frame — not the sequence.
        let mut s = store();
        remember(&mut s, NOW, "alice", Some("zeta"), "z").unwrap();
        remember(&mut s, NOW, "alice", Some("alpha"), "a").unwrap();
        let before = prime(&s).unwrap();
        assert_eq!(
            keys(&before.memories),
            ["zeta", "alpha"],
            "unfiled and unplaced: insertion order, the reading order's own tail"
        );

        classify(&mut s, NOW, "alice", "alpha", &category("rules")).unwrap();
        let after = prime(&s).unwrap();

        let mut sorted_before = keys(&before.memories);
        sorted_before.sort_unstable();
        let mut sorted_after = keys(&after.memories);
        sorted_after.sort_unstable();
        assert_eq!(
            sorted_after, sorted_before,
            "the replayed set is the same — filing moves a memory, it never adds or drops one"
        );
        assert_eq!(
            keys(&after.memories),
            ["alpha", "zeta"],
            "…but it MOVED: a filed memory leads the still-unsorted one"
        );
        assert_ne!(
            render_memory_blocks(&after.memories),
            render_memory_blocks(&before.memories),
            "and the session sees that move — the reading position is stored AND shown"
        );
        assert_eq!(
            after.migration.as_ref().map(|m| m.unsorted),
            Some(1),
            "one of the two is filed now"
        );
        assert_eq!(before.migration.as_ref().map(|m| m.unsorted), Some(2));

        // Filing the last one closes the migration, and then `prime` says nothing about it at all.
        classify(&mut s, NOW, "alice", "zeta", &category("rules")).unwrap();
        let done = prime(&s).unwrap();
        assert!(done.migration.is_none(), "silent when none is open");
        assert!(
            !done.render_markdown().contains("Open migration"),
            "{}",
            done.render_markdown()
        );
    }

    #[test]
    fn prime_replays_the_memories_as_the_argument_they_were_filed_into() {
        // 6j6v.643z, the ticket's own acceptance in miniature: what the workspace IS, then the idea
        // that carries it, then the hard rules — then the rest, then whatever nobody filed. Written
        // against `prime` (not the store) because THIS is the surface a session is handed, and the
        // one where the order was stored and never shown.
        let mut s = store();
        for (key, class) in [
            ("later", None),
            ("gate", Some("rules")),
            ("branch", Some("rules")),
            ("crates", Some("architecture")),
            ("what", Some("introduction")),
            ("cut", Some("release-process")),
        ] {
            let class = class.map(category).unwrap_or_else(introduced);
            super::remember(&mut s, NOW, "alice", Some(key), "body", &class).unwrap();
        }
        reorder(
            &mut s,
            NOW,
            "alice",
            &["branch".to_string(), "gate".to_string()],
        )
        .unwrap();

        let report = prime(&s).unwrap();
        assert_eq!(
            keys(&report.memories),
            ["what", "crates", "branch", "gate", "cut", "later"]
        );
        // The rendered document says the same thing — the order is not merely in the struct.
        let doc = report.render_markdown();
        let positions: Vec<usize> = report
            .memories
            .iter()
            .map(|m| {
                doc.find(&format!("**{}**", m.key))
                    .expect("every memory is an index line")
            })
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "the entries appear in reading order:\n{doc}"
        );
    }

    #[test]
    fn one_reorder_across_categories_places_each_memory_within_its_own_section() {
        // PR #291 review, Test Quality #3. `reorder` numbers its whole key list 1..n regardless of
        // category, and this workspace's own migration did exactly that — the judge handed down one
        // global sequence of 43. The ranking then compares those ordinals only WITHIN a category, so
        // the question this pins is whether a global numbering still says the right thing locally:
        // it must preserve the relative order the caller asked for inside each section and never
        // leak a section's numbering into the ranking between sections.
        let mut s = store();
        for (key, class) in [
            ("gate", "rules"),
            ("branch", "rules"),
            ("crates", "architecture"),
            ("spine", "architecture"),
        ] {
            super::remember(&mut s, NOW, "alice", Some(key), "body", &category(class)).unwrap();
        }
        // One call, one global 1..4 — and deliberately leading with the `rules` pair, so a ranking
        // that compared ordinals across categories would put them first.
        let placed = reorder(
            &mut s,
            NOW,
            "alice",
            &[
                "branch".to_string(),
                "gate".to_string(),
                "spine".to_string(),
                "crates".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            placed.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3), Some(4)],
            "the numbering really is global — the premise of this test"
        );

        assert_eq!(
            keys(&prime(&s).unwrap().memories),
            ["spine", "crates", "branch", "gate"],
            "`architecture` still leads `rules` despite carrying the HIGHER ordinals, and inside \
             each section the caller's relative order survives"
        );
    }

    #[test]
    fn two_memories_sharing_an_ordinal_still_read_in_a_stable_order() {
        // PR #291 review, Integrity #1. Two `reorder` calls each number from 1, so two memories in
        // one category CAN end up on the same position — nothing rejects it, and the second caller
        // has no way to know. The tie must therefore resolve, and resolve the same way every time:
        // by insertion order, the reading order's own fallback. An unstable tie would make the
        // SessionStart bytes flap between runs for no visible reason.
        let mut s = store();
        for key in ["beta", "alpha"] {
            super::remember(&mut s, NOW, "alice", Some(key), "body", &category("rules")).unwrap();
        }
        reorder(&mut s, NOW, "alice", &["beta".to_string()]).unwrap();
        reorder(&mut s, NOW, "alice", &["alpha".to_string()]).unwrap();
        assert_eq!(
            (
                recall(&s, "beta").unwrap().ordinal,
                recall(&s, "alpha").unwrap().ordinal
            ),
            (Some(1), Some(1)),
            "the tie is genuinely reachable through the public verb"
        );

        let replayed = |s: &MemoryStore| -> Vec<String> {
            prime(s)
                .unwrap()
                .memories
                .iter()
                .map(|m| m.key.clone())
                .collect()
        };
        assert_eq!(
            replayed(&s),
            ["beta", "alpha"],
            "insertion order breaks the tie — not the alphabet, which would invert this pair"
        );
        // Stable, not merely deterministic-looking: the same log read again, and read again after a
        // refold, says the same thing.
        assert_eq!(replayed(&s), ["beta", "alpha"]);
        s.refold();
        assert_eq!(replayed(&s), ["beta", "alpha"]);
    }

    // ---- the retrieval rule (6j6v.srpg) --------------------------------------------------------

    /// A classification naming just a reach.
    fn reach(scope: Scope) -> Classification {
        Classification {
            scope: Some(scope),
            ..introduced()
        }
    }

    /// A classification filing a memory against board items, item-scoped.
    fn about(ids: &[&str]) -> Classification {
        Classification {
            scope: Some(Scope::Item),
            refs: Some(ids.iter().map(|i| i.to_string()).collect()),
            ..introduced()
        }
    }

    /// One memory per reach, so a test can assert the whole column of the rule's table at once.
    fn three_reaches(s: &mut MemoryStore) {
        super::remember(s, NOW, "alice", Some("ticket"), "t", &about(&["6j6v.srpg"])).unwrap();
        super::remember(s, NOW, "alice", Some("proj"), "p", &reach(Scope::Project)).unwrap();
        super::remember(s, NOW, "alice", Some("glob"), "g", &reach(Scope::Global)).unwrap();
    }

    fn keys(records: &[MemoryRecord]) -> Vec<&str> {
        records.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn the_bootstrap_replays_project_and_global_but_not_item_scoped_memories() {
        // The `nxs prime` column of the rule's table, all three cells at once. The pair is in
        // reading order (nothing here is filed, so: as written), which is what makes the ABSENT
        // third one the assertion rather than the sequence of the other two.
        let mut s = store();
        three_reaches(&mut s);
        assert_eq!(keys(&prime(&s).unwrap().memories), ["proj", "glob"]);
    }

    #[test]
    fn a_board_item_carries_its_item_scoped_memories_and_nothing_else() {
        // The `nxf show <id>` column: the item-scoped memory naming this id, and neither the
        // project nor the global one — those read at the bootstrap instead.
        let mut s = store();
        three_reaches(&mut s);
        let by_item = memories_about(&s, &["6j6v.srpg"]).unwrap();
        assert_eq!(keys(&by_item["6j6v.srpg"]), ["ticket"]);
    }

    #[test]
    fn an_item_with_no_memories_is_absent_from_the_bucket_map() {
        // Absent, not an empty vector: "does this item carry any?" must be one lookup, which is
        // what the `next` hint asks once per row.
        let mut s = store();
        three_reaches(&mut s);
        let by_item = memories_about(&s, &["6j6v.srpg", "6j6v.e0z6"]).unwrap();
        assert_eq!(
            by_item.len(),
            1,
            "only the named item is a key: {by_item:?}"
        );
        assert!(!by_item.contains_key("6j6v.e0z6"));
        assert!(
            memories_about(&s, &[]).unwrap().is_empty(),
            "no ids, no read"
        );
    }

    #[test]
    fn one_memory_reaches_every_item_it_names() {
        // `refs` is a set, so a memory filed against two items shows up on both — and the bucket
        // for each carries the full record, not a reference to a shared one.
        let mut s = store();
        super::remember(
            &mut s,
            NOW,
            "alice",
            Some("pair"),
            "both",
            &about(&["6j6v.srpg", "6j6v.9a1r"]),
        )
        .unwrap();
        let by_item = memories_about(&s, &["6j6v.9a1r", "6j6v.srpg"]).unwrap();
        assert_eq!(keys(&by_item["6j6v.srpg"]), ["pair"]);
        assert_eq!(keys(&by_item["6j6v.9a1r"]), ["pair"]);
    }

    #[test]
    fn an_item_memory_that_names_nothing_is_replayed_instead_of_vanishing() {
        // PR #282 review, Code Quality #1: `--scope item` with no `--refs` is reachable through the
        // seam (each register moves on its own, so it is the first half of the natural two-step),
        // and it appears on no board item. The bootstrap is what keeps it from being invisible
        // everywhere — including for a memory written that way before this rule existed.
        let mut s = store();
        super::remember(
            &mut s,
            NOW,
            "alice",
            Some("halfway"),
            "body",
            &reach(Scope::Item),
        )
        .unwrap();
        assert_eq!(keys(&prime(&s).unwrap().memories), ["halfway"]);
        assert!(memories_about(&s, &["6j6v.srpg"]).unwrap().is_empty());

        // …and naming an item moves it off the bootstrap onto that item, in the same step.
        classify(&mut s, NOW, "alice", "halfway", &about(&["6j6v.srpg"])).unwrap();
        assert!(prime(&s).unwrap().memories.is_empty());
        assert_eq!(
            keys(&memories_about(&s, &["6j6v.srpg"]).unwrap()["6j6v.srpg"]),
            ["halfway"]
        );

        // …and clearing the references again hands it back to the bootstrap rather than dropping it.
        classify(&mut s, NOW, "alice", "halfway", &about(&[])).unwrap();
        assert_eq!(keys(&prime(&s).unwrap().memories), ["halfway"]);
    }

    #[test]
    fn a_forgotten_item_memory_leaves_its_board_item() {
        // The rule rides on the SAME active-only read as every other surface: a tombstone is not
        // a memory a reader is handed.
        let mut s = store();
        super::remember(&mut s, NOW, "alice", Some("ticket"), "t", &about(&["x"])).unwrap();
        forget(&mut s, NOW, "alice", "ticket").unwrap();
        assert!(memories_about(&s, &["x"]).unwrap().is_empty());
    }

    #[test]
    fn a_reach_this_build_does_not_know_still_reaches_the_bootstrap() {
        // Forward compatibility, end to end: a reach a NEWER peer wrote arrives over the wire, is
        // stored opaquely, and is not `item` — so it must not fall out of every surface at once.
        // Written as a foreign op because no local write path can produce it: that is the point.
        let mut s = store();
        remember(&mut s, NOW, "alice", Some("k"), "body").unwrap();
        s.apply(&[nxs_foundation::model::Op {
            op_id: "01J0SCOPE".into(),
            lamport: s.clock() + 1,
            site: 2,
            domain: model::DOMAIN_FACT.into(),
            target_kind: model::FACT_KIND.into(),
            target_id: "k".into(),
            field: model::FIELD_SCOPE.into(),
            op_type: model::OP_SET.into(),
            value: Some("team".into()),
            author: "device-B".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }]);
        assert_eq!(recall(&s, "k").unwrap().scope, "team", "kept verbatim");
        assert_eq!(keys(&prime(&s).unwrap().memories), ["k"]);
        assert!(memories_about(&s, &["x"]).unwrap().is_empty());
    }

    // ---- prime: the lifted session bootstrap (nxf 6j6v.wph0) -----------------------------------

    fn row(key: &str, body: Option<&str>) -> MemoryRecord {
        MemoryRecord {
            key: key.to_string(),
            body: body.map(str::to_string),
            author: "tester".to_string(),
            updated: String::new(),
            active: true,
            category: model::CATEGORY_UNSORTED.to_string(),
            scope: Scope::DEFAULT.as_str().to_string(),
            refs: vec![],
            ordinal: None,
            introduction: Some(format!("what `{key}` is about")),
        }
    }

    #[test]
    fn a_foreign_introduction_is_bounded_at_render_so_the_index_stays_one_line_per_memory() {
        // Review of PR #381, Integrity #3. The write path refuses an empty, multi-line or over-long
        // introduction — but a `fact` op from a sync peer is folded on its SHAPE alone, so a value
        // this build would never have written can still be in the register. Rendering it verbatim
        // would put back both failures the item removed: one memory as several bullets, and an
        // index whose size nothing bounds.
        let mut multi = row("peer-wrote-two-lines", Some("body"));
        multi.introduction = Some("the first line\nand a second one".into());
        let out = render_memory_index(&[multi]);
        assert_eq!(
            out, "- **peer-wrote-two-lines**: the first line…",
            "a multi-line value is cut at the first line, and the ellipsis says something was \
             dropped rather than pretending the line was always this"
        );
        assert_eq!(out.lines().count(), 1, "one memory is one bullet, always");

        let mut long = row("peer-wrote-an-essay", Some("body"));
        long.introduction = Some("x".repeat(model::INTRODUCTION_MAX_CHARS + 500));
        let out = render_memory_index(&[long]);
        assert!(
            out.chars().count() < model::INTRODUCTION_MAX_CHARS + 60,
            "an over-long value is bounded, so `memories × {}` really is the index's ceiling: {out}",
            model::INTRODUCTION_MAX_CHARS
        );
        assert!(out.ends_with('…'), "…and says so: {out}");

        // A locally written line is untouched — this is a defensive floor, not a second rule about
        // what an introduction may be. Exactly at the limit, so an off-by-one would show here.
        let mut exact = row("written-here", Some("body"));
        let at_limit = "y".repeat(model::INTRODUCTION_MAX_CHARS);
        exact.introduction = Some(at_limit.clone());
        assert_eq!(
            render_memory_index(&[exact]),
            format!("- **written-here**: {at_limit}"),
            "no ellipsis, no cut, byte for byte what its author wrote"
        );
    }

    #[test]
    fn renders_a_single_memory_as_a_headed_block() {
        // The bd-style one-liner becomes a headed block: heading from the key, body as its own
        // block beneath. No trailing separator for a lone memory.
        let out = render_memory_blocks(&[row("auth-jwt", Some("auth uses JWT, not sessions"))]);
        assert_eq!(out, "### `auth-jwt`\n\nauth uses JWT, not sessions");
    }

    #[test]
    fn separates_multiple_memories_with_a_horizontal_rule() {
        let out = render_memory_blocks(&[row("a", Some("body a")), row("b", Some("body b"))]);
        assert_eq!(out, "### `a`\n\nbody a\n\n---\n\n### `b`\n\nbody b");
    }

    #[test]
    fn a_multiparagraph_body_keeps_its_internal_structure_and_stays_bounded() {
        // The core 7ca bug: an imported memory is a multi-paragraph doc with its OWN markdown
        // (`##`, `-`). It must render verbatim under its heading, and the `---` must still mark
        // the boundary to the next memory even though the body carries a higher-level `##` heading.
        let imported = "## Background\n\n- decision one\n- decision two\n\nFollow-up prose.";
        let out = render_memory_blocks(&[row("imported", Some(imported)), row("next", Some("n"))]);
        assert_eq!(
            out,
            format!("### `imported`\n\n{imported}\n\n---\n\n### `next`\n\nn"),
            "body keeps its internal markdown; `---` bounds it regardless of its own heading levels"
        );
    }

    #[test]
    fn renders_a_bodyless_memory_as_just_the_heading() {
        assert_eq!(render_memory_blocks(&[row("k", None)]), "### `k`");
        assert_eq!(render_memory_blocks(&[row("k", Some("   "))]), "### `k`");
    }

    // ---- `PrimeCommand::render_markdown`: callerless since `PrimeReport::render_markdown` stopped
    // using it (nxf q065, task 4), tested directly here for the same reason chat's task 3 covers its
    // own copy directly — a public, embedding-host-facing renderer keeps direct coverage even after
    // its one-time caller in `render_markdown` leaves, so a change to it still reddens here rather
    // than only in a golden that no longer exercises it.

    #[test]
    fn prime_command_render_markdown_joins_invocations_and_appends_the_summary() {
        let single = PrimeCommand {
            invocations: &["nxm recall <key>"],
            summary: "read one memory's full text",
        };
        assert_eq!(
            single.render_markdown(),
            "- `nxm recall <key>` — read one memory's full text"
        );
        // No shipped entry pairs two invocations today (see `PRIME_COMMANDS`'s own doc comment),
        // but the renderer still has to join them correctly if one ever does.
        let paired = PrimeCommand {
            invocations: &[
                "nxm classify <key> --scope item --refs <item-id>",
                "nxm forget <key>",
            ],
            summary: "retired pair, kept only to exercise the join",
        };
        assert_eq!(
            paired.render_markdown(),
            "- `nxm classify <key> --scope item --refs <item-id>` / `nxm forget <key>` — retired \
             pair, kept only to exercise the join"
        );
    }

    #[test]
    fn prime_carries_the_rule_the_commands_and_the_memories_as_data() {
        // The point of the lift: everything the SessionStart block says is reachable as DATA from
        // the library — no host has to know the prose to reproduce it.
        let mut s = store();
        remember(&mut s, "2026-06-20T10:00:00Z", "alice", Some("k"), "v").unwrap();
        let report = prime(&s).unwrap();

        assert_eq!(report.memory_rule, PRIME_MEMORY_RULE);
        assert!(
            report.memory_rule.contains("CRITICAL")
                && report.memory_rule.contains("NEVER")
                && report.memory_rule.contains("never replayed"),
            "the rule keeps its emphasis, imperative and consequence (nexus-flow-0f7)"
        );
        assert_eq!(report.context_recovery, PRIME_CONTEXT_RECOVERY);
        assert_eq!(
            report
                .commands
                .iter()
                .flat_map(|c| c.invocations)
                .collect::<Vec<_>>(),
            [
                &"nxm remember \"<fact>\" --introduction \"<one line>\" [--key <key>]",
                &"nxm recall <key>",
                &"nxm memories [<search>]",
                &"nxm classify <key> --scope item --refs <item-id>",
                &"nxm classify <key> --introduction \"<one line>\"",
                &"nxm forget <key>",
            ]
        );
        assert_eq!(
            report.memories,
            memories(
                &s,
                &MemoryQuery {
                    ordered: true,
                    ..MemoryQuery::default()
                }
            )
            .unwrap(),
            "the same records the ordered read serves — `prime` selects, it does not re-shape"
        );
    }

    #[test]
    fn prime_replays_active_memories_in_reading_order_and_drops_forgotten_ones() {
        let mut s = store();
        let now = "2026-06-20T10:00:00Z";
        remember(&mut s, now, "alice", Some("dolt"), "phantoms").unwrap();
        remember(&mut s, now, "alice", Some("auth"), "jwt").unwrap();
        remember(&mut s, now, "alice", Some("gone"), "obsolete").unwrap();
        forget(&mut s, now, "alice", "gone").unwrap();

        let report = prime(&s).unwrap();
        let keys: Vec<&str> = report.memories.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["dolt", "auth"],
            "a workspace that has filed nothing reads in the order it wrote — the reading order's \
             own tail, not the alphabet (6j6v.643z), and the forgotten one is out either way"
        );
    }

    #[test]
    fn prime_explains_why_the_context_document_is_not_imported(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        // nxf 6j6v.sebs. The note exists to answer ONE recurring question at the moment it is
        // asked, so it has to name all three things a reader is holding: the hook that delivered
        // these memories, the file that projects the same store, and `CLAUDE.md`, which does not
        // import it on purpose. A note missing any of the three sends the reader looking again.
        //
        // **Superseded 2026-08-28 (nxf q065, task 4).** The task-4 brief pulled this note out of
        // the human view for the session-start budget — `report.context_document` and
        // `to_value()["context_document"]` still carry it, asserted below, but
        // `render_markdown()` no longer does; see [`PrimeReport::render_markdown`]'s doc comment
        // for the full accounting and [`PRIME_CONTEXT_DOCUMENT`]'s own doc comment for where the
        // explanation still lives.
        //
        // **Corrected 2026-08-28 (nxf n2m6 + a2a1): the hook it names is `nxm prime`.** The list
        // below required "nxs prime", which was the hook that delivered these memories until the
        // wiring became one hook per active module. The three-things requirement above is
        // unchanged — this is still the hook that delivered them, it is just a different command —
        // and it is asserted rather than dropped precisely because naming the WRONG hook would
        // send the reader looking exactly as an absent one would.
        let report = prime(&store())?;
        assert_eq!(report.context_document, PRIME_CONTEXT_DOCUMENT);
        for named in ["nxm prime", "NEXUS_MEMORY.md", "CLAUDE.md"] {
            assert!(
                report.context_document.contains(named),
                "the note has to name {named}: {}",
                report.context_document
            );
        }
        assert_eq!(
            report.to_value()["context_document"],
            PRIME_CONTEXT_DOCUMENT
        );

        // The human view does NOT carry it any more — dropped, not merely reworded, so a substring
        // check would give a false pass if a future edit reworded the constant without noticing
        // the human view still quoted it.
        let md = report.render_markdown();
        assert!(
            !md.contains("This block IS how the project's memory reaches you"),
            "the NEXUS_MEMORY.md/CLAUDE.md explanation left the human view (nxf q065):\n{md}"
        );
        Ok(())
    }

    #[test]
    fn prime_renders_the_empty_workspace_as_an_invitation() {
        let md = prime(&store()).unwrap().render_markdown();
        assert!(md.ends_with(&format!("## Memories (0)\n\n{PRIME_NO_MEMORIES}")));
        assert!(
            !md.ends_with('\n'),
            "no trailing newline — the caller's println! supplies it, so the block composes"
        );
    }

    #[test]
    fn prime_json_and_markdown_project_the_same_record() {
        // The two views may not be two assemblies: whatever the Markdown says, the JSON carries —
        // for the fields the human view still renders. `memory_rule`/`context_recovery` are two of
        // the fields the task-4 cut (nxf q065) kept in `--json` and dropped from `render_markdown`
        // (see that method's doc comment), so THOSE two are asserted on `to_value()` only below,
        // not as markdown substrings any more.
        let mut s = store();
        remember(&mut s, "2026-06-20T10:00:00Z", "alice", Some("k"), "v").unwrap();
        let report = prime(&s).unwrap();
        let v = report.to_value();

        assert_eq!(v["memory_rule"], report.memory_rule);
        assert_eq!(v["context_recovery"], report.context_recovery);
        assert_eq!(v["count"], 1);
        assert_eq!(v["memories"][0]["key"], "k");
        assert_eq!(
            v["commands"][0]["invocations"],
            json!(report.commands[0].invocations)
        );
        assert_eq!(v["commands"][0]["summary"], report.commands[0].summary);

        let md = report.render_markdown();
        assert!(md.contains(PRIME_INTRO), "{md}");
        assert!(md.contains(PRIME_KEEP_THEM_TRUE), "{md}");
        assert!(md.contains("nxm recall <key>"), "{md}");
        // One line per memory, under the sentence that says the block IS an index (6j6v.xbnh).
        assert!(md.contains(PRIME_MEMORY_INDEX_RULE), "{md}");
        assert!(md.contains("## Memories (1)\n\n"), "{md}");
        assert!(md.contains(&format!("- **k**: {INTRO}")), "{md}");
        assert!(!md.contains("\n\nv"), "the body is NOT replayed:\n{md}");
    }

    #[test]
    fn forget_of_an_unknown_key_is_not_found() {
        let err =
            forget(&mut store(), "2026-06-22T08:00:00Z", "alice", "ghost").expect_err("unknown");
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert_eq!(err.msg, "no memory 'ghost'");
    }

    // ---- #76u.13 / jo9: a db error surfaces as `io`, never a panic or a masked not_found ----------

    #[test]
    fn memories_surfaces_a_db_error_as_io_instead_of_panicking() {
        // The primary 76u.13 case: `memories` reads `MemoryStore::memories`, now fallible. Drop the
        // `memories` view out from under it to force a real rusqlite error — the read must map it to
        // the structured `io` kind (so `memory_list`/`memory_search` report a tool error) rather than
        // `.unwrap()` unwinding the long-lived stdio server.
        let s = store();
        s.connection()
            .execute_batch("DROP TABLE memories;")
            .expect("drop the memories view to corrupt the read");
        let err =
            memories(&s, &MemoryQuery::default()).expect_err("a dropped view is a read error");
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "a db read failure maps to io: {err}"
        );
    }

    // ---- the bounded prime index + the whole index (`nxm index`, nxf 6j6v.5jm3) ----------------

    /// A workspace of `n` memories whose introductions each sit at the MAXIMUM a write can store
    /// ([`model::INTRODUCTION_MAX_CHARS`]). Measuring at the maximum makes the result a BOUND and
    /// not a sample: no future memory can push the index further than this, because the write path
    /// refuses a longer line. The same fixture shape `crates/nxs/tests/the_session_start_ceiling.rs`
    /// builds against the real binaries — here in-process, so the arithmetic is testable without a
    /// workspace on disk.
    fn a_workspace_of_worst_case_introductions(n: usize) -> MemoryStore {
        let mut s = store();
        for i in 0..n {
            let head = format!("memory {i:03}: ");
            let introduction = format!(
                "{head}{}",
                "x".repeat(model::INTRODUCTION_MAX_CHARS - head.chars().count())
            );
            super::remember(
                &mut s,
                NOW,
                "alice",
                Some(&format!("memory-{i:03}")),
                "a body nobody reads at session start",
                &Classification {
                    introduction: Some(introduction),
                    // Filed, so the open-migration blockquote stays out of the way of the
                    // arithmetic under test.
                    category: Some("architecture".to_string()),
                    ..Classification::default()
                },
            )
            .unwrap();
        }
        s
    }

    /// The index bullets of a rendered block, in order.
    fn shown_keys(block: &str) -> Vec<&str> {
        block
            .lines()
            .filter_map(|l| l.strip_prefix("- **"))
            .filter_map(|l| l.split("**:").next())
            .collect()
    }

    #[test]
    fn the_prime_index_fills_the_byte_budget_and_then_stops() {
        let s = a_workspace_of_worst_case_introductions(80);
        let block = prime(&s).unwrap().render_markdown();

        assert!(
            block.len() <= PRIME_BLOCK_BUDGET_BYTES,
            "the block is {} bytes against a budget of {PRIME_BLOCK_BUDGET_BYTES}",
            block.len()
        );
        // …and it FILLS it: a bound that stopped far short would keep the block under the host's
        // cut-off while throwing away memories that would have arrived. The slack left here is one
        // bullet's worth at most, and a bullet is ~220 B at this width.
        assert!(
            block.len() + 260 > PRIME_BLOCK_BUDGET_BYTES,
            "the block is only {} bytes of a {PRIME_BLOCK_BUDGET_BYTES} budget — it stops early \
             rather than filling what the host delivers",
            block.len()
        );

        // It is the reading order's PREFIX, not a sample: what a session loses is the tail.
        let shown = shown_keys(&block);
        assert!(
            !shown.is_empty() && shown.len() < 80,
            "some but not all of the 80 are listed, got {}",
            shown.len()
        );
        assert_eq!(shown[0], "memory-000");
        assert_eq!(
            shown[shown.len() - 1],
            format!("memory-{:03}", shown.len() - 1)
        );
    }

    #[test]
    fn a_cut_index_says_it_is_an_excerpt_how_many_there_are_and_where_the_rest_is() {
        // The Definition of Done, as three separate readings a session must be able to make.
        let s = a_workspace_of_worst_case_introductions(80);
        let block = prime(&s).unwrap().render_markdown();
        let shown = shown_keys(&block).len();

        assert!(
            block.contains(&format!("## Memories (showing {shown} of 80)")),
            "the heading names the excerpt and the whole:\n{block}"
        );
        assert!(
            block.contains(&format!("{} more not listed", 80 - shown)),
            "the cut is named with its size:\n{block}"
        );
        assert!(
            block.contains("`nxm index`"),
            "the verb that serves the rest is named:\n{block}"
        );
    }

    #[test]
    fn an_index_that_fits_is_rendered_whole_with_no_cut_to_announce() {
        // The state every real workspace is in today (nexus-flow's own 50 memories measure 8.763 B
        // against a 9.216 B budget), and the one a reader must be able to tell apart from a cut one.
        let s = a_workspace_of_worst_case_introductions(12);
        let block = prime(&s).unwrap().render_markdown();

        assert_eq!(shown_keys(&block).len(), 12);
        assert!(block.contains("## Memories (12)"), "{block}");
        assert!(!block.contains("showing"), "nothing to announce:\n{block}");
        assert!(!block.contains("more not listed"), "{block}");
        assert!(!block.contains("`nxm index`"), "{block}");
    }

    #[test]
    fn the_cut_is_a_rendering_decision_and_json_still_carries_every_memory() {
        // The same asymmetry 6j6v.xbnh established for the bodies: what the host truncates is the
        // Markdown, so an embedding host reading `--json` keeps the whole set and renders its own.
        let s = a_workspace_of_worst_case_introductions(80);
        let report = prime(&s).unwrap();
        assert!(shown_keys(&report.render_markdown()).len() < 80);

        let value = report.to_value();
        assert_eq!(value["count"], 80);
        assert_eq!(value["memories"].as_array().unwrap().len(), 80);
    }

    #[test]
    fn a_candidate_that_measures_exactly_the_budget_is_kept_and_one_byte_more_is_not() {
        // Review of PR #389, Test Quality #1. The fill loop's comparison is `>`, and until this
        // case existed nothing said so: the covering test asserted only that the result lands
        // NEAR the budget, so flipping `>` to `>=` — a block one whole memory shorter than it had
        // to be, at every session start — stayed green.
        //
        // Pinned without a hand-computed byte count, which would be a second arithmetic that could
        // disagree with the render: render once against a budget that truncates, take the length
        // the renderer itself produced, and hand THAT back as the budget. A candidate whose length
        // equals the budget must survive.
        let s = a_workspace_of_worst_case_introductions(80);
        let report = prime(&s).unwrap();

        let cut = report.render_within(4 * 1024);
        let exactly = cut.len();
        assert!(
            shown_keys(&cut).len() < 80,
            "the fixture has to truncate for this to say anything"
        );

        let at_the_boundary = report.render_within(exactly);
        assert_eq!(
            at_the_boundary.len(),
            exactly,
            "a candidate that measures exactly the budget fits it"
        );
        assert_eq!(shown_keys(&at_the_boundary).len(), shown_keys(&cut).len());

        // …and one byte less is one bullet less, so the boundary is a real edge and not a plateau.
        let just_under = report.render_within(exactly - 1);
        assert_eq!(
            shown_keys(&just_under).len(),
            shown_keys(&cut).len() - 1,
            "one byte under the exact length drops the last bullet"
        );

        // The SAME edge on the untruncated side, which is where the whole-index guard decides. The
        // guard measures the bullets and skips assembling the whole block when they alone exceed
        // the budget; it overcounts by one newline on purpose, so this pins that the overcount can
        // never cost a workspace a block that would have fitted exactly.
        let whole = report.render_within(usize::MAX);
        assert_eq!(shown_keys(&whole).len(), 80);
        assert_eq!(
            report.render_within(whole.len()),
            whole,
            "a block that measures exactly its budget is not a truncation"
        );
        assert!(
            shown_keys(&report.render_within(whole.len() - 1)).len() < 80,
            "and one byte under it is"
        );
    }

    #[test]
    fn a_budget_smaller_than_the_fixed_prose_still_yields_a_block() {
        // Review of PR #389, Test Quality #2. `render_within`'s doc comment promises this in so
        // many words — "a workspace whose fixed prose alone exceeds the budget still gets its
        // block" — and nothing drove it. The promise is the whole point of the cut: it exists to
        // keep the module speaking, so a budget it cannot meet must not turn into silence.
        let s = a_workspace_of_worst_case_introductions(80);
        let block = prime(&s).unwrap().render_within(10);

        assert!(block.starts_with("# nexus-memory"), "{block}");
        assert!(
            block.len() > 10,
            "the floor is honoured over the budget, not the other way round"
        );
        assert!(
            shown_keys(&block).is_empty(),
            "no bullet could fit:\n{block}"
        );
        // And it is HONEST about it rather than looking like an empty workspace: the heading and
        // the note still say how many there are and where they are.
        assert!(block.contains("## Memories (showing 0 of 80)"), "{block}");
        assert!(block.contains("80 more not listed"), "{block}");
        assert!(block.contains("`nxm index`"), "{block}");
    }

    #[test]
    fn a_host_with_its_own_limit_renders_the_block_against_that_limit() {
        // The escape hatch that keeps the default a change a consumer can answer rather than only
        // absorb: `PRIME_BLOCK_BUDGET_BYTES` is derived from ONE host's cut-off, and an embedding
        // app composing its own context window is a different host with a different number.
        let s = a_workspace_of_worst_case_introductions(80);
        let report = prime(&s).unwrap();

        let generous = report.render_within(64 * 1024);
        assert_eq!(
            shown_keys(&generous).len(),
            80,
            "a roomier host loses nothing"
        );
        assert!(!generous.contains("more not listed"), "{generous}");

        let tight = report.render_within(4 * 1024);
        assert!(tight.len() <= 4 * 1024, "{} bytes", tight.len());
        assert!(
            shown_keys(&tight).len() < shown_keys(&report.render_markdown()).len(),
            "a tighter host loses more than the default one does"
        );
    }

    #[test]
    fn the_whole_index_is_the_same_selection_as_prime_with_nothing_left_out() {
        // `nxm index` is the FULL form of the block's index — same memories, same reading order,
        // unbounded in count — which is what makes the block's "N more not listed" arithmetic true
        // and what the block sends a reader to.
        let s = a_workspace_of_worst_case_introductions(80);
        let index = index(&s).unwrap();
        assert_eq!(keys(&index.memories), keys(&prime(&s).unwrap().memories));

        let rendered = index.render_markdown();
        assert_eq!(shown_keys(&rendered).len(), 80);
        assert!(
            rendered.len() > PRIME_BLOCK_BUDGET_BYTES,
            "the whole index is deliberately unbounded — it is read on demand, not at session start"
        );
        assert!(
            !rendered.contains("body nobody reads"),
            "an index is keys and lines, never bodies:\n{rendered}"
        );

        let value = index.to_value();
        assert_eq!(value["count"], 80);
        assert_eq!(value["memories"].as_array().unwrap().len(), 80);
    }

    #[test]
    fn the_whole_index_of_an_empty_workspace_invites_a_first_memory() {
        let rendered = index(&store()).unwrap().render_markdown();
        assert!(rendered.contains(PRIME_NO_MEMORIES), "{rendered}");
    }

    #[test]
    fn the_whole_index_leaves_out_what_reads_on_a_board_item() {
        // The retrieval rule (6j6v.srpg) decides the SELECTION, and `index` is the full form of
        // that selection — not a second, wider read. An item-scoped memory that names an item reads
        // on the item; listing it here would make the block's count disagree with this verb's.
        let mut s = store();
        super::remember(
            &mut s,
            NOW,
            "alice",
            Some("on-the-item"),
            "about one ticket",
            &Classification {
                introduction: Some("a line about one ticket".to_string()),
                scope: Some(Scope::Item),
                refs: Some(vec!["ab12.0007".to_string()]),
                ..Classification::default()
            },
        )
        .unwrap();
        super::remember(
            &mut s,
            NOW,
            "alice",
            Some("workspace-wide"),
            "about the workspace",
            &introduced(),
        )
        .unwrap();

        assert_eq!(keys(&index(&s).unwrap().memories), ["workspace-wide"]);
    }

    #[test]
    fn recall_surfaces_a_db_error_as_io_not_a_masked_not_found() {
        // jo9: `recall`/`store.get` USED to `.ok()`-swallow a db error into `None` → a misleading
        // `not_found`. Now the error is distinguished — a genuine db failure is `io`, while only a
        // truly absent key is `not_found`. Drop the view to force the db-error branch.
        let s = store();
        s.connection()
            .execute_batch("DROP TABLE memories;")
            .expect("drop the memories view");
        let err = recall(&s, "auth-jwt").expect_err("a dropped view is a read error");
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "a db error is io, not a masked not_found: {err}"
        );
    }
}

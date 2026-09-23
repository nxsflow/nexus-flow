//! memory's `fact` vocabulary (spec §2.1) + the `memories` view row shape. The substrate op shape
//! is domain-agnostic and owned by the foundation ([`nxs_foundation::model::Op`]); this module owns
//! only the constants that tag a fact op and the materialized row a read returns.

/// The reducer domain for memory facts — the axis the registry dispatches on (spec §4.1).
pub const DOMAIN_FACT: &str = "fact";
/// Every fact op targets a `fact` entity (`target_kind`).
pub const FACT_KIND: &str = "fact";
/// The LWW register field carrying the memory's text (spec §3): `body`. This is the register the
/// memory IS — `author`/`updated`/`active` move with it.
pub const FACT_FIELD: &str = "body";
/// The register carrying the memory's **written introduction** (6j6v.xbnh) — the one line
/// `nxs prime` replays for it. Its own register, like the classification ones: rewriting the
/// introduction must not disturb the body, and vice versa.
pub const FIELD_INTRODUCTION: &str = "introduction";
/// The classification register naming which section a memory belongs to (6j6v.e0z6).
pub const FIELD_CATEGORY: &str = "category";
/// The classification register naming how far a memory reaches (6j6v.e0z6).
pub const FIELD_SCOPE: &str = "scope";
/// The classification register naming the board items a memory is about (6j6v.e0z6). Its value is
/// the canonical (trimmed, deduplicated, sorted) comma-joined list — see [`normalize_refs`].
pub const FIELD_REFS: &str = "refs";
/// The register carrying an explicit reading position within a category (6j6v.e0z6). Written ONLY
/// by the deliberate `reorder` verb; unset means "no explicit position, sort by insertion order".
pub const FIELD_ORDINAL: &str = "ordinal";
/// The field the judging migration's mark rides on (6j6v.9yaj). Deliberately NOT a register column:
/// the fact reducer answers no such field, so the op is **store-don't-fold** — durable in the log,
/// carried over the sync wire to every device on the stream, and never a row in `memories`. That is
/// what makes the mark visible to a second device without making it visible to a reader.
pub const FIELD_MIGRATION: &str = "migration";
/// `remember` op type: set the body, mark the memory active. Also the op type of every
/// classification register write (`category`/`scope`/`refs`/`ordinal`).
pub const OP_SET: &str = "set";
/// `forget` op type: clear the body (NULL) and mark the memory inactive — a reversible tombstone.
pub const OP_FORGET: &str = "forget";

/// The auto-key prefix for system-minted (content-hash) keys (spec §2.2). Explicit `--key` values
/// are used verbatim (bd parity); only auto-keys carry this fact-domain prefix.
pub const AUTO_KEY_PREFIX: &str = "f-";

/// **The introduction's hard limit, in characters** (6j6v.xbnh): 200.
///
/// It is a limit on the WRITER, not a truncation on the reader, and that asymmetry is the whole
/// design. The session bootstrap replays exactly one line per memory, so the size of what a session
/// is handed is `memories × this number` — a quantity nobody can drift past by accident. Cutting an
/// over-long line at render time would have produced the same bytes and taught nobody anything;
/// refusing it at the write reaches the one moment somebody is deciding what the memory SAYS.
///
/// 200 is also the argument the number carries: a rule that cannot be said in 200 characters is not
/// a rule but an essay with a rule inside it. Counted in CHARACTERS, not bytes, because the writer
/// counts characters — an umlaut is not two thirds of an allowance.
pub const INTRODUCTION_MAX_CHARS: usize = 200;

/// Whether `introduction` is a well-formed introduction, or the reason it is not (6j6v.xbnh).
///
/// Three conditions, and each answers a way the ONE replayed line goes wrong: empty says nothing,
/// a newline turns one memory into two bullets, and over-length is the drift the limit exists to
/// stop. The error names the MEASURED length, because "too long" without a number leaves the writer
/// counting characters by hand.
pub fn check_introduction(introduction: &str) -> Result<(), String> {
    let trimmed = introduction.trim();
    if trimmed.is_empty() {
        return Err(
            "an introduction must not be empty: it is the ONE line a session start is handed for \
             this memory"
                .to_string(),
        );
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(
            "an introduction is ONE line: it renders as a single bullet in the session start, so \
             it carries no line breaks"
                .to_string(),
        );
    }
    let measured = trimmed.chars().count();
    if measured > INTRODUCTION_MAX_CHARS {
        return Err(format!(
            "this introduction is {measured} characters and the limit is \
             {INTRODUCTION_MAX_CHARS}: it is replayed to every session, so it has to say the \
             thing rather than announce it. Put the detail in the body — `nxm recall <key>` is \
             what fetches that."
        ));
    }
    Ok(())
}

/// The reserved category every memory carries until something classifies it (6j6v.e0z6). It is the
/// migration default precisely because it is **countable**: `nxm memories --category unsorted` is
/// the signal the later judging migration (6j6v.9yaj) works from.
pub const CATEGORY_UNSORTED: &str = "unsorted";

/// The categories the structure ships with. NOT a closed set — a category is any slug
/// ([`is_valid_category`]), so a consumer can add its own; these are the ones a generated document
/// is expected to lead with, in this order.
pub const CONVENTIONAL_CATEGORIES: &[&str] = &["introduction", "architecture", "rules"];

/// How far a memory reaches — the "Reichweite" register (6j6v.e0z6).
///
/// The engine stores and addresses the reach; **deciding** which memory deserves which reach is the
/// product's job (the epic's IP boundary), so nothing here judges. [`Scope::DEFAULT`] is
/// [`Scope::Project`] because that is exactly what every memory written before this existed already
/// did: `nxs prime` replays it for this workspace. The default therefore describes the status quo
/// rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Reaches only the board items the memory names (`refs`) — a ticket/epic-local fact.
    Item,
    /// Reaches this workspace. The default, and today's behaviour for every existing memory.
    Project,
    /// Reaches beyond this workspace — a fact that holds wherever the agent works.
    Global,
}

impl Scope {
    /// The reach a memory has when nobody said otherwise: [`Scope::Project`].
    pub const DEFAULT: Scope = Scope::Project;

    /// Every reach, in widening order — the order a chooser or a `--help` list should show.
    pub const ALL: &'static [Scope] = &[Scope::Item, Scope::Project, Scope::Global];

    /// The wire/storage spelling — the exact `value` a `scope` op carries.
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Item => "item",
            Scope::Project => "project",
            Scope::Global => "global",
        }
    }

    /// Parse a stored/typed reach, or `None` when the word is not one this build knows. A value a
    /// NEWER peer wrote is deliberately kept as an opaque string in the view (never rejected on
    /// read) — only the write path parses, so an unknown reach survives a round-trip instead of
    /// being dropped.
    pub fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.iter().copied().find(|v| v.as_str() == s)
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **The retrieval rule** (6j6v.srpg), half one: whether a memory with this stored reach is
/// replayed by the session bootstrap (`nxs prime`).
///
/// The reach decides **where and how deep** a memory surfaces — one deterministic rule, a total
/// function of the stored register. No embedding, no similarity measure, no network:
///
/// | reach     | `nxf next`        | `nxf show <id>` | `nxs prime` |
/// |-----------|-------------------|-----------------|-------------|
/// | `item`    | a hint that there are some | **full text** | — |
/// | `project` | —                 | —               | **full text** |
/// | `global`  | —                 | —               | **full text** |
///
/// It takes the STORED spelling rather than a [`Scope`] on purpose: a reach a NEWER peer wrote is
/// kept opaque on read, and it must still reach a reader.
///
/// **The rule withholds a memory from the bootstrap only when it has somewhere else to appear.**
/// That is why `refs` is a parameter and not an afterthought: an `item`-scoped memory that names no
/// board item appears on no board item either ([`reaches_item`] needs a named id), so withholding it
/// here would make it invisible EVERYWHERE. It is replayed instead. This is not a courtesy — an
/// invisible memory is a silent data-loss bug, and `--scope item` before `--refs …` is the natural
/// two-step of an API whose registers deliberately move one at a time. Rejecting that write instead
/// would break the incremental classification the seam promises, and would leave any memory already
/// written that way (possible since 6j6v.e0z6) stranded; making the read total fixes both at once.
///
/// The one case the engine still cannot see is a `refs` entry naming an item that does not exist:
/// memory validates the SHAPE of a reference and never its existence (spec §4.4, the fact reducer
/// knows no flow vocabulary), so a typo'd id is a memory filed against nothing. That is the
/// deliberate product boundary, not an oversight here.
pub fn replayed_at_session_start(stored_scope: &str, refs: &[String]) -> bool {
    stored_scope != Scope::Item.as_str() || refs.is_empty()
}

/// **The retrieval rule** (6j6v.srpg), half two: whether a memory with this stored reach and these
/// references belongs to the board item `id` — the `nxf show <id>` full text and the `nxf next`
/// hint of the table in [`replayed_at_session_start`].
///
/// Membership is exact and shallow: the memory must be `item`-scoped AND name `id` in its `refs`.
/// It deliberately does NOT walk the board — a memory about an epic does not leak onto that epic's
/// children — because the engine addresses reach, and traversal would be a judgement about what a
/// reader deserves. A `project`/`global` memory that happens to name an item stays out: its reach
/// says it belongs to the whole workspace, and the bootstrap is where the whole workspace reads.
pub fn reaches_item(stored_scope: &str, refs: &[String], id: &str) -> bool {
    stored_scope == Scope::Item.as_str() && refs.iter().any(|r| r == id)
}

/// Whether `c` is a well-formed category slug: `[a-z0-9]` first, then `[a-z0-9_-]`. The category
/// names a section of a generated document, so it stays a stable, lower-case, separator-free handle
/// — which is what keeps a section anchor deterministic. Extensibility is intact: any slug is a
/// category, [`CONVENTIONAL_CATEGORIES`] are only the ones the structure leads with.
pub fn is_valid_category(c: &str) -> bool {
    let mut chars = c.chars();
    match chars.next() {
        None => false,
        Some(first) if !first.is_ascii_lowercase() && !first.is_ascii_digit() => false,
        Some(_) => {
            chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        }
    }
}

/// Canonicalize a set of board-item references into the single string the `refs` register stores:
/// trimmed, empty entries dropped, deduplicated, sorted. Sorting is what makes the register
/// **canonical** — two writers naming the same items in different orders converge on the same value
/// instead of fighting over an LWW register forever.
///
/// memory validates the SHAPE of a reference, never its existence: the fact reducer knows no flow
/// vocabulary (spec §4.4), so a `refs` entry is an opaque handle here.
pub fn normalize_refs<S: AsRef<str>>(refs: &[S]) -> Vec<String> {
    let mut out: Vec<String> = refs
        .iter()
        .map(|r| r.as_ref().trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Split the stored `refs` register value back into its entries. The inverse of joining
/// [`normalize_refs`] with `,`; an empty/absent value is no references at all.
pub fn split_refs(stored: &str) -> Vec<String> {
    stored
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect()
}

/// One materialized memory — a row of the `memories` view (spec §4). `body` is `None` when the
/// memory's winning op was a `forget` (it is then `active == false` and hidden from reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    /// The stable entity key (the op `target_id`).
    pub key: String,
    /// The memory's text, or `None` if it was forgotten.
    pub body: Option<String>,
    /// The author of the memory's latest mutation.
    pub author: String,
    /// The display-only timestamp of the latest mutation (the op `wall_clock`; may be empty).
    pub updated: String,
    /// Whether the memory is currently remembered (`true`) or forgotten (`false`).
    pub active: bool,
    /// Which section this memory belongs to — [`CATEGORY_UNSORTED`] until classified. Kept as a
    /// string, not a parsed enum: categories are extensible, and a category a newer peer wrote must
    /// survive a round-trip through this build.
    pub category: String,
    /// How far the memory reaches ([`Scope`]), as stored. A string for the same forward-compatible
    /// reason as `category`; [`Scope::parse`] turns a known one into the enum.
    pub scope: String,
    /// The board items this memory is about — canonical (sorted, deduplicated), possibly empty.
    pub refs: Vec<String>,
    /// The explicit reading position, or `None` when the memory has never been reordered (it then
    /// sorts by insertion order, after every explicitly placed sibling).
    pub ordinal: Option<i64>,
    /// The written introduction — the one line the session bootstrap replays (6j6v.xbnh), or `None`
    /// when nobody has written one yet. `None` is a real state and not a placeholder for the empty
    /// string: every memory written before this register existed is in it, and the bootstrap
    /// renders it as a NAMED gap rather than deriving a stand-in.
    pub introduction: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_round_trips_through_its_stored_spelling() {
        for s in Scope::ALL {
            assert_eq!(Scope::parse(s.as_str()), Some(*s), "{s} round-trips");
        }
        assert_eq!(
            Scope::DEFAULT,
            Scope::Project,
            "project is today's behaviour"
        );
        assert_eq!(
            Scope::parse("workspace"),
            None,
            "an unknown reach is not guessed"
        );
    }

    #[test]
    fn the_bootstrap_replays_every_reach_except_a_filed_item_memory() {
        let filed = vec!["6j6v.srpg".to_string()];
        assert!(!replayed_at_session_start(Scope::Item.as_str(), &filed));
        assert!(replayed_at_session_start(Scope::Project.as_str(), &filed));
        assert!(replayed_at_session_start(Scope::Global.as_str(), &filed));
        // A reach a NEWER peer wrote is unknown here, and must NOT vanish from every surface:
        // `item` is withheld because it appears on its board items instead — nothing else is.
        assert!(
            replayed_at_session_start("team", &filed),
            "an unknown reach still reaches a reader"
        );
    }

    #[test]
    fn an_item_memory_that_names_no_item_is_replayed_rather_than_lost() {
        // The rule withholds only what has somewhere else to appear. `--scope item` with no `refs`
        // appears on no board item, so withholding it here would make it invisible EVERYWHERE —
        // and `--scope item` before `--refs …` is the natural two-step of a seam whose registers
        // move one at a time.
        assert!(
            replayed_at_session_start(Scope::Item.as_str(), &[]),
            "an item memory naming no item still reaches a reader"
        );
        assert!(
            !reaches_item(Scope::Item.as_str(), &[], "6j6v.srpg"),
            "…and it is on no board item, which is exactly why"
        );
    }

    #[test]
    fn a_board_item_carries_only_the_item_scoped_memories_that_name_it() {
        let refs = vec!["6j6v.5gvj".to_string(), "6j6v.srpg".to_string()];
        assert!(reaches_item(Scope::Item.as_str(), &refs, "6j6v.srpg"));
        assert!(
            !reaches_item(Scope::Item.as_str(), &refs, "6j6v.e0z6"),
            "an item it does not name"
        );
        // Reach decides the surface: a project/global memory belongs to the whole workspace and
        // reads at the bootstrap, even when it happens to name an item.
        assert!(!reaches_item(Scope::Project.as_str(), &refs, "6j6v.srpg"));
        assert!(!reaches_item(Scope::Global.as_str(), &refs, "6j6v.srpg"));
        assert!(
            !reaches_item(Scope::Item.as_str(), &[], "6j6v.srpg"),
            "item-scoped with no references reaches nothing"
        );
    }

    #[test]
    fn the_rule_is_total_and_its_two_halves_never_overlap() {
        // The table's cells partition: every memory surfaces on EXACTLY one of them — never
        // both (that would be noise) and never neither (that would be silent data loss). Proven
        // over every reach this build knows plus an unknown one, and over both the filed and the
        // unfiled `refs` case, so neither a later reach nor a half-classified memory can slip
        // through a hole.
        let filed = vec!["6j6v.srpg".to_string()];
        let unfiled: Vec<String> = vec![];
        for scope in Scope::ALL
            .iter()
            .map(|s| s.as_str())
            .chain(std::iter::once("team"))
        {
            for refs in [&filed, &unfiled] {
                let bootstrap = replayed_at_session_start(scope, refs);
                let on_item = reaches_item(scope, refs, "6j6v.srpg");
                assert!(
                    bootstrap != on_item,
                    "{scope} with refs {refs:?}: bootstrap={bootstrap}, on_item={on_item} — a \
                     memory must read on exactly one surface"
                );
            }
        }
    }

    #[test]
    fn a_category_is_a_lower_case_slug() {
        for ok in ["rules", "architecture", "adr-0007", "ci_gates", "0mq"] {
            assert!(is_valid_category(ok), "{ok} is a slug");
        }
        for bad in ["", "Rules", "two words", "trailing ", "-lead", "emoji✨"] {
            assert!(!is_valid_category(bad), "{bad:?} is not a slug");
        }
    }

    #[test]
    fn refs_normalize_to_a_canonical_sorted_set() {
        // Order and duplication must not survive: the register is one LWW value, so two writers
        // naming the same items in different orders have to land on the same bytes.
        assert_eq!(
            normalize_refs(&["6j6v.e0z6", " 6j6v.5gvj ", "6j6v.e0z6", "  "]),
            ["6j6v.5gvj", "6j6v.e0z6"]
        );
        assert!(normalize_refs::<&str>(&[]).is_empty());
    }

    #[test]
    fn refs_split_back_out_of_the_stored_value() {
        assert_eq!(
            split_refs("6j6v.5gvj,6j6v.e0z6"),
            ["6j6v.5gvj", "6j6v.e0z6"]
        );
        assert!(split_refs("").is_empty(), "no references at all");
        assert_eq!(
            split_refs(&normalize_refs(&["b", "a"]).join(",")),
            ["a", "b"],
            "join/split are inverse"
        );
    }
}

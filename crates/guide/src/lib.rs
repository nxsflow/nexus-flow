//! The ONE embedded-guide mechanism, shared by the three building blocks (nexus-flow-6j6v.9e3r).
//!
//! `nxf guide` (E7-S3) was the only narrative-docs surface in the suite, and its machinery —
//! `include_dir!` embed, `TOPICS` list, `--json` contract, terminal markdown renderer, drift
//! guard — lived inside `crates/cli`. The docs layer now shows the suite as the THREE building
//! blocks it is, each with its own guides, so that machinery is needed three times. Copying it
//! three times would put the `--json` contract of `nxf guide`, `nxm guide` and `nxc guide` on
//! three separate drift trajectories; it lives here once instead, and each product supplies only
//! the two things that are genuinely its own: its embedded directory and its [`Catalog::topics`].
//!
//! **Why the product crate still owns the `include_dir!` call.** The macro expands against the
//! INVOKING crate's `$CARGO_MANIFEST_DIR`, so `crates/memory/docs/guide/en` can only be embedded
//! from `crates/memory`. This crate therefore takes a `&'static Dir` rather than a path, and each
//! product pairs it with a `build.rs` `rerun-if-changed` line — `include_dir!` emits no rerun
//! directive of its own, so an ADDED or REMOVED topic would otherwise not rebuild (nexus-flow-e1qn).
//!
//! Output is **pipe-safe**: rendering never consults `IsTerminal`, so piped output is
//! byte-for-byte identical to interactive output — the agent contract is determinism.

use include_dir::Dir;
use nxs_foundation::error::{NxfError, Result};
use std::iter::Peekable;
use std::str::Chars;

/// One building block's guide collection: the binary that serves it, its ordered topic list, and
/// the directory embedded into the binary at compile time.
///
/// A catalog with NO topics is legitimate and must answer cleanly, not panic (6j6v.9e3r): a
/// block's guide CONTENT is its own ticket and the mechanism lands first — chat until `6j6v.t6vd`,
/// memory until `6j6v.h4k0`, and whichever block gets a guide tree next. "Serves its own topics" is
/// satisfied by an honest empty answer — an empty `--json` array and a line that points at the
/// suite listing — and every path below is written for it. All three shipped catalogs are populated
/// today, which is why the empty shape is pinned on this crate's own fixture below rather than on
/// whichever product happens to be empty.
pub struct Catalog {
    /// The binary a reader types to reach these guides (`nxf`), used in the listing header, the
    /// empty-state hint, and the unknown-topic error. Never inferred — a routed
    /// `nxs flow guide` must read as `nxf`, exactly like every other persona-aware message.
    pub binary: &'static str,
    /// The single ordered source of truth for `(topic, one-line summary)`. The order IS the agent
    /// contract for `<binary> guide --json`; do not reorder without intent. Held against the
    /// embedded directory by [`assert_catalog_parity`], which every product's test suite runs.
    pub topics: &'static [(&'static str, &'static str)],
    /// The embedded EN guide content. EN-only in the binary: the DE tree beside it is website-only
    /// (the federated docs at `/open-source/docs` carry both locales) and compiles into nothing.
    pub embedded: &'static Dir<'static>,
}

impl Catalog {
    /// Does this block ship any guides yet? The one branch every empty-state path reads.
    pub fn is_empty(&self) -> bool {
        self.topics.is_empty()
    }

    /// The `[{topic, summary}]` listing as JSON, in fixed [`Catalog::topics`] order. Extracted from
    /// the printing path because `nxs guide`'s fan-out re-serves exactly these records under its
    /// own envelope — one shape, produced once.
    pub fn topics_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.topics
                .iter()
                .map(|(topic, summary)| serde_json::json!({ "topic": topic, "summary": summary }))
                .collect(),
        )
    }

    /// Resolve a topic to its raw embedded markdown, or a `validation` error listing the valid
    /// topics (in fixed order) — the same shape every other command uses for bad input.
    pub fn lookup(&self, topic: &str) -> Result<&'static str> {
        if self.is_empty() {
            return Err(NxfError::validation(format!(
                "`{bin}` has no guide topics yet; run `nxs guide` to see the suite's",
                bin = self.binary
            )));
        }
        if !self.topics.iter().any(|(t, _)| *t == topic) {
            let valid: Vec<&str> = self.topics.iter().map(|(t, _)| *t).collect();
            return Err(NxfError::validation(format!(
                "unknown guide topic '{topic}'; expected one of {}",
                valid.join(", ")
            )));
        }
        self.embedded
            .get_file(format!("{topic}.md"))
            .and_then(|f| f.contents_utf8())
            // A listed topic with no embedded file is the exact drift `assert_catalog_parity`
            // forbids; if it ever happens at runtime it is a build defect, surfaced loudly not
            // silently.
            .ok_or_else(|| {
                NxfError::io(format!(
                    "embedded guide '{topic}.md' is missing or non-UTF-8"
                ))
            })
    }
}

/// `<binary> guide [topic]`: list topics (no arg) or print one topic's embedded markdown.
///
/// The `--json` contract is FROZEN and identical for all three binaries (6j6v.9e3r DoD): the
/// listing is `[{"summary":…,"topic":…}]` and one topic is `{"content":…,"topic":…}`, exactly the
/// bytes `nxf guide --json` has always emitted. `nxs guide` wraps these records rather than
/// reshaping them, so an agent that learned the shape from one binary knows it everywhere.
pub fn run(catalog: &Catalog, json: bool, topic: Option<&str>) -> Result<()> {
    match topic {
        None => list_topics(catalog, json),
        Some(t) => show_topic(catalog, json, t),
    }
}

/// No topic: list every topic with its one-line summary (human) or a deterministic
/// `[{topic, summary}]` array (json), always in the fixed [`Catalog::topics`] order.
///
/// An empty catalog prints one honest line and an empty JSON array — never an error. A reader who
/// types `nxm guide` before memory's guides are written gets an answer and a next step, and an
/// agent parsing `--json` gets a well-formed empty list rather than a non-zero exit to special-case.
fn list_topics(catalog: &Catalog, json: bool) -> Result<()> {
    if json {
        println!("{}", catalog.topics_json());
        return Ok(());
    }
    if catalog.is_empty() {
        println!(
            "`{bin}` has no guides yet — run `nxs guide` to see the suite's.",
            bin = catalog.binary
        );
        return Ok(());
    }
    println!("Guides (run `{} guide <topic>`):\n", catalog.binary);
    let width = catalog
        .topics
        .iter()
        .map(|(t, _)| t.len())
        .max()
        .unwrap_or(0);
    for (topic, summary) in catalog.topics {
        println!("  {topic:<width$}  {summary}");
    }
    Ok(())
}

/// A topic: print its embedded markdown. With `--json`, emit `{topic, content}` carrying
/// the raw markdown (agents render it themselves); otherwise render simple terminal markdown.
fn show_topic(catalog: &Catalog, json: bool, topic: &str) -> Result<()> {
    let content = catalog.lookup(topic)?;
    if json {
        let out = serde_json::json!({ "topic": topic, "content": content });
        println!("{out}");
    } else {
        print!("{}", render_markdown(content));
    }
    Ok(())
}

/// The drift guard every product's test suite runs against its own catalog: the embedded dir and
/// the `topics` list must agree exactly.
///
/// This is the "drift = build/test failure" principle — adding a `.md` without listing it (or vice
/// versa) fails here rather than shipping a half-wired guide. It lives in the library, not in a
/// `#[cfg(test)]` block, precisely so all three products assert the SAME rule instead of each
/// re-deriving it; the mirror on the website side is `assertTopicParity` in `content/lib/docs.mjs`,
/// which additionally holds the DE tree.
///
/// # Panics
/// With a legible diff of the two sets when they disagree.
pub fn assert_catalog_parity(catalog: &Catalog) {
    use std::collections::BTreeSet;

    let listed: BTreeSet<String> = catalog
        .topics
        .iter()
        .map(|(t, _)| format!("{t}.md"))
        .collect();
    let embedded: BTreeSet<String> = catalog
        .embedded
        .files()
        .filter_map(|f| f.path().file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();

    assert_eq!(
        listed, embedded,
        "`{}`: the topics list and the embedded docs/guide/en/*.md must match exactly",
        catalog.binary
    );
}

/// Render simple terminal markdown: headings, fenced code blocks, and bullet lists. This
/// is intentionally minimal (no ANSI, no wrapping) so output is pipe-safe and deterministic
/// — the same bytes whether piped or shown in a terminal.
pub fn render_markdown(src: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in src.lines() {
        // HTML comments (e.g. the TODO marker) are authoring metadata, not user content.
        if !in_code && line.trim_start().starts_with("<!--") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("```") {
            in_code = !in_code;
            // Drop the fence line itself but indent the block so it reads as code.
            let _ = rest; // language hint is irrelevant to plain-terminal output
            continue;
        }
        if in_code {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push_str(&render_line(line));
        out.push('\n');
    }
    out
}

/// Render one non-code markdown line: strip heading hashes, normalize list bullets, and
/// drop inline emphasis markers so the terminal text is clean.
fn render_line(line: &str) -> String {
    let trimmed = line.trim_end();
    if let Some(heading) = trimmed.strip_prefix("# ") {
        return strip_inline(heading).to_uppercase();
    }
    if let Some(heading) = trimmed.strip_prefix("## ") {
        return strip_inline(heading);
    }
    if let Some(heading) = trimmed.strip_prefix("### ") {
        return strip_inline(heading);
    }
    if let Some(item) = trimmed.strip_prefix("- ") {
        return format!("  - {}", strip_inline(item));
    }
    strip_inline(trimmed)
}

/// The `[text](target)` tail shared by a link and an image, with the leading `[` already eaten.
///
/// `Pair` carries the link text. `Verbatim` means the run was NOT a well-formed pair — no
/// closing `]`, or a `(` that never closes — and it carries exactly what belongs on the output
/// in place of everything after that leading `[`, with the text portion itself rendered.
/// Buffering rather than discarding is what lets a malformed run come out verbatim instead of
/// silently swallowing the rest of the line; the caller prepends its own opener (`[` or `![`).
enum LinkTail {
    /// A well-formed `[text](target)`. The target is parsed so it can be dropped deliberately
    /// rather than by accident; no caller renders it.
    Pair { text: String },
    /// Not a link at all: exactly the text to emit after the caller's own opener.
    Verbatim(String),
}

fn take_link_tail(chars: &mut Peekable<Chars<'_>>, depth: u32) -> LinkTail {
    let mut text = String::new();
    let mut closed = false;
    for inner in chars.by_ref() {
        if inner == ']' {
            closed = true;
            break;
        }
        text.push(inner);
    }
    if !closed {
        return LinkTail::Verbatim(strip_inline_at(&text, depth + 1));
    }
    if chars.peek() != Some(&'(') {
        return LinkTail::Verbatim(format!("{}]", strip_inline_at(&text, depth + 1)));
    }
    chars.next(); // consume '('
    let mut target = String::new();
    for inner in chars.by_ref() {
        if inner == ')' {
            return LinkTail::Pair { text };
        }
        target.push(inner);
    }
    LinkTail::Verbatim(format!("{}]({}", strip_inline_at(&text, depth + 1), target))
}

/// Render inline markdown for the terminal: drop `*` emphasis markers, keep inline-code
/// content **verbatim** (only the surrounding backticks go), render `[text](target)` links as
/// just their `text`, and leave `_` intact. Keeping `_` and code-span contents is what stops
/// real doc text — `belongs_to`, `--json`, `a*b` globs inside code — from being mangled (the
/// gap flagged in the PR #41 review). The `--json` output and the website keep the raw
/// markdown (links and all); this is only the plain-terminal rendering. Full inline parsing is
/// out of scope; this is the minimum that survives v1 content.
///
/// Note what dropping the target means for the slug scheme (6j6v.9e3r): a cross-topic link is
/// written `[core concepts](nxf-core-concepts)` and the TERMINAL never sees the target at all, so
/// renaming slugs cannot break CLI output. The website is the surface where a target matters, and
/// `checkCrossLinks` in `content/lib/docs.mjs` is what holds it.
fn strip_inline(s: &str) -> String {
    strip_inline_at(s, 0)
}

/// How many nested re-renders [`strip_inline_at`] performs before it stops rendering and hands the
/// rest of the line back as it stands (nxf 6j6v.1p3g).
///
/// Every re-entry here is a stack frame, and one unpaired `[` buys one: a run of them recursed once
/// per bracket, so the depth was bounded by the LENGTH OF THE INPUT and by nothing else. That was
/// survivable only because of who calls this — guide pages are `include_dir!`-compiled, so every
/// byte reaching it was written and reviewed in this repository. The signature `&str -> String`
/// promises no such thing to the next caller, and that is what this constant answers.
///
/// **Eight is headroom, not a guess.** Nesting in the shipped guide and develop pages was counted:
/// all 395 runs of `[` across them are a single bracket. Real markdown nests a level or two — a
/// link whose text carries a bracketed aside — so anything reachable by writing prose is far below
/// this, and for every page this repository ships the output is byte-identical either way.
const MAX_INLINE_DEPTH: u32 = 8;

/// [`strip_inline`]'s body, carrying how deep the re-rendering already is.
///
/// At [`MAX_INLINE_DEPTH`] it stops and returns `s` unrendered. That is a real change of output —
/// emphasis markers and code spans below the limit survive as themselves — and it is the honest
/// one: the alternative for a line that deep is not better output, it is no output and a dead
/// process.
fn strip_inline_at(s: &str, depth: u32) -> String {
    if depth >= MAX_INLINE_DEPTH {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // Inline code: copy the span verbatim, dropping only the backtick delimiters. An
            // unterminated backtick degrades gracefully (it just drops that one delimiter).
            '`' => {
                for inner in chars.by_ref() {
                    if inner == '`' {
                        break;
                    }
                    out.push(inner);
                }
            }
            // Emphasis marker (`*bold*`): drop. We never use `_` for emphasis, so it stays
            // literal — that is exactly what protects snake_case identifiers and flags.
            '*' => {}
            // Image `![alt](target)`: the terminal has no picture to draw, so render the alt
            // text in a marker and drop the target. Without this arm the `!` falls through to
            // the literal case below and the line comes out as `!alt` (6j6v.jepw) — neither the
            // picture nor a sentence. The marker deliberately invents NO url: an image's target
            // is the ASSET path, not a page a reader could open, so the page that carries one
            // says in prose where the rendered picture lives.
            '!' => {
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    match take_link_tail(&mut chars, depth) {
                        LinkTail::Pair { text: alt } => {
                            out.push_str("[image: ");
                            out.push_str(&strip_inline_at(&alt, depth + 1));
                            out.push(']');
                        }
                        LinkTail::Verbatim(verbatim) => {
                            out.push_str("![");
                            out.push_str(&verbatim);
                        }
                    }
                } else {
                    out.push('!');
                }
            }
            // Link `[text](target)`: keep just the (recursively rendered) text, drop the
            // target. Anything that is not a well-formed link is emitted literally.
            '[' => match take_link_tail(&mut chars, depth) {
                LinkTail::Pair { text } => out.push_str(&strip_inline_at(&text, depth + 1)),
                LinkTail::Verbatim(verbatim) => {
                    out.push('[');
                    out.push_str(&verbatim);
                }
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use include_dir::{include_dir, Dir};

    /// This crate's own fixture tree — two topics, so the catalog logic (listing order, lookup,
    /// unknown-topic error) is exercised here rather than against any one product's real guides.
    static FIXTURE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/tests/fixture/en");
    /// A deliberately EMPTY tree: the zero-topics shape a block ships between its mechanism and
    /// its content (chat until `6j6v.t6vd`, memory until `6j6v.h4k0`). Held here so "empty answers
    /// cleanly" is a tested rule of the mechanism — no shipped product is empty any more, and the
    /// rule must not go untested because of it.
    static EMPTY: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/tests/fixture/empty");

    fn filled() -> Catalog {
        Catalog {
            binary: "nxq",
            topics: &[("alpha", "The first."), ("beta", "The second.")],
            embedded: &FIXTURE,
        }
    }

    fn empty() -> Catalog {
        Catalog {
            binary: "nxq",
            topics: &[],
            embedded: &EMPTY,
        }
    }

    #[test]
    fn a_filled_catalog_is_in_sync_with_its_embedded_dir() {
        assert_catalog_parity(&filled());
    }

    #[test]
    fn an_empty_catalog_is_in_sync_with_an_empty_dir() {
        // The zero-topics wrinkle at the parity guard: `.gitkeep` is not a `.md`, so both sides are
        // the empty set and the guard passes rather than needing an exemption.
        assert_catalog_parity(&empty());
    }

    #[test]
    fn topics_json_is_the_frozen_listing_shape_in_declared_order() {
        assert_eq!(
            filled().topics_json().to_string(),
            r#"[{"summary":"The first.","topic":"alpha"},{"summary":"The second.","topic":"beta"}]"#
        );
    }

    #[test]
    fn an_empty_catalog_lists_as_an_empty_json_array_not_an_error() {
        // The agent-facing half of the zero-topics answer: a well-formed empty list, so a caller
        // never has to special-case a non-zero exit for a block whose guides are not written yet.
        assert_eq!(empty().topics_json().to_string(), "[]");
        assert!(list_topics(&empty(), true).is_ok());
        assert!(list_topics(&empty(), false).is_ok());
    }

    #[test]
    fn every_listed_topic_resolves_to_real_content() {
        for (topic, summary) in filled().topics {
            assert!(!summary.is_empty(), "{topic} has a summary");
            let content = filled().lookup(topic).expect("topic resolves");
            assert!(content.contains("# "), "{topic} has a top heading");
        }
    }

    #[test]
    fn unknown_topic_is_validation_error_listing_topics() {
        let err = filled().lookup("does-not-exist").unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        for (topic, _) in filled().topics {
            assert!(err.msg.contains(topic), "error lists {topic}: {}", err.msg);
        }
    }

    #[test]
    fn asking_an_empty_catalog_for_a_topic_names_the_suite_listing() {
        // Not a panic and not a bare "unknown topic" with an empty expected-list: the honest
        // answer is that this block has none YET, plus where the suite's are.
        let err = empty().lookup("getting-started").unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        assert!(err.msg.contains("no guide topics yet"), "{}", err.msg);
        assert!(err.msg.contains("nxs guide"), "{}", err.msg);
    }

    #[test]
    fn strip_inline_preserves_code_spans_and_identifiers() {
        // Inline code keeps its content verbatim (only backticks removed).
        assert_eq!(
            strip_inline("run `nxf next --json` now"),
            "run nxf next --json now"
        );
        // snake_case / flags survive both inside and outside code spans (`_` is literal).
        assert_eq!(
            strip_inline("the `belongs_to` field"),
            "the belongs_to field"
        );
        assert_eq!(
            strip_inline("a bare belongs_to stays"),
            "a bare belongs_to stays"
        );
        // `*` emphasis is stripped outside code, italic (`*x*`) and bold (`**x**`) alike —
        // each `*` is dropped independently, so `**bold**` (shipped in the guides) renders clean.
        assert_eq!(strip_inline("a *bold* word"), "a bold word");
        assert_eq!(strip_inline("a **bold** word"), "a bold word");
        // … but preserved inside a code span (e.g. a glob).
        assert_eq!(strip_inline("glob `a*b`"), "glob a*b");
        // Markdown links render as their text; the target is dropped — including the namespaced
        // slugs the docs layer now uses (6j6v.9e3r), which is why the rename is invisible here.
        assert_eq!(
            strip_inline("see [core](nxf-core-concepts) now"),
            "see core now"
        );
        assert_eq!(
            strip_inline("see [the `next` cmd](nxf-commands)"),
            "see the next cmd"
        );
        // A lone bracket pair without a target is kept literally.
        assert_eq!(strip_inline("a [bracketed] word"), "a [bracketed] word");
        // A malformed link (no closing paren) is emitted verbatim — it must NOT swallow the
        // rest of the line (regression guard for the bounded target scan).
        assert_eq!(
            strip_inline("see [text](commands and more"),
            "see [text](commands and more"
        );
        // The third malformed shape: no closing `]` AT ALL. `take_link_tail` reaches the end of
        // the input with `closed` still false, and the whole run must come back verbatim. Left
        // uncovered when the parser became shared between links and images (6j6v.jepw review,
        // Test Quality #1) — the two OTHER malformed shapes were tested, this one was not.
        assert_eq!(
            strip_inline("see [text and no bracket"),
            "see [text and no bracket"
        );
        // Verbatim does not mean unrendered: the text portion still goes through `strip_inline`,
        // which is what the `Verbatim` doc promises and what keeps emphasis out of the fallback.
        assert_eq!(strip_inline("see [*text* and on"), "see [text and on");
    }

    /// An image is the one inline form the terminal cannot honour: there is no picture to draw.
    /// It must not come out as `!alt` — the shape `strip_inline` produced before this arm existed
    /// (6j6v.jepw), which reads as neither the picture nor a sentence. Note what the marker does
    /// NOT do: it invents no URL. The target of an image is the ASSET path, not a page a reader
    /// could open, so the page that carries one says in PROSE where the rendered picture lives.
    #[test]
    fn strip_inline_renders_an_image_as_a_marked_alt_text() {
        assert_eq!(
            strip_inline("![the architecture at a glance](/nxs/docs/assets/x.svg)"),
            "[image: the architecture at a glance]"
        );
        // The alt text is rendered like any other inline run — code spans and emphasis included.
        assert_eq!(
            strip_inline("![the `nxs` map](/a/b.svg)"),
            "[image: the nxs map]"
        );
        // An image inside a sentence keeps everything around it.
        assert_eq!(
            strip_inline("see ![map](/a.svg) above"),
            "see [image: map] above"
        );
        // A `!` that begins no image stays literal — it is ordinary punctuation in prose.
        assert_eq!(strip_inline("stop! now"), "stop! now");
        assert_eq!(strip_inline("wow!"), "wow!");
        // A malformed image degrades verbatim, exactly as a malformed link does: it must not
        // swallow the rest of the line.
        assert_eq!(
            strip_inline("see ![alt](unclosed and more"),
            "see ![alt](unclosed and more"
        );
        assert_eq!(strip_inline("a ![bracketed] word"), "a ![bracketed] word");
        // …and the same "no closing `]` at all" shape through the IMAGE opener, which must
        // restore both characters it consumed, not just the bracket (6j6v.jepw review, TQ #1).
        assert_eq!(
            strip_inline("see ![alt and no bracket"),
            "see ![alt and no bracket"
        );
    }

    /// What [`MAX_INLINE_DEPTH`] COSTS, so the limit is a documented behaviour and not a silent
    /// one: past it a line stops being rendered rather than being rendered differently. The pair is
    /// the point — one bracket below the limit proves the rendering is still fully alive there, so
    /// a future edit that lowered the limit into reach of real prose would fail here.
    #[test]
    fn past_the_nesting_limit_the_rest_of_the_line_is_handed_back_unrendered() {
        let limit = MAX_INLINE_DEPTH as usize;
        // One short of the limit: the tail is rendered as ever, emphasis markers and all.
        let below = "[".repeat(limit - 1);
        assert_eq!(
            strip_inline(&format!("{below}*bold*")),
            format!("{below}bold")
        );
        // At the limit: the tail comes back exactly as it stands, `*` included.
        let at = "[".repeat(limit);
        assert_eq!(strip_inline(&format!("{at}*bold*")), format!("{at}*bold*"));
    }

    /// nxf 6j6v.1p3g: one unpaired `[` cost one stack frame, so a line of them cost one frame
    /// each — `strip_inline` recursed into the remainder of the line for every bracket that never
    /// closed. Nothing in the signature `&str -> String` tells a future caller that its input has
    /// to be short; that the only text reaching it today is compiled into this binary is a fact
    /// about the CALLERS, not a property of this function.
    #[test]
    fn a_long_run_of_unpaired_brackets_does_not_exhaust_the_stack() {
        let deep = "[".repeat(100_000);
        assert_eq!(strip_inline(&deep), deep);
    }

    #[test]
    fn render_strips_fences_headings_and_comments() {
        let md = "<!-- TODO -->\n# Title\nText *bold*.\n- item\n```bash\nnxf init\n```\n";
        let out = render_markdown(md);
        assert!(!out.contains("<!--"), "comments dropped");
        assert!(!out.contains("```"), "fences dropped");
        assert!(out.contains("TITLE"), "h1 upcased");
        assert!(out.contains("Text bold."), "emphasis stripped");
        assert!(out.contains("    nxf init"), "code indented");
    }
}

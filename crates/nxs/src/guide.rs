//! `nxs guide` fan-out (nexus-flow-6j6v.9e3r) — the suite-wide view of the three building blocks'
//! guides, modelled on [`crate::prime`].
//!
//! The docs layer stopped pretending the suite is one product: `nxf`, `nxm` and `nxc` each serve
//! their OWN topics (`nxs-guide`), and this is the umbrella's reading of all of them at once. Like
//! `prime` it hardcodes no roster — it shells out to each module's registered `binary` (TB-5) and
//! composes what comes back — so a fourth product appears here purely by registering its
//! [`ModuleInit`].
//!
//! # Two deliberate differences from `prime`
//!
//! **No shared `now`, and no `--db`.** A guide is static content compiled into the binary; there is
//! no clock to pin and no store to open, so the children are run bare. `--db` is still accepted on
//! the umbrella (it is global) and still used to RESOLVE which modules are active — it just never
//! reaches a child.
//!
//! **A missing workspace is not an error here.** `prime` is a session hook and legitimately
//! requires a workspace; `guide` is what a person reads BEFORE they have one — `getting-started`
//! is addressed to exactly that reader. So the fan-out resolves the workspace's active modules when
//! there is a workspace that declares any, and otherwise falls back to the FULL registered roster
//! rather than refusing to answer. Nothing is guessed either way: with a workspace the answer is
//! the workspace's, and the fallback is the honest superset of "everything this binary can serve".
//! The resolution is deliberately READ-ONLY — unlike `prime` it never back-fills
//! `active_modules`, because reading a manual must not write to the user's config.
//!
//! # The umbrella carries guides of its OWN (6j6v.0fvt)
//!
//! This module was a pure fan-out until the umbrella became a building block in its own right.
//! First setup, the workspace on disk, and the map of which block does what belong to no single
//! product — a reader needs them BEFORE they know that `nxf`, `nxm` and `nxc` exist — so `nxs`
//! now owns [`TOPICS`] and an embedded tree like every other block, and `nxs guide` is
//! "its own topics, then the modules'" rather than pure fan-out.
//!
//! **Two consequences, both deliberate.**
//!
//! The `--json` SHAPE is untouched — the umbrella is one more entry in `modules`
//! (`{"module":"nxs","binary":"nxs","topics":[…]}`) — so a FIELD-KEYED consumer, one that looks an
//! entry up by `binary`/`module`, is unaffected. Say that precisely rather than "still parses",
//! because the ORDER did change in a way it never had before: `modules[0]` was flow for the entire
//! life of this endpoint, not by luck but by construction — `sort_roster` orders by
//! `ModuleInit.order` and flow carries the lowest (10), so every previous arrival (memory 20,
//! chat 30) was a pure append that moved nothing. The umbrella is `insert`ed at 0 in
//! `cli::guide_cmd`, deliberately AHEAD of that order-sorted roster, because it is not a module
//! and has no `order` to be sorted by. A consumer keyed on POSITION therefore breaks. That was
//! never a documented contract — but it was never falsified either, which is why the move is
//! called out in the release notes instead of being waved through as compatible.
//!
//! The AMBIGUITY rule gets a precedence rule rather than an exception. `getting-started` is
//! carried by four blocks now, and a bare `nxs guide getting-started` answering "that name is
//! ambiguous" would be absurd: the umbrella's IS the suite's first setup, the one the others point
//! at. So [`GuideFanOut::resolve`] gives the umbrella precedence when it carries the name, and
//! leaves the error path exactly as it was for every topic it does not carry — `commands` still
//! names `nxf guide commands` / `nxm guide commands` / `nxc guide commands` and picks none of them.

use crate::error::{NxfError, Result};
use crate::registry::{self, ModuleInit};
use crate::spawn;
use include_dir::{include_dir, Dir};
use nxs_foundation::error::ErrorKind;
use nxs_foundation::workspace::Workspace;
use nxs_guide::Catalog;
use std::path::Path;

/// The umbrella's own registry key AND binary — it is both, being neither a module nor reached
/// through one. The one place this string is written; every precedence test below reads it.
pub const UMBRELLA: &str = "nxs";

/// The umbrella's embedded EN guide content. EN-only in the binary, exactly like the three blocks':
/// the DE tree beside it is website-only and compiles into nothing.
static GUIDE_EN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/docs/guide/en");

/// The single ordered source of truth for the umbrella's `(topic, one-line summary)`. The order is
/// the reading order and the agent contract for `nxs guide --json`; do not reorder without intent.
///
/// Three topics, and the cut is the DoD's: `getting-started` is the ONE document a reader who just
/// installed needs (install, `nxs init`, what it creates, what restores context, which blocks
/// exist), and the other two are the elaborations it points at. Everything product-specific stays
/// in the product's own guide — the umbrella owns no domain.
///
/// What is NOT here is the second tree below. This list serves someone who wants to USE the suite;
/// [`DEVELOP_TOPICS`] serves someone who wants to build ON it (6j6v.jepw). Splitting them is the
/// whole point: one flat list mixing first-install with the op-log's internals would serve the
/// first reader worse with every page the second one gains.
const TOPICS: &[(&str, &str)] = &[
    (
        "getting-started",
        "Install the suite, set up a workspace, and see what restores an agent's context.",
    ),
    (
        "modules",
        "What flow, memory and chat are each for, and which one to reach for.",
    ),
    (
        "the-workspace",
        "What `nxs init` leaves on disk, what is committed, and how to check it is healthy.",
    ),
];

/// The develop tree's embedded EN content — a SECOND `include_dir!` in this crate, not a second
/// crate. `include_dir!` can only expand from the crate that owns the path, and the umbrella
/// binary is what serves both.
static DEVELOP_EN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/docs/develop/en");

/// The registry key for the develop group. It is a MODULE key with no binary of its own: `nxs` is
/// what serves it, so the group is a fifth entry in the listing while `nxs guide <topic>` keeps
/// working unchanged. That is deliberate — the split is about AUDIENCE, not about a new persona.
pub const DEVELOP: &str = "develop";

/// Topics for people building ON nexus-flow rather than using it: contributors and the authors of
/// embedding apps. Ordered as a reading order, like every other catalog here.
const DEVELOP_TOPICS: &[(&str, &str)] = &[
    (
        "architecture",
        "The map: five layers, which way the arrows point, and which seams are contracts.",
    ),
    (
        "the-journey-of-one-op",
        "One `nxf close`, followed from the verb to a derived answer nobody wrote.",
    ),
    (
        "a-reply-across-two-machines",
        "The same trip over two replicas: what an arriving op carries, and what stays behind.",
    ),
];

/// The develop group's catalog. Same `binary` as the umbrella's — one binary, two catalogs — which
/// is exactly why [`topic_names_do_not_collide_across_the_two_nxs_catalogs`] exists: the ambiguity
/// error the fan-out raises for a topic in two BLOCKS ("ask one of them") has no meaningful form
/// when both answers would be the same command.
pub const DEVELOP_CATALOG: Catalog = Catalog {
    binary: UMBRELLA,
    topics: DEVELOP_TOPICS,
    embedded: &DEVELOP_EN,
};

/// The develop group's listing, in the fan-out's shape.
pub fn develop_guide() -> ModuleGuide {
    ModuleGuide {
        module: DEVELOP.to_string(),
        binary: UMBRELLA.to_string(),
        topics: DEVELOP_TOPICS
            .iter()
            .map(|(topic, summary)| TopicRef {
                topic: topic.to_string(),
                summary: summary.to_string(),
            })
            .collect(),
    }
}

/// The catalog compiled into THIS binary that serves `module`, if any.
///
/// The three blocks return `None`: their content lives in their own binaries and is fetched by
/// shelling out. The two that return `Some` are read directly — spawning `nxs guide <topic>` from
/// inside `nxs guide` would be unbounded recursion.
pub fn catalog_for(module: &str) -> Option<&'static Catalog> {
    match module {
        UMBRELLA => Some(&CATALOG),
        DEVELOP => Some(&DEVELOP_CATALOG),
        _ => None,
    }
}

/// The umbrella's guide catalog. Unlike the three blocks' catalogs this one is never reached by
/// shelling out — it is compiled into the very binary that serves `nxs guide`, so the lookup path
/// reads it directly (see `cli::show_guide`). Spawning `nxs guide <topic> --json` from inside
/// `nxs guide` would be an unbounded recursion, not an implementation detail.
pub const CATALOG: Catalog = Catalog {
    binary: UMBRELLA,
    topics: TOPICS,
    embedded: &GUIDE_EN,
};

/// The umbrella's own listing in the same shape the fan-out produces, so the listing and the
/// `--json` envelope treat it like any other entry instead of branching on it.
pub fn umbrella_guide() -> ModuleGuide {
    ModuleGuide {
        module: UMBRELLA.to_string(),
        binary: UMBRELLA.to_string(),
        topics: TOPICS
            .iter()
            .map(|(topic, summary)| TopicRef {
                topic: topic.to_string(),
                summary: summary.to_string(),
            })
            .collect(),
    }
}

/// One topic as the fan-out carries it — the same `(topic, summary)` pair every module's
/// `guide --json` listing emits, parsed back into a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRef {
    pub topic: String,
    pub summary: String,
}

/// One module's guide listing: its registry key, the binary a reader types to reach it, and its
/// topics in the module's own declared order. `topics` MAY be empty — a block ships the guide
/// mechanism before its content (chat until `6j6v.t6vd`, memory until `6j6v.h4k0`), and an empty
/// block is a legitimate, well-formed answer that this fan-out passes through rather than hides.
/// All three carry guides today; the pass-through is held by the tests below on a synthetic
/// roster, so it stays a rule rather than an accident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleGuide {
    pub module: String,
    pub binary: String,
    pub topics: Vec<TopicRef>,
}

/// The fan-out result: every fanned-over module's listing, in fan-out order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideFanOut {
    pub modules: Vec<ModuleGuide>,
}

impl GuideFanOut {
    /// Every module that lists `topic`, in fan-out order. The basis of the ambiguity rule below:
    /// `getting-started` and `commands` exist in more than one block by design, so "which one did
    /// you mean" is a normal question, not an edge case.
    pub fn carriers(&self, topic: &str) -> Vec<&ModuleGuide> {
        self.modules
            .iter()
            .filter(|m| m.topics.iter().any(|t| t.topic == topic))
            .collect()
    }

    /// Resolve `topic` to the ONE module that serves it, or a `validation` error.
    ///
    /// Never picks for the user when several blocks carry the same topic name: it names the exact
    /// per-tool commands instead. Silently preferring the first module would make `nxs guide
    /// commands` mean "flow's commands" forever, which is precisely the flow-is-the-suite framing
    /// this ticket exists to undo.
    pub fn resolve(&self, topic: &str) -> Result<&ModuleGuide> {
        let carriers = self.carriers(topic);
        // The umbrella wins when it carries the name (6j6v.0fvt). This is precedence, not a
        // tie-break: `nxs guide getting-started` asking "which one did you mean" would be absurd
        // when the umbrella's getting-started is precisely the suite's, the one the blocks point
        // back at. Every topic the umbrella does NOT carry falls through to the rule below
        // unchanged — nothing is ever picked for the user among the blocks themselves.
        if let Some(umbrella) = carriers.iter().find(|m| m.binary == UMBRELLA) {
            return Ok(umbrella);
        }
        match carriers.as_slice() {
            [one] => Ok(one),
            [] => Err(NxfError::validation(format!(
                "unknown guide topic '{topic}'; {}",
                self.available()
            ))),
            many => Err(NxfError::validation(format!(
                "guide topic '{topic}' exists in {} building blocks — ask one of them: {}",
                many.len(),
                many.iter()
                    .map(|m| format!("`{} guide {topic}`", m.binary))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    /// The "what is on offer" half of an unknown-topic error, as `<binary> guide <topic>` pairs in
    /// fan-out order — or a plain statement when nothing is on offer yet.
    fn available(&self) -> String {
        let all: Vec<String> = self
            .modules
            .iter()
            .flat_map(|m| {
                m.topics
                    .iter()
                    .map(move |t| format!("{} {}", m.binary, t.topic))
            })
            .collect();
        if all.is_empty() {
            "no building block ships guides yet".to_string()
        } else {
            format!("expected one of: {}", all.join(", "))
        }
    }
}

/// The modules `nxs guide` fans out over: the workspace's ACTIVE modules, else the full roster.
///
/// See the module docs for why the fallback exists rather than a "no workspace" error. Note the two
/// separate paths into it — no `.nxs/` at all, and a workspace that has registered no modules (a
/// pre-`active_modules` checkout) — because a reader must not have to know which of the two they
/// are in to read a manual. An unknown ACTIVE module is still a loud error, exactly as in `prime`:
/// a config naming a module this binary cannot serve is broken, and `resolve_active` is the one
/// place that says so.
pub fn resolve_modules(db: Option<&str>, start: &Path) -> Result<Vec<&'static ModuleInit>> {
    let roster = registry::roster();
    let active = match Workspace::resolve(db, start) {
        Ok(ws) => registry::resolve_active(&roster, &ws.config)?,
        Err(e) if e.kind == ErrorKind::NoWorkspace => Vec::new(),
        Err(e) => return Err(e),
    };
    Ok(if active.is_empty() { roster } else { active })
}

/// Fan the guide LISTING out over `modules` by shelling out to `<binary> guide --json`. Production
/// path — see [`fan_out_with`] for the testable core.
pub fn fan_out(modules: &[&ModuleInit]) -> Result<GuideFanOut> {
    fan_out_with(modules, |m| {
        spawn::run_capture(m.binary, &["guide", "--json"])
    })
}

/// [`fan_out`] with the per-module runner injected — the testable core. `run(module)` returns that
/// module's `guide --json` stdout; results are collected in the given (fan-out) order.
///
/// Parsing is strict: a module whose listing is not the declared `[{topic, summary}]` array fails
/// the whole call, naming the offender. The alternative — skipping it — would render a suite
/// listing that silently omits a block, which is the failure mode this whole ticket is about.
pub fn fan_out_with<F>(modules: &[&ModuleInit], run: F) -> Result<GuideFanOut>
where
    F: Fn(&ModuleInit) -> Result<String>,
{
    let modules = modules
        .iter()
        .copied()
        .map(|m| {
            Ok(ModuleGuide {
                module: m.key.to_string(),
                binary: m.binary.to_string(),
                topics: parse_listing(m.binary, &run(m)?)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(GuideFanOut { modules })
}

/// Parse one module's `guide --json` listing into records, preserving its declared order.
fn parse_listing(binary: &str, raw: &str) -> Result<Vec<TopicRef>> {
    let value: serde_json::Value = serde_json::from_str(raw.trim())
        .map_err(|e| NxfError::io(format!("parsing `{binary} guide --json` output: {e}")))?;
    let arr = value.as_array().ok_or_else(|| {
        NxfError::io(format!(
            "`{binary} guide --json` did not emit a topic array"
        ))
    })?;
    arr.iter()
        .map(|entry| {
            let field = |name: &str| {
                entry
                    .get(name)
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        NxfError::io(format!(
                            "`{binary} guide --json` topic entry is missing a string `{name}`"
                        ))
                    })
            };
            Ok(TopicRef {
                topic: field("topic")?,
                summary: field("summary")?,
            })
        })
        .collect()
}

/// Fetch ONE topic's raw markdown from the module that serves it, by shelling out to
/// `<binary> guide <topic> --json`.
///
/// `--json` and not the human form on purpose: it hands back the markdown VERBATIM, so `nxs` runs
/// the same `nxs_guide::render_markdown` every module runs and the RENDERED GUIDE is byte-identical
/// to `<binary> guide <topic>`. (Not the whole stream: flow frames its human output with a leading
/// and trailing blank line — nexus-flow-vwx — which is `nxf`'s presentation, not the guide's, and
/// the umbrella has framing of its own.) Rendering in the child and capturing its text would instead
/// make the umbrella's output depend on the child's terminal detection.
pub fn fetch_topic(binary: &str, topic: &str) -> Result<String> {
    let raw = spawn::run_capture(binary, &["guide", topic, "--json"])?;
    parse_topic(binary, topic, &raw)
}

/// Pull the `content` field out of a module's `guide <topic> --json` record.
fn parse_topic(binary: &str, topic: &str, raw: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(raw.trim()).map_err(|e| {
        NxfError::io(format!(
            "parsing `{binary} guide {topic} --json` output: {e}"
        ))
    })?;
    value
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NxfError::io(format!(
                "`{binary} guide {topic} --json` carried no string `content`"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::InitRequest;

    fn noop(_: &InitRequest) -> Result<()> {
        Ok(())
    }

    /// A test roster descriptor (the fan-out reads `.key`/`.binary`).
    fn module(key: &'static str, binary: &'static str, order: u16) -> ModuleInit {
        ModuleInit {
            key,
            binary,
            now_env: "X_NOW",
            blurb: "",
            recommended: false,
            order,
            accepts_plugin: false,
            init_fn: noop,
            welcome: None,
            details: None,
            first_command: None,
        }
    }

    fn listing(pairs: &[(&str, &str)]) -> String {
        let arr: Vec<serde_json::Value> = pairs
            .iter()
            .map(|(t, s)| serde_json::json!({ "summary": s, "topic": t }))
            .collect();
        serde_json::Value::Array(arr).to_string()
    }

    fn suite() -> GuideFanOut {
        let (flow, memory, chat) = (
            module("flow", "nxf", 10),
            module("memory", "nxm", 20),
            module("chat", "nxc", 30),
        );
        let modules = vec![&flow, &memory, &chat];
        fan_out_with(&modules, |m| {
            Ok(match m.key {
                "flow" => listing(&[("getting-started", "Flow's."), ("commands", "Flow's.")]),
                // The zero-topics shape a block ships before its content — a well-formed EMPTY
                // listing. Synthetic since 6j6v.h4k0: no shipped block is empty any more.
                "memory" => "[]".to_string(),
                _ => listing(&[("getting-started", "Chat's.")]),
            })
        })
        .unwrap()
    }

    #[test]
    fn fan_out_collects_each_module_in_order_with_its_binary() {
        let out = suite();
        assert_eq!(
            out.modules
                .iter()
                .map(|m| (m.module.as_str(), m.binary.as_str(), m.topics.len()))
                .collect::<Vec<_>>(),
            vec![("flow", "nxf", 2), ("memory", "nxm", 0), ("chat", "nxc", 1)]
        );
    }

    #[test]
    fn a_block_with_no_guides_yet_is_carried_through_not_dropped() {
        // The zero-topics wrinkle at the umbrella: memory APPEARS in the suite listing with an
        // empty topic set. Dropping it would make "the suite has three building blocks" a claim
        // the listing itself contradicts.
        let out = suite();
        let memory = out.modules.iter().find(|m| m.module == "memory").unwrap();
        assert!(memory.topics.is_empty());
    }

    #[test]
    fn a_topic_only_one_block_carries_resolves_to_that_block() {
        assert_eq!(suite().resolve("commands").unwrap().binary, "nxf");
    }

    #[test]
    fn a_topic_several_blocks_carry_is_a_loud_error_naming_each_command() {
        // The collision case the slug scheme exists for, seen from the CLI: never pick silently.
        let err = suite().resolve("getting-started").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(
            err.msg.contains("`nxf guide getting-started`"),
            "{}",
            err.msg
        );
        assert!(
            err.msg.contains("`nxc guide getting-started`"),
            "{}",
            err.msg
        );
        assert!(
            !err.msg.contains("`nxm guide"),
            "memory carries no such topic: {}",
            err.msg
        );
    }

    #[test]
    fn an_unknown_topic_lists_every_available_per_tool_command() {
        let err = suite().resolve("does-not-exist").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("nxf getting-started"), "{}", err.msg);
        assert!(err.msg.contains("nxc getting-started"), "{}", err.msg);
    }

    #[test]
    fn an_unknown_topic_with_no_guides_anywhere_says_so_plainly() {
        let memory = module("memory", "nxm", 20);
        let out = fan_out_with(&[&memory], |_| Ok("[]".to_string())).unwrap();
        let err = out.resolve("anything").unwrap_err();
        assert!(
            err.msg.contains("no building block ships guides yet"),
            "{}",
            err.msg
        );
    }

    #[test]
    fn a_module_whose_listing_is_not_the_declared_shape_fails_loud_naming_it() {
        let memory = module("memory", "nxm", 20);
        let err = fan_out_with(&[&memory], |_| Ok("{\"not\":\"an array\"}".into())).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(err.msg.contains("nxm guide --json"), "{}", err.msg);
    }

    #[test]
    fn a_failing_module_surfaces_rather_than_being_skipped() {
        let (flow, memory) = (module("flow", "nxf", 10), module("memory", "nxm", 20));
        let err = fan_out_with(&[&flow, &memory], |m| {
            if m.key == "memory" {
                Err(NxfError::io("nxm guide blew up"))
            } else {
                Ok("[]".into())
            }
        })
        .unwrap_err();
        assert!(err.msg.contains("nxm guide blew up"));
    }

    /// The umbrella's own catalog obeys the same drift rule every block's does: a `.md` added
    /// without listing it, or a topic listed with no file, must go red here.
    #[test]
    fn the_umbrellas_embedded_dir_and_topics_list_are_in_sync() {
        nxs_guide::assert_catalog_parity(&CATALOG);
    }

    /// The develop tree obeys it too. Two `include_dir!` trees in one crate means two chances for
    /// a file to exist unlisted, and the second one has no other reader to notice.
    #[test]
    fn the_develop_embedded_dir_and_topics_list_are_in_sync() {
        nxs_guide::assert_catalog_parity(&DEVELOP_CATALOG);
    }

    /// The constraint that comes WITH a second catalog under one binary, and the reason it is a
    /// test rather than a comment.
    ///
    /// The fan-out already refuses to guess when a topic exists in several blocks — it answers
    /// "ask one of them: `nxf guide commands`, `nxm guide commands`". That answer only works
    /// because the blocks have different binaries. Both catalogs here are served by `nxs`, so a
    /// name in both would produce "ask one of them: `nxs guide x`, `nxs guide x`" — a refusal
    /// with no way to act on it, and no other gate would notice (6j6v.jepw).
    #[test]
    fn topic_names_do_not_collide_across_the_two_nxs_catalogs() {
        for (topic, _) in TOPICS {
            assert!(
                !DEVELOP_TOPICS.iter().any(|(other, _)| other == topic),
                "`{topic}` is in BOTH nxs catalogs — `nxs guide {topic}` could not say which it \
                 means, and the fan-out's \"ask one of them\" would name the same command twice"
            );
        }
    }

    /// Every umbrella topic must resolve to markdown with a top heading — the H1 the website's
    /// assembler is fail-closed on, asserted here rather than discovered in a red content build.
    #[test]
    fn every_umbrella_topic_resolves_to_markdown_with_a_heading() {
        for (topic, summary) in TOPICS {
            assert!(!summary.is_empty(), "{topic} has no summary");
            let md = CATALOG.lookup(topic).expect("listed topic must resolve");
            assert!(md.starts_with("# "), "{topic} does not open with an H1");
        }
    }

    /// The umbrella takes precedence for a name it carries — the 6j6v.0fvt behaviour change.
    #[test]
    fn a_topic_the_umbrella_carries_resolves_to_the_umbrella_not_an_ambiguity_error() {
        let mut fan = suite();
        fan.modules.insert(0, umbrella_guide());
        // flow and chat both carry `getting-started` in this roster, so the pre-0fvt rule was an
        // ambiguity error; the umbrella carrying it is what makes the answer unambiguous.
        assert_eq!(fan.resolve("getting-started").unwrap().binary, UMBRELLA);
    }

    /// …and ONLY for a name it carries. The error path for the blocks' own collisions is untouched:
    /// this is precedence, not "the umbrella answers everything".
    #[test]
    fn a_collision_the_umbrella_does_not_carry_still_refuses_to_pick() {
        let mut fan = suite();
        fan.modules.insert(0, umbrella_guide());
        // Anchored on the binary, never on a position: inserting the umbrella shifts every
        // index, and an off-by-one here would build a roster where `commands` has ONE carrier —
        // a test that passes while asserting nothing.
        let chat = fan.modules.iter_mut().find(|m| m.binary == "nxc").unwrap();
        chat.topics.push(TopicRef {
            topic: "commands".into(),
            summary: "Chat's.".into(),
        });
        let err = fan.resolve("commands").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("`nxf guide commands`"), "{}", err.msg);
        assert!(!err.msg.contains("`nxs guide"), "{}", err.msg);
    }

    #[test]
    fn fetching_a_topic_takes_the_content_field_verbatim() {
        let raw = serde_json::json!({ "content": "# T\n\nbody\n", "topic": "t" }).to_string();
        assert_eq!(parse_topic("nxf", "t", &raw).unwrap(), "# T\n\nbody\n");
    }

    #[test]
    fn fetching_a_topic_from_a_malformed_record_fails_loud() {
        let err = parse_topic("nxf", "t", "{\"topic\":\"t\"}").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(err.msg.contains("content"), "{}", err.msg);
    }
}

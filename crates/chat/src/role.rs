//! Role declarations for the nexus-chat "role runtime" (nxf epic 6j6v.zenf): a YAML file per role
//! under `.nxs-personas/*.yaml`, loaded into a [`RoleDecl`] and turned into the system prompt a
//! live SDK session is launched with. This module is pure data + composition — no SDK, no network,
//! no I/O beyond reading the YAML files themselves — so it is fully deterministic and covered by
//! `cargo test` alone (T3 of the walking-skeleton plan). T6 wires this into the actual session
//! launch; T5's `RoleSpec` (the wire/API shape) is expected to mirror this struct.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::channel::{ChannelDecl, ValidationError};
use crate::error::{NxfError, Result};

/// Which Claude Code system-prompt preset (if any) the role's prompt builds on top of. `ClaudeCode`
/// is the default: most roles want the full Claude Code tool-use/agentic preset with the role's
/// `system_prompt` appended, not replacing it (spec: "append, don't override" — 5jz-era decision
/// carried into the role runtime). `None` opts a role out entirely — its `system_prompt` stands
/// alone (e.g. a narrow, non-agentic reviewer role).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasePrompt {
    #[default]
    ClaudeCode,
    None,
}

/// Whether the role's composed prompt includes the project's own `CLAUDE.md`. `Inherit` (default)
/// prepends it ahead of the role's `system_prompt` — the role sees the same project context a plain
/// Claude Code session would. `Ignore` drops it (a role that shouldn't see project conventions).
/// `Override` is reserved for a future per-role replacement document — T3 only needs to round-trip
/// the tag; composition for it is not yet specified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeMd {
    #[default]
    Inherit,
    Ignore,
    Override,
}

/// **Declared, and read by nothing** — the honest state of this field, and the reason it survived
/// the removal of its channel-level twin (nxf 6j6v.fepb).
///
/// It names the question "does a persona's turn start fresh, or continue what it already had?", and
/// THE CALL ANSWERS THAT, NOT THE DECLARATION. `nxc send --to <persona>` mints a fresh internal
/// session and spawns with `resume_real: None`; a `nxc reply --thread` into that persona's own
/// thread resumes the session behind the thread's return address
/// ([`crate::orchestration::resume_return_address`]), carrying everything that session already
/// knows. A policy here has nothing left to decide that the verb has not decided already. The line
/// that used to stand here described `Fresh` as "the safe, isolated choice for a sub-agent-style
/// role", which reads as a choice being made; none is.
///
/// **Why it stays where [`crate::channel::ChannelDecl`]'s `member_session` went** (nxf 6j6v.fepb,
/// a decision the owner left to the cleanup): the two ask the same question, but only one of them
/// ACTED. `member_session` refused its own second value by name in preflight, stood behind an
/// `unreachable!()`, and pointed a user-visible error at a closed ticket about something else — a
/// declaration could fail a whole invocation on it, so removing it BOUGHT something. This field
/// steers nothing and refuses nothing, so removing it buys nothing at all. The `pub`-API break is
/// the same price for both (both are `pub`, both would need the same breaking-change note on a
/// surface downstream consumers pin in lockstep) — it is worth paying once and not twice, and that
/// asymmetry is in what the two purchases are, not in what they cost (PR #349 review, Code Quality
/// #2, which was right that the price alone distinguishes nothing). It belongs instead where
/// `sub_agents`/`reports_to` already are: the guide's
/// "declared but not yet read" list, which now states the rule above rather than a false one (nxf
/// 6j6v.mg5b — the previous wording, "every persona's turn currently starts a fresh session", cost
/// an external testbed five declarations built on an amnesia that does not exist).
///
/// `Fresh` (the default) is what every declaration written before and since means, unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPolicy {
    #[default]
    Fresh,
    Continue,
}

/// Whether a role's or channel's sessions need EXCLUSIVE use of the repo's working copy and build
/// directory, or may SHARE it with other concurrent role sessions (nxf epic 6j6v.bqe0, the
/// working-tree lease). A persona may be named anything, so the runtime cannot infer this from the
/// task at hand — the need is recognized at the DECLARATION, not the trigger, which is why this
/// lives on [`RoleDecl`]/[`crate::channel::ChannelDecl`] rather than being derived at runtime.
///
/// `Shared` (default) is what every declaration written before this field existed means, unchanged:
/// no lease is taken, and any number of role sessions may run against the same working copy at
/// once, exactly as today.
///
/// What READS it is `orchestration::needs_working_tree`, the one place either declaration
/// is consulted: `Exclusive` here short-circuits it to `true`, and a `Shared` role still takes the
/// lease when the thread it is triggered into belongs to a declared channel that is itself
/// `Exclusive` ([`crate::channel::ChannelDecl::working_tree`]). `true` there is what makes
/// `trigger_role` acquire-or-queue via [`crate::working_tree`].
///
/// `#[non_exhaustive]` for the same reason as [`Model`]/[`WakeSkipReason`](crate::orchestration::WakeSkipReason):
/// a third value (`isolated`, nxf 6j6v.3npy) must be an additive minor, not a break for a downstream
/// `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkingTree {
    #[default]
    Shared,
    Exclusive,
}

impl WorkingTree {
    /// Whether this is the default — the `skip_serializing_if` predicate that keeps a declaration
    /// that does not set `working_tree:` from growing the key on serialize. `pub` (unlike
    /// [`Addressable::is_general`], which only `RoleDecl` in this same module needs) because
    /// `ChannelDecl` in `channel.rs` needs it too, for its own `working_tree` field.
    pub fn is_shared(&self) -> bool {
        matches!(self, WorkingTree::Shared)
    }
}

/// Which Claude model a role's session runs on. The declaration's wire form is the bare alias
/// (`fable`/`opus`/`sonnet`); [`Model::sdk_id`] turns it into the string the Claude Agent SDK's
/// `options.model` expects. The mapping lives HERE, in the engine, and never in
/// `agent-sidecar/src/main.mjs` — one table in one place, and the sidecar stays a dumb executor of
/// the spec it is handed (the same division the `tools`/`permissions` keys already follow).
///
/// `#[non_exhaustive]`: this type reaches the app-facade surface, which goes irreversibly public
/// with the open-source launch. Adding a fourth model later must stay an additive minor rather than
/// a break for a downstream `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Model {
    Fable,
    Opus,
    Sonnet,
    /// The small, fast one. Added with nxf 6j6v.e76c, whose thread-naming derivation the owner
    /// decided runs on it — see [`crate::naming::NAMING_MODEL`], which names this variant rather
    /// than spelling an sdk id of its own, because the table below is the only place that mapping
    /// is allowed to live.
    ///
    /// Declarable like the other three, and that is a consequence rather than the purpose: the
    /// alias namespace IS this enum, so a model the engine can pick is a model a role author can
    /// name. The [`Stage`] ladder below is untouched — no band resolves here.
    Haiku,
}

impl Model {
    /// The Claude Agent SDK `options.model` string for this alias.
    pub fn sdk_id(self) -> &'static str {
        match self {
            Model::Fable => "claude-fable-5",
            Model::Opus => "claude-opus-5",
            Model::Sonnet => "claude-sonnet-5",
            Model::Haiku => "claude-haiku-4-5-20251001",
        }
    }
}

/// The persona's seniority band (nxf 6j6v.p6m1, surface draft §3.1) — `junior` / `senior` /
/// `principal`, mapped to the model classes by [`Stage::model`].
///
/// **It exists to give the user a vocabulary that is not a model name.** A role author decides how
/// much thinking a job is worth without knowing which models exist this month, and the cost question
/// becomes discussable in the same words the work is described in. The mapping is one table, here,
/// for [`Model::sdk_id`]'s reason: the declaration's spelling and the engine's choice cannot drift
/// apart if there is only one place that joins them.
///
/// A declared `model:` still wins — see [`RoleDecl::effective_model`]. `#[non_exhaustive]` for the
/// reason [`Model`] is: this reaches the app-facade surface, and a fourth band later must be an
/// additive minor rather than a break for a downstream `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Stage {
    Junior,
    Senior,
    Principal,
}

impl Stage {
    /// The model class this band resolves to, ordered by capability (and by cost, which is the same
    /// ladder): `junior` → [`Model::Sonnet`], `senior` → [`Model::Opus`], `principal` →
    /// [`Model::Fable`].
    pub fn model(self) -> Model {
        match self {
            Stage::Junior => Model::Sonnet,
            Stage::Senior => Model::Opus,
            Stage::Principal => Model::Fable,
        }
    }
}

/// Who may open a conversation with a persona (nxf 6j6v.p6m1, surface draft §3.1; widened to name
/// CALLERS by nxf 6j6v.st83 / 6j6v.d575). Four declared forms:
///
/// ```yaml
/// addressable: general        # the default — anyone, human or persona, may send directly
/// addressable: none           # nobody directly; come through a channel whose cast names me
/// addressable:                # exactly these callers, and nobody else
///   personas: [pm]
///   humans: true
/// addressable: [review]       # DEPRECATED (nxf 6j6v.g0yn) — read as `none`
/// ```
///
/// # The whitelist is a whitelist: an omitted half names NOBODY of that class
///
/// `{humans: true}` is the human at the terminal and no persona; `{personas: [pm]}` is the PM and
/// no human. That rule is what makes the two shipped cases one form rather than two — the PM a
/// person may DM while every persona goes through the round, and the specialist one named peer may
/// DM while a person goes through the channel. `{}` is legal and says the same thing `none` does.
///
/// # Why a caller-derived limit is admissible HERE, when the guide says it is not
///
/// The rule this surface is built on (see [`crate::persona`]) is that omitting your identity must
/// never let you do MORE than declaring it, which is why an ADDRESS BOOK is guidance and never
/// enforcement. A whitelist inverts none of that, and the distinction is worth stating exactly
/// because the earlier text drew it too widely:
///
/// * `general` and `none` read the same for every caller, as they always have.
/// * A whitelist can only ever REFUSE somebody `general` would have admitted. Dropping an identity
///   moves a caller into [`Identity::Human`](crate::persona::Identity::Human), so on a
///   `{personas: [...]}` persona anonymity grants strictly LESS — the direction the rule demands.
/// * On a `{humans: true}` persona it grants MORE, and that is the honest cost. It is bounded by
///   what a session stamp IS: `nxc send` has no `--persona`, so the class comes from `NXC_SESSION`
///   looked up in the session map this workspace minted. A persona that drops it to pass stops
///   being itself for that call — no return address, no resume, nothing to attribute the answer to
///   ([`crate::orchestration::resolve_hop`]) — so it does not get the same thing plus, it gets a
///   different, worse thing.
///
/// And one level up, unchanged: on a single machine no boundary inside `nxc` is stronger than
/// access to the workspace file. This is CORRECTNESS — a declaration that means what it says —
/// rather than defence, exactly as [`crate::persona`]'s module docs state for every check here.
///
/// `#[non_exhaustive]` for the reason [`Stage`] and [`Model`] carry it, learned here the expensive
/// way: this type reaches the app-facade surface, a downstream `match` on it compiles against a
/// pinned tag, and THIS change — two new variants at once — is exactly the break that argument
/// predicts. Adding the marker as part of the break it describes costs nothing extra and makes a
/// fifth form later an additive minor rather than another one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Addressable {
    /// Anyone may open a conversation directly. The default, and what a role that declares no
    /// `addressable:` at all means.
    #[default]
    General,
    /// Nobody may open a conversation directly (nxf 6j6v.st83). The route in is a channel whose
    /// cast names this persona — derived, never declared here, which is the half nxf 6j6v.g0yn
    /// takes off this field.
    Nobody,
    /// Exactly these callers may, and nobody else: the named declared personas, and — when
    /// `humans` — every caller that is not a declared persona (the terminal, a host that starts no
    /// `nxc` session).
    Only { personas: Vec<String>, humans: bool },
    /// **Deprecated** (nxf 6j6v.g0yn): a list of channel names, which meant "nobody directly, come
    /// through one of these". It is read exactly as [`Nobody`](Addressable::Nobody) — the channels
    /// a persona takes part in are the CAST's answer now ([`crate::channel::cast`]), declared once
    /// at the channel.
    ///
    /// Kept as a variant of its own rather than folded into `Nobody` at parse time so the two
    /// things that still need the author's words can have them: the deprecation warning that tells
    /// them what to write instead, and the referential check that a listed name is a channel that
    /// exists at all (nxf 6j6v.st83's second finding — until now a persona handle written here was
    /// silently read as a channel name and nothing complained until somebody tried to send).
    ViaChannels(Vec<String>),
}

impl Addressable {
    /// Whether `caller` may open a direct conversation with this persona.
    ///
    /// The one question the enforcement point asks ([`crate::surface::send_to`]), and the reason
    /// this takes an [`Identity`](crate::persona::Identity) rather than a bare handle: "a human"
    /// is a caller class in its own right, not the absence of one.
    pub fn allows_direct_from(&self, caller: &crate::persona::Identity) -> bool {
        match self {
            Addressable::General => true,
            Addressable::Nobody | Addressable::ViaChannels(_) => false,
            Addressable::Only { personas, humans } => match caller.persona() {
                None => *humans,
                Some(handle) => personas.iter().any(|p| p == handle),
            },
        }
    }

    /// Whether ANY caller may open a direct conversation — what a reader needs when there is no
    /// caller to ask about, such as a catalogue rendered for nobody in particular.
    pub fn allows_direct_from_anyone(&self) -> bool {
        match self {
            Addressable::General => true,
            Addressable::Nobody | Addressable::ViaChannels(_) => false,
            Addressable::Only { personas, humans } => *humans || !personas.is_empty(),
        }
    }

    /// The channels named by the DEPRECATED list form, empty for every other form. Not "the
    /// channels this persona takes part in" — that is [`crate::channel::cast`]'s answer, derived
    /// from the channel declarations, and a caller wanting the route in should ask
    /// [`crate::channel::channels_casting`] instead.
    pub fn channels(&self) -> &[String] {
        match self {
            Addressable::ViaChannels(channels) => channels,
            _ => &[],
        }
    }

    /// The declared personas of a whitelist, empty for every other form — what the referential
    /// check walks.
    pub fn named_personas(&self) -> &[String] {
        match self {
            Addressable::Only { personas, .. } => personas,
            _ => &[],
        }
    }

    /// Whether this is the default — the `skip_serializing_if` predicate that keeps a role file
    /// that declares no `addressable:` from growing the key on serialize.
    fn is_general(&self) -> bool {
        matches!(self, Addressable::General)
    }
}

/// The mapping form's own shape, named so serde can derive both directions of it and so the
/// defaults live in exactly one place: an omitted key is "nobody of that class".
///
/// **`deny_unknown_fields` is load-bearing, not tidiness.** Both fields default, so without it any
/// mapping at all satisfies this variant — `addressable: {persona: [pm]}`, singular by a typo,
/// would parse as an empty whitelist and silently mean "nobody", which is the exact class of quiet
/// misconfiguration nxf 6j6v.st83 exists to end. With it the untagged enum finds no variant and the
/// role file fails to load, naming itself.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnlyRaw {
    #[serde(default)]
    personas: Vec<String>,
    #[serde(default)]
    humans: bool,
}

impl Serialize for Addressable {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Addressable::General => serializer.serialize_str("general"),
            Addressable::Nobody => serializer.serialize_str("none"),
            Addressable::Only { personas, humans } => OnlyRaw {
                personas: personas.clone(),
                humans: *humans,
            }
            .serialize(serializer),
            Addressable::ViaChannels(channels) => channels.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Addressable {
    /// Accept `"general"`, `"none"`, a mapping of caller classes, or the deprecated list of channel
    /// names; anything else is a `serde::de::Error::custom`, which `load_role` surfaces as the same
    /// `validation` error (naming the file) as any other malformed role content.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            List(Vec<String>),
            Only(OnlyRaw),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Str(s) if s == "general" => Ok(Addressable::General),
            Raw::Str(s) if s == "none" => Ok(Addressable::Nobody),
            Raw::Str(s) => Err(serde::de::Error::custom(format!(
                "invalid addressable value '{s}'; expected \"general\", \"none\", a mapping of \
                 `personas:`/`humans:`, or a list of channel names"
            ))),
            Raw::List(channels) => Ok(Addressable::ViaChannels(channels)),
            Raw::Only(only) => Ok(Addressable::Only {
                personas: only.personas,
                humans: only.humans,
            }),
        }
    }
}

/// One line of a persona's address book (nxf 6j6v.p6m1, surface draft §3.1): a persona or channel
/// it may open a conversation with, and what for.
///
/// **The description is the load-bearing half.** It works like a skill's frontmatter — a short
/// sentence a model chooses from — which is why `why` is rendered everywhere `to` is. What the
/// address book deliberately does NOT do in this slice is restrict: see
/// [`crate::persona`]'s module docs for why deriving a limit from a droppable identity would be
/// worse than useless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressBookEntry {
    /// A declared persona handle or channel name.
    pub to: String,
    /// What this persona would address it about — one short line, in the author's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// **Which of the suite's session-start blocks this persona is handed** (nxf 6j6v.k8zq).
///
/// Owner, 2026-08-23: *"Jede Rolle sollte den Inhalt von `nxs prime --persona` erhalten. Wir
/// muessen in der Deklaration die Moeglichkeit bekommen, einzelne Dienste zu filtern (z. B. das
/// Brett oder die Erinnerungen auszuschliessen)."* A pure reviewer does not need the board; a PM
/// does. The filter answers WHICH services contribute — never how big their contribution is, which
/// is what nxf 6j6v.4mmk and nxf 6j6v.waq9 answer.
///
/// Every field defaults to `true`, so `prime: {}` is `prime: true` and a map that names one service
/// leaves the others alone — the shape a declaration author expects when they write down the one
/// thing they want changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimeServices {
    /// `nxf prime` — the board: what is next, what is blocked, what this workspace is working on.
    #[serde(default = "default_true")]
    pub flow: bool,
    /// `nxm prime` — the project's durable memory, rules first (nxf 6j6v.waq9).
    ///
    /// **Excluding it can cost more than memories** — see [`PrimeServices::project_rules_are_lost`].
    #[serde(default = "default_true")]
    pub memory: bool,
    /// `nxc prime --persona <handle>` — who this persona is, whom it may address, and how it
    /// answers. Filterable for symmetry, and almost never the right thing to switch off: what it
    /// carries is how a summoned session ENDS ITS TURN. With it off, the persona's identity falls
    /// back to the `job_title`/`job_description`/`expected_output` lines
    /// ([`compose_system_prompt`]), so a declaration cannot make a session anonymous by accident.
    #[serde(default = "default_true")]
    pub chat: bool,
}

impl Default for PrimeServices {
    /// Every service contributes — what an omitted `prime:` and a plain `prime: true` both mean.
    fn default() -> PrimeServices {
        PrimeServices {
            flow: true,
            memory: true,
            chat: true,
        }
    }
}

impl PrimeServices {
    /// No service contributes — what `prime: false` resolves to.
    pub const NONE: PrimeServices = PrimeServices {
        flow: false,
        memory: false,
        chat: false,
    };

    /// **The trap this filter creates, named rather than sprung** (nxf 6j6v.k8zq).
    ///
    /// `memory: false` reads as "this role does not need the project's memories". In a MIGRATED
    /// project it means something else: the judging migration (6j6v.9yaj) moves each section of the
    /// hand-written context document into the store and **strips the section from the file**
    /// (`migration.rs`: "a document is only stripped of a section once that section is a memory";
    /// `keep_sources` is the exception). So the project's rules arrive through `nxm prime`, and
    /// `claude_md: inherit` contributes nothing any more, because the file no longer has them.
    ///
    /// | project      | `memory: false` | where the project rules come from |
    /// | ------------ | --------------- | --------------------------------- |
    /// | migrated     | no              | `nxm prime` — `claude_md:` is redundant |
    /// | migrated     | **yes**         | **nowhere** |
    /// | not migrated | either          | only `claude_md: inherit` |
    ///
    /// The middle row is this item's own doing, and silence is the worst answer to it. `true` here
    /// says the composed prompt is about to be in that row, and the caller is expected to SAY so —
    /// see [`compose_system_prompt`], which puts the warning in the prompt itself, where the one
    /// party who can act on it is reading.
    pub fn project_rules_are_lost(&self, claude_md_contributes: bool) -> bool {
        !self.memory && !claude_md_contributes
    }
}

/// The `prime:` field's two written forms (nxf 6j6v.k8zq): the switch it has always been, or the
/// per-service map.
///
/// ```yaml
/// prime: true            # every service — also what an omitted `prime:` means
/// prime: false           # no prime block at all (the forced ending survives regardless)
/// prime:
///   flow: false          # no `nxf prime` for THIS persona; memory and chat unchanged
/// ```
///
/// Untagged on purpose: **existing declarations with `prime: true` / `prime: false` must load
/// unchanged**, which is an acceptance point of the item and not merely good manners — there are
/// declarations in the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrimeDecl {
    /// `prime: true` / `prime: false` — all services, or none.
    All(bool),
    /// `prime: {flow: …, memory: …, chat: …}` — service by service.
    Services(PrimeServices),
}

impl Default for PrimeDecl {
    fn default() -> PrimeDecl {
        PrimeDecl::All(true)
    }
}

impl PrimeDecl {
    /// Whether this role is primed at all — `false` only for the explicit `prime: false`. A map
    /// that switches every service off is still PRIMED: it asked for a filtered block, and the
    /// identity that block would otherwise have carried is composed from the declaration instead.
    pub fn is_primed(&self) -> bool {
        !matches!(self, PrimeDecl::All(false))
    }

    /// Which services contribute.
    pub fn services(&self) -> PrimeServices {
        match self {
            PrimeDecl::All(true) => PrimeServices::default(),
            PrimeDecl::All(false) => PrimeServices::NONE,
            PrimeDecl::Services(services) => *services,
        }
    }
}

/// One role's declaration, loaded verbatim from `.nxs-personas/<handle>.yaml`. Only `handle` and
/// `system_prompt` are required; every other field defaults (see the per-field docs above and on
/// [`BasePrompt`]/[`ClaudeMd`]/[`SessionPolicy`]) so a minimal role file is just two lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDecl {
    /// The role's stable identifier (also its filename stem by convention, though `load_role` does
    /// not enforce that — the YAML `handle:` field is the source of truth).
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports_to: Option<String>,
    /// The role-specific prompt text. Composition rule lives in [`compose_system_prompt`].
    pub system_prompt: String,
    #[serde(default)]
    pub base_prompt: BasePrompt,
    #[serde(default)]
    pub claude_md: ClaudeMd,
    /// The role's declared base toolset (SDK `options.tools`). `None` when the YAML omits
    /// `tools:` entirely — the role author declared no intent, so the caller must NOT collapse
    /// this into an empty list: `RoleSpec`/`SidecarWorker` (worker.rs) thread the `Option`
    /// through so the sidecar spec's `tools` key reaches `agent-sidecar/src/main.mjs` as JSON
    /// `null`, which leaves `options.tools` unset and the SDK's own full default toolset applies.
    /// `Some(vec![])` is the role EXPLICITLY declaring zero tools (a narrow, non-agentic role) —
    /// a materially different state that must reach the spec as a real empty array (fix round on
    /// 6j6v.zenf's final review: before this, `Vec<String>` always serialized, so an omitted
    /// `tools:` and an explicit `tools: []` were indistinguishable and BOTH produced a
    /// zero-tools session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    /// Which model this role's sessions run on. `None` — the default, and what every role file
    /// written before this field existed deserializes to — means the role declares no preference,
    /// so nothing is sent to the sidecar and the SDK's own default model applies. The `Option` is
    /// load-bearing for exactly the reason [`RoleDecl::tools`]' is: it keeps "declared nothing"
    /// distinguishable from a declared value all the way down to the spec JSON.
    ///
    /// A caller-supplied override on the request beats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    /// **The machine this persona's chats run on** (nxf 6j6v.1c6k) — a machine id, or a name the
    /// workspace's relay knows it by (`nxs sync machines` lists both). `None`, the default, runs a
    /// chat on the machine that starts it.
    ///
    /// Consulted ONCE, when a chat with this persona starts, and its answer is written into the
    /// chat (the thread's `machine` register), because this file is read from each machine's own
    /// checkout and two machines may hold different versions of it; the register is the one answer
    /// both read. A chat's own `--machine` beats it. Not consulted when the persona is commissioned
    /// as a declared channel's member: a channel runs where it is started (see
    /// `docs/specs/E4-executing-machine.md` §2.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// Declared intent only, read by nothing — [`SessionPolicy`] carries what decides this in fact.
    #[serde(default)]
    pub session: SessionPolicy,
    #[serde(default)]
    pub sub_agents: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_output: Option<String>,
    /// Whether — and with WHICH of the suite's session-start blocks — triggering this role prepends
    /// the "prime block" ahead of everything else in [`compose_system_prompt`]'s output. Defaults to
    /// every service, which is what an omitted `prime:` and a plain `prime: true` both mean.
    ///
    /// Priming is the whole point of the feature (6j6v.cvsp): it lets a trigger message carry only
    /// the task, because the role already knows who it is and how to answer. `prime: false` opts a
    /// role out (a narrow role whose `system_prompt` covers this ground itself) — and never opts it
    /// out of the forced ending, which is the engine's, not the declaration's (see
    /// [`compose_system_prompt`]).
    ///
    /// See [`PrimeDecl`] for the map form and [`PrimeServices`] for what each service contributes.
    #[serde(default)]
    pub prime: PrimeDecl,
    /// The persona's seniority band (nxf 6j6v.p6m1), resolved to a model class by
    /// [`Stage::model`] when — and only when — [`model`](RoleDecl::model) declares nothing. `None`
    /// is every role file written before this field existed, and means exactly what it did then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// Who may open a conversation with this persona (nxf 6j6v.p6m1). Defaults to
    /// [`Addressable::General`], which is what every role file written before this field existed
    /// resolves to — so no existing declaration changes meaning.
    #[serde(default, skip_serializing_if = "Addressable::is_general")]
    pub addressable: Addressable,
    /// Who this persona may address, and what for (nxf 6j6v.p6m1).
    ///
    /// **Three states, exactly like [`tools`](RoleDecl::tools), and for the same reason** (nxf
    /// 6j6v.vce2). It was a plain `Vec` until then, which could express only two of them:
    ///
    /// * **omitted** (`None`) — "nobody wrote this down". The surface shows the persona the whole
    ///   declared team, because an absent book is not a prohibition, and reading it as one would
    ///   silently mute every persona declared before this field existed.
    /// * **`address_book: []`** (`Some(vec![])`) — "commissions nothing". A pure reviewer, a
    ///   summarizer, any role at a leaf of the tree: it answers on its own thread and calls nobody.
    ///   A common, well-founded role that could not be DECLARED before — writing `[]` produced the
    ///   whole team, identically to omitting the key, so the only way to say it was a sentence in
    ///   the `system_prompt`, which is the inversion of what the declaration file is for.
    /// * **a non-empty list** — that book, in the author's order.
    ///
    /// The `Option` is load-bearing all the way down to the spec JSON, exactly as `tools`' is: a
    /// plain `Vec` always serializes, so an omitted key and an explicit empty list would be
    /// indistinguishable on the round trip too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_book: Option<Vec<AddressBookEntry>>,
    /// Whether this role's sessions need exclusive use of the repo's working copy and build
    /// directory (nxf epic 6j6v.bqe0, the working-tree lease). Defaults to [`WorkingTree::Shared`],
    /// which is what every role file written before this field existed means, unchanged.
    ///
    /// **This is LIVE**, and the line that used to stand here ("nothing reads this field yet —
    /// this ticket only adds the declared intent") was true only of the ticket that introduced it:
    /// `orchestration::needs_working_tree` short-circuits to `true` on [`WorkingTree::Exclusive`]
    /// here, which is what makes `trigger_role` acquire-or-queue the lease. [`WorkingTree`]'s own
    /// doc said so already, so the two halves of one field contradicted each other — the exact
    /// shape of nxf 6j6v.mg5b one layer down, found while reading that item's section.
    #[serde(default, skip_serializing_if = "WorkingTree::is_shared")]
    pub working_tree: WorkingTree,
}

impl RoleDecl {
    /// The model this role's sessions run on absent any override from above it: its own declared
    /// [`model`](RoleDecl::model), else whatever its [`stage`](RoleDecl::stage) maps to, else `None`
    /// (the SDK's own default applies).
    ///
    /// **`model:` beats `stage:` and the order is not arbitrary**: the band is the coarse,
    /// discussable choice, and a named model is the author saying something the band cannot express.
    /// The precedence lives beside the fields it reads — [`crate::orchestration`]'s own
    /// `resolve_model` handles the level ABOVE the role (a per-call override), and the two do not
    /// overlap.
    ///
    /// The body is [`effective_model_at`](RoleDecl::effective_model_at) with no step band, so the
    /// rule has one place even though two callers ask it (nxf 6j6v.4c3e).
    pub fn effective_model(&self) -> Option<Model> {
        self.effective_model_at(None)
    }

    /// [`effective_model`](RoleDecl::effective_model) with the band a STEP of a declared flow asked
    /// this role to run at (nxf 6j6v.4c3e, [`crate::channel::FlowStep::stage`]). `None` is every
    /// caller that is not serving such a step, and answers exactly as `effective_model` does.
    ///
    /// **The step's band sits BELOW a declared `model:` and ABOVE the role's own band**, which is
    /// the one order the item argues for at length and the one a reader is most likely to get
    /// backwards:
    ///
    /// * a named `model:` means *"this role always runs on this"*, and a caller must not be able to
    ///   take that away — the rule is transitional and lapses when `model:` does, which is the
    ///   direction the declaration surface is moving anyway;
    /// * a band is the coarse, discussable judgement about what a job is worth, and WHICH job it is
    ///   is precisely what the step knows and the role cannot.
    ///
    /// The level ABOVE all three — a per-call override — is collapsed by
    /// [`crate::orchestration::resolve_model`] before it meets this, so the two never apply a
    /// precedence twice.
    pub fn effective_model_at(&self, step: Option<Stage>) -> Option<Model> {
        self.model.or_else(|| step.or(self.stage).map(Stage::model))
    }
}

fn default_true() -> bool {
    true
}

/// The result of [`compose_system_prompt`]: the final prompt text plus whether the caller should
/// launch the SDK session with the Claude Code preset on top of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedPrompt {
    pub use_claude_code_preset: bool,
    pub system_prompt: String,
}

/// Compose a role's final system prompt.
///
/// Four layers, in this order, and the order is the argument:
///
/// 1. **The prime block** — `prime_block`, the composed `nxs prime --persona <handle>` text
///    (nxf 6j6v.k8zq), plus whatever identity the filter left it without, plus the forced ending.
///    First, ahead of even `CLAUDE.md`: "who am I, what does this workspace know, how do I answer"
///    should anchor how the model reads everything after it.
/// 2. **The project `CLAUDE.md`**, when `role.claude_md` is [`ClaudeMd::Inherit`] and
///    `project_claude_md` is a non-empty string.
/// 3. **The role's own `system_prompt`** — the specific job, last, closest to the conversation the
///    trigger message starts.
/// 4. **The task the STEP declared** (nxf 6j6v.6kam), when `step_task` names one — under the role,
///    because it is the narrower of the two and because it is what the role could not know: which
///    job this station is, out of the several the same role serves. See
///    [`crate::channel::FlowStep::task`] for why it hangs on the step rather than on the channel,
///    and [`step_task_layer`] for what marks it as the step's rather than the persona's.
///
/// The result is trimmed. `use_claude_code_preset` mirrors `role.base_prompt == BasePrompt::ClaudeCode`
/// — the caller uses this to decide whether to layer the SDK's Claude Code preset under the returned
/// text (append, never override — see the module docs).
///
/// # `prime_block`
///
/// **This is the whole of what nxf 6j6v.k8zq changed here.** A spawned persona gets no `nxs prime`
/// of its own: `agent-sidecar/src/spec-helpers.mjs` sets `settingSources: []` (SDK isolation,
/// 6j6v.93hz), so no `.claude/settings.json` loads, so the SessionStart hook never fires. The
/// isolation stays — it is what protects a role session from the operator's plugins, skills and
/// hooks — and the project's knowledge comes in as TEXT, on this path, exactly as `CLAUDE.md`
/// already does.
///
/// It is a parameter and not something this function computes because composing it needs three
/// stores and a module registry, and `crates/chat` is one module. The caller composes
/// ([`crate::orchestration`] for a trigger, `crates/nxs` for `nxs prime --persona`, an embedding
/// host over [`crate::engine::Engine::persona_prime`]); the layering is decided rather than
/// smuggled. Pass `""` for none, which is what an unprimed role and a caller with nothing to
/// contribute both give.
///
/// # `reply_thread`
///
/// The second half of the epic's guidance move (nxf 6j6v.enrs): the thread id a caller's trigger
/// has already declared, on ITS OWN thread, that it expects this role's reply on (nxf 6j6v.jepk,
/// [`crate::orchestration::coordinator_commission`]) — `None` whenever THIS trigger registered no
/// such expectation. **That used to be two cases and is now one** (nxf 6j6v.ntp9): the thread-less
/// trigger is gone, because the coordinator opens a thread when it is handed none, so what is left
/// is the thread it landed in already carrying somebody else's quorum, which the coordinator
/// deliberately does not overwrite. In that case an obligation exists on that thread and simply is
/// not this trigger's to announce.
///
/// # `step_task`
///
/// The fourth layer (nxf 6j6v.6kam), `None` for every trigger that is not serving a step which
/// declared one — which is every trigger a workspace made before the field existed, and what makes
/// this change nothing at all for a declaration that does not use it.
///
/// **A [`StepTask`] and not a second `Option<&str>` beside `project_claude_md`**, for the reason
/// [`crate::orchestration::RoleSpawn`] gives for having fields at all: two arguments of one type
/// can be swapped and still compile, and this pair would swap the project's conventions with a
/// step's instructions — a wrong prompt that no compiler and no reviewer's eye would catch.
pub fn compose_system_prompt(
    role: &RoleDecl,
    project_claude_md: Option<&str>,
    reply_thread: Option<ReplyObligation<'_>>,
    prime_block: &str,
    step_task: Option<StepTask<'_>>,
) -> ComposedPrompt {
    let claude_md = match (role.claude_md, project_claude_md) {
        (ClaudeMd::Inherit, Some(text)) if !text.trim().is_empty() => Some(text),
        _ => None,
    };
    let mut prompt = String::new();
    if role.prime.is_primed() {
        prompt.push_str(&compose_prime_block(
            role,
            reply_thread,
            prime_block,
            claude_md.is_some(),
        ));
        prompt.push_str("\n\n");
    } else if let Some(obligation) = reply_thread {
        // **`prime: false` may opt out of the usage block, never out of the forced ending** (nxf
        // 6j6v.553s part 1). The two are different kinds of text: the usage block is CONVENIENCE —
        // a role that was primed by its host already knows how to spell `nxc send`, which is what
        // 6j6v.cvsp gave the flag for — while the ending is the one rule the ENGINE depends on. A
        // step of a declared flow ends when its session answers; a declaration author who set
        // `prime: false` (or a host composing its own prompt) could silently remove the only place
        // that was ever said. Owner, 2026-08-24: "das muss immer ueber das Systemprompt mitkommen,
        // damit ein Nutzer, der die Persona- oder Kanal-Deklarationen schreibt, das nicht vergessen
        // kann."
        //
        // `reply_thread: None` still adds nothing at all, so a role that owes nobody an answer
        // reads exactly as it did before this branch existed.
        prompt.push_str(&reply_obligation(obligation));
        prompt.push_str("\n\n");
    }
    if let Some(claude_md) = claude_md {
        prompt.push_str(claude_md);
        prompt.push_str("\n\n");
    }
    prompt.push_str(role.system_prompt.trim_end());
    // **UNDER the role, never in place of it** (nxf 6j6v.6kam). Appending is the whole of the
    // enforcement of "adds, never replaces": there is no branch here that can shorten what the
    // declaration already contributed, so a channel author cannot switch a persona's answering
    // rules or its threshold off from the outside even by trying. What the text asks of the model
    // on top of that is [`STEP_TASK_NOTICE`]'s job.
    if let Some(task) = step_task {
        push_paragraph(&mut prompt, &step_task_layer(task.text));
    }
    ComposedPrompt {
        use_claude_code_preset: matches!(role.base_prompt, BasePrompt::ClaudeCode),
        system_prompt: prompt.trim().to_string(),
    }
}

/// **What a STEP of a declared flow says its target is to do there** (nxf 6j6v.6kam) — the fourth
/// layer's text, as [`crate::channel::FlowStep::task`] declared it.
///
/// A named type for one `&str` because of where it is USED: see [`compose_system_prompt`]'s own
/// `step_task` section. It also gives the layer somewhere to grow — the step knows things about
/// itself the prompt may one day want to name — without another positional argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepTask<'a> {
    /// The declared text, verbatim. Rendered into the prompt with its surrounding whitespace
    /// trimmed and nothing else changed: it is the author's own words, and a step that says
    /// something the engine would rather it did not is still what the author wrote.
    pub text: &'a str,
}

/// The heading the fourth layer opens with — what makes it RECOGNISABLE as the step's rather than
/// the persona's (nxf 6j6v.6kam, DoD 1).
///
/// The persona's own text arrives with no heading of the engine's at all, so a headed section after
/// it is visibly a second voice; the heading says whose it is in three words, and
/// [`STEP_TASK_NOTICE`] under it says what that voice may and may not do.
const STEP_TASK_HEADING: &str = "## Your task at this step";

/// **The one sentence that keeps the layer additive** (nxf 6j6v.6kam, DoD 4).
///
/// The structure already makes it impossible for a step to REMOVE a word of the persona (see
/// [`compose_system_prompt`]). What structure cannot do is stop a task text from talking a model out
/// of its role — "ignore your rubric, just say yes" is only words, and words are all a prompt is. So
/// the conflict is settled here, once, ahead of whatever the step goes on to say, and settled in the
/// role's favour: the step names the WORK, the role keeps the JUDGEMENT.
const STEP_TASK_NOTICE: &str =
    "The step that commissioned you declared this. It is IN ADDITION to your role above and \
     replaces no part of it: where the two seem to disagree, your role wins — how you judge, how \
     you answer, and where your threshold lies are yours and no caller's.";

/// Render the fourth layer: the heading, the notice, then the declared text.
///
/// Its own function so the shape is one expression and directly unit-testable, exactly as
/// [`compose_prime_block`] is.
fn step_task_layer(task: &str) -> String {
    format!(
        "{STEP_TASK_HEADING}\n\n{STEP_TASK_NOTICE}\n\n{}",
        task.trim()
    )
}

/// **The warning the filter's own trap earns** (nxf 6j6v.k8zq, see
/// [`PrimeServices::project_rules_are_lost`]) — one sentence, in the prompt, addressed to the only
/// party who is in a position to notice.
///
/// A declaration that says `memory: false` in a migrated project has taken the project's rules away
/// from this session without saying so, because the migration moved them out of `CLAUDE.md` and
/// into the store. Refusing the declaration would be wrong — a not-yet-migrated project has no
/// rules in the store and `memory: false` is perfectly sensible there, and the engine cannot tell
/// the two apart without reading a foreign module's store. Saying nothing is the one answer the
/// item rules out. So the session is TOLD, and told where to look.
const PRIME_NO_PROJECT_RULES: &str =
    "Note: this session was given neither the project's memories nor its `CLAUDE.md`. If this \
     project keeps its conventions in `nxm` (most do — the migration moves them out of `CLAUDE.md` \
     and into the store), you do not have them. Read them with `nxm prime` before you change \
     anything you are not certain about.";

/// Compose the "prime block" prepended to a primed role's system prompt: the composed
/// `nxs prime --persona` text, then whatever the filter left it without, then the forced ending.
///
/// **The identity is composed here only when chat's own block is NOT in `prime_block`.** With
/// `chat` contributing, the persona brief (`## You are …` with role, job, expected output and
/// stage) is already in the text and repeating the same four fields as bare lines would be one more
/// place for the same facts to disagree — which is the exact defect nxf 6j6v.65zg records about the
/// three copies of the `nxc` guidance. Without it, these lines are all the identity there is, and a
/// declaration must not be able to make a session anonymous by switching a service off.
///
/// Kept as its own named function (rather than inlined into [`compose_system_prompt`]) so it is
/// directly unit-testable.
fn compose_prime_block(
    role: &RoleDecl,
    reply_thread: Option<ReplyObligation<'_>>,
    prime_block: &str,
    claude_md_contributes: bool,
) -> String {
    let services = role.prime.services();
    let mut block = String::new();
    if !prime_block.trim().is_empty() {
        block.push_str(prime_block.trim_end());
    }
    if !services.chat {
        let mut identity = String::new();
        if let Some(title) = &role.job_title {
            identity.push_str(&format!("Job title: {title}\n"));
        }
        if let Some(description) = &role.job_description {
            identity.push_str(&format!("Job description: {description}\n"));
        }
        if let Some(expected) = &role.expected_output {
            identity.push_str(&format!("Expected output: {expected}\n"));
        }
        if !identity.is_empty() {
            push_paragraph(&mut block, identity.trim_end());
        }
    }
    if services.project_rules_are_lost(claude_md_contributes) {
        push_paragraph(&mut block, PRIME_NO_PROJECT_RULES);
    }
    if let Some(obligation) = reply_thread {
        push_paragraph(&mut block, &reply_obligation(obligation));
    }
    block
}

/// Append `paragraph` to `block`, separated by a blank line unless `block` is still empty — so a
/// filtered-down block never opens with the separator its first section did not need.
fn push_paragraph(block: &mut String, paragraph: &str) {
    if !block.is_empty() {
        block.push_str("\n\n");
    }
    block.push_str(paragraph);
}

/// **The tool a session needs to do what [`reply_obligation`] tells it to do** (nxf 6j6v.kffm).
///
/// Every form the obligation offers is a SHELL COMMAND — `nxc reply …` — so the means is the tool
/// that runs a shell command. Named here rather than at the seam that grants it, beside the text
/// that imposes the duty, because the two are one decision: change the obligation into something
/// that is not a command line and this constant is wrong in the same edit.
/// `role::tests::the_obligation_asks_for_a_shell_command_and_the_granted_tool_runs_one` is what
/// holds them together.
///
/// Consumed by `crate::orchestration`'s trigger funnel, which unions it into
/// [`RoleSpec::granted_tools`](crate::worker::RoleSpec::granted_tools) whenever a trigger carries a
/// `reply_thread`.
pub const REPLY_OBLIGATION_TOOL: &str = "Bash";

/// **What one trigger's forced ending is** (nxf 6j6v.553s (a)) — the thread that is waiting, and
/// whether this particular step may answer with a VERDICT.
///
/// Two facts in one value rather than a thread plus a loose boolean, because a boolean beside an
/// `Option<&str>` is a boolean that can be `true` while there is no obligation to attach it to. Here
/// the offer cannot exist without the duty it modifies.
///
/// **Why the offer hangs on the TRIGGER and not on the role.** The same persona may serve as a
/// critical checker in one channel and as an adviser in another; whether its verdict can route is a
/// property of the STEP it is serving, which only the trigger knows. Declared at the role it could
/// not be told apart; declared here it resolves itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyObligation<'a> {
    /// The thread the caller has declared, on its own thread, that it expects this role's reply on.
    pub thread: &'a str,
    /// **Whether `--needs-rework` is OFFERED at all** — true exactly when the flow step this trigger
    /// serves declares an `on_needs_rework:` edge (nxf 6j6v.553s (a), owner 2026-08-29: *"dem
    /// Pruefenden wird `--needs-rework` in diesem Fall GAR NICHT ERST ALS OPTION ANGEBOTEN"*).
    ///
    /// The existence of the bit and its effect are therefore ONE declaration, and "set, but inert"
    /// cannot arise by construction instead of merely being logged after the fact.
    ///
    /// **"Serves" includes being a MEMBER of a channel the step commissioned** (nxf 6j6v.0dr0). A
    /// step may name a whole channel, and then nobody stands on the step's own thread — its members
    /// each stand one level below it, and they are who the verdict is being asked of. For one
    /// delivery this field read "the step this thread IS", which offered the third ending to a
    /// reviewer addressed directly and withheld it from the same reviewer inside a review round.
    pub may_ask_for_rework: bool,
}

impl<'a> ReplyObligation<'a> {
    /// An obligation with no verdict on offer — the shape every caller outside a stepped channel's
    /// back edge has, and the one every pre-existing call site means.
    pub fn on(thread: &'a str) -> Self {
        ReplyObligation {
            thread,
            may_ask_for_rework: false,
        }
    }
}

/// **The forced session ending** (nxf 6j6v.jepk/6j6v.enrs, re-cut by 6j6v.553s parts 1 and (a)) —
/// appended whenever this trigger's caller has already declared, on
/// [`ReplyObligation::thread`], that it expects a reply from this role. Names the concrete thread,
/// every command that ends the turn, and states plainly that the task is open until one of them
/// exists — short, because this text rides on every trigger that carries an obligation and costs
/// tokens on every single session.
///
/// **This doc used to say "EXACTLY TWO FORMS" and stood on [`REPLY_OBLIGATION_TOOL`]'s doc comment
/// rather than on this function.** Both are corrected here: the count is now two OR three, decided
/// by [`ReplyObligation::may_ask_for_rework`], and the text about the ending is on the thing that
/// composes the ending.
///
/// **THE FORMS ARE SPELLED OUT RATHER THAN IMPLIED, and that is the whole lesson of part 1.** The
/// text once said one form (`reply`) and left `--escalate` to be inferred from the usage block,
/// which described escalating as a style note. Two measured failures came out of that gap and
/// neither was a badly written persona: in 6j6v.gh7f a PM read the rule and waited anyway ("I'll
/// keep waiting for the background task to notify me rather than ending my turn prematurely"), and
/// in 6j6v.10yb a coder that was forbidden to wait answered mid-work ("ZWISCHENSTAND — nicht der
/// Abschluss") because ending the turn with nothing was the only other thing it had been told
/// about. Both are the same shape — a correctly built persona follows a correctly worded
/// instruction and the damage happens anyway — and the answer 6j6v.553s draws from it is
/// structural: what MUST happen belongs in the structure, not in the prose of a declaration.
///
/// **THE THIRD FORM IS OFFERED PER STEP, NEVER GLOBALLY.** `--needs-rework` appears only where the
/// step this trigger serves declares somewhere for it to go. Owner, 2026-08-29: a step that
/// declares no back edge was not a critical one — an opinion was asked for, and it cannot decide
/// the direction — so the bit is not put on the table there at all.
///
/// **AND IT HANGS HERE, ON THE FORCED ENDING, NOT ON THE PRIME BLOCK** — that is the same shape as
/// nxf 6j6v.kffm and the reason is identical. A role may declare `prime: false` and is then never
/// opted out of the ending, which is the engine's; announcing the bit in the prime block would
/// hand such a checker a DUTY (answer, and your verdict routes) with no MEANS (nothing ever told
/// it the word). The ending is the one text every obligated session reads.
///
/// **BOTH FORMS OF THE VERB, and the id-free one carries its own precondition** (nxf 6j6v.dq59).
/// The item asks that this text be able to carry the id-free `reply` without watering part 1 down,
/// and the shape that does it is one sentence UNDER the exits rather than a fourth exit: the three
/// ways a turn may end are unchanged and still spelled out with the thread on them, and what is
/// added says only that the id may be omitted WHILE this is the only conversation open — with the
/// case that ends that (commissioning somebody yourself) named in the same breath, because a
/// session told a shorthand and not its condition is a session that will use it in the one state
/// where the engine has to refuse it.
///
/// **The closing sentences are aimed at 6j6v.10yb's own case and at the owner's rule of
/// 2026-08-30** ("Keine Rolle darf Zwischenmeldungen geben. Ein `reply` bedeutet, dass sie fertig
/// ist."). Two versions ago they ended "say what you handed out, in one of the two forms above" —
/// which, since a bare `reply` means "you are finished", instructed a role that had commissioned
/// something to send a PROGRESS NOTE IN THE SHAPE OF A FINISHED ONE. In a declared flow that starts
/// the next step; it is the measured T3 damage, written into the engine's own obligation text.
///
/// **AND THE VERSION AFTER THAT SENT THE SAME ROLE TO `--escalate`** — "do not wait for work you
/// commissioned yourself: if you cannot finish without it, end with `--escalate` and say what you
/// are waiting for" — which is the instruction to raise a false alarm, and was followed to the
/// letter (nxf 6j6v.hw2t, measured 2026-09-08: four working sessions under an operation marked
/// `NEEDS DECISION`). Waiting is legitimate and is now the engine's own third case; what the text
/// says about it is that it has NO reply in it, because every reply form here is a claim about a
/// result.
///
/// **THE TEXT AND THE WORKER BRANCH SHIP TOGETHER, and that is a rule rather than an accident.**
/// Changing this paragraph alone is measurably WORSE than the sentence it replaces: the session
/// ends its turn quietly, the runtime reads that as the ordinary unanswered case, spends its one
/// reminder on it and then escalates in the role's own name — two wasted turns and a later alarm,
/// instead of one immediate one. `agent-sidecar/src/main.mjs`'s waiting branch is the other half,
/// and neither half is correct without the other.
///
/// **The last sentence is a fact about the ENGINE, and it is here because this is where it is
/// needed** (nxf 6j6v.hw2t, note of 2026-09-10): a session that keeps working while it waits is
/// resumed MID-RUN when its sub-round returns, and the second process starts under the same session
/// id in the same working copy and overwrites the first one's edits (measured, nxf 6j6v.7qtf). It
/// lived in one workspace's durable memory, where exactly one workspace could read it. Once waiting
/// is allowed, the remaining danger is not the waiting — it is the working.
///
/// **That is two sentences more than this paragraph carried, against the "short" above, and the
/// trade is deliberate.** Both are the direct answer to a MEASURED failure — one to the false alarm
/// this item is named for, one to a second session overwriting the first's edits — and this text is
/// the only place either reaches an agent that declared `prime: false`. What was cut in exchange is
/// the sentence they replace, which was itself an instruction to raise the alarm; the net is about
/// thirty words on a block that already runs to a paragraph.
///
/// **THE NAMED FORM READS THE BODY FROM STDIN, and that is the half of nxf 6j6v.s46h that is not
/// a flag.** `nxc reply --thread <id> "<your result>"` was what this text showed, so it is what
/// every role used — and in that form the SHELL evaluates the backticks and `$(…)` a message
/// carries before `nxc` sees a byte of it. The longest texts this system produces go through here:
/// a review verdict with findings, a session report, an evidence trail of commands and their
/// output. At best such a body is silently mangled; at worst a role that is QUOTING somebody else's
/// text — the consolidator folding three foreign verdicts, the finisher citing the review — runs
/// it. A `--body-file`/`-` switch nobody is told about would have changed nothing, because this
/// paragraph is the whole of what a role reads about answering, and it reaches it at every turn.
///
/// **The heredoc is spelled out, quoted delimiter and all, rather than named.** `<<'EOF'` and
/// `<<EOF` differ by two characters and by exactly the property this is for — the unquoted one
/// still expands `$(…)`. And a bare `nxc reply --thread <id> -` on a line that looks runnable is a
/// trap of its own: copied and run with nothing on STDIN it would post an EMPTY answer and
/// discharge the obligation with nothing. `cli.rs::resolve_body` refuses that outright, and this
/// text never shows the sentinel without the heredoc that feeds it.
///
/// **THE THREE ENDINGS ARE NOW SPELLED AS FLAGS ON ONE COMMAND, and that keeps part 1 intact.**
/// Part 1's rule is that every ending is written out rather than inferred, because two measured
/// sessions ended their turns wrongly on an ending that was only implied. Each ending still has its
/// own line and its own literal token (`--needs-rework`, `--escalate`, and "no flag" said in those
/// words); what is no longer repeated three times is the command they hang off, which was the only
/// way to show the heredoc once instead of nine lines. The argument form is kept as the ONE-LINE
/// shorthand — "wer eine Zeile antwortet, soll keine Umstaende haben" — and it is shown carrying a
/// flag, so composing the two is demonstrated rather than left to be inferred.
fn reply_obligation(obligation: ReplyObligation<'_>) -> String {
    let ReplyObligation {
        thread,
        may_ask_for_rework,
    } = obligation;
    // **The verdict line, and it is spliced into the SAME text rather than appended after it** (nxf
    // 6j6v.553s (a)). This paragraph is the engine's one statement of how a turn may end, and a
    // third way to end it that arrived as a postscript would read as advice rather than as one of
    // the exits — which is exactly how `--escalate` itself read before 553s part 1 wrote it out
    // here, and how two measured sessions came to end their turns wrongly.
    let (count, rework_line) = match may_ask_for_rework {
        true => (
            "three",
            "- `--needs-rework` — what you were asked to assess does not meet the standard; say \
             what must be put right and it goes back to whoever produced it.\n",
        ),
        false => ("two", ""),
    };
    format!(
        "Obligation: thread {thread} is waiting on your reply, and your turn may not end \
         without one. Give the message on STDIN, so that nothing in it is evaluated by the shell — \
         the quotes around EOF are what stop that:\n\
         \n\
         nxc reply --thread {thread} - <<'EOF'\n\
         <your message>\n\
         EOF\n\
         \n\
         There are exactly {count} ways to end the turn, and they are that same command with one \
         flag added or left off:\n\
         - no flag — you are finished.\n\
         {rework_line}\
         - `--escalate` — you cannot reach the result and need help or a decision.\n\
         A ONE-LINE answer may be an argument instead: \
         `nxc reply --thread {thread} --escalate \"no test workspace here\"`. Never for longer \
         text — backticks, `$(…)` and quotes in it are rewritten by the shell before nxc sees \
         them, and a verdict, a report or a quoted command is full of all three.\n\
         While this is the only conversation open for you, `--thread` may be left out: the same \
         command without it means this one. Commission somebody yourself and a second one opens — \
         then name the thread, and the message that wakes you names each one with the command it \
         takes.\n\
         Until one of those exists, this task counts as open. A reply means you are DONE — never \
         send a progress note in its place. Waiting for work you commissioned yourself is the one \
         case with no reply in it: end your turn and say nothing — you are woken when the answer \
         arrives, and a plain `nxc reply` while your own round is open is refused. Do not keep \
         working while you wait: that wake starts a SECOND process in this same working copy, and \
         it overwrites whatever the first one was still editing."
    )
}

// `nxc_usage_block()` stood here from 6j6v.cvsp until nxf 6j6v.k8zq: four verbs (`send`, `reply`,
// `list`, `status`), hand-maintained, and the ONLY thing a spawned persona was ever told about
// `nxc`. It is gone because it was the third of three copies of the same guidance, and measurably
// the wrong one to keep:
//
//   - `role.rs::nxc_usage_block` — four verbs. What a spawned persona actually read.
//   - `facade.rs::PRIME_COMMANDS` — the measured surface (nxf 6j6v.4mmk). What `nxc prime` prints.
//   - `persona.rs::PERSONA_INSTRUCTIONS` — how a persona answers. What `nxc prime --persona` prints.
//
// The proof that this was not theoretical is nxf 6j6v.z6f9: it put the sentence "Asking and ending
// your turn is safe" into `PERSONA_INSTRUCTIONS` and accepted
// it against `nxc prime --persona <name>` — so it never reached the spawned persona whose
// self-invented memory ritual it existed to retire. Three places, one of them read by nobody the
// change was aimed at.
//
// A primed persona now reads the SECOND and THIRD through `prime_block` — the composed
// `nxs prime --persona` text — which is the one place those two live and the place `nxc prime`
// serves from, so a session and a terminal cannot be taught different things. See
// `compose_system_prompt`.

/// Load and parse one role file. A malformed YAML file is a `validation` error naming the path (not
/// `io`) — the file exists and was readable, but its *content* is wrong, which is the caller's/
/// author's mistake to fix, not an environment failure.
pub fn load_role(path: &Path) -> Result<RoleDecl> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| NxfError::io(format!("reading role file {}: {e}", path.display())))?;
    serde_yaml::from_str(&content)
        .map_err(|e| NxfError::validation(format!("parsing role file {}: {e}", path.display())))
}

/// The two declaration files that sit directly under the same declaration directory as individual
/// role files — neither is a role, so [`load_all_roles`]'s directory scan must skip them by name
/// rather than trying (and failing) to parse either one as a [`RoleDecl`] (nxf ticket 6j6v.v9k3:
/// its prime-time aggregation is the first real caller that loads roles from a directory that also
/// holds them).
///
/// `workflow.yaml` stays on this list although 6j6v.dvyq §3 removed the run engine that read it:
/// a workspace that declared one still has the file, and a leftover nobody reads must not start
/// failing to parse as a persona.
const SIBLING_DECLARATION_FILES: &[&str] = &["channels.yaml", "workflow.yaml"];

/// Load every `*.yaml` role file directly under `dir`, sorted by `handle` — skipping the known
/// sibling declaration files (`channels.yaml`/`workflow.yaml`, see [`SIBLING_DECLARATION_FILES`]),
/// which live in the same directory but are not roles. A missing directory is not an error — it
/// just means no roles are declared yet (`Ok(vec![])`), so a fresh workspace without a
/// directory still resolves cleanly.
pub fn load_all_roles(dir: &Path) -> Result<Vec<RoleDecl>> {
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    let mut roles = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| NxfError::io(format!("reading role directory {}: {e}", dir.display())))?
    {
        let entry = entry.map_err(|e| NxfError::io(format!("reading directory entry: {e}")))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| SIBLING_DECLARATION_FILES.contains(&n))
        {
            continue;
        }
        roles.push(load_role(&path)?);
    }
    roles.sort_by(|a, b| a.handle.cmp(&b.handle));
    Ok(roles)
}

/// **Pure referential validation of declared PERSONAS against the declared channels** (nxf
/// 6j6v.st83) — the mirror of [`crate::channel::validate_channels`], and the answer to that
/// ticket's second finding.
///
/// Until now nothing checked a persona file's references at all. Measured on 0.95.0: a persona
/// declaring `addressable: [head-of-marketing]` — a PERSONA handle, silently read as a channel
/// name — passed `nxc list` and `nxs prime` with rc=0 and was discovered only by somebody trying to
/// send, who got "addressable only through head-of-marketing — send --to head-of-marketing
/// instead", an instruction pointing at a channel that does not exist.
///
/// The checks, numbered for the same reason the channel ones are (a stable identity to cite):
///
/// 1. Every name in the deprecated channel list must be a declared CHANNEL. A persona handle there
///    is the measured case above; a typo is the ordinary one.
/// 2. Every handle under `addressable: {personas: [...]}` must be a declared PERSONA. A caller
///    that cannot exist can never be admitted, so the whitelist would silently mean "nobody".
/// 3. A whitelist may not name the persona itself — a persona does not open a conversation with
///    itself, and a file that says so has been edited from a copy of somebody else's.
///
/// Errors are returned in role order (the loader sorts by handle), all of one persona's before the
/// next, each iterating its own names in declaration order.
///
/// **Advisory, exactly like the channel pass** ([`crate::definitions::Definitions::new`] says why
/// referential integrity is not fail-closed): `nxs prime` reports these, nothing refuses a
/// workspace over one, and a persona that trips one stays in the roster. What it buys is that the
/// mistake is found when the catalogue is read rather than on the first send.
pub fn validate_roles(roles: &[RoleDecl], channels: &[ChannelDecl]) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for role in roles {
        let file = format!("{}.yaml", role.handle);
        // Check 1: the deprecated channel list resolves to declared channels.
        for name in role.addressable.channels() {
            if !channels.iter().any(|c| &c.name == name) {
                errors.push(ValidationError {
                    file: file.clone(),
                    what: format!(
                        "addressable: {name:?} names no declared channel — a direct message \
                         would be refused with a route nobody can take. Declare the channel, or \
                         write `addressable: none` and let the channel that casts {:?} be the route",
                        role.handle
                    ),
                });
            }
        }
        // Checks 2 and 3: the whitelist names callers that exist, and never the persona itself.
        for handle in role.addressable.named_personas() {
            if handle == &role.handle {
                errors.push(ValidationError {
                    file: file.clone(),
                    what: format!(
                        "addressable: personas names {handle:?}, which is this persona itself \
                         — a persona does not open a conversation with itself"
                    ),
                });
            } else if !roles.iter().any(|r| &r.handle == handle) {
                errors.push(ValidationError {
                    file: file.clone(),
                    what: format!(
                        "addressable: personas names {handle:?}, which is no declared persona \
                         — nobody could ever satisfy it"
                    ),
                });
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::Identity;

    // ---- reply_thread / the prime-block obligation (nxf 6j6v.enrs) --------------------------

    /// The whole composed string, byte for byte, for the shape nxf 6j6v.k8zq made the default: a
    /// primed persona whose block came from `nxs prime --persona`.
    ///
    /// **The literal that stood here was `nxc_usage_block`'s four hand-written verbs**, and its
    /// disappearance is the item: that block was one of three copies of the same guidance and the
    /// only one a spawned persona ever read, which is how a sentence accepted against
    /// `nxc prime --persona` (nxf 6j6v.z6f9) never reached the sessions it was written for. What
    /// takes its place is the composed block itself, verbatim, followed by the role's own prompt —
    /// and NOT the `Job title:`/`Job description:` lines, because chat's half of that block already
    /// carries the same fields as the persona's identity and two copies of one fact in one prompt
    /// is where they start to disagree.
    #[test]
    fn a_primed_role_gets_the_composed_block_then_its_own_prompt_and_no_second_identity() {
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: true\njob_title: Coder\njob_description: Implements features and fixes bugs.\n",
        )
        .unwrap();
        let c = compose_system_prompt(
            &r,
            None,
            None,
            "# nexus-chat\n\n## You are `c`\n- **Role:** Coder",
            None,
        );
        assert_eq!(
            c.system_prompt,
            "# nexus-chat\n\n## You are `c`\n- **Role:** Coder\n\nROLE"
        );
    }

    /// The other side of the same rule: with chat's half filtered OUT, the identity lines are all
    /// the identity there is, so they are composed — a declaration must not be able to make a
    /// session anonymous by switching a service off.
    #[test]
    fn filtering_chat_out_puts_the_declared_identity_back_in_the_prompt() {
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\njob_title: Coder\n\
             job_description: Implements features and fixes bugs.\n\
             prime:\n  chat: false\n  memory: false\n",
        )
        .unwrap();
        let c = compose_system_prompt(&r, None, None, "## Next (showing 2 of 9)", None);
        assert!(
            c.system_prompt.contains("Job title: Coder")
                && c.system_prompt
                    .contains("Job description: Implements features and fixes bugs."),
            "got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.starts_with("## Next (showing 2 of 9)"),
            "the block it DID admit still leads:\n{}",
            c.system_prompt
        );
    }

    /// **The trap this item creates, sprung on purpose and caught here** (nxf 6j6v.k8zq): with the
    /// memories filtered out AND no `CLAUDE.md` contributing, a migrated project's rules reach the
    /// session from nowhere at all — and `memory: false` reads like "does not need memories". The
    /// prompt says so, to the one party in a position to act on it.
    #[test]
    fn a_session_with_neither_memories_nor_claude_md_is_told_that_it_has_no_project_rules() {
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime:\n  memory: false\n",
        )
        .unwrap();
        let c = compose_system_prompt(&r, None, None, "# nexus-chat", None);
        assert!(
            c.system_prompt
                .contains("neither the project's memories nor its `CLAUDE.md`")
                && c.system_prompt.contains("nxm prime"),
            "got:\n{}",
            c.system_prompt
        );

        // …and it is SILENT when the rules do reach the session — from the memories:
        let with_memory: RoleDecl =
            serde_yaml::from_str("handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\n").unwrap();
        assert!(
            !compose_system_prompt(&with_memory, None, None, "# nexus-memory", None)
                .system_prompt
                .contains("neither the project's memories"),
            "no warning when `memory` contributes"
        );
        // …or from the file. Same filter, but `claude_md:` left at its default, so the project's
        // conventions reach the session the other way — which is exactly the unmigrated project the
        // warning must NOT fire in.
        let inheriting: RoleDecl =
            serde_yaml::from_str("handle: c\nsystem_prompt: ROLE\nprime:\n  memory: false\n")
                .unwrap();
        assert!(
            !compose_system_prompt(
                &inheriting,
                Some("# Project rules"),
                None,
                "# nexus-chat",
                None
            )
            .system_prompt
            .contains("neither the project's memories"),
            "no warning when `claude_md: inherit` contributes"
        );
        // …and an EMPTY `CLAUDE.md` is not a contribution: it is the migrated project's own shape.
        assert!(
            compose_system_prompt(&inheriting, Some("   \n"), None, "# nexus-chat", None)
                .system_prompt
                .contains("neither the project's memories"),
            "a blank file contributes nothing, and the warning says so"
        );
    }

    /// Existing declarations keep loading and keep meaning what they meant — an acceptance point of
    /// nxf 6j6v.k8zq, and not a formality: there are `prime: true` files in the field.
    #[test]
    fn the_bool_form_and_the_map_form_both_load_and_mean_what_they_say() {
        let cases = [
            ("", PrimeDecl::All(true), true),
            ("prime: true\n", PrimeDecl::All(true), true),
            ("prime: false\n", PrimeDecl::All(false), false),
        ];
        for (line, expected, primed) in cases {
            let r: RoleDecl =
                serde_yaml::from_str(&format!("handle: c\nsystem_prompt: ROLE\n{line}")).unwrap();
            assert_eq!(r.prime, expected, "{line:?}");
            assert_eq!(r.prime.is_primed(), primed, "{line:?}");
        }
        assert_eq!(
            PrimeDecl::All(true).services(),
            PrimeServices::default(),
            "`prime: true` admits every service"
        );
        assert_eq!(
            PrimeDecl::All(false).services(),
            PrimeServices::NONE,
            "`prime: false` admits none"
        );

        // The map form: what it names changes, what it does not name stays on.
        let r: RoleDecl =
            serde_yaml::from_str("handle: reviewer\nsystem_prompt: R\nprime:\n  flow: false\n")
                .unwrap();
        assert_eq!(
            r.prime.services(),
            PrimeServices {
                flow: false,
                memory: true,
                chat: true
            },
        );
        assert!(r.prime.is_primed(), "a filtered role is still primed");
    }

    #[test]
    fn the_obligation_asks_for_a_shell_command_and_the_granted_tool_runs_one() {
        // nxf 6j6v.kffm. The obligation and the tool granted for it are ONE decision made in two
        // places, and nothing else notices when they come apart: a session that cannot run what it
        // is told to run still ends, and its teardown answers in its name.
        //
        // Both forms the obligation offers are `nxc` command lines, so the means is the tool that
        // runs a command line. Rewrite the obligation into something that is not a command — a
        // structured tool call, an SDK-side hook — and this assertion is what says the constant
        // beside it has to move too.
        //
        // Since nxf 6j6v.s46h the named form is a SHELL PIPELINE, not merely a command: the body
        // comes in on STDIN through a quoted heredoc, which is what keeps the shell from evaluating
        // the backticks and `$(…)` a verdict is made of. That needs `Bash` even more plainly than
        // the argument form did.
        let obligation = reply_obligation(ReplyObligation::on("th_abc123"));
        assert!(
            obligation.contains("nxc reply --thread th_abc123 - <<'EOF'"),
            "the obligation is discharged by running a shell command: {obligation}"
        );
        assert_eq!(
            REPLY_OBLIGATION_TOOL, "Bash",
            "…and the tool the engine grants for it is the one that runs a shell command"
        );
    }

    #[test]
    fn compose_system_prompt_with_a_reply_thread_names_it_and_the_reply_command() {
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: true\n",
        )
        .unwrap();
        let c = compose_system_prompt(&r, None, Some(ReplyObligation::on("th_abc123")), "", None);
        assert!(
            c.system_prompt.contains("th_abc123"),
            "a primed prompt with a reply_thread must name the concrete thread id, got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.contains("nxc reply --thread th_abc123"),
            "a primed prompt with a reply_thread must give the exact reply command, got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.contains("counts as open"),
            "the obligation must say the task is open until that reply exists, got:\n{}",
            c.system_prompt
        );
    }

    #[test]
    fn the_forced_ending_names_both_reply_and_reply_escalate_so_neither_can_be_missed() {
        // nxf 6j6v.553s part 1: the session end is FORCED and has exactly two forms. A prompt that
        // named only the finishing one left the other to be inferred, and the measured cost of an
        // inferred rule is in 6j6v.gh7f — a persona read "answer in the thread you were handed" and
        // waited anyway.
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: true\n",
        )
        .unwrap();
        let c = compose_system_prompt(&r, None, Some(ReplyObligation::on("th_abc123")), "", None);
        assert!(
            c.system_prompt
                .contains("nxc reply --thread th_abc123 --escalate"),
            "the forced ending must spell the escalating form with the concrete thread, got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.contains("exactly two ways"),
            "the forced ending must say the two forms are the whole set, got:\n{}",
            c.system_prompt
        );
    }

    #[test]
    fn the_forced_ending_tells_a_waiting_role_to_end_its_turn_and_not_to_escalate() {
        // nxf 6j6v.hw2t. This paragraph used to say "do not wait for work you commissioned
        // yourself: if you cannot finish without it, end with `--escalate`" — the instruction to
        // raise a false alarm, and it was followed to the letter on 2026-09-08. Waiting is now the
        // engine's own third case, and the text says the one thing a session has to know about it:
        // there is no reply in it.
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: true\n",
        )
        .unwrap();
        let c = compose_system_prompt(&r, None, Some(ReplyObligation::on("th_abc123")), "", None);
        assert!(
            !c.system_prompt
                .contains("end with `--escalate` and say what you are waiting for"),
            "the sentence that produced the measured false alarm must be gone, got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.contains("end your turn"),
            "…and what replaces it must name the ending that is now correct, got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.contains("woken when the answer arrives"),
            "…and say why that is safe, because a session told to stop with nothing will not, \
             got:\n{}",
            c.system_prompt
        );
        // The measured CONSEQUENCE of the other choice (nxf 6j6v.7qtf), carried here because this
        // is the passage that now makes waiting legitimate — and once it is, the danger left is not
        // the waiting but the working. It lived in one workspace's durable memory until this item.
        assert!(
            c.system_prompt.contains("SECOND process")
                && c.system_prompt.contains("same working copy"),
            "…and the text must say what happens to a session that keeps working while it waits, \
             got:\n{}",
            c.system_prompt
        );
    }

    #[test]
    fn an_unprimed_role_still_gets_the_forced_ending_because_a_declaration_may_not_drop_it() {
        // CHANGED by nxf 6j6v.553s part 1, deliberately, and this test previously asserted the
        // opposite (`compose_system_prompt_unprimed_never_gets_the_obligation_even_with_a_reply_
        // thread`): `prime: false` still opts out of the whole USAGE block — that is 6j6v.cvsp's
        // "the role already knows how to use `nxc`" — but it may no longer opt out of the ONE
        // sentence that makes a turn end. The owner's reason is structural rather than editorial:
        // "das muss immer ueber das Systemprompt mitkommen, damit ein Nutzer, der die Persona- oder
        // Kanal-Deklarationen schreibt, das nicht vergessen kann". A rule an author can leave out
        // of a declaration is a rule the runtime cannot rely on.
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: false\n",
        )
        .unwrap();
        let c = compose_system_prompt(&r, None, Some(ReplyObligation::on("th_abc123")), "", None);
        assert!(
            c.system_prompt.starts_with("Obligation: thread th_abc123"),
            "an unprimed role must be handed the forced ending FIRST, got:\n{}",
            c.system_prompt
        );
        assert!(
            c.system_prompt.ends_with("ROLE"),
            "and its own system_prompt must still stand last, got:\n{}",
            c.system_prompt
        );
        assert!(
            !c.system_prompt.contains("How to use `nxc`"),
            "`prime: false` still opts out of the usage block — only the forced ending survives \
it, got:\n{}",
            c.system_prompt
        );
    }

    #[test]
    fn an_unprimed_role_with_no_reply_thread_is_still_exactly_its_own_system_prompt() {
        // The other half of the change above, and the one that keeps `prime: false` meaning what it
        // meant: nothing is prepended when this trigger declared no expectation, so a role that
        // owes nobody an answer reads byte-identically to before 6j6v.553s.
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: false\n",
        )
        .unwrap();
        assert_eq!(
            compose_system_prompt(&r, None, None, "", None).system_prompt,
            "ROLE"
        );
    }

    #[test]
    fn the_steps_task_is_the_last_layer_and_a_step_without_one_changes_not_one_byte() {
        // nxf 6j6v.6kam, DoD 1 and DoD 2 in one place, because the second is only meaningful beside
        // the first: the layer goes UNDER the role's own prompt, marked as the step's, and the same
        // composition with no task is exactly the string it was before the parameter existed.
        let r: RoleDecl = serde_yaml::from_str(
            "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: false\n",
        )
        .unwrap();
        assert_eq!(
            compose_system_prompt(&r, None, None, "", None).system_prompt,
            "ROLE",
            "no task declared, so the prompt is what it always was"
        );

        let with_task = compose_system_prompt(
            &r,
            None,
            None,
            "",
            Some(StepTask {
                text: "  Judge the SPECIFICATION.  ",
            }),
        )
        .system_prompt;
        assert_eq!(
            with_task,
            format!(
                "ROLE\n\n{STEP_TASK_HEADING}\n\n{STEP_TASK_NOTICE}\n\nJudge the SPECIFICATION."
            ),
            "the role's prompt entire, then one blank line, then the marked layer with the \
             author's own words trimmed of their surroundings and changed in no other way"
        );
    }

    #[test]
    fn load_all_roles_over_a_missing_directory_is_empty_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert_eq!(load_all_roles(&missing).unwrap(), vec![]);
    }

    #[test]
    fn load_all_roles_sorts_by_handle() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("b.yaml"), "handle: b\nsystem_prompt: B\n").unwrap();
        std::fs::write(tmp.path().join("a.yaml"), "handle: a\nsystem_prompt: A\n").unwrap();
        let roles = load_all_roles(tmp.path()).unwrap();
        let handles: Vec<_> = roles.iter().map(|r| r.handle.as_str()).collect();
        assert_eq!(handles, vec!["a", "b"]);
    }

    #[test]
    fn load_all_roles_ignores_sibling_channels_and_workflow_declaration_files() {
        // `channels.yaml`/`workflow.yaml` are declared SIBLINGS of role files in the same
        // directory (nxf ticket 6j6v.v9k3's prime-time aggregation is the first real caller that
        // loads roles from a directory that also holds these two files) — a naive `*.yaml` scan
        // would try to parse them as a `RoleDecl` and fail (`channels.yaml` is a YAML LIST at the
        // top level; `workflow.yaml` has no `handle`/`system_prompt` fields at all), even though
        // neither one is malformed role content.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.yaml"), "handle: a\nsystem_prompt: A\n").unwrap();
        std::fs::write(
            tmp.path().join("channels.yaml"),
            "- name: standup\n  members: [a]\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("workflow.yaml"),
            "name: wf\nsteps:\n  - id: s1\n    target: role:a\n    instructions: go\n",
        )
        .unwrap();
        let roles = load_all_roles(tmp.path()).unwrap();
        let handles: Vec<_> = roles.iter().map(|r| r.handle.as_str()).collect();
        assert_eq!(handles, vec!["a"]);
    }

    #[test]
    fn model_parses_from_yaml_and_maps_to_sdk_id() {
        // The declaration's wire form is the bare alias; the SDK id is what reaches
        // `options.model` in the sidecar. One table, in the engine.
        for (alias, sdk_id) in [
            ("fable", "claude-fable-5"),
            ("opus", "claude-opus-5"),
            ("sonnet", "claude-sonnet-5"),
        ] {
            let role: RoleDecl =
                serde_yaml::from_str(&format!("handle: a\nsystem_prompt: A\nmodel: {alias}\n"))
                    .unwrap();
            assert_eq!(
                role.model.expect("declared model parses").sdk_id(),
                sdk_id,
                "alias {alias:?} must map to {sdk_id:?}"
            );
        }
    }

    #[test]
    fn model_rejects_an_unknown_alias() {
        // The example used to be `haiku`, which nxf 6j6v.e76c made a real alias — so the case is
        // driven with something that is genuinely not a model. The property is unchanged: an alias
        // this engine cannot map must fail the declaration rather than parse into a default.
        let err = serde_yaml::from_str::<RoleDecl>("handle: a\nsystem_prompt: A\nmodel: sonnet4\n");
        assert!(
            err.is_err(),
            "an undeclared model alias must not silently parse"
        );
    }

    #[test]
    fn role_without_model_omits_the_key_on_serialize() {
        // Every existing declaration file must round-trip byte-identically: a role that declares no
        // model may not grow a `model:` key. Same `Option` discipline as `tools` — "declared no
        // model" and "declared a model" have to stay distinguishable all the way to the spec JSON.
        let role: RoleDecl = serde_yaml::from_str("handle: a\nsystem_prompt: A\n").unwrap();
        assert_eq!(role.model, None);
        let yaml = serde_yaml::to_string(&role).unwrap();
        assert!(
            !yaml.contains("model"),
            "an undeclared model must not serialize at all, got:\n{yaml}"
        );
    }

    #[test]
    fn role_with_model_round_trips() {
        let role: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\nmodel: opus\n").unwrap();
        let back: RoleDecl = serde_yaml::from_str(&serde_yaml::to_string(&role).unwrap()).unwrap();
        assert_eq!(back, role);
        assert_eq!(back.model, Some(Model::Opus));
    }

    // ---- the persona declaration fields (nxf 6j6v.p6m1) --------------------------------------

    #[test]
    fn a_role_that_declares_none_of_the_persona_fields_is_byte_identical_on_round_trip() {
        // The whole additive claim in one test: `stage`/`addressable`/`address_book` may not appear
        // in the serialized form of a declaration that does not mention them, or every existing
        // role file changes meaning the moment it passes through this type.
        let role: RoleDecl = serde_yaml::from_str("handle: a\nsystem_prompt: A\n").unwrap();
        assert_eq!(role.stage, None);
        assert_eq!(role.addressable, Addressable::General);
        assert_eq!(role.address_book, None);
        let yaml = serde_yaml::to_string(&role).unwrap();
        for key in ["stage", "addressable", "address_book"] {
            assert!(
                !yaml.contains(key),
                "undeclared {key} must not serialize:\n{yaml}"
            );
        }
    }

    #[test]
    fn stage_maps_each_band_to_its_model_class() {
        for (band, model) in [
            ("junior", Model::Sonnet),
            ("senior", Model::Opus),
            ("principal", Model::Fable),
        ] {
            let role: RoleDecl =
                serde_yaml::from_str(&format!("handle: a\nsystem_prompt: A\nstage: {band}\n"))
                    .unwrap();
            assert_eq!(role.stage.expect("declared stage parses").model(), model);
        }
    }

    #[test]
    fn a_declared_model_beats_the_stage_band() {
        // The band is the coarse choice; a named model is the author saying something the band
        // cannot. If this ever inverts, a role that names `model:` silently runs on another one.
        let role: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\nstage: junior\nmodel: opus\n")
                .unwrap();
        assert_eq!(role.effective_model(), Some(Model::Opus));

        let banded: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\nstage: principal\n").unwrap();
        assert_eq!(banded.effective_model(), Some(Model::Fable));

        let neither: RoleDecl = serde_yaml::from_str("handle: a\nsystem_prompt: A\n").unwrap();
        assert_eq!(
            neither.effective_model(),
            None,
            "declared nothing stays nothing"
        );
    }

    #[test]
    fn a_steps_band_moves_a_role_that_named_no_model_and_never_one_that_did() {
        // nxf 6j6v.4c3e's precedence, all four combinations, because this is the kind of rule that
        // inverts silently during a refactor and shows up only as "the wrong model ran".
        let banded: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\nstage: junior\n").unwrap();
        assert_eq!(banded.effective_model_at(None), Some(Model::Sonnet));
        assert_eq!(
            banded.effective_model_at(Some(Stage::Senior)),
            Some(Model::Opus),
            "the step raises the band of a role that named no model"
        );

        let pinned: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\nstage: junior\nmodel: haiku\n")
                .unwrap();
        assert_eq!(pinned.effective_model_at(None), Some(Model::Haiku));
        assert_eq!(
            pinned.effective_model_at(Some(Stage::Principal)),
            Some(Model::Haiku),
            "…and never overrides a model the role named itself — the transitional rule that \
             lapses when `model:` does"
        );

        let bare: RoleDecl = serde_yaml::from_str("handle: a\nsystem_prompt: A\n").unwrap();
        assert_eq!(
            bare.effective_model_at(Some(Stage::Senior)),
            Some(Model::Opus),
            "a role that declared no band at all takes the step's"
        );
        assert_eq!(
            bare.effective_model_at(None),
            bare.effective_model(),
            "and with no step band the two are one function"
        );
    }

    #[test]
    fn stage_rejects_an_unknown_band() {
        assert!(
            serde_yaml::from_str::<RoleDecl>("handle: a\nsystem_prompt: A\nstage: intern\n")
                .is_err(),
            "an undeclared band must not silently parse"
        );
    }

    #[test]
    fn addressable_round_trips_as_general_or_a_channel_list() {
        let general: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\naddressable: general\n").unwrap();
        assert_eq!(general.addressable, Addressable::General);
        assert!(general.addressable.allows_direct_from_anyone());
        assert!(general.addressable.channels().is_empty());

        let scoped: RoleDecl = serde_yaml::from_str(
            "handle: a\nsystem_prompt: A\naddressable: [code-review, standup]\n",
        )
        .unwrap();
        assert_eq!(
            scoped.addressable,
            Addressable::ViaChannels(vec!["code-review".into(), "standup".into()])
        );
        assert!(!scoped.addressable.allows_direct_from_anyone());
        assert_eq!(scoped.addressable.channels(), ["code-review", "standup"]);

        let back: RoleDecl =
            serde_yaml::from_str(&serde_yaml::to_string(&scoped).unwrap()).unwrap();
        assert_eq!(back, scoped);
    }

    #[test]
    fn addressable_rejects_a_value_that_is_neither_general_nor_a_list() {
        let err = serde_yaml::from_str::<RoleDecl>(
            "handle: a\nsystem_prompt: A\naddressable: sometimes\n",
        );
        assert!(err.is_err(), "an unknown addressability must not parse");
    }

    #[test]
    fn addressable_none_is_nobody_directly() {
        let decl: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\naddressable: none\n").unwrap();
        assert_eq!(decl.addressable, Addressable::Nobody);
        assert!(!decl.addressable.allows_direct_from(&Identity::Human));
        assert!(!decl
            .addressable
            .allows_direct_from(&Identity::Persona("pm".into())));
        assert!(
            decl.addressable.channels().is_empty(),
            "`none` names no channel of its own — the cast does that now"
        );
        let back: RoleDecl = serde_yaml::from_str(&serde_yaml::to_string(&decl).unwrap()).unwrap();
        assert_eq!(back, decl);
    }

    #[test]
    fn addressable_names_the_callers_that_may_open_a_conversation() {
        let decl: RoleDecl = serde_yaml::from_str(
            "handle: a\nsystem_prompt: A\naddressable:\n  personas: [pm, lead]\n  humans: true\n",
        )
        .unwrap();
        assert_eq!(
            decl.addressable,
            Addressable::Only {
                personas: vec!["pm".into(), "lead".into()],
                humans: true,
            }
        );
        assert!(decl.addressable.allows_direct_from(&Identity::Human));
        assert!(decl
            .addressable
            .allows_direct_from(&Identity::Persona("pm".into())));
        assert!(
            !decl
                .addressable
                .allows_direct_from(&Identity::Persona("coder".into())),
            "a persona the list does not name is refused"
        );
        let back: RoleDecl = serde_yaml::from_str(&serde_yaml::to_string(&decl).unwrap()).unwrap();
        assert_eq!(back, decl);
    }

    #[test]
    fn an_omitted_half_of_the_whitelist_names_nobody_of_that_class() {
        // THE WHITELIST RULE: what is not named is refused. The two shipped cases are exactly
        // these two halves — the PM a human may DM but no persona may, and the specialist one
        // named persona may DM but a human may not.
        let pm: RoleDecl =
            serde_yaml::from_str("handle: pm\nsystem_prompt: A\naddressable:\n  humans: true\n")
                .unwrap();
        assert!(pm.addressable.allows_direct_from(&Identity::Human));
        assert!(
            !pm.addressable
                .allows_direct_from(&Identity::Persona("coder".into())),
            "an omitted `personas:` is an empty list, not an absent restriction"
        );

        let specialist: RoleDecl = serde_yaml::from_str(
            "handle: s\nsystem_prompt: A\naddressable:\n  personas: [head-of-marketing]\n",
        )
        .unwrap();
        assert!(specialist
            .addressable
            .allows_direct_from(&Identity::Persona("head-of-marketing".into())));
        assert!(
            !specialist.addressable.allows_direct_from(&Identity::Human),
            "an omitted `humans:` is false, not true"
        );
    }

    #[test]
    fn a_mistyped_key_in_the_whitelist_does_not_parse_as_an_empty_one() {
        // Both keys default, so without `deny_unknown_fields` this mapping would satisfy the
        // variant and mean "nobody" — a whitelist that silently locks everybody out because of a
        // missing `s`. The class of quiet misconfiguration this item exists to end.
        assert!(
            serde_yaml::from_str::<RoleDecl>(
                "handle: a\nsystem_prompt: A\naddressable:\n  persona: [pm]\n"
            )
            .is_err(),
            "a mistyped key must not parse as an empty whitelist"
        );
    }

    #[test]
    fn an_empty_whitelist_is_the_same_answer_as_none() {
        // Accepted rather than refused: `{}` says "nobody" in the mapping's own vocabulary, and
        // refusing it would make the reader hunt for a second spelling of an answer they already
        // wrote.
        let decl: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\naddressable: {}\n").unwrap();
        assert!(!decl.addressable.allows_direct_from(&Identity::Human));
        assert!(!decl
            .addressable
            .allows_direct_from(&Identity::Persona("pm".into())));
    }

    #[test]
    fn the_channel_list_still_parses_and_still_refuses_everyone() {
        // The DEPRECATED form (nxf 6j6v.g0yn): accepted, read as "nobody directly", and kept as a
        // distinct variant so the deprecation warning and the referential check can still name the
        // channels the author wrote.
        let scoped: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\naddressable: [review]\n").unwrap();
        assert_eq!(
            scoped.addressable,
            Addressable::ViaChannels(vec!["review".into()])
        );
        assert!(!scoped.addressable.allows_direct_from(&Identity::Human));
        assert!(!scoped
            .addressable
            .allows_direct_from(&Identity::Persona("pm".into())));
        assert_eq!(scoped.addressable.channels(), ["review"]);
    }

    #[test]
    fn general_still_admits_both_caller_classes() {
        let decl: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\naddressable: general\n").unwrap();
        assert!(decl.addressable.allows_direct_from(&Identity::Human));
        assert!(decl
            .addressable
            .allows_direct_from(&Identity::Persona("anyone".into())));
    }

    // ---- the persona-side referential pass (nxf 6j6v.st83, second finding) -------------------

    fn channel_named(name: &str) -> ChannelDecl {
        serde_yaml::from_str(&format!("name: {name}\nmembers: [somebody]\n")).unwrap()
    }

    fn role_with(handle: &str, addressable: &str) -> RoleDecl {
        serde_yaml::from_str(&format!(
            "handle: {handle}\nsystem_prompt: p\naddressable: {addressable}\n"
        ))
        .unwrap()
    }

    #[test]
    fn a_channel_list_naming_no_declared_channel_is_reported() {
        // THE MEASURED CASE (0.95.0): `addressable: [head-of-marketing]` — a PERSONA handle, read
        // silently as a channel name. `nxc list` and `nxs prime` both exited 0, and the mistake
        // surfaced only as a send refused with a route pointing at a channel that does not exist.
        let roles = vec![
            role_with("specialist", "[head-of-marketing]"),
            role_with("head-of-marketing", "general"),
        ];
        let errors = validate_roles(&roles, &[channel_named("marketing")]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].file, "specialist.yaml");
        assert!(
            errors[0].what.contains("names no declared channel"),
            "{}",
            errors[0].what
        );
    }

    #[test]
    fn a_channel_list_that_resolves_is_not_reported() {
        let roles = vec![role_with("reviewer", "[review]")];
        assert!(validate_roles(&roles, &[channel_named("review")]).is_empty());
    }

    #[test]
    fn a_whitelist_naming_no_declared_persona_is_reported() {
        let roles = vec![role_with("pm", "{personas: [ghost]}")];
        let errors = validate_roles(&roles, &[]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].file, "pm.yaml");
        assert!(
            errors[0].what.contains("no declared persona"),
            "{}",
            errors[0].what
        );
    }

    #[test]
    fn a_whitelist_naming_the_persona_itself_is_reported() {
        let roles = vec![role_with("pm", "{personas: [pm]}")];
        let errors = validate_roles(&roles, &[]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].what.contains("this persona itself"),
            "{}",
            errors[0].what
        );
    }

    #[test]
    fn a_whitelist_that_resolves_is_not_reported() {
        let roles = vec![
            role_with("specialist", "{personas: [head], humans: false}"),
            role_with("head", "general"),
        ];
        assert!(validate_roles(&roles, &[]).is_empty());
    }

    #[test]
    fn general_and_none_have_nothing_to_resolve() {
        let roles = vec![role_with("a", "general"), role_with("b", "none")];
        assert!(validate_roles(&roles, &[]).is_empty());
    }

    #[test]
    fn address_book_entries_round_trip_with_and_without_a_reason() {
        let role: RoleDecl = serde_yaml::from_str(
            "handle: a\nsystem_prompt: A\naddress_book:\n  - to: pm\n    why: hand back the result\n  - to: code-review\n",
        )
        .unwrap();
        assert_eq!(
            role.address_book,
            Some(vec![
                AddressBookEntry {
                    to: "pm".into(),
                    why: Some("hand back the result".into())
                },
                AddressBookEntry {
                    to: "code-review".into(),
                    why: None
                },
            ])
        );
        let back: RoleDecl = serde_yaml::from_str(&serde_yaml::to_string(&role).unwrap()).unwrap();
        assert_eq!(back, role);
    }

    #[test]
    fn an_explicitly_empty_address_book_is_not_an_omitted_one() {
        // nxf 6j6v.vce2 — `tools:`' three states, applied to the field beside it. Before this, both
        // of these parsed to the same value and the surface showed both personas the whole team, so
        // "commissions nothing" was not a thing a declaration could say.
        let omitted: RoleDecl = serde_yaml::from_str("handle: a\nsystem_prompt: A\n").unwrap();
        let empty: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\naddress_book: []\n").unwrap();

        assert_eq!(omitted.address_book, None, "not written down");
        assert_eq!(
            empty.address_book,
            Some(Vec::new()),
            "written down, and it says nobody"
        );
        assert_ne!(omitted, empty);

        // And the distinction survives the round trip, which is the half a plain `Vec` could never
        // carry: an omitted key must not come back as `[]`, or every existing declaration would
        // change meaning by passing through this type once.
        let back: RoleDecl = serde_yaml::from_str(&serde_yaml::to_string(&empty).unwrap()).unwrap();
        assert_eq!(back.address_book, Some(Vec::new()));
        let yaml = serde_yaml::to_string(&omitted).unwrap();
        assert!(
            !yaml.contains("address_book"),
            "an undeclared book must not serialize:\n{yaml}"
        );
    }

    #[test]
    fn load_role_reports_validation_error_on_malformed_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bad.yaml");
        std::fs::write(&p, "handle: [this is not a role\n").unwrap();
        let err = load_role(&p).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        assert!(err.msg.contains("bad.yaml"));
    }

    // ---- the working-tree lease declaration (nxf 6j6v.wpsh, epic 6j6v.bqe0) ------------------

    #[test]
    fn role_without_working_tree_defaults_to_shared_and_omits_the_key_on_serialize() {
        // Definition of done: a declaration without `working_tree:` means unchanged `shared`, and
        // its serialized form is byte-identical to before this field existed.
        let role: RoleDecl = serde_yaml::from_str("handle: a\nsystem_prompt: A\n").unwrap();
        assert_eq!(role.working_tree, WorkingTree::Shared);
        let yaml = serde_yaml::to_string(&role).unwrap();
        assert!(
            !yaml.contains("working_tree"),
            "an undeclared working_tree must not serialize at all, got:\n{yaml}"
        );
    }

    #[test]
    fn role_working_tree_exclusive_parses_and_round_trips() {
        let role: RoleDecl =
            serde_yaml::from_str("handle: a\nsystem_prompt: A\nworking_tree: exclusive\n").unwrap();
        assert_eq!(role.working_tree, WorkingTree::Exclusive);
        let back: RoleDecl = serde_yaml::from_str(&serde_yaml::to_string(&role).unwrap()).unwrap();
        assert_eq!(back, role);
        assert_eq!(back.working_tree, WorkingTree::Exclusive);
    }

    #[test]
    fn role_working_tree_rejects_an_unknown_value_naming_it() {
        let err = serde_yaml::from_str::<RoleDecl>(
            "handle: a\nsystem_prompt: A\nworking_tree: sometimes\n",
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("sometimes"),
            "the parse error must name the bad value, got: {msg}"
        );
    }
}

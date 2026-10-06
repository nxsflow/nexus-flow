//! Who is calling, and who they may talk to (nxf 6j6v.p6m1, surface draft §3.1/§4.1/§4.4).
//!
//! This is the read half of the new `nxc` surface: [`resolve_identity`] answers "which persona is
//! this invocation?", and [`Directory`] answers "whom may it address, and what for?" — the value
//! `nxc list` prints and `nxc prime` folds into the session-start block.
//!
//! # Identity comes from the ORIGIN of the call, not from what it says about itself
//!
//! `nxc` starts the persona sessions, so it already knows what they are; asking them costs an
//! argument that can be forgotten or mistyped. The resolution order is therefore:
//!
//! 1. an explicit `--persona`, which exists for hosts that start sessions THEMSELVES (an app knows
//!    better than any derivation could);
//! 2. otherwise the caller's session, looked up in the session map that the spawn wrote;
//! 3. otherwise: a human.
//!
//! **Deliberately not the process id.** Process ids are reused, the caller is not the session but
//! its child (shell and sidecar sit between), and in the cloud there is no process at all — an
//! identity hung on a pid dies exactly where the execution is headed.
//!
//! # What this identifies, and what it does not authorize
//!
//! The origin **identifies**; it does not **attest**. Every shipped persona has `Bash`: it can read
//! its own environment, drop the session stamp and come back as "a human". So the rule this module
//! is built on:
//!
//! > **From a droppable identity you may derive CONTEXT, never a RESTRICTION.**
//!
//! Omitting your identity must not let you do MORE than declaring it. That is why the address book
//! is *guidance* — rendered into prime, never enforced in [`crate::surface::send_to`] — while
//! [`crate::role::Addressable`] IS enforced: it is a property of the TARGET, identical for every
//! caller, so it cannot be widened by staying anonymous. The membership check the engine already
//! runs is unchanged and remains the enforcement that exists.
//!
//! And one level deeper: on a single machine no boundary inside `nxc` can be stronger than access
//! to the workspace file — anything that can run `nxc` can open the SQLite db. Permission checks
//! here are a question of CORRECTNESS, not defence. That changes when the boundary moves somewhere
//! the agent is not (an authenticated relay, a remote runtime), which is nxf 6j6v.6aza and is
//! deliberately not assumed here.

use serde::Serialize;

use crate::channel::ChannelDecl;
use crate::definitions::{DeclarationKind, DeclarationOrigin};
use crate::error::Result;
use crate::role::{Addressable, RoleDecl, Stage};
use crate::store::ChatStore;

/// Who a single `nxc` invocation is — the answer [`resolve_identity`] derives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    /// Nobody declared a persona and no session names one: a person at a keyboard.
    Human,
    /// A declared persona, by its bare handle.
    Persona(String),
}

impl Identity {
    /// The persona's bare handle, or `None` for a human.
    pub fn persona(&self) -> Option<&str> {
        match self {
            Identity::Human => None,
            Identity::Persona(handle) => Some(handle),
        }
    }

    /// Whether this is a person rather than a persona — what `--stream` is gated on.
    pub fn is_human(&self) -> bool {
        matches!(self, Identity::Human)
    }
}

/// Resolve who is calling, in the order documented on this module: explicit `--persona`, else the
/// persona bound to `session`, else a human.
///
/// An explicit `declared` handle must name a declared persona — `not_found` otherwise, because a
/// host that names one has made a claim worth checking. A handle discovered from the SESSION MAP is
/// taken as-is even when the declaration has since been deleted: the session genuinely is that
/// persona, and a role file removed mid-flight must not silently promote a running agent to
/// "human", which is precisely the direction the rule above forbids.
///
/// It takes the declared roles as a plain slice rather than a whole [`crate::definitions`]
/// catalogue so that `prime` — which is deliberately tolerant of a broken team roster, reporting it
/// rather than refusing to start — does not become fail-closed on a duplicate handle just because
/// it now resolves an identity.
pub fn resolve_identity(
    store: &ChatStore,
    roles: &[RoleDecl],
    declared: Option<&str>,
    session: Option<&str>,
) -> Result<Identity> {
    if let Some(handle) = declared {
        let decl = roles.iter().find(|r| r.handle == handle).ok_or_else(|| {
            crate::error::NxfError::not_found(format!("no such persona: {handle}"))
        })?;
        return Ok(Identity::Persona(decl.handle.clone()));
    }
    if let Some(session) = session {
        if let Some(handle) = store.session_role(session)? {
            return Ok(Identity::Persona(handle));
        }
    }
    Ok(Identity::Human)
}

/// One persona as the directory shows it. Declared field order = the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonaEntry {
    pub handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// Whether **the audience this directory was projected for** may `nxc send --to <handle>` —
    /// the human at the terminal for [`Directory::full`], the named persona for
    /// [`Directory::for_persona`].
    ///
    /// **It used to mean "possible at all"**, and it could, while [`crate::role::Addressable`] read
    /// the same for every caller. Since a persona may declare exactly who reaches it (nxf
    /// 6j6v.st83) the caller-free answer is no longer the useful one: it would offer a human the PM
    /// that only its own agents may DM, and hide from an agent the peer it alone is allowed to
    /// call.
    pub direct: bool,
    /// The route in when [`direct`](PersonaEntry::direct) is `false`: the channels whose CAST names
    /// this persona ([`crate::channel::channels_casting`]), or the deprecated list its own
    /// declaration carries. Empty when the audience may simply send.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// Where this persona's declaration came from (nxf 6j6v.7k58) — stamped by
    /// [`Directory::with_declarations`], and absent from the wire form for the workspace's own
    /// folder, so a workspace without a user-level folder reads exactly as it did.
    #[serde(skip_serializing_if = "DeclarationOrigin::is_workspace")]
    pub origin: DeclarationOrigin,
    /// The file this persona was read from — its form, its folder, and what a skill passes
    /// through (nxf 6j6v.dw16). Stamped by [`Directory::with_declarations`]; `--json` only, the
    /// human list does not show a form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration: Option<crate::definitions::DeclarationFile>,
}

impl PersonaEntry {
    /// One entry, answered FOR an audience — see [`direct`](PersonaEntry::direct) for why the
    /// answer cannot be audience-free any more, and [`route_in`] for what fills `channels`.
    fn from_decl(decl: &RoleDecl, audience: &Identity, channels: &[ChannelDecl]) -> PersonaEntry {
        let direct = decl.addressable.allows_direct_from(audience);
        PersonaEntry {
            handle: decl.handle.clone(),
            job_title: decl.job_title.clone(),
            job_description: decl.job_description.clone(),
            stage: decl.stage,
            direct,
            channels: match direct {
                true => Vec::new(),
                false => route_in(decl, channels),
            },
            origin: DeclarationOrigin::Workspace,
            declaration: None,
        }
    }
}

/// How to reach `decl` when a direct message is not available: the channels whose CAST names it,
/// derived from the channel declarations (nxf 6j6v.g0yn), falling back to the deprecated list the
/// persona's own file carries when it has one.
///
/// The declared list wins where it exists so a workspace that has not migrated yet reads exactly as
/// it did. Everywhere else this is the answer that cannot drift: it is computed from the same
/// declaration that decides who is actually commissioned.
fn route_in(decl: &RoleDecl, channels: &[ChannelDecl]) -> Vec<String> {
    match decl.addressable.channels() {
        [] => crate::channel::channels_casting(channels, &decl.handle),
        declared => declared.to_vec(),
    }
}

/// One channel as the directory shows it. Declared field order = the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelEntry {
    pub name: String,
    pub members: Vec<String>,
    /// What the channel is for, from its own declaration (nxf 6j6v.frek). Absent when the author
    /// declared none — the entry then carries only its members, which it always names anyway.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Where this channel's declaration came from — as on [`PersonaEntry::origin`].
    #[serde(skip_serializing_if = "DeclarationOrigin::is_workspace")]
    pub origin: DeclarationOrigin,
    /// The file this channel was read from (nxf 6j6v.k3qy) — as on [`PersonaEntry::declaration`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration: Option<crate::definitions::DeclarationFile>,
}

impl ChannelEntry {
    fn from_decl(decl: &ChannelDecl) -> ChannelEntry {
        ChannelEntry {
            name: decl.name.clone(),
            // **The CAST** (nxf 6j6v.g0yn): what a reader needs is who actually takes part, and on
            // a stepped channel `members` is not that. Measured before this changed: a `coding`
            // channel running `coder` then `reviewer` off its steps rendered as
            // `members: coder` — the persona doing half the round appeared nowhere at all.
            members: crate::channel::cast(decl),
            description: decl.description.clone(),
            origin: DeclarationOrigin::Workspace,
            declaration: None,
        }
    }
}

/// One PUBLIC channel of this workspace as the directory shows it — a FRONT DOOR (nxf 6j6v.bd6g).
///
/// **This is what `Engine::public_channels` was**, folded into the directory by nxf 6j6v.yr59 —
/// owner, 2026-08-21: *"`directory` zeigt uebrigens auch die Kanaele an und wofuer sie da sind."*
/// It is a STORE read where everything else on [`Directory`] is a DECLARATION read, and that is
/// exactly why it could not simply be dropped: a front door reaching this workspace by SYNC carries
/// no declaration here, so `channels` below cannot name it and the 2026-08-19 decision kept it as
/// a read of its own. Riding on the directory is what makes "what is there" one call instead of
/// two.
///
/// **`unread` did NOT come along, and that is a decision** (yr59). `public_channels` decorated its
/// rows with the caller's per-channel unread counts, which is READ STATE, not addressability — it
/// belonged to `Engine::channels`, the membership-scoped lane read that went in the same cut. For
/// the audience this entry exists for it was always `0` anyway: a non-member had no read state at
/// all. Keeping it would have forced an acting handle onto `directory`'s signature for a number
/// nobody discovering a door can use.
///
/// **nxf 6j6v.4d2z settled it everywhere** (2026-09-08): there is no unread count on any record any
/// more, on either read. This entry was right two weeks early, and for the reason that generalised
/// — read state is not addressability, and nobody was reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrontDoor {
    /// The substrate channel id — what a read is addressed by, and the only name a synced door is
    /// guaranteed to have.
    pub channel_id: String,
    /// The channel's declared display name, when its `name` register has folded here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// How many handles own the door. Not WHICH: the discovery read has never carried member
    /// handles across a project boundary and this does not start.
    pub members: usize,
}

/// Whom someone may address, and what for — `nxc list`'s whole value (surface draft §4.4).
///
/// **Two audiences, one record, two projections.** Without a persona it is the complete catalogue,
/// which is what an app renders as its directory; with one it is that persona's own possibilities,
/// which is what `prime` hands a starting session. Same data, so the two can never disagree about
/// what exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Directory {
    /// The persona this is projected FOR; `None` is the full catalogue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    pub personas: Vec<PersonaEntry>,
    pub channels: Vec<ChannelEntry>,
    /// The workspace's PUBLIC channels — the front doors, including ones that arrived by SYNC and
    /// that no declaration here names (nxf 6j6v.yr59 folded `Engine::public_channels` in here). See
    /// [`FrontDoor`] for what travels and what deliberately does not.
    ///
    /// Empty unless somebody FILLED it: the pure [`Directory::full`]/[`Directory::for_persona`]
    /// constructors know only declarations, and the store read is attached by whoever has a store
    /// — [`crate::engine::Engine::directory`] and `nxc list`, through
    /// [`Directory::with_front_doors`], the same shape [`declarations`](Directory::declarations)
    /// already had. A `prime` address book therefore carries none, deliberately: a persona's brief
    /// is what IT may address, and another project's door is not on that list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub public_channels: Vec<FrontDoor>,
    /// Where the declarations behind this directory came from, and where they belong (nxf
    /// 6j6v.dvyq) — attached by whoever resolved the catalogue, via
    /// [`Directory::with_declarations`].
    ///
    /// **It rides on the directory rather than beside it** because an EMPTY directory is precisely
    /// the case that needs it: an empty list is indistinguishable from "there is nothing here", and
    /// the answer has to arrive with the list, not through a second call an app has to know to
    /// make. Both surfaces attach it — `nxc list --json` and
    /// [`crate::engine::Engine::directory`] — which is what keeps the parity differential able to
    /// compare them at all.
    ///
    /// `None` only for a directory projected without a resolved catalogue behind it (the pure
    /// `full`/`for_persona` constructors, used wherever the source is not the question — `prime`'s
    /// own address book, which carries the resolution on the report itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declarations: Option<crate::definitions::DeclarationSource>,
}

impl Directory {
    /// The complete catalogue: every declared persona and channel, in the catalogue's own order
    /// (roles handle-sorted by the loader, channels in declaration order).
    pub fn full(roles: &[RoleDecl], channels: &[ChannelDecl]) -> Directory {
        // **Its audience is a HUMAN**, and that is what `None` has always meant on this
        // constructor: `nxc list` builds it for the person at the terminal, and an app builds it
        // for its logged-in user. The other projection names a persona, so between the two every
        // caller class is covered — see [`PersonaEntry::direct`].
        let audience = Identity::Human;
        Directory {
            persona: None,
            personas: roles
                .iter()
                .map(|r| PersonaEntry::from_decl(r, &audience, channels))
                .collect(),
            channels: channels.iter().map(ChannelEntry::from_decl).collect(),
            public_channels: Vec::new(),
            declarations: None,
        }
    }

    /// Attach the resolution the catalogue behind this directory came from — see the
    /// [`declarations`](Directory::declarations) field — and stamp each entry with where it came
    /// from (nxf 6j6v.7k58), which only the resolution knows.
    pub fn with_declarations(
        mut self,
        source: Option<&crate::definitions::DeclarationSource>,
    ) -> Directory {
        if let Some(source) = source {
            for p in &mut self.personas {
                p.origin = source.origin_of(DeclarationKind::Persona, &p.handle);
                p.declaration = source.file_of(DeclarationKind::Persona, &p.handle).cloned();
            }
            for c in &mut self.channels {
                c.origin = source.origin_of(DeclarationKind::Channel, &c.name);
                c.declaration = source.file_of(DeclarationKind::Channel, &c.name).cloned();
            }
        }
        self.declarations = source.cloned();
        self
    }

    /// Attach the workspace's PUBLIC channels — see the
    /// [`public_channels`](Directory::public_channels) field. Called by whoever holds a store,
    /// which is the two surfaces that render a directory and neither of the two pure constructors.
    pub fn with_front_doors(mut self, doors: Vec<FrontDoor>) -> Directory {
        self.public_channels = doors;
        self
    }

    /// One persona's own possibilities: the whole catalogue minus the persona itself and minus the
    /// channels it is a member of, each entry answered FOR this persona — directly addressable or
    /// reached through a channel, by what the TARGET declares in `addressable`.
    ///
    /// **Derived, never declared** (nxf 6j6v.xjh3, owner decisions of 2026-10-03 and 2026-10-05). A
    /// persona used to carry an `address_book` of its own; it was a second place the route was
    /// written down, it could disagree with the target's own `addressable`, and its `why` lines
    /// rotted when a target's job changed. The key is retired: a declaration that still carries it
    /// loads, the key is ignored and `nxs prime` says so. "Commissions nothing" (`address_book: []`)
    /// has no successor — a target appears in a persona's directory when the target admits it.
    ///
    /// **The channels the persona sits in are dropped.** Offering a reviewer the `review` channel it
    /// sits in is help in no reading: commissioning your own channel passes every check, which is
    /// exactly how the proving ground's three reviewers came to be offered it.
    pub fn for_persona(roles: &[RoleDecl], channels: &[ChannelDecl], handle: &str) -> Directory {
        /// Whether `channel` already has this persona in it — the channel it sits in is never a
        /// target worth offering it.
        fn sits_in(channel: &ChannelDecl, handle: &str) -> bool {
            crate::channel::cast(channel).iter().any(|m| m == handle)
        }
        let audience = Identity::Persona(handle.to_string());
        let personas = roles
            .iter()
            .filter(|r| r.handle != handle)
            .map(|r| PersonaEntry::from_decl(r, &audience, channels))
            .collect();
        let channels = channels
            .iter()
            .filter(|c| !sits_in(c, handle))
            .map(ChannelEntry::from_decl)
            .collect();
        Directory {
            persona: Some(handle.to_string()),
            personas,
            channels,
            public_channels: Vec::new(),
            declarations: None,
        }
    }

    /// Whether there is nothing at all to show — the caller's cue to omit the section.
    ///
    /// **DECLARATIONS only**: [`public_channels`](Directory::public_channels) is deliberately not
    /// consulted (nxf 6j6v.yr59). What an empty directory triggers is the orientation message
    /// "nothing is declared, and here is where it goes", and a front door that reached this
    /// workspace by sync does not make that untrue — it is somebody ELSE's declaration.
    pub fn is_empty(&self) -> bool {
        self.personas.is_empty() && self.channels.is_empty()
    }

    /// The address book as Markdown: **how to address anyone, said once**, then one paragraph per
    /// target you can actually address. `""` when there is nothing to show — the empty-string
    /// convention every session-start section renderer follows, so a caller can omit the whole
    /// section.
    ///
    /// # Only what you can address (nxf 6j6v.frek)
    ///
    /// A persona declared reachable only through a channel is NOT an entry here. It used to be —
    /// and in the workspace this was cut from, four of seven lines were personas nobody could
    /// address, each one an invitation to type `--to code-quality` and be refused. The information
    /// does not disappear, it MOVES: to the channel entry, which is the thing you actually address.
    ///
    /// Personas and channels share one list because they share one way of being reached. That is
    /// also why the invocation appears once, at the top, instead of on every line: repeating
    /// `nxc send --to` seven times says nothing the header has not already said, and it crowded out
    /// the part that differs — who they are and what they are for.
    ///
    /// **The FRONT DOORS are not rendered here, by the same rule** (nxf 6j6v.yr59). A public
    /// channel of another project is DISCOVERABLE — that is why it rides on the record — but it is
    /// not addressable with `nxc send --to`, which resolves its target against the DECLARATIONS
    /// and refuses a channel none of them names. Printing one in an address book would be the
    /// invitation-to-be-refused this section was cut down to remove. The record carries it for the
    /// app that renders discovery; the agent's brief does not.
    pub fn render_markdown(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut entries: Vec<String> = self
            .personas
            .iter()
            .filter(|p| p.direct)
            .map(render_persona_entry)
            .collect();
        entries.extend(self.channels.iter().map(render_channel_entry));
        // Everything declared is channel-only and no channel is declared either: a real (broken)
        // state, and one the reader has to be told about rather than shown an empty heading.
        if entries.is_empty() {
            return "## Who you can address\n\nNobody: every declared persona is reachable only \
                    through a channel, and no channel is declared."
                .to_string();
        }
        format!(
            "## Who you can address\n\n\
             Address any of them the same way: `nxc send --to <handle> -` — the `-` reads the \
             message from STDIN, and a one-line message may be an argument instead.\n\n{}",
            entries.join("\n\n")
        )
    }
}

/// What an entry from the user-level folder carries in its parenthesis (nxf 6j6v.7k58). Only `nxc
/// list` stamps origins ([`Directory::with_declarations`]), so a persona's own address book in
/// `prime` never shows it — where a peer's definition is kept is not what a persona addresses it by.
const USER_LEVEL_QUALIFIER: &str = "from the user-level folder";

/// One persona's entry: who they are, how to name them, and what they are for.
///
/// The bold name is the human one (`job_title`), because that is what a reader recognises; the
/// handle — the thing you actually type — is in the parenthesis beside it, with the seniority band
/// that the parenthesis of a channel does not have. The trailing text is the caller's OWN reason
/// (`why`, from its address book) when there is one, since a line written for this caller beats the
/// target's general self-description, and that description otherwise.
fn render_persona_entry(p: &PersonaEntry) -> String {
    let mut qualifiers = vec![format!("handle: `{}`", p.handle)];
    if let Some(stage) = p.stage {
        qualifiers.push(format!("level: {}", stage_label(stage)));
    }
    if !p.origin.is_workspace() {
        qualifiers.push(USER_LEVEL_QUALIFIER.to_string());
    }
    let mut line = format!(
        "**{}** ({})",
        display_name(p.job_title.as_deref(), &p.handle),
        qualifiers.join(", ")
    );
    if let Some(about) = p.job_description.clone() {
        line.push_str(&format!(" — {about}"));
    }
    line
}

/// One channel's entry, in the same shape as a persona's — it is addressed the same way, so it
/// reads the same way. The parenthesis carries what makes the type: a persona has a `level`, a
/// channel has MEMBERS.
///
/// **The members are always named**, not only when nothing else is. They are the one thing a
/// channel entry knows that is otherwise unreachable — the personas reached only through it appear
/// nowhere else in the list — so dropping them the moment an author writes a `description` would
/// let one YAML line silently delete what acceptance 4 says must survive ("ihre Zugehörigkeit steht
/// am Kanal"). Acceptance 3's "name the members rather than leave the entry empty" is then simply
/// always true, instead of true only on the fallback path.
///
/// The trailing text is the caller's own `why` (written for this pairing, so more specific) else
/// the channel's declared description.
fn render_channel_entry(c: &ChannelEntry) -> String {
    let mut qualifiers = vec![format!("handle: `{}`", c.name)];
    if !c.members.is_empty() {
        qualifiers.push(format!("members: {}", c.members.join(", ")));
    }
    if !c.origin.is_workspace() {
        qualifiers.push(USER_LEVEL_QUALIFIER.to_string());
    }
    let mut line = format!(
        "**{}** ({})",
        display_name(None, &c.name),
        qualifiers.join(", ")
    );
    if let Some(about) = c.description.clone() {
        line.push_str(&format!(" — {about}"));
    }
    line
}

/// The human-readable name for an entry: the declared `title` when there is one, else the handle
/// with each `-`/`_`-separated word capitalized (`review` → `Review`, `code-review` →
/// `Code-Review`). Derived rather than required, so no declaration has to be rewritten to read
/// well — a channel has no title field at all.
fn display_name(title: Option<&str>, handle: &str) -> String {
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        return title.to_string();
    }
    let mut out = String::with_capacity(handle.len());
    let mut start_of_word = true;
    for ch in handle.chars() {
        if start_of_word {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
        start_of_word = ch == '-' || ch == '_';
    }
    out
}

/// The band's own spelling — the declaration's wire form, so what a reader sees in `prime` is what
/// they would write in the file.
fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Junior => "junior",
        Stage::Senior => "senior",
        Stage::Principal => "principal",
    }
}

/// What a persona is told about itself at session start (surface draft §4.1): its identity, and how
/// it works with `nxc`. Declared field order = the JSON contract.
///
/// The `nxc` instructions are part of the record rather than left to the renderer because they are
/// the answer to "how do I answer?" — the one thing a summoned persona must not have to guess. The
/// escalation mechanic is the SAME move as answering (reply into the thread and say what you need),
/// which is why it is one sentence here and not a second surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonaBrief {
    pub handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// Whether ANYBODY may address this persona directly — `false` only for the two forms that
    /// admit no one ([`crate::role::Addressable::Nobody`] and the deprecated channel list).
    pub direct: bool,
    /// **The declared policy itself** (nxf 6j6v.st83), in the declaration's own vocabulary.
    ///
    /// It replaced a pair of derived booleans (`direct_personas` + `direct_humans`) that the
    /// renderer below then tried to recover the policy FROM — and could not: `{personas: [],
    /// humans: true}` and `general` produce the identical pair, so the one form this field exists
    /// to announce rendered as the default and said nothing. Carrying the answer instead of two
    /// shadows of it is the fix, and it is smaller than what it replaces.
    pub addressable: Addressable,
    /// The channels it is reached through — its cast, derived from the channel declarations.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// How this persona uses `nxc` — one line per rule, in display order.
    pub instructions: &'static [&'static str],
    /// The user-level folder this persona's declaration came from, when it did (nxf 6j6v.7k58) —
    /// absent for a declaration in the workspace's own folder.
    ///
    /// **It answers a question the session cannot answer for itself:** where the files a
    /// declaration keeps BESIDE itself are. A system prompt that says "read `knowledge/x.md`,
    /// relative to the root of this workspace" means the repository the session runs in — its
    /// working directory, and the working directory a hurdle runs in — never the folder the
    /// declaration was read from; nothing in the engine resolves a path inside a declaration. A
    /// user-level persona whose knowledge lives beside its definition is told where that is.
    ///
    /// **A skill is always told** (nxf 6j6v.dw16), wherever it lives, and `declared_in` is then its
    /// OWN folder: a skill's relative references mean that folder — the Agent Skills rule — which is
    /// the opposite of the YAML rule above, so a session must be able to see which of the two
    /// applies to it. [`declared_form`](PersonaBrief::declared_form) says which.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_in: Option<std::path::PathBuf>,
    /// The form of the declaration [`declared_in`](PersonaBrief::declared_in) names — present
    /// exactly when that is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_form: Option<crate::definitions::DeclarationForm>,
}

/// The `nxc` rules every persona is handed. Rendered in order; the answering rule comes first
/// because it is the one every summoned persona needs and the only one some of them ever need.
///
/// **The asking rule ends by saying what happens to the ASKER** (nxf 6j6v.z6f9), and that sentence
/// is here rather than in a guide because it is the one thing a persona cannot look up mid-turn.
/// Inviting "ask and end your turn" while saying nothing about the interval leaves a careful agent
/// to guess — and the guess is pessimistic: an external testbed's five declarations grew a ritual
/// of `[<role>-state: <stage>]` trailer lines and a mandatory `nxc threads show` at the top of
/// every turn, a memory substitute against an amnesia that does not exist. One sentence retires
/// all of it.
///
/// **One sentence, appended to the rule that already offers the pattern — not a fifth bullet.**
/// nxf 6j6v.waq9 measures 85% of `nxs prime`'s start-up load as memories in full text, so a line
/// here has to earn itself; this one removes more prompt than it adds. The MECHANICS behind it — a
/// fan-out mints a fresh session, a thread reply resumes one through the return address — are what
/// somebody writing DECLARATIONS needs, and they live in the guides (`nxc guide personas`,
/// `nxc guide channels`), not in every persona's session start.
///
/// **And it names its own limit, because the unlimited reading cost twenty minutes** (nxf
/// 6j6v.0vd9). The promise holds WHILE THE ROUND IS OPEN and not after: once a set has been
/// consolidated and handed on, a reply into one of its threads wakes nobody, and
/// [`crate::surface::reply_in_thread`] now refuses it rather than accepting it in silence. A
/// declaration in the proving ground had spelled the promise out with no such condition ("the next
/// `nxc reply --thread <your thread>` resolves that address and RESUMES you") and its owner acted
/// on it against a finished operation; the sentence an agent is handed at every session start is
/// where that condition has to be, because it is the one thing an agent cannot look up mid-turn.
pub const PERSONA_INSTRUCTIONS: &[&str] = &[
    // **The id-free form first, with its precondition** (nxf 6j6v.dq59). This line named
    // `--thread` unconditionally, which is the most common misfire it produced: a role answering
    // its one open conversation had to carry an id it was given minutes earlier, and the whole
    // point of that role is that it knows its task and whom it may answer — not who called it. The
    // id-bearing form stays and is always allowed; what is taught first is the one that is right
    // in the ordinary case, and the condition that ends it is named in the same breath so the
    // shorthand is never reached for in the state the engine has to refuse it.
    "Answer in the conversation you were handed: `nxc reply - <<'EOF'`, your text, then `EOF` on \
     its own line. The `-` reads the message from STDIN, where the shell evaluates nothing in it — \
     a one-line answer may be an argument instead (`nxc reply \"lgtm\"`), longer text must not, \
     because backticks, `$(…)` and quotes in it are rewritten before nxc sees them. While that is \
     the only conversation open for you, no thread id is needed; open a second — commission \
     somebody yourself — and `nxc reply --thread <thread-id> -` says which, and the message that \
     wakes you names each one with the command it takes.",
    "Need something before you can answer? Same move — reply into the thread and say what you \
     need. Whoever is waiting sees it and answers you back in the same thread. Asking and ending \
     your turn is safe: while the round is open, the answer resumes you with everything you \
     already know — once it has been answered and handed on, that thread is closed and replying \
     into it is refused, so start the next thing with `nxc send`.",
    "Starting something NEW with someone else: `nxc send --to <persona|channel> -`, with the body \
     on STDIN exactly as above (or a short one as an argument). It returns a thread id; that \
     thread is where their answer arrives.",
    // `nxc inbox` was named here too, until 6j6v.dvyq §3 took it off the AGENT surface: a persona
    // is handed what it needs at session start and on resume, so a verb for asking again is a door
    // it never has to find. 6j6v.1gm9 finished that thought and REMOVED THE VERB — the pull half of
    // the message surface is gone from the binary, because the reasoning above is the whole of it
    // and nothing was left that the reasoning did not cover. 6j6v.4d2z then removed the READ as
    // well, on the same reasoning one layer down: an unread view is a second copy of what a push
    // already delivered. A human reads a conversation with `nxc threads show` / `nxc status`.
    "`nxc list` shows who you can address and what for.",
];

impl PersonaBrief {
    /// Build the brief from a persona's own declaration, and from the channels that cast it —
    /// which is where "how am I reached" now comes from (nxf 6j6v.g0yn), rather than from a list
    /// the persona repeats about itself.
    pub fn from_decl(decl: &RoleDecl, channels: &[ChannelDecl]) -> PersonaBrief {
        PersonaBrief {
            handle: decl.handle.clone(),
            job_title: decl.job_title.clone(),
            job_description: decl.job_description.clone(),
            expected_output: decl.expected_output.clone(),
            stage: decl.stage,
            // Whether ANYONE may — this is the persona reading about ITSELF, not one caller asking
            // whether it may send, so the audience-shaped question next door is the wrong one here.
            direct: decl.addressable.allows_direct_from_anyone(),
            addressable: decl.addressable.clone(),
            channels: route_in(decl, channels),
            instructions: PERSONA_INSTRUCTIONS,
            declared_in: None,
            declared_form: None,
        }
    }

    /// Say where the declaration came from — see [`declared_in`](PersonaBrief::declared_in): the
    /// persona's own folder for a skill, the user-level folder for a YAML file kept there, and
    /// nothing for a YAML file in the workspace's own folder.
    pub fn declared_in(
        mut self,
        folder: Option<std::path::PathBuf>,
        form: crate::definitions::DeclarationForm,
    ) -> PersonaBrief {
        self.declared_form = folder.as_ref().map(|_| form);
        self.declared_in = folder;
        self
    }

    /// The "you are …" block as Markdown — identity first, then the `nxc` rules.
    pub fn render_markdown(&self) -> String {
        let mut lines = vec![format!("## You are `{}`", self.handle)];
        for (label, value) in [
            ("Role", self.job_title.clone()),
            ("Job", self.job_description.clone()),
            ("Expected output", self.expected_output.clone()),
        ] {
            if let Some(value) = value {
                lines.push(format!("- **{label}:** {value}"));
            }
        }
        if let Some(stage) = self.stage {
            lines.push(format!("- **Stage:** {}", stage_label(stage)));
        }
        // **What it says has to survive the migration** (nxf 6j6v.st83). A persona declaring
        // `[planning]` was told "you are addressed through planning"; the same persona declaring
        // `{humans: true}` is MORE restricted and must not fall silent about it, which is what a
        // line rendered only for `!direct` would do. So the line states the policy whenever there
        // is one, and stays absent for `general` — the case with nothing to say.
        let via = |lead: &str| match self.channels.is_empty() {
            true => String::new(),
            false => format!("{lead} {}", self.channels.join(", ")),
        };
        // The wording for the forms that admit nobody, unchanged from before the whitelist existed
        // — for those two nothing about the answer changed.
        let nobody_directly = match self.channels.is_empty() {
            true => "you are not addressed directly".to_string(),
            false => via("you are addressed through"),
        };
        // **On the VARIANT, not on booleans derived from it** (review of PR #472, Code Quality and
        // Integrity, independently). This matched a `(direct, personas, humans)` tuple, and
        // `{humans: true}` produces exactly `general`'s — so the `general` arm swallowed the one
        // case the paragraph above promises never to fall silent about. The type carries the
        // distinction; asking anything else for it was the defect.
        //
        // `Addressable` is `#[non_exhaustive]`, which binds only OTHER crates: here the match must
        // stay exhaustive, so a fifth form reds this line rather than quietly picking an arm.
        let reachable = match &self.addressable {
            // Everyone may, which is the default and says nothing worth a line.
            Addressable::General => None,
            Addressable::Nobody | Addressable::ViaChannels(_) => Some(nobody_directly),
            // An empty whitelist admits nobody — the same answer as `none`, reached through the
            // mapping's own spelling, so it reads the same way.
            Addressable::Only { personas, humans } if personas.is_empty() && !humans => {
                Some(nobody_directly)
            }
            // A whitelist: say WHO, then where everyone else goes.
            Addressable::Only { personas, humans } => {
                let who = match (personas.as_slice(), humans) {
                    ([], _) => "only the person at the terminal".to_string(),
                    (named, false) => format!("only {}", named.join(", ")),
                    (named, true) => {
                        format!("only {} and the person at the terminal", named.join(", "))
                    }
                };
                Some(format!(
                    "{who} may message you directly{}",
                    via("; everyone else reaches you through")
                ))
            }
            // The whitelist with callers from outside (nxf 6j6v.t5xb): who inside, then who from
            // other workspaces — a persona told it can be commissioned from outside should know.
            Addressable::OnlyWithExternal {
                personas,
                humans,
                external,
            } => {
                let inside = match (personas.as_slice(), humans) {
                    ([], false) => "nobody in this workspace".to_string(),
                    ([], true) => "only the person at the terminal".to_string(),
                    (named, false) => format!("only {}", named.join(", ")),
                    (named, true) => {
                        format!("only {} and the person at the terminal", named.join(", "))
                    }
                };
                Some(format!(
                    "{inside} may message you directly, and from other workspaces on this machine \
                     {}{}",
                    external.join(", "),
                    via("; everyone else reaches you through")
                ))
            }
        };
        if let Some(reachable) = reachable {
            lines.push(format!("- **Reachable:** {reachable}"));
        }
        match (&self.declared_in, self.declared_form) {
            (Some(folder), Some(crate::definitions::DeclarationForm::Skill)) => {
                lines.push(format!(
                    "- **Declared in:** `{}`, as a skill (`SKILL.md`). A relative path in your \
                     instructions means a file in THAT folder, wherever you run, unless they say \
                     it is relative to the repository; the repository you work in is your \
                     working directory.",
                    folder.display()
                ))
            }
            (Some(folder), _) => lines.push(format!(
                "- **Declared in:** the user-level folder `{}`, not in this repository. A path your \
                 instructions give relative to this repository means THIS repository; files kept \
                 beside your declaration are under `{}`.",
                folder.display(),
                folder.display()
            )),
            (None, _) => {}
        }
        lines.push(String::new());
        lines.push("### How you use `nxc`".to_string());
        for rule in self.instructions {
            lines.push(format!("- {rule}"));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(handle: &str) -> RoleDecl {
        serde_yaml::from_str(&format!("handle: {handle}\nsystem_prompt: p\n")).unwrap()
    }

    fn channel(name: &str, members: &[&str]) -> ChannelDecl {
        ChannelDecl {
            name: name.to_string(),
            members: members.iter().map(|m| m.to_string()).collect(),
            kind: Default::default(),
            description: None,
            expects: Default::default(),
            timeout: None,
            on_complete: Default::default(),
            summary_prompt: None,
            summary_model: None,
            visibility: Default::default(),
            working_tree: Default::default(),
            flow: Default::default(),
            preconditions: Vec::new(),
            steps: Vec::new(),
            rework_notice: None,
        }
    }

    fn defs(roles: Vec<RoleDecl>, channels: Vec<ChannelDecl>) -> crate::definitions::Definitions {
        crate::definitions::Definitions::new(roles, channels).unwrap()
    }

    // ---- identity ----------------------------------------------------------------------------

    #[test]
    fn an_explicit_persona_wins_over_the_session() {
        let mut store = ChatStore::open_in_memory(1);
        store.create_pending_session("s-1", "coder").unwrap();
        let defs = defs(vec![role("coder"), role("pm")], vec![]);
        let id = resolve_identity(&store, defs.roles(), Some("pm"), Some("s-1")).unwrap();
        assert_eq!(id, Identity::Persona("pm".into()));
    }

    #[test]
    fn the_session_names_the_persona_when_nothing_was_declared() {
        // The whole point of "identity from the origin": a spawned session says nothing about
        // itself and is still recognised, because the spawn wrote the session map.
        let mut store = ChatStore::open_in_memory(1);
        store.create_pending_session("s-1", "coder").unwrap();
        let defs = defs(vec![role("coder")], vec![]);
        let id = resolve_identity(&store, defs.roles(), None, Some("s-1")).unwrap();
        assert_eq!(id, Identity::Persona("coder".into()));
        assert_eq!(id.persona(), Some("coder"));
        assert!(!id.is_human());
    }

    #[test]
    fn neither_a_declaration_nor_a_known_session_is_a_human() {
        let store = ChatStore::open_in_memory(1);
        let defs = defs(vec![role("coder")], vec![]);
        assert_eq!(
            resolve_identity(&store, defs.roles(), None, None).unwrap(),
            Identity::Human
        );
        // A session id that names no session of OURS — a hand-set `NXC_SESSION`, or one from
        // another workspace — contributes nothing, exactly as it does to the depth guard.
        assert_eq!(
            resolve_identity(&store, defs.roles(), None, Some("never-minted")).unwrap(),
            Identity::Human
        );
    }

    #[test]
    fn an_undeclared_explicit_persona_is_not_found() {
        let store = ChatStore::open_in_memory(1);
        let defs = defs(vec![role("coder")], vec![]);
        let err = resolve_identity(&store, defs.roles(), Some("ghost"), None).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::NotFound);
    }

    #[test]
    fn a_session_whose_role_is_no_longer_declared_stays_that_persona() {
        // Fail-closed in the direction the module rule demands: a deleted role file must not
        // promote a running agent to "human", which is the identity with the fewest limits.
        let mut store = ChatStore::open_in_memory(1);
        store.create_pending_session("s-1", "deleted").unwrap();
        let defs = defs(vec![role("coder")], vec![]);
        assert_eq!(
            resolve_identity(&store, defs.roles(), None, Some("s-1")).unwrap(),
            Identity::Persona("deleted".into())
        );
    }

    // ---- directory ---------------------------------------------------------------------------

    #[test]
    fn the_full_catalogue_lists_every_declared_persona_and_channel() {
        let defs = defs(
            vec![role("coder"), role("pm")],
            vec![channel("code-review", &["coder", "pm"])],
        );
        let dir = Directory::full(defs.roles(), defs.channels());
        assert_eq!(dir.persona, None);
        let handles: Vec<&str> = dir.personas.iter().map(|p| p.handle.as_str()).collect();
        assert_eq!(handles, ["coder", "pm"]);
        assert_eq!(dir.channels.len(), 1);
        assert_eq!(dir.channels[0].members, ["coder", "pm"]);
    }

    #[test]
    fn no_address_book_shows_the_whole_team_minus_yourself_and_minus_the_channels_you_sit_in() {
        // nxf 6j6v.vce2's second half. `standup` has the reader in it; `release` does not. Offering
        // a persona the channel it is a MEMBER of is help in no reading — commissioning your own
        // channel is not declaration cyclicity, so it passes every check there is, which is exactly
        // how the proving ground's three reviewers came to be offered `review`.
        let defs = defs(
            vec![role("coder"), role("pm")],
            vec![
                channel("standup", &["coder", "pm"]),
                channel("release", &["pm"]),
            ],
        );
        let dir = Directory::for_persona(defs.roles(), defs.channels(), "coder");
        let handles: Vec<&str> = dir.personas.iter().map(|p| p.handle.as_str()).collect();
        assert_eq!(handles, ["pm"], "never yourself");
        let names: Vec<&str> = dir.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["release"], "never the room you are already in");
    }

    #[test]
    fn a_channel_only_persona_is_not_an_entry_of_its_own_but_its_channel_is() {
        // nxf 6j6v.frek. The record still knows the persona is channel-only (an app renders what it
        // likes from the JSON); the LIST shows only what you can address, and what you address to
        // reach that persona is the channel.
        let mut reviewer = role("reviewer");
        reviewer.addressable = Addressable::ViaChannels(vec!["code-review".into()]);
        let defs = defs(vec![reviewer], vec![channel("code-review", &["reviewer"])]);
        let dir = Directory::full(defs.roles(), defs.channels());
        assert!(!dir.personas[0].direct, "the record keeps the whole truth");
        assert_eq!(dir.personas[0].channels, ["code-review"]);

        let md = dir.render_markdown();
        assert!(
            !md.contains("**Reviewer**"),
            "a persona nobody can address directly must not be an entry of its own:\n{md}"
        );
        assert!(
            !md.contains("--to reviewer"),
            "never print an invocation that would be refused:\n{md}"
        );
        assert!(
            md.contains("**Code-Review** (handle: `code-review`, members: reviewer)"),
            "the channel carries it instead, and always names its members:\n{md}"
        );
    }

    #[test]
    fn the_way_to_address_anyone_is_stated_exactly_once() {
        // The owner's first complaint: `nxc send --to` stood in every single line, identical every
        // time, crowding out the part that actually differs.
        let defs = defs(
            vec![role("coder"), role("pm")],
            vec![channel("review", &["coder", "pm"])],
        );
        let md = Directory::full(defs.roles(), defs.channels()).render_markdown();
        assert_eq!(
            md.matches("nxc send --to").count(),
            1,
            "the invocation belongs in the header, once:\n{md}"
        );
        assert!(md.contains("Address any of them the same way:"), "{md}");
    }

    #[test]
    fn an_entry_is_one_paragraph_naming_who_they_are_how_to_type_them_and_what_for() {
        let mut coder = role("coder");
        coder.job_title = Some("Coder".into());
        coder.job_description = Some("Implements the work order and merges.".into());
        coder.stage = Some(Stage::Senior);
        let defs = defs(vec![coder], vec![]);
        let md = Directory::full(defs.roles(), defs.channels()).render_markdown();
        assert!(
            md.contains(
                "**Coder** (handle: `coder`, level: senior) — Implements the work order and merges."
            ),
            "{md}"
        );
        // Blank-line separated, so a description that runs long stays one readable block rather
        // than a wrapped bullet.
        assert!(md.contains("\n\n**Coder**"), "{md}");
    }

    #[test]
    fn an_undeclared_title_falls_back_to_the_handle_and_a_declared_one_wins() {
        // Nothing has to be rewritten to read well: a channel has no title field at all, and a
        // persona that declared none is still named rather than shown a bare handle.
        assert_eq!(display_name(None, "review"), "Review");
        assert_eq!(display_name(None, "code-review"), "Code-Review");
        assert_eq!(display_name(None, "head_of_product"), "Head_Of_Product");
        assert_eq!(
            display_name(Some("Head of Product"), "hop"),
            "Head of Product"
        );
        assert_eq!(
            display_name(Some("   "), "hop"),
            "Hop",
            "a whitespace-only title is not a title"
        );
    }

    #[test]
    fn a_channel_names_its_members_whether_or_not_it_declares_a_description() {
        // The members are the only route to a persona that is reachable through this channel and
        // nothing else — it appears nowhere else in the list. If a declared `description` displaced
        // them, editing one YAML line would silently delete exactly what acceptance 4 requires to
        // survive, and nothing would notice.
        let mut review = channel("review", &["code-quality", "integrity"]);
        review.description =
            Some("the four-member review quorum that judges a diff together".into());
        let described = defs(vec![], vec![review.clone()]);
        let md = Directory::full(described.roles(), described.channels()).render_markdown();
        assert!(
            md.contains(
                "**Review** (handle: `review`, members: code-quality, integrity) — the four-member \
                 review quorum that judges a diff together"
            ),
            "{md}"
        );

        review.description = None;
        let bare = defs(vec![], vec![review]);
        let md = Directory::full(bare.roles(), bare.channels()).render_markdown();
        assert!(
            md.contains("**Review** (handle: `review`, members: code-quality, integrity)"),
            "and with no description the entry is still not mute:\n{md}"
        );
    }

    #[test]
    fn a_directory_of_nothing_addressable_says_so_instead_of_showing_an_empty_heading() {
        let mut reviewer = role("reviewer");
        reviewer.addressable = Addressable::ViaChannels(vec!["code-review".into()]);
        let defs = defs(vec![reviewer], vec![]);
        let md = Directory::full(defs.roles(), defs.channels()).render_markdown();
        assert!(md.contains("## Who you can address"), "{md}");
        assert!(md.contains("Nobody:"), "{md}");
    }

    #[test]
    fn an_empty_directory_renders_nothing_at_all() {
        let defs = defs(vec![], vec![]);
        assert!(Directory::full(defs.roles(), defs.channels()).is_empty());
        assert_eq!(
            Directory::full(defs.roles(), defs.channels()).render_markdown(),
            ""
        );
    }

    /// **Every declared form, rendered** (review of PR #472) — the coverage whose absence let the
    /// tuple-dispatch defect ship: nothing called `render_markdown` for any of the three forms
    /// nxf 6j6v.st83 added, so the arm that swallowed `{humans: true}` had nobody watching it.
    #[test]
    fn the_brief_states_the_reachability_policy_for_every_declared_form() {
        let brief = |addressable: Addressable, channels: &[&str]| {
            let mut decl: RoleDecl = role("pm");
            decl.addressable = addressable;
            let channels: Vec<ChannelDecl> =
                channels.iter().map(|name| channel(name, &["pm"])).collect();
            PersonaBrief::from_decl(&decl, &channels).render_markdown()
        };

        // `general`: the default says nothing worth a line, as it always did.
        assert!(
            !brief(Addressable::General, &[]).contains("**Reachable:**"),
            "the default has nothing to announce"
        );

        // THE CASE THAT SHIPPED BROKEN. Its tuple is identical to `general`'s, so a renderer that
        // asks the booleans instead of the type reports nothing here.
        let humans_only = brief(
            Addressable::Only {
                personas: Vec::new(),
                humans: true,
            },
            &["planning"],
        );
        assert!(
            humans_only.contains(
                "- **Reachable:** only the person at the terminal may message you directly; \
                 everyone else reaches you through planning"
            ),
            "{humans_only}"
        );

        // A named caller, with and without the human beside it.
        let one_peer = brief(
            Addressable::Only {
                personas: vec!["head".into()],
                humans: false,
            },
            &["marketing"],
        );
        assert!(
            one_peer.contains(
                "- **Reachable:** only head may message you directly; everyone else reaches you \
                 through marketing"
            ),
            "{one_peer}"
        );
        let peer_and_human = brief(
            Addressable::Only {
                personas: vec!["head".into()],
                humans: true,
            },
            &[],
        );
        assert!(
            peer_and_human
                .contains("- **Reachable:** only head and the person at the terminal may message"),
            "{peer_and_human}"
        );

        // `none`, and the empty whitelist that means the same thing — both keep the wording the
        // channel-list form had before the whitelist existed.
        for admits_nobody in [
            Addressable::Nobody,
            Addressable::Only {
                personas: Vec::new(),
                humans: false,
            },
        ] {
            let md = brief(admits_nobody.clone(), &["review"]);
            assert!(
                md.contains("- **Reachable:** you are addressed through review"),
                "{admits_nobody:?}:\n{md}"
            );
            let unreachable = brief(admits_nobody.clone(), &[]);
            assert!(
                unreachable.contains("- **Reachable:** you are not addressed directly"),
                "{admits_nobody:?} with no channel casting it:\n{unreachable}"
            );
        }

        // And the deprecated channel list still reads exactly as it did.
        let listed = brief(Addressable::ViaChannels(vec!["review".into()]), &[]);
        assert!(
            listed.contains("- **Reachable:** you are addressed through review"),
            "{listed}"
        );
    }

    #[test]
    fn the_persona_brief_names_the_identity_and_how_to_answer() {
        let mut pm: RoleDecl = role("pm");
        pm.job_title = Some("Product manager".into());
        pm.stage = Some(Stage::Senior);
        let brief = PersonaBrief::from_decl(&pm, &[]);
        let md = brief.render_markdown();
        assert!(md.contains("## You are `pm`"));
        assert!(md.contains("Product manager"));
        assert!(md.contains("senior"));
        assert!(
            md.contains("nxc reply --thread"),
            "the answering rule is the one a summoned persona must never have to guess:\n{md}"
        );
        // nxf 6j6v.s46h: the rule a persona reads about ANSWERING is one of the three texts that
        // taught `nxc reply "<…>"` — the form in which the shell evaluates a verdict's backticks
        // and `$(…)` before `nxc` sees them. It shows the safe source, and it spells the quoted
        // heredoc rather than naming it, because `<<EOF` without the quotes still expands.
        assert!(
            md.contains("nxc reply - <<'EOF'"),
            "and it shows where the message comes from:\n{md}"
        );
    }
}

//! The two verbs that move a conversation (nxf 6j6v.p6m1, surface draft §4.2/§4.3):
//! [`send_to`] opens a thread, [`reply_in_thread`] answers in one.
//!
//! # What this layer is
//!
//! A **thin layer over the existing seam** — not a second implementation of anything. `send --to`
//! resolves its target's DECLARATION and then runs the same [`crate::orchestration`] verb the
//! matching old entry point ran; `reply --thread` runs [`orchestration::reply`] verbatim and adds
//! exactly one thing to it.
//!
//! **These two ARE the writing surface now** (nxf 6j6v.ckeq). This module's first cut called itself
//! "additive beside today's verbs" and said the consolidation of the older entrances was "a later,
//! separate decision that wants usage numbers this slice does not have yet". That decision has been
//! taken: `Engine::send` (the raw post into a substrate channel, which woke nobody) and
//! `Engine::reply` (a reply to a message-or-thread `target`) are gone, and owner, 2026-08-21:
//! "`send_to` und `reply_thread` sollen der einzige Weg der Kommunikation sein. Es startet immer
//! auch die jeweilige Sitzung bzw. weckt sie wieder auf." There is no way left to lay something down
//! without somebody picking it up.
//!
//! The same item cut the eleven caller options to three per verb; each removal is argued at the
//! field it took, and [`SendToRefs`] is the one thing it ADDED — the obligation to say what a first
//! message is about.
//!
//! # Why the surface is shaped this way
//!
//! The seam knows several ways to begin something, and they differ along three axes — does a
//! channel come into being, does somebody wake up, is a thread stamped — that are **orthogonal in
//! fact but baked into the choice of verb**. A caller therefore has to hold a matrix in its head,
//! and in the night before 2026-08-05 a real session did not: it reached for `ask`, hit "the
//! channel must exist", and concluded the specification had a hole. It had none — it was the wrong
//! verb. Here the caller states the TARGET and the target's declaration decides the rest.
//!
//! # Thread ids are workspace-unique
//!
//! Every `send --to` stamps a thread, and a thread id identifies a conversation on its own — no
//! channel context needed to reply into it, nothing for an app to mint, and a return address that
//! survives the wake receipt not carrying one.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::awaiting::Await;
use crate::definitions::DeclarationSource;
use crate::error::{NxfError, Result};
use crate::model::{Disposition, MessageKind, Priority, Refs};
use crate::orchestration::{self, ChannelOpenRequest, Commission, Ctx, ReplyReceipt, ReplyRequest};
use crate::store::ChatStore;

/// What a `--to` target turned out to BE — the resolution that decides what happens next.
/// Serializes in the receipt as its snake_case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TargetKind {
    /// A declared persona: the direct conversation is materialised and the persona is started.
    Persona,
    /// A declared channel: its own policy decides the fan-out, the expected set and the deadline.
    Channel,
    // `Conversation` was here — "an existing substrate channel that no declaration names: the
    // message is posted and nobody is woken". REMOVED with the raw channels (6j6v.dvyq §3).
    // Nothing can produce it any more, and a variant a caller can match on but never receive is a
    // claim the type makes and the code cannot keep. The enum stays `#[non_exhaustive]`, so a
    // consumer that already matched with a wildcard is unaffected; one that named this arm
    // explicitly gets a compile error, which is the honest signal that a target it handled is no
    // longer reachable.
}

/// **What this first message REFERS TO — the three-state answer `send_to` demands** (nxf
/// 6j6v.ckeq §5). Owner, 2026-08-21: "jede erste Nachricht bezieht sich wahrscheinlich auf ein
/// Ticket."
///
/// # Why three states and not `Option<Refs>`
///
/// The item names the design constraint outright: *die Naht muss "nicht angegeben" von
/// "ausdruecklich keins" unterscheiden koennen*. Both silent states post the message and both stamp
/// no pointers — what differs is whether the caller ANSWERED. An `Option<Refs>` has one `None` and
/// would have to mean both, so the distinction would have to live somewhere else (a second boolean
/// beside it, or a convention about `Refs::default()`), and a distinction that lives beside the
/// value it qualifies is one a caller can set inconsistently. Here it cannot: the three states are
/// the three answers.
///
/// # Why the empty set is not a declared nothing
///
/// [`declared`](SendToRefs::declared) is the constructor, and it answers [`Unspecified`] for an
/// empty [`Refs`]. A caller that assembles pointers and finds none has said nothing, not "I looked
/// and there is nothing" — and if `Declared(Refs::default())` counted as an answer, the obligation
/// could be met by wrapping the absence, which is the one shape that would make the whole rule
/// decorative. Building the variant by hand is still possible (it is public data, like every other
/// request field here), and it lands in the same place: [`is_unspecified`](Self::is_unspecified)
/// asks about the CONTENT, so an empty `Declared` warns exactly as an `Unspecified` does.
///
/// # What this can and cannot buy
///
/// The item says it plainly: *Erzwingen laesst sich die ANGABE, nicht ihre RICHTIGKEIT.* A caller
/// can answer [`ExplicitlyNone`] where a ticket belonged, and nothing here can tell. What makes the
/// obligation bite is elsewhere — `prime_as` teaches it to the persona in the text it reads anyway,
/// and an operation with no ticket stands out in `nxc status`.
///
/// [`Unspecified`]: SendToRefs::Unspecified
/// [`ExplicitlyNone`]: SendToRefs::ExplicitlyNone
#[derive(Debug, Clone, PartialEq, Eq)]
// **`Refs` is larger than the two empty variants beside it, and that is accepted** (nxf 6j6v.2af2,
// which is the field that pushed it over clippy's 200-byte threshold). The alternatives are boxing
// the payload here or boxing [`crate::anchor::Anchor`] inside `Refs`, and both would buy an
// allocation and an indirection to save a stack move on a value that is built ONCE per `send`,
// matched twice and dropped — on a call path that goes on to start an operating-system session. What
// the lint protects against is a large variant travelling in bulk; this type never does (the one that
// does is `MessageView`, where the same `Refs` is the CONTENT of the read and cannot be elsewhere).
// Stated rather than silenced: if a future field makes `Refs` big enough to matter, the answer is to
// box it in `Refs` — one place — and not to reshape this enum, which is the one an embedder writes.
#[allow(clippy::large_enum_variant)]
pub enum SendToRefs {
    /// At least one pointer: what this conversation is about. `nxf_ids` is repeatable, because a
    /// first message may name several items (owner: "Es muss eine LISTE angegeben werden koennen").
    Declared(Refs),
    /// `--no-ref`: the caller looked and there genuinely is nothing to point at. An ANSWER, so it
    /// does not warn.
    ExplicitlyNone,
    /// Nothing was said. The send still happens — this is a warning, not a refusal, because the
    /// message is worth more posted than lost — and the receipt carries
    /// [`refs_warning`](SendToReceipt::refs_warning) so an app sees it too.
    Unspecified,
}

impl SendToRefs {
    /// The constructor: an EMPTY [`Refs`] is [`Unspecified`](SendToRefs::Unspecified), never a
    /// declared nothing. See the type's own doc for why that is the whole point.
    pub fn declared(refs: Refs) -> SendToRefs {
        if refs == Refs::default() {
            SendToRefs::Unspecified
        } else {
            SendToRefs::Declared(refs)
        }
    }

    /// Whether the obligation was left unmet — the one question [`send_to`] asks, and it asks it of
    /// the CONTENT rather than of the variant, so a hand-built empty `Declared` cannot slip past.
    pub fn is_unspecified(&self) -> bool {
        match self {
            SendToRefs::Declared(refs) => refs == &Refs::default(),
            SendToRefs::ExplicitlyNone => false,
            SendToRefs::Unspecified => true,
        }
    }

    /// The pointers to stamp on the message: whatever was declared, else none.
    pub fn into_refs(self) -> Refs {
        match self {
            SendToRefs::Declared(refs) => refs,
            SendToRefs::ExplicitlyNone | SendToRefs::Unspecified => Refs::default(),
        }
    }
}

/// **The warning a `send_to` that named nothing carries** — one text, so the stderr line a terminal
/// reader sees and the [`refs_warning`](SendToReceipt::refs_warning) field an app reads cannot say
/// different things (nxf 6j6v.ckeq §5, the same discipline
/// [`orchestration::FailedConsequence`]'s `Display` follows).
pub const UNREFERENCED_WARNING: &str =
    "nothing was named as the subject of this conversation. Say what it is about — \
     `--ref nxf_ids=<id>`, repeatable — or `--no-ref` if there really is nothing.";

/// Open a conversation with a target (surface draft §4.2).
///
/// **Three statements, and every one of them is the caller's own** (nxf 6j6v.ckeq). What went:
/// `kind`, `priority` and `disposition` — owner, 2026-08-21: "interessante Features, die wir zu
/// einem spaeteren Zeitpunkt bei Bedarf zurueckholen, aber dann mit mehr Wissen ueber den
/// tatsaechlichen Bedarf" — plus `model` and `deadline`, which are DECLARED at the persona and at
/// the channel, so a per-call override was a second answer to a question the declaration already
/// answers ("sonst wird ein Agent vielleicht uebereifrig Optionen aendern"). The fields they fed on
/// [`ChannelOpenRequest`]/[`Commission`] are untouched; this surface simply stops offering a
/// way to set them.
#[derive(Debug, Clone)]
pub struct SendToRequest<'a> {
    /// A declared persona handle or a declared channel name — resolved in that order (see
    /// [`send_to`]). A substrate channel id is NOT a target: since 6j6v.dvyq §3 a channel is
    /// addressable only through the declaration that names it.
    pub to: &'a str,
    pub body: &'a str,
    /// What this conversation is about — MANDATORY since nxf 6j6v.ckeq, in the sense that saying
    /// nothing is one of the three answers and the only one that warns. See [`SendToRefs`].
    pub refs: SendToRefs,
    /// **The machine this chat runs on, chosen for this chat** (nxf 6j6v.1c6k) — a machine id or a
    /// name the workspace's relay knows. `None` takes the persona's `machine:`, else this machine.
    /// A persona target only: a declared channel runs where it is started, and naming a machine for
    /// one is refused. See [`crate::machine::resolve`] for the precedence and when it asks instead.
    pub machine: Option<&'a str>,
}

/// What opening a conversation produced. Declared field order = the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SendToReceipt {
    /// The workspace-unique thread this conversation is now addressed by — what `reply --thread`
    /// takes, and the one value a caller must keep.
    pub thread_id: String,
    pub message_id: String,
    /// The target as the caller wrote it.
    pub to: String,
    /// What that target turned out to be, so a caller can tell "a persona is now working on it"
    /// from "this went into a channel" without re-deriving the declarations itself.
    pub target: TargetKind,
    /// The substrate channel the message landed in (the materialised direct conversation, the
    /// declared channel's own id, or the channel that was named).
    pub channel: String,
    /// The persona session this send started, when it started exactly one (the persona path). A
    /// channel start begins its declared flow — every member at once, or just its first step — and
    /// reports none of those sessions here; the board is the addressable thing, not the individual
    /// sessions.
    ///
    /// **Minted is not running**: when [`queue_position`](SendToReceipt::queue_position) is set, this
    /// session exists but nothing has started on it yet — see that field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The `scope_key` of the chain currently holding the working tree, when the persona this send
    /// summoned had to wait for it (nxf 6j6v.303b). Always `None` on the channel and conversation
    /// paths; see [`orchestration::TriggerReceipt::queued_behind`], which this relays verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued_behind: Option<String>,
    /// The summoned persona's 1-based position in the working-tree queue, when it had to wait.
    ///
    /// `Some` means **the persona is NOT working on this yet**: the thread is open, the message is
    /// posted and the session is minted, but the spawn happens only when the current holder releases
    /// (nxf 6j6v.fe0f). The epic's §6 names this receipt by name — a `send --to @coder` that silently
    /// looked like a started session, while the trigger sat in a queue, is exactly the silent-failure
    /// class this field exists to prevent.
    ///
    /// Both fields are ADDITIVE and skipped when absent, so an existing `--json` reader sees a
    /// byte-identical receipt for every send that behaves as it did before this ticket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<i64>,
    /// Who the thread now expects a reply from: a declared channel's own policy for a channel
    /// target, the summoned persona for a persona target (nxf 6j6v.jepk —
    /// [`orchestration::coordinator_commission`] declares it on the fresh thread this call just
    /// opened),
    /// empty for a plain conversation target, which has no declared policy and wakes nobody.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expects: Vec<String>,
    /// The CHANNEL THREAD's resolved absolute deadline, when the channel declared one or the caller
    /// passed one — the instant the declared window first falls due at, and unchanged by this item.
    ///
    /// It is not when any given member is given up on. Since nxf 6j6v.nf38 a `timeout` declared as a
    /// duration is a per-member IDLE window, restarted by every write to that member's session
    /// transcript, so the moment a member actually times out is on its own thread and moves
    /// (`nxc status --thread <member>`; `member_deadline.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// **The typed discriminator, relayed from [`orchestration::TriggerReceipt::spawned`] on the
    /// persona path** (nxf 6j6v.hpv8, PR review, Important #4) — see that field's own doc for why
    /// `!warnings.is_empty()` is not itself the signal a caller should branch on.
    ///
    /// `true` on the CHANNEL path, mirroring the pre-existing, deliberate simplification
    /// [`queued_behind`]/[`queue_position`] already make there: a fan-out has one receipt for N
    /// triggers and there is no field on it that could say "member 3 of 5 did not spawn" (see
    /// `open_declared_channel_and_fan_out`'s own comment) — a channel branch that returns `Ok` at
    /// all already means every member's [`orchestration::trigger_role`] call succeeded, because that
    /// verb still propagates `Err` synchronously on this path (untouched by this ticket). `false` on
    /// the CONVERSATION path, where no role is ever addressed at all — nothing was asked to spawn,
    /// so nothing did.
    ///
    /// [`queued_behind`]: SendToReceipt::queued_behind
    /// [`queue_position`]: SendToReceipt::queue_position
    pub spawned: bool,
    /// **The machine this chat was handed to instead of started here** (nxf 6j6v.1c6k) — `Some`
    /// exactly when the chat's designated machine is another one: nothing was started on this
    /// machine, and that one picks the order up after its next pull. Skipped when absent, so every
    /// receipt of a chat that runs where it was written is byte-identical to before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handed_to: Option<crate::machine::ExecutingMachine>,
    /// Best-effort steps that did NOT happen even though the thread and message above ARE durably
    /// posted (nxf 6j6v.hpv8). Relayed verbatim from [`orchestration::TriggerReceipt::warnings`] on
    /// the persona path — see that field's own doc for the failure this closes (a summon that fails
    /// AFTER the post used to discard the whole receipt behind an opaque `Err`) and for why the two
    /// seams express "receipt AND failure" differently on top of the SAME always-`Ok` shape.
    ///
    /// **The CHANNEL path fills it too since nxf 6j6v.p3sm** — with what that path has that is
    /// genuinely best-effort: the one-shot job that watches the board's declared `timeout:`
    /// ([`orchestration::ConsequenceClass::TickUnscheduled`]). Everything else on that path still
    /// propagates as an `Err` rather than reporting, which is its own deliberate contract. Always
    /// `Vec::new()` on the conversation path, which neither ticket touches.
    ///
    /// **ALWAYS present in `--json`, never `#[serde(skip_serializing_if)]`.** The one field on this
    /// receipt (besides [`spawned`](SendToReceipt::spawned)) that is not additive-and-omittable like
    /// `queued_behind`/`queue_position` above: those two are absent on every trigger that behaves as
    /// it always did, so omitting them changes nothing for an existing reader. `warnings` is the
    /// opposite kind of field — its entire job is to be seen — so it renders even empty, exactly as
    /// the FORM this ticket sets as the precedent for 6j6v.r6pf says: "leer = nichts Auffaelliges;
    /// im `--json` IMMER vorhanden".
    ///
    /// Its element carries a CLASS and a machine-readable reason since nxf 6j6v.93zd (which is how
    /// nxf 6j6v.6m6x closed); the human sentence it used to be is that element's `detail`, and its
    /// `Display`.
    pub warnings: Vec<orchestration::FailedConsequence>,
    /// **The `refs` obligation, unmet** (nxf 6j6v.ckeq §5) — [`UNREFERENCED_WARNING`] when the
    /// caller named no pointers AND did not answer [`SendToRefs::ExplicitlyNone`], `null` otherwise.
    ///
    /// **A FIELD, not a line on stderr, and that is the whole requirement.** The item names the trap
    /// by name: a warning a terminal reader meets and an app never does is the breadcrumb class nxf
    /// 6j6v.93zd closed for [`warnings`](SendToReceipt::warnings). So this renders in `--json` on
    /// every send, `null` included, exactly as `warnings` renders even empty — a reader never has to
    /// tell "the key is absent" from "there was nothing to report", and the field's VALUE is the
    /// signal it branches on rather than a substring of its prose.
    ///
    /// It is deliberately NOT an element of `warnings`: every [`orchestration::ConsequenceClass`]
    /// is a shape of "a consequence that was owed and did not happen", and nothing failed here. The
    /// thread is open, the message is posted, the persona is running — only nobody said what any of
    /// it is for.
    pub refs_warning: Option<String>,
    /// **How this caller learns the answer** (nxf 6j6v.7qfm) — the same content the human line
    /// carries, as fields, because an agent must not parse prose.
    ///
    /// It answers the question the old receipt never did. `opened thread <id> in dm:<hash> — reply
    /// with …` told a requester how to keep TALKING at the moment nobody had answered yet; this
    /// says where the answer lands, which read to run, what "done" and "stopped" look like in that
    /// read, where the answer sits in it, and how patient to be.
    ///
    /// **Two shapes, and which one a caller gets is decided by who it is**: a registered persona
    /// is RESUMED by the coordinator and must not poll; everybody else has nobody to wake it. See
    /// [`crate::awaiting::AwaitHow`].
    ///
    /// **ALWAYS present, never `#[serde(skip_serializing_if)]`** — [`warnings`](Self::warnings)'
    /// argument verbatim: its whole job is to be seen.
    #[serde(rename = "await")]
    pub await_: Await,
}

/// Whether the caller is a REGISTERED persona — the one question that decides which of
/// [`Await`]'s two shapes a receipt carries (nxf 6j6v.7qfm, decision 1).
///
/// The same question [`crate::persona::resolve_identity`] answers for the `--stream` refusal, asked
/// through the same function rather than re-derived: a caller the engine started is exactly the one
/// the coordinator can resume, and a second answer here could disagree with the refusal a line
/// away. A process that simply never registers is "a human" to this check, exactly as it is there —
/// and lands on the advice that works for anybody, which is the benign direction.
fn caller_identity(ctx: &Ctx, store: &ChatStore) -> Result<crate::persona::Identity> {
    crate::persona::resolve_identity(store, ctx.defs.roles(), None, ctx.session)
}

/// What to tell a caller the target does not admit (nxf 6j6v.st83) — **who may, and where this
/// caller should go instead**.
///
/// Two things stay exactly as they were, deliberately: the wording for the two forms that admit
/// NOBODY is byte-identical to what 0.95.0 produced (it is quoted verbatim in `nxc guide
/// commands`), and a persona still carrying the deprecated channel list says that list. What is new
/// is the whitelist case, and that its route in is DERIVED where nothing was declared — a refusal
/// that names no way forward leaves the caller with no move, and
/// [`crate::channel::channels_casting`] can always answer it now.
fn refusal(decl: &crate::role::RoleDecl, caller: &crate::persona::Identity, ctx: &Ctx) -> String {
    use crate::role::Addressable;
    // The channels this persona is actually reached through: its own declared list where it has
    // one (a workspace mid-migration reads as it did), else the channels whose cast names it.
    let via = match decl.addressable.channels() {
        [] => crate::channel::channels_casting(ctx.defs.channels(), &decl.handle),
        declared => declared.to_vec(),
    };
    let through = |lead: &str| match via.first() {
        None => String::new(),
        Some(first) => format!("{lead}{} — send --to {first} instead", via.join(", ")),
    };
    match &decl.addressable {
        // Nobody may, whoever is asking — the answer this surface has always been able to give,
        // and `General` cannot reach here at all (it admits everyone).
        Addressable::General | Addressable::Nobody | Addressable::ViaChannels(_) => {
            match via.is_empty() {
                true => format!("{} is not addressable directly", decl.handle),
                false => format!(
                    "{}{}",
                    decl.handle,
                    through(" is addressable only through ")
                ),
            }
        }
        // A whitelist refused THIS caller. Naming the caller's own class is what makes the refusal
        // actionable rather than merely true: "a persona may not" is the thing to change a
        // declaration over, and it is invisible from the outside.
        Addressable::Only { personas, humans } => {
            let who = match (personas.as_slice(), humans) {
                ([], true) => "only the human at the terminal may".to_string(),
                // An empty whitelist admits nobody — the same answer as `none`, reached through
                // the mapping's own spelling.
                ([], false) => "nobody may".to_string(),
                (named, false) => format!("only {} may", named.join(", ")),
                (named, true) => format!(
                    "only {} and the human at the terminal may",
                    named.join(", ")
                ),
            };
            let you = match caller.persona() {
                Some(handle) => format!(" (you are `{handle}`)"),
                None => " (you are the human at the terminal)".to_string(),
            };
            format!(
                "{} is not addressable by you{you} — {who} open a conversation directly{}",
                decl.handle,
                // Just the hint, not a second recital of the channel names: `who` has already said
                // who may, so "everyone else goes through planning — send --to planning" would say
                // `planning` twice in one sentence.
                match via.first() {
                    None => String::new(),
                    Some(first) => format!("; send --to {first} instead"),
                }
            )
        }
    }
}

/// Append the "nothing is attending this workspace" finding to a receipt's own warnings, when
/// there is one (nxf 6j6v.0j12).
///
/// A function rather than the expression inlined twice, so the channel branch and the persona
/// branch of [`send_to`] cannot come to disagree about whether a send reports it.
fn with_service_finding(
    ctx: &Ctx,
    mut warnings: Vec<orchestration::FailedConsequence>,
    thread: &str,
) -> Vec<orchestration::FailedConsequence> {
    warnings.extend(orchestration::service_finding(ctx, thread));
    warnings
}

/// Send a FIRST message and open a thread for it.
///
/// **The target's declaration decides what happens**, in this resolution order:
///
/// 1. a declared **channel** — [`orchestration::channel_open`], i.e. the channel's own `expects`,
///    `timeout` and fan-out policy, exactly as `nxc ask <declared>` has always run it;
/// 2. a declared **persona** — the direct conversation is materialised, a thread is stamped on it,
///    the message is posted and the persona is started on a fresh session;
/// 3. an existing **substrate channel** that no declaration names — REFUSED (`validation`), naming
///    the file to declare it in. It was a post-and-wake-nobody path until 6j6v.dvyq §3 made a
///    channel addressable only through its declaration; the branch below is what enforces that, and
///    this list said the opposite until nxf 6j6v.t6vd read it while writing the guides;
/// 4. otherwise `not_found`, naming all three namespaces that were searched — or, in a workspace
///    where nothing at all is declared, saying THAT rather than sending a caller hunting for a typo.
///
/// Channels resolve before personas because a channel name is the more specific claim: a declared
/// channel is always addressable, whereas a persona may have declared that it is not.
///
/// **[`crate::role::Addressable`] is enforced here and the address book is not**, which looks
/// inconsistent and is the whole point: addressability is the TARGET's own property and reads the
/// same for every caller, while an address book is a property of the CALLER — and a caller's
/// identity can be dropped, so deriving a restriction from it would mean that omitting `--persona`
/// grants MORE. See [`crate::persona`] for the rule in full.
pub fn send_to(ctx: &Ctx, store: &mut ChatStore, req: SendToRequest) -> Result<SendToReceipt> {
    // The refs obligation is resolved ONCE, here, before any branch — both target paths post a
    // message and both owe the caller the same answer about it (nxf 6j6v.ckeq §5). Resolved BEFORE
    // the target is resolved, too, so a `not_found` and a missing reference cannot race to be the
    // thing the caller is told about: a refused send never reaches the receipt this field lives on.
    let refs_warning = req
        .refs
        .is_unspecified()
        .then(|| UNREFERENCED_WARNING.to_string());
    let refs = req.refs.into_refs();
    // Who is asking, resolved ONCE for both branches (nxf 6j6v.7qfm): the two targets differ in
    // what the answer costs to wait for, never in who is waiting.
    // …and it is the WHOLE identity since nxf 6j6v.st83, not the bool this used to keep: the same
    // answer decides which [`Await`] shape the receipt carries AND whether the target admits this
    // caller at all. One resolution, so the two can never disagree about who is calling.
    let identity = caller_identity(ctx, store)?;
    let persona = identity.persona().is_some();

    // (1) A declared channel: hand the whole thing to the existing verb.
    if ctx.defs.declared_channel(req.to)?.is_some() {
        // A channel runs where it is started (nxf 6j6v.1c6k, spec §2.1): its supervisor's state is
        // this machine's own, so a member elsewhere would answer into a supervisor that is not there.
        if req.machine.is_some() {
            return Err(NxfError::validation(format!(
                "{} is a channel, and a channel runs on the machine that starts it — --machine \
                 names the machine of a persona chat",
                req.to
            )));
        }
        let receipt = orchestration::channel_open(
            ctx,
            store,
            ChannelOpenRequest {
                channel: req.to,
                body: req.body,
                // The board's window is the CHANNEL's own `timeout:` — the per-call override went
                // with `send --deadline` (nxf 6j6v.ckeq, decision 2). `None` is what "read the
                // declaration" has always meant on this field.
                deadline: None,
                // The two the caller no longer chooses (decision 3). `Info`/`Normal` are the
                // defaults every caller already got unless it said otherwise, so what a bare
                // `send --to <channel>` writes is byte-identical to before.
                kind: MessageKind::Info,
                priority: Priority::Normal,
                refs,
                model: None,
            },
        )?;
        let channel = format!("decl:{}", req.to);
        let thread_id = receipt.board.thread_id;
        return Ok(SendToReceipt {
            handed_to: None,
            thread_id: thread_id.clone(),
            message_id: receipt.board.message_id,
            to: req.to.to_string(),
            target: TargetKind::Channel,
            channel,
            session: None,
            queued_behind: None,
            queue_position: None,
            expects: receipt.board.expects,
            deadline: receipt.board.deadline.clone(),
            // `channel_open`'s fan-out propagates `Err` synchronously on a member trigger FAILURE,
            // so reaching `Ok` here means no member failed to start.
            //
            // **It does not mean every member is running**, and this comment used to say it did
            // (corrected under nxf 6j6v.1xw1). A member whose trigger loses the working-tree race is
            // QUEUED, not failed: `trigger_role` answers `TriggerAdmission::Queued` and
            // `coordinator_commission` reports it as `spawned: false` on its own `TriggerReceipt`
            // — which
            // this branch never sees, because `channel_open` returns one receipt for the whole
            // fan-out and does not relay the members' admissions. So a `working_tree: exclusive`
            // channel opening behind another holder reports `spawned: true` here with every member
            // parked. Pre-existing and unchanged by 6j6v.1xw1, which only makes it easier to reach
            // (the whole fan-out now queues together where a `shared` member used to start beside a
            // queued one). Where a requester DOES find out is the board:
            // `nxc threads list`/`show` report `working_tree: "waiting"` and the queue position for
            // each member thread (nxf 6j6v.qk5b), pinned by
            // `working_tree_claim_scope.rs::a_channel_is_protected_because_a_declared_member_needs_it_and_review_does_not_inherit_that`.
            // Relaying it onto this receipt is a change to what a channel send REPORTS, which is
            // 6j6v.hq71's surface question and not this item's.
            spawned: true,
            // Relayed from the fan-out (nxf 6j6v.p3sm). This was `Vec::new()` with a doc paragraph
            // explaining that the channel path had nothing to report — which was the defect, not the
            // design: the board's declared `timeout:` is armed inside that call, and a caller whose
            // window was never armed read `warnings: []` over a round nothing would ever release.
            // Real fan-out FAILURES still propagate as `Err` here, unchanged; what travels is what
            // is genuinely best-effort.
            // **Plus the state of the machine itself** (nxf 6j6v.0j12): a fan-out whose windows
            // were all armed successfully still reports nothing running to honour them, and a
            // channel that declares no `timeout:` at all now reports it too — where before it was
            // the one shape that could not.
            warnings: with_service_finding(ctx, receipt.warnings, &thread_id),
            refs_warning,
            // The DECLARED window, which on this branch is a real number: a channel's `timeout:`
            // resolves to an absolute instant when the board opens, and it is the difference
            // between "wait inline" and "come back later" (nxf 6j6v.7qfm, decision 3).
            await_: Await::for_caller(persona, &thread_id, receipt.board.deadline),
        });
    }

    // (2) A declared persona. Looked up on the catalogue directly rather than through
    // `Definitions::role`, whose `validation` error for a malformed handle would be the wrong
    // answer for a target that is simply a channel id with a shape no role handle may have.
    if let Some(decl) = ctx
        .defs
        .roles()
        .iter()
        .find(|r| r.handle == req.to)
        .cloned()
    {
        if !decl.addressable.allows_direct_from(&identity) {
            return Err(NxfError::validation(refusal(&decl, &identity, ctx)));
        }
        // THE DEPTH GUARD, BEFORE THE FIRST WRITE (nxf 6j6v.dvyq §3, block b). This branch used to
        // persist two things of its own before the guard ran — `ensure_dm_channel` materialising a
        // channel and `open_thread` stamping a thread — so a caller past the cap left both behind
        // and only THEN was refused.
        //
        // **Both moved into the coordinator in nxf 6j6v.ntp9, and the guard is its first act**, so
        // this line no longer protects a write of this branch's own — there is none left. It stays
        // because REFUSING EARLY is the property, not the ordering that happened to give it:
        // `tests/parity.rs`'s rejection case asserts that a refused verb leaves the op log exactly
        // as `init` left it, on both seams, and it must keep passing if a future line is ever added
        // above the hand-off. Re-running the check inside the coordinator is harmless and
        // deliberate: [`orchestration::resolve_hop`] reads and checks, never writes, and the
        // `CheckedHop` it returns is the certificate the spawning primitives demand.
        orchestration::resolve_hop(ctx, store)?;

        // **Handed to the persona coordinator, whole** (nxf 6j6v.ntp9) — the twin of the channel
        // branch above, which hands its target to `channel_open` and builds a receipt from what
        // comes back.
        //
        // This branch used to do three things first: materialise the direct conversation, open the
        // thread, and compose the line that tells the persona which thread to answer into. All
        // three are the coordinator's now, and moving them is the point rather than tidying: they
        // are what "the coordinator opens the thread and hands back its id" means, and while they
        // lived here they held for `nxc send` and `Engine::send_to` and for no other route into the
        // same verb. `crate::orchestration::Commission::thread` states the defect that leaves.
        let receipt = orchestration::coordinator_commission(
            ctx,
            store,
            Commission {
                machine: req.machine,
                role: &decl.handle,
                body: req.body,
                // The coordinator materialises the direct conversation between this caller and this
                // persona — it is the same `ensure_dm_channel` this branch used to call, one level
                // down, and naming it here would be a second place deciding which channel a 1:1
                // lives in.
                channel: None,
                // The three the caller no longer chooses (nxf 6j6v.ckeq, decision 3) and the model
                // the DECLARATION chooses (decision 2). Every one of these is the default a caller
                // that said nothing already got, so a bare `send --to <persona>` is unchanged.
                //
                // `priority` is the one with a consequence beyond the label, and it is deliberate:
                // it is the working-tree queue's first sort key (`working_tree.rs`:
                // `(priority, enqueued_at, id)`), so a constant `Normal` makes that queue pure FIFO.
                // Owner: "Reines FIFO ist erst einmal okay. […] ich will erst die Probleme sammeln,
                // die durch mangelnde Prio entstehen." Held by
                // `surface_cli.rs::the_working_tree_queue_is_fifo_and_nothing_on_the_surface_can_reorder_it`.
                kind: MessageKind::Info,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                // **The coordinator opens it** and hands the id back on the receipt. A fresh
                // `send --to <persona>` starts a conversation; it has no thread to name.
                thread: None,
                refs,
                model: None,
                // A fresh `send --to <persona>` starts a conversation, so there is no session of
                // this caller's to continue. Only a stepped channel's advance ever passes `Some`
                // (nxf 6j6v.y1t9).
                continue_session: None,
            },
        )?;
        let thread_id = receipt.thread;
        // The coordinator (nxf 6j6v.jepk) just declared the summoned persona as who this fresh
        // thread expects a reply from — read it back rather than hardcode `[origin/handle]`, so the
        // receipt never claims less than `thread_quorums` will show for the same thread (PR review,
        // Finding 3: an app reading `expects: []` here used to conclude nobody owed a reply while
        // the store already disagreed).
        let quorum = store.thread_quorum(&thread_id, ctx.now)?;
        // Through [`crate::awaiting::deadline_of`], which is where "only while somebody stands
        // behind it" is written (nxf 6j6v.0vd9). A no-op on this path in the ordinary case — the
        // coordinator has just declared the summoned persona — and here anyway, because the two
        // surfaces that report a window must not report it under two rules.
        let deadline = crate::awaiting::deadline_of(quorum.as_ref());
        let expects = quorum.map(|q| q.expects).unwrap_or_default();
        return Ok(SendToReceipt {
            handed_to: receipt.handed_to,
            thread_id: thread_id.clone(),
            message_id: receipt.message_id,
            to: req.to.to_string(),
            target: TargetKind::Persona,
            channel: receipt.channel,
            // A chat handed to another machine has no session here (nxf 6j6v.1c6k) — the machine
            // that picks it up mints its own.
            session: (!receipt.session.is_empty()).then_some(receipt.session),
            // Relayed, never re-derived: `coordinator_commission` already asked the lease, and
            // asking a
            // second time here would be a second answer that could disagree with the first.
            queued_behind: receipt.queued_behind,
            queue_position: receipt.queue_position,
            expects,
            deadline: None,
            // Relayed, same reasoning as `queued_behind`/`queue_position` above:
            // `coordinator_commission`
            // already resolved this, so re-deriving it here would risk a second, disagreeing answer.
            spawned: receipt.spawned,
            // Relayed verbatim (nxf 6j6v.hpv8): `coordinator_commission` never fails outright for a
            // post-persist trigger failure any more — it reports it here instead — so this is the
            // ONLY place that fact can still reach a `send --to <persona>` caller.
            warnings: with_service_finding(ctx, receipt.warnings, &thread_id),
            refs_warning,
            // Read off the THREAD rather than assumed absent: no persona declaration carries a
            // window today, so this is `None` in every workspace that exists — and it is the
            // thread's own register that says so, which is what keeps the field true if one ever
            // does.
            await_: Await::for_caller(persona, &thread_id, deadline),
        });
    }

    // (3) A channel that EXISTS but that NO DECLARATION NAMES — a raw channel. Until nxf 6j6v.dvyq
    // §3 this branch posted to it, in a thread of its own; that is the entrance the consolidation
    // abolishes. The owner's reason, 2026-08-13: a channel no declaration names is something
    // addressable with no policy behind it — nobody declared who answers, by when, or what becomes
    // of the answers, so nothing here can route them.
    //
    // It is REFUSED BY NAME rather than falling through to (4b)'s "no such target", and the
    // difference is the whole point: the id the caller typed is spelled perfectly and the channel
    // really is there, so "no such target" would send them hunting for a typo that does not exist.
    // Same shape and same reason as the empty-catalogue rejection below, and as nxf 6j6v.hpv8.
    //
    // `validation`, not `not_found`: the target was found. What is wrong is that it cannot be
    // addressed — the same answer, and the same error kind, a persona whose `addressable` forbids a
    // direct conversation gets in (2) above.
    //
    // The reachable case in a workspace built after the cut is the DIRECT CONVERSATION that (2)
    // opens: `ensure_dm_channel` materialises a real channel that no declaration names, and it is
    // reached by naming the PERSONA, never the channel id. Older workspaces also hold whatever
    // `channels create`/`join` left behind; those messages stay readable through `search` and
    // `inbox`, but the channel stops being a target.
    if store.channel_exists(req.to)? {
        let belongs = ctx
            .defs
            .source()
            .map(|s| format!("{}/channels.yaml", s.path.display()))
            .unwrap_or_else(|| "the workspace's declaration folder".to_string());
        return Err(NxfError::validation(format!(
            "{} exists in this workspace, but no declaration names it, so it cannot be \
             addressed. Declare it in {belongs} — or, for a direct conversation, name the \
             persona: `send --to <persona>`.",
            req.to,
        )));
    }

    // (4a) Nothing is declared AT ALL — the bootstrap case (nxf 6j6v.dvyq step 4). This is the ONE
    // of the three empty-catalogue surfaces that genuinely fails: `send --to` is an action with a
    // mandatory target, and without a declaration there is none. `list` and `prime` are orientation
    // and merely say it; `Engine::definitions` returns the empty catalogue, because a read that
    // fails BECAUSE there is nothing to read is a bad read. What must not happen is the version
    // this replaces: "no such target: x", which sends the caller looking for a typo in a workspace
    // where no target could have existed.
    if ctx.defs.is_empty() {
        return Err(NxfError::not_found(format!(
            "nothing is declared in this workspace, so `send --to {}` has no target it could \
             reach. {}",
            req.to,
            ctx.defs
                .source()
                .map(DeclarationSource::explain)
                .unwrap_or_else(|| "No declaration catalogue was resolved.".to_string()),
        )));
    }

    // (4b) Nothing matched. Name all three namespaces that were searched — a target that is a typo
    // for a persona and a target that is a stale channel id read identically otherwise.
    Err(NxfError::not_found(format!(
        "no such target: {} (not a declared channel, not a declared persona, not a channel in this \
         workspace) — `nxc list` shows what can be addressed",
        req.to
    )))
}

// `open_thread` stood here — `ambient_parent` plus `open_child_thread`, the tree edge a
// `send --to <persona>` stamps (nxf 6j6v.a71h §3.1). MOVED INTO THE COORDINATOR by nxf 6j6v.ntp9,
// which is the ticket's own "Faden anlegen und die Id zurueckgeben": while it stood here, a
// commission carried a thread because the SURFACE remembered to open one, and any other route into
// the same verb reached a role with nothing to answer into. It is `coordinator_commission_in`'s
// second act now, and its id comes back on the receipt.

/// **Name a thread from the message that opened it** (nxf 6j6v.e76c) — what the run the coordinator
/// commissioned actually does, and the only path in this engine that reaches a model for a name.
///
/// Three steps and none of them may fail the caller: read the thread's FIRST message, ask
/// [`crate::naming::Namer::derive`] for a name, and write it under
/// [`crate::facade::name_thread`]'s once-only rule. A thread with no messages, a namer that
/// declines, a model that is unreachable — all three answer `named: false` with no name, which is
/// the state every surface already renders.
///
/// **The FIRST message, not the newest**, and that is the item's third property in the one place it
/// can be enforced: a thread is named at its opening, so a name derived later from a later message
/// would move under a reader. `messages_in_thread` is in causal order, so "first" is the message
/// the thread was opened with.
///
/// An unknown thread is `not_found` — [`crate::facade::name_thread`]'s answer, reached through it.
///
/// # Two cheap questions come FIRST, and both were once asked too late
///
/// **May this caller read the thread at all** (review of PR #452, Integrity & Robustness · Medium).
/// This used to read `messages_in_thread` outright, which made it the one thread-content read in
/// this crate with no gate on it: a caller `nxc threads show <id>` refuses could still have that
/// thread's opening message read out — and handed to an external model. The gate is
/// [`crate::facade::require_thread_readable`], resolved exactly as `withdraw` resolves it one
/// module over, and what makes it applicable HERE is that the commissioned run carries the
/// commissioning caller's own identity rather than the machine's `$USER`
/// ([`crate::naming::ModelNamer::commission`] stamps it).
///
/// **Is it already named** (review of PR #452, Integrity & Robustness · High). The once-only rule
/// lives one layer up in [`crate::facade::name_thread`], and while that is where it BELONGS — a
/// rule at the write cannot be forgotten by a call site — it only discards the RESULT. Everything
/// expensive had already happened: [`crate::naming::Namer::derive`] is a real `claude` subprocess
/// with a 45-second bound, and `name_thread` is deliberately not gated on the opener, so a caller
/// re-running `nxc threads name <id>` in a loop against one already-named thread bought unbounded
/// model invocations for exactly no effect. Asking the register first is a point read, and the
/// answer still comes back through the facade so the receipt shape stays in one place.
pub fn name_thread_from_its_opening_message(
    ctx: &Ctx,
    store: &mut ChatStore,
    thread_id: &str,
) -> Result<crate::facade::NameReceipt> {
    let channel = store
        .thread_channel(thread_id)
        .or_else(|| store.thread_channel_via_message(thread_id))
        .ok_or_else(|| NxfError::not_found(format!("no such thread: {thread_id}")))?;
    let policy = crate::channel::declared_policy(ctx.defs.channels(), Some(&channel));
    crate::facade::require_thread_readable(
        store,
        &channel,
        thread_id,
        &ctx.caller_handle(),
        &policy,
    )?;
    if store.thread_name(thread_id)?.is_some() {
        return crate::facade::name_thread(store, ctx.now, &ctx.caller_handle(), thread_id, "");
    }
    // The namer is a model run over this text, so it reads only a message an agent action may follow
    // (nxf 6j6v.pzkb): an opening nobody here can vouch for derives no name — `named: false`.
    let opening = store
        .acting_messages_in_thread(thread_id)?
        .into_iter()
        .next()
        .map(|m| m.body);
    let derived = opening.as_deref().and_then(|body| ctx.namer.derive(body));
    match derived {
        Some(name) => {
            crate::facade::name_thread(store, ctx.now, &ctx.caller_handle(), thread_id, &name)
        }
        // Still through the facade, so an unknown thread is still `not_found` and a thread that was
        // already named still reports the name it has: what changed is only that nothing new was
        // derived, and that is `named: false`, not a different verb.
        None => crate::facade::name_thread(store, ctx.now, &ctx.caller_handle(), thread_id, ""),
    }
}

/// Answer in an existing thread (surface draft §4.3).
///
/// **Three statements, and the third is one bit** (nxf 6j6v.ckeq). `kind`, `priority` and
/// `disposition` went as caller options with `send_to`'s; `model` went to the declaration; `refs`
/// went because a reply INHERITS its subject — the thread already says what the conversation is
/// about, which is exactly why the obligation landed on `send_to` and not here (owner: "jede ERSTE
/// Nachricht bezieht sich wahrscheinlich auf ein Ticket"). The return address a reply carries is
/// stamped by [`orchestration::reply`] from the ambient session either way, so nothing that made a
/// conversation resumable depended on the caller passing it.
///
/// [`if_unanswered`] went for a different reason and is documented at
/// [`settle_if_unanswered`], which is where the mechanism now lives.
///
/// [`if_unanswered`]: settle_if_unanswered
#[derive(Debug, Clone)]
pub struct ReplyThreadRequest<'a> {
    /// The workspace-unique thread id — no channel context needed.
    pub thread: &'a str,
    pub body: &'a str,
    /// **"I cannot reach the result on my own — I need help, or a decision."** One of the three
    /// things an agent may say with a reply; the others are "I am finished" (a bare reply) and
    /// [`needs_rework`](ReplyThreadRequest::needs_rework), which is about somebody ELSE's work.
    ///
    /// It is a BOOLEAN and not a `kind` since nxf 6j6v.ckeq, and that is the shape it always
    /// effectively had: `--escalate` was the one door to [`MessageKind::Escalation`] and
    /// `--kind escalation` was refused outright, so the five-valued option was really one bit plus
    /// four labels nothing read. The CARRIER is unchanged — this resolves to that same variant, the
    /// reducer writes it into `messages.kind`, and `ChatStore::last_reply_escalated` compares that
    /// column against [`crate::model::KIND_ESCALATION`]. Both rules that hang off it are therefore
    /// untouched: whether the claim thread closes and releases the working copy (nxf 6j6v.1xw1), and
    /// whether a consolidator folds an escalating set or passes it through (nxf 6j6v.e9qj).
    ///
    /// **A THIRD rule hangs off it since nxf 6j6v.ma7v**: on a `flow: sequential` channel this bit
    /// ends the chain instead of releasing the next step. An embedding app that sets it is therefore
    /// not only labelling an answer — it is deciding that the work which was to follow will not be
    /// commissioned, and that the round goes back to whoever asked for it.
    pub escalate: bool,
    /// **"What you handed me does not meet the standard."** The THIRD and last thing an agent may
    /// say with a reply (nxf 6j6v.553s (a)), and the one that is a statement about SOMEBODY ELSE's
    /// work rather than about the sender's own.
    ///
    /// Resolves to [`MessageKind::NeedsRework`], the same way [`escalate`](ReplyThreadRequest::escalate)
    /// resolves to `Escalation` — one mapping, [`reply_kind`], so a host and the CLI cannot mean two
    /// things by it.
    ///
    /// **What it DOES is declared, not fixed here**: on a channel whose flow declares
    /// [`steps:`](crate::channel::ChannelDecl::steps), the step this reply answers routes over its
    /// `on_needs_rework:` edge — back to the step that produced the work, with the run's pass counter
    /// advanced and `max_passes` deciding whether another attempt exists at all. A step that declares
    /// no such edge never OFFERS the bit to the party serving it, and a bit set anyway is dropped and
    /// reported on that reply's own receipt.
    ///
    /// **Setting it together with `escalate` is a validation error**, refused before anything is
    /// posted: the two are statements about different work with opposite consequences (one ends the
    /// chain and travels up; the other sends it back and keeps going), and picking a winner would be
    /// this engine deciding which of two things the caller meant.
    pub needs_rework: bool,
    /// **"I know what the review said. This goes on anyway."** — the FOURTH thing a reply can say
    /// (nxf 6j6v.am8j), and the only one that is not said by a party serving a step.
    ///
    /// Resolves to [`MessageKind::Accepted`] through the same one mapping the other two bits go
    /// through ([`reply_kind`]), so a host and the CLI cannot mean two things by it.
    ///
    /// **What it DOES**: on the thread of a round the caller commissioned, the step whose verdict
    /// sent the work back counts as satisfied and the run continues over that step's
    /// [`next:`](crate::channel::FlowStep::next). The successor is handed the caller's own words
    /// together with the verdict they set aside
    /// ([`compose_accepted_notice`](crate::channel::compose_accepted_notice)), so it cannot mistake
    /// an override for a review that passed.
    ///
    /// **Who may set it is a rule, not a convention.** It is honoured only on a CHANNEL thread whose
    /// recorded opener is the caller — the party that asked for the round. A party whose own work
    /// was sent back is refused by name and pointed at
    /// [`escalate`](ReplyThreadRequest::escalate): a producer that could declare its own reviewer
    /// satisfied dissolves the separation the round exists to create. An app embedding this engine
    /// gets the same rule; it is enforced here and not left to the host.
    ///
    /// **Setting it together with either other bit is a validation error**, refused before anything
    /// is posted, for [`refuse_two_verdicts`]'s reason: they are statements about different work
    /// with different consequences, and picking a winner would be the engine deciding which of them
    /// the caller meant.
    pub accept: bool,
    /// **Hand this chat to another machine** (nxf 6j6v.1c6k) — a machine id or a name the
    /// workspace's relay knows. `None` leaves the chat on the machine it runs on. Only a persona chat
    /// has a machine; naming one for any other thread is refused.
    pub machine: Option<&'a str>,
}

/// The kind a reply's declared bits resolve to — the ONE mapping, so the CLI's `--escalate`/
/// `--needs-rework`/`--accept` and an app's [`ReplyThreadRequest`] fields cannot drift into meaning
/// different things.
///
/// **The ENDINGS A STEP HAS are a trichotomy** (owner, 2026-08-29) — done, again, cannot — and the
/// first two bits name them. `--accept` (nxf 6j6v.am8j) is not a fourth ending and does not belong
/// to that list at all: it is said by the party that COMMISSIONED a round, about a judgement the
/// round produced, and nobody serving a step may say it. So the three bits name four kinds, and
/// every combination of two or more is not a state but a caller saying several different things
/// about several different pieces of work.
///
/// Those combinations are refused by [`refuse_two_verdicts`] before this is ever reached; the arm
/// order here is the belt to that braces, and it lets `Escalation` win because holding a working
/// copy is the benign direction of a mistake.
fn reply_kind(escalate: bool, needs_rework: bool, accept: bool) -> MessageKind {
    match (escalate, needs_rework, accept) {
        (true, _, _) => MessageKind::Escalation,
        (false, true, _) => MessageKind::NeedsRework,
        (false, false, true) => MessageKind::Accepted,
        (false, false, false) => MessageKind::Info,
    }
}

/// Refuse a reply that sets MORE THAN ONE declared bit, before anything is written.
///
/// Named rather than inlined because both entrances ([`reply_in_thread`] and
/// [`settle_if_unanswered`]) run it and a second copy is how the two would come to accept different
/// combinations. Counting rather than enumerating the pairs since nxf 6j6v.am8j made them three:
/// a list of pairs is a thing the next author has to remember to extend, and the fourth bit would
/// have arrived with three unchecked combinations.
fn refuse_two_verdicts(req: &ReplyThreadRequest) -> Result<()> {
    if [req.escalate, req.needs_rework, req.accept]
        .iter()
        .filter(|set| **set)
        .count()
        > 1
    {
        return Err(NxfError::validation(
            "a reply carries at most one declared bit: `--escalate` says you cannot carry out your \
             OWN task and ends the chain; `--needs-rework` says somebody ELSE's work must be put \
             right and sends the round back; `--accept` says a verdict on a round YOU commissioned \
             is overruled and the chain goes on. Setting more than one says several different \
             things about several different pieces of work",
        ));
    }
    Ok(())
}

/// Post a reply into `thread` and hand the turn to whoever is on the other side of it.
///
/// **[`orchestration::reply`] does the work.** The quorum-completion routing, a declared channel's
/// `on_complete` policy, the depth guard, the ambient return-address stamp — all of it runs here
/// verbatim, because this calls that verb with the
/// thread as its target. There is no second copy of any of it.
///
/// What this adds is the one thing a thread-addressed conversation needs and a message-addressed
/// one never did: **a return address for the THREAD.** `orchestration::reply`'s direct resume reads
/// `refs.session_id` off the target MESSAGE, so a thread id — which is not a message id — resolves
/// to nothing and today's `nxc reply <thread-id>` wakes nobody. The thread's return address is the
/// newest message in it from somebody else that carries a session, which is exactly "the session
/// this conversation is currently with".
///
/// It runs **only when the shared verb posted AND woke nobody** ([`settle_if_unanswered`] may have
/// found nothing owed, and nothing owed means nothing posted, means nothing to hand a return address
/// a reply that does not exist; beyond that, no direct resume, no completion wake, and no wake it
/// tried and missed), so a completing reply into a quorum board keeps its own routing and no path
/// can wake twice. When there is nothing to resume — the counterpart has not answered yet, or it is
/// a human, who has no session — the reply is still posted and the receipt says plainly that nothing
/// was woken.
pub fn reply_in_thread(
    ctx: &Ctx,
    store: &mut ChatStore,
    req: ReplyThreadRequest,
) -> Result<ReplyReceipt> {
    refuse_a_reply_the_supervisor_is_finished_with(ctx, store, req.thread)?;
    refuse_a_finished_claim_while_your_own_round_runs(ctx, store, &req)?;
    // **An acceptance that would move nothing is refused rather than posted** (nxf 6j6v.am8j). It
    // is the one thing this verb says whose entire content is a consequence, so a message with no
    // consequence behind it is not a partial success — it reads, afterwards, exactly like one that
    // worked. See the orchestration function for what it asks and what each refusal sends the
    // caller to do instead.
    if req.accept {
        orchestration::refuse_an_acceptance_that_cannot_be_honoured(ctx, store, req.thread)?;
    }
    refuse_two_verdicts(&req)?;
    let machine = the_chats_machine_before_the_post(ctx, store, &req)?;
    let mut receipt = reply_in_thread_inner(ctx, store, req, false)?;
    // The richer answer the preflight read (name, online, last seen) replaces the bare id the
    // routing knows, for the same machine.
    if let (Some(to), Some(m)) = (receipt.handed_to.as_ref(), machine) {
        if to.machine_id == m.machine_id && !m.here {
            receipt.handed_to = Some(m);
        }
    }
    Ok(receipt)
}

/// What [`machine`] is asked about (nxf 6j6v.1c6k): a chat about to start, or one that runs.
#[derive(Debug, Clone, Copy)]
pub enum MachineQuery<'a> {
    /// A new chat with this persona — where it would run, and whether starting it would ask.
    /// `machine` is the choice the caller is about to make (`send --to <p> --machine <m>`).
    Persona {
        to: &'a str,
        machine: Option<&'a str>,
    },
    /// A chat that exists — where it runs, and whether a reply would ask. `machine` is a hand-over
    /// the caller is about to make (`reply --thread <t> --machine <m>`).
    Thread {
        thread: &'a str,
        machine: Option<&'a str>,
    },
}

/// **The question, as data, before anything is sent** (nxf 6j6v.1c6k) — `nxc machine`,
/// `Engine::machine`. Where a chat runs or would run, where that came from, whether it is online,
/// whether sending now would ASK, and the machines to choose from. It writes nothing; it is exactly
/// the resolution [`send_to`] and [`reply_in_thread`] make before they write, so an app can render
/// the choice first and a refusal never surprises it.
pub fn machine(
    ctx: &Ctx,
    store: &ChatStore,
    query: MachineQuery,
) -> Result<crate::machine::MachineAnswer> {
    use crate::machine::{resolve, Wanted};
    match query {
        MachineQuery::Persona { to, machine } => {
            if ctx.defs.declared_channel(to)?.is_some() {
                return Err(NxfError::validation(format!(
                    "{to} is a channel, and a channel runs on the machine that starts it"
                )));
            }
            let decl = ctx.defs.role(to)?;
            let persona = ctx
                .session
                .is_none()
                .then_some(decl.machine.as_deref())
                .flatten();
            Ok(resolve(
                ctx.machines,
                ctx.db_path,
                Wanted {
                    choice: machine,
                    chat: None,
                    persona,
                },
            ))
        }
        MachineQuery::Thread { thread, machine } => {
            let chat = orchestration::persona_chat(ctx, store, thread)?.ok_or_else(|| {
                NxfError::validation(format!(
                    "only a persona chat runs on a machine, and {thread} is not one"
                ))
            })?;
            let on = chat.machine.filter(|m| m.acts);
            Ok(resolve(
                ctx.machines,
                ctx.db_path,
                Wanted {
                    choice: machine,
                    chat: on.as_ref().map(|m| m.machine_id.as_str()),
                    persona: None,
                },
            ))
        }
    }
}

/// **Which machine this reply goes to, settled before anything is written** (nxf 6j6v.1c6k).
///
/// Two things happen here and nowhere later, because both must come BEFORE the post:
///
/// - **`--machine` hands the chat over.** Resolved (a name the relay does not know asks, like at
///   the start of a chat), refused for a thread that is not a persona chat and for a caller that
///   is a session — handing a chat to another machine is the person's decision — and then written
///   into the chat's register, so the routing that follows the post already sees the new machine.
/// - **A chat whose machine is not online ASKS** — the owner's rule, for a reply as for a start:
///   nothing is posted, and the refusal lists the machines that are online. Only a person is asked;
///   a persona answering in its own chat is by definition standing on the chat's machine.
fn the_chats_machine_before_the_post(
    ctx: &Ctx,
    store: &mut ChatStore,
    req: &ReplyThreadRequest,
) -> Result<Option<crate::machine::ExecutingMachine>> {
    use crate::machine::{resolve_or_ask, Wanted};
    const RETRY: &str = "--machine <id|name>";
    let chat = orchestration::persona_chat(ctx, store, req.thread)?;
    if let Some(choice) = req.machine {
        let Some(chat) = chat else {
            return Err(NxfError::validation(format!(
                "only a persona chat runs on a machine, and {} is not one — a channel runs where it \
                 is started",
                req.thread
            )));
        };
        if ctx.machines.is_none() {
            return Err(NxfError::validation(
                "this host names no machines, so a chat cannot be handed to one — `--machine` \
                 needs the `nxs` binary or an embedding host that passes its machines",
            ));
        }
        if ctx.session.is_some() {
            return Err(NxfError::validation(
                "handing a chat to another machine is the decision of the person it runs for, not \
                 of a session inside it",
            ));
        }
        let answer = resolve_or_ask(
            ctx.machines,
            ctx.db_path,
            Wanted {
                choice: Some(choice),
                ..Wanted::default()
            },
            RETRY,
        )?;
        let Some(to) = answer.machine else {
            return Ok(None);
        };
        let already = chat
            .machine
            .as_ref()
            .is_some_and(|m| m.acts && m.machine_id == to.machine_id);
        if !already {
            store.set_thread_machine(req.thread, &to.machine_id, &ctx.caller_handle());
        }
        return Ok(Some(to));
    }
    let Some(on) = chat.and_then(|c| c.machine).filter(|m| m.acts) else {
        return Ok(None);
    };
    if ctx.session.is_some() {
        return Ok(None);
    }
    let answer = resolve_or_ask(
        ctx.machines,
        ctx.db_path,
        Wanted {
            chat: Some(&on.machine_id),
            ..Wanted::default()
        },
        RETRY,
    )?;
    Ok(answer.machine)
}

/// **Refuse "I am finished" from a caller whose own sub-round is still running** (nxf 6j6v.hw2t).
///
/// # What a bare reply means, and why this one would be false
///
/// A reply means DONE — the engine's own obligation text says so in the words every commissioned
/// session reads, and a declared flow acts on it by starting the next step. A role that consulted
/// somebody and is waiting for the answer is not done, and the three things it could say before this
/// item were all untrue: `reply` claims a result it does not have, silence buys a reminder and then
/// a substitute escalation posted in its name, and `--escalate` says "I need help, or a decision"
/// while stopping a chain nothing was wrong with. Measured on 2026-09-08 in the owner's own workspace: a
/// `head-of-marketing` picked the third, and the operation view read `NEEDS DECISION` over four
/// working sessions.
///
/// The fourth answer is to say nothing at all and end the turn — the coordinator wakes the session
/// when the sub-round comes back (nxf 6j6v.gn8b delivers it) — and this refusal is the sentence that
/// names it, at the one point where the false claim would otherwise be written. Same shape as
/// [`refuse_a_reply_the_supervisor_is_finished_with`] beside it (nxf 6j6v.0vd9): turn it away at
/// the write point with a reason that can be acted on, rather than accepting it in silence.
///
/// # What stays possible
///
/// **`--escalate`**, deliberately. "I need help, or a decision" can be true while a sub-round runs —
/// an agent that discovers it lacks a credential learns nothing more by waiting — and it is the only
/// remaining way a chain is stopped, which is what gives the marker its meaning back. Same for
/// `--needs-rework`, which is a verdict on somebody else's work and not a claim about this caller's
/// own. Only the FINISHED claim is impossible here.
///
/// **And the teardown's door is untouched**: [`settle_if_unanswered`] does not run this, for exactly
/// the reason it does not run its neighbour. That door fires when a session ended still owing an
/// answer, and refusing it there would leave the thread owing one forever. The runtime does not
/// reach it in this state at all — `agent-sidecar/src/main.mjs` reads the same derived field off
/// `nxc status` and posts nothing.
///
/// # The order of the checks is the cost argument
///
/// The item asked that the children read be checked rather than assumed, and it is — see
/// [`ChatStore::thread_children`] for the measured plan (a covering-index scan, not a tree walk).
/// It is still put LAST behind three cheaper questions, in this order: is this a plain reply at all
/// (free), does this thread have a board (one indexed seek, which
/// [`reply_in_thread_inner`] makes further down anyway), and is the CALLER one of the handles it is
/// waiting for (free, off that seek). An ordinary reply — the one that discharges its thread — stops
/// at the third and touches no scan.
fn refuse_a_finished_claim_while_your_own_round_runs(
    ctx: &Ctx,
    store: &ChatStore,
    req: &ReplyThreadRequest<'_>,
) -> Result<()> {
    // Neither of the other two things a reply can say is a claim of completion, so neither is
    // touched by this rule. Free, and first.
    if req.escalate || req.needs_rework || req.accept {
        return Ok(());
    }
    let Some(q) = store.thread_quorum(req.thread, ctx.now)? else {
        return Ok(());
    };
    let caller = ctx.caller_handle();
    // **Only the party that OWES this thread an answer can claim to be finished with it.** A
    // requester taking another turn in its own conversation, a bystander posting a note, the
    // supervisor's own traffic — none of them is saying "done", and there is nothing to refuse.
    if !q.outstanding.iter().any(|owed| owed == &caller) {
        return Ok(());
    }
    let kids = store.thread_children(req.thread)?;
    if kids.is_empty() {
        return Ok(());
    }
    // **BOUNDED, on the precedent the crate already set** (review of PR #460, Integrity #3):
    // [`ChatStore::thread_quorums`] binds one SQL variable per id and SQLite's default
    // `SQLITE_MAX_VARIABLE_NUMBER` is 999, which is why `facade`'s own reads switch to the
    // workspace-wide query past [`facade::MAX_BOUND_IDS`] rather than growing the list. Nothing
    // stops an agent from opening a thousand consultations under one thread, and the failure
    // without this would be the worst kind: an opaque SQL error in place of a clean success or the
    // typed refusal below, on the ordinary `nxc reply` path.
    let ids: BTreeSet<&str> = kids.iter().map(String::as_str).collect();
    let child_quorums = match ids.len() > crate::facade::MAX_BOUND_IDS {
        // **Filtered back down to the children, and that is not optional**: the fallback answers
        // for EVERY thread in the workspace, while the predicate below weighs an `opener` and an
        // `outstanding` and asks nothing about parentage. Handed the whole workspace it would
        // report a caller's consultations from a different operation as this thread's sub-round.
        true => store
            .thread_quorums_all(ctx.now)?
            .into_iter()
            .filter(|q| ids.contains(q.thread_id.as_str()))
            .collect(),
        false => store.thread_quorums(&ids.iter().copied().collect::<Vec<_>>(), ctx.now)?,
    };
    // **Scoped to THIS caller, not to the thread** (review of PR #460, Code Quality #2 / Integrity
    // #2). The status row asks the thread-level question; a refusal is about one named party's
    // claim, and on a board with two outstanding handles the thread-level answer would refuse a
    // caller for somebody else's consultation. See [`crate::awaiting::own_open_sub_round_of`].
    let waiting = crate::awaiting::own_open_sub_round_of(&caller, &q, child_quorums.iter());
    let Some(first) = waiting.first() else {
        return Ok(());
    };
    // Named rather than counted: the caller has to be able to go and READ the round it is waiting
    // on (`nxc threads show <id>`), and a count is not something a next command takes. The rest are
    // counted, because a role with four consultations open needs the sentence to stay readable.
    //
    // **`nxc withdraw` is deliberately NOT offered here**, although "give the round up instead"
    // reads like the obvious third way out, and for two reasons. One is now the engine's own: since
    // nxf 6j6v.ezbr (owner, 2026-09-20) it is a person's verb, and a caller that is a session this
    // workspace started — which is who reaches this refusal, nearly always — is refused it. The
    // other holds for a person too, and it is what this refusal is FOR: a caller who tried to say
    // "finished" while waiting on its own round needs the way to WAIT — end the turn, be woken. Withdrawing is the opposite decision, and a heavy one: it stops somebody's
    // session mid-turn and puts their half-done work on a branch. A refusal that lists it beside
    // "end your turn" invites the reader to reach for it casually, in the one moment they are
    // being told that nothing is owed and patience costs nothing. The verb is documented where a
    // reader deciding to give a round up will look (`nxc guide commands`), not here.
    let and_more = match waiting.len() {
        1 => String::new(),
        n => format!(" (and {} more)", n - 1),
    };
    // `Forbidden` for its neighbour's reason: nothing about this call is malformed — a rule looked
    // at the world and said no, and `Validation` would send the caller to fix the wrong thing.
    Err(NxfError::forbidden(format!(
        "you are waiting on a round you commissioned yourself — thread {first}{and_more} is \
         still open — and a plain `nxc reply` here means you are FINISHED, which is not true \
         yet. Simply end your turn instead: nothing is owed while you wait, and you are woken \
         with the answer when it arrives. Read where that round stands with `nxc threads show \
         {first}`. If you cannot reach a result at all — and that can be true while a round \
         of yours runs — `nxc reply --thread {} --escalate -` is still \
         open to you.",
        req.thread
    )))
}

/// **Refuse a reply into a round the channel supervisor has already closed** (nxf 6j6v.0vd9).
///
/// # What this refuses, and what it measured
///
/// Live in `watch-bundestag` on 2026-09-08: a `decl:coding` operation had delivered — every step
/// answered, `__delivered__` set, `nxc status` reading `finished` — and a reply into the coder's own
/// step thread, exactly the move that persona's declaration invites, came back
/// `replied <id> in thread <id>` with `The answer is due by <instant>` under it. Nothing had been
/// started and nothing could be. The owner waited twenty minutes on that deadline for a rework that
/// was never commissioned; an agent reads the same field and waits the same way, because a due date
/// is precisely what a waiting loop keys on.
///
/// # Why refusing, rather than really waking the round
///
/// The item names two acceptable behaviours — refuse with a usable reason, or reopen the round for
/// real — and rules out only the third, which is what shipped: accept, do nothing, promise an
/// answer. Reopening is the larger thing and it is somebody else's: a delivered operation has
/// consolidated its answers, released its working copy and told its requester it is done, so
/// "resume it" means deciding what happens to a delivery that has already gone out. The engine
/// already HAS doors for going on — the channel takes a new round, and its requester can take
/// another turn by answering in the round's own thread ([`orchestration`]'s
/// `supervisor_hand_out_next_turn`) — so the caller is not short of a route, only of the sentence
/// that names it. This refusal is that sentence, and it costs three reads the accepted path was
/// making anyway.
///
/// It also changes the EXIT CODE, which is the half a warning could not do: `nxc reply` exits 0 for
/// every posted message, so a caller checking the status of its own command still believed the work
/// was commissioned. A refusal is the only report a script reads without being told to.
///
/// # The predicate, and why it is drawn at the SET
///
/// It mirrors, exactly, the condition under which [`orchestration`]'s channel supervisor declines to
/// act on a reply (`supervisor_consider_set`'s own early return): the thread was opened by the
/// supervisor for ONE real party to answer, and its parent — the channel thread the set hangs under
/// — no longer expects the supervisor, which means that set was consolidated and handed on. Whenever
/// that holds, every other route out of [`orchestration::reply`] is closed on this thread by
/// construction: the direct resume skips a quorum board, the completion wake is gated on the
/// supervisor not having handled the reply, and [`reply_in_thread_inner`]'s own fallback hands the
/// turn back only to the thread's OPENER, which here is the supervisor. So the refusal never takes
/// away a wake that would have happened — it names one that could not.
///
/// Drawn at the set rather than at "this thread has been answered" deliberately: a step that already
/// replied is ordinary traffic while its run is still going (a correction, a second word, the
/// teardown's own settle), and refusing that would break the run instead of the silence.
///
/// **[`settle_if_unanswered`] deliberately does not run this**, which is why it sits on the public
/// door and not in the shared body. That door is the sidecar teardown's (see its doc), and it fires
/// exactly when a session ended still OWING an answer — including on a slot whose set already
/// settled through its deadline. Refusing it there would leave the thread owing an answer forever,
/// which is the state that door exists to end.
fn refuse_a_reply_the_supervisor_is_finished_with(
    ctx: &Ctx,
    store: &ChatStore,
    thread: &str,
) -> Result<()> {
    // No board at all — a plain conversation, or a thread this verb is about to reject as unknown.
    let Some(q) = store.thread_quorum(thread, ctx.now)? else {
        return Ok(());
    };
    let supervisor = orchestration::supervisor_qualified(ctx.origin);
    // A slot the supervisor opened for one real party. `expects` being non-empty and free of the
    // engine's own identities is what tells a member/step slot from the CHANNEL thread above it,
    // the same discriminator `supervisor_after_reply` branches on — and it is also exactly the
    // shape `orchestration::reply` reads as a quorum board, which is what closes its direct resume.
    if q.opener.as_deref() != Some(supervisor.as_str())
        || q.expects.is_empty()
        || q.expects
            .iter()
            .any(|h| orchestration::is_engine_identity(h))
    {
        return Ok(());
    }
    // A slot whose parent this workspace has not folded decides nothing here: the supervisor itself
    // does nothing with such a thread rather than deciding a set from one member, and refusing on
    // an ancestry that is only half here would be deciding it from less.
    let Some(parent) = store.thread_parent(thread)? else {
        return Ok(());
    };
    let Some(parent_q) = store.thread_quorum(&parent, ctx.now)? else {
        return Ok(());
    };
    if parent_q.expects == [supervisor] {
        return Ok(());
    }
    // **What the caller is sent to instead, named from the record rather than guessed.** The
    // DECLARED channel this slot lives in is the door that starts another round, and it is the one
    // piece of advice that is right whoever is asking: the parent thread's own next turn belongs to
    // its requester alone (`supervisor_after_reply` hands one out only to the thread's opener), so
    // pointing a bystander at it would be a second instruction that does not work either. The
    // parent is named as a READ, which is what it is good for from here.
    let channel = q
        .channel_id
        .as_deref()
        .and_then(|c| c.strip_prefix(orchestration::DECLARED_CHANNEL_PREFIX));
    let start_another = match channel {
        Some(name) => format!("`nxc send --to {name} \"…\"`"),
        // A supervised slot outside a declared channel is not a shape this engine opens; the
        // generic form is the honest fallback rather than a name invented for the message.
        None => "`nxc send --to <persona|channel>`".to_string(),
    };
    // `Forbidden`, not `Validation`, and for `supervisor_fan_out`'s reason at its own refusal:
    // nothing about this call is malformed. A rule looked at the world and said no, and the kind
    // reserved for "you wrote this wrongly" would send the caller to fix the wrong thing.
    Err(NxfError::forbidden(format!(
        "this round is over: thread {thread} served a step of the round in thread {parent}, and \
         that set has already been consolidated and handed on — nobody is waiting for a reply \
         here, and nothing would be woken by one. Read how it ended with `nxc threads show \
         {parent}`, and start the next piece of work with {start_another}."
    )))
}

/// [`reply_in_thread`], but ONLY if the caller still owes a reply on `thread` — a deliberate
/// `posted: false` no-op (never an error, never a resume attempt) when it does not. See
/// [`facade::reply`](crate::facade::reply) for exactly what "still owes" means.
///
/// **This is the sidecar teardown's door, and it is not an option on the app seam** (nxf 6j6v.ckeq,
/// decision 4). `if_unanswered` was a field on [`ReplyThreadRequest`] until then, which read as a
/// choice an agent or an app makes. It is neither. It is the IDEMPOTENCY SWITCH of one caller:
/// `agent-sidecar/src/main.mjs`'s teardown (nxf 6j6v.ww0a) runs
/// `nxc reply --thread <id> --if-unanswered <text>` when an SDK session ends without ever answering
/// the thread it owed, so the thread does not go quiet forever — and it runs it unconditionally,
/// because it cannot tell whether the session already answered. An APP never needs that: it holds
/// its own sessions and knows.
///
/// So the mechanism keeps a name of its own, one caller, and a doc that says who that caller is,
/// instead of a boolean on the request every app fills in. `Engine` deliberately does NOT expose it
/// — held by `tests/seam_disposition.rs`'s `reply --if-unanswered` row — while the CLI flag stays,
/// because the teardown reaches this binary and nothing else.
pub fn settle_if_unanswered(
    ctx: &Ctx,
    store: &mut ChatStore,
    req: ReplyThreadRequest,
) -> Result<ReplyReceipt> {
    reply_in_thread_inner(ctx, store, req, true)
}

/// The one body both entrances run. `if_unanswered` is a parameter rather than a request field so
/// that the door a caller picks IS the answer — see [`settle_if_unanswered`].
fn reply_in_thread_inner(
    ctx: &Ctx,
    store: &mut ChatStore,
    req: ReplyThreadRequest,
    if_unanswered: bool,
) -> Result<ReplyReceipt> {
    // Reject an unknown thread BEFORE the post: `facade::reply` would reject it too, but this way
    // the error names the thread rather than "thread or message", which is all this verb takes.
    refuse_two_verdicts(&req)?;
    if store.thread_channel(req.thread).is_none()
        && store.thread_channel_via_message(req.thread).is_none()
    {
        return Err(NxfError::not_found(format!(
            "no such thread: {}",
            req.thread
        )));
    }
    let hop = orchestration::resolve_hop(ctx, store)?;
    let mut receipt = orchestration::reply(
        ctx,
        store,
        ReplyRequest {
            target: req.thread,
            body: req.body,
            // The escalation note IS the kind (nxf 6j6v.ckeq, decision 3), and the rest are the
            // defaults every caller that said nothing already got. `refs` carries no CALLER-supplied
            // pointer: `orchestration::reply` stamps the ambient return address onto it
            // (`with_return_address`), which is the only pointer a reply ever needed.
            kind: reply_kind(req.escalate, req.needs_rework, req.accept),
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            // **The one pointer this function decides itself** (nxf 6j6v.kffm). `if_unanswered` is
            // the sidecar teardown's door and nobody else's (see [`settle_if_unanswered`]), so a
            // post through it is BY CONSTRUCTION the runtime speaking for a session that ended
            // owing an answer. Marking it here rather than at the CLI is what makes that true of
            // every caller of the door, and what keeps the fact out of the reach of an agent: no
            // request field carries it, so nothing an agent writes can claim it or clear it.
            //
            // A `--if-unanswered` call that finds nothing owed posts NOTHING, so no message ever
            // carries this without a substitution having happened.
            refs: Refs {
                substituted: if_unanswered,
                ..Refs::default()
            },
            model: None,
            if_unanswered,
        },
    )?;

    // `receipt.posted` gates this FIRST (nxf 6j6v.ww0a, review finding 1): `orchestration::reply`
    // already returns every other field falsy/`None` on a skip (`resumed: false`, `woke: None`,
    // `wake_skipped: None`), which on its own would satisfy the condition below and send this
    // fallback hunting for a return address to resume with a reply that was never written. A skip
    // must wake nobody, exactly as it posts nothing — `posted` is what tells the two apart from
    // "posted, but genuinely found nobody to resume" (the ordinary case this fallback exists for).
    //
    // **And only where the turn really IS handed back** (nxf 6j6v.dvyq §3). This fallback continues
    // a TWO-ENDED conversation: A answers, and B — whoever is on the other side — is resumed with
    // it. A fan-in BOARD has no other side. Its requester is woken by the board's own COMPLETION,
    // exactly once, and letting this fallback fire there wakes it again for every partial answer
    // and again for every late one.
    //
    // That property used to be carried by the CLI's SHAPE rather than stated here: `reply <target>`
    // reached `orchestration::reply` WITHOUT this fallback, so a caller answering a multi-handle
    // board never met it, while `reply --thread` was the door an agent used on its own 1:1 thread.
    // §3 collapses the two entrances into one, so the surviving shape has to carry the property the
    // removed one had — otherwise the consolidation silently regresses the "woken exactly once"
    // contract the PUSH-wake (nxf 6j6v.zebd) was built for. Its three regression tests say exactly
    // that, and each one is a caller this guard now excludes: a member whose reply COMPLETES the
    // board and happens to carry the requester's own session
    // (`reply_to_a_review_quorum_does_not_wake_the_replying_session_itself`), a member replying
    // again afterwards (`a_later_reply_…`), and a bystander who was never expected
    // (`a_bystander_…`).
    let caller = ctx.caller_handle();
    // Read ONCE and used twice (nxf 6j6v.7qfm): this decides who is resumed, and its `deadline` is
    // what the caller's own `await` block reports as "how patient to be". A second read of the same
    // register is a second answer that could disagree with the first.
    //
    // **Read AFTER the post, and that is what makes the window honest** (nxf 6j6v.0vd9): the reply
    // this call just wrote may itself be what discharged the thread's obligation, and a window
    // nobody owes an answer against is an instant a caller waits out for nothing. Which windows
    // survive that is [`crate::awaiting::deadline_of`]'s single rule, shared with `send_to` above.
    let quorum = store.thread_quorum(req.thread, ctx.now)?;
    let deadline = crate::awaiting::deadline_of(quorum.as_ref());
    let hands_the_turn_back = match &quorum {
        // Not a board at all, or a board that declares nobody — the ordinary conversation this
        // fallback exists for, where the only other party IS the other side.
        None => true,
        Some(q) if q.expects.is_empty() => true,
        // A thread that DECLARES who owes it an answer hands its turn back through the board: the
        // requester is woken by the completion, once. The one caller with a turn to hand back on
        // such a thread is the REQUESTER itself, taking another turn in its own conversation —
        // which is how a 1:1 opened by `send --to <persona>` gets a second and third exchange after
        // its first answer completed the board.
        Some(q) => q.opener.as_deref() == Some(caller.as_str()),
    };
    // **A chat that runs on a machine was already routed** (nxf 6j6v.1c6k): handed to its machine,
    // or — for a message TO the persona — served there through `orchestration::serve_chat`, claims
    // and all. This fallback would be a second route around both: it resumed the persona's session
    // on THIS machine after a hand-over, and could deliver a message a pickup had already delivered.
    let routed_by_its_machine = receipt.handed_to.is_some()
        || orchestration::persona_chat(ctx, store, req.thread)?
            .is_some_and(|chat| chat.machine.is_some_and(|m| m.acts) && chat.persona != caller);
    if receipt.posted
        && !receipt.resumed
        && receipt.woke.is_none()
        && receipt.wake_skipped.is_none()
        && hands_the_turn_back
        && !routed_by_its_machine
    {
        if let Some(addr) = thread_return_address(store, req.thread, &ctx.caller_handle())? {
            // The resumed session is handed the reply PLUS the thread it must answer into — the
            // same line `send_to` gives a freshly summoned persona, and for the same reason. A live
            // run is what proved this is not decoration: without it the second turn came back as
            // assistant text inside the session's own transcript and the thread stayed silent, so
            // the human watching saw the answer being written and never received it. The posted
            // message is untouched; this is only what the session is told.
            // …and, when this reply is a HAND-BACK, the notice that says so goes ahead of it
            // (nxf 6j6v.gk9j): this is the 1:1 path, where an escalation used to arrive as ordinary
            // text and the recipient had to infer from prose that it was being asked for help.
            // **And the coordinator names the addressees while it is at it** (nxf 6j6v.dq59):
            // whom the resumed session may answer, with the verb each one takes, derived for the
            // ROLE behind the return address — resolved from the session map rather than assumed,
            // because a return address is a session id and the register that decides this holds
            // qualified handles.
            // **Never `?`, and that is a rule of this path rather than caution**: everything below
            // the post is forbidden to fail the call (see `orchestration::reply`'s step 4b), and a
            // session-map lookup that errors must not turn a written reply into an `Err`. An
            // unresolvable role reads as an empty handle, which owes nothing and is owed nothing —
            // so `reply_guidance` falls back to the sentence that names the thread.
            let resumed_handle = store
                .session_role(&addr)
                .ok()
                .flatten()
                .map(|role| ctx.qualify(&role))
                .unwrap_or_default();
            let wake = orchestration::wake_message(
                req.body,
                req.escalate,
                &orchestration::reply_guidance(store, ctx.now, &resumed_handle, Some(req.thread)),
            );
            // What is HELD if the caller turns out to be mid-turn (nxf 6j6v.gn8b) — the raw facts,
            // for the reason `orchestration::reply`'s own site states: the delivery is composed
            // once, when it goes out.
            let hold = crate::collecting::Held {
                thread_id: req.thread.to_string(),
                // **`expect`, not `unwrap_or_default`** (review of this branch, Code Quality): this
                // block is reached only under `receipt.posted`, and `orchestration::reply` sets
                // `message_id: Some(..)` for every posted reply. An empty id would be a silent
                // collision key — two held answers overwriting each other under `""` — which is the
                // one shape this queue must never produce, so the invariant is asserted rather than
                // papered over.
                message_id: receipt
                    .message_id
                    .clone()
                    .expect("a posted reply carries its message id"),
                sender: ctx.caller_handle(),
                body: req.body.to_string(),
                escalated: req.escalate,
            };
            let attempt = orchestration::resume_return_address(
                ctx,
                store,
                orchestration::Resume {
                    addr: &addr,
                    body: &wake,
                    // No per-call model any more (nxf 6j6v.ckeq, decision 2): which model the
                    // resumed session runs on is its own declaration's answer, not this replier's.
                    model: None,
                    hop,
                    // Same as the reply path's own resume: the thread this answer landed in is
                    // where the woken session now stands (nxf 6j6v.a71h §3.1).
                    thread: Some(req.thread),
                    on_busy: orchestration::OnBusy::Hold(&hold),
                },
            );
            receipt.resumed = attempt.woke.is_some();
            receipt.held = attempt.held;
            receipt.woke = attempt.woke;
            // **Into `warnings` as well as into the Option** (nxf 6j6v.93zd's invariant, applied
            // here by 6j6v.7qtf). `orchestration::reply`'s own resume site has always recorded both
            // — the Option is the FIRST finding of its class, projected out of the list, never a
            // second place to look — and this fallback, which is a resume like any other, recorded
            // only the Option. `nxc guide limits-and-safety` tells an app that `warnings` is the
            // array to watch; a wake that failed and appeared nowhere in it is exactly the silence
            // that guide is written against.
            //
            // Found in the live run of 6j6v.7qtf, not in the suite: with the single-process guard in
            // place a wake onto a session that is still working is REFUSED, and the refusal came
            // back as `wake_skipped: {reason: "already_running"}` beside an empty `warnings`.
            //
            // The class is `RequesterNotWoken` for the reason the reply path states at its own site:
            // this wake is aimed at the party whose message is being answered.
            if let Some(w) = &attempt.skipped {
                receipt
                    .warnings
                    .push(orchestration::FailedConsequence::not_woken(
                        Some(req.thread),
                        w,
                    ));
            }
            receipt.wake_skipped = attempt.skipped;
        }
    }
    // The state of the machine, on the third of the three verbs that carry a `warnings` array
    // (nxf 6j6v.0j12). A reply is where a member discharges its turn — and where a round that
    // nothing will ever release looks, from the inside, exactly like one that will.
    receipt.warnings.extend(orchestration::service_finding(
        ctx,
        receipt.thread_id.as_deref().unwrap_or(req.thread),
    ));
    // **The same treatment `send_to` gives the requester** (nxf 6j6v.7qfm): whoever hands a turn
    // back is in the same position — they have said their piece and until this the surface said
    // nothing at all about what happens next. Filled HERE rather than in `orchestration::reply`
    // because the advice depends on WHO is asking, which is the surface's question; see
    // `ReplyReceipt::await_`.
    //
    // Filled on a `posted: false` skip too. The thread exists either way, and where it stands is
    // exactly as answerable — the caller that ran `--if-unanswered` and found nothing owed still
    // wants to know how the round ends.
    receipt.await_ = Some(Await::for_caller(
        caller_identity(ctx, store)?.persona().is_some(),
        req.thread,
        deadline,
    ));
    Ok(receipt)
}

/// The session this thread is currently with, from `me`'s point of view: the NEWEST message in the
/// thread that somebody else sent and that carries a return address.
///
/// Newest rather than the thread's root, because a conversation moves: the root's address is the
/// opener's, which is the right answer only on the very first turn. Newest-from-somebody-else
/// rather than newest outright, so a persona replying twice in a row does not resume itself.
/// `None` — nobody has answered yet, or the counterpart is a human with no session — is an ordinary
/// state, not a failure: the reply is posted either way.
///
/// Read only off messages an agent action may follow (nxf 6j6v.pzkb): the address names a session
/// on THIS machine that the reply resumes, so a message nobody here can vouch for never chooses it.
fn thread_return_address(store: &ChatStore, thread: &str, me: &str) -> Result<Option<String>> {
    let messages = store.acting_messages_in_thread(thread)?;
    for row in messages.iter().rev() {
        if row.sender == me {
            continue;
        }
        let Some(refs) = &row.refs else { continue };
        let parsed: Refs = match serde_json::from_str(refs) {
            Ok(parsed) => parsed,
            // A message whose refs do not parse is skipped rather than fatal: this is a routing
            // hint, and one unreadable row must not make the whole conversation unanswerable.
            Err(_) => continue,
        };
        if let Some(session) = parsed.session_id {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MessageEnvelope;

    fn post(store: &mut ChatStore, thread: &str, sender: &str, session: Option<&str>) -> String {
        store.set_channel_field("c-1", "kind", "group", "local/human");
        store.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(thread.into()),
            refs: Refs {
                session_id: session.map(str::to_string),
                ..Refs::default()
            },
            body: "hi".into(),
        })
    }

    #[test]
    fn the_thread_return_address_is_the_newest_message_from_somebody_else() {
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "t-1", "local/human", None);
        post(&mut store, "t-1", "local/pm", Some("s-old"));
        post(&mut store, "t-1", "local/pm", Some("s-new"));
        assert_eq!(
            thread_return_address(&store, "t-1", "local/human").unwrap(),
            Some("s-new".into()),
            "a conversation moves — the newest turn is the live session"
        );
    }

    #[test]
    fn my_own_messages_are_never_my_return_address() {
        // Otherwise a persona replying twice in a row resumes itself, which is the shape the depth
        // guard exists to bound in the first place.
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "t-1", "local/pm", Some("s-pm"));
        post(&mut store, "t-1", "local/coder", Some("s-coder"));
        assert_eq!(
            thread_return_address(&store, "t-1", "local/coder").unwrap(),
            Some("s-pm".into())
        );
    }

    #[test]
    fn a_thread_nobody_has_answered_yet_has_no_return_address() {
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "t-1", "local/human", None);
        assert_eq!(
            thread_return_address(&store, "t-1", "local/human").unwrap(),
            None
        );
        assert_eq!(
            thread_return_address(&store, "t-unknown", "local/human").unwrap(),
            None
        );
    }

    /// The address resumes a session on THIS machine, so a message nobody here can vouch for — a
    /// peer's, until its key is trusted — never names it, however new it is (nxf 6j6v.pzkb).
    #[test]
    fn a_message_nobody_here_can_vouch_for_is_never_the_return_address() {
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "t-1", "local/pm", Some("s-pm"));
        let mut peer = ChatStore::open_in_memory(2);
        peer.apply(&store.export());
        post(&mut peer, "t-1", "local/coder", Some("s-local-coder"));
        let theirs: Vec<_> = peer.export().into_iter().filter(|o| o.site == 2).collect();
        store.apply(&theirs);
        assert_eq!(
            thread_return_address(&store, "t-1", "local/human").unwrap(),
            Some("s-pm".into()),
            "the newer message is unvouched, so the vouched one is still the address"
        );
        store.trust_key(peer.key_id(), "peer", "").unwrap();
        assert_eq!(
            thread_return_address(&store, "t-1", "local/human").unwrap(),
            Some("s-local-coder".into())
        );
    }

    #[test]
    fn a_message_with_unreadable_refs_is_skipped_not_fatal() {
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "t-1", "local/pm", Some("s-pm"));
        store
            .connection()
            .execute(
                "UPDATE messages SET refs='{not json' WHERE sender='local/pm'",
                [],
            )
            .unwrap();
        post(&mut store, "t-1", "local/other", Some("s-other"));
        assert_eq!(
            thread_return_address(&store, "t-1", "local/human").unwrap(),
            Some("s-other".into())
        );
    }

    #[test]
    fn the_wake_message_carries_the_guidance_it_was_given_verbatim() {
        // The persona must be able to copy the invocation, not reconstruct it — and since nxf
        // 6j6v.dq59 WHICH invocations those are is the coordinator's own block
        // (`orchestration::reply_guidance`), composed against the addressees that are actually
        // open. What this pins is that the wake carries it through untouched, body first.
        let msg = orchestration::wake_message(
            "review this",
            false,
            "  - `nxc reply --thread t-42 -` — you are finished.",
        );
        assert!(msg.starts_with("review this"));
        assert!(msg.contains("nxc reply --thread t-42"));
    }

    #[test]
    fn a_handed_back_wake_says_so_before_the_answer_itself() {
        // nxf 6j6v.gk9j: the escalation has to be recognisable in the text the model reads FIRST.
        // Asserting `starts_with` rather than `contains` is the whole claim — a notice after the
        // body is a notice the reader meets once it has already read the hand-back as a result.
        let msg = orchestration::wake_message("I cannot do this", true, "reply --thread t-42");
        assert!(
            msg.starts_with("ESCALATION"),
            "the notice must come before the body, got:\n{msg}"
        );
        assert!(
            msg.contains("working copy"),
            "and name the cost while it stands"
        );
        assert!(
            msg.contains("Decide now"),
            "and what is expected of the reader"
        );
        assert!(
            msg.contains("I cannot do this"),
            "without swallowing the answer"
        );
    }
}

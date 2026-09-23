//! THE READ SEAM IS EIGHT VERBS (nexus-flow-6j6v.yr59, the owner correction of 2026-08-21, and
//! nexus-flow-6j6v.k8zq), and this file is the gate that says so.
//!
//! `Engine` carried SIXTEEN read verbs when this item opened — `channels`, `public_channels`,
//! `inbox`, `messages`, `thread`, `search`, `threads`, `thread_quorums`, `thread_board`, `status`,
//! `opener_wake`, `prime`, `prime_as`, `directory`, `transcript`, `transcript_page` — plus
//! `subscribe` beside them. Seven carry the work. Six of them answer a question none of the others
//! answers:
//!
//! | verb | the question |
//! | --- | --- |
//! | `status` | where does this operation stand — the thread TREE, across channel borders |
//! | `directory` | whom and WHAT may I address, and what for — personas AND channels AND front doors |
//! | `prime_as` | session start (it calls `directory` implicitly) |
//! | `thread` | what was SAID |
//! | `transcript_page` | what one session actually DID |
//! | `subscribe` | so a view stays alive |
//!
//! # The seventh, and why it is not in that table
//!
//! `search` is on the seam by an OWNER CORRECTION of 2026-08-21, not by the argument above. The
//! item covers it nowhere — not the keep table, not the "what goes and why" list, not the five
//! verbs its DoD sends to be decided one by one — and it was on the surface, so the build decided
//! it in the item's own terms and removed it as a NAMED LOSS: it answers none of the six questions,
//! being discovery-by-text with the channel-shaped scoping `channels`/`inbox` are leaving for. The
//! gap in the item was reported, not glossed.
//!
//! # The eighth, added deliberately (nxf 6j6v.k8zq)
//!
//! `persona_prime` is the composed session-start TEXT a spawned persona is handed — the sibling
//! modules' blocks its declaration admits, then chat's own. It is here for the second kind of
//! reason, the one `search` established: a measured consumer. app-foundations starts role sessions
//! of its own (`crates/agent-runtime`) and composes their system prompts itself; without this call
//! it would reassemble the block from `prime_as` plus its own idea of the order, the filter and the
//! two sibling modules, and the first change on either side would leave an app's persona and an
//! `nxc` persona reading different instructions. That is the two-contracts split the verb-seam gate
//! (6j6v.vtvs) exists against, one layer up — and the item that added it names the seam as an
//! acceptance point, not as a convenience.
//!
//! It is NOT a second spelling of `prime_as`. `prime_as` answers "what does chat know about this
//! session" as DATA, for a caller that renders it; this answers "what is this persona TOLD",
//! composed across the modules its declaration admits, and there is no way to derive one from the
//! other without duplicating the composition.
//!
//! The owner overruled that on the fact the shape argument cannot see: app-foundations'
//! `ChatClient` (`packages/engine-client/src/chat-client.ts`) consumes `search` TODAY — measured on
//! 2026-08-21, one of the 14 methods of that interface that still exist on this seam, not one of
//! the 14 that no longer do. Removing a live consumed method inside the item whose purpose is to
//! cost that consumer ONE migration instead of two is the break the item exists to prevent. So the
//! count this gate holds is SEVEN, and `search` sits in `SURFACE` below with its own reason —
//! which is deliberately NOT "it answers a seventh question", because it does not.
//!
//! # Why a source-level gate and not a code one
//!
//! Nothing fails to compile when an eighth read grows back. `verb_seam.rs` cannot see it (it
//! asks whether every CLI verb is REACHABLE, so a seam read with no verb is invisible in both
//! directions) and `seam_disposition.rs` cannot see it either (it holds every recorded row true,
//! and there is no row for a method nobody has decided about yet). Both gates are about the
//! relationship between two surfaces; this one is about the SIZE of one of them, which is the thing
//! yr59 actually bought and the thing that erodes without anyone deciding to erode it.
//!
//! So the table below is the whole surface — reads, writes, constructors and the handle's own
//! self-description — and every entry says what it is for, and (since nxf 6j6v.dh41) where what it
//! writes down comes back. Adding a method means adding a row here, which is the moment to ask
//! whether it is a genuinely new question, a second spelling of one the six already answer, or —
//! the shape `search` turned out to have — a live obligation to a consumer that outranks the shape
//! of the surface. And, if it writes, which read gives its fact back.
//!
//! It reads the SOURCE rather than the type, through the same `public_symbols` parser the two
//! neighbouring gates read their seam with, because a `pub fn` is what an embedder can reach and no
//! runtime reflection can enumerate one.
//!
//! # THE OTHER DIRECTION: NOTHING IS RECORDED THAT CANNOT BE READ BACK (nxf 6j6v.dh41)
//!
//! Everything above is one rule and it is about the SIZE of the read surface. It cannot see the
//! defect that filled the proving ground's findings list, because that one is about the surface's
//! SHAPE — seven findings, one shape:
//!
//! | item | the shape |
//! | --- | --- |
//! | `6j6v.h383` | `session bind`/`ended` are WRITTEN, and nothing could ask what became of them |
//! | `6j6v.4d2z` | `mark_read` advances a cursor nothing evaluates any more (closed — see below) |
//! | `6j6v.1vxs` | `awaiting_human` is REPORTED and cannot be discharged (closed — see below) |
//! | `6j6v.0djn` | `queue_position` is reported, and there was no call to answer it with |
//! | `6j6v.qfmz` | a fan-out receipt does not say which member started and which waits |
//! | `6j6v.gk9j` | `escalated: true` stands there and the recipient does not read it (closed) |
//! | `6j6v.v39s` | the tree is shown and its threads are not readable (closed) |
//!
//! The rule they share — the read-side counterpart, and BESIDE the write side's rule rather than
//! under it:
//!
//! > **Every fact this seam RECORDS or REPORTS must be readable back through this seam — and where
//! > it asks a surface to branch, answerable through it.**
//!
//! **The write side has its own rule now, and it is the third section below** (nxf 6j6v.12nn).
//! When dh41 shipped, what stood below it was still the write side's COUNT — four names plus a
//! per-row argument — and this paragraph said so rather than claiming a criterion that was not
//! there. The BUILD SHAPE dh41 borrowed from 12nn's owner decision is the one 12nn then built
//! with: a criterion rather than a number, standing BESIDE its neighbour rather than replacing it.
//!
//! **One of the seven was not a build error at all but the LEFTOVER OF A REMOVAL**, which is why
//! this is a rule and not seven repairs. yr59 cut the read seam to seven verbs and took every read
//! that consumed the unread cursor with it — `Engine::inbox` went, `mark_read` stayed. Nothing
//! failed to compile and nothing failed to run; it was two months before it came up in
//! conversation, and the measurement that followed found five layers of pipe with no water
//! (`6j6v.4d2z`). A removal leaves NO IDENTIFIER TO GREP FOR. What it does leave is a writer whose
//! reader is gone, and that is a thing a gate can see. **It is closed** — 4d2z removed the writer
//! and everything under it on 2026-09-08 — and the closing is the rule's own vindication rather
//! than its retirement: the entry stood here for three days while its removal waited for a
//! replacement to be built, and a reader arriving in that window was told exactly what was owed and
//! by whom.
//!
//! (`6j6v.dh41` counts TWO such leftovers and describes one. The second was not among the seven it
//! lists: `6j6v.px98` was the other residue of the same cut — `Engine::thread` gained the declared
//! `visibility` filter in yr59 and `search` beside it did not — and it is taken up at the end,
//! where it did not become an exception to this rule. **It is closed**, and the paragraph at the
//! end says how.)
//!
//! ## What counts as a FACT — the narrowest version that carries all seven
//!
//! A fact is one of exactly two things:
//!
//! * **RECORDED** — what a call on this handle PERSISTS. Not "what the workspace knows": what THIS
//!   surface writes, through the one write path `engine.rs` has ([`WRITE_PATH`]).
//! * **REPORTED** — a field on a value one of these calls RETURNS ([`REPORTED`]).
//!
//! Wider readings were tried against the seven and dropped. *"A state the guide tells a surface to
//! branch on"* is the phrasing `nxc guide limits-and-safety` invites, and a guide sentence is not a
//! seam — it would make the rule's scope a prose document. *"Anything the store holds"* would put
//! every column of `store.rs` inside a rule about the SEAM. Neither buys anything: the narrow
//! reading carries all seven findings anyway, because every one of them is a thing this handle
//! either wrote or handed out. A rule that covers everything holds nothing.
//!
//! ## How a source gate checks "asks a surface to branch" — it does not
//!
//! No parser can tell that `awaiting_human` asks a human to act while `depth` does not. So the two
//! halves are held in two different ways, and the difference is stated rather than blurred:
//!
//! * The RECORDED half is **mechanical in both of its terms**. WHICH calls record is read out of
//!   `engine.rs` — a call records when its body reaches the store's write path — so the answer
//!   comes from the source, and a row cannot exempt itself by choosing the word `Nothing`. THROUGH
//!   WHAT it comes back is the row's own claim, and the gate holds that claim to the surface: the
//!   named reader has to be on this table and must not itself be a write.
//!
//! **What that mechanical half rests on, and where each rest is held** (PR #427 review). The
//! chain is: every write leaves `engine.rs` through a spelling in [`WRITE_PATH`] → every such
//! spelling funnels into the foundation handle's one mutable lender → nothing writes from a place
//! this table cannot hold a row for. Each link has its own test rather than a sentence:
//! [`the_write_path_has_exactly_the_two_spellings_this_gate_knows`] fails on a new private wrapper,
//! [`the_foundation_handle_lends_a_mutable_store_through_exactly_one_method`] fails on a NEW lender
//! one layer down — the route a gate keyed on wrapper names structurally cannot see — and
//! [`no_write_reaches_the_store_from_a_trait_impl_in_engine_rs`] refuses the one shape that could
//! write on the handle and never be given an honest row, because `SURFACE` is held against
//! `public_symbols`, which does not see trait-impl methods at all. The residual blind spot is a
//! call produced by a MACRO body: `syn` reads the source before expansion, so `engine.rs` growing
//! one would be invisible here. It is named on `method_callers` and named again here rather than
//! discovered later.
//! * The REPORTED half is **a reasoned line per entry** ([`REPORTED`]), exactly as each `Write` row
//!   below carries its own argument. What can be checked is checked — the reporting call and the
//!   answering call both have to exist here, and the answer has to be a WRITE, because a fact you
//!   act on is answered by doing something and never by looking again. What CANNOT be checked is
//!   completeness, and this file does not pretend otherwise: nothing here can tell that a new field
//!   on `StatusThread` asks for a decision. That is a reviewer's sentence, and the table is where
//!   it goes.
//!
//! `6j6v.qfmz` is the shape neither table can hold, and it is named here rather than left to be
//! rediscovered: a fan-out receipt folds N triggers into one and does not say which member started.
//! That is a fact the seam HAS and does not hand out — there is no row to point at, and a gate can
//! only see what somebody wrote down.
//!
//! ## What happens to a breach that already exists
//!
//! It is named, dated, and carries the item that ends it — and the LIST of them is asserted BY NAME
//! ([`THE_RECORDED_AND_UNREADABLE`], [`THE_REPORTED_AND_UNANSWERABLE`]), because a list that grows
//! by one quiet row is the next broken assurance. That assertion has its own counter-proof in both
//! directions ([`an_exception_list_that_drifts_from_the_table_it_guards_is_red`]) — it is the
//! centerpiece claim of this rule, and until PR #427's review it was only ever run against tables
//! that already agreed, where a no-op comparison would have passed just as well. Today it is ONE,
//! in the recorded half; the reported half's list is empty, and the entry that used to stand there
//! is the second bullet:
//!
//! * **`mark_read` recorded the read cursor and nothing on this seam read it back** (`6j6v.4d2z`)
//!   — **closed on 2026-09-08, by removing the write.** It was the finding this gate was built out
//!   of and the last entry on either list; both lists are empty now, and that is a state to be
//!   suspicious of rather than proud of, which is why
//!   [`an_exception_list_that_drifts_from_the_table_it_guards_is_red`] drives both directions off
//!   synthetic tables instead of off the real one.
//!
//!   How it closed is the part worth keeping. The breach was NOT repaired by giving the cursor a
//!   reader: it was measured, and the measurement came out the other way twice. The 2026-08-25
//!   version said the cursor had one consumer and no caller, which made removal look free; the gate
//!   this file's rule demanded was run on 2026-09-05 BEFORE anything fell and found a second, live
//!   reader (`ChatStore::opener_wake`, feeding `prime --json`'s `threads_you_opened`), so the
//!   removal was HELD. What cleared it was building the replacement first — `6j6v.2hx9` derived
//!   that notice from the operation instead — and only then removing the cursor, the ack, the
//!   unread set and the per-channel counts together. "Nothing evaluates it" and "nothing reads it
//!   back" were never the same sentence, and keeping them apart is what made the difference between
//!   a free removal and a broken app seam.
//! * **`awaiting_human` was reported and could not be discharged** (`6j6v.1vxs`) — **closed on
//!   2026-09-06, and closed by deciding it was never that kind of fact.** The row said a human owed
//!   this operation something and had no verb to pay it with, and the search for the missing verb
//!   was the wrong search: `awaiting_human` is derived from two facts that never stop being true —
//!   the root asked, the root was answered — so there is nothing to discharge and nothing a WRITE
//!   could do about it. It only read as a debt because `StatusOperation::live` counted it, which is
//!   what put every finished operation permanently into the default view (thirteen of thirteen,
//!   424 lines, nine with nothing open, measured 2026-09-06). 1vxs took the flag out of `live`
//!   instead of inventing the verb — the verb collides head-on with `6j6v.4d2z`'s open question
//!   about who clears what — and with that the fact stops asking a surface to act and becomes a
//!   LABEL on a finished operation, beside `depth` and `state`. So the row left [`REPORTED`]
//!   rather than changing its `answer`, and the reported half's exception list is empty. What
//!   remains readable back is unchanged: the flag is still on every `status` form that shows the
//!   operation, and `StatusScope::All` is what makes a finished one findable without its id.
//!
//! **`6j6v.px98` was NOT one of them, and that was a decision rather than an oversight** — and the
//! decision is left standing here now that the defect itself is fixed, because it is the reasoning
//! that keeps this rule narrow, not a note about one open item. `search` joined through live
//! MEMBERSHIPS and did not apply the channel's declared `visibility`, which `thread` beside it did
//! — and the owner ruled on 2026-08-23 that `visibility` is an access rule and not a display rule,
//! so it was a real defect with a real item. It was not THIS rule's shape. The fact is declared in
//! the workspace's own folder, `definitions` reads it back and `thread` applies it: recorded,
//! readable, acted on. What was wrong is that a SECOND reader answered the same question
//! differently. Calling that "not readable" would read "readable" as "readable consistently", and
//! then every disagreement between two readers is a breach of this rule — which is the widening the
//! first decision above exists to refuse.
//!
//! **Closed 2026-09-05.** `facade::search` applies the same policy through the same
//! `filter_board_messages` semantics, resolved from the same catalogue `Engine::thread` resolves
//! from, and `crates/chat/tests/channel_visibility.rs` holds the two readers to the one answer. The
//! rule above is unchanged by that: it never carried this finding and does not gain it now.
//!
//! # THE WRITE SIDE: NOTHING STARTS THAT WAS NOT ALREADY COMMISSIONED (nxf 6j6v.12nn)
//!
//! The two rules above are the READ seam's — how BIG it is, and that nothing it writes down is
//! lost. This third one is the WRITE seam's, and it stands beside them for the reason the second
//! stands beside the first: it answers a question neither of them asks, and folding it into either
//! would make both vaguer.
//!
//! `6j6v.ckeq` cut the writing seam to two verbs on the owner's sentence *"`send_to` und
//! `reply_thread` sollen der einzige Weg der KOMMUNIKATION sein"*. Two more writers arrived after
//! it — `withdraw` (`6j6v.0djn`) and `release_working_tree` (`6j6v.fabb`) — and BOTH were admitted
//! on the same sentence: they do not speak, they make the surface do LESS. That sentence did not
//! survive being measured. **`release_working_tree` STARTED a session.** It handed this device's
//! working copy on, and `orchestration::release_and_fire` took the next commission out of the
//! queue and triggered it; `nxc release` even named whom it started. The row carried "wakes nobody"
//! as its argument and a parenthesis as the truth, and the source said otherwise the whole time.
//!
//! A number over the whole write seam cannot see that, and neither can the word a row picks for
//! itself. So the rule the owner decided on 2026-08-25 is a criterion:
//!
//! > **A call may reach the write seam when it cannot cause work that was not already
//! > commissioned.**
//!
//! ## What is mechanical here, and what is a sentence
//!
//! The criterion has two halves and a parser can decide only the first — the same honest split the
//! RECORDED and REPORTED halves above are held with, and it is stated for the same reason.
//!
//! * **Can it start work at all?** — read out of the SOURCE. A call starts a session when its body
//!   reaches [`SPAWN_FUNNEL`] through however many frames of compute layer, which is a question
//!   about a PATH rather than about a call: all four writes are one-line forwardings onto a compute
//!   layer thousands of lines deep. [`callers_reaching`] over the call graph of `chat`'s whole
//!   `src/` answers it, and
//!   [`the_compute_layer_starts_a_session_through_exactly_one_funnel`] is what keeps that one name
//!   the whole list. So [`Causes::Nothing`] is a claim the source can contradict — and
//!   `release_working_tree`'s old wording WAS RED under this gate rather than merely wrong, which
//!   is exactly the row nxf 6j6v.b9nf removed rather than left standing to fail it forever.
//! * **Was the work it starts already commissioned?** — a reasoned line, with the one
//!   cross-reference a gate can hold: [`Causes::OnlyWhatWasCommissioned`] must NAME the call that
//!   commissioned it, and that call must be on this table and be one of the two ways to speak. What
//!   cannot be checked is whether the naming is TRUE — no parser tells a commission from a
//!   coincidence — so that is a reviewer's sentence, and the row is where it goes.
//!
//! ## Why the category is not a bucket
//!
//! Three things close it, and none of them is a number over the write seam as a whole.
//!
//! 1. **SPEAKING is still two and still closed** ([`THE_TWO_WAYS_TO_SPEAK`]), and it is a count
//!    because the criterion cannot do that job. `send_to` commissions, so the criterion excludes
//!    it; `reply_thread` continues something commissioned, so the criterion would ADMIT it, the
//!    way it used to admit `release_working_tree`. What makes both of them ckeq's named exception is
//!    whose words they carry, which is not a thing this criterion measures — so a fifth writer
//!    that speaks declares itself [`Causes::Speaks`] and the count refuses it. That is ckeq,
//!    unchanged and unwidened, and it is why this rule has two mechanisms rather than one.
//! 2. **Every admission carries its own argument**, and the LIST of them is asserted by name
//!    ([`THE_WRITES_ADMITTED_BY_THE_CRITERION`]) — not as the category's gate, which is the
//!    criterion, but as its RECEIPT, for the reason the two exception lists above are asserted: an
//!    admission that joins in silence is the next broken assurance. Both entries on it are owner
//!    decisions with an item behind them.
//! 3. **`6j6v.dvyq` §5's closed list is explained rather than reopened** ([`THE_CLOSED_LIST`]), and
//!    where §5's entrance still has a compute half in this crate, whether it starts a session is
//!    MEASURED against the same funnel instead of asserted. One entry is refused by ckeq's count
//!    and not by this criterion, and it says so: a rule that claimed to refuse everything on that
//!    list would be claiming more than it holds.
//!
//! ## What the measurement says about the whole handle
//!
//! Applied to every row rather than to the four writes, it finds calls that can start a session
//! outside `Kind::Write` too — `session_ended` among them, a `Handle` row. A host announcing that
//! one session's process is over opens the next step of the round that session stood in, and a
//! rule that had looked only at `Kind::Write` would never have asked. Two of the rows speak; the
//! rest that start anything start nothing that was not already commissioned, and each names the
//! call that commissioned it.
//!
//! That is what the owner's assurance to the app user rests on, and it is derived now rather than
//! believed: *"the app can start nothing I did not commission."*
//!
//! **What that sentence is scoped to, said here rather than left to be assumed** (PR #430 review,
//! Integrity #3): the calls on `Engine`, measured through `chat`'s own sources. It is not a claim
//! about everything the crate exposes. A host that holds an `Arc<dyn Worker>` can start whatever it
//! likes — `Worker` is a trusted-collaborator contract, not a seam this rule guards, and the trust
//! model says so where the trait is declared. The assurance is about the SURFACE an app is given,
//! which is what the owner's sentence is about too.

use nxs_test_support::verb_seam::{
    call_graph, callers_reaching, crate_relative, method_callers, public_fns_lending_mut,
    public_symbols, MethodCaller,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// What a method on the handle IS. The split is the item's own: a READ VERB asks the workspace
/// about its messages, and everything else on the handle either writes, opens the handle, or lets
/// the handle describe itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// One of the eight. The count is asserted, not merely documented.
    Read,
    /// A writing verb. Two of them SPEAK and that count is closed (6j6v.ckeq); the rest are on
    /// the seam by a criterion rather than by a number (6j6v.12nn) — see [`Causes`].
    Write,
    /// A constructor.
    Open,
    /// The handle answering about ITSELF rather than about the workspace's messages: its
    /// workspace, its origin, its declarations, its session bindings, its retention.
    Handle,
}

/// How the fact a call RECORDS comes back off this seam (nxf 6j6v.dh41) — the read-side rule's
/// checkable half.
///
/// **Which calls record is not this word's decision.** It is read out of `engine.rs`: a call
/// records when its body reaches the store's write path ([`WRITE_PATH`]). So a row cannot make
/// itself exempt by choosing [`Recorded::Nothing`] — the source contradicts it and the gate says
/// which of the two is lying.
#[derive(Debug, Clone)]
enum Recorded {
    /// Records no fact of its own: a constructor, one of the eight reads, or the handle answering
    /// about itself without writing anything down.
    Nothing,
    /// It records something, and that something is readable back through these members of
    /// `SURFACE` — each of which the gate checks is on this table and is not itself a write.
    ReadBy {
        /// WHAT it records, in one line: the thing that has to come back.
        fact: &'static str,
        /// The members of `SURFACE` it comes back through.
        by: &'static [&'static str],
    },
    /// It records a fact and NOTHING on this seam gives it back. A dated breach carrying the item
    /// that ends it; the set of these is asserted by name in [`THE_RECORDED_AND_UNREADABLE`], so it
    /// cannot grow by one more quiet row.
    Unread {
        /// WHAT it records that cannot be read back.
        fact: &'static str,
        /// The board item that closes the breach.
        item: &'static str,
        /// Why it still stands. An exception without a reason is a finding, not a free pass.
        why: &'static str,
    },
}

/// What a call on this handle can CAUSE — the write-side rule's checkable half (nxf 6j6v.12nn).
///
/// **Whether a call can start a session is not this word's decision.** It is read out of the
/// crate's own sources: a call starts when its body reaches [`SPAWN_FUNNEL`], through however many
/// frames of compute layer. So a row cannot make itself harmless by choosing [`Causes::Nothing`] —
/// the source contradicts it and the gate says which of the two is lying.
///
/// That is why this is a field and not a sentence in `why`. `release_working_tree` argued for a
/// fortnight that it "makes the surface do LESS" and "wakes nobody"; it starts a session, it always
/// did, and no reader of that row would have guessed.
#[derive(Debug, Clone)]
enum Causes {
    /// Starts nothing at all — every read, every constructor, and the one write that only takes
    /// something back.
    Nothing,
    /// It SPEAKS: the caller's own words, put where somebody has to answer them, and a session
    /// started or woken to answer them.
    ///
    /// **What puts a call in this variant is WHOSE words it carries, not what it starts**, and the
    /// two members show why that has to be the test. `send_to` commissions with those words, so the
    /// criterion excludes it outright. `reply_thread` continues what was already commissioned, so
    /// on the criterion alone it would pass exactly as `release_working_tree` used to — and it is
    /// counted anyway, because a way for an app to put its own words in front of somebody is what
    /// 6j6v.ckeq closed at two ([`THE_TWO_WAYS_TO_SPEAK`]). A rule with only the criterion would
    /// have no reason to stop at a third.
    Speaks {
        /// What it sets going, in one line.
        work: &'static str,
    },
    /// It starts a session, and the work that session does was commissioned through one of
    /// [`THE_TWO_WAYS_TO_SPEAK`] before this call was ever made.
    ///
    /// The criterion's whole content, and the reason it is a criterion rather than a number. The
    /// gate holds the cross-reference — the named call has to be on this table and has to be one
    /// that speaks — and cannot hold the claim itself, which is a reviewer's sentence.
    OnlyWhatWasCommissioned {
        /// WHAT it starts. Said plainly: a row that starts something and reads as though it did
        /// not is the defect this variant exists against.
        starts: &'static str,
        /// The member of `SURFACE` that commissioned that work, so "already commissioned" names a
        /// call rather than a mood.
        commissioned_by: &'static str,
    },
}

#[derive(Debug, Clone)]
struct Member {
    name: &'static str,
    kind: Kind,
    /// What it is for — and, for a `Handle` entry, why it is not an eighth read verb.
    why: &'static str,
    /// Where what it writes comes back off the seam (nxf 6j6v.dh41). `Nothing` for everything that
    /// writes nothing, which the SOURCE — not this field — decides.
    records: Recorded,
    /// What work it can set going (nxf 6j6v.12nn). `Nothing` for everything that starts nothing,
    /// which the SOURCE — not this field — decides, exactly as for [`Member::records`].
    causes: Causes,
}

/// The two spellings a write reaches the store through in `engine.rs`, and the whole mechanical
/// half of the read-side rule rests on this being the whole list.
///
/// `with_state_mut` is the foundation handle's write lock, taken directly. `with_orchestration` is
/// `Engine`'s own private funnel, which takes that same lock one frame further down — five of the
/// nine recording calls are spelled that way, so a gate that knew only the first would call more
/// than half of them plain reads. That is not a hypothetical failure mode: the guard built in nxf
/// 6j6v.pkw9 saw one spelling of the thing it forbade and missed four others, which is how the item
/// before this one found five start paths without a receipt where it had claimed one.
///
/// A third funnel cannot appear unnoticed —
/// [`the_write_path_has_exactly_the_two_spellings_this_gate_knows`] fails on any private helper in
/// `engine.rs` that reaches the write lock and is not named in [`WRITE_PATH_FUNNELS`].
const WRITE_PATH: &[&str] = &["with_state_mut", "with_orchestration"];

/// The entries of [`WRITE_PATH`] that are themselves only a spelling: private helpers inside
/// `engine.rs` that write by taking the lock for somebody else, rather than entrances an embedder
/// can reach.
const WRITE_PATH_FUNNELS: &[&str] = &["with_orchestration"];

/// The one funnel a session is ever STARTED through on this crate's compute layer, and the whole
/// mechanical half of the write-side rule rests on this being the whole list (nxf 6j6v.12nn).
///
/// `orchestration::trigger_and_bind` is where a `TriggerRequest` meets the worker, and its own doc
/// already claims the position this gate needs:
/// *"the one place a `TriggerOutcome` is acted on, so both spawn paths — every declared role via
/// `trigger_role`, and the ephemeral synthesizer that deliberately bypasses it — treat a
/// worker-supplied session id identically."* A claim in a doc comment is not a gate, which is why
/// [`the_compute_layer_starts_a_session_through_exactly_one_funnel`] holds it at the root instead.
///
/// It is named rather than `Worker::trigger` itself for a reason the read side's `WRITE_PATH` has
/// too: `trigger` is what the two worker DELEGATORS call as well, and a delegator forwarding to
/// another worker is not a spawn path — it is already past the funnel.
const SPAWN_FUNNEL: &[&str] = &["trigger_and_bind"];

/// The recording calls whose fact NOTHING on this seam gives back — asserted by name, because an
/// exception list that grows in silence is the next broken assurance (nxf 6j6v.dh41).
///
/// **Empty since 2026-09-08**, when `6j6v.4d2z` closed the last entry (`mark_read`) by removing the
/// write rather than by finding it a reader — the module doc carries how, and why that route took
/// three days and a held removal. An empty list is the state this rule is FOR; it is also the state
/// in which a comparison that has quietly stopped comparing looks identical to a healthy one, which
/// is why [`an_exception_list_that_drifts_from_the_table_it_guards_is_red`] drives both directions
/// off synthetic tables and a synthetic list.
const THE_RECORDED_AND_UNREADABLE: &[&str] = &[];

use Kind::{Handle, Open, Read, Write};

const SURFACE: &[Member] = &[
    // ---- the eight read verbs -----------------------------------------------------------------
    Member {
        name: "subscribe",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "So a view stays alive: a coalesced `Change` whenever ANOTHER writer commits, which \
              is the app's cue to re-read. Domain-blind by design — it says that something moved, \
              never what.",
    },
    Member {
        name: "status",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "Where the operation stands — the thread TREE across channel borders, with each \
              thread's state, deadline, assignee session, that session's own state and \
              working-tree hold derived from the log. Since yr59 it also takes an EXPLICIT SET of \
              threads (`StatusScope::Threads`), which is what `thread_quorums` was, and it \
              absorbed `threads`/`thread_board`. The two LISTING forms show only what is still \
              going on (nxf 6j6v.1vxs); `StatusScope::All` is the same selection unfiltered.",
    },
    Member {
        name: "directory",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "Whom and WHAT may I address, and what for: the declared personas, the declared \
              channels with their purpose, and — since yr59 absorbed `public_channels` — the \
              workspace's public front doors, including ones that arrived by sync and carry no \
              declaration here.",
    },
    Member {
        name: "prime_as",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "Session start. It is `prime` with the persona named — `prime` WAS `prime_as(c, \
              None, now)` and went as the pure overload it was — and it calls `directory` \
              implicitly, so a starting session is told its own address book without a second \
              call.",
    },
    Member {
        name: "thread",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "What was SAID. The ONE message reader (yr59): a thread id is the value `send_to` \
              hands back and `reply_thread` consumes, so the reader is keyed the same way the \
              rest of the surface is. It applies the channel's declared `visibility`, which is \
              the policy `thread_board` used to carry.",
    },
    Member {
        name: "transcript_page",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "What one session actually DID, in a window the caller asked for. `transcript` was \
              `transcript_page(s, -1, None)` by its own doc and went as the overload it was.",
    },
    Member {
        name: "search",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "THE SEVENTH, and the only entry here that is NOT justified by a question of its own \
              — it answers none of the six, and the build removed it for exactly that reason. It \
              is on the seam by the OWNER CORRECTION of 2026-08-21: app-foundations' `ChatClient` \
              consumes `search` today, one of the 14 methods of that interface still alive on this \
              seam, and cutting a live consumed method inside the item whose purpose is to cost \
              that consumer ONE migration instead of two is the break the item exists to prevent. \
              A measured obligation to a consumer outranks the shape of the surface. It was scoped \
              by MEMBERSHIP only and applied no declared `visibility`, unlike `thread` beside it — \
              the one residual, named on the method itself, and CLOSED by nxf 6j6v.px98: it now \
              applies the same policy `thread` does, from the same catalogue, so the seam's two \
              message readers give one answer to \"may I see this?\".",
    },
    Member {
        name: "persona_prime",
        kind: Read,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "THE EIGHTH, and the second entry justified by a measured consumer rather than by a \
              question of its own — the reason `search` established. It is the composed \
              `nxs prime --persona <handle>` TEXT a spawned persona is handed: the sibling \
              modules' blocks its declaration admits, then chat's own. \
              \
              WHY IT IS NOT A SECOND SPELLING OF `prime_as`: that answers what chat knows about a \
              session, as DATA, for a caller that renders it. This answers what a persona is TOLD, \
              composed across the modules its declaration admits, and neither is derivable from \
              the other without duplicating the composition. \
              \
              WHY IT HAD TO EXIST (nxf 6j6v.k8zq): app-foundations starts role sessions of its own \
              (`crates/agent-runtime`) and composes their system prompts itself. Reassembling this \
              app-side means an app's persona and an `nxc` persona drift apart the first time \
              either the order, the filter or a module's block changes — the two-contracts split \
              the verb-seam gate (6j6v.vtvs) exists against, one layer up. The item names the seam \
              as an acceptance point.",
    },
    // ---- the writing verbs: two ways to speak (nxf 6j6v.ckeq) + one to take it back (6j6v.0djn) -
    Member {
        name: "send_to",
        kind: Write,
        records: Recorded::ReadBy {
            fact: "the message, the thread it opened, WHERE THE WORKING COPY STOOD when the turn \
                   changed hands (nxf 6j6v.2af2) and — on the persona path — the session it minted \
                   and the place it took in the working-tree queue",
            by: &["thread", "status", "session_state"],
        },
        causes: Causes::Speaks {
            work: "the persona's own session, started to answer what the caller just said — or, \
                   into a declared channel, one per member of the round it opens",
        },
        why: "THE FIRST WAY TO SPEAK. Open a conversation with a declared target and get its \
              thread back — the one call an app needs to start anything, and the call that \
              COMMISSIONS: it puts the caller's own words where somebody has to answer them and \
              starts the session that will. Everything else on this handle that starts anything \
              starts something this call commissioned. \
              \
              IT IS WHAT THE CRITERION EXCLUDES, not something admitted by it (nxf 6j6v.12nn): a \
              call that can cause work nobody commissioned is exactly what a write seam is \
              supposed to refuse. It is here as 6j6v.ckeq's named exception to that, and the \
              exception is closed at two. This is the ONE row on the handle of which that is \
              literally true — `reply_thread` beside it would pass the criterion and is counted \
              for a different reason, which its own row carries.",
    },
    Member {
        name: "reply_thread",
        kind: Write,
        records: Recorded::ReadBy {
            fact: "the message, the turn handed back through the thread's return address, WHERE \
                   THE WORKING COPY STOOD when it changed hands (nxf 6j6v.2af2) and — when the \
                   caller on the other side turns out to be mid-turn — the HELD delivery that will \
                   reach it when its session settles (nxf 6j6v.gn8b)",
            by: &["thread", "status", "held_deliveries"],
        },
        causes: Causes::Speaks {
            work: "the turn handed back: the session on the other side of the thread, woken to \
                   read what the caller just said",
        },
        why: "THE SECOND WAY TO SPEAK, and the last. Answer in a thread, handing the turn back \
              through its return address — which wakes the session on the other side. \
              \
              WHY IT IS COUNTED RATHER THAN ADMITTED BY THE CRITERION (nxf 6j6v.12nn): what it \
              starts is a turn inside a commission that already exists, so on the criterion alone \
              it would pass the way `release_working_tree` used to. What puts it in the count instead \
              is whose words go with it — the CALLER's, to somebody who must answer them. That is \
              what 6j6v.ckeq closed at two, and the distinction is the reason this rule has a \
              count and a criterion rather than one of them.",
    },
    Member {
        name: "name_thread",
        kind: Write,
        records: Recorded::ReadBy {
            fact: "what the conversation is CALLED — the thread's display name (nxf 6j6v.e76c), \
                   written once at its opening and never moved afterwards",
            by: &["status"],
        },
        causes: Causes::Nothing,
        why: "THE FOURTH WRITE, and it is here BY THE CRITERION like the two below it rather than \
              by widening ckeq's count. It gives a thread the short display name an app shows in \
              place of `dm:<24 hex>` — a `dm:` id is a `sha256` over the two sorted handles, so a \
              direct conversation has by construction no name, and a live app screen showed a user \
              nothing else. \
              \
              AGAINST THE CRITERION — can it cause work that was not already commissioned? NONE, \
              and the source is what says so: no path out of it reaches the spawn funnel. It \
              writes ONE LWW register and posts nothing at all, so it is not a third way to speak \
              either: no words of the caller's reach anybody who has to answer them. \
              \
              WHY IT IS ON THE HANDLE and not left to `nxc`: an app is the consumer this exists \
              for. `EngineConfig::namer` defaults to naming nothing — the shipped derivation \
              starts a detached process that runs a MODEL, and a host that opened a workspace to \
              read and post must not begin paying for that — so a host with a model of its own \
              derives the name inside its own budget and writes it here, and one with none lets \
              its user type it. \
              \
              ONCE, and the receipt says which happened: a thread that already carries a name \
              keeps it and comes back `named: false`. That is a no-op rather than an error \
              because the other caller is a commissioned run that can be retried — and it is what \
              makes the name safe for a surface to render, since a name that moves under a reader \
              is worse than none.",
    },
    Member {
        name: "withdraw",
        kind: Write,
        records: Recorded::ReadBy {
            fact: "that the commission is taken back — the thread leaves the queue, or its session \
                   is stopped, and it stops being expected of anybody; and, for a running round \
                   that held the working copy, that its work is to be parked",
            by: &["status"],
        },
        causes: Causes::Nothing,
        why: "THE THIRD WRITE, added by nxf 6j6v.0djn, and since nxf 6j6v.12nn it is here BY THE \
              CRITERION rather than by widening a count. It takes back a commission that is still \
              PARKED behind the working-copy lease — no session has started, no transcript exists, \
              no model call was made — and, since nxf 6j6v.b9nf, a round that is RUNNING: the \
              thread is discharged, the session is asked to STOP, and when the round holds the \
              working copy a marker is written for the background tick to park its work once the \
              process is gone. \
              \
              AND `status` GENUINELY READS THIS, since fix round 3 of nxf 6j6v.b9nf's own review \
              (Integrity #2) — before that fix the marker this row is about stood behind \
              `schema.rs`'s own \"nothing in `nxc status` reads this row today\", so what this row \
              claimed was written but not yet surfaced anywhere. `StatusOperation::withdrawn` is \
              the read this row names now: who withdrew the round, when, and which sessions still \
              pin the claim; the human line says in as many words that the work is \"waiting to \
              park\". \
              \
              AGAINST THE CRITERION — can it cause work that was not already commissioned? It \
              causes NONE, and that is the source's answer rather than this row's: no path out of \
              it reaches the spawn funnel, which is the one place a session is ever started — \
              measured again after the running case landed, and the measurement is what found the \
              two edges the by-name join had invented (see [`chat_sources`]; neither the \
              discharge of the chain a withdrawal interrupts, nor the park preconditions, nor the \
              service finding reaches a spawn path). The \
              running case makes the point sharper, not weaker: the one process-level act it \
              performs is a STOP, the opposite of a start, and the park that follows is the \
              tick's, not this call's — the hand-off it causes there starts only what was already \
              queued behind the copy, exactly as any release does. \
              \
              WHAT IT DOES WRITE, said here because \"it only takes something away\" is the loose \
              kind of argument this rule replaced: it removes the queue entry, retargets the \
              thread's expectation onto the withdrawn identity, POSTS an Info message in that \
              identity's name saying what happened (and, for a stopped session, carrying that \
              session as the message's return address so the next reply into the thread resumes \
              it onto its park branch), does the same for every thread above it that was still \
              waiting on what it took back (nxf 6j6v.s2cj — a waiting persona thread carries the \
              session that stood in it, for the same reason), and records the withdrawn-holder \
              marker and the sessions it stopped. Words — but not \
              the caller's, and to nobody who has to answer them. What ckeq closed is the caller's \
              own words reaching somebody who must, and there are two of those. \
              \
              WHY IT HAD TO EXIST: `SendToReceipt` already hands an app `queue_position` / \
              `queued_behind` — a state `nxc guide limits-and-safety` tells a surface to branch on \
              — and until 0djn there was no call to answer it with. `verb_seam.rs` waived the verb \
              `CliOnly` on the premise that an embedding app has no such queue; that premise was \
              MEASURED and is false (`embed_surface.rs::engine_send_to_a_busy_exclusive_persona_…` \
              reaches `queue_position: Some(1)` through `Engine::send_to` alone, because \
              `working_tree: exclusive` is a declaration and the lease gate sits in the one trigger \
              funnel both surfaces pass through). A reported state that cannot be acted on is the \
              class this house closes.",
    },
    // `release_working_tree` was here — THE FOURTH WRITE, added by nxf 6j6v.fabb and admitted by
    // the criterion because the session it started was already commissioned by a `send_to` long
    // parked behind this device's lease. REMOVED by nxf 6j6v.b9nf: `release` left the user surface
    // and the seam alike, so there is no row to hold true or false any more — see
    // `tests/seam_disposition.rs`'s row for the decision and removal dates.
    // ---- constructors -------------------------------------------------------------------------
    Member {
        name: "open",
        kind: Open,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "Open the workspace and hold it. `EngineConfig::default()`.",
    },
    Member {
        name: "open_with",
        kind: Open,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "The same, with an explicit `EngineConfig`. There is no THIRD constructor since \
              yr59: `open_with_poll_interval` existed for ONE value and that value is \
              `EngineConfig::poll_interval` now.",
    },
    // ---- the handle describing itself ---------------------------------------------------------
    Member {
        name: "workspace",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "The resolved workspace (db path, replica identity). About the HANDLE, not about \
              what was said in it.",
    },
    Member {
        name: "origin",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "The minting workspace identity every write through this handle is stamped with (nxf \
              6j6v.07me). It exists so a caller that no longer SUPPLIES the origin can still name \
              what it wrote; it reads no messages.",
    },
    Member {
        name: "definitions",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "The declaration catalogue, read fresh from `.nxs-personas/`. Named in \
              `seam_disposition.rs` by app-foundations' own request. It is the raw catalogue; \
              `directory` is the QUESTION asked of it, which is why the read verb is the other \
              one.",
    },
    Member {
        name: "bind_runtime_session",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "which runtime session the internal one a trigger minted became",
            by: &["session_state"],
        },
        causes: Causes::Nothing,
        why: "Bind a runtime's own session id to the internal one a trigger minted — the \
              out-of-band half of the worker seam (nxf 6j6v.5x9j). A write about the handle's own \
              machinery.",
    },
    Member {
        name: "session_ended",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "that the process behind a session is over — the fact an ordered channel's \
                   advance rests on since nxf 6j6v.10yb",
            by: &["session_state"],
        },
        causes: Causes::OnlyWhatWasCommissioned {
            starts: "the next step of the round this session stood in: announcing the end opens \
                     the advance gate a declared channel was waiting on, and the step it opens is \
                     a session started inside this call",
            commissioned_by: "send_to",
        },
        why: "A session reports that its OWN process is over (nxf 6j6v.10yb) — \
              `bind_runtime_session`'s sibling and the other half of the same machinery: one says \
              which runtime session a trigger became, this one says it is finished. A WRITE, and it \
              is on the handle for its sibling's reason — a local sidecar can shell out to `nxc`, a \
              runtime that owns its sessions elsewhere cannot reach anything but its host. Without \
              it such a host's ordered channels fall back to process liveness (which its worker may \
              not be able to answer) and then to the channel's declared `timeout:`. \
              \
              AND IT STARTS A SESSION — written down since nxf 6j6v.12nn rather than left implied, \
              because the measurement went looking at the whole handle and not only at `Kind: \
              Write`. Announcing the end opens the advance gate an ordered channel was held on, so \
              the next step of that round begins inside this call. What it starts is a step of a \
              round `send_to` commissioned, which is why it passes the criterion; that a `Handle` \
              row starts sessions at all is the sort of thing a rule scoped to the write seam would \
              never have asked about.",
    },
    Member {
        name: "session_interrupted",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "that a session stopped because the model was not available to it, which \
                   boundary it hit, and when that boundary falls",
            by: &["session_state", "status"],
        },
        causes: Causes::Nothing,
        why: "A session reports that it ran into an AVAILABILITY BOUNDARY (nxf 6j6v.npy3) — the \
              runtime's third callback, beside `bind_runtime_session` and `session_ended`, and on \
              the handle for their reason: a local sidecar can shell out to `nxc`, a runtime that \
              owns its sessions elsewhere cannot reach anything but its host. \
              \
              It says something the engine CANNOT work out for itself, which is why it is a \
              callback and not a derivation: only the runtime knows that a provider refused this \
              session as opposed to crashing under it, and only the runtime is told when the \
              window lifts. Before it, the classification knew two classes — log in, or something \
              is broken — so an exhausted quota was indistinguishable from a crash and the teardown \
              posted 'I cannot carry this out' in the agent's name over a round that was fine. \
              \
              IT STARTS NOTHING. It arms a deadline (the way back), which is a different thing from \
              starting a session and is why this row is `Causes::Nothing` while its two neighbours \
              are not: what runs at that deadline is `resume_interrupted`, in its own call.",
    },
    Member {
        name: "resume_interrupted",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "that a hold has been taken up — the interruption `session_interrupted` recorded \
                   is stamped, and stops standing over the operation",
            by: &["session_state", "status"],
        },
        causes: Causes::OnlyWhatWasCommissioned {
            starts: "the interrupted session itself, continued with its own runtime conversation \
                     in front of it — never a fresh one, and never a replay of what it already did",
            commissioned_by: "send_to",
        },
        why: "THE WAY BACK from an availability boundary (nxf 6j6v.npy3) — run by a human who is \
              not waiting for the window, and by the background service when it falls. A WRITE in \
              `session_ended`'s sense: it moves the handle's own machinery and starts a session \
              doing it. \
              \
              WHAT IT STARTS is the very session that stopped, resumed on the runtime session it \
              was already bound to, so what it begins is a continuation of a round `send_to` \
              commissioned rather than anything new — which is what passes the criterion here. \
              \
              MOSTLY IT STARTS NOTHING, and that is the point rather than a caveat: the round may \
              already be answered, a process may already be alive, the window may not have lifted, \
              the working copy may have moved, or the runtime may not continue conversations it \
              began. Each of those is a named outcome on the receipt, and each is a decision NOT to \
              spend a turn. \
              \
              WHY IT IS ON THE HANDLE AT ALL: `deliver_held`'s line, verbatim. On the command line \
              the background service arms it and runs `nxc resume`; a host that drives its own \
              runtime has no such arming to ride on.",
    },
    Member {
        name: "deliver_held",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "that a caller's held answers have been handed over — the queue `reply_thread` \
                   filled is emptied, and only once the resume was actually accepted",
            by: &["held_deliveries"],
        },
        causes: Causes::OnlyWhatWasCommissioned {
            starts: "the caller's own session, resumed with the answers to the commissions IT \
                     opened — one wake for all of them, in the shape a channel round uses",
            commissioned_by: "send_to",
        },
        why: "Hand a settled caller everything that arrived while it was working (nxf 6j6v.gn8b). \
              A WRITE in the same sense `session_ended` is one — it moves the handle's own \
              machinery and starts a session doing it — and it is here rather than among the write \
              verbs for the same reason: the words it carries are not the caller's own, they are \
              answers to commissions the caller already made. \
              \
              WHY IT IS ON THE HANDLE AT ALL. On the command line the background service arms it: \
              `session ended` writes a deadline into the workspace's book and the service runs \
              `nxc session deliver` once the grace has passed, because the announcing process is \
              still alive while it speaks and a resume from inside it is refused by the \
              single-process guard. A host that runs its own sessions has no such arming to ride \
              on — it knows when one settles and needs to say so.",
    },
    Member {
        name: "pick_up",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "that a chat designated to this machine was taken up — the persona session \
                   started for it, or resumed, standing on the chat's thread",
            by: &["session_state", "status"],
        },
        causes: Causes::OnlyWhatWasCommissioned {
            starts: "the persona of a chat that `send_to` opened FOR this machine, or a turn \
                     `reply_thread` wrote into it from another machine — never anything of its \
                     own, and only for messages whose op may carry an action here",
            commissioned_by: "send_to",
        },
        why: "THE EXECUTING MACHINE'S HAND (nxf 6j6v.1c6k). A chat runs on one machine; every \
              other machine posts and starts nothing, and this is what the designated one runs \
              after its sync pulled the order in. A WRITE in `resume_interrupted`'s sense: it \
              moves the handle's own machinery and starts a session doing it, and what it starts \
              was commissioned by a person on another device — which is what passes the criterion. \
              \
              WHY IT IS ON THE HANDLE AT ALL: `deliver_held`'s line again. On the command line the \
              background service runs `nxc pick-up` after a pass; a host that runs persona \
              sessions itself and syncs on its own has no such pass to ride on.",
    },
    Member {
        name: "machine",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "THE QUESTION BEFORE A CHAT IS SENT, as data (nxf 6j6v.1c6k): which machine a chat \
              with a persona would run on, or an existing chat runs on, where that came from, \
              whether it is online, and the machines to offer. It is a READ, and not a ninth read \
              verb for the reason every handle entry is not: it is about the handle's host — which \
              machine this is and who else is online, answered by `EngineConfig::machines` — not \
              about the workspace's record. `send_to` and `reply_thread` refuse with exactly this \
              question when it has to be asked, and an app renders the choice from here first.",
    },
    Member {
        name: "held_deliveries",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "THE READER for the queue the two calls above move (nxf 6j6v.gn8b). `reply_thread` \
              fills it whenever the caller it answers is mid-turn, `deliver_held` empties it, and \
              until this an app could do both and never ask what was in it — a fact recorded on \
              this seam and unreadable through it, which is what this whole record exists to \
              refuse. It refused this one: the gate went red on `deliver_held` before this read \
              existed. \
              \
              WHY IT IS NOT A NINTH READ VERB: `session_state`'s line, verbatim. It asks about the \
              handle's own delivery machinery, not about what was said in the workspace, and none \
              of the six questions is 'what has not been handed over yet'. What was SAID is in \
              `thread` and `status` either way — the held answers are ordinary messages in their \
              own threads, durable from the moment they were posted. This says only who has not \
              been told.",
    },
    Member {
        name: "session_state",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "THE READER for the two writes above (nxf 6j6v.h383) — same rows, same seam. \
              `bind_runtime_session` records which runtime session a trigger became and \
              `session_ended` records that its process is over, and until this nothing could ask \
              for either of them back. It answers `running` / `ended <when>` / `unknown`, scoped to \
              one session or to every session recorded under a thread. \
              \
              WHY IT IS NOT AN EIGHTH READ VERB: it asks about the handle's own session machinery, \
              not about what was said in the workspace — the same line that puts its two siblings \
              here. None of the six questions is 'what became of the process behind this thread', \
              and `status` deliberately does not grow one: that question needs a \
              `Worker`, and the eight reads take a store and a clock. \
              \
              WHY IT HAD TO EXIST, measured: over six hours of continuous operation in a foreign \
              project the coordinator answered `is this thread thinking or is it dead?` with \
              `ps -p $(cat .nxs/agent-logs/<id>.pid)` — reaching past this seam into `nxc`'s own \
              directory for a file whose format is promised nowhere. A fact this house writes, \
              branches a channel's advance on, and offers no way to read is the class \
              nxf 6j6v.dh41 names.",
    },
    Member {
        name: "worker_answers_liveness",
        kind: Handle,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "Whether this handle's WORKER can answer the liveness question at all (nxf \
              6j6v.t41e) — `session_state`'s companion, and about the handle's machinery in the \
              most literal sense: it reads no rows, takes no scope and asks nothing of the store. \
              \
              WHY IT IS NOT A FIELD OF `session_state`'s REPORT AND NOTHING MORE: it is also \
              there, because `unknown` is where the ambiguity is felt — but the consumer that \
              asked for this warns at STARTUP, before any session exists, and a report scoped to a \
              session or a thread cannot be asked then. \
              \
              WHY IT HAD TO EXIST, reported and measured (app-foundations 41j0.4yjx, on nxs \
              0.67.0): `Worker::session_is_running` defaults to `false`, so a `false` meaning \
              `nothing is running` and one meaning `I never looked` are the same word — and every \
              hand-off this crate makes acts on it, handing this device's checkout to a rival \
              while the holder may still have hands on the git index (`release_working_tree` did \
              this alone until nxf 6j6v.b9nf removed it; the guard is shared by every park now). \
              That host shipped a stand-in from v0.67.0 which classified the resolved \
              `WorkerConfig` in \
              its own repository, against an enum it does not own, and stayed silent for \
              `Custom` — the host that brings its own worker and needs the answer most.",
    },
    // The `mark_read` row stood here — `Handle`, `Recorded::Unread`, the ONE entry on
    // `THE_RECORDED_AND_UNREADABLE` and the finding this whole rule was built out of. It went with
    // the call on 2026-09-08 (nxf 6j6v.4d2z): the ack, the synced cursor it advanced, the unread
    // set that consumed the cursor and the per-channel counts derived from it were removed
    // together, after the one live reader left on the seam had been REPLACED (nxf 6j6v.2hx9)
    // rather than deleted. The module doc carries the full account.
    Member {
        name: "transcript_append",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "what one session DID, entry by entry",
            by: &["transcript_page"],
        },
        causes: Causes::Nothing,
        why: "Record what a session DID — the write half of `transcript_page` (nxf 6j6v.c6e8). It \
              belongs to the same machinery as `bind_runtime_session`/`session_ended` and is here \
              for the identical reason: a local sidecar shells out to `nxc transcript append`, a \
              host that drives the SDK itself cannot reach anything but this handle. Measured, not \
              supposed — app-foundations bound a session and then had nothing to show for it \
              (`41j0.9brv`: `real=…  (0 entries)`). \
              \
              WHY IT IS NOT A THIRD WAY TO SPEAK, which is the shape 6j6v.ckeq closed: it posts no \
              message, addresses no target and wakes nobody. It appends to a device-local table \
              that is never synced and that only `transcript_page` reads.",
    },
    Member {
        name: "prune_transcripts",
        kind: Handle,
        records: Recorded::ReadBy {
            fact: "that transcripts older than the window are gone — readable as their absence in \
                   the window a caller asks for, and reported entry by entry by this call itself",
            by: &["transcript_page"],
        },
        causes: Causes::Nothing,
        why: "The operator's lever over stored transcripts. A write about retention, not a read \
              of the conversation.",
    },
];

/// The eight, spelled once more — SORTED, so the assertion is about the SET and the table above
/// stays free to read in the order a reader wants it in.
const THE_EIGHT: &[&str] = &[
    "directory",
    "persona_prime",
    "prime_as",
    "search",
    "status",
    "subscribe",
    "thread",
    "transcript_page",
];

/// The WRITING seam's two ways to SPEAK, closed by nexus-flow-6j6v.ckeq and asserted here because
/// nothing else notices a third (gate added by the PR #347 review, Test Quality #2).
///
/// `seam_disposition.rs` records `Engine::send`/`Engine::reply` as gone, but it compares NAMES over
/// the union of `engine.rs` + `facade.rs` — and `facade::send`/`facade::reply` stay, as the writes
/// everything else is built from. So that gate cannot tell a handle METHOD from a compute-layer
/// free function, says so in its own doc, and a third writing verb could grow back on `Engine`
/// without it noticing. This list is what notices.
///
/// **It counts SPEAKING and no longer the write seam as a whole** (nxf 6j6v.12nn). Until then it
/// was four names — these two plus `withdraw` and `release_working_tree` — and a count that had to
/// be widened twice in a fortnight while calling itself closed is not describing the thing it
/// guards. What ckeq's owner sentence bought is THIS count; the other two writes are on the seam by
/// the criterion in the module doc's third section, and the gate holds each of them against the
/// source. What this must never admit is a third way to POST — that is the shape ckeq removed:
/// `send` posted into a substrate channel and woke nobody, `reply` took a message OR a thread. If
/// one is genuinely needed again, that is a decision to re-take with the owner and to record in
/// `seam_disposition.rs` — not a method to add here quietly.
const THE_TWO_WAYS_TO_SPEAK: &[&str] = &["reply_thread", "send_to"];

/// The writes admitted by the CRITERION rather than by the count — the receipt of that category,
/// not its gate (nxf 6j6v.12nn).
///
/// The gate is the criterion, held row by row against the source: `withdraw` and `name_thread`
/// reach no spawn path at all. This list exists for [`THE_RECORDED_AND_UNREADABLE`]'s reason one
/// rule over — an admission that joins in silence is the next broken assurance — and each of these
/// two was an owner decision with an item behind it (`6j6v.0djn`, `6j6v.e76c`).
///
/// **It got SHORTER, not longer, exactly as the board said it would**: `release_working_tree` used
/// to be the third name here — it reached the spawn funnel and said whose commission it let begin
/// — and `6j6v.2cbw` decided it leaves the user surface. `nxf 6j6v.b9nf` carried that out: the row
/// is gone from [`SURFACE`] along with the verb, not merely excused from this list.
const THE_WRITES_ADMITTED_BY_THE_CRITERION: &[&str] = &["name_thread", "withdraw"];

/// An entrance `6j6v.dvyq` §5 closed the door on, and what this criterion says about it — the
/// list EXPLAINED rather than reopened, and where the source can still answer, measured (nxf
/// 6j6v.12nn).
struct Refused {
    /// The entrance, spelled as §5 spells it.
    name: &'static str,
    /// The compute-layer function in THIS crate it would forward onto, and `None` where §5's
    /// entrance has no compute half here at all — which is checked too: such a name must be absent
    /// from the call graph, or this row has quietly become false.
    compute: Option<&'static str>,
    /// Whether that compute half reaches [`SPAWN_FUNNEL`]. Checked against the source, not stated:
    /// the point of this table is that the closed list agrees with the criterion for reasons a
    /// reader can verify.
    starts: bool,
    /// What refuses it, and why that agrees with §5 instead of re-deciding it.
    why: &'static str,
}

/// What `6j6v.dvyq` §5 keeps off this handle, read against the criterion.
///
/// **Three of the four are refused BY THE CRITERION and one is not**, and saying so is the point:
/// a rule that claimed to explain every entry of somebody else's closed list would be claiming more
/// than it holds. The three start something nobody had commissioned before the call, which is the
/// criterion refusing them in its own terms — ckeq's count refuses all three a second time, and
/// that agreement is worth having rather than glossing. `ask` is the one the criterion has nothing
/// to say about. What the criterion promises about the list as a whole is the weaker and checkable
/// thing: it never ADMITS anything the list excludes.
const THE_CLOSED_LIST: &[Refused] = &[
    Refused {
        name: "role_trigger",
        compute: Some("trigger_role"),
        starts: true,
        why: "IT IS THE SPAWN: given a role and a message it mints a session and triggers it. The \
              criterion refuses it on its own terms — it starts (measured, beside this line) and \
              what it starts is a commission the caller is making in this very call, so it can \
              cause work nobody had commissioned. ckeq refuses it a second time, because on the \
              handle it would be a second spelling of `send_to`'s persona path: a caller naming \
              whom to start, with words of its own, which is a `Causes::Speaks` row, and the \
              count is two.",
    },
    Refused {
        name: "role_resume",
        compute: Some("role_resume"),
        starts: true,
        why: "The same shape one step along: it starts a session for a role that has one already, \
              and the TURN it gives that session comes from the caller — so the criterion refuses \
              it too, for the reason it refuses `role_trigger` rather than by analogy. \
              `reply_thread` is the call for that case where a THREAD holds the return address, \
              which is the case an app has; resuming by role id is the agent surface's spelling \
              and stays there.",
    },
    Refused {
        name: "channel_open",
        compute: Some("channel_open"),
        starts: true,
        why: "It opens a round and fans it out — N sessions, none of which existed before the \
              call and none of which anybody had commissioned before it, which is the criterion's \
              refusal in one sentence. `send_to` into a declared channel IS this, decided by the \
              TARGET's declaration rather than by picking a second verb, which is the whole \
              argument of the surface it belongs to.",
    },
    Refused {
        name: "ask",
        compute: Some("ask"),
        starts: false,
        why: "THE ONE THIS CRITERION DOES NOT REFUSE, and it is here for that. What survives of \
              the verb is `facade::ask`, the write `send_to` is built out of: it posts and starts \
              nothing, so the criterion has no objection to it at all. What refuses it is ckeq — \
              it carries the CALLER's words to somebody who must answer them, so it is a way to \
              SPEAK, and there are two. A gate that let this row claim the criterion's refusal \
              would be teaching a later reader that the criterion is wider than it is.",
    },
];

/// The §5 entrances with no compute half in this crate at all — checked as an ABSENCE, so the
/// sentence above cannot quietly stop being true (nxf 6j6v.12nn).
///
/// They belong to a surface `nxc` does not carry. §5 decided them and this criterion neither
/// refuses nor admits them; what it says about them is only that they are not here. (§5's three
/// event-stream items of acceptance point 4 are deliberately not in this list: they are not verbs
/// with a function of their own to look for, and a row naming nothing checkable would be a
/// decoration.)
const THE_CLOSED_LIST_WITHOUT_A_COMPUTE_HALF: &[&str] = &[
    "workflow_liveness",
    "workflow_start",
    "workflow_step_done",
    "workflow_tick",
    "workflow_tickets_add",
];

/// A fact this seam HANDS OUT that asks a surface to branch — the half of the read-side rule no
/// source gate can measure, held as a reasoned line per entry (nxf 6j6v.dh41).
///
/// What the gate does check is the two cross-references: the call that reports it and the call that
/// answers it both have to be on `SURFACE`, and the answer has to be a WRITE. What it cannot check
/// is that this list is COMPLETE — no parser can tell that `awaiting_human` asks a human to act
/// while `depth` does not — so completeness is a reviewer's sentence, and this table is where it
/// goes.
struct Reported {
    /// The field, spelled where it is declared.
    fact: &'static str,
    /// The member of `SURFACE` that hands it out.
    reported_by: &'static str,
    /// The call that answers it, or the item that is still open.
    answer: Answer,
    /// Why it asks for a decision at all, and what answering it means.
    why: &'static str,
}

/// What a surface does about a [`Reported`] fact.
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// Answerable through this member of `SURFACE`, which the gate checks is a WRITE: a fact you
    /// act on is answered by DOING something, never by looking at it again.
    By(&'static str),
    /// Reported and not answerable through this seam at all. Carries the item that closes it; the
    /// set is asserted by name in [`THE_REPORTED_AND_UNANSWERABLE`].
    Open(&'static str),
}

/// The reported facts that cannot be answered — asserted by name, for
/// [`THE_RECORDED_AND_UNREADABLE`]'s reason (nxf 6j6v.dh41).
///
/// **Empty since nxf 6j6v.1vxs, and empty is a claim rather than an absence**: every fact
/// [`REPORTED`] holds names the write that answers it. The one entry that stood here left because
/// the fact stopped asking a surface to act, not because a verb was found for it — the module doc's
/// second bullet carries that argument in full, and it is the argument a future entry has to beat
/// before it is removed rather than answered.
const THE_REPORTED_AND_UNANSWERABLE: &[&str] = &[];

/// The facts this seam reports that ask a surface to DO something about them.
///
/// It holds facts about the WORKSPACE that a caller is expected to act on, which is why two
/// neighbours are deliberately absent. `SessionStateReport::worker_answers_liveness` reports on the
/// handle's own machinery and the decision it informs is the host's configuration, not a call here
/// (nxf 6j6v.t41e). `SendToReceipt::spawned`/`warnings` report that a summon did not start — the
/// answer is to commission it again through the same call, so there is no second verb to name and
/// nothing this table could hold to account.
const REPORTED: &[Reported] = &[
    Reported {
        fact: "SendToReceipt::queue_position / queued_behind",
        reported_by: "send_to",
        answer: Answer::By("withdraw"),
        why: "MINTED IS NOT RUNNING: the thread is open, the message is posted and the session is \
              minted, and the spawn happens only when the current holder releases the working-copy \
              lease. `nxc guide limits-and-safety` tells a surface to branch on exactly this, and \
              until nxf 6j6v.0djn there was no call to answer it with — the receipt said \
              \"position 3\" and the seam offered nothing to do about it. Answering it means \
              taking the commission back before anything starts.",
    },
    Reported {
        fact: "StatusOperation::holds_working_tree",
        reported_by: "status",
        answer: Answer::By("withdraw"),
        why: "Somewhere under this root the device-local working copy is held, and until nxf \
              6j6v.fabb the documented way out — answering the escalation — was the only one. \
              Measured in the proving ground (4jgn.g90w): ten threads holding the checkout, every \
              one complete with nothing outstanding, no process anywhere, and a round parked \
              behind them for good. `release_working_tree` answered it by name until nxf 6j6v.b9nf \
              removed the verb; a held copy of a chain that is still running is now taken back the \
              same way a queued commission is, by withdrawing it — a chain that is simply dead is \
              the sweep's to notice, unattended, and needs no answer through this seam at all.",
    },
    Reported {
        fact: "StatusThread::escalated / StatusOperation::needs_decision",
        reported_by: "status",
        answer: Answer::By("reply_thread"),
        why: "The agent side handed the task back instead of answering it, and an unanswered \
              escalation holds the lease, so it stops the machine as well as the round. Answering \
              it is a reply into that thread — which is also why nxf 6j6v.gk9j had to put the \
              notice in the TEXT the woken session reads first: the field was already here, and \
              two escalations still went an hour unread because a field a recipient never looks \
              at is not a signal.",
    },
];

#[test]
fn the_read_seam_is_eight_verbs_and_the_whole_handle_is_accounted_for() {
    let source = crate_relative(env!("CARGO_MANIFEST_DIR"), "src/engine.rs");
    let text = std::fs::read_to_string(&source)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", source.display()));
    let actual: BTreeSet<String> = public_symbols(&text).expect("engine.rs parses as Rust");
    assert!(
        !actual.is_empty(),
        "the gate has lost sight of the surface it watches — engine.rs declares no `pub fn` at all"
    );
    let declared: BTreeSet<String> = SURFACE.iter().map(|m| m.name.to_owned()).collect();

    let undeclared: Vec<&String> = actual.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "\n\n`Engine` carries public methods this record does not name: {undeclared:?}\n\n  \
         Every method on the handle is a decision. Add a row to `SURFACE` saying what it is for —\n  \
         and if you are adding a READ, say which of the six questions it answers, or why it is on\n  \
         the seam anyway — `search` and `persona_prime` are here because a consumer measurably\n  \
         uses them, which is the other kind of reason (nexus-flow-6j6v.yr59, -6j6v.k8zq).\n"
    );
    let missing: Vec<&String> = declared.difference(&actual).collect();
    assert!(
        missing.is_empty(),
        "\n\n This record names methods `Engine` no longer has: {missing:?}\n\n  \
         A record that has fallen behind the code reads as a considered decision while describing\n  \
         a surface that does not exist. Delete the row, or put the method back.\n"
    );

    let mut reads: Vec<&str> = SURFACE
        .iter()
        .filter(|m| m.kind == Read)
        .map(|m| m.name)
        .collect();
    reads.sort_unstable();
    assert_eq!(
        reads, THE_EIGHT,
        "\n\n The read seam is EIGHT verbs (nexus-flow-6j6v.yr59, the owner correction of \
         2026-08-21, and nexus-flow-6j6v.k8zq). Found: {reads:?}\n"
    );
    assert!(
        SURFACE.iter().all(|m| !m.why.trim().is_empty()),
        "a member without a reason is a note, not a decision"
    );
}

/// The removals yr59 executed, asserted by NAME on the handle rather than by the absence of a
/// compile error — deleting the row above would otherwise be enough to make this file green about
/// a method that came back.
///
/// `facade::` counterparts are deliberately NOT checked here: most of them stay
/// (`facade::threads`, `facade::messages`, `facade::opener_wake`, …) because the CLI verbs that
/// reach them stay, and `seam_disposition.rs` is where that split is recorded row by row.
/// (`facade::inbox` stood in that list until nxf 6j6v.4d2z removed both halves — the exception
/// that proves the split is real, and its row next door says so.) What this holds is the half
/// that has no other gate: the FLAT METHOD an app reached for is gone from the handle.
///
/// `search` STOOD ON THIS LIST and does not any more — the owner's correction of 2026-08-21 put
/// `Engine::search` back, and a list that still named it would fail the moment the restoration
/// landed, which is the gate working. What holds `search` PRESENT is `SURFACE` and `THE_EIGHT`
/// above; the module doc carries why.
///
/// ELEVEN ENTRANCES, NOT ELEVEN READS, and the name says entrances for that reason (PR #347
/// review, Code Quality #4). Ten are reads; the eleventh is `open_with_poll_interval`, a
/// CONSTRUCTOR that yr59 folded into `EngineConfig::poll_interval`. It belongs in this list — it
/// is a flat entrance an app reached for, which is exactly what this gate holds gone — but calling
/// it a read would make the name claim something the list does not contain.
#[test]
fn the_eleven_entrances_yr59_took_off_the_handle_are_gone_from_it() {
    let source = crate_relative(env!("CARGO_MANIFEST_DIR"), "src/engine.rs");
    let text = std::fs::read_to_string(&source).expect("engine.rs reads");
    let actual: BTreeSet<String> = public_symbols(&text).expect("engine.rs parses as Rust");
    for (gone, successor) in [
        ("prime", "prime_as(consumer, None, now)"),
        ("transcript", "transcript_page(session, -1, None)"),
        ("open_with_poll_interval", "EngineConfig::poll_interval"),
        ("public_channels", "directory"),
        ("channels", "directory"),
        ("inbox", "prime_as"),
        ("opener_wake", "prime_as / status"),
        ("threads", "status"),
        ("thread_board", "status + thread"),
        ("thread_quorums", "status(StatusScope::Threads)"),
        ("messages", "thread"),
    ] {
        assert!(
            !actual.contains(gone),
            "`Engine::{gone}` is back on the handle. yr59 folded it into `{successor}`; if it \
             genuinely has to return, that is a decision to re-take with the owner, not a method \
             to re-add."
        );
    }
}

// ---- the read-side rule: nothing is recorded that cannot be read back (nxf 6j6v.dh41) ----------

/// `engine.rs` as text — what both halves of this rule are derived from, read the same way the
/// gate above reads it.
fn engine_source() -> String {
    let source = crate_relative(env!("CARGO_MANIFEST_DIR"), "src/engine.rs");
    std::fs::read_to_string(&source)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", source.display()))
}

/// The public methods of `Engine` whose body reaches the store's write path — the RECORDING calls,
/// derived from the source instead of declared beside it.
///
/// A syntax walk, not a text search, and `engine.rs` is why: the doc comment of the private
/// `resolve_definitions` names `Engine::with_orchestration` while the function writes nothing at
/// all. A gate that grepped would report it as a writer, and a gate whose findings have to be
/// checked by hand teaches its readers to skip them.
fn recording_calls(source: &str) -> BTreeSet<String> {
    method_callers(source, WRITE_PATH)
        .expect("engine.rs parses as Rust")
        .into_iter()
        // Inherent public methods only — that is what `SURFACE` is a table of. A trait-impl body
        // that writes is neither on this list nor silently dropped: it is a finding of its own,
        // held by `no_write_reaches_the_store_from_a_trait_impl` below.
        .filter(|c| c.public && !c.trait_impl)
        .map(|c| c.name)
        .collect()
}

/// The RECORDED half of the rule as a pure function over the writers the SOURCE names and the table
/// that claims to account for them — so the counter-proof can walk both directions without a second
/// `engine.rs` on disk. `verb_seam.rs::verdict` has the same shape for the same reason.
///
/// `None` means the seam holds. `Some(text)` is the complaint, already formatted for the panic.
///
/// `declared` is the exception list to hold the table against — [`THE_RECORDED_AND_UNREADABLE`] for
/// the real check, a synthetic one for the counter-proof. It became a parameter when 4d2z emptied
/// the real list: with a `const` list the counter-proof could only ever drive drift in the GROWING
/// direction, and a list that is empty for real is exactly where an unchecked comparison hides.
/// `reported_verdict` next door has taken its list this way since it was written.
fn recorded_verdict(
    writers: &BTreeSet<String>,
    surface: &[Member],
    declared: &[&str],
) -> Option<String> {
    let members: BTreeSet<&str> = surface.iter().map(|m| m.name).collect();
    let mut findings: Vec<String> = Vec::new();
    let mut unread: Vec<&str> = Vec::new();

    for m in surface {
        let writes = writers.contains(m.name);
        match &m.records {
            Recorded::Nothing if writes => findings.push(format!(
                "{:<24} reaches the store's write path and this row says it records NOTHING. Name \
                 what it writes down and the read it comes back through — or, if it genuinely \
                 cannot come back yet, `Recorded::Unread` with the item that ends that.",
                m.name
            )),
            Recorded::Nothing => {}
            _ if !writes => findings.push(format!(
                "{:<24} claims to record a fact and never reaches the store's write path. A row \
                 describing a write that does not happen reads as a considered decision about \
                 nothing.",
                m.name
            )),
            Recorded::ReadBy { fact, by } => {
                if fact.trim().is_empty() {
                    findings.push(format!(
                        "{:<24} names no fact. What comes back has to be said before it can be \
                         said where from.",
                        m.name
                    ));
                }
                if by.is_empty() {
                    findings.push(format!(
                        "{:<24} records something and names no read it comes back through.",
                        m.name
                    ));
                }
                for reader in *by {
                    if !members.contains(reader) {
                        findings.push(format!(
                            "{:<24} says its fact comes back through `{reader}`, which is not on \
                             this surface at all.",
                            m.name
                        ));
                    } else if *reader == m.name {
                        findings.push(format!(
                            "{:<24} names ITSELF as the read its fact comes back through.",
                            m.name
                        ));
                    } else if writers.contains(*reader) {
                        findings.push(format!(
                            "{:<24} says its fact comes back through `{reader}`, which the source \
                             says is a WRITE. A write is not how something is read back.",
                            m.name
                        ));
                    }
                }
            }
            Recorded::Unread { fact, item, why } => {
                unread.push(m.name);
                if fact.trim().is_empty() || item.trim().is_empty() || why.trim().is_empty() {
                    findings.push(format!(
                        "{:<24} is carried as a breach without naming its fact, its item or its \
                         reason. An exception without all three is a finding, not a free pass.",
                        m.name
                    ));
                }
            }
        }
    }

    for writer in writers {
        if !members.contains(writer.as_str()) {
            findings.push(format!(
                "{writer:<24} reaches the store's write path and this record does not name it at \
                 all."
            ));
        }
    }

    let mut declared: Vec<&str> = declared.to_vec();
    declared.sort_unstable();
    unread.sort_unstable();
    if unread != declared {
        findings.push(format!(
            "the list of recorded-and-unreadable facts is {unread:?}, and \
             `THE_RECORDED_AND_UNREADABLE` says {declared:?}. A list that grows in silence is the \
             next broken assurance — if a new breach genuinely has to stand, name it there and say \
             which item ends it."
        ));
    }

    (!findings.is_empty()).then(|| {
        format!(
            "\n\n Every fact this seam RECORDS must be readable back through this seam (nxf \
             6j6v.dh41).\n\n  {}\n\n  \
             WHICH CALLS RECORD IS NOT THE TABLE'S DECISION: it is read out of `engine.rs`, where \
             a call\n  records when its body reaches {WRITE_PATH:?}. So the row and the source \
             disagree, and the\n  source is the one that is true.\n",
            findings.join("\n  ")
        )
    })
}

/// The REPORTED half — the cross-references a source gate CAN hold. Completeness it cannot, and the
/// table's own doc says so rather than implying otherwise.
fn reported_verdict(
    writers: &BTreeSet<String>,
    surface: &[Member],
    reported: &[Reported],
    declared_open: &[&str],
) -> Option<String> {
    let members: BTreeSet<&str> = surface.iter().map(|m| m.name).collect();
    let mut findings: Vec<String> = Vec::new();
    let mut open: Vec<&str> = Vec::new();

    for r in reported {
        if r.fact.trim().is_empty() || r.why.trim().is_empty() {
            findings.push(format!(
                "{:<24} is listed without a fact or without a reason.",
                r.reported_by
            ));
        }
        if !members.contains(r.reported_by) {
            findings.push(format!(
                "`{}` is said to be reported by `{}`, which is not on this surface.",
                r.fact, r.reported_by
            ));
        }
        match r.answer {
            Answer::By(call) => {
                if !members.contains(call) {
                    findings.push(format!(
                        "`{}` is said to be answered by `{call}`, which is not on this surface.",
                        r.fact
                    ));
                } else if !writers.contains(call) {
                    findings.push(format!(
                        "`{}` is said to be answered by `{call}`, and the source says `{call}` \
                         writes nothing. A fact a surface acts on is answered by DOING something.",
                        r.fact
                    ));
                }
            }
            Answer::Open(item) => open.push(item),
        }
    }

    let mut declared: Vec<&str> = declared_open.to_vec();
    declared.sort_unstable();
    open.sort_unstable();
    if open != declared {
        findings.push(format!(
            "the list of reported-and-unanswerable facts is {open:?}, and the record says \
             {declared:?}."
        ));
    }

    (!findings.is_empty()).then(|| {
        format!(
            "\n\n A fact this seam REPORTS that asks a surface to branch must be answerable \
             through it (nxf 6j6v.dh41).\n\n  {}\n",
            findings.join("\n  ")
        )
    })
}

#[test]
fn every_fact_this_seam_records_is_readable_back_through_it() {
    let source = engine_source();
    let writers = recording_calls(&source);
    assert!(
        !writers.is_empty(),
        "the gate has lost sight of the path it watches — no public method of `Engine` reaches \
         {WRITE_PATH:?} any more, so every claim below it would pass for free"
    );
    if let Some(complaint) = recorded_verdict(&writers, SURFACE, THE_RECORDED_AND_UNREADABLE) {
        panic!("{complaint}");
    }
}

#[test]
fn every_reported_fact_that_asks_for_a_decision_names_the_call_that_answers_it() {
    let writers = recording_calls(&engine_source());
    if let Some(complaint) =
        reported_verdict(&writers, SURFACE, REPORTED, THE_REPORTED_AND_UNANSWERABLE)
    {
        panic!("{complaint}");
    }
}

/// The whole mechanical half rests on [`WRITE_PATH`] being the whole list of ways a write reaches
/// the store from `engine.rs`. This is what notices a third one: a private helper that takes the
/// write lock for somebody else is a new SPELLING of a write, and a gate that does not know it
/// calls every entry point behind it a plain read.
#[test]
fn the_write_path_has_exactly_the_two_spellings_this_gate_knows() {
    let callers = method_callers(&engine_source(), WRITE_PATH).expect("engine.rs parses as Rust");
    let funnels: Vec<&str> = callers
        .iter()
        .filter(|c| !c.public && !c.trait_impl)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        funnels, WRITE_PATH_FUNNELS,
        "\n\n `engine.rs` has private helpers reaching the store's write lock that this gate does \
         not know as spellings of a write: found {funnels:?}, expected {WRITE_PATH_FUNNELS:?}.\n\n  \
         Add it to `WRITE_PATH` **and** to `WRITE_PATH_FUNNELS`, or the public methods that write \
         through it\n  will read to this gate as calls that record nothing.\n"
    );
}

/// The write path's completeness held at its ROOT rather than asserted (PR #427 review, Test
/// Quality #2).
///
/// [`WRITE_PATH`] names the two spellings `engine.rs` writes through TODAY. The test above notices
/// a new private WRAPPER around one of those names — but a wholly new lender on the foundation
/// handle (a hypothetical `Handle::with_batch_mut`), called straight from a new `Engine` method,
/// would appear in no gate at all: `method_callers` would never be asked about that name, so the
/// row would read as `Recorded::Nothing` and pass. That is the one route around the mechanical half
/// that this file could not see, and naming it in a doc comment would leave it exactly as invisible.
///
/// So the claim is held where it is decidable: the foundation handle lends a `&mut State` through
/// EXACTLY ONE method, and everything `engine.rs` writes through has to funnel into it. Grow a
/// second lender and this fails, which is the moment to re-decide `WRITE_PATH` — before a caller
/// exists, not after one has slipped through.
#[test]
fn the_foundation_handle_lends_a_mutable_store_through_exactly_one_method() {
    let source = crate_relative(env!("CARGO_MANIFEST_DIR"), "../foundation/src/engine.rs");
    let text = std::fs::read_to_string(&source)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", source.display()));
    let lenders: Vec<String> = public_fns_lending_mut(&text, "State")
        .expect("foundation/src/engine.rs parses as Rust")
        .into_iter()
        .collect();
    assert_eq!(
        lenders,
        vec!["with_state_mut".to_owned()],
        "\n\n The foundation handle now hands out a `&mut State` through more than the one method \
         this gate's `WRITE_PATH` is built on: {lenders:?}\n\n  \
         `WRITE_PATH` is the list of spellings a write reaches the store through FROM \
         `engine.rs`,\n  and it is only complete while every route funnels into a single lender \
         down here. A second\n  lender is a route the read-side rule cannot see: a new `Engine` \
         method calling it directly\n  would read to this gate as a method that records nothing.\n\n  \
         Decide what the new lender means for the seam, then add its `engine.rs` spelling to \
         `WRITE_PATH`\n  and this name here. Do not simply widen this assertion.\n"
    );
}

/// A write reaching the store from inside a trait impl on `Engine` — the gap two of PR #427's
/// reviewers found independently, closed rather than documented.
///
/// `method_callers` REPORTS such a body (it does not drop it the way `public_symbols` does), so
/// this can refuse it. And refusing is the right answer rather than classifying it: `SURFACE` is
/// compared against `public_symbols`, which does not see trait-impl methods at all, so such a write
/// could never be given an honest row — it would be a write on the handle that this record
/// structurally cannot account for.
#[test]
fn no_write_reaches_the_store_from_a_trait_impl_in_engine_rs() {
    let callers = method_callers(&engine_source(), WRITE_PATH).expect("engine.rs parses as Rust");
    let in_trait_impl: Vec<&str> = callers
        .iter()
        .filter(|c| c.trait_impl)
        .map(|c| c.name.as_str())
        .collect();
    assert!(
        in_trait_impl.is_empty(),
        "\n\n `engine.rs` writes to the store from inside a TRAIT impl: {in_trait_impl:?}\n\n  \
         `SURFACE` is held against `public_symbols`, which does not see trait-impl methods — so \
         such a\n  write can never be given a row here, and the read-side rule cannot account for \
         it at all.\n  Move the write to an inherent method and give it a row, or say here why a \
         trait is the\n  right place for it and what reads its fact back.\n"
    );
}

/// The counter-proof, in both directions and in BOTH SPELLINGS of the same violation: a tenth
/// write whose fact nothing on the seam gives back.
///
/// The two bodies are the two ways `engine.rs` really writes — the handle's lock taken directly,
/// and the private orchestration funnel that takes it one frame down. A gate that saw only the
/// first would be exactly the hole this file exists to close, and it is not a hypothetical one:
/// the guard built in nxf 6j6v.pkw9 recognised one spelling of the start path it watched and let
/// four others through.
#[test]
fn a_recorded_fact_with_no_reader_is_red_in_either_spelling_of_the_write() {
    let real = recording_calls(&engine_source());

    for (spelling, body) in [
        (
            "the handle's write lock, taken directly",
            "self.handle.with_state_mut(|s| facade::stamp_the_seal(&mut s.store, seal))",
        ),
        (
            "the private orchestration funnel, one frame further down",
            "self.with_orchestration(caller, |ctx, store| orchestration::stamp_the_seal(ctx, store, seal))",
        ),
    ] {
        let source = format!(
            "impl Engine {{\n    \
             /// A tenth write. `with_orchestration` is named in this doc comment too, which is \
             the\n    /// shape a text search cannot tell from a call.\n    \
             pub fn stamp_the_seal(&self, caller: Caller<'_>, seal: &str) -> Result<()> {{\n        \
             {body}\n    }}\n}}\n"
        );
        let mut writers = recording_calls(&source);
        assert!(
            writers.contains("stamp_the_seal"),
            "the gate cannot SEE a write spelled as {spelling}, so every finding it makes about \
             one is an accident"
        );
        writers.extend(real.iter().cloned());

        let mut unread = SURFACE.to_vec();
        unread.push(Member {
            name: "stamp_the_seal",
            kind: Handle,
            records: Recorded::Nothing,
            causes: Causes::Nothing,
            why: "the counter-proof's tenth write",
        });
        let complaint = recorded_verdict(&writers, &unread, THE_RECORDED_AND_UNREADABLE).unwrap_or_else(|| {
            panic!(
                "a fact recorded through {spelling} and read back by NOTHING left the gate green"
            )
        });
        assert!(
            complaint.contains("stamp_the_seal"),
            "the gate reddened without naming the call that caused it:{complaint}"
        );

        // The other direction, same method and same spelling: name the read it comes back through
        // and the gate is satisfied. Without this the test above would also pass for a gate that
        // is simply always red.
        let mut read_back = SURFACE.to_vec();
        read_back.push(Member {
            name: "stamp_the_seal",
            kind: Handle,
            records: Recorded::ReadBy {
                fact: "the seal this call stamps",
                by: &["status"],
            },
            causes: Causes::Nothing,
            why: "the counter-proof's tenth write",
        });
        assert!(
            recorded_verdict(&writers, &read_back, THE_RECORDED_AND_UNREADABLE).is_none(),
            "naming the read a recorded fact comes back through did not satisfy the gate, \
             spelled as {spelling}"
        );
    }
}

/// The counter-proof for the thing the two exception lists EXIST for (PR #427 review, Test Quality
/// #1): that a list quietly falling out of step with the table it guards is caught.
///
/// Without this, the mismatch check was only ever run against the real tables, where the two agree
/// by construction — so a no-op or an inverted comparison would have passed every test in this
/// file while the module doc claimed "a list that grows in silence is the next broken assurance".
/// The claim is the centerpiece; it needed its own red.
///
/// Both directions, because drift has two of them: a breach that JOINS the table without joining
/// the list, and one that LEAVES the list while the table still carries it.
#[test]
fn an_exception_list_that_drifts_from_the_table_it_guards_is_red() {
    let writers = recording_calls(&engine_source());

    // Both directions run against a SYNTHETIC breach and a synthetic list, not against the real
    // table: `THE_RECORDED_AND_UNREADABLE` is empty since nxf 6j6v.4d2z, so the shrinking direction
    // has no real entry left to repair — and an empty list is exactly the state where a comparison
    // that has stopped comparing looks like a healthy one.
    let source = "impl Engine {\n    pub fn stamp_the_seal(&self, seal: &str) -> Result<()> {\n        \
                  self.handle.with_state_mut(|s| facade::stamp_the_seal(&mut s.store, seal))\n    }\n}\n";
    let mut grown = writers.clone();
    grown.extend(recording_calls(source));

    // GROWING: an unreadable fact appears in the table and not in the list.
    let mut table = SURFACE.to_vec();
    table.push(Member {
        name: "stamp_the_seal",
        kind: Handle,
        records: Recorded::Unread {
            fact: "the seal",
            item: "6j6v.zzzz",
            why: "a breach that never joined the list",
        },
        causes: Causes::Nothing,
        why: "the counter-proof's breach",
    });
    let complaint = recorded_verdict(&grown, &table, THE_RECORDED_AND_UNREADABLE)
        .expect("a recorded-and-unreadable fact outside the list left the gate green");
    assert!(
        complaint.contains("stamp_the_seal") && complaint.contains("THE_RECORDED_AND_UNREADABLE"),
        "the gate reddened without naming the drift:{complaint}"
    );

    // …and the same table WITH the entry named is green, so the direction above is the list check
    // firing rather than the row being rejected on some other ground.
    assert!(
        recorded_verdict(&grown, &table, &["stamp_the_seal"]).is_none(),
        "a breach named in the list it is held against did not satisfy the gate"
    );

    // SHRINKING: the breach is repaired — it names a read its fact comes back through — and the
    // list still carries it.
    let mut repaired = table.clone();
    let row = repaired
        .iter_mut()
        .find(|m| m.name == "stamp_the_seal")
        .expect("the synthetic breach is the entry this direction is about");
    row.records = Recorded::ReadBy {
        fact: "the seal",
        by: &["status"],
    };
    let complaint = recorded_verdict(&grown, &repaired, &["stamp_the_seal"])
        .expect("a repaired breach still named in the list left the gate green");
    assert!(
        complaint.contains("THE_RECORDED_AND_UNREADABLE"),
        "the gate reddened for the wrong reason:{complaint}"
    );

    // …and the same for the REPORTED half's list, whose real content is EMPTY since nxf 6j6v.1vxs —
    // which is the direction that matters there: a fact nothing can answer, added to the table and
    // not to the list, is exactly how an empty list would stop being a claim.
    let drifted = &[Reported {
        fact: "SomeReport::an_unanswerable_thing",
        reported_by: "status",
        answer: Answer::Open("6j6v.zzzz"),
        why: "an unanswerable fact that never joined the list",
    }];
    let complaint = reported_verdict(&writers, SURFACE, drifted, THE_REPORTED_AND_UNANSWERABLE)
        .expect("a reported-and-unanswerable fact outside the list left the gate green");
    assert!(
        complaint.contains("6j6v.zzzz"),
        "the gate reddened without naming the drift:{complaint}"
    );
}

/// A recording call with NO row here at all — the branch that catches the method somebody added
/// without touching this file (PR #427 review, Test Quality #3).
///
/// The neighbouring `the_read_seam_is_eight_verbs_…` also fails in that case, which is why this is
/// the cheap end of the review's list rather than a gap. It is held anyway because the two say
/// different things: that one says the RECORD is incomplete, this one says a WRITE is unaccounted
/// for — and the day the surface test is scoped differently, this branch would otherwise be the
/// only thing standing and never once proven to work.
#[test]
fn a_writer_with_no_row_at_all_is_red() {
    let source = "impl Engine {\n    pub fn stamp_the_seal(&self, seal: &str) -> Result<()> {\n        \
                  self.handle.with_state_mut(|s| facade::stamp_the_seal(&mut s.store, seal))\n    }\n}\n";
    let mut writers = recording_calls(&engine_source());
    writers.extend(recording_calls(source));

    let complaint = recorded_verdict(&writers, SURFACE, THE_RECORDED_AND_UNREADABLE)
        .expect("a write this record never mentions left the gate green");
    assert!(
        complaint.contains("stamp_the_seal") && complaint.contains("does not name it at all"),
        "the gate reddened without naming the unaccounted write:{complaint}"
    );
}

/// The REPORTED half's own counter-proof: a fact answered by a READ is not answered.
#[test]
fn a_reported_fact_answered_by_a_read_is_red() {
    let writers = recording_calls(&engine_source());
    // No `Answer::Open` row at all, so this table agrees with the (empty) exception list and the
    // list check cannot be what reddens it — which is the whole point of the counter-proof.
    let table = &[Reported {
        fact: "StatusOperation::holds_working_tree",
        reported_by: "status",
        answer: Answer::By("status"),
        why: "answered by looking again, which is not answering",
    }];
    let complaint = reported_verdict(&writers, SURFACE, table, THE_REPORTED_AND_UNANSWERABLE)
        .expect("a reported fact whose answer is a READ left the gate green");
    assert!(
        complaint.contains("writes nothing"),
        "the gate reddened for the wrong reason:{complaint}"
    );
}

// ---- the write-side rule: nothing starts that was not already commissioned (nxf 6j6v.12nn) -----

/// Every `.rs` file under `chat`'s own `src/`, sorted — the sources the write-side rule joins into
/// one call graph.
///
/// **The WHOLE crate, and not the three files the write seam forwards into**, because a path that
/// leaves those three and comes back is exactly what a narrower join would miss while reporting an
/// all-clear. The cost of joining is stated where [`callers_reaching`] documents it: a node is a
/// NAME, so `Engine::withdraw` and `orchestration::withdraw` are one node whose calls are the union
/// of both bodies. For a seam of thin forwardings that merge is what makes the question answerable
/// at all, and it errs toward REACHABLE — a false finding rather than a false all-clear.
///
/// **That false finding arrived, and what it cost is written down here rather than left as a
/// warning nobody ever collected on** (nxf 6j6v.b9nf). When `withdraw` grew the running case it
/// gained two frames — `Worker::stop_session`, and the park preconditions `will_park` is derived
/// from, which run `git rev-parse`. Neither starts anything; but `worker::SidecarWorker::
/// stop_session_by` calls its injected signaller `send(pid)` and `park::WorkingCopy::git_bytes`
/// hands a drained pipe over with `tx.send(buf)`, and `chat` also has a free `fn send` that
/// reaches the spawn funnel. Joined by name, this gate said `withdraw` starts sessions. EVERY path
/// it found went through that one node, and both of its entrances were a call Rust cannot make to
/// that function — a method call, and a call to a parameter.
///
/// It was settled by sharpening the graph and not by widening the row: [`call_graph`] no longer
/// records a call to the enclosing function's own parameter, and [`callers_reaching`] no longer
/// walks a method-call edge into a name the graph defines only at module scope. Both drop edges
/// the language cannot take, so nothing real was hidden — and [`a_further_write_that_starts_a_session_and_calls_itself_harmless_is_red`]
/// is what holds that, since it drives the same graph with a write that genuinely starts one.
fn chat_sources() -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{} could not be listed: {e}", dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|e| panic!("an entry of {} could not be read: {e}", dir.display()))
                .path();
            // A SYMLINK IS REFUSED RATHER THAN FOLLOWED (PR #430 review, Integrity #4). Following
            // one would either pull code this rule never meant to measure into the graph, or — in
            // a cycle — overflow the stack instead of saying anything. There are none under
            // `chat/src` today, and the day somebody adds one is the day this gate's reader has to
            // decide what it means, which is what a loud refusal asks for and a silent skip does
            // not.
            assert!(
                !path.is_symlink(),
                "{} is a symlink, and the write-side rule walks `chat/src` to decide what can \
                 start a session. Decide what the link means for that measurement — include its \
                 target deliberately, or take the link out — rather than letting this walk follow \
                 it.",
                path.display()
            );
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&crate_relative(env!("CARGO_MANIFEST_DIR"), "src"), &mut out);
    out.sort();
    assert!(
        out.iter().any(|p| p.ends_with("engine.rs")),
        "the write-side gate has lost sight of the sources it walks — `chat/src` does not contain \
         engine.rs, so every claim below it would pass for free"
    );
    out
}

/// `chat`'s own sources as ONE call graph, joined by name.
fn chat_call_graph() -> Vec<MethodCaller> {
    let mut graph = Vec::new();
    for source in chat_sources() {
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|e| panic!("{} could not be read: {e}", source.display()));
        graph.extend(
            call_graph(&text)
                .unwrap_or_else(|e| panic!("{} parses as Rust: {e}", source.display())),
        );
    }
    graph
}

/// The calls that can START a session — derived from the source instead of declared beside it, the
/// way [`recording_calls`] is.
///
/// It is a set over the whole joined graph, not over the handle: `SURFACE`'s rows are looked up in
/// it. A public method of `Engine` that starts something and has NO row here fails next door, in
/// [`the_read_seam_is_eight_verbs_and_the_whole_handle_is_accounted_for`], before it can reach this
/// rule — which is why this one does not walk the other direction as the recorded half has to.
fn starting_calls(graph: &[MethodCaller]) -> BTreeSet<String> {
    let starters = callers_reaching(graph, SPAWN_FUNNEL);
    assert!(
        starters.contains("trigger_role"),
        "the gate has lost sight of the path it watches — `trigger_role`, whose whole job is to \
         start a declared role, no longer reaches {SPAWN_FUNNEL:?}, so every row below would read \
         as a call that starts nothing"
    );
    starters
}

/// The write-side rule as a pure function over the calls the SOURCE says can start work and the
/// table that claims to account for them — so the counter-proofs can walk both directions without
/// a second `chat/src` on disk. [`recorded_verdict`] has the same shape for the same reason.
///
/// `None` means the seam holds. `Some(text)` is the complaint, already formatted for the panic.
fn causes_verdict(
    starters: &BTreeSet<String>,
    surface: &[Member],
    speak: &[&str],
    admitted: &[&str],
) -> Option<String> {
    let members: BTreeSet<&str> = surface.iter().map(|m| m.name).collect();
    let speakers: BTreeSet<&str> = surface
        .iter()
        .filter(|m| matches!(m.causes, Causes::Speaks { .. }))
        .map(|m| m.name)
        .collect();
    let mut findings: Vec<String> = Vec::new();

    for m in surface {
        let starts = starters.contains(m.name);
        // THE ARM ORDER IS LOAD-BEARING (PR #430 review, Code Quality): `_ if !starts` has to sit
        // ahead of the two variants below it, so that "this row claims a start the source does not
        // have" is reported as itself rather than falling into a check about the row's fields.
        // `recorded_verdict` above is built the same way for the same reason.
        match &m.causes {
            Causes::Nothing if starts => findings.push(format!(
                "{:<24} reaches the spawn funnel and this row says it starts NOTHING. Say what it \
                 sets going, and — unless the caller's own words are what set it going, which is \
                 `Causes::Speaks` and a count of two — name the call that commissioned it.",
                m.name
            )),
            Causes::Nothing => {}
            _ if !starts => findings.push(format!(
                "{:<24} claims to start work and reaches no spawn path at all. A row describing a \
                 start that cannot happen reads as a considered decision about nothing.",
                m.name
            )),
            Causes::Speaks { work } => {
                if work.trim().is_empty() {
                    findings.push(format!(
                        "{:<24} speaks and names no work. What it sets going has to be said.",
                        m.name
                    ));
                }
                if m.kind != Write {
                    findings.push(format!(
                        "{:<24} carries the caller's words to somebody who must answer them and is \
                         not classified as a write. Speaking IS writing; 6j6v.ckeq's count is over \
                         `Kind::Write`.",
                        m.name
                    ));
                }
            }
            Causes::OnlyWhatWasCommissioned {
                starts: what,
                commissioned_by,
            } => {
                if what.trim().is_empty() {
                    findings.push(format!(
                        "{:<24} starts something and does not say what. That sentence is the whole \
                         point of this variant.",
                        m.name
                    ));
                }
                if !members.contains(commissioned_by) {
                    findings.push(format!(
                        "{:<24} says its work was commissioned by `{commissioned_by}`, which is \
                         not on this surface at all.",
                        m.name
                    ));
                } else if *commissioned_by == m.name {
                    findings.push(format!(
                        "{:<24} names ITSELF as what commissioned the work it starts, which is the \
                         claim it was supposed to rule out.",
                        m.name
                    ));
                } else if !speakers.contains(commissioned_by) {
                    findings.push(format!(
                        "{:<24} says its work was commissioned by `{commissioned_by}`, and that \
                         row commissions nothing. Work is commissioned by SPEAKING — one of \
                         {speak:?} — so either the naming is wrong or a third way to speak has \
                         grown.",
                        m.name
                    ));
                }
            }
        }
    }

    let mut speaking: Vec<&str> = speakers.iter().copied().collect();
    speaking.sort_unstable();
    let mut declared_speak: Vec<&str> = speak.to_vec();
    declared_speak.sort_unstable();
    if speaking != declared_speak {
        findings.push(format!(
            "the ways to SPEAK are {speaking:?} and the record says {declared_speak:?}. \
             nexus-flow-6j6v.ckeq closed that count on the owner's sentence \"`send_to` und \
             `reply_thread` sollen der einzige Weg der KOMMUNIKATION sein\" — a third is a \
             decision to re-take with the owner and to record in `seam_disposition.rs`, not a row \
             to add here quietly."
        ));
    }

    let mut by_criterion: Vec<&str> = surface
        .iter()
        .filter(|m| m.kind == Write && !speakers.contains(m.name))
        .map(|m| m.name)
        .collect();
    by_criterion.sort_unstable();
    let mut declared_admitted: Vec<&str> = admitted.to_vec();
    declared_admitted.sort_unstable();
    if by_criterion != declared_admitted {
        findings.push(format!(
            "the writes admitted by the criterion are {by_criterion:?}, and \
             `THE_WRITES_ADMITTED_BY_THE_CRITERION` says {declared_admitted:?}. That list is the \
             RECEIPT of a category the criterion gates — an admission that joins it in silence is \
             the next broken assurance, and both entries on it are owner decisions with an item \
             behind them."
        ));
    }

    (!findings.is_empty()).then(|| {
        format!(
            "\n\n A call may reach the write seam when it cannot cause work that was not already \
             commissioned (nxf 6j6v.12nn).\n\n  {}\n\n  \
             WHETHER A CALL STARTS WORK IS NOT THE TABLE'S DECISION: it is read out of `chat/src`, \
             where a\n  call starts when its body reaches {SPAWN_FUNNEL:?} through however many \
             frames. So where a row and\n  the source disagree, the source is the one that is \
             true.\n",
            findings.join("\n  ")
        )
    })
}

#[test]
fn nothing_on_this_seam_starts_work_that_was_not_already_commissioned() {
    let starters = starting_calls(&chat_call_graph());
    if let Some(complaint) = causes_verdict(
        &starters,
        SURFACE,
        THE_TWO_WAYS_TO_SPEAK,
        THE_WRITES_ADMITTED_BY_THE_CRITERION,
    ) {
        panic!("{complaint}");
    }
}

/// The whole mechanical half rests on [`SPAWN_FUNNEL`] being the whole list of ways a session is
/// started, and this is what notices a second one — the write side's counterpart to
/// [`the_foundation_handle_lends_a_mutable_store_through_exactly_one_method`], and held at the same
/// place: the ROOT, where the claim is decidable, rather than one layer up where a new route would
/// simply be invisible.
///
/// `Worker::trigger` is the root. Everything that spawns has to call it, and in `chat/src` exactly
/// one function that is not itself a `Worker` does: `trigger_and_bind`. The other two callers are
/// `Worker::trigger` implementations — the CLI's `LazyWorker`, which resolves a worker and forwards,
/// and the bounded delegator that runs a host worker on a thread — and a worker calling a worker is
/// past the funnel rather than a way around it, which is why they are excluded by SHAPE
/// (`trait_impl`) instead of by name. Since PR #430's review that shape also covers a trait's own
/// DEFAULT body, which is the right side of the line for the same reason: `Worker`'s defaults are
/// the worker's own code. What that review actually closed is a level below this test — such a
/// body was in no call graph at all, so its outgoing calls were invisible to
/// [`callers_reaching`] and to the rule that rests on it.
#[test]
fn the_compute_layer_starts_a_session_through_exactly_one_funnel() {
    let mut spawners: Vec<String> = Vec::new();
    for source in chat_sources() {
        let text = std::fs::read_to_string(&source).expect("a chat source reads");
        let callers = method_callers(&text, &["trigger"])
            .unwrap_or_else(|e| panic!("{} parses as Rust: {e}", source.display()));
        spawners.extend(
            callers
                .into_iter()
                .filter(|c| !c.trait_impl)
                .map(|c| c.name),
        );
    }
    spawners.sort();
    spawners.dedup();
    let spawners: Vec<&str> = spawners.iter().map(String::as_str).collect();
    assert_eq!(
        spawners,
        SPAWN_FUNNEL.to_vec(),
        "\n\n `chat` hands a `TriggerRequest` to a worker from somewhere this gate does not know \
         as a spawn: found {spawners:?}, expected {SPAWN_FUNNEL:?}.\n\n  \
         Every row of `SURFACE` that says it starts nothing is only as true as this list. Add the \
         new funnel\n  to `SPAWN_FUNNEL` — and then read the write rows again, because some of \
         them have just become\n  wrong — or route the spawn through the funnel that is already \
         there.\n"
    );
}

/// The counter-proof for the fact this whole rule was built out of: ONE MORE write that starts a
/// session and calls itself harmless.
///
/// It read "a FIFTH write" while there were four; there are five since nxf 6j6v.e76c added
/// `name_thread`, and the counter-proof was never about the count — it is about a row that says it
/// starts nothing while the source says otherwise. Spelled without the number so the next write
/// does not silently make this doc false.
///
/// It is the exact shape `release_working_tree` had for a fortnight — a thin forwarding whose row
/// says it only takes something away — and a gate that could not redden on it would be a decoration
/// over the defect it was built for. Both directions, because a gate that is always red passes the
/// first half just as well: naming what it starts and who commissioned it satisfies it.
#[test]
fn a_further_write_that_starts_a_session_and_calls_itself_harmless_is_red() {
    let synthetic = "impl Engine {\n    \
                     /// One more write, spelled the way the real ones are.\n    \
                     pub fn seal_the_round(&self, caller: Caller<'_>, thread: &str) -> Result<()> {\n        \
                     self.with_orchestration(caller, |ctx, store| trigger_role(ctx, store, thread))\n    }\n}\n";
    let mut graph = chat_call_graph();
    graph.extend(call_graph(synthetic).expect("the synthetic write parses as Rust"));
    let starters = starting_calls(&graph);
    assert!(
        starters.contains("seal_the_round"),
        "the gate cannot SEE a write that starts a session through the compute layer, so every \
         finding it makes about one is an accident"
    );

    let mut harmless = SURFACE.to_vec();
    harmless.push(Member {
        name: "seal_the_round",
        kind: Write,
        records: Recorded::Nothing,
        causes: Causes::Nothing,
        why: "the counter-proof's extra write, arguing that it only takes something away",
    });
    let complaint = causes_verdict(
        &starters,
        &harmless,
        THE_TWO_WAYS_TO_SPEAK,
        THE_WRITES_ADMITTED_BY_THE_CRITERION,
    )
    .expect("a write that starts a session and says it starts nothing left the gate green");
    assert!(
        complaint.contains("seal_the_round") && complaint.contains("says it starts NOTHING"),
        "the gate reddened without naming the write that caused it:{complaint}"
    );

    // The other direction, same method: say what it starts and name the call that commissioned it,
    // and the gate is satisfied — with the admitted list widened, because that list is a receipt
    // and a new admission has to be written into it.
    let mut honest = SURFACE.to_vec();
    honest.push(Member {
        name: "seal_the_round",
        kind: Write,
        records: Recorded::Nothing,
        causes: Causes::OnlyWhatWasCommissioned {
            starts: "the round's last member, held back until the seal",
            commissioned_by: "send_to",
        },
        why: "the counter-proof's extra write, arguing honestly",
    });
    assert!(
        causes_verdict(
            &starters,
            &honest,
            THE_TWO_WAYS_TO_SPEAK,
            &["name_thread", "seal_the_round", "withdraw"],
        )
        .is_none(),
        "naming what a write starts and who commissioned it did not satisfy the gate"
    );
}

/// The count 6j6v.ckeq closed, with its own red in BOTH directions — the half the criterion must
/// never be allowed to widen.
///
/// GROWING is the fifth write above, declared honestly as a third way to speak: honest and still
/// refused, because what refuses it is the count and not the criterion. SHRINKING is the direction
/// a list-versus-table comparison silently loses if it is written the wrong way round.
#[test]
fn a_third_way_to_speak_is_red_and_so_is_a_second_that_went_missing() {
    // The same synthetic write as the counter-proof above, declared HONESTLY this time: it starts,
    // the source agrees that it starts, and it says the words are the caller's. Honest and still
    // refused — because what refuses it is the count, which is the half the criterion must never
    // be allowed to soften. It stands in for a `role_trigger` grown back onto the handle.
    let synthetic = "impl Engine {\n    \
                     pub fn ask_directly(&self, caller: Caller<'_>, to: &str, body: &str) -> Result<()> {\n        \
                     self.with_orchestration(caller, |ctx, store| trigger_role(ctx, store, to, body))\n    }\n}\n";
    let mut graph = chat_call_graph();
    graph.extend(call_graph(synthetic).expect("the synthetic write parses as Rust"));
    let starters = starting_calls(&graph);

    let mut third = SURFACE.to_vec();
    third.push(Member {
        name: "ask_directly",
        kind: Write,
        records: Recorded::Nothing,
        causes: Causes::Speaks {
            work: "a session started to answer words the caller supplied",
        },
        why: "the counter-proof's third way to speak",
    });
    let complaint = causes_verdict(
        &starters,
        &third,
        THE_TWO_WAYS_TO_SPEAK,
        THE_WRITES_ADMITTED_BY_THE_CRITERION,
    )
    .expect("a third way to speak left the gate green");
    assert!(
        complaint.contains("the ways to SPEAK are"),
        "the gate reddened for the wrong reason:{complaint}"
    );

    let complaint = causes_verdict(
        &starting_calls(&chat_call_graph()),
        SURFACE,
        &["reply_thread", "send_to", "withdraw"],
        THE_WRITES_ADMITTED_BY_THE_CRITERION,
    )
    .expect("a speaker the record names and the table no longer has left the gate green");
    assert!(
        complaint.contains("the ways to SPEAK are"),
        "the gate reddened for the wrong reason:{complaint}"
    );
}

/// The cross-reference's own red: "already commissioned" has to name a call that COMMISSIONS.
///
/// Without this the naming would be a formality — any member of `SURFACE` would do, including the
/// reads — and the criterion's one checkable half would check nothing.
#[test]
fn work_said_to_be_commissioned_by_a_call_that_commissions_nothing_is_red() {
    let starters = starting_calls(&chat_call_graph());
    let mut table = SURFACE.to_vec();
    let row = table
        .iter_mut()
        .find(|m| m.name == "resume_interrupted")
        .expect("the entry this test is about");
    row.causes = Causes::OnlyWhatWasCommissioned {
        starts: "the interrupted session, continued",
        commissioned_by: "status",
    };
    let complaint = causes_verdict(
        &starters,
        &table,
        THE_TWO_WAYS_TO_SPEAK,
        THE_WRITES_ADMITTED_BY_THE_CRITERION,
    )
    .expect("work commissioned by a READ left the gate green");
    assert!(
        complaint.contains("commissions nothing"),
        "the gate reddened for the wrong reason:{complaint}"
    );
}

/// The defensive branches of [`causes_verdict`] that no other test reaches (PR #430 review, Test
/// Quality #1) — five ways a row can be wrong about itself that are not "it starts and says it
/// does not".
///
/// They are cheap to write and were worth the finding: an unexercised branch is a claim about what
/// a gate would do, and dropping one — the `kind != Write` check, say — would ship in silence
/// behind seventeen green tests. Each case reddens for its OWN reason, asserted on the sentence
/// that names it rather than on "something panicked", because several of these also disturb one of
/// the two name lists and a test that accepted any complaint would pass on the wrong one.
///
/// One direction only, deliberately: the green direction is
/// [`nothing_on_this_seam_starts_work_that_was_not_already_commissioned`] running the same function
/// over the same real table, so every case here is a single edit away from a proven-green baseline.
#[test]
fn a_row_that_is_wrong_about_itself_in_any_of_the_five_smaller_ways_is_red() {
    let starters = starting_calls(&chat_call_graph());

    // The five, each as an edit to one row of the real table plus the sentence it must produce.
    let cases: [(&str, &str, &str, Causes); 5] = [
        (
            "a way to speak that does not say what it sets going",
            "send_to",
            "speaks and names no work",
            Causes::Speaks { work: "  " },
        ),
        (
            "a start with no account of what is started",
            "resume_interrupted",
            "starts something and does not say what",
            Causes::OnlyWhatWasCommissioned {
                starts: "",
                commissioned_by: "send_to",
            },
        ),
        (
            "work commissioned by a call this table does not have",
            "resume_interrupted",
            "not on this surface at all",
            Causes::OnlyWhatWasCommissioned {
                starts: "the interrupted session, continued",
                commissioned_by: "coordinator_commission",
            },
        ),
        (
            "a row that names ITSELF as what commissioned its work",
            "resume_interrupted",
            "names ITSELF",
            Causes::OnlyWhatWasCommissioned {
                starts: "the interrupted session, continued",
                commissioned_by: "resume_interrupted",
            },
        ),
        (
            // `session_ended` is the row that makes this case possible at all: it is a `Handle`
            // entry and the source says it starts, so it can be given the wrong CATEGORY without
            // the source contradicting the start itself.
            "speaking from a row that is not classified as a write",
            "session_ended",
            "is not classified as a write",
            Causes::Speaks {
                work: "the next step of the round",
            },
        ),
    ];

    for (what, row_name, expected, causes) in cases {
        let mut table = SURFACE.to_vec();
        let row = table
            .iter_mut()
            .find(|m| m.name == row_name)
            .unwrap_or_else(|| panic!("`{row_name}` is the row `{what}` is written against"));
        row.causes = causes;
        let complaint = causes_verdict(
            &starters,
            &table,
            THE_TWO_WAYS_TO_SPEAK,
            THE_WRITES_ADMITTED_BY_THE_CRITERION,
        )
        .unwrap_or_else(|| panic!("{what} left the gate green"));
        assert!(
            complaint.contains(expected),
            "{what} reddened without saying so — expected a finding containing \
             {expected:?}:{complaint}"
        );
    }
}

/// `6j6v.dvyq` §5's closed list, held against the criterion rather than beside it — the DoD's
/// "explained, not refuted", and where the source can answer, measured.
///
/// Three things at once, because they are one claim: none of those entrances is on the handle;
/// each row's `starts` is what the SOURCE says about the compute half it names; and the entrances
/// with no compute half here are absent from the graph, so a row claiming they are elsewhere
/// cannot go quietly stale.
#[test]
fn the_closed_list_of_dvyq_5_is_off_the_handle_and_the_criterion_agrees_with_it() {
    let graph = chat_call_graph();
    let starters = starting_calls(&graph);
    let text = std::fs::read_to_string(crate_relative(env!("CARGO_MANIFEST_DIR"), "src/engine.rs"))
        .expect("engine.rs reads");
    let handle: BTreeSet<String> = public_symbols(&text).expect("engine.rs parses as Rust");

    for refused in THE_CLOSED_LIST {
        assert!(
            !handle.contains(refused.name),
            "`Engine::{}` is on the handle, and 6j6v.dvyq §5 closed the door on it. The criterion \
             does not reopen that list; if this entrance genuinely has to return, that is a \
             decision to re-take with the owner.",
            refused.name
        );
        assert!(
            !refused.why.trim().is_empty(),
            "`{}` is refused without a reason, which is a list entry and not a decision",
            refused.name
        );
        let Some(compute) = refused.compute else {
            continue;
        };
        assert_eq!(
            starters.contains(compute),
            refused.starts,
            "\n\n `{}` says its compute half `{compute}` {} a session, and the source says \
             otherwise.\n\n  \
             This table is how the closed list is EXPLAINED rather than asserted, so a row that \
             has drifted\n  from the source explains nothing — re-read what `{compute}` does and \
             say which refusal\n  actually applies to it, ckeq's count or this criterion.\n",
            refused.name,
            if refused.starts {
                "starts"
            } else {
                "does not start"
            }
        );
    }

    let names: BTreeSet<&str> = graph.iter().map(|f| f.name.as_str()).collect();
    for absent in THE_CLOSED_LIST_WITHOUT_A_COMPUTE_HALF {
        assert!(
            !names.contains(absent),
            "`{absent}` has a function in `chat/src` now, and this record says that surface is not \
             carried here at all. Either it came back — in which case 6j6v.dvyq §5 has to be read \
             again — or it is a name collision, and this list needs the more specific one."
        );
    }
}

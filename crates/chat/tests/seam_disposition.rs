//! nxc's half of the SEAM-DISPOSITION gate (nexus-flow-6j6v.dvyq, acceptance point 3).
//!
//! > **The AGENT surface and the APP SEAM are cut apart:** for **every** verb removed from the CLI
//! > it is **decided and recorded** whether the corresponding read/write **stays on `Engine`**. **A
//! > gate holds this, rather than leaving it to diligence.**
//!
//! This file is that record. [`nxs_test_support::seam_disposition`] is the gate that keeps every
//! row of it true, and its module doc explains what each state claims and how each claim is
//! checked.
//!
//! # The trap this exists for, in the item's own words
//!
//! > Removing a CLI verb is NOT the same as removing a seam read. `Engine::inbox` is what an app
//! > builds its unread view from; `threads`/`thread_board` are what `nxc status` is made of. What
//! > disappears from the AGENT surface stays on the APP seam, provided an app renders it. **Too
//! > coarse a cut breaks app-foundations at a place the CLI cannot see.**
//!
//! `verb_seam.rs` next door cannot see that place either — it asks only whether every verb an agent
//! types is reachable from the seam, so a read that leaves the agent surface and stays on the seam
//! is invisible to it in both directions. Delete `Engine::inbox` while `nxc inbox` is also gone and
//! that gate is perfectly green — a sentence written as a hypothetical and since made literal three
//! times over: `Engine::inbox` went in 6j6v.yr59, `nxc inbox` in 6j6v.1gm9 and `facade::inbox` in
//! 6j6v.4d2z, `verb_seam.rs` stayed green through all three, and the only record that any of them
//! happened is the `inbox` row in this file.
//!
//! # Two lists, and only one of them is negotiable
//!
//! **What LEAVES `Engine` is closed** — the item's §5 names it: `role_trigger`, `role_resume`,
//! `channel_open`, `ask`, `workflow_start`, `workflow_step_done`, `workflow_tickets_add`,
//! `workflow_tick`, `workflow_liveness`, plus the three event-stream items of acceptance point 4.
//! **Everything not on that list stays on `Engine` even when its CLI verb goes.** Where this table
//! records `Stays` for something whose verb is on the removal list, that is the closed list being
//! applied, not a judgement of mine.
//!
//! # A second item writes into the same table
//!
//! nxf 6j6v.ckeq cut the WRITING seam to two verbs (`send_to`, `reply_thread`) and three statements
//! each, which is a set of removals 6j6v.dvyq's closed list says nothing about — it was decided
//! afterwards, verb by verb at the source with the owner on 2026-08-21. Its rows carry their own
//! block below and their own reasons; the gate does not care which item a row came from, only that
//! it still describes reality.
//!
//! Two shapes appear there that had no precedent here. A row whose CLI half STANDS while the SEAM
//! field goes (`reply --if-unanswered`) — the table's usual direction reversed, because that switch
//! belongs to one non-app caller. And two rows that read `Stays` about methods that are GONE
//! (`Engine::send`, `Engine::reply`), which is this gate's documented limit rather than a fudge: it
//! compares symbol NAMES across `engine.rs` + `facade.rs`, and `facade::send`/`facade::reply` are
//! still on that union as the writes everything else is built out of. `ask` set that precedent
//! already; the reasons spell out what actually left.
//!
//! # A THIRD item writes into the same table
//!
//! nxf 6j6v.yr59 cut the READING seam to seven verbs, decided with the owner on 2026-08-21 in the
//! same verb-by-verb pass. Ten reads leave `Engine`; its rows are the block headed
//! *"the READING seam cut to seven verbs"* below, plus five rows in the first block that this item
//! REWROTE rather than added (`inbox`, `read`, `channels list`, `channels public`, `threads
//! list`/`threads show`).
//!
//! **Seven, not the six in that item's title**, and the `search` row below is where that is
//! recorded: the item covers `search` nowhere, the build removed it as a named loss in the item's
//! own terms and reported the gap, and the OWNER OVERRULED that on 2026-08-21 because
//! app-foundations consumes the method today. That row carries both arguments side by side, which
//! is the shape this table already uses for the two 2026-08-19 decisions yr59 reversed.
//!
//! **Two of those rewrites reverse a decision recorded here**, which is worth saying out loud
//! because the record is what a later reader trusts. `inbox` was kept by §5's own naming ("an app
//! builds its UNREAD VIEW from `Engine::inbox`") and `channels public` by an explicit owner
//! decision of 2026-08-19 that declined to fold discovery into `list`. Both are overturned by a
//! LATER owner decision, and each row now carries the old argument and the new one, so nothing
//! reads as an oversight. The 2026-08-19 argument is in fact why the `public_channels` fold had to
//! carry the store read into `directory` rather than drop the question.
//!
//! # Reading the `cli` column
//!
//! `Standing` means the entrance is STILL THERE, and the gate holds that: a `Standing` row whose
//! verb disappears fails. It carried a second meaning while §3 was in flight — "decided, not yet
//! executed", paired with `Leaves { landed: false }` on the seam half — and every such row has
//! since been executed. What is left on `Standing` is there by decision: `tick`, which no human
//! types at all and which the scheduled `at` job invokes; and — since 6j6v.yr59 — `prime`,
//! `threads list`/`show` and `transcript show`, whose verbs are untouched and whose HANDLE method
//! went. That last shape is the §5 warning running in its natural direction, and it is why those
//! rows exist at all.
//!
//! **`inbox` and `read` were the third kind and are not any more** (nxf 6j6v.1gm9, 2026-08-27).
//! They stood on `Standing` for a specific reason — off the AGENT surface (what `prime` teaches)
//! while staying in the binary for a human — and that is exactly the position the gate proved worth
//! recording, because it is the one a later decision can reverse. It did: the owner answered the
//! product question the other way (a human reads a CONVERSATION, not a flat unread list), the block
//! `read` was the brake on came out of `prime`, and both rows went `Gone` with their old argument
//! and the new one side by side.
//!
//! **And then both SEAM halves went too** (nxf 6j6v.4d2z, 2026-09-08), so the two rows are the only
//! ones in this table that have travelled the whole distance the gate can record: `Standing` +
//! `Stays`, then `Gone` + `Stays`, now `Gone` + `Leaves … landed`. That middle state is the one §5
//! warns about and this file exists to hold — a capability an app still renders after the verb has
//! gone — and it held for a fortnight while the removal was measured, held once for a live reader
//! it had missed, and then executed. The gate checked each half against the source at every step;
//! what it could not check, and what the reasons carry, is WHY.
//!
//! `search` stood in that first trio and is out of it after the owner's correction of 2026-08-21,
//! because it never fit: `prime`'s command reference TEACHES `nxc search` (see
//! `tests/prime_golden.rs`'s golden block, which lists it beside `list`/`send`/`reply`/`status`/
//! `transcript show` where `inbox` and `read` do not appear). So it gave up neither surface — it is
//! on the agent's, on the human's and, since the correction, on the app's.

use nxs_test_support::seam_disposition::{
    assert_seam_dispositions, CliState, Disposition, SeamState,
};
use nxs_test_support::verb_seam::crate_relative;

use CliState::{Gone, NeverWas, Standing};

/// Shorthand for the two seam states that name a symbol, so the table below reads as a table.
const fn stays(symbol: &'static str) -> SeamState {
    SeamState::Stays { symbol }
}
// `leaves(…)` — `Leaves { landed: false }`, a removal DECIDED and not yet executed — stood here
// and has no row left to serve: 6j6v.dvyq §3's last block executed the eight that were open. Kept
// as a comment rather than as an unused helper, because the STATE it built is still part of the
// gate and the next removal will want it back (`SeamState::Leaves { landed: false }`).
const fn left(symbol: &'static str) -> SeamState {
    SeamState::Leaves {
        symbol,
        landed: true,
    }
}

const RECORD: &[Disposition] = &[
    // ---- §5's trap: the read an app renders outlives the verb an agent typed ------------------
    Disposition {
        verb: "inbox",
        cli: Gone,
        seam: left("inbox"),
        reason: "§5 named this one by name — \"an app builds its UNREAD VIEW from `Engine::inbox`\" \
                 — and 6j6v.yr59 OVERTURNED that half on 2026-08-21. Owner: \"`inbox` gibt es \
                 eigentlich nicht [mehr].\" `Engine::inbox` was REMOVED from the handle then. \
                 \
                 Because the sentence §5 wrote it into came true: a persona gets what it needs at \
                 session start and on resume, and `Engine::prime_as` carried the catch-up already \
                 SPLIT by disposition — which is the whole of what `--all` distinguished. An app \
                 rendering an unread view rendered that record. THE RESIDUAL, named at the time: an \
                 app that wants the list WITHOUT priming has to prime, which does more work \
                 (roster, rule, address book). \
                 \
                 THE VERB WENT TOO, on 2026-08-27 (6j6v.1gm9), and this row is where that reversal \
                 is recorded. It had been `Standing` on one argument: the verb comes off the AGENT \
                 surface (the set `prime` TEACHES) while staying in the binary for a HUMAN, and \
                 whether a human wants it is a product question rather than a seam one. The owner \
                 ANSWERED that product question the other way: a human reads a CONVERSATION — `nxc \
                 threads show`, `nxc status` — not a flat unread list of messages with no thread to \
                 answer in. The boundary the whole ticket rests on is \"people pull, agents get \
                 pushed\", and this verb was on neither side of it. \
                 \
                 AND THE COMPUTE HALF WENT ON 2026-09-08 (6j6v.4d2z), which is why this row now \
                 reads `Leaves … landed`. For a fortnight it read `Stays` about a method that was \
                 gone from the HANDLE — this gate's documented limit, and the precedent \
                 `ask`/`Engine::send` set: it compares symbol NAMES across `engine.rs` + \
                 `facade.rs`, and `facade::inbox` was still on that union because `prime` read it. \
                 `Leaves` would have claimed a removal that had not happened; now it has. What went \
                 with it: `ChatStore::inbox`, the `read_cursors` table it gated on, \
                 `PrimeReport::in_turn`/`next_session`/`count`, and the per-channel unread counts \
                 on `ChannelView`. THE RESIDUAL above dissolves rather than being paid: there is no \
                 list to want. Coverage: `parity.rs`'s unread differential is gone with its two \
                 surfaces, and the note where it stood says why nothing was rehomed.",
    },
    Disposition {
        verb: "read",
        cli: Gone,
        seam: left("mark_read"),
        reason: "Off the AGENT surface with `inbox` in the same commit, and by the same rule: the \
                 agent surface is what a session is TAUGHT at its start, and `prime` no longer \
                 names the read cursor. THE VERB THEN WENT TOO, on 2026-08-27 (6j6v.1gm9) — it had \
                 been kept as inner mechanics (§4 listed it there), and the ticket that removed the \
                 one block it bounded found there was no mechanics left for it to be: `nxc read` \
                 was the brake on `prime`'s \"Threads you opened\", the brake measured 0 uses in 66 \
                 role sessions, and with the block gone nothing on the CLI renders or clears that \
                 list. \
                 \
                 `Engine::mark_read` STAYED at the time, on the argument that advancing the synced \
                 cursor is what makes delivery at-least-once and an app that renders a conversation \
                 is exactly who acks it. It had survived 6j6v.yr59 for that reason although the \
                 READ opposite it (`Engine::inbox`) did not. \
                 \
                 THE ARGUMENT DID NOT SURVIVE BEING MEASURED, and the write went on 2026-09-08 \
                 (6j6v.4d2z) — hence `Leaves … landed`. No product code called the ack over five \
                 layers of seam, so the at-least-once assurance was unredeemed rather than \
                 aspirational; `read_surface.rs` carried exactly that as its ONE \
                 recorded-and-unreadable entry under 6j6v.dh41, and that list is empty now. The \
                 removal was HELD once, on 2026-09-05, when its own gate found a second live reader \
                 of the cursor (`ChatStore::opener_wake`, feeding `prime --json`'s \
                 `threads_you_opened`); it landed only after 6j6v.2hx9 REPLACED that reader by \
                 deriving the notice from the operation. What went with the ack: `read_cursors`, \
                 the `read_cursor` op kind, `ChatStore::advance_read_cursor`/`read_cursor_seen`, \
                 and `ReadReceipt`. What replaces it: nothing — a delivery is retried by waking the \
                 session again, not by a watermark. Coverage that goes with it: `embed.rs`'s \
                 `mark_read` case and `public_channel.rs`'s membership refusal, both named where \
                 they stood.",
    },
    Disposition {
        verb: "channels list",
        cli: Gone,
        seam: stays("channels"),
        reason: "THE ONE ROW IN THIS TABLE WHOSE `Stays` HIDES AN UNCOMPENSATED HALF, and it is \
                 said first rather than at the end (PR #347 review, verification request 4: the \
                 reason admitted a loss the disposition did not reflect). Every OTHER read 6j6v.\
                 yr59 took off the handle has another HANDLE verb answering the same question. \
                 This one does not, entirely: PER-CHANNEL UNREAD COUNTS have no successor on the \
                 handle at all. \
                 \
                 So the row is half a collapse and half a NAMED LOSS, and both halves are the \
                 decision rather than an oversight. Half one, the collapse: \"which channels are \
                 there, and what for\" is `Engine::directory`, which reads the DECLARATION — the \
                 source of truth for membership since §3. Half two, the loss: the item put it \
                 exactly there (\"bleibt nur, wenn eine App Ungelesen-Zaehler je Kanal zeigen \
                 will\"), and after §3 the surface is THREAD-shaped — `send_to` returns a thread, \
                 `reply_thread` takes one, `status` is a tree of them — so an unread badge on a \
                 channel is a badge on the one thing an app can no longer address. manufakt.io is \
                 being rebuilt against the new surface, which is what makes now the cheap moment. \
                 What an app that still wanted the ack kept at the time: `mark_read`, reachable \
                 through a `MessageView` from `thread`, which carries both `channel_id` and \
                 `message_id`. That ack went on 2026-09-08 (6j6v.4d2z) and took the per-channel \
                 counts off `facade::channels` too, so the NAMED LOSS above is now the whole of \
                 what happened to unread on this read, on both surfaces. \
                 \
                 WHY THE STATE IS STILL `Stays`, and why that is not a fudge: this gate has three \
                 states and none of them says \"partly\". `Leaves` would be a lie — it holds the \
                 symbol ABSENT, and `facade::channels` is on the seam and is what \
                 `tests/seam_reads.rs` drives. `Stays` is the mechanically true one — unlike \
                 `Engine::inbox` above, whose compute half has since gone too. What the state \
                 cannot carry, this reason does; the \
                 gate's own doc calls that limit out (\"names, not behaviour\"). \
                 \
                 The CLI half is unchanged: the verb collapsed into `list` in 6j6v.dvyq (the \
                 declaration carries the lifecycle), and the READ was kept then because §5's \
                 closed list did not name it.",
    },
    Disposition {
        verb: "channels public",
        cli: Gone,
        seam: stays("public_channels"),
        reason: "REMOVED with the rest of the `channels` group — and it is the one row in this \
                 table where the CLI LOSES something rather than handing it over, which is why the \
                 reason says so instead of reading like a collapse. §3's line is `channels \
                 list/public -> list`, and that holds for `list`: `nxc list` reads the DECLARED \
                 team and its channels. It does NOT hold for `public`. This read is the \
                 workspace's DISCOVERY read (6j6v.bd6g) — how you find another PROJECT's front \
                 door — and such a door reaches this workspace by SYNC, carrying no declaration \
                 here for `list` to read. So the two are not the same question, and calling them \
                 one would be exactly the paper mapping acceptance point 1 forbids. \
                 \
                 Decided by the owner on 2026-08-19, with the alternative (teach `list` to read the \
                 store's public channels alongside the declarations) on the table and declined: \
                 cross-project discovery leaves the agent surface, `Engine::public_channels` stays, \
                 and an app renders it. Its coverage moved to \
                 `tests/public_channel.rs::a_non_member_discovers_a_public_channel_through_the_seam` \
                 — a read with no CLI half has no parity partner and needs a gate of its own. \
                 \
                 6j6v.yr59 (owner, 2026-08-21) ABSORBED `Engine::public_channels` INTO \
                 `Engine::directory` — \"`directory` zeigt uebrigens auch die Kanaele an und wofuer \
                 sie da sind\" — and the flat method is gone. This is not the 2026-08-19 decision \
                 reversed by forgetting it: the argument above is exactly WHY the fold had to carry \
                 the store read rather than drop the question, and `Directory::public_channels` is \
                 that store read riding on the record. What a caller gains is that everything \
                 addressable arrives in ONE call; what it loses is the per-caller `unread` the rows \
                 used to be decorated with, which is read state rather than addressability, was \
                 always `0` for the discovering audience, and would have forced an acting handle \
                 onto `directory`'s signature. Named on `persona::FrontDoor`. \
                 \
                 `Stays` about a removed method, this table's usual limit: `facade::public_channels` \
                 is on the seam and the suite above still drives it.",
    },
    Disposition {
        verb: "threads list",
        cli: Standing,
        seam: stays("threads"),
        reason: "§5 names `threads`/`thread_board` as what `nxc status` is MADE OF, and in 6j6v.yr59 \
                 that sentence was taken at its word: `Engine::threads` COLLAPSES INTO \
                 `Engine::status` and is gone from the handle. Decided on its own, as that item \
                 required. \
                 \
                 On the item's own CONDITION — \"sobald `status` je Faden genug traegt\" — which is \
                 why `StatusThread` grew `deadline`, `working_tree` and \
                 `working_tree_queue_position` in the same change rather than later: without them \
                 this would have been a loss wearing a collapse's clothes. The item's own evidence \
                 for going ahead: app-foundations' `ChatClient` interface names `threads` nowhere. \
                 \
                 ONE THING IS DELIBERATELY NOT CARRIED OVER: the MEMBERSHIP SCOPING. `status` is \
                 not membership-scoped and must not be — an operation crosses channels its reader \
                 is not in, and gating on membership answers \"where do we stand\" with a third of \
                 the chain (6j6v.a71h §3.2). The two were never the same list; the fold keeps the \
                 richer question. \
                 \
                 THE VERB STAYS `Standing` and `facade::threads` with it — `nxc threads list` runs \
                 it and `verb_seam.rs` requires the symbol — which is why this row still reads \
                 `Stays`. `parity.rs` keeps the `threads list` differential, driven off the compute \
                 layer now. The seam-level coverage MOVED to \
                 `embed.rs::engine_status_reports_working_tree_holding_and_waiting`.",
    },
    Disposition {
        verb: "threads show",
        cli: Standing,
        seam: stays("thread_board"),
        reason: "The other half of the same sentence in §5: one board with its replies, which is \
                 what an app renders when a human opens a thread. 6j6v.yr59 removes \
                 `Engine::thread_board` from the handle, decided on its own beside `threads`. \
                 \
                 It SPLITS rather than collapses, and the cost is said out loud: the quorum half is \
                 a `StatusThread` off `Engine::status`, the message half is `Engine::thread` — TWO \
                 calls where there was one. What makes the split safe rather than merely cheaper is \
                 that the `visibility` POLICY moved with the messages: `Engine::thread` now \
                 resolves the thread's declared channel policy and applies it, exactly as this \
                 method did. Had it not, a cut sold as a consolidation would have WIDENED what a \
                 non-requester may read through the reader that stayed — which is the one thing a \
                 consolidation must not do. Pinned by \
                 `embed.rs::the_one_message_reader_enforces_a_declared_channels_requester_only_\
                 visibility` (the same regression this method's own fix round was written for, \
                 moved onto the survivor) and by \
                 `channel_visibility.rs::the_one_message_reader_applies_the_channels_declared_\
                 visibility`. \
                 \
                 `Stays` because `facade::thread_board` is on the seam: `nxc threads show` runs it, \
                 `verb_seam.rs` requires it, and `parity.rs` still drives its differential.",
    },
    Disposition {
        verb: "threads expect",
        cli: Gone,
        seam: stays("set_expects"),
        reason: "REMOVED in 6j6v.cg8g's fix round under the standing owner directive of \
                 2026-08-14 (owner's reason: \"sehe ich nicht, wofuer das notwendig ist\"). The \
                 WRITE deliberately stayed: 6j6v.hq71's channel supervisor re-declares a role's \
                 next turn through `facade::set_expects`, so removing it would stop the declared \
                 flow rather than a human-typed door. This is the shape the whole table is about — \
                 the entrance went, the capability did not.",
    },
    Disposition {
        verb: "workflow status",
        cli: Gone,
        seam: left("workflow_status"),
        reason: "THE ONE ROW WHERE §5's CLOSED LIST WAS NOT FOLLOWED, and it is a decision rather \
                 than an oversight (owner, 2026-08-20). The verb collapses into `nxc status` \
                 (6j6v.a71h). The READ is not on §5's list, and until this block that is why it \
                 stayed — the tension with acceptance point 4 was raised in the dvyq report rather \
                 than settled. \
                 \
                 What settled it is CONSTRUCTION, not a change of mind about the rule. §5's rule \
                 protects a read an APP RENDERS. `workflow::start_run` had exactly two callers, the \
                 CLI verb and `Engine::workflow_start`, and a declared channel's flow opens no run \
                 at all (`channel_supervisor.rs::a_send_to_a_channel_starts_the_whole_flow_and_\
                 it_is_the_only_way_in`, renamed from `…_with_no_workflow_run_behind_it` in this \
                 same cut, since the run record it named is gone). With both entrances gone nothing \
                 can ever write a \
                 `workflow_runs` row again, so this read renders a table that can never be filled. \
                 The rule does not apply to it; it was not waived for it. \
                 \
                 The tables are DROPPED on the next open rather than left standing (`schema.rs`), \
                 which is what makes the sentence above a fact about the database.",
    },
    Disposition {
        verb: "workflow list",
        cli: Gone,
        seam: left("workflow_list"),
        reason: "Same as `workflow status` beside it, same owner decision, same construction: the \
                 verb collapses into `nxc status`, and the read enumerates a table nothing can \
                 write. It is the read that needed no run id and no session, which is exactly what \
                 `nxc status` with no argument is now.",
    },
    Disposition {
        verb: "send --model",
        cli: Gone,
        seam: stays("send_to"),
        reason: "REMOVED in 6j6v.e9qj (owner: \"vielleicht nehmen wir es spaeter wieder rein\") \
                 because e9qj moved \"which model\" onto the DECLARATION, and a per-call override \
                 beside a declared value is two answers to one question. The FLAG went; the SEAM \
                 did not — `SendToRequest`/`Commission`/`RoleResumeRequest`/ \
                 `ChannelOpenRequest` all still carry `model`, and an embedding app still chooses \
                 per call. What is gone is the way an AGENT typed it.",
    },
    // ---- nxf 6j6v.ckeq: the writing seam cut to two verbs and three statements each -----------
    //
    // Owner, 2026-08-21, verb by verb at the source: "`send_to` und `reply_thread` sollen der
    // einzige Weg der Kommunikation sein. Es startet immer auch die jeweilige Sitzung bzw. weckt sie
    // wieder auf." Two of the four writing verbs go, and with them nine of the eleven caller
    // options.
    //
    // THE TWO METHOD ROWS BELOW READ `Stays` AND THE METHODS ARE GONE, which looks like a
    // contradiction and is the gate's own documented limit ("names, not behaviour"): it compares
    // symbol NAMES across `engine.rs` + `facade.rs`, and `facade::send`/`facade::reply` are still on
    // that union — deliberately, as the WRITES every post in this crate is built out of. `ask` set
    // this precedent in the block above and for the same reason. Recording `Leaves` here would claim
    // a removal that did not happen; the reasons say what did.
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("send"),
        reason: "`Engine::send` — the RAW post into a substrate channel id, which woke nobody — is \
                 REMOVED (6j6v.ckeq, decision 1). It had been off the command line since 6j6v.dvyq \
                 §3 and survived only here, which is the shape the cut exists to end: an entrance \
                 an app can reach and an agent cannot is a second contract nobody reviews. \
                 \
                 A NAMED LOSS, not a collapse, and the loss is the point. `send_to` is not a \
                 renaming of it: it takes a DECLARED target and both its branches start or wake a \
                 session, which is exactly what the owner's sentence demands and exactly what the \
                 raw post could not do. A caller holding a bare channel id has no path left — and \
                 since 6j6v.dvyq §3 such a channel is not addressable at all (`send_to` refuses one \
                 by name), so the entrance was already writing into a namespace the rest of the \
                 surface had stopped recognising. \
                 \
                 THE WRITE STAYS: `facade::send` is what `orchestration` posts through in three \
                 places, so the symbol this gate compares against is still on the seam. What left \
                 is the flat handle method, not the mechanism.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("reply"),
        reason: "`Engine::reply` — a reply to a message-OR-thread `target` — is REMOVED (6j6v.ckeq, \
                 decision 1), off the command line since 6j6v.dvyq §3 for the same reason its \
                 sibling was. \
                 \
                 It COLLAPSES into `Engine::reply_thread` on the thread half, and the collapse is \
                 total: `reply_thread` calls `orchestration::reply` with the thread as its target, \
                 so the quorum-completion routing, a declared channel's `on_complete` policy, the \
                 depth guard and the return-address resume are the same code on the same path. \
                 \
                 WHAT IS LOST is the MESSAGE half — `target` also took a message id and resolved it \
                 to that message's thread. An app holding one reads its thread (`Engine::messages` \
                 and `thread_board` both carry it) and replies into that: one read. Named rather \
                 than papered over, and the alternative was keeping a second address for one \
                 conversation — the shape 6j6v.dvyq §3 spent a whole block removing from the CLI. \
                 One test moved rather than died with it: `orchestration_reply.rs::a_non_dm_thread_\
                 with_no_expects_still_resumes_the_return_address_directly`, whose regression is \
                 visible only on a message target, now drives `orchestration::reply` directly. \
                 \
                 THE WRITE STAYS: `facade::reply` is what `orchestration::reply` posts through, and \
                 the consolidation claim calls it too.",
    },
    Disposition {
        verb: "send --kind",
        cli: Gone,
        seam: stays("send_to"),
        reason: "REMOVED as a CALLER option (6j6v.ckeq, decision 3). Owner: \"interessante \
                 Features, die wir zu einem spaeteren Zeitpunkt bei Bedarf zurueckholen, aber dann \
                 mit mehr Wissen ueber den tatsaechlichen Bedarf.\" The FIELD went from \
                 `SendToRequest` too — this is not a flag-only cut, so an app cannot set what an \
                 agent may not — while `MessageKind` itself, `ChannelOpenRequest::kind` and \
                 `Commission::kind` are untouched and `send_to` passes `MessageKind::Info`, \
                 the default every caller that said nothing already got. \
                 \
                 `kind` REMAINS AS A CARRIER, which is the half that would be easy to lose: \
                 `reply --escalate` IS `kind: escalation`, `ChatStore::last_reply_escalated` reads \
                 that column, and 6j6v.1xw1's claim rule and 6j6v.e9qj's fold rule both branch on \
                 it. Exactly two of the five values stay in use — pinned by \
                 `writing_seam.rs::the_escalation_note_is_the_whole_of_what_kind_still_carries_at_\
                 the_seam`.",
    },
    Disposition {
        verb: "send --priority",
        cli: Gone,
        seam: stays("send_to"),
        reason: "REMOVED as a CALLER option with `--kind` and `--disposition`, same owner sentence \
                 (6j6v.ckeq, decision 3), and this is the one of the three with a CONSEQUENCE \
                 BEYOND THE LABEL. `priority` is the first component of the working-tree queue's \
                 ordering key (`working_tree.rs`: `(priority, enqueued_at, id)`), so with every \
                 entry carrying `Priority::Normal` the key degenerates and the queue is pure FIFO. \
                 \
                 ACCEPTED EXPLICITLY rather than overlooked. Owner: \"Reines FIFO ist erst einmal \
                 okay. Wir werden das nochmal anpacken muessen, aber ich will erst die Probleme \
                 sammeln, die durch mangelnde Prio entstehen.\" 6j6v.xr3z is where the lever comes \
                 back, with the problems collected. The COLUMN and the `ORDER BY` stay, so bringing \
                 it back is re-opening an entrance rather than rebuilding a mechanism. \
                 \
                 Held as a decided state and not as an accident by \
                 `surface_cli.rs::the_working_tree_queue_is_fifo_and_nothing_on_the_surface_can_\
                 reorder_it`, which asserts BOTH halves: three summons drain in arrival order, AND \
                 the flag that could have asked for anything else is refused by name.",
    },
    Disposition {
        verb: "send --disposition",
        cli: Gone,
        seam: stays("send_to"),
        reason: "REMOVED as a CALLER option, same owner sentence as `--kind`/`--priority` \
                 (6j6v.ckeq, decision 3). `send_to` passes `Disposition::InTurn`, the default a \
                 caller that said nothing already got, so what a bare `send --to` writes is \
                 unchanged. The field survives on `Commission` and on the envelope, and \
                 `Engine::inbox`'s two-disposition split (`--all`) was the READ that consumed it, \
                 untouched here because nothing about reading a catch-up depended on a sender being \
                 able to choose — and itself removed by 6j6v.4d2z. The field is still WRITTEN and \
                 still travels; what is gone is the only read that ever branched on it.",
    },
    Disposition {
        verb: "send --deadline",
        cli: Gone,
        seam: stays("send_to"),
        reason: "REMOVED (6j6v.ckeq, decision 2): when a declared channel's board must be answered \
                 by is the CHANNEL's own `timeout:`, and a per-call override beside a declared value \
                 is two answers to one question — owner: \"sonst wird ein Agent vielleicht \
                 uebereifrig Optionen aendern.\" The field went from `SendToRequest` with the flag; \
                 `ChannelOpenRequest::deadline` survives and `send_to` passes `None`, so there is no \
                 app-only door either. \
                 \
                 NOTHING IS LOST, and it is worth stating because this flag's own arrival note \
                 (6j6v.dvyq §3, moving it off `ask`) argued the opposite: \"a duration has a \
                 declared form (`timeout:`), an absolute instant has none\". That was already \
                 untrue. `timeout:` is parsed by the SAME `facade::resolve_deadline_spec`, which \
                 tries RFC3339 first — so `timeout: 2026-08-16T10:30:00Z` declares a \
                 non-resettable instant exactly as `--deadline` did. Proven by \
                 `verbs.rs::a_declared_timeout_resolves_a_duration_and_an_instant_alike_and_rejects_\
                 bad_input` and by `channel_member_timeout.rs`'s absolute-instant case, both of \
                 which now declare what they used to pass.",
    },
    Disposition {
        verb: "reply --kind",
        cli: Gone,
        seam: stays("reply_thread"),
        reason: "REMOVED with `send --kind` (6j6v.ckeq, decision 3) and replaced on the request by \
                 ONE BOOLEAN, `ReplyThreadRequest::escalate`. That is the shape the flag effectively \
                 had: `escalation` was deliberately absent from its value list, `--escalate` was the \
                 one door to the marker and `conflicts_with` refused asking for both — so five \
                 values were really one bit plus four labels nothing in this engine read. \
                 `surface::reply_kind` is the single mapping both surfaces run through, so they \
                 cannot drift. \
                 \
                 ONE VALUE DID SOMETHING AND IS NOW UNREACHABLE, and it is a named LOSS: \
                 `--kind question`. The owner's ruling of 2026-08-16 (6j6v.1xw1) makes an open \
                 QUESTION hold the working-tree claim exactly as an escalation does, and \
                 `working_tree::hands_the_task_back` still branches on it — but no caller can \
                 produce one. For the CLAIM the difference was never a difference (in both shapes \
                 nobody is working and the task is mid-flight); for the RECORD it was. The branch \
                 keeps its coverage: `working_tree_claim_scope.rs`'s three question cases call \
                 `orchestration::reply` directly through a helper that says exactly why.",
    },
    Disposition {
        verb: "reply --priority",
        cli: Gone,
        seam: stays("reply_thread"),
        reason: "REMOVED with `send --priority`, same owner sentence (6j6v.ckeq, decision 3). It \
                 has no queue consequence on this verb — a reply enqueues nothing — so the FIFO \
                 argument recorded on `send --priority` is the whole of that story and this row is \
                 the label half of it.",
    },
    Disposition {
        verb: "reply --disposition",
        cli: Gone,
        seam: stays("reply_thread"),
        reason: "REMOVED with `send --disposition`, same owner sentence (6j6v.ckeq, decision 3). \
                 `reply_in_thread` passes `Disposition::InTurn`, which is what an answer is: the \
                 turn is being handed back, so \"catch up later\" was never the honest value for it.",
    },
    Disposition {
        verb: "reply --ref",
        cli: Gone,
        seam: stays("reply_thread"),
        reason: "REMOVED (6j6v.ckeq): a reply INHERITS its subject. The thread already says what the \
                 conversation is about, which is precisely why the item put the refs OBLIGATION on \
                 `send --to` and nowhere else — owner: \"jede ERSTE Nachricht bezieht sich \
                 wahrscheinlich auf ein Ticket.\" The DoD's enumeration of what `reply_thread` \
                 carries (`thread`, `body`, the escalation note) is what settles it. \
                 \
                 NOTHING THAT MADE A CHAIN RESUMABLE DEPENDED ON IT, which was the one real risk: \
                 the return address is stamped by `orchestration::reply`'s `with_return_address` \
                 from the caller's own ambient session, never read off a caller-supplied \
                 `--ref session_id=`. The persona prompts in `agent-sidecar/scripts/skeleton.sh` \
                 still passed one by hand and no longer need to; they were updated in the same \
                 change. `verbs.rs::reply_ambient_stamps_its_own_return_address` is the surviving \
                 half of the pair that pinned the precedence, and it covers the only path left.",
    },
    Disposition {
        verb: "reply --if-unanswered",
        cli: Standing,
        seam: stays("reply_thread"),
        reason: "THE ONE ROW IN THIS TABLE THAT RUNS THE OTHER WAY: the FLAG stays and the SEAM \
                 FIELD goes (6j6v.ckeq, decision 4). `ReplyThreadRequest::if_unanswered` is removed; \
                 `nxc reply --if-unanswered` is not. \
                 \
                 Because it was never an agent's option or an app's. It is the IDEMPOTENCY SWITCH \
                 of exactly one caller: `agent-sidecar/src/main.mjs`'s teardown (6j6v.ww0a) runs \
                 `nxc reply --thread <id> --if-unanswered <text>` when an SDK session ends without \
                 ever answering the thread it owed, so the thread does not go quiet forever — and it \
                 runs it unconditionally, because it cannot tell whether the session already \
                 answered. An app holds its own sessions and knows. \
                 \
                 The mechanism keeps a NAME of its own rather than a boolean every app fills in: \
                 `surface::settle_if_unanswered`, a second entrance onto the one shared body, \
                 reachable from the CLI and absent from `Engine`. Two Engine-seam tests went with \
                 the field (`embed_surface.rs`, `embed_orchestration.rs`) because there is no app \
                 call left to pin; the same chained pm-to-coder fixture keeps the regression they \
                 guarded — a SKIP must not resume a return address with a reply that was never \
                 written — in `surface_cli.rs`'s own `reply_thread_if_unanswered_*` pair, and the \
                 teardown's exact invocation is 6j6v.ckeq's acceptance point \
                 `surface_cli.rs::the_sidecar_teardown_still_writes_its_failure_reply_when_a_\
                 session_ends_unanswered`.",
    },
    Disposition {
        verb: "send --thread",
        cli: Gone,
        seam: stays("send"),
        reason: "REMOVED. The ENTRANCE collapses into `reply --thread` (§3) — and `send --to` is \
                 how a thread comes into being at all, minting the id rather than taking one, \
                 which is what `--thread` on a fresh `send` could never do honestly: it accepted \
                 an id no opener had ever stamped. The seam does not move: `facade::send`'s \
                 `SendRequest` still carries `thread`, every threaded post an app makes goes \
                 through it, and so does `reply --thread` itself.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("definitions"),
        reason: "THE NAMED ENTRY app-foundations asked for (note of 2026-08-14, §2). The READ of \
                 the catalogue must stay while the WRITE goes: the app still reads roles and \
                 channels to render them, it merely no longer supplies them. \"Werden Lesung und \
                 Schreibung beim Aufraeumen zusammen gegriffen, bricht die App an einer Stelle, \
                 die die CLI nicht sieht\" — so it is held by this gate rather than by care.",
    },
    // ---- nxf 6j6v.yr59: the READING seam cut to seven verbs -----------------------------------
    //
    // Decided in the seam pass with the owner on 2026-08-21, verb by verb at the source, as its
    // predecessors 6j6v.ckeq (writes) and 6j6v.07me (the caller) were. Sixteen read verbs stood on
    // `Engine`; seven carry the work — `status`, `directory`, `prime_as`, ONE conversation reader,
    // `transcript_page`, `subscribe`, and `search`, which the item never covered and the owner
    // KEPT on 2026-08-21 over this build's removal of it.
    //
    // Four of the removals have a row above, because their entrance already had one (`inbox`,
    // `channels list`, `channels public`, `threads list`/`threads show`). What follows is the rest:
    // two reads whose CLI verb is untouched, three that never had one, and the third constructor —
    // plus the `search` row, which opens the block and is the one row here that records a read
    // STAYING ON THE HANDLE rather than leaving it.
    //
    // Nearly all of them read `Stays`, and for the reason the block above `Engine::send` states:
    // this gate compares symbol NAMES across `engine.rs` + `facade.rs`, the CLI verbs that reach
    // these bodies are still verbs, and `verb_seam.rs` requires the bodies to exist. What left is
    // the FLAT HANDLE METHOD an app reached for. The one gate that can see the handle by itself is
    // `tests/read_surface.rs`, which holds the SEVEN and names each of these ten as gone.
    Disposition {
        verb: "search",
        cli: Standing,
        seam: stays("search"),
        reason: "THE ITEM DID NOT COVER THIS ONE, AND THE OWNER RULED IT STAYS. Both halves belong \
                 in the record, because the second overturns the first and a row carrying only one \
                 of them would be a false map either way round. \
                 \
                 THE CUT'S ARGUMENT, which is not withdrawn: `search` appears in neither yr59's \
                 keep table nor its \"what goes\" list, and the DoD does not name it among the five \
                 to decide individually — yet it was on the surface, so this build decided it in \
                 the item's own terms and removed `Engine::search` as a NAMED LOSS. It answers none \
                 of the six questions: not where we stand, not who may be addressed, not what was \
                 said in THIS conversation, not one session's history, not session start, not \
                 liveness. It is discovery-by-text, and its scoping is the channel-shaped one \
                 `channels`/`inbox` are leaving for. The gap in the item was REPORTED, not glossed. \
                 \
                 THE OWNER OVERRULED IT ON 2026-08-21, on a fact the shape argument cannot see: \
                 app-foundations' `ChatClient` (`packages/engine-client/src/chat-client.ts`) \
                 carries `search` TODAY. Measured that day against the seam, 14 of its 28 methods \
                 no longer exist here — and `search` is NOT one of them. It is a LIVE, CONSUMED \
                 method, so removing it is not clearing away a corpse; it is a break, inside the \
                 very item whose stated purpose is to cost app-foundations ONE migration instead \
                 of two. That is the trap §5 of 6j6v.dvyq names and the reason this table exists. \
                 A measured obligation to the consumer outranks the shape of the surface. \
                 \
                 THE REJECTED ALTERNATIVE, stated so the next reader does not re-open it: remove \
                 it now and let app-foundations re-add substring search over the store itself, or \
                 wait for it to come back \"deliberately, and probably not as a `LIKE`\". Both cost \
                 the consumer a second migration for a method it is already calling, and neither \
                 buys anything but a rounder number in the item's title. The number is what gave \
                 way: the read seam is SEVEN verbs. \
                 \
                 SO NOTHING MOVES ON EITHER HALF. `Engine::search` is on the handle, unchanged — a \
                 `try_with_state` forwarder to `facade::search` — and `nxc search` stays \
                 `Standing`. NOT as a human-only verb, unlike `inbox` and `read` above: `prime`'s \
                 command reference teaches `nxc search` to every agent it primes (it is in \
                 `tests/prime_golden.rs`'s golden block, where those two are not), so this verb \
                 gave up no surface at all — agent, human and app all keep it. `parity.rs` drives \
                 the differential from the HANDLE again, which is the point of a differential: two \
                 seams, compared. `tests/read_surface.rs` counts seven and no longer names this \
                 method as gone. \
                 \
                 THE ONE RESIDUAL WAS NAMED HERE RATHER THAN LEFT TO BE DISCOVERED, AND IT IS \
                 CLOSED (nxf 6j6v.px98, 2026-09-05). This read was MEMBERSHIP-scoped and applied \
                 no declared `visibility`, while `Engine::thread` gained exactly that policy in \
                 this cut — so on a `requester_only` board `search` returned a body `thread` \
                 filtered out for the same caller. Unchanged behaviour, not new, which is why it \
                 was recorded and not treated as a regression of this cut; the owner then ruled \
                 (2026-08-23) that `visibility` is an ACCESS rule, which made the wider answer a \
                 broken promise rather than an inconsistency. `facade::search` now applies the \
                 same policy through the same `filter_board_messages` semantics, resolved from \
                 the same catalogue, and `nxc search` reads THROUGH the facade instead of \
                 straight at the store so the two surfaces cannot drift apart again. Nothing \
                 about this row's disposition moved: both halves stay, and the differential in \
                 `parity.rs` compares them under the policy now. \
                 \
                 AND THAT FIX NAMED ITS OWN RESIDUAL, WHICH IS ALSO CLOSED (nxf 6j6v.cs03, \
                 2026-09-05): the two readers agreed about `visibility` and still gave two \
                 answers to the question under it — am I in this channel at all. This read \
                 joined the substrate's materialised member set, so a handle struck from a \
                 declaration kept finding bodies, a handle just written into one found none \
                 until the next `send`, and on a declared channel this answered \"no matches\" \
                 to every member that had not itself opened a round there, including for its \
                 own words — measured in a live run, and the shape a user met first. Membership \
                 is `facade::is_channel_member` for both readers now. Still nothing moves on \
                 this row: what changed is which channels the read is scoped to, not whether \
                 either half stands.",
    },
    Disposition {
        verb: "withdraw",
        cli: Standing,
        seam: stays("withdraw"),
        reason: "THE ONE ROW HERE THAT RECORDS AN ARRIVAL RATHER THAN A DEPARTURE, and it is here \
                 because `read_surface.rs` sent it: widening the write seam is a decision that \
                 gate refuses to let anyone take quietly, and this table is where it told the \
                 taker to write it down. nxf 6j6v.0djn is the decision item. \
                 \
                 WHY THIS ROW DOES NOT ARGUE THE ADMISSION, since nxf 6j6v.12nn: the write seam \
                 now has a RULE — a call may reach it when it cannot cause work that was not \
                 already commissioned — and `read_surface.rs` holds this method against it, with \
                 the source and not the row deciding whether it starts anything. Two places \
                 arguing one decision is how they drift apart (6j6v.65zg), and this one had the \
                 weaker copy: it said \"starts nothing\" as an aside, which for the sibling write \
                 admitted on the same words turned out to be false. What belongs here is the \
                 DISPOSITION — the verb stands, the method is on the handle, the waiver fell — and \
                 that is what is below. \
                 \
                 `Engine::withdraw` was ADDED (2026-08-23) as a thin `with_orchestration` \
                 forwarding onto `orchestration::withdraw`, and the `Waiver::CliOnly` that stood \
                 in `verb_seam.rs` fell with it. The waiver's premise — \"a device-local queue \
                 over one checkout, which is exactly the thing an embedding app does not have\" — \
                 was measured and is FALSE: `working_tree: exclusive` is a DECLARATION, the lease \
                 gate that reads it sits in `orchestration::trigger_role`, the one trigger funnel \
                 `Engine::send_to` passes through as much as `nxc send --to` does, and \
                 `embed_surface.rs::engine_send_to_a_busy_exclusive_persona_reports_the_queue_instead_of_a_started_session` \
                 reaches `queue_position: Some(1)` through the handle with no `nxc` process \
                 anywhere in it. \
                 \
                 §5's trap runs the OTHER WAY here, which is why it is worth a row: the usual \
                 danger is a read an app renders being cut with the verb an agent typed. This was \
                 a WRITE an app could not reach while the seam handed it the very receipt \
                 (`SendToReceipt::queue_position`) that says the write is called for — a state \
                 announced and not actionable.",
    },
    Disposition {
        verb: "prime",
        cli: Standing,
        seam: stays("prime"),
        reason: "`Engine::prime` is REMOVED as the PURE OVERLOAD it was: its whole body was \
                 `self.prime_as(consumer, None, now)`, and `None` is the human at the keyboard. \
                 Nothing was decided here because there was nothing to decide — the item lists it \
                 under \"sichere, reine Ueberladungen\" for that reason. \
                 \
                 The VERB is untouched (hidden from `--help` since 6j6v.dvyq, invoked by the `nxs \
                 prime` SessionStart hook), and `facade::prime`/`prime_for` are what it runs. \
                 `parity.rs`'s `prime --json` + rendered-Markdown differential now drives \
                 `prime_as(handle, None, NOW)` and is otherwise unchanged.",
    },
    Disposition {
        verb: "transcript show",
        cli: Standing,
        seam: stays("transcript"),
        reason: "`Engine::transcript` is REMOVED as a PURE OVERLOAD: `transcript_page(session, -1, \
                 None)` is the same value, which its own doc already said (\"`transcript` itself \
                 now delegates to it\"). The whole session is still reachable by asking for it, \
                 which is the honest form — what it materialises is unbounded `tool_use` inputs, \
                 and that is why the BOUNDED read is the one that stayed rather than the other way \
                 round. \
                 \
                 The VERB is untouched, `--from-seq`/`--limit` included, and `facade::transcript` \
                 is what `verb_seam.rs` waives it onto — hence `Stays`. Recorded although \
                 acceptance point 3's literal wording is about verbs REMOVED from the CLI: a seam \
                 read that disappears while its verb stands is precisely the direction §5 warns \
                 about, and leaving it unrecorded would make this table quieter than the change.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("messages"),
        reason: "`Engine::messages` — all the messages in a CHANNEL — is REMOVED, and this is \
                 yr59's \"exactly ONE message reader\" decided: `Engine::thread` survives, \
                 `messages` does not. Never a CLI entrance; `nxc` has no channel-read verb. \
                 \
                 THE ARGUMENT IS THE KEY, not the payload. A thread id is the value `send_to` hands \
                 back (\"the only value the caller has to keep\"), the only value `reply_thread` \
                 takes, and what `status` speaks in; a channel id is a value no surviving verb \
                 hands out. And the GRAIN moved under it: since 6j6v.pf6j a declared channel holds \
                 the requester's board and every member's own thread side by side, so \"the \
                 messages in this channel\" is several conversations interleaved where \"the \
                 messages in this thread\" is one. `ThreadView` also carries `channel_id` and \
                 `opener`, which `Vec<MessageView>` did not. \
                 \
                 WHAT IS LOST: reading a whole channel's flat history in ONE call — a public front \
                 door's, for instance. The route is `status` with `StatusScope::Channel` for the \
                 thread ids and then the reader per thread; what goes is the single call, not the \
                 capability. \
                 \
                 `Stays` because `facade::messages` is on the seam and is what `cli.rs` and the \
                 suites read a channel with (`tests/common`'s `channel_messages`).",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("thread_quorums"),
        reason: "`Engine::thread_quorums` — the bulk quorum read for an EXPLICIT SET of thread ids \
                 — is no longer a VERB. It is a PARAMETER of its neighbour: `StatusScope::Threads`. \
                 The item's ruling in its own words: \"die Anti-N+1-Eigenschaft muss bleiben, der \
                 zweite Name nicht.\" \
                 \
                 The neighbour is `status` rather than `threads`, because `threads` left in the \
                 same cut — the item's own \"gehen in `status` auf\" decides which of the two the \
                 set attaches to. `StatusScope::Thread(&str)` was REPLACED by \
                 `Threads(&[&str])` rather than joined by it: keeping both would be the second \
                 spelling this item exists to remove, and the single-id form is the one-element \
                 case, `not_found` behaviour included. \
                 \
                 THE PROPERTY IS MEASURED, NOT ASSERTED, which is the acceptance point: \
                 `tests/bulk_quorum.rs::a_set_of_threads_costs_the_same_number_of_queries_however_\
                 many_are_in_it` counts the SELECT statements SQLite PREPARES for a two-thread read \
                 and a forty-thread read and fails if the number moves. `facade::thread_quorums` \
                 stays on the seam (it is the passthrough `ChatStore::thread_quorums` is reached \
                 through), hence `Stays`.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("opener_wake"),
        reason: "`Engine::opener_wake` is REMOVED, and it is the least contested of the eleven: by \
                 its OWN doc it was \"the SAME derivation `nxc inbox`/`prime` renders as 'Threads \
                 you opened'\", and `PrimeReport::wake` carries it verbatim. Two names for one \
                 derivation is the whole subject of this item. `Engine::status` answers the same \
                 question from the other end, per operation. \
                 \
                 Never a CLI entrance of its own — `nxc prime` renders it — and `facade::opener_wake` \
                 stays as what both `cli.rs` and `facade::prime_for` call, hence `Stays`. \
                 `parity.rs` still pins that the report's wake IS this derivation rather than a \
                 second one.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: left("open_with_poll_interval"),
        reason: "NOT A READ, and the only row in this block that reads `Leaves` — because it is the \
                 only one with no `facade::` twin to keep its name on the seam. A third \
                 CONSTRUCTOR beside `open`/`open_with`, carrying ONE value: the change-watcher's \
                 poll interval, a documented test seam. \
                 \
                 It is `EngineConfig::poll_interval` now. `EngineConfig` existed precisely to carry \
                 this kind of choice, and the constructor form had a real defect beyond the count: \
                 a caller who wanted a worker AND a fast cadence could not have both, since \
                 `open_with_poll_interval` hardcoded `EngineConfig::default()`. \
                 \
                 The two `tests/embed.rs` call sites moved to the field in the same change. \
                 `crates/facade` and `crates/memory` keep their own same-named constructors, \
                 untouched: this row is about chat's handle, and this gate reads chat's sources.",
    },
    // ---- what really falls off `Engine` (§5's closed list) ------------------------------------
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: left("set_definitions"),
        reason: "REMOVED in step 6 of this item. The condition attached on 2026-08-13 was \
                 discharged on 2026-08-14: app-foundations depends on it, named its six call \
                 sites, and released it anyway — owner decision obtained specifically for this \
                 point. Declarations come from `.nxs-personas/` and are not codified in the app; \
                 even a later in-app role editor writes into that folder. The teardown on their \
                 side is their work.",
    },
    Disposition {
        verb: "send --role",
        cli: Gone,
        seam: left("role_trigger"),
        reason:
            "REMOVED on both surfaces at once. The entrance COLLAPSES into `send --to <persona>` \
                 (§3): `send_to`'s persona branch is the one door and runs the same \
                 `orchestration::coordinator_commission` body. The proof is not a paper mapping \
                 — since \
                 §3 took `send`'s positional channel, `--role` could no longer name one either, so \
                 both forms opened the caller-to-role DM themselves and differed only in what they \
                 left behind: `--to` stamps a THREAD and tells the persona which one to answer \
                 into, `--role` posted threadlessly. With `reply --thread` the one surviving reply \
                 form, a threadless trigger produced answers nothing could address. §5's closed \
                 list names `role_trigger` among what leaves `Engine`, and it did. \
                 \
                 THE BODY STAYS, exactly as for `channel_open`/`ask`: \
                 `orchestration::coordinator_commission` (named `role_trigger`/`role_trigger_with` \
                 until nxf 6j6v.ntp9) is what every persona summon still runs on. What left is \
                 the flat handle method, not the mechanism.",
    },
    Disposition {
        verb: "send --session",
        cli: Gone,
        seam: left("role_resume"),
        reason:
            "REMOVED on both surfaces, and — unlike every other row here — WITHOUT A SUCCESSOR. §3 \
                 writes `send --session -> reply --thread`, and for this line that is no \
                 construction proof (owner, 2026-08-19): a resume COMMISSIONS an existing session \
                 (`ChainMove::Deeper`, registering no obligation), a reply ANSWERS one \
                 (`ChainMove::Unwind`, through the thread's return address). Concretely, \
                 `surface::thread_return_address` reads the newest message from somebody ELSE \
                 carrying a session id, so a persona summoned by `send --to` that has not answered \
                 yet posts none and a follow-up wakes nobody. \
                 \
                 Recorded as a named LOSS rather than a collapse, the same shape `channels public` \
                 took in this item's first block — the CLI gives something up here instead of \
                 handing it to a survivor, and a row that read like a collapse would be a false \
                 map. The residual case, delivering into a session that is mid-turn, is nxf \
                 6j6v.xr3z's `reply --thread <id> --force`; it waits on a runtime signal that does \
                 not exist yet (`worker.rs`: \"Teardown is deliberately absent\"). §5's closed list \
                 names `role_resume`, and the body stays for the same reason `role_trigger`'s does.",
    },
    Disposition {
        verb: "ask",
        cli: Gone,
        seam: stays("ask"),
        reason: "REMOVED from the CLI: collapses into `send --to <channel>` (§3), whose channel \
                 branch runs the declared channel's own `expects`/`timeout`/fan-out policy. §5's \
                 closed list names `ask` among what leaves `Engine`, and it did — \
                 `Engine::ask` and the router beneath it (`orchestration::ask`, which chose \
                 between a declared channel and a raw one) are both gone. \
                 \
                 The row says `Stays` because this gate compares NAMES across the whole declared \
                 seam, and `facade::ask` is still on it — deliberately: it is the WRITE (mint the \
                 thread, declare its expects, stamp the deadline, post the request) that \
                 `open_declared_channel_and_fan_out` and the consolidation claim are built out of, \
                 so a `send --to <channel>` runs it on every call. What left is the flat handle \
                 method and the routing decision; the write did not, and recording `Leaves` here \
                 would claim a removal that did not happen. The gate's own doc names this limit \
                 (`names, not behaviour`); this is the row where it bites. \
                 \
                 ONE THING THE COLLAPSE SILENTLY CHANGED, named here rather than left to be found \
                 (PR review of block b, §2): `ask --kind` defaulted to `question`, `send --kind` \
                 defaults to `info`, so a bare `nxc ask <channel> <body>` opened a board labelled \
                 `question` and its successor labels it `info`. DECIDED as a named loss, not \
                 restored: `ask` could default that way because it only ever addressed a CHANNEL, \
                 while `send --to` addresses a persona OR a channel — a default that varied by what \
                 the target turned out to be would put back exactly the shape §3 removed, a flag \
                 whose meaning depends on a resolution the caller cannot see. A caller who wants \
                 the old label types `--kind question`. The consequence is confined to the label: \
                 the only functional reader of `MessageKind` is \
                 `working_tree::hands_the_task_back`, which reads REPLY kinds via `last_reply_kind` \
                 and never the opening message's. VON MIR ENTSCHIEDEN, ueberstimmbar.",
    },
    Disposition {
        verb: "ask --expect",
        cli: Gone,
        seam: left("channel_open"),
        reason:
            "Falls with the RAW CHANNELS (§3, owner 2026-08-13: a channel no declaration names \
                 is something addressable without policy — exactly the state this rebuild \
                 abolishes). With every channel declared, who must answer is written in the \
                 channel, so a per-call override is a second answer to a declared question. \
                 `send --to` already refuses to take one.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: left("channel_open"),
        reason:
            "On §5's closed list. It has no CLI entrance of its own — `ask`/`send --to <channel>` \
                 are how it is reached — so it is recorded here by symbol. The BODY \
                 (`orchestration::channel_open`) stays and is what `Engine::send_to`'s channel \
                 branch runs; what leaves is the flat method on the handle.",
    },
    Disposition {
        verb: "workflow start",
        cli: Gone,
        seam: left("workflow_start"),
        reason: "REMOVED on both surfaces at once, together with the `workflow_runs` record, \
                 `WORKFLOW_RUN_ADDR_PREFIX` and the `outcome:` token path — which is what \
                 `orchestration.rs` said Task 10 would take, and it took them as one because a \
                 half-removed run engine is a mechanism nobody can start and nobody can read. \
                 \
                 Falls with \"channel = flow\" (6j6v.hq71, delivered): a channel declares its own \
                 flow, so `send --to <channel>` IS starting one. The construction proof is \
                 `consolidated_entrances.rs`, and the differential that made it one — the SAME \
                 fixed order driven through both surfaces, asserting the same sessions started in \
                 the same order — was \
                 `channel_flow.rs::the_same_fixed_order_runs_through_send_to_a_channel_and_through_\
                 workflow_start_alike`, whose surviving half is now \
                 `a_declared_flow_runs_the_fixed_order_that_used_to_need_a_run`. On §5's closed \
                 list.",
    },
    Disposition {
        verb: "workflow step done",
        cli: Gone,
        seam: left("workflow_step_done"),
        reason: "REMOVED. Collapses into `reply --thread` — the channel's SUPERVISOR decides what \
                 comes next (6j6v.pf6j/6j6v.hq71), so a step does not announce its own completion, \
                 and the one thing it could say that the supervisor cannot work out for itself is \
                 `reply --escalate`. On §5's closed list.",
    },
    Disposition {
        verb: "workflow step done --outcome",
        cli: Gone,
        seam: left("workflow_step_done"),
        reason: "Falls with no replacement (§3). Superseded rather than replaced: `reply \
                 --escalate` is this flag boiled down to the ONE bit that was ever load-bearing \
                 (\"I cannot\"), DECLARED instead of free — the supervisor branches on a signal \
                 whose meaning is written in the channel, not on a token an agent invents and no \
                 reader can enumerate.",
    },
    Disposition {
        verb: "workflow tickets add",
        cli: Gone,
        seam: left("workflow_tickets_add"),
        reason: "REMOVED. Owner, 2026-08-13: \"ueberfluessig\". On §5's closed list, so the seam \
                 method went with the verb rather than surviving as an app-only write — both \
                 halves in one commit, which is the shape this gate exists to hold. It gets no \
                 replacement: a run's commissioned scope was named once, at commissioning \
                 (`--ticket`), and what a run turned out to TOUCH was readable from the messages it \
                 sent — every one carried the run's ticket set on `refs.nxf_ids`. The retractable \
                 thing was always the thread<->item edge, never this set. \
                 \
                 THAT STAMPING IS NOW GONE TOO, with the run record (`orchestration::with_run_refs`, \
                 removed in the same block): with no commissioned set there is nothing to stamp, \
                 and `--ref nxf_ids=<id>` — which always won over the stamp anyway — is what names \
                 a message's subject matter. Recorded here rather than as its own row because it is \
                 the same capability seen from the other end.",
    },
    Disposition {
        verb: "tick --thread",
        cli: Standing,
        seam: left("workflow_tick"),
        reason:
            "THE ONE VERB OF THE `workflow` GROUP THAT SURVIVES, and it survives because it is not \
             a way to run anything: it is a clock's hand. A declared channel's `timeout` schedules \
             a one-shot `at` job at the fan-out site, and that job invokes this binary. Delete the \
             verb and a declared `timeout:` becomes a promise with no mechanism behind it — a \
             straggler is never struck, and the requester waits forever for a member that wrote one \
             line and then hung. \
             \
             So it MOVED rather than went: `nxc workflow tick --thread` -> `nxc tick --thread`, \
             with `#[command(hide = true)]`, which is the form `nxc prime` already has. Owner, \
             2026-08-20: \"an der Aussenflaeche muss es weg\" — off `nxc --help`, off the help \
             goldens, off the prime block; still there for the `at` job, which is the only caller \
             that is not a human forcing a check by hand. A hidden verb is still a verb to this \
             gate (`verb_seam.rs`: \"Hidden commands count\"), which is why it is recorded as \
             `Standing` rather than quietly omitted. \
             \
             §5's closed list names `workflow_tick` and the seam method DID go, so the verb has no \
             library twin — a `KnownGap` in `verb_seam.rs`, deliberately: an embedding app does not \
             drive a board's deadline by hand, the scheduled job does. \
             \
             THE PROPERTY THAT GOES WITH THE SEAM METHOD, said out loud as the earlier note asked: \
             6j6v.nf38 left an accepted defect riding on an app POLLING this — one pending `at` job \
             accumulated per declining poll, because a re-armed member deadline has no cancel. With \
             no seam method there is no poller, so the way that property was reachable at all is \
             gone. The scheduled chain itself is unaffected: it re-arms exactly once per decline, \
             which is what nf38 built.",
    },
    Disposition {
        verb: "workflow liveness",
        cli: Gone,
        seam: left("workflow_liveness"),
        reason: "REMOVED ENTIRELY — verb, seam method, the `workflow_step_liveness` table and the \
                 timer that armed it — and recorded as a NAMED LOSS rather than as inner mechanics \
                 moving below the surface, which is what §3/§4 sketched for it. \
                 \
                 §3's line does not survive contact with the code: this check is keyed on a RUN's \
                 CURRENT STEP and was armed only where a run step fired (`arm_step_liveness`, two \
                 call sites, both inside the run engine). With no runs there is no current step to \
                 watch, so there is nothing to take below any surface. \
                 \
                 WHAT IS LOST: the dead-man's watch over an agent that has gone quiet — the stall \
                 the v3 self-build run hit three times, where a session backgrounded a long gate \
                 command, the process died with no completion signal, and the session waited on a \
                 signal that was never coming. A channel flow never had it (it was run-keyed), so \
                 nothing that works today stops working; what goes is a mechanism that covered a \
                 shape the surface no longer has. Recorded, not re-filed — owner, 2026-08-20: \
                 \"nur benannter Verlust\". \
                 \
                 What remains against a quiet member is the PER-MEMBER channel deadline \
                 (6j6v.nf38, `member_deadline.rs`), which is the other clock and a different \
                 question: it asks whether a member has produced any transcript at all inside its \
                 declared window, and strikes through `nxc tick`. It does not nudge, and it does \
                 not escalate.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: left("workflow_events"),
        reason: "ACCEPTANCE POINT 4, and §5 called it easy to overlook: this is the stream an app \
                 SUBSCRIBES to, so if the workflow dies as a concept and the stream does not, the \
                 deleted concept lives on exactly where the app sees it. \
                 \
                 It is DISCHARGED BY REMOVAL rather than by the rebuild the acceptance sketches \
                 (\"sondern den Faden-/Kanal-Begriff\"). Owner decision of 2026-08-20, taken on two \
                 findings rather than on preference: \
                 \
                 (1) NOBODY CONSUMES IT. Checked in the neighbour repos, not assumed: manufakt.io \
                 has no call site for `workflowEvents`/`subscribeWorkflow` at all, and \
                 app-foundations carries only the pass-through through engine-bridge/engine-server \
                 plus its own tests. (Contrast `workflowStatus`/`workflowStart`/`workflowLiveness`, \
                 which have three LIVE call sites over there — `use-role-runtime.ts:90`, \
                 `WorkflowsTab.tsx:176` and `:191`.) \
                 \
                 (2) THE REPLACEMENT IS ALREADY ON THE SEAM. `Engine::subscribe` is the coalesced \
                 \"someone wrote, re-read\" tick, and `Engine::status`/`threads`/`thread_board` are \
                 what a reader reads afterwards. What the run stream added over that was a true \
                 DELTA with a payload, and its own module doc gave the reason: in a chat workspace \
                 nearly every write is a MESSAGE, so a run-watcher would have re-enumerated every \
                 run on every message. For a FADEN stream that argument inverts — the messages ARE \
                 the events — so `subscribe` plus a thread read is very nearly the stream itself. \
                 \
                 THE RESIDUAL, named rather than left to be found: a coalesced tick does not say \
                 WHICH thread moved, so an app re-reads the tree where a delta would have told it. \
                 That is a cost, and it is the whole cost.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: left("workflow_event_cursor"),
        reason:
            "The cursor of the same stream (acceptance point 4). It goes with what it indexes: a \
             reader's position in a stream of run phases means nothing once there are no runs.",
    },
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: left("subscribe_workflow"),
        reason:
            "The live half of the same stream (acceptance point 4) — the receiver an app holds for \
             the whole session, which is precisely where a deleted concept would be most visible \
             and hardest to take back. `Engine::subscribe` is the receiver that stays.",
    },
    // ---- entrances with no seam counterpart at all --------------------------------------------
    //
    // These are the `KnownGap` rows of `verb_seam.rs` seen from the other side. The decision is
    // "nothing stays, because nothing was ever there", and the gate holds exactly that: if someone
    // lifts one of these onto the seam before the verb is cut, the claim stops being true and the
    // row has to be re-decided rather than quietly meaning something else.
    Disposition {
        verb: "channels create",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason:
            "The CLI mints the channel id and sets the fields straight on the store; the facade \
                 can only READ channels (6j6v.hcq1). The verb falls to the declaration — \
                 `ensure_declared_channel` materialises a channel on the first `send --to` — so \
                 nothing needs lifting first.",
    },
    Disposition {
        verb: "channels dm",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason: "The shared half exists as `orchestration::ensure_dm_channel`, but no facade/ \
                 `Engine` verb offers it as a capability of its own (6j6v.hcq1). `send --to \
                 <persona>` materialises the direct conversation itself, so the entrance goes and \
                 there is nothing on the seam to decide about.",
    },
    Disposition {
        verb: "channels join",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason:
            "Membership adds are store-direct in the CLI (6j6v.hcq1). Membership now stands in \
                 the channel declaration's `members:`, so the verb has no successor and the seam \
                 never had a counterpart to keep.",
    },
    Disposition {
        verb: "channels leave",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason: "Membership removes are store-direct in the CLI (6j6v.hcq1); same reasoning as \
                 `channels join` beside it.",
    },
    Disposition {
        verb: "agents list",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason:
            "Profile reads are store-direct (`store.profiles()`, 6j6v.1xdd). The verb collapses \
                 into `list`, which reads the DECLARED team through `Engine::directory` — a \
                 different read of a different thing, so this row records an absence rather than a \
                 rename.",
    },
    Disposition {
        verb: "agents register",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason:
            "REMOVED. Profile writes were store-direct (6j6v.1xdd), so nothing on the seam was \
                 given up. Registering is now DECLARING: a persona file in `.nxs-personas/`, which \
                 is readable, reviewable and survives the run — a runtime profile row lived in one \
                 workspace's database and nothing reviewed it.",
    },
    Disposition {
        verb: "agents search",
        cli: Gone,
        seam: SeamState::NoCounterpart,
        reason: "REMOVED. Profile search was store-direct (`store.search_profiles()`, 6j6v.1xdd). \
                 Collapses into `list`, which carries the two fields it searched — `job_title` and \
                 `job_description` — for the whole declared team.",
    },
    // ---- what a CALLER stopped saying, 6j6v.07me ----------------------------------------------
    //
    // ONE row, and the block says why it is one rather than four. `Caller` lost `origin`, `hop` and
    // the mandatory `now`; none of the three was a CLI entrance (their command-line halves are the
    // environment variables `NXC_ORIGIN`/`NXC_HOP`/`NXC_NOW`, every one of which STAYS), and two of
    // them left no seam symbol behind to decide about — the depth is a `session_map` column and the
    // clock is a default inside the adapter. There is nothing for this table to hold there; what
    // holds them is `tests/caller_seam.rs`, which drives a chain to the cap with nothing passed.
    //
    // `origin` is different, and it is different in exactly this gate's §5 shape: taking the field
    // away means an app can no longer construct the `<origin>/<actor>` handle it must pass BACK
    // into `messages`/`channels`/`inbox` to read what it just wrote. The read that replaces the
    // field is the thing an app renders, so it gets a row.
    Disposition {
        verb: "",
        cli: NeverWas,
        seam: stays("origin"),
        reason: "`Caller.origin` — the minting workspace identity as a per-call ARGUMENT — went \
                 in 6j6v.07me, and `Engine::origin` is what replaces it. Not a rename: the field \
                 was a claim (two calls on one handle could name two workspaces, on the value \
                 access rights will hang off, 6j6v.6aza) and this is a READ of the handle's own \
                 answer. \
                 \
                 It stays because of §5's rule and not by inertia — every qualified handle an app \
                 passes back into a read is `<origin>/<actor>`, so deleting this read would leave \
                 a caller that no longer supplies the origin unable to name what it just wrote. \
                 \
                 Never a CLI entrance: the command line's half is `NXC_ORIGIN`, which `cli.rs` \
                 still reads and passes — and since 6j6v.07me falls back to the SAME \
                 `workspace::origin_of` the handle answers with, so the two adapters cannot name \
                 one workspace differently (`tests/parity.rs::\
                 both_adapters_resolve_one_origin_for_one_workspace`).",
    },
    // ---- `release` leaves the user surface AND the seam alike, nxf 6j6v.b9nf ---------------------
    Disposition {
        verb: "release",
        cli: Gone,
        seam: left("release_working_tree"),
        reason: "OWNER DECISION, 2026-09-17: `nxc release --thread <id>` and `Engine::\
                 release_working_tree` both leave — removed 2026-09-19. THE SAFETY REASON, in the \
                 owner's own terms: offered on the agent surface, this call could take the working \
                 copy away from a running coding operation, and on a `WorkerConfig::Custom` worker \
                 that never implemented `Worker::session_is_running` it was an unconditional \
                 release with no guard at all — the one shape epic 6j6v.bqe0 (\"KEIN NEUES \
                 MUTATIONSVERB\") was written to keep off this surface, reached anyway because the \
                 verb's own refusal was the ONLY thing standing between it and that override. \
                 \
                 WHY BOTH HALVES GO TOGETHER, unlike `inbox`'s slower unwind above: an app holding \
                 the handle is exactly the caller this risk is about, so a seam method with no CLI \
                 path in front of it would still be the same unconditional override for a host that \
                 built its own `Worker` without the liveness check — leaving it standing would not \
                 have closed anything. \
                 \
                 WHAT REPLACES IT (PR #478 on main, and this branch): every hand-off now PARKS the \
                 holder's work first, so a working copy held by a chain that is dead is parked and \
                 handed on by the background service on its own — past its two-hour bound, or \
                 inside it once `sweep_expired_working_tree`/`holder_is_provably_dead` can prove the \
                 chain is gone — and a working copy held by a chain that is still running is taken \
                 back on purpose with `nxc withdraw --thread <id>` / `Engine::withdraw`, which \
                 discharges the round, stops its session and parks whatever it left uncommitted \
                 rather than yanking the checkout out from under it. Nothing here waited on a \
                 successor before removing the verb: both answers already existed when this row was \
                 written.",
    },
];

#[test]
fn every_entrance_taken_off_the_agent_surface_has_a_recorded_seam_decision() {
    // The same seam union `verb_seam.rs` reads: the long-lived handle plus the compute layer under
    // it. The store below is deliberately NOT seam — a capability that exists only there is an
    // omission, not a counterpart.
    let sources = ["src/engine.rs", "src/facade.rs"]
        .map(|rel| crate_relative(env!("CARGO_MANIFEST_DIR"), rel))
        .to_vec();
    assert_seam_dispositions("nxc", &nexus_chat::cli::command(), &sources, RECORD);
}

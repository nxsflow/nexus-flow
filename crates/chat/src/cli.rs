//! nxc — the nexus-chat agent CLI (spec §5.1/§5.2). A thin, deterministic pull consumer of the T1
//! chat views: every verb resolves a `.nxs/` workspace, opens the [`ChatStore`], and appends an op or
//! reads a view. `--json` everywhere is the agent contract; declared struct field order is the
//! byte-stable contract. Membership checks (the friendly not_found/forbidden message) live here; the
//! store stays pure. T3 adds the setup/onboarding contract verbs `init`/`agent-manifest`/`prime` +
//! the `inventory` self-registration ([`chat_init_entry`]) that seats `nxc` in the `nxs init` chooser
//! and the `nxs prime` fan-out.

use crate::error::{self, NxfError, Result};
use crate::facade;
use crate::model::*;
use crate::onboarding;
use crate::store::ChatStore;
use crate::transcript::TranscriptEntry;
use crate::workspace::{self, ChatWorkspaceExt, Workspace};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// nexus-chat agent CLI.
#[derive(Parser, Debug)]
#[command(name = "nxc", version, about = "nexus-chat agent CLI", long_about = None)]
struct Cli {
    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    json: bool,
    /// Use this sqlite db file directly instead of discovering `.nxs/`.
    #[arg(long, global = true, env = "NXC_DB")]
    db: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize chat in the current directory: ensure a `.nxs` workspace, register the `chat`
    /// module, then delegate the shared agent file + one SessionStart hook per active module to
    /// the `nxs` assembler (re-assembling every active module) — parity with `nxm init`.
    Init {
        /// Driven/quiet mode: set up chat and delegate the agent files + hook, but render no banner.
        /// The seam the `nxs` umbrella uses to drive `nxc init` without output bleeding through; also
        /// the non-interactive agent entry.
        #[arg(long)]
        quiet: bool,
    },
    /// Emit nxc's declared contribution to the shared agent file (the prime command + the
    /// SessionStart hook) as data. `--json` is the machine contract the `nxs` umbrella assembles
    /// from.
    AgentManifest,
    /// Session bootstrap: pin the chat rule, name the core verbs, say who can be addressed here,
    /// and name the commissions of this caller's own that finished while it was away. Hidden from
    /// `--help`: the umbrella `nxs prime` is the single user-facing entry and fans out to this verb
    /// as a subprocess.
    #[command(hide = true)]
    Prime {
        /// Read the record as this handle (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
        /// Prime as this declared persona: adds its identity block and its address
        /// book. OPTIONAL — a session started by `nxc` is recognised from the session it runs
        /// under, so this is for hosts that start sessions THEMSELVES (an app). Without either,
        /// the caller is a human.
        #[arg(long)]
        persona: Option<String>,
    },
    /// Who can be addressed here, and what for. Without `--persona` a human sees
    /// the whole declared team — which is also what an app renders as its directory — while a
    /// running persona sees its own address book, recognised from its session. With `--persona` it
    /// is that persona's view, whoever is asking.
    List {
        /// Project the directory for this declared persona.
        #[arg(long)]
        persona: Option<String>,
    },
    /// Open a conversation with a declared persona or channel: posts the first message, stamps a
    /// thread, and starts (or wakes) whoever is on the other side. Returns the thread id.
    Send {
        /// The message body. The TARGET is `--to`: a declared persona or a declared channel.
        /// Pass `-` to read the body from STDIN instead (escaping-free, multi-line — the safe form
        /// for anything longer than a sentence), or supply it via `--body-file`.
        // ONE positional since 6j6v.dvyq §3. It used to be a 1-or-2-token variadic, because a
        // channel could be named positionally: `send <channel> <body>`. That form is gone with the
        // raw channels — `send --to <name>` is how a target is named, and the name resolves against
        // the DECLARATIONS. A verb whose first token means "channel" in one invocation and "body"
        // in the next was also the one shape clap could not check for us.
        //
        // `Option<String>` + `required_unless_present` since nxf 6j6v.s46h, which is the ONE thing
        // this ticket relaxes about the positional: a body may now arrive from a file instead, and
        // a positional that clap still insisted on would make `--body-file` unusable. Missing on
        // BOTH is still clap's own refusal, naming `<BODY>` exactly as before.
        #[arg(required_unless_present = "body_file")]
        body: Option<String>,
        /// Read the body from this file (UTF-8, verbatim); `-` reads STDIN. Mutually exclusive
        /// with the positional body — the same notation `nxf` uses for `--description-file`.
        #[arg(long = "body-file")]
        body_file: Option<String>,
        // `--kind`, `--priority` and `--disposition` were here. REMOVED by 6j6v.ckeq (owner,
        // 2026-08-21): "interessante Features, die wir zu einem spaeteren Zeitpunkt bei Bedarf
        // zurueckholen, aber dann mit mehr Wissen ueber den tatsaechlichen Bedarf." Each one let a
        // caller answer a question nobody had asked it, and what a bare `send --to` writes is
        // unchanged — the three defaults (`info`/`normal`/`in_turn`) are what the seam now passes.
        //
        // TWO CONSEQUENCES, both deliberate and both named where they bite. `kind` survives as a
        // CARRIER — `reply --escalate` is `kind: escalation`, and exactly two of the five values
        // stay in use. `priority` was the working-tree queue's first sort key, so the queue is pure
        // FIFO now (`crate::surface::send_to`'s persona branch carries the argument;
        // `surface_cli.rs::the_working_tree_queue_is_fifo_and_nothing_on_the_surface_can_reorder_it`
        // holds it).
        //
        // `--thread` was here too. REMOVED by 6j6v.dvyq §3 — `reply --thread <id>` is how you post
        // into an existing thread, and `send --to` is how one comes into being (it mints the
        // thread and returns its id). Naming an arbitrary thread on a fresh `send` was the third
        // way to say the same thing, and the only one that let a caller invent a thread id no
        // opener had ever stamped.
        /// What this conversation is about — a ref pointer `k=v`, repeatable: `nxf_ids` (a ticket,
        /// repeatable), `branch`, `pr`, `session_id`. REQUIRED unless you say `--no-ref`.
        #[arg(long = "ref")]
        refs: Vec<String>,
        /// This conversation really is about nothing — the explicit answer that turns off the
        /// `--ref` reminder. Says "I looked", where saying nothing says nothing.
        #[arg(long, conflicts_with = "refs")]
        no_ref: bool,
        // `--role` and `--session` were here. REMOVED by 6j6v.dvyq §3, which leaves `--to` as the
        // one way to name a target.
        //
        // `--role <handle>` COLLAPSES into `--to <handle>`: both run
        // `orchestration::coordinator_commission`, and since §3 took `send`'s positional channel
        // away
        // neither can name a channel any more, so both open the caller↔role DM themselves. What
        // `--to` adds is what made the collapse necessary rather than merely possible: it stamps a
        // THREAD and tells the persona which one to answer into, so the answer has an address.
        // `--role` posted threadlessly, which left its replies unaddressable by `reply --thread` —
        // the one surviving reply form.
        //
        // `--session <id>` is a LOSS, not a collapse, and §3's "-> reply --thread" is no
        // construction proof for it (owner, 2026-08-19). `reply --thread` wakes through the
        // thread's RETURN ADDRESS, and `surface::thread_return_address` reads the newest message
        // from somebody ELSE carrying a session id — a persona summoned by `send --to` that has not
        // answered yet has posted none, so a follow-up posts but wakes nobody. Delivering into a
        // session that is mid-turn is `reply --thread <id> --force`, carried by nxf 6j6v.xr3z as
        // the messaging half of the overtaking lane; the runtime signal it needs does not exist
        // yet (`worker.rs`: "Teardown is deliberately absent"). Until then the gap stands open and
        // named, rather than papered over with a verb that answers a different question.
        /// Open a conversation with a declared persona or channel and get a thread id back.
        /// The TARGET's declaration decides what happens — a persona is started on a fresh
        /// session, a channel fans out under its own policy. A channel no declaration names is
        /// not a target (6j6v.dvyq §3).
        // REQUIRED since 6j6v.dvyq §3 took `--role`/`--session`, and required in CLAP rather than
        // refused in the body — the same shape `reply --thread` took in the same §3, for the same
        // reason. While three flags could each be a target, none of them was individually
        // mandatory and clap could only say "the following required arguments were not provided";
        // a hand-written refusal naming all three was the more useful answer. With ONE target flag
        // left, clap names exactly the missing flag, and an OPTIONAL-looking `--to` on a verb that
        // cannot do anything without it would make `nxc send --help` — an agent-facing page — say
        // something untrue.
        //
        // A plain `String` rather than `Option<String>` + `required = true`: clap derives the same
        // requirement from the type, and the type then carries it into the dispatcher, which would
        // otherwise have to unwrap an `Option` that cannot be `None` — a panic reachable only by
        // someone later relaxing the flag, which is exactly the kind of thing a type should make
        // unwritable rather than a message should apologise for.
        #[arg(long)]
        to: String,
        // `--deadline` was here, and it MOVED here from `ask --deadline` in 6j6v.dvyq §3 only to be
        // REMOVED by 6j6v.ckeq (owner, 2026-08-21) together with `--model`'s seam half: when a
        // declared channel's board must be answered by is the CHANNEL's own `timeout:`, and a
        // per-call override beside a declared value is two answers to one question — "sonst wird
        // ein Agent vielleicht uebereifrig Optionen aendern".
        //
        // NOTHING IS LOST, and it is worth saying because the flag's own arrival note argued the
        // opposite for `ask --deadline`: "a duration has a declared form (`timeout:`), an absolute
        // instant has none". That was already untrue by then. A channel's `timeout:` is parsed by
        // the SAME `facade::resolve_deadline_spec` this flag used, and that parser tries RFC3339
        // first — so `timeout: 2026-08-16T10:30:00Z` declares an absolute, non-resettable instant
        // exactly as `--deadline` did, with the same per-member semantics on the other side. Pinned
        // by `channel_member_timeout.rs::a_deadline_given_as_an_absolute_instant_arms_no_clock_and_
        // no_transcript_moves_it`, which now declares it instead of passing it.
        //
        // The seam field went with the flag (`ChannelOpenRequest::deadline` survives and `send_to`
        // passes `None`), so there is no app-only door either.
        /// Run this chat on that machine — an id or a name from `nxs sync machines`. Without it the
        /// chat runs where the persona's `machine:` says, else on this machine. A machine that is not
        /// online is ASKED about, and nothing is sent: the refusal lists the machines that are
        /// online, and naming one here is the answer. A machine named here is honoured even when it
        /// is not online — the chat then waits in the log until it is back. Persona chats only.
        #[arg(long)]
        machine: Option<String>,
        /// Watch it happen instead of returning immediately: the thread AND the personas'
        /// transcripts, until somebody answers. For a human at the keyboard — refused once a
        /// persona is registered.
        #[arg(long)]
        stream: bool,
    },
    /// Answer in a thread: posts into the thread's own channel and hands the turn back to whoever
    /// is on the other side of it.
    Reply {
        /// The message body. The thread is `--thread`. Pass `-` to read the body from STDIN
        /// instead — the form to use for a verdict, a report or anything quoting a command, since
        /// nothing on that path is evaluated by the shell — or supply it via `--body-file`.
        // ONE positional since 6j6v.dvyq §3, for the reason `send`'s is one: `reply <target>
        // <body>` named the conversation positionally, and a first token that means "target" or
        // "body" depending on how many follow it is a shape clap cannot check. `--thread <id>` is
        // the one address, and it is what a persona's own prime block has always taught.
        //
        // OPTIONAL since nxf 6j6v.s46h — see `send`'s identical note for what that relaxes and
        // what it does not.
        #[arg(required_unless_present = "body_file")]
        body: Option<String>,
        /// Read the body from this file (UTF-8, verbatim); `-` reads STDIN. Mutually exclusive
        /// with the positional body — the same notation `nxf` uses for `--description-file`.
        #[arg(long = "body-file")]
        body_file: Option<String>,
        // `--kind` was here, with `escalation` deliberately absent from its value list because
        // `--escalate` was the one door to that marker. REMOVED by 6j6v.ckeq with `send`'s: the flag
        // offered five values, one of which (`escalation`) it refused, one of which (`info`) was the
        // default, and three of which (`task`/`report`/`decision`) nothing in this engine ever read.
        //
        // THE ONE VALUE THAT DID SOMETHING AND IS NOW UNREACHABLE: `--kind question`. The owner's
        // ruling of 2026-08-16 (nxf 6j6v.1xw1) makes an open QUESTION hold the working-tree claim
        // exactly as an escalation does — `working_tree::hands_the_task_back` still branches on it,
        // and `working_tree_claim_scope.rs` still covers that branch — but no caller can produce one
        // any more. A member with a real follow-up question says it with `--escalate` (the claim is
        // held either way, which is the property that mattered) or asks in a fresh `send --to`.
        // Recorded as a named LOSS on the way to bringing the option back "mit mehr Wissen ueber den
        // tatsaechlichen Bedarf", not as a collapse.
        //
        // `--priority`, `--disposition` and `--ref` were here too. The first two went for `send`'s
        // reasons; `--ref` went because a reply INHERITS its subject — the thread already says what
        // the conversation is about, which is why 6j6v.ckeq put the refs obligation on `send --to`
        // and nowhere else. Nothing that made a chain resumable depended on it: the return address is
        // stamped from the ambient session by `orchestration::reply`'s `with_return_address`, never
        // read off a caller-supplied `--ref session_id=`.
        /// "I cannot reach the result on my own — I need help, or a decision." One of the three
        /// things an agent may say with `reply`; the others are "I am finished" (a bare reply) and
        /// `--needs-rework`, which is about somebody ELSE's work. Everything else it produces
        /// belongs in the transcript.
        ///
        /// **A DECISION counts, and saying so is half of what this flag is for** (nxf 6j6v.aqqa).
        /// The short reading — "I cannot carry out this task" — describes only the blocked half,
        /// and a session that CAN carry its task out reads it as not applying to it. Measured: a
        /// PM with two open product questions ended its turn with a plain reply, and the chain
        /// handed the questions DOWNWARD to a review round that had no standing to answer them. A
        /// decision that is not yours goes UP, to whoever commissioned you, and this is the flag
        /// that takes it there.
        ///
        /// It is not a wastebasket for anything that is not a result. A real follow-up question
        /// ("what do you mean by X?") used to be `--kind question` and is a different thing still;
        /// with `--kind` gone (6j6v.ckeq) there is no way to spell it on a reply, so ask it by
        /// opening a conversation of its own — or say it here, since for the WORKING COPY the two
        /// are the same fact anyway (nobody is working, the task is mid-flight).
        ///
        /// The turn is discharged either way — the session ends, and the debt with it. What is
        /// different is the OUTCOME, and the channel's supervisor decides what becomes of it.
        ///
        /// On a `flow: sequential` channel that decision is: the chain ENDS here. The step after
        /// this one is not started, the round is handed up as it stands, and whoever commissioned it
        /// decides what happens next (nxf 6j6v.ma7v). A round that is merely SLOW has no spelling on
        /// this verb — do not reach for this flag to buy time, because a caller reads it as work
        /// that will not arrive.
        ///
        /// It is DECLARED rather than free, which is what makes it a signal at all: the
        /// supervisor branches on a bit whose meaning is written in the channel, not on a token
        /// you invent and no reader can enumerate.
        #[arg(long)]
        escalate: bool,
        /// "What you handed me does not meet the standard." The THIRD and last thing an agent may
        /// say with `reply` — and the one about SOMEBODY ELSE's work rather than your own.
        ///
        /// Use it when you were asked to assess something and what you were given cannot go
        /// forward as it stands. Name what must be put right IN THE BODY: those words are handed
        /// to the party that produced the work, and they are its next task.
        ///
        /// **It is not `--escalate` and the two may not be combined.** `--escalate` says you
        /// cannot reach the result of YOUR task without help or a decision, and ends the chain
        /// upward; this says the round goes BACK and continues. Setting both is refused.
        ///
        /// **What it does is written in the channel** (`steps:` → `on_needs_rework:`), which is
        /// what makes it a signal rather than a token you invent. A step whose channel declares no
        /// back edge will not have offered it to you at all; set on such a step it is dropped, and
        /// the reply's receipt says so.
        #[arg(long, conflicts_with = "escalate")]
        needs_rework: bool,
        /// "I know what the review said. This goes on anyway." **Not for the party whose work was
        /// reviewed** — for the party that ASKED for the round (nxf 6j6v.am8j).
        ///
        /// Answer the round's own thread with it and the step whose verdict sent the work back
        /// counts as satisfied: the chain continues over that step's `next:` instead of going round
        /// the cycle again, and the next step is handed your words together with the verdict you
        /// set aside, so it knows the judgement was overruled rather than met.
        ///
        /// **Without it there is no way out of a cycle whose reviewer never becomes satisfied.**
        /// `max_passes` bounds ONE run, and answering the round in any other way commissions a new
        /// one — which arms the same ceiling again. Measured in `watch-bundestag` on 2026-09-12: six
        /// draft/review passes, six verdicts, not one ticket handed on.
        ///
        /// **You may not accept your own reviewer.** It is honoured only on the thread of a round
        /// YOU commissioned; on the slot you are serving it is refused, because the separation
        /// between producing and checking is the whole reason the round has two parties. What an
        /// agent does instead is `--escalate`, which asks the party above it to decide — and this is
        /// the verb that party then has.
        ///
        /// It is recorded as its own kind (`accepted`), so an override stays findable afterwards as
        /// something other than "the review passed" — because it is not that.
        #[arg(long, conflicts_with_all = ["escalate", "needs_rework"])]
        accept: bool,
        /// Answer in this thread. A thread id identifies a conversation on its own — no channel
        /// context — and hands the turn back to whoever is on the other side of it.
        ///
        /// **Optional since 6j6v.dq59, while exactly ONE conversation is open for you**: then a
        /// bare `nxc reply "…"` answers it, because there is nothing else it could mean. With
        /// several open — you owe your commissioner an answer AND the role you consulted has come
        /// back to you — this is required again, and the message that woke you named each one with
        /// the command it takes. Passing it is always allowed, including when only one is open.
        #[arg(long)]
        thread: Option<String>,
        /// FOR THE AGENT SIDECAR'S TEARDOWN, not for you: post ONLY if the caller still owes a
        /// reply on this thread — it is named in the thread's `expects_reply_from` (nxf
        /// 6j6v.jepk/6j6v.enrs) and has not posted there yet. Otherwise a deliberate no-op — exit 0,
        /// `posted: false` in `--json`, nothing written — never an error.
        ///
        /// That is what lets a caller which cannot tell whether it already answered ask
        /// unconditionally and safely every time (nxf 6j6v.ww0a), and there is exactly one such
        /// caller: `agent-sidecar/src/main.mjs` runs `nxc reply --thread <id> --if-unanswered
        /// <text>` when an SDK session ends without ever answering the thread it owed, so the thread
        /// does not go quiet forever. When owed, this behaves exactly like a plain reply, return
        /// address and all; when not owed, the skip wakes nobody either.
        ///
        /// It is deliberately NOT on the library seam (nxf 6j6v.ckeq): an app holds its own sessions
        /// and knows whether they answered.
        #[arg(long)]
        if_unanswered: bool,
        /// Hand this chat to another machine — an id or a name from `nxs sync machines`. The persona
        /// is started there with the conversation so far; this machine starts nothing. Persona
        /// chats only, and a person's decision: a session inside the chat cannot move it.
        #[arg(long)]
        machine: Option<String>,
        /// Watch the answer arrive (see `send --stream`).
        #[arg(long)]
        stream: bool,
    },
    // `ask <channel> <body> [--expect …] [--deadline …]` was here. REMOVED by 6j6v.dvyq §3 —
    // `send --to <channel>` opens the board, and the DECLARATION supplies who must answer, by when
    // and what becomes of the answers. `ask` predates declared channels: it was the verb you used
    // when a channel had no policy of its own, which is why it had to take `--expect` at all. With
    // the raw channels gone there is no channel without a policy, so a per-call `--expect` would be
    // a second answer to a question the channel already answers — and `send --to` refuses one for
    // exactly that reason.
    /// Thread quorum boards: `list` the caller's boards, or `show` one in full.
    Threads {
        #[command(subcommand)]
        action: ThreadsAction,
    },
    /// Where an operation stands: the whole thread TREE from its root down, across channel
    /// borders. `--thread` shows the one operation that thread belongs to; `--channel` the live
    /// operations that started in a channel; `--all` adds the finished ones; with none of them,
    /// everything still going on here.
    Status {
        /// Any thread of the operation — the tree is shown from ITS root, whether it is still
        /// running or already finished.
        #[arg(long, conflicts_with_all = ["channel", "all"])]
        thread: Option<String>,
        /// A declared channel name (or a raw channel id): the live operations whose ROOT sits
        /// there. An entry point, not an anchor — an operation crosses channels.
        #[arg(long)]
        channel: Option<String>,
        /// Finished operations too, not just what is still going on — with `--channel`, the
        /// finished ones of that channel.
        #[arg(long)]
        all: bool,
    },
    // `inbox` and `read` were here. REMOVED by 6j6v.1gm9 — the AGENT-PULL half of the message
    // surface, taken away together with the "## Threads you opened" block `read` was supposed to
    // bound. The boundary that decided it: **humans pull, agents get pushed.** A human at a
    // terminal has no prompt to push into, so `nxc threads show` and `nxc status` stand untouched;
    // an agent is pushed on all three delivery paths (`send` into a fresh prompt, `reply` through
    // `resume_return_address` with the reply's body, a completed quorum through
    // `wake_role_requester`), so a pull verb for it is the duplicate of what already arrived. The
    // measurement behind that: `read` 0/66 and `inbox` 0/66 role sessions (6j6v.4mmk), and neither
    // was ever in `PRIME_COMMANDS` — the surface a session is TAUGHT — so nothing that reads this
    // binary's own bootstrap loses a verb it was told about.
    //
    // WHAT DID NOT GO WITH THEM AT THE TIME, and where it was decided: the compute halves stayed —
    // `facade::inbox`, which `prime` read, and `facade::mark_read`, the ack that advanced the
    // synced cursor — because whether the apparatus survived at all was **nxf 6j6v.4d2z**, which
    // overlapped this ticket exactly here. 4d2z has since answered it: BOTH halves are gone, with
    // `read_cursors`, `ChatStore::inbox` and `PrimeReport`'s catch-up fields. So nothing is left
    // behind these two entrances at all. See `tests/seam_disposition.rs`.
    // `channels list|public|create|dm|join|leave` was here. REMOVED by 6j6v.dvyq §3 — a channel
    // is a DECLARATION now, and its lifecycle is that file's: `ensure_declared_channel`
    // materialises it on the first `send --to`, `members:` says who is in it, and `nxc list` is the
    // read over the declared team and its channels. `channels dm` went the same way: `send --to
    // <persona>` opens the direct conversation itself.
    //
    // `channels public` is the one that has NO CLI successor, and saying so plainly is the point:
    // it listed the PUBLIC channels in the workspace's store, including front doors that reached
    // this workspace by sync and that no declaration here names — which is precisely what `nxc
    // list` cannot show, because `list` reads declarations. Cross-project discovery therefore
    // LEAVES the agent surface and an app renders it (owner, 2026-08-19). It stayed as
    // `Engine::public_channels` until 6j6v.yr59 folded it INTO the directory — so `nxc list --json`
    // and `Engine::directory` both carry the front doors now, while the human rendering still does
    // not (a door no declaration here names is discoverable, not addressable). Recorded in
    // `tests/seam_disposition.rs` rather than dressed up as a collapse into `list`.
    // `agents list`/`register`/`search` were here. REMOVED by 6j6v.dvyq §3 — a team is DECLARED,
    // not registered: a persona is a `.nxs-personas/<handle>.yaml`, and `nxc list` is the read over
    // it (with `job_title`/`job_description`, the two fields `agents search` searched). A profile
    // row registered at runtime was a second, weaker answer to "who is here": it lived only in one
    // workspace's database, nothing reviewed it, and it did not survive the run.
    /// Case-insensitive substring over message bodies in the caller's channels (bound LIKE),
    /// deterministically ordered.
    Search {
        /// The substring to search for.
        query: String,
        /// Search as this handle's channels (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
    },
    /// The role-runtime internal↔real SDK session map.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// The role session's Claude Agent SDK transcript: the normalized stream — assistant text,
    /// thinking, tool calls and their results, and any subagent activity they spawned.
    Transcript {
        #[command(subcommand)]
        action: TranscriptAction,
    },
    /// Which machine a chat runs on — a new one with `--to <persona>`, or `--thread <id>` — and
    /// whether sending now would ask instead: the machine, where that came from, whether it is
    /// online, and the machines that are. Writes nothing.
    Machine {
        /// The persona a new chat would be with.
        #[arg(long, conflicts_with = "thread", required_unless_present = "thread")]
        to: Option<String>,
        /// A chat that exists.
        #[arg(long)]
        thread: Option<String>,
        /// The machine you are about to choose, to see what that would do.
        #[arg(long)]
        machine: Option<String>,
    },
    /// Start or resume the personas of every chat THIS machine runs that is owed a turn — what the
    /// background service runs after a pass pulled something. Idempotent. Hidden from `--help`:
    /// the service is its caller.
    #[command(hide = true, name = "pick-up")]
    PickUp,
    /// Take back a commission — one still waiting for the working copy, or a round that is running.
    ///
    /// A person's verb: only the caller who sent the operation's first commission may take it
    /// back, and a session this workspace started is refused — an agent that wants a round stopped
    /// escalates (`nxc reply --thread <id> --escalate`).
    ///
    /// A commission parked behind the working-tree lease has no session, no transcript and no model
    /// call behind it, so withdrawing it costs nothing and loses nothing. A round that has STARTED
    /// is discharged and its session asked to stop; if it holds this workspace's working copy,
    /// whatever it left uncommitted is parked onto a branch by the background service once the
    /// process is gone — never rolled back — and the next commission into the same thread brings
    /// it back. A host whose worker cannot stop a session refuses the whole call before changing
    /// anything.
    ///
    /// Name the thread your first `send` handed you — a member thread below it is refused. Everything
    /// under it is taken back and the chain below it stops: no next step is commissioned and nothing
    /// is consolidated. Each thread is discharged with a message saying who withdrew it and what
    /// became of its work.
    Withdraw {
        /// The operation's first thread — the one your first `send` handed you. A thread below it
        /// is refused, with this one named.
        #[arg(long)]
        thread: String,
    },
    /// Take up an operation that stopped because the model was not available to it — an exhausted
    /// quota, a provider that was unreachable, a broken connection.
    ///
    /// Nothing was broken and nobody did anything wrong, so the round was never handed back: the
    /// thread still owes its answer and the session is waiting to give it. `nxc status` marks such
    /// an operation and says when the boundary falls.
    ///
    /// **The session continues with its own transcript** — everything it did before the
    /// interruption, including every command and what that command returned — rather than starting
    /// over. That is what stops the work being done a second time.
    ///
    /// **It checks before it starts anything**: a round that was answered before the interruption is
    /// not run again, a session that is already working is left alone, and a working copy that has
    /// moved since sends a question back to the thread that started the operation instead of
    /// continuing blind.
    ///
    /// The background service does this by itself once the boundary falls. Run it by hand to go
    /// earlier — with another account, say — with `--force`.
    Resume {
        /// A thread of the interrupted operation — the id `nxc status` shows.
        #[arg(long, conflicts_with = "session", required_unless_present = "session")]
        thread: Option<String>,
        /// The interrupted session itself, by its internal id. What the background service's own
        /// job carries; a person normally names a thread.
        #[arg(long)]
        session: Option<String>,
        /// Start it even though the runtime says its window has not lifted yet.
        #[arg(long)]
        force: bool,
    },
    // `Release { thread }` was here — "give this workspace's working copy back when the chain
    // holding it is over and nothing is going to give it back on its own". REMOVED by nxf 6j6v.b9nf
    // (owner decision, 2026-09-17: `release` leaves the user surface). Its safety reason: offered to
    // an agent, it could take the working copy away from a running coding operation, and a custom
    // worker that never implemented `session_is_running` made it an unconditional release. Every
    // hand-off now parks first (PR #478) and the way out it named is now two answers instead of one
    // manual override — a held copy of a dead chain is parked by the sweep, on its own, past the
    // bound or inside it where liveness is answered; a held copy of a live chain a caller wants back
    // is `nxc withdraw --thread <id>`, which discharges it and stops its session. See
    // `tests/seam_disposition.rs`'s row for the decision and removal dates.
    // `workflow start|step done|status|list|liveness` was here, and so was the whole `workflow`
    // group. REMOVED by 6j6v.dvyq §3 together with the run record itself: a channel DECLARES its
    // flow (6j6v.hq71), `send --to <channel>` starts it, and `nxc status` is where an operation's
    // position is read (6j6v.a71h). What is left of the group is the one verb below, which was
    // never a way to run anything — it is a clock's hand.
    /// Re-check a thread's completion/staleness and, if due, route it through its declared
    /// channel's `on_complete` policy — what a channel's scheduled one-shot timer runs when it
    /// fires. IDEMPOTENT: a thread whose completion was already handled — by a real completing
    /// `reply`, or an earlier `tick` — is a clean no-op, never a second wake or synthesizer spawn.
    ///
    /// Hidden from `--help` for `prime`'s reason, one step further: nobody types this. The
    /// one-shot job a channel's declared `timeout` schedules is what runs it, and that job
    /// re-schedules itself on every decline, because a member's clock is restarted by that
    /// member's own transcript.
    // USER-VISIBLE `--help` text above, so it names no removed verb and carries no board id.
    // The rest of the reasoning — why the verb survives the group it stood in, and the owner's
    // direction of 2026-08-20 ("an der Aussenflaeche muss es weg") — is in
    // `tests/seam_disposition.rs`'s row for it, beside every other decision of this cut.
    #[command(hide = true)]
    Tick {
        /// The thread to re-check. Keyed on the THREAD: a declared channel's `timeout` hangs on
        /// the board it opened, and the board's own return address is what the routing resolves
        /// back through.
        #[arg(long)]
        thread: String,
    },
    /// Print an embedded, offline guide. Omit the topic to list the available ones.
    ///
    /// One of the three building blocks' guide verbs (6j6v.9e3r): `nxc guide` serves CHAT's
    /// topics, `nxf guide` flow's, `nxm guide` memory's, and `nxs guide` fans out over the active
    /// modules. Needs no workspace — the content is compiled into the binary.
    Guide {
        /// Guide topic; omit to list topics.
        topic: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SessionAction {
    /// Bind a previously-minted internal session id to the real Claude Agent SDK session id the
    /// SDK handed back. Called by the sidecar once a session starts; an unknown `internal` id is
    /// reported as `not_found`.
    Bind {
        /// The internal (nxc-minted) session id.
        internal: String,
        /// The real Claude Agent SDK session id.
        real: String,
    },
    // USER-VISIBLE `--help` text below, so it carries no board id — `Tick`'s rule, for its reason.
    // The rest of the reasoning: this is the sidecar's SECOND callback, beside `bind`, and it is
    // what nxf 6j6v.10yb needs. A channel declared `working_tree: exclusive` opens its next step
    // once the previous one has answered AND its session is over; "over" cannot be observed from
    // outside at the moment it matters, because the process making the announcement is by definition
    // still alive while it speaks. Measured cost of ending a step on the answer alone: a coder
    // replied at 00:16:33 and wrote on until 00:34:27 while the next step ran beside it in the same
    // checkout.
    /// Report that a session's own process is over. Called by the sidecar's teardown as its last
    /// act; an unknown `internal` id is reported as `not_found`.
    ///
    /// **Not a way to stop a session** — nothing here ends anything. It records a fact only the
    /// ending session can know, so that a channel which needs the working copy to itself can open
    /// its next step on the session being over rather than on a message having arrived.
    ///
    /// Idempotent: the first announcement wins, and a repeat is a clean no-op.
    Ended {
        /// The internal (nxc-minted) session id whose process has ended.
        internal: String,
    },
    // USER-VISIBLE `--help` text below, so it carries no board id — `Tick`'s rule. This is the
    // sidecar's THIRD callback, beside `bind` and `ended`, and nxf 6j6v.npy3 is what needs it: an
    // exhausted quota used to be indistinguishable from a crash, so the runtime posted "I cannot
    // carry this out" in the session's name — false — and the operation stopped on NEEDS DECISION
    // with nothing anywhere able to restart it. Hidden from `--help` for `Tick`'s reason: nobody
    // types this. `nxc resume` is the verb a person runs, and it is not hidden.
    #[command(hide = true)]
    /// Report that a session stopped because the model was not available to it. Called by the
    /// sidecar's teardown; an unknown `internal` id is reported as `not_found`.
    ///
    /// **Not a failure and not an ending** — the session is ON HOLD. Its thread keeps owing an
    /// answer, because the session that owes it is coming back, and the operation is marked
    /// interrupted rather than escalated.
    ///
    /// Idempotent: the first announcement wins, and a repeat is a clean no-op.
    Interrupted {
        /// The internal (nxc-minted) session id that stopped.
        internal: String,
        /// The runtime's own name for the boundary — which window, or the error class where it
        /// named no window.
        #[arg(long)]
        limit: String,
        /// When the boundary falls (RFC3339). Omit when the runtime stated no instant: the hold is
        /// still recorded and the way back is then `nxc resume --thread <id>`, run by hand.
        #[arg(long)]
        until: Option<String>,
        /// The runtime's own sentence about it, for a human reading a line.
        #[arg(long, default_value = "")]
        detail: String,
    },
    // USER-VISIBLE `--help` text below; the board id and the argument stay here, `Tick`'s rule.
    // This is the READ opposite of the two writes above (nxf 6j6v.h383) — the same rows, the same
    // seam. Both of them were written by the sidecar and neither could be asked for back, so the
    // coordinator of a six-hour run in a foreign project answered "is this thread thinking or is it
    // dead?" with `ps -p $(cat .nxs/agent-logs/<id>.pid)`: a file in a private directory whose
    // format is promised nowhere, read to get at a fact this store already holds.
    //
    // ONE verb for both questions that run asked, through a scope rather than a second verb: name a
    // session and you get that session, name a thread and you get everything that ran on it. The
    // direction here is to take verbs AWAY (6j6v.dvyq), and two spellings of one read would be the
    // shape that direction is against.
    // USER-VISIBLE `--help` text below; the board id stays here, `Tick`'s rule. This is the
    // scheduled job of nxf 6j6v.gn8b: `session ended` ARMS it, the background service runs it once
    // the grace has passed, and the tick's own sweep is the backstop for a caller that was killed
    // before it could announce anything. Hidden from `--help` for `Tick`'s reason — nobody types
    // this either, and it exists so that something can.
    #[command(hide = true)]
    /// Hand a caller everything that arrived while it was working: one resume with every answer
    /// held for this session, naming what is still outstanding.
    ///
    /// IDEMPOTENT and safe to run twice: an empty queue is a clean no-op, and the held answers are
    /// released only once the resume has actually been handed over.
    Deliver {
        /// The internal (nxc-minted) session id whose held answers to deliver.
        internal: String,
    },
    /// Say what became of a session: running, ended (and when), or neither.
    ///
    /// Name a session to ask about that one — "is this still alive, or is it dead?". Name
    /// `--thread` instead to get every session that ran on a thread, ended ones included — "is the
    /// previous one really over before I touch this?".
    ///
    /// `unknown` is not a hedge: it means nothing announced an end and no live process answers for
    /// it — a session killed hard, or one this machine never ran.
    State {
        /// The internal (nxc-minted) session id — the value `send --to` hands back as `session`
        /// and `refs.session_id` carries, not the runtime's own id.
        #[arg(conflicts_with = "thread", required_unless_present = "thread")]
        internal: Option<String>,
        /// A thread: report every session recorded under it instead of one named session.
        #[arg(long)]
        thread: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum TranscriptAction {
    /// Append normalized transcript entries, read as JSON-lines from STDIN, to a session's
    /// transcript (`seq` is assigned by the store, so repeated calls for the same session — the
    /// `resume` case — continue it rather than colliding).
    ///
    /// THE SIDECAR'S CALLBACK CONTRACT, like `session bind`'s: `agent-sidecar/src/main.mjs` already
    /// shells out to exactly `nxc transcript append --session <internal>` and writes one
    /// `JSON.stringify(entry)` per line to this process's stdin. The flag NAME, the stdin framing,
    /// and the `--json` record are therefore all load-bearing — an already-committed producer
    /// depends on them. Blank lines are framing (the sidecar ends its batch with a newline) and are
    /// skipped; a line that does not parse, or an entry with an empty `kind`, is a `validation`
    /// error naming the line — the sidecar propagates everything except clap's "unrecognized
    /// subcommand", so a silent skip here would lose transcript with no trace anywhere.
    Append {
        /// The INTERNAL (nxc-minted) session id whose transcript this is — the same id
        /// `session bind` maps to the real SDK session, not the real SDK id itself.
        #[arg(long)]
        session: String,
    },
    /// Render a session's stored transcript as a timeline: the main conversation
    /// in `seq` order, with each Task-spawned subagent's own entries nested under the `tool_use`
    /// that spawned them. `--json` is the same transcript record an embedding app receives.
    ///
    /// An OPERATOR surface, with no membership gate:
    /// a transcript has no channel to gate on, and `agent_transcript` is device-local and never
    /// synced, so a gate here would buy nothing anyone with the workspace file cannot already do
    /// with `sqlite3`. Note what that means for the OUTPUT though: it contains raw tool inputs and
    /// results — whatever the agent read, wrote, or ran — so treat a dumped transcript like the
    /// workspace db itself, not like a message log, when pasting it into a bug report.
    ///
    /// An unknown session renders an EMPTY transcript rather than failing: a session whose sidecar
    /// never flushed is indistinguishable from one that had nothing to say.
    Show {
        /// The INTERNAL (nxc-minted) session id — the id `send --to <persona>` returns as
        /// `session` and `refs.session_id` carries, not the real SDK id.
        session: String,
        /// Start AFTER this `seq` instead of at the beginning — the paging cursor (nxf 6j6v.t7pa).
        /// Pair it with `--limit` to walk a long session in chunks: pass the largest `seq` the last
        /// chunk showed, and a chunk shorter than `--limit` is the end.
        ///
        /// A subagent entry whose spawning `tool_use` fell outside the window is shown at top level
        /// with its `subagent_type` — nothing is hidden, but the nesting of a WINDOW is the nesting
        /// of the rows in it. Read without these flags for the exact tree.
        /// `allow_negative_numbers` because `-1` is the "from the start" sentinel the read itself
        /// uses (`seq` starts at 0), so a script walking the cursor can pass it back verbatim
        /// instead of special-casing the first call into "omit the flag".
        #[arg(long, value_name = "SEQ", allow_negative_numbers = true)]
        from_seq: Option<i64>,
        /// Show at most this many entries. Counts stored entries, nested subagent ones included —
        /// that is what bounds the memory this read holds, and a single `Task` can carry hundreds of
        /// nested entries. Zero or negative reads as "no limit", the same way the library seam
        /// treats it (and the same way SQLite spells it).
        ///
        /// `allow_negative_numbers` to match `--from-seq` beside it: without it clap rejects
        /// `--limit -1` before the value is ever seen, which left the store's own
        /// "negative means no limit" contract unreachable from the command line (PR review, Test
        /// Quality #3).
        #[arg(long, value_name = "N", allow_negative_numbers = true)]
        limit: Option<i64>,
    },
    /// Retire the transcripts of sessions nobody has written to for a while, and say what went
    /// (nxf 6j6v.t7pa). WHOLE sessions, aged on their LAST recorded entry — a session still being
    /// written to is never a candidate, however long it has been running, and a long-running one is
    /// never cut in half.
    ///
    /// **You do not have to run this to keep the table bounded**: the same retention rides the first
    /// flush of every new role session. Reach for it to clear out a workspace that has gone quiet —
    /// where no new session is coming to trigger that — or to apply a tighter window once.
    ///
    /// Sessions carrying no readable timestamp are of unknown age and are KEPT, and reported
    /// separately so they are a visible gap rather than a silent one.
    ///
    /// This stops `.nxs/db.sqlite` growing; it does not make it smaller. The freed pages are reused
    /// by later writes, but shrinking the file itself takes a `VACUUM`, which is not run here — it
    /// rewrites the whole shared workspace database under an exclusive lock.
    Prune {
        /// Keep sessions whose last entry is within this many days. Defaults to the configured
        /// window (`NXC_TRANSCRIPT_KEEP_DAYS`, itself defaulting to 30 days).
        #[arg(long, value_name = "DAYS")]
        keep_days: Option<i64>,
        /// Report what would be removed and remove nothing. Takes no write lock either, so it is
        /// safe to point at a busy workspace.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ThreadsAction {
    /// List the caller's threads (membership-scoped), each with its bulk quorum state.
    List {
        /// List as this handle's threads (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
        /// Restrict to one channel.
        #[arg(long)]
        channel: Option<String>,
    },
    /// Show one board in full: the quorum state plus the reply messages in order.
    Show {
        /// The thread id to show.
        thread_id: String,
        /// Read as this handle (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
    },
    /// Give a conversation its NAME — once (nxf 6j6v.e76c). Without `<name>` the name is DERIVED
    /// from the thread's opening message, which is what the run `send` commissions does; a thread
    /// that already has one keeps it.
    Name {
        /// The thread to name.
        thread_id: String,
        /// The name to give it, at most seven words. Omitted, it is derived from the thread's
        /// opening message.
        name: Option<String>,
    },
    // `expect` was here (M2's opener-only re-declare). REMOVED by 6j6v.dvyq §3 — the owner's
    // reason, verbatim: "sehe ich nicht, wofür das notwendig ist". The WRITE survives on the seam
    // as `facade::set_expects`, which is what the channel supervisor (6j6v.hq71/6j6v.pf6j) uses to
    // open a role's next turn; only the human-typed door is gone. `list`/`show` leave later, with
    // the rest of this group.
}

/// The fully-built clap command tree for `nxc`.
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// Entry point for the `nxc` binary. [`run_from`] is the multicall seam the `nxs` umbrella routes
/// `nxc`/`nxs chat` through.
pub fn run() -> ExitCode {
    run_from(std::env::args_os())
}

/// Run the `nxc` surface over an explicit argv INCLUDING the program name at index 0 (multicall
/// seam, mirrors `nxm`).
pub fn run_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    run_from_with(args, None, None)
}

/// **Where the sibling modules' session-start blocks come from, for this process** (nxf 6j6v.k8zq)
/// — set once by [`run_from_with`] at the CLI entry and read by [`CliCtx::resolve`].
///
/// Process-wide, and that is a deliberate exception with a boundary: this is the ADAPTER, the layer
/// that already resolves `NXC_NOW`, `NXC_TIMER` and the workspace from the ambient process, and the
/// value is written exactly once before any verb runs. Everything BELOW it — [`crate::orchestration::Ctx`],
/// [`crate::engine::Engine`] — takes the provider as a parameter, so the library keeps having no
/// ambient environment. Threading it through the eight `CliCtx::resolve` call sites instead would
/// put the same one-time constant in eight signatures to say the same thing.
static MODULE_PRIMES: std::sync::OnceLock<&'static dyn crate::facade::ModulePrimes> =
    std::sync::OnceLock::new();

/// **Which machine this is, who else is online, and the per-machine claims** (nxf 6j6v.1c6k) —
/// set once by [`run_from_with`] for [`MODULE_PRIMES`]'s reason: the composition root is the one
/// place that links the service home and a relay client. `None` for anyone linking this CLI on its
/// own, which designates no machine and starts every persona where it is summoned, as before.
static MACHINES: std::sync::OnceLock<&'static dyn crate::machine::Machines> =
    std::sync::OnceLock::new();

/// [`run_from`] with the sibling modules' prime source injected — what the `nxs` multicall binary
/// calls, because the composition root is the one place that knows the module registry.
///
/// Calling it twice in one process keeps the FIRST provider (`OnceLock`), which is what a CLI entry
/// means: there is one.
pub fn run_from_with<I, T>(
    args: I,
    module_primes: Option<&'static dyn crate::facade::ModulePrimes>,
    machines: Option<&'static dyn crate::machine::Machines>,
) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    if let Some(source) = module_primes {
        let _ = MODULE_PRIMES.set(source);
    }
    if let Some(machines) = machines {
        let _ = MACHINES.set(machines);
    }
    let cli = Cli::parse_from(args);
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(error::emit(&e, cli.json) as u8),
    }
}

/// **The message body, from whichever of its three sources the caller used** (nxf 6j6v.s46h) —
/// the positional, `-` on the positional for STDIN, or `--body-file <path>` (whose own `-` is the
/// same sentinel).
///
/// A thin adapter over [`nxs_foundation::text_input`], which is `nxf`'s
/// `--description-file` / `--set field=-` seam and now the ONE implementation both surfaces call.
/// Shared rather than re-written here because the alternative is `-` coming to mean one thing in
/// `nxf` and another in `nxc` — and because what a caller must be able to trust across both is
/// exactly the part that is easy to re-implement differently: the text is read VERBATIM (no
/// trailing-newline trimming), capped, UTF-8-enforced, and two sources for one body are refused
/// before anything is written.
///
/// `nxc` has one long text per call, so the [`StdinReader`](nxs_foundation::text_input::StdinReader)
/// is built here and dropped here; the multi-field bookkeeping it exists for is flow's.
///
/// The `None` arm is unreachable through clap (`required_unless_present = "body_file"` makes one
/// of the two mandatory) and is an error rather than an `expect` for the reason the flag's own note
/// gives: the day somebody relaxes that attribute, this should say what is missing instead of
/// aborting the process.
///
/// **An EMPTY read is refused, and that is the one trap the sentinel brings with it.** Every text
/// that names the STDIN form shows it with the heredoc that feeds it
/// ([`crate::role`]'s forced ending, [`crate::addressees`]'s wake lines, the sidecar's reminder) —
/// but a line that LOOKS runnable gets copied and run, and `nxc reply --thread <id> -` with nothing
/// on STDIN posts an empty answer, discharges the expectation and ends the round with no result at
/// all. Nobody would ever see an error, because there is none to see. So the mis-copy costs a
/// message on stderr instead of a lost round.
///
/// **On the STDIN/file path only.** An empty ARGUMENT (`nxc reply --thread <id> ""`) is typed on
/// purpose, it behaved this way before this item and refusing it is a different decision than the
/// one being made here. Both halves of that scope are pinned —
/// `a_body_given_on_stdin_arrives_unchanged.rs` asserts the refusal AND the argument that still
/// goes through, because without the second one a later `reads_a_stream &&` deletion would pass
/// every test in silence (PR #463 review, Test Quality).
///
/// **THE SIZE CEILING IS INHERITED, NOT CHOSEN, and this note is what keeps that visible** (PR
/// #463 review, Integrity & Robustness; the decision itself is nxf 6j6v.fqfc). A body is capped at
/// `nxs_foundation::text_input`'s 64 MiB cap, a number picked for a flow item's
/// DESCRIPTION. Until this item the ceiling was not in the code at all but in the kernel — the
/// body arrived as argv, so `MAX_ARG_STRLEN` held it to 128 KiB on Linux and `ARG_MAX` to under
/// 1 MiB on macOS — and it has risen about 500-fold. That matters more here than for a flow field
/// because a message is written as an op, folded into `messages`, and SYNCED TO EVERY PEER, which
/// keeps it forever. Nothing has gone wrong (a verdict or a report is far under 1 MiB); what is
/// missing is a decision, and picking a second number unilaterally would put two caps in the tree
/// that can drift apart. So the inherited one stands, said out loud rather than assumed.
fn resolve_body(body: Option<&str>, body_file: Option<&str>) -> Result<String> {
    use nxs_foundation::text_input::{resolve, StdinReader, STDIN_SENTINEL};
    let reads_a_stream = body_file.is_some() || body == Some(STDIN_SENTINEL);
    let mut stdin = StdinReader::new();
    let resolved = resolve(&mut stdin, "body", body, body_file)?.ok_or_else(|| {
        NxfError::validation(
            "no message body: pass it as the argument, as `-` to read it from STDIN, or with \
             --body-file <path>",
        )
    })?;
    if reads_a_stream && resolved.trim().is_empty() {
        return Err(NxfError::validation(format!(
            "the message body read from {} is empty; a reply with nothing in it would discharge \
             the thread's expectation and end the round with no result — write the text into the \
             heredoc (`nxc reply --thread <id> - <<'EOF'` … `EOF`) or pass a file that has it",
            match body_file {
                Some(path) if path != STDIN_SENTINEL => format!("file '{path}'"),
                _ => "STDIN".to_string(),
            }
        )));
    }
    Ok(resolved)
}

fn dispatch(cli: &Cli) -> Result<()> {
    let db = cli.db.as_deref();
    match &cli.command {
        Command::Init { quiet } => init(cli.json, *quiet),
        Command::AgentManifest => agent_manifest(cli.json),
        Command::Prime { consumer, persona } => {
            prime(cli.json, db, consumer.as_deref(), persona.as_deref())
        }
        Command::List { persona } => list(cli.json, db, persona.as_deref()),
        Command::Send {
            body,
            body_file,
            refs,
            no_ref,
            to,
            stream,
            machine,
        } => {
            let body = resolve_body(body.as_deref(), body_file.as_deref())?;
            send(
                cli.json,
                db,
                &body,
                refs,
                *no_ref,
                to,
                *stream,
                machine.as_deref(),
            )
        }
        Command::Reply {
            body,
            body_file,
            escalate,
            needs_rework,
            accept,
            thread,
            if_unanswered,
            stream,
            machine,
        } => {
            let body = resolve_body(body.as_deref(), body_file.as_deref())?;
            reply_thread(
                cli.json,
                db,
                ReplyArgs {
                    body: &body,
                    escalate: *escalate,
                    needs_rework: *needs_rework,
                    accept: *accept,
                    thread: thread.as_deref(),
                    if_unanswered: *if_unanswered,
                    stream: *stream,
                    machine: machine.as_deref(),
                },
            )
        }
        Command::Machine {
            to,
            thread,
            machine,
        } => machine_verb(
            cli.json,
            db,
            to.as_deref(),
            thread.as_deref(),
            machine.as_deref(),
        ),
        Command::PickUp => pick_up_verb(cli.json, db),
        Command::Threads { action } => threads(cli.json, db, action),
        Command::Status {
            thread,
            channel,
            all,
        } => status(cli.json, db, thread.as_deref(), channel.as_deref(), *all),
        Command::Search { query, consumer } => search(cli.json, db, query, consumer.as_deref()),
        Command::Session { action } => session_cmd(cli.json, db, action),
        Command::Transcript { action } => transcript_cmd(cli.json, db, action),
        Command::Withdraw { thread } => withdraw(cli.json, db, thread),
        Command::Resume {
            thread,
            session,
            force,
        } => resume(cli.json, db, thread.as_deref(), session.as_deref(), *force),
        Command::Tick { thread } => tick(cli.json, db, thread),
        Command::Guide { topic } => crate::guide::guide(cli.json, topic.as_deref()),
    }
}

// ---- shared helpers --------------------------------------------------------

/// The acting agent's bare name: `NXC_ACTOR`, else `USER`, else `nxc` (mirrors `nxm`). A
/// set-but-empty variable counts as unset (invariant 1, 6j6v.xsf3; see
/// `nxs_foundation::model::resolve_author`) — it rides straight into `op.author` via
/// [`actor_qualified`], and chat is the surface where an op authorizes an agent ACTION.
fn actor() -> String {
    nxs_foundation::model::resolve_author(
        std::env::var("NXC_ACTOR").ok(),
        std::env::var("USER").ok(),
        "nxc",
    )
}

/// The minting workspace identity: `NXC_ORIGIN`, else `ws`'s own replica prefix
/// ([`crate::workspace::origin_of`]).
/// The org/repo-qualified form is the federation bridge (`6j6v.kz8p`); M1 only needs a stable field.
///
/// **The default is the WORKSPACE's answer, not a literal spelled here** (nxf 6j6v.07me, owner
/// decision of 2026-08-21). It used to be the constant `"local"`, and the library seam used to
/// answer with the same constant; the owner replaced both with the replica prefix in one move, so
/// this fallback and [`crate::engine::Engine::origin`] still read ONE value out of ONE source. They
/// have to: `<origin>/<agent>` is written into message senders, `expects_reply_from`, session maps
/// and claim threads and matched later by exact string, so if an app minted `<prefix>/coder` while a
/// terminal minted `local/coder`, one declared channel would quietly have two disjoint member sets
/// and nothing would report it. `tests/parity.rs`'s
/// `both_adapters_resolve_one_origin_for_one_workspace` is the gate on that agreement.
///
/// The env READ stays here, at the adapter, exactly as `NXC_ACTOR`/`NXC_HOP`/`NXC_NOW` do: a spawned
/// `nxc` is handed its origin by [`crate::orchestration::trigger_env`] and has no other way to be
/// told. That stamp is now the resolved prefix rather than `"local"`, which is what keeps a whole
/// spawn chain minting under the identity its root resolved — even if `adopt_prefix` rewrites the
/// file mid-chain (the accepted hazard, nxf 6j6v.dnzt).
fn origin_in(ws: &Workspace) -> String {
    std::env::var("NXC_ORIGIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::workspace::origin_of(ws).to_string())
}

/// [`origin_in`] for a call site that has not resolved its workspace yet — it resolves one from
/// `--db`/cwd exactly as [`open`], [`workspace_root`] and [`declaration_dir`] next door do.
///
/// **Why this takes `db` at all** (nxf 6j6v.07me): the origin stopped being a value the process
/// environment alone can answer, so the function that answers it needs a workspace, and every
/// caller now says which one. Threading `db` was chosen over a lazily-memoized ambient workspace
/// because it keeps the file's ONE existing resolution rule — "`--db` if given, else discover from
/// cwd" — with no second, invisible copy of it, and because every call site already had `db` in
/// hand.
///
/// **When no workspace resolves, this FAILS** — `Workspace::resolve` returns the named
/// `no_workspace` error ("no `.nxs` workspace found from …; run `nxs init`"), and it propagates.
/// There is deliberately no literal fallback: an origin invented for a workspace-less invocation
/// would be a wrong identity written into durable handles, which is the failure this whole item
/// exists to prevent, whereas a missing workspace is a condition the user can see and fix. In
/// practice no invocation reaches this first: all six `--consumer` defaulting sites open the store
/// over the same `db` beforehand ([`resolve_consumer`]), so the identical error has already been
/// raised there.
fn origin(db: Option<&str>) -> Result<String> {
    Ok(origin_in(&Workspace::resolve(db, &cwd()?)?))
}

/// The caller's qualified handle `<origin>/<agent>` — the default `--consumer` and message `sender`.
fn caller_handle(db: Option<&str>) -> Result<String> {
    Ok(format!("{}/{}", origin(db)?, actor()))
}

/// An explicit `--consumer`, else the caller's own handle ([`caller_handle`]) — the one place the
/// six read verbs that take that flag say what its absence means.
///
/// Written as a helper rather than repeated because resolving the caller's handle now needs a
/// workspace and so returns a `Result`: `unwrap_or_else(caller_handle)` no longer typechecks, and
/// six hand-rolled `match`es would be six chances to resolve a different workspace than the store
/// was opened over. It stays LAZY in the same way the old `unwrap_or_else` was — an explicit
/// `--consumer` resolves no workspace.
fn resolve_consumer(consumer: Option<&str>, db: Option<&str>) -> Result<String> {
    match consumer {
        Some(c) => Ok(c.to_string()),
        None => caller_handle(db),
    }
}

/// The ambient caller session: `NXC_SESSION` — the internal session id the
/// `SidecarWorker` stamps into a triggered role's env (see `worker.rs`) so its in-session `nxc`
/// calls resolve their own caller — else `None` when there is no real ambient session (review
/// finding #2): a bare-terminal kickoff has no live session to route a follow-up back to, and
/// stamping a fixed sentinel string in its place would be a false signal that could later be
/// mistaken for a genuine session identity. That is the same refusal [`origin`] makes for its own
/// value since nxf 6j6v.07me — it used to answer a workspace-independent `"local"` and now either
/// resolves the real one or fails; the difference is only that a missing origin is an error while a
/// missing session is a legitimate `None`. Only ONE call site
/// (`fn send`'s refs-defaulting) reads this; it leaves `refs.session_id` unset (not a placeholder)
/// when this returns `None`.
fn session() -> Option<String> {
    std::env::var("NXC_SESSION").ok().filter(|s| !s.is_empty())
}

/// Spawned iff any of NXC_ACTOR/NXC_WORKER/NXC_SESSION is set in this process's env — the sidecar
/// stamps all three into a triggered role's environment (`worker.rs`); their total absence is what
/// marks a human-at-the-keyboard interactive session. Used ONLY by `prime`'s rendering
/// decision (surface declaration errors, declaration warnings and the writing-declarations pointer
/// iff NOT spawned) — deliberately reads raw env PRESENCE via
/// `std::env::var(...).is_ok()`, NOT `actor()`/`hop()`'s own defaulting readers, which always return
/// a value regardless of whether the var was actually set (the wrong question here: this needs "was
/// it SET", not "what does it resolve to"). `session()` IS reused for the `NXC_SESSION` check since
/// it already answers exactly "was it set to a non-empty value" (same question, just already named).
fn is_spawned_context() -> bool {
    std::env::var("NXC_ACTOR").is_ok() || std::env::var("NXC_WORKER").is_ok() || session().is_some()
}

/// The role-runtime depth-guard counter this invocation CLAIMS:
/// `NXC_HOP`, defaulting to `0` for a fresh (non-role-triggered) invocation. An unset or unparseable
/// value reads as `0`, never a hard error. `trigger_env` propagates the depth of the session being
/// triggered into its env so this read advances with the chain (review finding #1) — one deeper for
/// a commission, unchanged for a wake carrying a result back.
///
/// **Claimed, not authoritative** since nxf 6j6v.m48m. This variable lives in the environment of a
/// process that is often a Bash-enabled role, so it can be rewritten by exactly the thing the guard
/// exists to bound. [`crate::orchestration::resolve_hop`] therefore reads the depth back from the
/// session map — written by whoever spawned this session — and treats what this returns as a FLOOR
/// it may not fall below. Which is also why the CLI no longer runs a guard of its own before the
/// store opens: there is nothing left to check without the store, and one check in the shared verb
/// beats two that could disagree.
fn hop() -> u32 {
    std::env::var("NXC_HOP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// The `created` display timestamp: a pinned `NXC_NOW` (golden determinism), else the system clock.
///
/// **The CLI keeps resolving this and keeps PASSING it** (nxf 6j6v.07me). `Caller::now` became an
/// override with the clock as its default on the library seam; nothing about that reaches here,
/// because every `Ctx` this file builds carries what this returns. The pin is what makes `--json`
/// byte-stable, and it stays a CLI concern — the shared half is only the formatting, which
/// [`crate::orchestration::system_now`] now owns so both seams render one instant identically.
fn resolve_now() -> Result<String> {
    match std::env::var("NXC_NOW").ok().filter(|s| !s.is_empty()) {
        Some(s) => Ok(s),
        None => crate::orchestration::system_now(),
    }
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))
}

/// Resolve the workspace and open chat's store over it.
fn open(db: Option<&str>) -> Result<ChatStore> {
    let ws = Workspace::resolve(db, &cwd()?)?;
    ws.open_chat_store()
}

/// The workspace ROOT this invocation acts in — the parent of `.nxs/`, which is where
/// `.nxs-personas/` (and any legacy `roles/`) sits.
fn workspace_root(db: Option<&str>) -> Result<PathBuf> {
    Ok(Workspace::resolve(db, &cwd()?)?
        .workspace_root()?
        .to_path_buf())
}

/// The declaration directory this invocation READS — `.nxs-personas/`, or a legacy `roles/` folder
/// when that is the one carrying declarations (nxf 6j6v.dvyq step 4). One resolution, owned by
/// [`crate::definitions::DeclarationSource::locate`], so the CLI and `Engine` cannot disagree.
fn declaration_dir(db: Option<&str>) -> Result<PathBuf> {
    Workspace::resolve(db, &cwd()?)?.declaration_dir()
}

/// Everything [`crate::orchestration::Ctx`] borrows, owned — the CLI adapter's job in one place.
///
/// The orchestration verbs take their inputs explicitly so a library caller can supply them;
/// resolving those inputs from the process environment and the workspace is exactly what makes the
/// CLI the CLI. Every verb in this file that reaches into `orchestration` goes through here rather
/// than assembling its own, so there is one answer to "where does the CLI's `now`/`actor`/`origin`/
/// worker/catalogue come from".
struct CliCtx {
    defs: crate::definitions::Definitions,
    worker: LazyWorker,
    /// The CLI's timer, resolved from `NXC_TIMER` HERE at the adapter (PR #269 review, Code Quality
    /// #3) rather than from inside the orchestration verbs, which is where it used to be read. Same
    /// move `worker` already made: the env dependency survives, in exactly one place, at the edge.
    timer: LazyTimer,
    /// The CLI's namer, resolved from `NXC_NAMER` at this same adapter and for the same reason
    /// (nxf 6j6v.e76c) — see [`LazyNamer`].
    namer: LazyNamer,
    now: String,
    origin: String,
    actor: String,
    session: Option<String>,
    db_path: String,
    project_claude_md: Option<String>,
    /// Where the board's and the memory's session-start blocks come from when this process summons
    /// a persona (nxf 6j6v.k8zq). Injected by the umbrella binary through [`run_from_with`] — the
    /// composition root is the one place that knows the module registry — and `None` for anyone
    /// linking this CLI on its own, which composes chat's own half alone.
    module_primes: Option<&'static dyn crate::facade::ModulePrimes>,
    hop: u32,
}

impl CliCtx {
    /// Resolve the ambient context. `hop` is the CURRENT (pre-increment) depth-guard counter as this
    /// process CLAIMS it ([`hop`]); the verb resolves the authoritative one from the store.
    ///
    /// **One `Workspace::resolve`, not three** (nxf 6j6v.07me). This used to resolve twice — once
    /// through `workspace_root(db)` and once inline for `db_path` — and `origin` becoming a
    /// workspace question would have made it three. All three answers come off the same resolved
    /// workspace instead, which is not just cheaper: `origin` and `db_path` now provably name the
    /// SAME workspace, and `origin`+`db_path` are exactly the pair
    /// [`crate::orchestration::trigger_env`] stamps into a spawned hop's environment.
    fn resolve(db: Option<&str>, hop: u32) -> Result<CliCtx> {
        let ws = Workspace::resolve(db, &cwd()?)?;
        let root = ws.workspace_root()?.to_path_buf();
        Ok(CliCtx {
            defs: crate::definitions::Definitions::resolve(&root)?,
            // **THE WORKSPACE ROOT, not the process cwd** (nxf 6j6v.npn0). `Workspace::resolve`
            // above walked UPWARDS to find the `.nxs/`, so an invocation from a subdirectory is
            // ordinary — and this one path decides three things at once in `SidecarWorker`: where
            // `.nxs/agent-logs/` is written, what the spec tells the session its working directory
            // is, and what the spawned process is `chdir`'d into. Built from the cwd, a
            // `nxc send` from `crates/chat/` created `crates/chat/.nxs/agent-logs/` and rooted the
            // agent there; worse, a LATER call from a different directory looked for the pid file
            // where it was not and read `false` for "is this session still running" — the
            // destructive direction at every site that asks, the channel-advance gate of nxf
            // 6j6v.10yb included.
            worker: LazyWorker { cwd: root.clone() },
            timer: LazyTimer,
            namer: LazyNamer,
            now: resolve_now()?,
            origin: origin_in(&ws),
            actor: actor(),
            session: session(),
            db_path: ws.db_path_str()?,
            // The project CLAUDE.md is optional — a workspace without one still composes cleanly.
            project_claude_md: std::fs::read_to_string(root.join("CLAUDE.md")).ok(),
            module_primes: MODULE_PRIMES.get().copied(),
            hop,
        })
    }

    fn ctx(&self) -> crate::orchestration::Ctx<'_> {
        crate::orchestration::Ctx {
            now: &self.now,
            origin: &self.origin,
            actor: &self.actor,
            session: self.session.as_deref(),
            hop: self.hop,
            defs: &self.defs,
            worker: &self.worker,
            timer: &self.timer,
            namer: &self.namer,
            db_path: &self.db_path,
            project_claude_md: self.project_claude_md.as_deref(),
            module_primes: self.module_primes,
            machines: MACHINES.get().copied(),
        }
    }
}

/// The CLI's timer, resolved from `NXC_TIMER` on FIRST USE — [`LazyWorker`]'s twin, lazy for the
/// same reason (PR #269 review, Code Quality #3): most CLI verbs never schedule anything, and an
/// unknown `NXC_TIMER` value should be a loud error only for the ones that do, exactly as it was
/// when `select_timer()` was called from inside the verb itself.
struct LazyTimer;

/// Resolves the real namer from `NXC_NAMER` on FIRST USE — [`LazyTimer`]'s twin, lazy for its
/// reason: most CLI verbs open no thread at all, and an unknown `NXC_NAMER` should be a loud error
/// only for the ones that do.
struct LazyNamer;

impl crate::naming::Namer for LazyNamer {
    fn commission(&self, run: crate::naming::Naming<'_>) -> Result<()> {
        crate::naming::select_namer()?.commission(run)
    }
    fn derive(&self, body: &str) -> Option<String> {
        crate::naming::select_namer().ok()?.derive(body)
    }
}

impl crate::timer::Timer for LazyTimer {
    fn schedule(
        &self,
        thread_id: &str,
        deadline: &str,
        command: &str,
    ) -> Result<crate::timer::TimerHandle> {
        crate::timer::select_timer()?.schedule(thread_id, deadline, command)
    }
    fn cancel(&self, handle: &crate::timer::TimerHandle) -> Result<()> {
        crate::timer::select_timer()?.cancel(handle)
    }
    /// **Forwarded, not inherited** (nxf 6j6v.0j12). The trait defaults this to `None`, which is the
    /// right answer for a backend that does not depend on the service — and the wrong one here,
    /// where the answer belongs to whichever backend `NXC_TIMER` names. A wrapper that silently
    /// keeps a default is a wrapper that reports "nothing to see" on every CLI call there is; it
    /// cost this ticket one red test to notice.
    ///
    /// An unresolvable `NXC_TIMER` is `None` rather than an error: this rides along on calls that
    /// have already succeeded, and the verb that genuinely needs the timer says so loudly on its own.
    fn service_fault(&self) -> Option<nxs_service::ServiceFault> {
        crate::timer::select_timer().ok()?.service_fault()
    }
}

/// Resolves the real worker on FIRST USE rather than when the context is built.
///
/// The orchestration verbs take a worker up front, but plenty of CLI paths that build a context
/// never trigger anything — `nxc list`, `search`, `status`, `threads show`. Resolving eagerly would
/// make those require `NXC_SIDECAR` to be set, which they never did before. (The two examples that
/// stood here, a positional `nxc send <channel> <body>` and an `ask` on a non-declared channel, are
/// both invalid syntax since 6j6v.dvyq §3 — every `send` that parses now triggers something.) Re-resolving per trigger is also exactly what the code did before this epic
/// (`select_worker` was called at each trigger site), so this is not a new cost.
///
/// **`cwd` IS THE WORKSPACE ROOT** (nxf 6j6v.npn0) — see [`CliCtx::resolve`], which is the only
/// place that builds one. Not the process working directory: `Workspace::resolve` walks upwards to
/// find the `.nxs/`, so a call from a subdirectory is ordinary, and this path decides both where a
/// session's `.nxs/agent-logs/` goes AND where a later call looks for its pid file.
///
/// **The one residual, and it IS a regression for one access pattern** (corrected after the review
/// of PR #375, Integrity #1 — the first version of this paragraph said "no answer gets worse than it
/// was", which is too kind). A session that was ALREADY RUNNING when this change landed, and that
/// was started from a subdirectory, has its pid file under that subdirectory and nothing here will
/// find it again. For an operator who called `nxc` from all over the tree that is no change: the
/// answer was already `false` from every directory but one. For an operator who worked CONSISTENTLY
/// out of that one subdirectory it is a loss — the liveness check worked for them, and now it reads
/// `false` for a process that is alive.
///
/// **What that costs is the failure nxf 6j6v.10yb exists to prevent**, reached through a misread
/// instead of through a finished session: `false` opens the advance gate, and the next step walks
/// into a checkout the previous one is still writing to. Bounded to that ONE session (every trigger
/// from then on writes at the root) and self-healing, but not free — pinned as a fact, with its
/// bound, by `tests/the_worker_stands_at_the_workspace_root.rs::a_session_whose_pid_file_still_sits_
/// in_a_subdirectory_reads_as_not_running_and_the_gate_opens`, and carried to the operator in the
/// changelog, which is what `nxs self-update` renders at exactly the moment this matters.
///
/// **Still no fallback scan for stray `.nxs/agent-logs/` directories, and no startup warning.** A
/// second place to LOOK for a pid file is a second place a future reader has to keep true, and it
/// would answer from a directory nothing writes to any more — where a claim nothing can judge (the
/// residual `SessionLock` names: a file written before identity was recorded, or a platform that
/// cannot say when a process began) reads as alive forever and wedges both the gate and the sweep's
/// own proof that a holder is dead (`orchestration::holder_is_provably_dead`): a chain that has
/// actually ended would never be recognized as gone, so its working copy would wait out the full
/// two-hour bound instead of being parked early. A pid the system merely REUSED is no longer one of
/// those cases — since nxf 6j6v.b9nf a claim records when its process started, so a stranger
/// holding the number reads as a session that ended.
/// A warning fares no better: nothing ever removes a stray directory, so it would fire for the life
/// of the workspace rather than for the life of the one session it is about. The cleanup of those
/// directories is its own small chore (nxf 6j6v.8h8z).
struct LazyWorker {
    cwd: PathBuf,
}

impl crate::worker::Worker for LazyWorker {
    fn trigger(&self, req: crate::worker::TriggerRequest) -> crate::worker::TriggerResult {
        // `?` on the selection converts through `From<NxfError> for TriggerError`; the inner
        // `trigger`'s own outcome — including a `SessionGone` a host worker raised — passes through
        // untouched, which is what keeps this wrapper invisible to the classification above it.
        crate::worker::select_worker(self.cwd.clone())?.trigger(req)
    }

    /// **A DELEGATOR has to delegate every method, and this one was missed** (nxf 6j6v.10yb).
    ///
    /// `Worker::session_is_running` has a default (`false`), which is what makes it safe to add to a
    /// trait every host implements — and it is also what let this wrapper silently answer for a
    /// worker it never asked. Every CLI call goes through here, so the gate that stops a sequential
    /// channel advancing while a step's session is still writing was inert on the one path that
    /// matters: the tests drove the engine seam with their own worker and were green, and the LIVE
    /// run put the finisher into the coder's checkout eight milliseconds after its answer.
    ///
    /// A selection failure is `false`, not a panic: this is a READ on the way to a decision that has
    /// a safe direction, and the failure it would report — no sidecar configured — is already
    /// reported loudly by `trigger` above the moment anything tries to start a session.
    fn session_is_running(&self, internal_session: &str) -> bool {
        crate::worker::select_worker(self.cwd.clone())
            .is_ok_and(|w| w.session_is_running(internal_session))
    }

    /// **The same delegation, one question up** (nxf 6j6v.t41e) — and the one that would be
    /// hardest to notice, because its default is the answer a WRONG implementation gives too. Every
    /// command-line call resolves its worker here, so a `nxc session state` that did not forward
    /// this would report the shipped sidecar — the one worker in this house that really does read a
    /// pid file — as unable to answer.
    ///
    /// A selection failure is `false` for the liveness read's reason, and here it is not even a
    /// fallback but the literal truth: a worker that could not be built answers nothing at all.
    fn answers_liveness(&self) -> bool {
        crate::worker::select_worker(self.cwd.clone()).is_ok_and(|w| w.answers_liveness())
    }

    /// **The same delegation, and this one was missed until nxf 6j6v.2af2** — the third instance of
    /// the class nxf 6j6v.10yb named at this very wrapper, and the one with the widest blast radius,
    /// because `working_copy` is the only thing that answers WHERE.
    ///
    /// Left on the trait default (`None`), every command-line call told the engine that this host
    /// runs its sessions in no directory anybody can see. Two consequences, both measured by reading
    /// the source rather than inferred:
    ///
    /// * **`nxc tick` could never park.** `orchestration::park_the_stranded_holder` asks this
    ///   question first and answers `ParkRefusal::NoWorkingCopy` when it comes back empty — so the
    ///   whole of nxf 6j6v.de9s was inert on the shipped path: an unanswered escalation under
    ///   contention was never committed onto a branch and the working copy was never handed on. The
    ///   engine SEAM was fine (`Engine` holds the real worker), which is why every test of that
    ///   feature stayed green.
    /// * **No handover could record its anchor** (this item), for the same one reason.
    ///
    /// A selection failure is `None`, which is the honest answer and the same direction the two
    /// reads above take: a worker that could not be built names no directory.
    fn working_copy(&self) -> Option<PathBuf> {
        crate::worker::select_worker(self.cwd.clone())
            .ok()
            .and_then(|w| w.working_copy())
    }

    /// **The FOURTH instance of the class nxf 6j6v.10yb named at this very wrapper** (nxf
    /// 6j6v.npy3) — and the one that made the feature it belongs to inert on the shipped binary
    /// before a review caught it.
    ///
    /// Left on the trait default (`false`), every command-line call told the engine that this
    /// host's runtime cannot continue a conversation it began. `orchestration::resume_interrupted`
    /// reads exactly this before it starts anything, so `nxc resume --thread <id>` — and the
    /// background service's own scheduled resume, which runs the same verb — answered
    /// `ProviderCannotResume` for every session that had ever been bound to a runtime session,
    /// which is every session the feature exists for. The engine SEAM was fine (`Engine` holds the
    /// real worker, and `SidecarWorker::resumes_sessions` answers `true`), which is precisely why
    /// every test of that feature stayed green — the same sentence the three doc comments above
    /// this one already had to write.
    ///
    /// **What made it a fourth rather than a first**: the structural gate built after the third
    /// (`tests/every_worker_answers_for_itself.rs`) enumerates the DEFAULTED methods by hand, and a
    /// fifth defaulted method does not arrive in that list on its own. It does now, which is what
    /// turns this comment into a gate rather than a warning.
    ///
    /// A selection failure is `false`, the honest answer and the same direction the three reads
    /// above take: a worker that could not be built continues nothing. It is also the safe
    /// direction here — a wrong `true` starts a fresh conversation under a name that has a
    /// transcript, which is exactly the state the transcript exists to prevent.
    fn resumes_sessions(&self) -> bool {
        crate::worker::select_worker(self.cwd.clone()).is_ok_and(|w| w.resumes_sessions())
    }

    /// **Forwarded in the same change that added it to the trait** (nxf 6j6v.b9nf) — which is the
    /// only order in which this wrapper has ever NOT shipped the class the four comments above
    /// describe. Left on the default (`false`), every `nxc withdraw` of a running round would
    /// refuse by name for a sidecar that stops its sessions perfectly well, and the engine seam
    /// would stay green throughout, as it did each time before.
    ///
    /// A selection failure is `false`, the honest answer and the safe direction: a worker that
    /// could not be built stops nothing, and saying so costs a named refusal rather than a
    /// withdrawal that believes a stop was delivered.
    fn stops_sessions(&self) -> bool {
        crate::worker::select_worker(self.cwd.clone()).is_ok_and(|w| w.stops_sessions())
    }

    /// The act behind the question one method up, forwarded with it (nxf 6j6v.b9nf). A selection
    /// failure is reported AS the refusal it is, in `run_precondition`'s shape below: there is no
    /// safe `Ok` to fall back to — an `Ok` here means "delivered", and a caller would wait on a
    /// process nothing was sent to.
    fn stop_session(
        &self,
        internal_session: &str,
    ) -> std::result::Result<crate::worker::SessionStop, String> {
        match crate::worker::select_worker(self.cwd.clone()) {
            Ok(w) => w.stop_session(internal_session),
            Err(e) => Err(format!(
                "no worker could be selected to stop session {internal_session}: {}",
                e.msg
            )),
        }
    }

    /// The same delegation for the same reason, one method later (nxf 6j6v.n92p): every command-line
    /// call passes through this wrapper, so a `run_precondition` left on the trait default would
    /// make every declared hurdle on the shipped path report "could not be checked" — fail-closed,
    /// so nothing would break silently, but no channel with `preconditions:` would ever run a step.
    ///
    /// A selection failure is reported AS the unavailability it is, rather than swallowed: unlike
    /// the liveness read above there is no safe boolean to fall back to, and the honest answer is
    /// the one the fail-closed rule already wants.
    fn run_precondition(&self, command: &str) -> crate::precondition::PreconditionOutcome {
        match crate::worker::select_worker(self.cwd.clone()) {
            Ok(w) => w.run_precondition(command),
            Err(e) => crate::precondition::PreconditionOutcome::Unavailable(format!(
                "no worker could be selected to run this hurdle: {}",
                e.msg
            )),
        }
    }
}

// `parse_enum`, `parse_kind`, `parse_priority`, `parse_disposition` and `escalate_or_kind` were
// here. REMOVED
// with the three flags they parsed (nxf 6j6v.ckeq, decision 3) and with `--kind` itself.
//
// `escalate_or_kind` is the one worth a sentence, because what it did SURVIVES and only its shape
// went. It resolved `--escalate` to `MessageKind::Escalation` and otherwise parsed `--kind`; with
// `--kind` gone there is nothing left to resolve AGAINST, so the mapping is one function of one
// boolean and it lives where both surfaces reach it: `surface::reply_kind`, called from
// `reply_in_thread`. The carrier is unchanged — `messages.kind`, compared against
// `model::KIND_ESCALATION` by `ChatStore::last_reply_escalated` — and `clap`'s
// `conflicts_with = "kind"` went with the flag it conflicted with rather than with the guarantee it
// bought: `--escalate` is still the ONE door to the marker, now because it is the only one there is.
//
// `parse_enum` — the shared "clap already constrained this, so re-read it through serde" helper —
// went with them: `--kind`/`--priority`/`--disposition` were its only three callers, and a helper
// kept for a caller that no longer exists is a shape the next reader has to work out is dead.

/// Build [`Refs`] from repeated `--ref k=v` (spec §2.1: pointers, not content). Known keys:
/// `session_id`, `branch`, `pr` (scalar), `nxf_ids`/`nxf_id` (repeatable). An unknown key is a loud
/// `validation` error (no silent drop).
fn parse_refs(pairs: &[String]) -> Result<Refs> {
    let mut refs = Refs::default();
    for p in pairs {
        let (k, v) = p
            .split_once('=')
            .ok_or_else(|| NxfError::validation(format!("--ref expects k=v, got: {p}")))?;
        match k {
            "session_id" => refs.session_id = Some(v.to_string()),
            "branch" => refs.branch = Some(v.to_string()),
            "pr" => refs.pr = Some(v.to_string()),
            "nxf_ids" | "nxf_id" => refs.nxf_ids.push(v.to_string()),
            other => return Err(NxfError::validation(format!("unknown --ref key: {other}"))),
        }
    }
    Ok(refs)
}

/// `send --to`'s THREE-state answer about what the conversation is for (nxf 6j6v.ckeq §5), out of
/// the two flags that express it: repeated `--ref k=v`, and `--no-ref`.
///
/// The mapping is the whole of the CLI's part in this — the obligation itself, the warning text and
/// the decision that an EMPTY `--ref` set is silence rather than a declared nothing all live on
/// [`crate::surface::SendToRefs`], so the command line and an embedding app cannot answer the same
/// question differently. `clap`'s `conflicts_with` already refuses `--ref` and `--no-ref` together,
/// which is why there is no third arm here: a caller cannot say both.
fn resolve_refs(pairs: &[String], no_ref: bool) -> Result<crate::surface::SendToRefs> {
    if no_ref {
        return Ok(crate::surface::SendToRefs::ExplicitlyNone);
    }
    Ok(crate::surface::SendToRefs::declared(parse_refs(pairs)?))
}

// `checked_caller_handle` was here — the caller's qualified handle with a U+001F guard, needed
// because `channels create`/`join` turned that handle straight into a membership id. Both verbs
// went with the raw channels (6j6v.dvyq §3), and every surviving write reaches membership through
// a DECLARATION rather than through the caller's own environment. The guard itself lives on where
// it is still reachable: `check_no_sep` at the two composite-id write helpers in `store.rs`.

// `print_ask_receipt` and the declared-channel fan-out banner were here. The banner described a
// bridge that no longer forks: `send`'s positional channel branch and `ask` both used to try the
// given string as a declared name first, and both are gone (6j6v.dvyq §3). ONE surface resolves a
// target now — `surface::send_to` — and it renders its own `SendToReceipt`, so there is no second
// receipt shape for this file to print. `open_declared_channel_and_fan_out` itself is untouched in
// `orchestration`; only the CLI's two doors onto it collapsed into one.

// ---- init ------------------------------------------------------------------

/// `nxc init`: ensure a `.nxs/` workspace (join an existing one, else create a fresh
/// chat workspace) via `foundation::setup`, register `chat` as an active module, materialize chat's
/// views, then **delegate** the shared agent file + one SessionStart hook per active module to the
/// ONE assembler in `nxs` (P3-S5) — re-assembling every active module. Joining a flow/memory
/// workspace re-assembles all modules under that one hook (no second hook, no self-assembly). Parity
/// with `nxm init`.
fn init(json: bool, quiet: bool) -> Result<()> {
    // Single front door (5jz.2): an interactive `nxc init` re-execs the shared `nxs init` frame with
    // chat PRE-SELECTED, so it lands on the same suite chooser. The driven `--quiet` seam the umbrella
    // calls, and `--json`/piped, stay chat-native and byte-stable. chat takes no `--plugin` and has
    // no beads migration. Non-recursive: `nxc init` (TTY) → `nxs init --preselect chat` →
    // `nxc init --quiet`.
    if !quiet && !json && nxs_init::frontdoor::is_interactive() {
        return nxs_init::frontdoor::reexec_umbrella_init("chat", json, None, false);
    }
    let root = cwd()?;
    // Delegate the foundation to setup() — idempotent: join an existing workspace, else create one.
    let ws = match workspace::discover(&root) {
        Ok(existing) => existing,
        Err(_) => workspace::setup(&root, &workspace::chat_config())?,
    };
    // Register chat as an active module. setup loads an existing config UNCHANGED, so when chat joins
    // a flow/memory workspace this is what records it in `active_modules` (idempotent).
    workspace::activate_module(&ws.dir, workspace::CHAT_MODULE)?;
    // Open chat's store to materialize its views (setup created only the substrate db).
    ws.open_chat_store()?;
    // The declaration folder, with its explainer (nxf 6j6v.dvyq step 4). Without `Supplied` and
    // without raw channels, a workspace with no `.nxs-personas/` has not one `send --to` target —
    // so `init` has to leave behind the place a first declaration goes, not just a database. EMPTY,
    // with no shipped personas (owner note on 6j6v.m4xe puts those outside this epic).
    let personas = ensure_personas_dir(&ws)?;
    // Re-resolve so the config reflects the just-activated chat module (and any already-active
    // sibling), then delegate to the assembler — it re-assembles every active module's block under
    // one SessionStart hook per active module.
    let ws = workspace::discover(&root)?;
    let report = assemble_agent_files(&root, &ws)?;
    let modules = ws.config.active_modules.clone();
    // Cross-sell the not-yet-active suite modules (aye.31, parity with `nxm init`): `Some` when an
    // addable module (flow/memory) is still inactive; `None` once every addable module is active.
    // chat is now in the `nxs-init` catalog, so a chat-only workspace surfaces flow + memory here.
    let ad = nxs_init::advertise::advertisement(&modules);

    if json {
        let mut out = serde_json::json!({
            "ok": true,
            "workspace": ws.dir.display().to_string(),
            "site_id": ws.replica.site_id,
            "prefix": ws.replica.prefix,
            "modules": modules,
            "personas": personas.display().to_string(),
            "onboarding": {
                "agents": report.agents.as_str(),
                "claude": report.claude.as_str(),
            },
            // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** `command` was ONE string while the
            // assembler wired one hook; it is now one entry per active module, so the field is a
            // LIST under the name that says so. A consumer reading the old key gets nothing rather
            // than the first of three silently mistaken for the whole wiring.
            "hook": {
                "hook_added": report.hook_added,
                "commands": report.hook_commands,
            },
        });
        // Additive `advertisement` field (aye.31): present only when an addable module is inactive.
        if let Some(ad) = &ad {
            out["advertisement"] = serde_json::Value::String(ad.agent_copy.clone());
        }
        println!("{out}");
    } else if !quiet {
        println!("initialized chat at {}", ws.dir.display());
        println!("  prefix:  {}", ws.replica.prefix);
        println!("  modules: {}", modules.join(", "));
        println!("  personas: {}", personas.display());
        println!("  agent files: {}", describe_onboarding(&report));
        println!(
            "  hook:    {}",
            // Rendered by the assembler, not spelled out here: the hook command grew a
            // `|| cat NEXUS_MEMORY.md` fallback in 6j6v.8q88, and became a LIST — one entry per
            // active module — in nxf n2m6 + a2a1. Hand-written copies across the module CLIs would
            // be that many places to go stale the next time it moves.
            nxs_init::assembler::describe_hooks(report.hook_added, &report.hook_commands)
        );
        // The suite cross-sell CTA (TTY-gated → ember on a terminal, plain otherwise).
        if let Some(ad) = &ad {
            let theme = nxs_ui::Theme::detect(false);
            println!("\n{}", theme.accent(&ad.human_cta));
        }
    }
    // `--quiet` without `--json`: silent success — the umbrella renders.
    Ok(())
}

/// Create `<workspace-root>/.nxs-personas/` and its explainer if they are not already there, and
/// return the path either way (nxf 6j6v.dvyq step 4).
///
/// IDEMPOTENT and NON-DESTRUCTIVE in both halves: an existing folder is left exactly as it is, and
/// an existing `README.md` is never overwritten — a user who edited the explainer, or who keeps a
/// README of their own in there, does not lose it to a second `nxc init`.
///
/// It does NOT touch a legacy `roles/` folder. The engine reads one when it finds one and reports
/// that it did; moving it is a tool a human or an app invokes deliberately, never a side effect of
/// `init` (see [`crate::definitions::DeclarationSource::locate`] for the reasoning).
fn ensure_personas_dir(ws: &Workspace) -> Result<PathBuf> {
    use crate::definitions::{PERSONAS_README, PERSONAS_README_BODY};
    let dir = ws.personas_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| NxfError::io(format!("creating {}: {e}", dir.display())))?;
    let readme = dir.join(PERSONAS_README);
    if !readme.exists() {
        std::fs::write(&readme, PERSONAS_README_BODY)
            .map_err(|e| NxfError::io(format!("writing {}: {e}", readme.display())))?;
    }
    Ok(dir)
}

/// chat's self-registered in-process init entry (5jz.3). The umbrella calls this DIRECTLY (no
/// `<bin> init --quiet` subprocess), so chat sets itself up inside the one umbrella process. chat has
/// no interactive sub-config of its own, so it always runs silently here: `json = false,
/// quiet = true` keeps it from ever printing its own record — the umbrella owns all visible output.
/// chat takes no `--plugin` (accepts_plugin: false).
pub fn chat_init_entry(_req: &nxs_init::InitRequest) -> Result<()> {
    init(false, true)
}

// Self-registration (5jz.1): chat declares its roster identity + chooser copy + in-process init
// entry; the umbrella collects this via `inventory`, so chat appears in the `nxs init` chooser and
// the `nxs prime` fan-out with only the load-bearing `use nexus_chat as _;` link line added to
// `nxs`'s composition root — no other `nxs` source change (spec §5.2).
inventory::submit! {
    nxs_init::ModuleInit {
        key: "chat",
        binary: "nxc",
        now_env: "NXC_NOW",
        blurb: "the channel — messages between you and your agents, on the record",
        recommended: false,
        // After flow (10) and memory (20): chat is the third product, so it sorts last in the roster.
        // Ahead of memory since 6j6v.xbnh: what a truncated session start must keep is what
        // nothing else can supply, and chat's block (who you are, who is waiting on you) is not
        // fetchable after the fact the way `nxm recall` is. See memory's own ordinal for the
        // measurement.
        order: 20,
        accepts_plugin: false,
        init_fn: chat_init_entry,
        // 5jz.6: chat's own welcome pitch, shown in the nxs frame when chat is preselected.
        welcome: Some(
            "chat is the channel between you and your agents — send, reply, and get the answer \
             back in the session that asked, so hand-offs stop getting lost in ad-hoc notes.",
        ),
        // huy: chat's "learn more" decision-help, shown in the picker's details panel.
        details: Some(
            "Messages between you and your agents, on the record: durable channels, DMs and \
             threads, with every message PUSHED to the session it is for. Hand-offs go \
             through `nxc` instead of ad-hoc files; a session start names the commissions that \
             finished while it was away. Offline-first — delivery rides the shared sync, with no \
             standing background service.",
        ),
        // 5jz.6: chat's post-init success moment — a concrete first channel to talk on.
        // The post-init success moment has to point the SAME way `init`'s own README does — declare
        // somebody (nxf 6j6v.dvyq §3/§4). It used to say `nxc channels create general`, a RAW
        // channel: "something addressable without policy — exactly the state this rebuild
        // abolishes", so the two halves of one command's onboarding pointed in opposite directions.
        first_command: Some(nxs_init::FirstCommand {
            label: "meet the team you have declared (start by adding one to .nxs-personas/)",
            command: "nxc list",
        }),
    }
}

/// Assemble the shared agent file + one SessionStart hook per active module from the workspace's
/// active modules' manifests (P3-S5; a single `nxs prime` hook until nxf n2m6 + a2a1). chat delegates to the ONE assembler in `nxs`, which shells out to each active
/// module's `agent-manifest` (TB-5) — so a flow+memory+chat workspace gets every block. A lone chat
/// binary assembles only the blocks it OWNS; an active sibling it does not link (flow/memory, when
/// `nxc init` joins their workspace) has already assembled its own block via its own init — so
/// resolve the modules this binary knows and skip the rest, rather than erroring.
fn assemble_agent_files(
    root: &std::path::Path,
    ws: &Workspace,
) -> Result<nxs_init::assembler::AssembleReport> {
    let roster = nxs_init::roster();
    let modules = nxs_init::resolve_known(&roster, &ws.config);
    let manifests = nxs_init::assembler::collect_manifests(&modules)?;
    nxs_init::assembler::assemble(root, &manifests)
}

/// One-line human summary of what the assembler wrote to AGENTS.md and, since nxf 6j6v.q6e3,
/// what it cleaned back out of CLAUDE.md (mirrors `nxm init`).
fn describe_onboarding(report: &nxs_init::assembler::AssembleReport) -> String {
    use nxs_init::assembler::{AgentsAction, ClaudeAction};
    let agents = match report.agents {
        AgentsAction::Created => "created AGENTS.md",
        AgentsAction::Augmented => "updated AGENTS.md",
    };
    let claude = match report.claude {
        ClaudeAction::Absent | ClaudeAction::Untouched => "",
        ClaudeAction::BlockRemoved => "; removed the retired block from CLAUDE.md",
        ClaudeAction::FileRemoved => "; removed CLAUDE.md, which held nothing else",
    };
    format!("{agents}{claude}")
}

// ---- contract verbs: agent-manifest + prime --------------------------------

/// `nxc agent-manifest`: emit nxc's declared contribution to the shared agent file as
/// data. Static (no workspace, no store), so it runs anywhere. `--json` is the contract the `nxs`
/// umbrella consumes; the typed struct's declaration order IS the field-order contract.
fn agent_manifest(json: bool) -> Result<()> {
    let manifest = onboarding::manifest();
    if json {
        let value = serde_json::to_string(&manifest)
            .map_err(|e| NxfError::io(format!("serializing manifest: {e}")))?;
        println!("{value}");
    } else {
        println!("nexus-chat agent manifest");
        println!("  prime command:  {}", manifest.prime_command);
        println!(
            "  hook:           {} → {}",
            manifest.hook.event, manifest.hook.command
        );
        println!("\nRun with --json for the machine contract the `nxs` umbrella assembles from.");
    }
    Ok(())
}

/// `nxc prime`: the SessionStart context block. It (1) pins the chat rule (agent-to-agent
/// coordination goes through `nxc`, not ad-hoc notes), (2) names the core verbs, and (3) names the
/// commissions of this caller's own that finished while it was away. Deterministic, so the
/// SessionStart hook output is byte-stable. Wired via `nxs prime`'s fan-out; hosts render the human
/// (Markdown) form as context.
///
/// It replayed the session agent's entire unread set as its third part — both dispositions, because
/// a new session is the catch-up moment — until nxf 6j6v.4mmk stopped rendering it and nxf
/// 6j6v.4d2z removed the set.
///
/// The record is computed by [`facade::prime`] and renders ITSELF — this command
/// resolves the ambient inputs (the caller handle, the clock, the workspace's declaration folder)
/// and picks
/// the view. It used to assemble the block here, reaching PAST the facade straight into the store,
/// so none of it was reachable from the library.
///
/// The one decision that stays here is [`is_spawned_context`]: declaration errors, the quality
/// WARNINGS beside them (nxf 6j6v.9w08) and the one line pointing at `nxc guide
/// writing-declarations` are all for a human at the keyboard, and "am I interactive?" is a question
/// only the terminal-facing seam can answer. The report carries the findings either way and takes
/// the answer as a parameter — a facade that decided this itself would serve an embedding app less
/// than it serves the CLI.
fn prime(
    json: bool,
    db: Option<&str>,
    consumer: Option<&str>,
    persona: Option<&str>,
) -> Result<()> {
    let store = open(db)?;
    // The record is for the session agent — the same `NXC_ACTOR`→`USER` qualified handle every
    // messaging verb resolves (spec §5.1), overridable with `--consumer`.
    let consumer = resolve_consumer(consumer, db)?;
    // WHO this is comes from the origin of the call (nxf 6j6v.p6m1): an explicit `--persona` for a
    // host that started the session itself, else the session this invocation runs under, else a
    // human. See `crate::persona` for why it is deliberately not the process id, and for what the
    // answer may and may not be used for.
    let declarations = crate::definitions::DeclarationSource::resolve(&workspace_root(db)?)?;
    let identity = crate::persona::resolve_identity(
        &store,
        &crate::role::load_all_roles(&declarations.read_dir)?,
        persona,
        session().as_deref(),
    )?;
    let report = facade::prime_for(
        &store,
        &consumer,
        identity.persona(),
        &resolve_now()?,
        &declarations,
    )?;
    let interactive = !is_spawned_context();
    if json {
        println!("{}", report.to_value(interactive));
    } else {
        // Human form: a structured Markdown context block hosts inject at session start.
        println!("{}", report.render_markdown(interactive));
    }
    Ok(())
}

// ---- list ------------------------------------------------------------------------------------

/// `nxc list [--persona <name>]` (nxf 6j6v.p6m1, surface draft §4.4): who can be addressed here.
///
/// Two audiences, one record. An explicit `--persona` projects that persona's own address book —
/// what an app asks for when it renders one agent's options. Without it the answer follows the same
/// identity rule `prime` does: a running persona (recognised from its session) sees ITS view, and a
/// human sees the complete declared team, which is the directory an app renders whole.
fn list(json: bool, db: Option<&str>, persona: Option<&str>) -> Result<()> {
    let store = open(db)?;
    let declarations = crate::definitions::DeclarationSource::resolve(&workspace_root(db)?)?;
    let roles = crate::role::load_all_roles(&declarations.read_dir)?;
    let channels = crate::channel::load_all_channels(&declarations.read_dir)?;
    let identity = crate::persona::resolve_identity(&store, &roles, persona, session().as_deref())?;
    let directory = match identity.persona() {
        Some(handle) => crate::persona::Directory::for_persona(&roles, &channels, handle),
        None => crate::persona::Directory::full(&roles, &channels),
    };
    // The directory AND where it came from, in one value: an app renders its empty list with the
    // explanation and the path beside it rather than re-deriving either (nxf 6j6v.dvyq). Attached
    // to the RECORD, not printed beside it, so `Engine::directory` serves the identical bytes —
    // which is what the parity differential next door compares.
    //
    // The FRONT DOORS are attached the same way and for the same reason (nxf 6j6v.yr59, which
    // folded `Engine::public_channels` into the directory): both surfaces read them through the one
    // `facade::front_doors`, so neither can drift. They ride on the `--json` RECORD and not on the
    // human rendering — `Directory::render_markdown` says why: a door no declaration here names is
    // discoverable but not addressable with `send --to`, and an address book that lists one is an
    // invitation to be refused.
    let directory = directory
        .with_declarations(Some(&declarations))
        .with_front_doors(facade::front_doors(&store)?);
    if json {
        println!(
            "{}",
            serde_json::to_string(&directory).expect("directory serializes")
        );
    } else if directory.is_empty() {
        // ORIENTATION, not failure: exit 0, and say what is missing and where it belongs. An empty
        // list on its own is a lie by omission; an error would make a legitimate state exceptional.
        println!("{}", declarations.explain());
    } else {
        println!("{}", directory.render_markdown());
        // A LEGACY folder is worth saying out loud every time it is the one being read — otherwise
        // the move to `.nxs-personas/` is invisible to the person who has to make it.
        if declarations.kind == crate::definitions::DeclarationSourceKind::LegacyRoles {
            println!("\n{}", declarations.explain());
        }
    }
    Ok(())
}

/// Whether a receipt's `warnings` should make this verb exit non-zero.
///
/// **Every class does, except two** (nxf 6j6v.p3sm, 6j6v.0j12). The rule the exit code encodes is
/// "something that was supposed to happen as part of THIS call did not" — a step nobody was put to
/// work on, a requester nobody woke — and a script that never parses JSON learns it from the status
/// alone.
///
/// [`ConsequenceClass::TickUnscheduled`] is a different statement: the call did everything it was
/// asked, the board is open and its members are running; what is missing is the future SAFETY NET
/// that would have noticed the window running out. Making it non-zero would mean that on macOS —
/// where `atrun` ships disabled, so no `at` job ever fires — every single `send --to <channel>` on a
/// channel with a declared `timeout:` exits 1, permanently, for a condition the caller cannot fix
/// from the command line. A status that is always red is a status nobody reads, and it would drown
/// the classes this rule exists for.
///
/// It is reported no less loudly for that: the finding rides the `--json` receipt like every other,
/// and the human line goes to stderr beside it. What it does not do is claim the call failed.
///
/// **This is a judgement, and it is the owner's to overturn**: the alternative reading — a channel
/// whose `timeout:` cannot fire is a broken channel and should be loud on every call until somebody
/// fixes the machine or removes the declaration — is coherent, and one line here changes it.
///
/// [`ConsequenceClass::ServiceNotRunning`] joins it for the same reason and more strongly still: it
/// is not about this call at all, it is about the machine, and it rides EVERY `send`/`reply`/`tick`
/// while the background service is down. Making it red would turn every verb in a workspace with a
/// stopped service permanently non-zero — the "status nobody reads" this doc argues against, in its
/// purest form.
///
/// [`ConsequenceClass::DeclarationChanged`] is the third, and it is the plainest of the three (nxf
/// 6j6v.pkw9): the call did everything it was asked, the session IS running, and what is reported is
/// that it runs under a different declaration than the last one did. The item is explicit that this
/// must be a report and **not a refusal** — a change is usually intended, and editing a persona and
/// then addressing it is the ordinary way to work. A non-zero exit is how a script learns a call
/// failed, and this one did not.
///
/// **The two PARK classes are the fourth and fifth** (fix round 1 of nxf 6j6v.8bv9), and both of
/// their own docs already said so in as many words — "It does NOT make the verb exit non-zero" —
/// while this function still turned them red. The doc was the rule; the code was the drift.
///
/// [`ConsequenceClass::WorkNotParked`] is the [`TickUnscheduled`](ConsequenceClass::TickUnscheduled)
/// argument exactly: the tick did everything it was asked, the queue is in the state it was already
/// in, the refusal is on `nxc status` and the retry is armed. What it reports is a TREE for a person
/// to go and fix, which is not the same statement as "this call failed".
///
/// [`ConsequenceClass::WorkHandedOnUnparked`] is the one that forced the question, because nxf
/// 6j6v.8bv9 made it reachable from the most ORDINARY drain there is. A workspace whose runtime
/// names no working copy — or a directory that is not a git repository, which is every workspace in
/// `working_tree_two_process_e2e.rs` — refuses every park permanently, so a routine `nxc tick` that
/// hands a queued commission its turn now carries this finding EVERY time. Leaving it red would mean
/// the background service's own drain exits 1 as a matter of course, on a condition no caller can
/// fix from the command line and about a tree that in that workspace does not exist: the
/// "status that is always red is a status nobody reads" case in its purest form, and it would drown
/// the classes this rule exists for.
///
/// Both stay on the receipt and in `--json`, and both keep their `warning:` line on stderr. What
/// changes is only the claim the exit code makes about the call.
fn changes_the_exit_code(warnings: &[crate::orchestration::FailedConsequence]) -> bool {
    use crate::orchestration::ConsequenceClass;
    warnings.iter().any(|w| {
        !matches!(
            w.class,
            ConsequenceClass::TickUnscheduled
                | ConsequenceClass::ServiceNotRunning
                | ConsequenceClass::DeclarationChanged
                | ConsequenceClass::WorkNotParked
                | ConsequenceClass::WorkHandedOnUnparked
        )
    })
}

// ---- send ------------------------------------------------------------------

/// `nxc send --to <persona|channel> <BODY>|- [--body-file <path>] [--stream]` (nxf 6j6v.p6m1,
/// surface draft §4.2; the body's three sources are nxf 6j6v.s46h — see [`resolve_body`]).
///
/// An ADAPTER, like every verb in this file since 6j6v.rws4: it parses the stringly-typed
/// arguments, resolves the ambient context, and hands the whole decision to
/// [`crate::surface::send_to`] — which resolves the target's declaration and then runs the existing
/// orchestration verb for it. Nothing about the target's meaning is decided here.
///
/// **This is the WHOLE verb since 6j6v.dvyq §3.** A second function stood in front of it, dispatched
/// to it for `--to` and otherwise parsed the stringly-typed arguments a second time to reach
/// `orchestration::role_trigger`/`role_resume` for `--role`/`--session` (the first is
/// `coordinator_commission` since nxf 6j6v.ntp9). With both flags gone there
/// is one target, one branch and one body: the fork is not simplified, it is absent. What that
/// front half additionally carried — a `--stream` refusal for the flags that could not follow a
/// conversation, and a `not_found` for a channel the positional named — went with the inputs that
/// could reach them.
#[allow(clippy::too_many_arguments)]
fn send(
    json: bool,
    db: Option<&str>,
    body: &str,
    refs: &[String],
    no_ref: bool,
    to: &str,
    stream: bool,
    machine: Option<&str>,
) -> Result<()> {
    // The three-arm resolution that stood here — "exactly one positional, and say so when a caller
    // passes two" — is clap's job now that `send` has a single required positional (6j6v.dvyq §3).
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let ctx = cli_ctx.ctx();
    let mut store = open(db)?;
    // The `--stream` gate is checked BEFORE the write: refusing it afterwards would leave the
    // caller with a sent message and an error, which is the one outcome worse than either. The
    // follow's TIMINGS are resolved here for the same reason — a rejected `NXC_STREAM_IDLE` after
    // the post is that same bad outcome, arrived at by a different route (PR #295 review).
    let follow_opts = if stream {
        refuse_stream_for_a_persona(&crate::persona::resolve_identity(
            &store,
            ctx.defs.roles(),
            None,
            ctx.session,
        )?)?;
        Some(stream_options()?)
    } else {
        None
    };

    let receipt = crate::surface::send_to(
        &ctx,
        &mut store,
        crate::surface::SendToRequest {
            machine,
            to,
            body,
            refs: resolve_refs(refs, no_ref)?,
        },
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&receipt).expect("send --to receipt serializes")
        );
    } else {
        // **The requester's line, and it answers the requester's question** (nxf 6j6v.7qfm). It
        // used to read `opened thread <id> in dm:<24 hex> — reply with `nxc reply --thread <id>`,
        // which told the reader how to keep TALKING at the moment nobody had answered yet, and
        // named a hash derived from the two handles that the reader — who knows perfectly well who
        // they wrote to — could do nothing with. The hash is in the receipt's `channel` field,
        // which is where a record belongs; what stands here is who was asked, on which thread, and
        // how the answer comes back.
        println!("-> {} · thread {}", receipt.to, receipt.thread_id);
        if let Some(to) = &receipt.handed_to {
            println!("{}", handed_to_line(to));
        }
        println!("{}", receipt.await_.render_human(&receipt.thread_id));
        // A second line rather than a replacement: the thread WAS opened and the message WAS
        // posted, so the line above is true — what is not true, and would be assumed, is that
        // anybody is working on it yet (nxf 6j6v.303b, epic §6).
        if let Some(position) = receipt.queue_position {
            println!(
                "QUEUED at position {position} behind {}: another chain holds the working copy, so \
                 {} has not started yet",
                receipt.queued_behind.as_deref().unwrap_or("the current holder"),
                receipt.to
            );
        }
        // stderr, not stdout (PR review, Minor #6) — see `send`'s identical choice for the reason.
        for w in &receipt.warnings {
            eprintln!("warning: {w}");
        }
    }
    // BOTH renderings, and the `--json` one is NOT an else-branch (nxf 6j6v.ckeq §5). The field is
    // already in the receipt printed above, which is what an app reads; this line is for the human,
    // and a human who piped `--json` into a file is still watching a terminal. Same text either way
    // (`surface::UNREFERENCED_WARNING`), so the two cannot say different things.
    if let Some(warning) = &receipt.refs_warning {
        eprintln!("warning: {warning}");
    }
    if changes_the_exit_code(&receipt.warnings) {
        // Same choice as `send`'s `--role`/`--session` path, and the same reason (nxf 6j6v.hpv8):
        // the thread and message above ARE persisted, so this is not `error::emit`'s error
        // envelope — `--json` already carries `spawned`/`warnings` as part of the ONE receipt
        // printed above. The exit code is the other half of the contract, for a script that never
        // parses JSON. `--stream` is skipped rather than followed: nothing is running for this
        // send, so following it would just wait out the idle timeout for no reason.
        //
        // `store` dropped explicitly first (PR review, Important #2) — see `send`'s identical
        // comment: `std::process::exit` skips `Store`'s `Drop`, which touches the sync daemon's
        // local-write marker on exactly this path, where a write WAS emitted.
        drop(store);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(1);
    }
    if let Some(opts) = follow_opts {
        follow_conversation(
            &store,
            &receipt.thread_id,
            &ctx.caller_handle(),
            receipt.session.as_deref(),
            json,
            opts,
        )?;
    }
    Ok(())
}

/// Refuse `--stream` to a registered persona (surface draft §4.5/§10.4).
///
/// **Hygiene, not a barrier**, and the wording says so rather than implying a guarantee: a process
/// that simply never registers is "a human" to this check, exactly as it is to every other
/// self-reported identity in the system. What it does buy is real — an agent cannot ACCIDENTALLY
/// block for half an hour waiting on a conversation it is itself part of.
fn refuse_stream_for_a_persona(identity: &crate::persona::Identity) -> Result<()> {
    match identity.persona() {
        None => Ok(()),
        // The one runtime message in this binary that ONLY an agent can ever see (this arm fires
        // on a REGISTERED persona), so it is agent-surface text and must name only what the agent
        // surface carries: 6j6v.dvyq §3 took `inbox` off that surface, and a persona is handed what
        // it needs at session start and on resume rather than asking for it (6j6v.4d2z then removed
        // the read behind the verb as well).
        Some(handle) => Err(NxfError::validation(format!(
            "--stream is for a human at the keyboard; you are running as {handle}. Send without \
             it — the answer arrives in the thread you opened, and `nxc status --thread <id>` \
             shows where it stands."
        ))),
    }
}

/// Follow a conversation until it settles, printing the thread and the transcripts as they happen.
///
/// The loop is the only part of `--stream` that sleeps; every decision it makes belongs to
/// [`crate::stream`] — what is new ([`Follow::poll`](crate::stream::Follow::poll)), whether the
/// conversation is finished ([`crate::stream::settled`]), and whether silence has gone on too long
/// ([`IdleClock`](crate::stream::IdleClock), which is reset by any activity and knocks before it
/// gives up). Ends cleanly on both outcomes: a `--stream` that times out is not an error, because
/// the message it followed was sent and is still sitting there being worked on.
///
/// `opts` arrives already resolved rather than being read here: [`stream_options`] can REJECT its
/// environment, and that rejection belongs before the caller's message is written — see the call
/// sites, which resolve it beside the persona gate for exactly that reason.
fn follow_conversation(
    store: &ChatStore,
    thread: &str,
    me: &str,
    seed: Option<&str>,
    json: bool,
    opts: crate::stream::StreamOptions,
) -> Result<()> {
    let mut follow = crate::stream::Follow::start(store, thread, me, seed)?;
    let mut clock = crate::stream::IdleClock::new(&opts);
    let mut last_activity = std::time::Instant::now();
    let mut last_seen: Option<(String, String)> = None;
    loop {
        if crate::stream::settled(store, thread, &follow, &resolve_now()?)? {
            return Ok(());
        }
        std::thread::sleep(opts.poll);
        let events = follow.poll(store)?;
        if !events.is_empty() {
            last_activity = std::time::Instant::now();
            clock.saw_activity();
        }
        for event in &events {
            if let crate::stream::Event::Did { kind, detail, .. } = event {
                last_seen = Some((kind.clone(), detail.clone()));
            }
            print_stream_event(json, event);
        }
        match clock.decide(last_activity.elapsed()) {
            crate::stream::Decision::Wait => {}
            crate::stream::Decision::Knock => {
                let knock = crate::stream::Knock {
                    sessions: follow.sessions(),
                    last: last_seen.clone(),
                    quiet_for: last_activity.elapsed(),
                };
                if json {
                    println!(
                        "{}",
                        serde_json::json!({ "stream": "knock", "detail": knock.render() })
                    );
                } else {
                    println!("… {}", knock.render());
                }
            }
            crate::stream::Decision::GiveUp => {
                let quiet = last_activity.elapsed().as_secs() / 60;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "stream": "gave_up", "thread_id": thread, "quiet_minutes": quiet
                        })
                    );
                } else {
                    println!(
                        "… nothing for {quiet}m — no longer watching. The message is still \
                         there; `nxc threads show {thread}` shows the answer when it lands."
                    );
                }
                return Ok(());
            }
        }
    }
}

/// The shortest poll interval the follow will honour, whatever the environment asks for.
///
/// A floor rather than a rejection (PR #295 review, Integrity #2): `NXC_STREAM_POLL=0` is a
/// perfectly clear request — "look as often as you can" — and reading it literally turns the loop's
/// only sleep into a no-op, re-reading SQLite at full CPU for the whole idle window. Rejecting `0`
/// while accepting `0.0001`, which busy-loops just as hard, would draw the line in a place nobody
/// could guess. 50ms is below anything a human perceives and above anything that spins.
const MIN_STREAM_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// The follow's timings, overridable through `NXC_STREAM_IDLE`/`NXC_STREAM_POLL` (seconds).
///
/// Environment rather than flags, deliberately: they exist so a TEST can drive the loop in
/// milliseconds and so an operator can shorten a wait on a machine where thirty minutes is absurd —
/// neither is a decision a caller should be making per invocation. An unparseable value is a
/// `validation` error rather than a silent fallback: a typo'd timeout that silently means "thirty
/// minutes" is exactly the kind of thing nobody notices until it has cost an hour.
fn stream_options() -> Result<crate::stream::StreamOptions> {
    stream_options_from(|key| std::env::var(key).ok())
}

/// [`stream_options`] over an injected lookup — the same discipline
/// [`WorkerConfig::from_ambient`](crate::worker::WorkerConfig::from_ambient) uses, so the unit tests
/// below can drive every rejection without touching process-global environment that parallel tests
/// in this binary would race on.
///
/// **"Seconds" is `f64` and therefore has values that are not durations**, which is where this
/// function had a genuine bug (PR #295 review, found by two reviews independently): `"-1"` and
/// `"nan"` both parse as `f64` and then panic inside `Duration::from_secs_f64`, so a mistyped
/// environment variable aborted the process with exit 101 instead of the `validation` error the doc
/// above promises. [`std::time::Duration::try_from_secs_f64`] is the whole fix — negative, NaN,
/// infinite and overflowing all come back as the one error this already knew how to report.
fn stream_options_from(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<crate::stream::StreamOptions> {
    let seconds = |key: &str| -> Result<Option<std::time::Duration>> {
        match lookup(key).filter(|v| !v.is_empty()) {
            None => Ok(None),
            Some(raw) => {
                let secs: f64 = raw.parse().map_err(|_| {
                    NxfError::validation(format!("{key} must be seconds, got {raw:?}"))
                })?;
                let dur = std::time::Duration::try_from_secs_f64(secs).map_err(|_| {
                    NxfError::validation(format!(
                        "{key} must be a number of seconds that is a duration — not negative, not \
                         NaN, not infinite; got {raw:?}"
                    ))
                })?;
                Ok(Some(dur))
            }
        }
    };
    let mut opts = crate::stream::StreamOptions::default();
    if let Some(idle) = seconds("NXC_STREAM_IDLE")? {
        opts.idle = idle;
    }
    if let Some(poll) = seconds("NXC_STREAM_POLL")? {
        opts.poll = poll.max(MIN_STREAM_POLL);
    }
    // The knock has to fit inside the window it warns about, however short that window was made.
    opts.knock_lead = opts.knock_lead.min(opts.idle / 6);
    Ok(opts)
}

/// One streamed event: the thread and the transcript, labelled apart (surface draft §4.5).
fn print_stream_event(json: bool, event: &crate::stream::Event) {
    match event {
        crate::stream::Event::Said {
            message_id,
            sender,
            body,
            foreign,
        } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "stream": "said", "message_id": message_id, "sender": sender,
                        "body": body, "foreign": foreign
                    })
                );
            } else {
                println!("\n{sender}: {body}");
            }
        }
        crate::stream::Event::Did {
            session,
            seq,
            kind,
            detail,
        } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "stream": "did", "session": session, "seq": seq,
                        "kind": kind, "detail": detail
                    })
                );
            } else if detail.is_empty() {
                println!("  · [{kind}]");
            } else {
                println!("  · [{kind}] {detail}");
            }
        }
    }
}

// ---- reply + the channel-completion tick ------------------------------------
//
// Both verbs are adapters now (nxf 6j6v.7dw6): the requester wake and the declared-channel
// `on_complete` routing they can drive both live in [`crate::orchestration`], so the CLI and the
// library handle run the SAME implementation.

/// **Take up an operation that stopped at an availability boundary** (`nxc resume`, nxf 6j6v.npy3).
///
/// A full orchestration context, because this verb STARTS a session and may arm a deadline —
/// exactly `session deliver`'s two reasons, in one verb.
///
/// **Every line below reports a decision NOT to start something**, except the last. That is the
/// shape of the feature rather than of this function: an interrupted turn may already have written
/// files and posted messages, so the world is checked before anything is run, and what a person
/// most often needs to be told is which check stopped it.
fn resume(
    json: bool,
    db: Option<&str>,
    thread: Option<&str>,
    session: Option<&str>,
    force: bool,
) -> Result<()> {
    use crate::orchestration::{ResumeOutcome, ResumeRequest, ResumeScope};
    let scope = match (thread, session) {
        (Some(thread), _) => ResumeScope::Thread(thread),
        (None, Some(session)) => ResumeScope::Session(session),
        // clap's `required_unless_present` already refuses this; the arm exists so the impossible
        // case is a stated refusal rather than a panic if that attribute is ever edited.
        (None, None) => {
            return Err(NxfError::validation(
                "name the operation to take up: `--thread <id>` (what `nxc status` shows) or \
                 `--session <id>`",
            ))
        }
    };
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let mut store = open(db)?;
    let r = crate::orchestration::resume_interrupted(
        &cli_ctx.ctx(),
        &mut store,
        ResumeRequest { scope, force },
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&r).expect("resume receipt serializes")
        );
        return Ok(());
    }
    let named = r.thread.as_deref().unwrap_or("-");
    match &r.outcome {
        ResumeOutcome::NothingOnHold => println!(
            "nothing is on hold here — this operation was never interrupted, or it has already \
             been taken up"
        ),
        ResumeOutcome::NotYet { until } => {
            // **Not "handed on" when its park was refused** (nxf 6j6v.8bv9): a refusal that can
            // pass holds the copy, and the `!` line below says what to finish. Saying the copy moved
            // above a finding that says it did not would leave the reader to pick one.
            let copy = match r
                .warnings
                .iter()
                .any(|w| w.class == crate::orchestration::ConsequenceClass::WorkNotParked)
            {
                true => {
                    "Its working copy is still held — its park was refused (see below) and is \
                     retried on every tick"
                }
                false => "Its working copy has been handed on",
            };
            match until {
                Some(until) => println!(
                    "not yet: the model is not available to this operation until {until}. {copy}, \
                     and it is taken up on its own once the window lifts. `--force` starts it now \
                     — with another account, say"
                ),
                // The one state nothing will ever leave on its own, so the line says so rather
                // than leaving a person waiting for a service that has nothing to wait for.
                None => println!(
                    "on hold, and the runtime stated no reset instant — so NOTHING will take this \
                     up on its own. {copy}. Run this again with `--force` when you know the model \
                     is available again"
                ),
            }
            if let Some(armed) = &r.armed_for {
                println!("  · armed to be taken up at {armed}");
            }
        }
        ResumeOutcome::AlreadyAnswered => println!(
            "nothing to continue: thread {named} was already answered before the interruption, so \
             the round is finished and the hold is closed"
        ),
        ResumeOutcome::AlreadyRunning => println!(
            "already running: a process is working for session {} right now, so nothing was \
             started — one session, one process",
            r.session.as_deref().unwrap_or("-")
        ),
        ResumeOutcome::TreeMoved { drift } => {
            println!(
                "NOT resumed: the working copy has moved since this operation was interrupted, so \
                 a question went back to the thread that started it"
            );
            if let Some(commit) = &drift.commit_now {
                println!("  · HEAD is a different commit now: {commit}");
            }
            if let Some(branch) = &drift.branch_now {
                match branch.is_empty() {
                    true => println!("  · the working copy is on a detached HEAD now"),
                    false => println!("  · the working copy is on branch {branch} now"),
                }
            }
            if drift.uncommitted_now {
                println!("  · the commit is where it was and the uncommitted work is not");
            }
        }
        ResumeOutcome::ProviderCannotResume => println!(
            "NOT resumed: this workspace's runtime cannot continue a conversation it began, so \
             continuing would start a fresh session with none of this one's history — which is \
             exactly what stops the work being done twice"
        ),
        ResumeOutcome::Resumed => println!(
            "resumed {} on thread {named}, with its own transcript in front of it",
            r.session.as_deref().unwrap_or("-")
        ),
    }
    for w in &r.warnings {
        println!("  ! {}", w.detail);
    }
    Ok(())
}

/// `nxc withdraw --thread <thread-id>` (nxf 6j6v.0h3p; a running round since nxf 6j6v.b9nf).
///
/// An ADAPTER, like every verb in this file: it resolves the ambient context and hands the decision
/// to [`crate::orchestration::withdraw`], whose doc carries the whole argument — the two cases,
/// why the queue removal is itself the precondition check for the queued one, why the running one
/// is refused whole on a host that cannot stop a session, and what the discharged thread is left
/// looking like.
///
/// **A stop that did not land changes the exit code** (`changes_the_exit_code`'s rule, applied):
/// the receipt is printed whole — `--json` carries `stopped` and `warnings` as one document — and
/// the process exits 1, because the one act this verb exists for on a running round did not
/// happen and a script that never parses JSON has to learn that the session is still writing.
fn withdraw(json: bool, db: Option<&str>, thread: &str) -> Result<()> {
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let ctx = cli_ctx.ctx();
    let mut store = open(db)?;
    let receipt = crate::orchestration::withdraw(&ctx, &mut store, thread)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&receipt).expect("withdraw receipt serializes")
        );
    } else {
        for w in &receipt.withdrawn {
            println!("withdrew {} on thread {} (never started)", w.role, w.thread);
        }
        // One line per STOPPED session (nxf 6j6v.b9nf), naming the session: the stop is a request
        // delivered to a process, and the session id is what a person looks for if it does not go.
        for s in &receipt.stopped {
            match &s.already_gone {
                None => println!(
                    "stopped {} on thread {} (session {} was asked to end)",
                    s.role, s.thread, s.session
                ),
                // **The goal state, said as one** (nxf 6j6v.b9nf, fix round 3, Code Quality #6):
                // nothing had to be signalled because the session was already over. The commission
                // is withdrawn either way, and this line exists so that "no line about a signal"
                // is never left to be inferred from silence.
                Some(note) => println!(
                    "withdrew {} on thread {} — session {} had already ended, so there was nothing \
                     to stop ({note})",
                    s.role, s.thread, s.session
                ),
            }
        }
        // Never silent about what it did NOT take back — an entry the release path fired between
        // this call's read and its write is RUNNING, and a caller that believed otherwise would be
        // worse off than one that never called.
        for thread in &receipt.started_meanwhile {
            println!(
                "NOT withdrawn: thread {thread} started while this ran — it has a session now"
            );
        }
        // What becomes of the work, said either way for a stopped round: a caller who took back a
        // running coder needs to know whether to expect a branch.
        //
        // **Three states, not two** (fix round 2 of this item's review, Code Quality #1 and
        // Integrity #4). `will_park` false used to mean one thing and now covers two, and the
        // middle one — work in the checkout that this host can never park — printed the sentence
        // belonging to the round that holds nothing at all. Both park lines also name the tick now:
        // the park and the unparked hand-on are BOTH the tick's, and on a workspace with no
        // background service nothing else will ever run one.
        if receipt.will_park {
            println!(
                "its work will be parked on a branch once nothing in this operation is still \
                 running (nothing is rolled back): `nxc status` lists the branch under the \
                 operation, and the next commission into the same thread — `nxc reply --thread \
                 {0}` — brings it back. `nxc tick --thread {0}` does the park by hand if no \
                 background service is running",
                receipt.thread_id
            );
        } else if let Some(why) = &receipt.cannot_park {
            println!(
                "its work is in this workspace's working copy and CANNOT be parked on a branch \
                 here ({why}) — the copy is handed on as it stands once nothing in this operation \
                 is still running, with the work left in the tree. `nxc tick --thread {}` does \
                 that hand-off by hand if no background service is running",
                receipt.thread_id
            );
        } else if !receipt.stopped.is_empty() {
            println!(
                "nothing of it is in this workspace's working copy, so there is nothing to park"
            );
        }
        if !receipt.withdrawn.is_empty() || !receipt.stopped.is_empty() {
            // **The chain below the named thread is over, and nothing moves it on** (nxf
            // 6j6v.s2cj). This line used to say the opposite — "the round above is now settled —
            // `nxc tick` moves it on" — which was true while a withdrawal left the round above it
            // standing, and is the sentence the owner's decision of 2026-09-20 took back: a
            // taken-back step does not commission its successor. Only a new commission starts
            // anything there again, and the park line above already names the one that brings
            // parked work back.
            //
            // **True on every successful call, and said with its one exception** (review of PR
            // #483, Code Quality #2). The interrupt runs before anything is taken back and
            // unconditionally, so the registers above the taken-back work ARE discharged — but a
            // queued commission that started while this ran is RUNNING below them, and a sentence
            // that said nothing below is being commissioned would be false about it.
            if receipt.started_meanwhile.is_empty() {
                println!(
                    "operation {} is interrupted: nothing below it is commissioned or consolidated \
                     any more",
                    receipt.thread_id
                );
            } else {
                println!(
                    "operation {0} is interrupted, except for what started while this ran — `nxc \
                     withdraw --thread {0}` again stops that too",
                    receipt.thread_id
                );
            }
        }
        // stderr, beside the `--json` field — the same split every other receipt's warnings take.
        for w in &receipt.warnings {
            eprintln!("warning: {w}");
        }
    }
    if changes_the_exit_code(&receipt.warnings) {
        // `send`'s choice, for `send`'s reason (nxf 6j6v.hpv8): the discharge above IS persisted,
        // so this is not `error::emit`'s envelope — the receipt printed above is the one document.
        // `store` dropped explicitly first: `std::process::exit` skips `Store`'s `Drop`.
        drop(store);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(1);
    }
    Ok(())
}

/// Keyed on the THREAD, never on anything above it (see `Command::Tick`'s own doc).
///
/// Since nxf 6j6v.wyhx this verb is an ADAPTER: the due-check, the declared-channel resolve, the
/// per-policy idempotency marker check, the return-address resolve and the routing itself all live
/// in [`crate::orchestration::tick`] — whose doc comment carries the full reasoning for each,
/// including why the ordinary due-check cannot stand in for the marker check. All that is left here
/// is rendering.
///
/// **It has no library-seam twin, deliberately** (6j6v.dvyq §5's closed list, recorded as a
/// `KnownGap` in `tests/verb_seam.rs`): an embedding app does not drive a board's deadline by hand,
/// the scheduled job does, and the job invokes this binary.
/// `nxc tick --thread <thread_id>`: what a declared channel's scheduled one-shot timer actually
/// invokes when it fires (`timer.rs`'s `schedule_tick`) — nothing ever waits synchronously for it.
///
/// It carries a SECOND job since nxf 6j6v.fabb, and the two are deliberately unrelated: before it
/// looks at the thread at all, it reclaims this workspace's working-tree lease if that lease is
/// past its bound and somebody is parked behind it. See `orchestration::sweep_expired_working_tree`
/// for why the drain rides this verb rather than one of its own.
fn tick(json: bool, db: Option<&str>, thread_id: &str) -> Result<()> {
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let mut store = open(db)?;
    let r = crate::orchestration::tick(
        &cli_ctx.ctx(),
        &mut store,
        crate::orchestration::TickRequest { thread_id },
    )?;
    print_tick_result(json, &r);
    Ok(())
}

/// Render `nxc tick`'s result. `--json`'s exact shape is [`crate::orchestration::TickReceipt`]'s
/// own: `{thread_id, acted, reason, delivered?, woke?, wake_skipped?, warnings, outstanding,
/// working_tree?, parked?}` —
/// `delivered` only present when `acted` is true (the receipt's `skip_serializing_if`), the routing
/// findings only when there is something to report, and `warnings` (nxf 6j6v.93zd — everything this
/// tick should have caused and did not) and `outstanding` (the expected handles that had not yet
/// replied at the moment tick decided whether to act) ALWAYS present, so a caller/human can tell who
/// didn't respond, and what did not happen, even for a no-op.
///
/// The findings reach the CLI for the same reason `nxc reply --json` renders `wake_skipped` (nxf
/// 6j6v.bxdd, and the PR #265 review for its sibling): this verb exits 0 whether or not the hand-off
/// landed, so `--json` would otherwise be the one place a reader looks that never learns the
/// requester was left un-woken. They cost nothing here — the receipt is serialized whole, so no
/// adapter code can drift from the library seam's answer.
/// What a hand-off of the working copy started, in words — the THREE lists plus what starting them
/// noticed, written once for the two hand-offs a tick can report (nxf 6j6v.de9s).
///
/// It was inline in the sweep's block and the park needs exactly the same account; a second copy is
/// how two reports of one movement start describing it differently. Every line is on stdout with
/// the header above it rather than on stderr with `r.warnings`, deliberately: this block is the
/// operator's only trace of an automatic hand-off, and splitting one account across two streams is
/// how half of it goes missing in a log. The `--json` receipt carries it either way.
fn print_promotions(promotions: &crate::orchestration::Promotions) {
    for p in &promotions.started {
        println!(
            "started {} on thread {} (it had been waiting for it)",
            p.role, p.thread
        );
    }
    for p in &promotions.requeued {
        println!(
            "back in line: {} on thread {} — the working copy had moved on again by the time it \
             was started, so it is waiting again, in the place it already had",
            p.role, p.thread
        );
    }
    for p in &promotions.failed {
        println!(
            "NOT started: {} on thread {} — its message is persisted and session {} is minted, but \
             nothing is running for it",
            p.role, p.thread, p.session
        );
    }
    // What starting the promoted entries NOTICED (nxf 6j6v.pkw9).
    for w in &promotions.warnings {
        println!("note: {w}");
    }
}

/// **The line a tick's sweep block opens with** — pulled out to its own pure function so the one
/// thing it must not do can be unit-tested without staging a lost compare-and-swap (fix round 1 of
/// nxf 6j6v.8bv9; the same reason `orchestration::park_safety_note` is a function).
///
/// What it must not do is claim a hand-off that did not happen. Since 6j6v.8bv9 the sweep PARKS
/// before it reclaims — that is the whole point, the work is saved first — so a `WorkingTreeSweep`
/// can now report a park whose reclaim then matched nothing, because the holder renewed its lease in
/// between. [`crate::orchestration::sweep_expired_working_tree`] deliberately still returns that
/// sweep (a commit on a branch is movement a receipt must carry), and this block used to open it
/// with "reclaimed this workspace's working copy from X" over a lease that never moved.
///
/// The promotions ARE the test: [`crate::orchestration::release_and_fire`] fires nothing at all when
/// the reclaim matches nothing, so all three lists empty is exactly "the copy stayed where it was".
/// That question is [`crate::orchestration::Promotions::nothing_moved`]'s and is asked of it rather
/// than re-derived here, so a field added to `Promotions` cannot be answered two ways.
fn sweep_headline(swept: &crate::orchestration::WorkingTreeSweep) -> String {
    if !swept.promotions.nothing_moved() {
        format!(
            "reclaimed this workspace's working copy from {} (its lease ran out at {})",
            swept.scope, swept.expired
        )
    } else {
        format!(
            "{} is past its bound ({}) and its work was saved, but this workspace's working copy \
             did NOT move: that operation renewed its claim while its work was being parked, so it \
             still holds the copy — and its own next trigger is told where the work is",
            swept.scope, swept.expired
        )
    }
}

fn print_tick_result(json: bool, r: &crate::orchestration::TickReceipt) {
    if json {
        println!(
            "{}",
            serde_json::to_string(r).expect("tick receipt serializes")
        );
        return;
    }
    // FIRST, and independent of everything below (nxf 6j6v.fabb; review of this branch, Code
    // Quality #6 / Integrity #7). The sweep is not about this thread, and it must not be reported
    // through a branch that is: a tick that reclaimed the working copy and started another agent
    // would otherwise print "nothing to do".
    //
    // It matters most on the one surface nobody watches. A tick started by the background service
    // (nxf 6j6v.8see) writes into `~/.nexusflow/logs/service.log` — so this is the only trace an
    // operator has of an automatic drain.
    //
    // **The park is printed FIRST, in the tick's own order** (nxf 6j6v.de9s): it runs before the
    // sweep, and on the one occasion both have something to say, a log that told them in the other
    // order would read as if the copy had been reclaimed and only then saved.
    if let Some(parked) = &r.parked {
        println!(
            "parked {}: it escalated and nobody answered, and the working copy has been contended \
             since {}. Its work is committed on branch {} at {}, and the tree is back on {}",
            parked.scope,
            parked.contended_since,
            parked.parked.branch,
            parked.parked.commit,
            if parked.parked.base_branch.is_empty() {
                parked.parked.base_commit.as_str()
            } else {
                parked.parked.base_branch.as_str()
            }
        );
        if !parked.parked.committed {
            println!("(there was nothing uncommitted to save — the tree was already clean)");
        }
        print_promotions(&parked.promotions);
    }
    // **Every other hand-off the park step made** (nxf 6j6v.8bv9), in the same place and for the
    // same reason: a copy that moved is the operator's business whether or not its work was parked
    // first — and the one that moved UNPARKED most of all, which is also on stderr as a warning.
    if let Some(handed) = &r.handed_on {
        let why = match handed.occasion {
            crate::orchestration::ParkOccasion::StrandedEscalation => {
                "it escalated and nobody answered while another operation waited"
            }
            crate::orchestration::ParkOccasion::AvailabilityBoundary => {
                "it is on hold at an availability boundary while another operation waits"
            }
            // The sweep past the bound reports its own hand-off on `working_tree` below, where its
            // `expired` belongs — so this arm is here to keep the match exhaustive (a further
            // occasion must still be a compile error at this site) and to stay truthful if that ever
            // changes.
            crate::orchestration::ParkOccasion::PastItsBound => {
                "its lease ran out with nothing running behind it while another operation waits"
            }
            // …and its sibling DOES arrive here (nxf 6j6v.xb24): the lease was still standing, so
            // there is no expiry to report and nothing to put on `working_tree`.
            crate::orchestration::ParkOccasion::DiedInsideItsBound => {
                "nothing in it is running any more and it never answered what it owed, so it died \
                 holding the copy while another operation waits"
            }
            // …and the one occasion that is somebody's decision (nxf 6j6v.b9nf): no clock, no
            // death, and no waiter required — the round was taken back.
            crate::orchestration::ParkOccasion::Withdrawn => {
                "its running round was withdrawn and nothing in it is running any more"
            }
        };
        match (&handed.parked, &handed.unparked) {
            (Some(parked), _) => println!(
                "handed on the working copy of {}: {why}. Its work is committed on branch {} at {}",
                handed.scope, parked.branch, parked.commit
            ),
            (None, Some(refusal)) => println!(
                "handed on the working copy of {} WITHOUT parking its work ({}): {why}, and its \
                 park cannot happen — whatever it left uncommitted is still in the tree",
                handed.scope,
                refusal.kind()
            ),
            (None, None) => println!("handed on the working copy of {}: {why}", handed.scope),
        }
        print_promotions(&handed.promotions);
    }
    if let Some(swept) = &r.working_tree {
        println!("{}", sweep_headline(swept));
        // **What happened to that operation's work** (nxf 6j6v.8bv9), on the same block and for the
        // reason the whole block is on stdout: this is the operator's only trace of an automatic
        // hand-off, and where the work went is the half they would otherwise have to go looking for.
        match (&swept.parked, &swept.unparked) {
            (Some(parked), _) if parked.committed => println!(
                "its work is committed on branch {} at {}, and the tree is back on {}",
                parked.branch,
                parked.commit,
                if parked.base_branch.is_empty() {
                    parked.base_commit.as_str()
                } else {
                    parked.base_branch.as_str()
                }
            ),
            (Some(parked), _) => println!(
                "it had nothing uncommitted to save — its working copy is safe at {} on {}",
                parked.commit, parked.branch
            ),
            (None, Some(refusal)) => println!(
                "its work was NOT parked ({}) — whatever it left uncommitted is still in the tree, \
                 where the next holder will find it",
                refusal.kind()
            ),
            (None, None) => {}
        }
        print_promotions(&swept.promotions);
    }
    if r.acted {
        println!(
            "tick acted on thread {}: {}",
            r.thread_id,
            r.delivered.as_deref().unwrap_or_default()
        );
    } else if r.reason == crate::orchestration::TICK_WAITING_FOR_A_SESSION {
        // **Not a no-op either** (review of PR #361, Code Quality #1): the board is settled and
        // ready to move, and the one thing left is a session that has not finished. Nothing watches
        // that on its own — the declared `timeout:` is a deadline for an ANSWER, and this member has
        // answered — so the honest line names what it is waiting for and how the wait ends.
        println!(
            "tick declined on thread {}: the set is settled, but a session behind it is still \
             running. It moves on when that session ends (the sidecar reports it) — or run this \
             again once it has.",
            r.thread_id
        );
    } else if r.reason == crate::orchestration::TICK_ADVANCED {
        // **Not acted, and not a no-op either** (nxf 6j6v.rs9k): an ordered flow whose step let its
        // window run out moved on to the next step, which routed no completion and delivered
        // nothing. Rendering it through the branch below would print "nothing to do" over a flow
        // that just started a session — so the findings say what happened, exactly as `--json`
        // carries them.
        println!(
            "tick advanced the flow on thread {}{}",
            r.thread_id,
            r.warnings
                .first()
                .map(|w| format!(": {w}"))
                .unwrap_or_default()
        );
    } else {
        println!(
            "tick: nothing to do for thread {} ({})",
            r.thread_id, r.reason
        );
    }
}

// `fn reply` stood here. It was the dispatcher's entry point and, since 6j6v.dvyq §3 made `--thread`
// required, a pure delegation to `reply_thread` below with an identical signature — kept because its
// doc carried the account of the OTHER shape it used to dispatch (a positional `<thread|message>`
// target) and of the seam method that still resolved both, `Engine::reply`.
//
// Both halves of that account are gone. `Engine::reply` was removed by 6j6v.ckeq, so a conversation
// has one address on every surface, and the wrapper had nothing left to say. What its doc explained
// that still matters — that this verb is an ADAPTER over `orchestration::reply` and that
// `--escalate` is parsed once, not twice — is on `reply_thread` and on `surface::reply_kind`,
// which is where a reader looking at the code that runs will find it. The dispatcher calls
// `reply_thread` directly.

/// `nxc reply --thread <thread-id> <BODY>|- [--body-file <path>] [--stream]` (nxf 6j6v.p6m1,
/// surface draft §4.3; the body's three sources are nxf 6j6v.s46h — see [`resolve_body`]).
///
/// The adapter over [`crate::surface::reply_in_thread`], which IS [`crate::orchestration::reply`]
/// plus the thread's own return address — so the quorum routing, the declared-channel `on_complete`
/// policy and the workflow advance are that verb's, unchanged.
///
/// The first four lines of this comment were deleted by 6j6v.dvyq §3 block (a) and the fifth was
/// not, leaving a fragment that began mid-sentence with "and" and still named the positional
/// `nxc reply <target>` the same block removed. Restored rather than deleted: this function is the
/// one that does the work — [`reply`] above is now pure delegation — so it is where a reader looks.
/// **The `reply` verb's own inputs, as ONE value** — the flags clap parsed, handed on together.
///
/// A struct rather than a parameter list, and not only because the list grew past clippy's bound
/// when `--needs-rework` joined it: `escalate` and `needs_rework` are two adjacent `bool`s that a
/// transposition would survive compilation, which is the exact shape [`crate::orchestration::Caller`]
/// and [`crate::orchestration::RoleSpawn`] are named fields for.
struct ReplyArgs<'a> {
    body: &'a str,
    escalate: bool,
    needs_rework: bool,
    accept: bool,
    /// The conversation to answer, or `None` for the id-free form (nxf 6j6v.dq59) — which resolves
    /// only while exactly one conversation is open for this caller, and otherwise refuses by naming
    /// them all.
    thread: Option<&'a str>,
    if_unanswered: bool,
    stream: bool,
    /// `--machine`: hand the chat to that machine (nxf 6j6v.1c6k).
    machine: Option<&'a str>,
}

fn reply_thread(json: bool, db: Option<&str>, args: ReplyArgs<'_>) -> Result<()> {
    let ReplyArgs {
        body,
        escalate,
        needs_rework,
        accept,
        thread,
        if_unanswered,
        stream,
        machine,
    } = args;
    // The positional arity check that stood here is clap's now (6j6v.dvyq §3): `reply` takes one
    // required positional and the thread is a flag.
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let ctx = cli_ctx.ctx();
    let mut store = open(db)?;
    // **The id-free form, resolved BEFORE anything is written** (nxf 6j6v.dq59) — the same
    // discipline `--stream`'s refusals below stand on: a caller left with a posted message and an
    // error is the one outcome worse than either. `where this session is standing` is passed too,
    // so this asks exactly the question the wake answered — see `crate::addressees`.
    let resolved;
    let thread = match thread {
        Some(named) => named,
        None => {
            resolved = crate::addressees::sole_open_thread(
                &store,
                &ctx.caller_handle(),
                crate::facade::parent_thread_of(&store, ctx.session).as_deref(),
                ctx.now,
            )?;
            resolved.as_str()
        }
    };
    // Both `--stream` refusals — who may follow, and whether the follow's timings parse — happen
    // before the reply is written, for the reason `send --to` states at its own call site.
    let follow_opts = if stream {
        refuse_stream_for_a_persona(&crate::persona::resolve_identity(
            &store,
            ctx.defs.roles(),
            None,
            ctx.session,
        )?)?;
        Some(stream_options()?)
    } else {
        None
    };

    let req = crate::surface::ReplyThreadRequest {
        machine,
        thread,
        body,
        escalate,
        needs_rework,
        accept,
    };
    // The flag picks the DOOR, not a field on the request (nxf 6j6v.ckeq, decision 4): the settle is
    // the sidecar teardown's, and giving it its own named entrance is what keeps it off the app seam
    // while leaving the CLI — the teardown's only caller — able to reach it. Both doors run one
    // body, so a `--if-unanswered` reply that IS owed behaves exactly like a plain one.
    let r = if if_unanswered {
        crate::surface::settle_if_unanswered(&ctx, &mut store, req)?
    } else {
        crate::surface::reply_in_thread(&ctx, &mut store, req)?
    };
    if json {
        // The receipt whole, exactly as `nxc reply --json` renders its own (review finding 4: both
        // shapes now agree that `posted` is unconditional, never additive): `woke`/`wake_skipped`
        // are the only place a caller learns its hand-off did not land, since this verb exits 0
        // either way (see `orchestration::reply`).
        println!(
            "{}",
            serde_json::to_string(&r).expect("reply receipt serializes")
        );
    } else if let Some(message_id) = &r.message_id {
        println!("replied {message_id} in thread {thread}");
        if let Some(to) = &r.handed_to {
            println!("{}", handed_to_line(to));
        }
        // **The same treatment `send --to` gives the requester** (nxf 6j6v.7qfm): whoever hands a
        // turn back has no more idea what happens next than the requester had, and until this the
        // line above was the whole of what they were told. Only on the branch that POSTED — a
        // `--if-unanswered` call that found nothing owed handed nothing over, so there is nothing
        // for it to learn the end of.
        // **The third outcome of a hand-off** (nxf 6j6v.gn8b), said before the guidance: the
        // answer landed and its caller is working, so it is queued rather than lost. Without this
        // line the replier reads "replied …" and nothing else, exactly as it did when the answer
        // really was dropped.
        if let Some(session) = &r.held {
            println!(
                "  held for session {session}, which is working — it is resumed with this, and \
                 with anything else that arrives meanwhile, once it settles"
            );
        }
        if let Some(guidance) = &r.await_ {
            println!("{}", guidance.render_human(thread));
        }
    } else {
        // `--if-unanswered` found nothing owed on this thread (review finding 5: `r.message_id` is
        // genuinely `None` here now that the flag is reachable via `--thread` too, not just an
        // unreachable case guarded elsewhere) — matches the plain path's wording.
        println!("nothing owed on this thread {thread}");
        // …and the guidance follows here TOO (review of nxf 6j6v.gn8b, Code Quality). It was
        // rendered only on the posting branch, on the argument that a call which handed nothing over
        // has nothing to learn the end of — and `crate::surface` fills the field on this branch
        // anyway, with the opposite argument written out at the assignment: the thread exists either
        // way and where it stands is exactly as answerable. Two places saying different things about
        // one field is the drift, so the surface's reason wins and the human sees what `--json`
        // carries.
        if let Some(guidance) = &r.await_ {
            println!("{}", guidance.render_human(thread));
        }
    }
    if let Some(opts) = follow_opts {
        follow_conversation(
            &store,
            thread,
            &ctx.caller_handle(),
            r.woke.as_deref(),
            json,
            opts,
        )?;
    }
    Ok(())
}

/// The line a chat handed to another machine gets (nxf 6j6v.1c6k): who runs it, whether it is
/// online, and when it starts.
fn handed_to_line(to: &crate::machine::ExecutingMachine) -> String {
    let when = match to.online {
        Some(false) => format!(
            "it is not online (last seen {}), so this waits until it is back",
            crate::machine::seen_ago(to.age_secs.unwrap_or_default())
        ),
        _ => "it starts there after that machine's next pull, usually within half a minute"
            .to_string(),
    };
    format!(
        "  runs on {} — nothing starts on this machine; {when}",
        to.label()
    )
}

/// `nxc machine --to <persona> | --thread <id> [--machine <m>]` (nxf 6j6v.1c6k): the question, as
/// data, before anything is sent.
fn machine_verb(
    json: bool,
    db: Option<&str>,
    to: Option<&str>,
    thread: Option<&str>,
    machine: Option<&str>,
) -> Result<()> {
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let ctx = cli_ctx.ctx();
    let store = open(db)?;
    let query =
        match (to, thread) {
            (Some(to), _) => crate::surface::MachineQuery::Persona { to, machine },
            (None, Some(thread)) => crate::surface::MachineQuery::Thread { thread, machine },
            (None, None) => return Err(NxfError::validation(
                "name the chat: --to <persona> for a new one, --thread <id> for one that exists",
            )),
        };
    let answer = crate::surface::machine(&ctx, &store, query)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&answer).expect("machine answer serializes")
        );
        return Ok(());
    }
    match &answer.machine {
        Some(m) => {
            let state = match (m.here, m.online) {
                (true, _) => "this machine".to_string(),
                (false, Some(true)) => format!(
                    "online, last seen {}",
                    crate::machine::seen_ago(m.age_secs.unwrap_or_default())
                ),
                (false, Some(false)) => format!(
                    "NOT online, last seen {}",
                    crate::machine::seen_ago(m.age_secs.unwrap_or_default())
                ),
                (false, None) => "online state unknown".to_string(),
            };
            let from = match m.source {
                crate::machine::DesignationSource::Choice => "chosen with --machine",
                crate::machine::DesignationSource::Chat => "the chat's machine",
                crate::machine::DesignationSource::Persona => "the persona's machine:",
                crate::machine::DesignationSource::StartedHere => "the machine that starts it",
            };
            println!("runs on {} — {state}; {from}", m.label());
        }
        None if !answer.must_ask => {
            println!("runs where it is written — this host names no machine");
        }
        None => {}
    }
    if answer.must_ask {
        println!("would ASK: {}", answer.question("--machine <id|name>").msg);
    } else if !answer.online.is_empty() {
        let names: Vec<String> = answer
            .online
            .iter()
            .map(|m| format!("{} ({})", m.name, m.machine_id))
            .collect();
        println!("online: {}", names.join(", "));
    }
    Ok(())
}

/// `nxc pick-up` (nxf 6j6v.1c6k): what the service runs after a pass pulled something.
fn pick_up_verb(json: bool, db: Option<&str>) -> Result<()> {
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let ctx = cli_ctx.ctx();
    let mut store = open(db)?;
    let report = crate::orchestration::pick_up(&ctx, &mut store)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&report).expect("pickup report serializes")
        );
    } else {
        if report.machine.is_none() {
            println!("this host names no machine, so it picks up nothing");
        }
        for s in &report.served {
            println!(
                "{} {} in thread {} (session {}, {} message(s))",
                match (s.started, s.held) {
                    (true, _) => "started",
                    (false, true) => "held for",
                    (false, false) => "resumed",
                },
                s.persona,
                s.thread,
                s.session,
                s.messages.len()
            );
        }
        for f in &report.failed {
            println!("could not pick up thread {}: {}", f.thread, f.reason);
        }
    }
    if report.failed.is_empty() {
        Ok(())
    } else {
        Err(NxfError::io(format!(
            "{} chat(s) this machine runs could not be picked up; the next pickup tries again",
            report.failed.len()
        )))
    }
}

// ---- threads ---------------------------------------------------------------

// `fn ask` was here. REMOVED with the verb (6j6v.dvyq §3). `facade::ask`/`ask_under` — the WRITE
// (mint a thread, declare its expects, stamp the deadline, post the request) — stay: they are what
// `open_declared_channel_and_fan_out` and the consolidation claim are built out of. What went is
// the ROUTER above them, `orchestration::ask`, whose whole job was deciding between a declared
// channel and a raw one.

/// `nxc threads [list|show]`: the quorum board surface.
fn threads(json: bool, db: Option<&str>, action: &ThreadsAction) -> Result<()> {
    match action {
        ThreadsAction::List { consumer, channel } => {
            threads_list(json, db, consumer.as_deref(), channel.as_deref())
        }
        ThreadsAction::Show {
            thread_id,
            consumer,
        } => threads_show(json, db, thread_id, consumer.as_deref()),
        ThreadsAction::Name { thread_id, name } => {
            threads_name(json, db, thread_id, name.as_deref())
        }
    }
}

/// `nxc threads name <thread-id> [<name>]`: what this conversation is called (nxf 6j6v.e76c).
///
/// **Two forms, one write.** With a name, it is stated; without one, it is DERIVED from the
/// thread's opening message through the namer this process resolves — which is exactly the run
/// `send` commissions in the background, so the automatic path and the by-hand one are the same
/// code and cannot come to disagree.
///
/// A thread that is already named keeps its name and reports `named: false`. That is a no-op rather
/// than an error for the reason [`crate::facade::name_thread`] states: the commissioned run can be
/// retried, and a retry that errors turns an idempotent convenience into something a caller has to
/// reason about.
fn threads_name(json: bool, db: Option<&str>, thread_id: &str, name: Option<&str>) -> Result<()> {
    let cli_ctx = CliCtx::resolve(db, hop())?;
    let ctx = cli_ctx.ctx();
    let mut store = open(db)?;
    let receipt = match name {
        Some(name) => {
            crate::facade::name_thread(&mut store, ctx.now, &ctx.caller_handle(), thread_id, name)?
        }
        None => crate::surface::name_thread_from_its_opening_message(&ctx, &mut store, thread_id)?,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string(&receipt).expect("name receipt serializes")
        );
    } else {
        match (&receipt.name, receipt.named) {
            (Some(name), true) => println!("{} · {name}", receipt.thread_id),
            (Some(name), false) => {
                println!(
                    "{} · {name} (already named — left as it is)",
                    receipt.thread_id
                )
            }
            (None, _) => println!(
                "{} · no name could be derived — it keeps its id",
                receipt.thread_id
            ),
        }
    }
    Ok(())
}

/// `nxc threads list [--consumer <h>] [--channel <id>]`: the caller's membership-scoped boards with
/// bulk quorum state, deterministically ordered (channel + thread ULID).
fn threads_list(
    json: bool,
    db: Option<&str>,
    consumer: Option<&str>,
    channel: Option<&str>,
) -> Result<()> {
    let store = open(db)?;
    let consumer = resolve_consumer(consumer, db)?;
    let boards = crate::facade::threads(&store, &consumer, &resolve_now()?, channel)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&boards).expect("boards serialize")
        );
    } else if boards.is_empty() {
        println!("no threads");
    } else {
        for b in &boards {
            let q = &b.quorum;
            let state = if q.complete {
                "complete"
            } else if q.stale {
                "stale"
            } else {
                "waiting"
            };
            // The name where the channel id used to stand alone (nxf 6j6v.e76c) — `status_line`'s
            // choice, for its reason: on a direct conversation that column is a hash.
            println!(
                "{}  {}  {}/{} in  [{}]{}",
                q.thread_id,
                q.name
                    .clone()
                    .or_else(|| q.channel_id.clone())
                    .unwrap_or_default(),
                q.replied.len(),
                q.expects.len(),
                state,
                working_tree_suffix(b.working_tree, b.working_tree_queue_position)
            );
        }
    }
    Ok(())
}

/// `nxc status [--thread <id> | [--channel <name>] [--all]]`: where an operation stands (nxf
/// 6j6v.a71h §3.2), and since nxf 6j6v.1vxs only where it still stands somewhere.
///
/// One call into [`crate::facade::status`] — the same read [`crate::engine::Engine::status`] serves
/// an app — and then rendering. `--json` IS the [`crate::facade::StatusReport`] verbatim.
fn status(
    json: bool,
    db: Option<&str>,
    thread: Option<&str>,
    channel: Option<&str>,
    all: bool,
) -> Result<()> {
    let store = open(db)?;
    // `--thread` takes ONE id and the scope takes a set (nxf 6j6v.yr59 folded `thread_quorums`'
    // explicit set in here); the command line is the single-element case of it, unchanged in
    // behaviour down to the `not_found` a missing id gets.
    let one = [thread.unwrap_or_default()];
    // `--all` is the SAME selection with the liveness filter off (nxf 6j6v.1vxs), which is why it
    // rides on the channel rather than replacing it: `--all --channel review` is that channel's
    // finished operations too, and `--thread` already showed a finished tree, so the two conflict
    // at the parser rather than silently meaning the same thing.
    let scope = match (thread, channel, all) {
        (Some(_), _, _) => crate::facade::StatusScope::Threads(&one),
        (None, c, true) => crate::facade::StatusScope::All(c),
        (None, Some(c), false) => crate::facade::StatusScope::Channel(c),
        (None, None, false) => crate::facade::StatusScope::Workspace,
    };
    // **The worker stands at the WORKSPACE ROOT** (nxf 6j6v.npn0), not at the process cwd: the
    // per-thread session state (nxf 6j6v.qmy6) is a `Worker::session_is_running` read, and one
    // built from the cwd answers `false` for every live session whenever the operator ran `nxc`
    // from a subdirectory — the misread that made `nxc status` show a live session as not-running
    // was moved off for.
    //
    // **Built here rather than taken off `CliCtx`, and that is the whole difference between this
    // verb and its neighbours.** `CliCtx::resolve` also parses the declaration folder and reads the
    // project `CLAUDE.md`, so a workspace with one malformed persona file would fail this read —
    // and `status` is the verb whose whole job is to be safe to run against a workspace in ANY
    // state, which is exactly what somebody runs when something is wrong. It needs one path and
    // nothing else.
    let worker = LazyWorker {
        cwd: workspace_root(db)?,
    };
    let report = crate::facade::status(&store, &worker, &resolve_now()?, scope)?;
    // **A LISTING trims the rows the same way it trims the operations** (nxf 6j6v.1vxs). The two
    // forms that filter by liveness print only the threads that carry the REASON their operation is
    // still listed; `--thread` and `--all` were asked for a tree and get the whole of it.
    //
    // Measured on the workspace this item came from, after the operation filter above had already
    // done its work: six live operations, 328 lines — because four of the six are alive on ONE
    // escalated thread apiece and carry 122, 83, 38 and 21 answered ones behind it. The question
    // "is anything hanging?" was answerable and still did not fit on a screen. With the rows
    // trimmed it is 32 lines, and the thread a reader has to act on is one of them instead of one
    // in a hundred and twenty. Nothing is lost from `--json`, which is unfiltered in every form.
    let listing = thread.is_none() && !all;
    if json {
        println!(
            "{}",
            serde_json::to_string(&report).expect("status serializes")
        );
    } else if report.operations.is_empty() {
        // "nothing open" is the whole answer only for the form that asked about EVERYTHING; for the
        // two filtered listings it is the answer to a narrower question, and since nxf 6j6v.1vxs
        // that narrower question is the common one — so the line says where the rest went rather
        // than leaving a reader to conclude the workspace is empty.
        match all {
            true => println!("nothing here"),
            false => println!("nothing open (`nxc status --all` also shows finished operations)"),
        }
    } else {
        for op in &report.operations {
            // The operation-level facts, on the operation's OWN line, because that is the line an
            // operator reads before deciding whether to look further (nxf 6j6v.gk9j/6j6v.fabb).
            // `needs decision` is what separates a finished operation waiting to be read from one
            // that has STOPPED with a hand-back under it — both otherwise render as "awaiting you"
            // on the root and nothing else. `holds working tree` is what saved a reader from asking
            // `nxc threads show` about every thread in the tree to find out why a rival round is
            // parked. Both stay silent in the ordinary case.
            //
            // `finished` is the fourth, and it can only appear in the two forms that show a
            // not-live operation at all — `--thread` and `--all` (nxf 6j6v.1vxs). It is the one
            // word that says "you are looking at history": without it the two forms that mix live
            // and finished trees would render both the same way, and the whole point of taking the
            // finished ones out of the default view is that a reader can tell them apart.
            let mut marks: Vec<&str> = Vec::new();
            if !op.live {
                marks.push("finished");
            }
            if op.needs_decision {
                marks.push("NEEDS DECISION");
            }
            if op.holds_working_tree {
                marks.push("holds working tree");
            }
            // **The mark that stands where a false alarm used to** (nxf 6j6v.npy3). An exhausted
            // quota used to PRODUCE `NEEDS DECISION`: the runtime posted "I cannot carry this out"
            // in the session's name, which discharged the thread as an escalation and called a
            // human to a chain with nothing wrong with it. Nothing is escalated now, so that mark
            // stays silent — and without this one the operation would print with no mark at all and
            // no explanation for why nobody is working in it.
            if op.interrupted {
                marks.push("INTERRUPTED");
            }
            // **The mark that says why a working copy is not moving** (nxf 6j6v.8bv9): its park was
            // refused for a reason somebody can fix, and the claim is held until they do. Capitals,
            // like the other two marks that ask a human to look; the line under the header says
            // what to fix.
            if op.park_refused.is_some() {
                marks.push("PARK REFUSED");
            }
            // **The mark for a round taken back on purpose, still waiting to park** (nxf 6j6v.b9nf,
            // Integrity #2 of this item's review). Before this the operation printed nothing beyond
            // `holds working tree` — indistinguishable from an ordinary running round — while its
            // stopped session refused to leave and the tick re-armed a look every liveness cadence
            // forever. The line under the header names the sessions still pinning it.
            if op.withdrawn.is_some() {
                marks.push("WITHDRAWN");
            }
            // The fourth reason an operation is listed, and the only one with no flag of its own on
            // the record (nxf 6j6v.1vxs): a dead end that FAILED. Derived from the rows the
            // operation already carries, exactly as the two above are derived at the seam — without
            // it an operation listed for this reason alone would print `0 open` and no mark, and a
            // reader would have to scan the rows to find out why it is there at all.
            if op
                .threads
                .iter()
                .any(|t| t.state == crate::facade::ThreadState::Orphaned)
            {
                marks.push("dead end");
            }
            let flags = match marks.is_empty() {
                true => String::new(),
                false => format!("  · {}", marks.join(" · ")),
            };
            println!(
                "operation {}  {}  {} thread(s), {} open{flags}",
                op.root,
                op.channel_id.as_deref().unwrap_or(""),
                op.threads.len(),
                op.open
            );
            for line in operation_detail_lines(op) {
                println!("{line}");
            }
            let mut hidden = 0usize;
            for t in &op.threads {
                if listing && !says_why(t) {
                    hidden += 1;
                    continue;
                }
                println!("{}", status_line(t));
            }
            if hidden > 0 {
                println!(
                    "    … {hidden} answered thread(s) not shown — `nxc status --thread {}`",
                    op.root
                );
            }
        }
        // **Said once, and only where its absence is felt** (nxf 6j6v.qmy6/6j6v.t41e). A worker that
        // cannot ask the process question answers `unknown` for every unended session, so every open
        // thread above silently lost the one distinction this read added — and a reader who does not
        // know that reads the silence as "nothing wrong". Printed only when there IS an open thread
        // with a session it would have spoken about: on a report where nothing was going to say
        // anything anyway, the notice would be a warning about a fact that changed nothing.
        let unanswered = report.operations.iter().flat_map(|o| &o.threads).any(|t| {
            t.state == crate::facade::ThreadState::Open
                && t.session_state == Some(crate::orchestration::SessionState::Unknown)
        });
        if !report.worker_answers_liveness && unanswered {
            println!(
                "note: this workspace's runtime cannot say whether a session's process is alive, \
                 so no thread above could be called running or hung."
            );
        }
        // **Said once, and only where its absence is felt** (nxf 6j6v.2af2) — the liveness note's
        // shape above, for the other host fact that silently empties a column. A runtime that names
        // no working copy records no anchors, so every row printed its state and none of them could
        // say which tree that state was about; a reader who does not know that reads the silence as
        // "nothing was going on in the repository".
        //
        // **"Felt" means an OPEN thread**, exactly as the liveness note above means one whose
        // session state would have been spoken about. The anchor's own uses are all about work in
        // flight — continue it, see what a reviewer had, rewind to it — so a report in which
        // everything is finished is not a report anybody is missing an anchor from, and a note there
        // would be a warning about a question nobody asked. It is also what keeps a finished
        // operation's rendering byte-identical to what it always was.
        if !report.worker_names_a_working_copy
            && report
                .operations
                .iter()
                .flat_map(|o| &o.threads)
                .any(|t| t.state == crate::facade::ThreadState::Open)
        {
            println!(
                "note: this workspace's runtime does not run its sessions in a directory this \
                 process can see, so no handover above could record where the working copy stood."
            );
        }
    }
    Ok(())
}

/// **The per-operation detail lines that print under an operation's own header line** (nxf
/// 6j6v.s9ex) — ONE place, deliberately, so a sibling fact extends this function instead of a
/// second `println!` finding its own spot in [`status`].
///
/// A withdrawn holder first (nxf 6j6v.b9nf, Integrity #2 of this item's review) — it explains why
/// nothing else here has moved yet, and it is the state a reader has to act on if the sessions it
/// names outlive a reasonable wait. Then a refused park (nxf 6j6v.8bv9) — the state of the copy NOW
/// otherwise — then the operation's unresumed parks, newest first. An operation with none of the
/// three prints nothing here, exactly as it always has; see [`StatusOperation::withdrawn`],
/// [`StatusOperation::parked`] and [`StatusOperation::park_refused`] for the attribution rule.
fn operation_detail_lines(op: &crate::facade::StatusOperation) -> Vec<String> {
    op.withdrawn
        .iter()
        .map(withdrawn_line)
        .chain(op.park_refused.iter().map(park_refused_line))
        .chain(op.parked.iter().map(parked_line))
        .collect()
}

/// One `nxc status` line for a round taken back on purpose, still waiting for its park: who
/// withdrew it, when, and the sessions still pinning the claim — or, once they have all left, that
/// nothing is pinning it any more and the next tick is what moves it on.
fn withdrawn_line(w: &crate::facade::WithdrawnHolderStatus) -> String {
    if w.pinned_by.is_empty() {
        format!(
            "  withdrawn by {} at {} — waiting to park; nothing is pinning it any more, the next \
             tick parks it",
            w.by, w.withdrawn_at
        )
    } else {
        format!(
            "  withdrawn by {} at {} — waiting to park; still pinned by session(s) {}",
            w.by,
            w.withdrawn_at,
            w.pinned_by.join(", ")
        )
    }
}

/// One `nxc status` line for a refused park being retried: which refusal, its own words — which
/// name what to fix — and since when the copy has been held for it.
fn park_refused_line(note: &crate::park::ParkRefusalNote) -> String {
    format!(
        "  park refused ({}): {} — retrying since {}",
        note.refusal, note.detail, note.since
    )
}

/// One `nxc status` line for one parked-work row: the branch, the commit's first seven characters
/// (a git short hash, not truncated for width), and when the working copy was handed on.
///
/// `committed` is `false` only when the park found nothing to commit — a clean tree handed on for
/// its branch alone — and is worth saying because "parked" would otherwise read as "there was
/// uncommitted work" even on the row where there was none.
fn parked_line(p: &crate::park::ParkedWork) -> String {
    let commit = &p.parked.commit[..p.parked.commit.len().min(7)];
    let mut line = format!(
        "  parked on {} at {commit} since {}",
        p.parked.branch, p.parked_at
    );
    if !p.parked.committed {
        line.push_str(", nothing was uncommitted");
    }
    line
}

/// One thread's line in `nxc status`, indented by its depth in the tree.
///
/// **The root's `awaiting you` is the point of this rendering, not decoration** (a71h §3.4 / its DoD
/// 5): an answered root and a thread nobody is working on any more are both "nothing is happening",
/// and only one of them is a problem. The first is the normal end of an operation — the human has
/// it — and it says so; the second is `waiting on <handle>` and stays visible until somebody acts.
///
/// **An ESCALATED thread is checked FIRST, and the word "answered" never appears on it** (nxf
/// 6j6v.mqad). A round that was handed back is discharged, so every other branch here would call it
/// `answered` — which is the exact sentence a human read while a persona session had in fact died at
/// an infrastructure error. The `--json` field is [`StatusThread::escalated`]; this is the same fact
/// in the terminal, said in the one place a terminal reader looks.
///
/// **A SUBSTITUTED thread says who wrote the answer, on every branch** (nxf 6j6v.kffm). It is not a
/// state of its own — a session that ends quietly without answering leaves a thread that is
/// genuinely discharged, and one that died leaves an escalation — it is a fact about the ANSWER, so
/// it rides as a suffix on whichever state applies. Without it "answered" was what a human read
/// about a round in which the agent never spoke, which is the same sentence, about the same
/// message, that `escalated` was added to stop being the whole story.
fn status_line(t: &crate::facade::StatusThread) -> String {
    let indent = "  ".repeat(t.depth + 1);
    let state = if t.escalated {
        let by = match t.expects.join(", ") {
            handles if handles.is_empty() => String::new(),
            handles => format!(" by {handles}"),
        };
        if t.awaiting_human {
            format!("awaiting you — HANDED BACK{by}, not answered")
        } else {
            format!("handed back{by} (escalation)")
        }
    } else if t.awaiting_human {
        format!("awaiting you (answered by {})", t.expects.join(", "))
    } else {
        match t.state {
            crate::facade::ThreadState::Open if t.stale => {
                format!("waiting on {} (overdue)", t.outstanding.join(", "))
            }
            crate::facade::ThreadState::Open => format!("waiting on {}", t.outstanding.join(", ")),
            crate::facade::ThreadState::Answered => "answered".to_string(),
            crate::facade::ThreadState::Orphaned => "ORPHANED (answered, nothing follows)".into(),
        }
    };
    let by_whom = match t.substituted {
        true => " — posted BY THE RUNTIME, not by the agent",
        false => "",
    };
    // **Waiting is not hanging, and until nxf 6j6v.hw2t this line could not say so.** Every other
    // word here is about what is owed; this one is about what the party that owes it is doing, and
    // it is the difference between an operation to act on and one to leave alone. The `--json`
    // field is [`StatusThread::waiting_on_sub_round`]; a suffix rather than a state because it
    // rides ORTHOGONALLY on whichever open branch applies, exactly as `substituted` does — an
    // overdue thread whose assignee is waiting is both, and both are worth reading.
    let own_round = match t.waiting_on_sub_round.len() {
        0 => String::new(),
        n => format!(" — waiting on its own sub-round ({n} open)"),
    };
    // **What the conversation is CALLED, where the reader's eye already is** (nxf 6j6v.e76c) — in
    // place of the channel id, not beside it. On the direct conversations this whole item was cut
    // for, that column is `dm:<24 hex>`: a hash over the two handles that a reader can do nothing
    // with. The id stays first, because it is what the next command is copied from; the channel is
    // still in `--json` for anyone who wants it.
    let what = match &t.name {
        Some(name) => name.clone(),
        None => t.channel_id.clone().unwrap_or_default(),
    };
    format!(
        "{indent}{}  {what}  {state}{by_whom}{own_round}{}{}{}",
        t.thread_id,
        session_suffix(t),
        interruption_suffix(t.interrupted.as_ref()),
        anchor_suffix(t.working_copy.as_ref())
    )
}

/// **The session working here is on hold because the model went away** (nxf 6j6v.npy3) — the
/// terminal half of [`crate::facade::StatusThread::interrupted`], and empty for every thread that is
/// not, which is almost all of them.
///
/// It is the one clause on this line that says WHY nobody is working, as opposed to WHAT is owed,
/// and it is the whole reason the row is worth printing: before this item such a thread rendered as
/// an ordinary `waiting on <handle>` — or, worse, as an escalation the runtime had posted in the
/// agent's name — and a reader had no way at all to tell an operation that had stopped for a week
/// from one that was thinking.
///
/// **Louder than its neighbours, and deliberately**: an availability boundary is the one state on
/// this line that a person may need to act on by choosing NOT to wait — resuming from another
/// account, or deciding the work can stand. The reset instant is spelled out because "on hold" with
/// no when is the sentence that makes somebody check again in five minutes.
fn interruption_suffix(hold: Option<&crate::interruption::Interruption>) -> String {
    match hold {
        None => String::new(),
        Some(h) => {
            let when = match &h.until {
                Some(until) => format!(", back at {until}"),
                // No instant means nothing will take it up on its own, and a reader who is not told
                // that will wait for a service that is not coming.
                None => ", and NOTHING will take it up on its own".to_string(),
            };
            format!(
                " — ON HOLD, the model is not available to it ({}{when})",
                h.limit
            )
        }
    }
}

/// **Where the working copy stood at this thread's last handover** (nxf 6j6v.2af2) — the terminal
/// half of [`crate::facade::StatusThread::working_copy`], and empty for a thread that recorded none,
/// so a workspace whose runtime names no directory reads exactly as it always did.
///
/// It is LAST on the line and deliberately so: every word before it is about who owes what, which is
/// what a reader scanning for something to act on is looking for. This is the provenance behind that
/// answer, and it earns its place by being the thing nothing else could say — `nxc status` could
/// report a round handed back and not which state of the tree it was handed back ON.
///
/// **`dirty` is spelled out rather than implied.** A commit with uncommitted work beside it does NOT
/// describe that tree, and the whole honesty of this record is in saying so where it is read
/// (`crate::anchor` has the argument, and nxf 6j6v.8bv9 owns the securing of that work).
fn anchor_suffix(anchor: Option<&crate::anchor::Anchor>) -> String {
    match anchor {
        None => String::new(),
        Some(a) => {
            let where_ = match a.branch.is_empty() {
                // A detached HEAD is a real state and says so, rather than being dressed up as a
                // branch nobody is on ([`crate::anchor::Anchor::branch`]).
                true => " (detached)".to_string(),
                false => format!(" on {}", a.branch),
            };
            let dirty = match a.dirty {
                true => ", uncommitted work NOT saved",
                false => "",
            };
            format!("  · tree {}{where_}{dirty}", a.short())
        }
    }
}

/// **Does this thread say why its operation is still listed?** (nxf 6j6v.1vxs) — the row filter the
/// two LISTING forms apply, and nothing else applies.
///
/// Five reasons — [`StatusOperation::live`]'s own terms, the root, and the two facts that make a
/// DISCHARGED thread worth a line anyway:
///
/// * the ROOT, always — it is the operation's identity, it carries `awaiting_human`, and an
///   operation rendered without its own first line would be a tree with no top;
/// * anything not [`ThreadState::Answered`] — an open thread is what somebody is waiting for, and
///   an `ORPHANED` one is a dead end that failed, which is a finding by construction;
/// * an ESCALATED thread — discharged, so the line above does not catch it, and it is the single
///   most important row there is: the operation is usually listed *because* of it;
/// * a SUBSTITUTED thread — discharged for the same reason and just as invisible without this
///   line. The runtime wrote that answer standing in for an agent that never spoke (nxf
///   6j6v.kffm), and the whole point of the field is that such a reply must never be mistaken for
///   an ordinary one. Folding it into "N answered thread(s) not shown" is exactly that mistake,
///   made by a filter instead of by a reader — found in the review of PR #444, where this
///   predicate shipped without it. It is ORTHOGONAL to `escalated` in both directions (a session
///   that ended cleanly without answering is substituted and not escalated), so neither line above
///   catches it;
/// * any relation to the working copy — holding it or queued for it.
///
/// What that leaves out is exactly the answered middle of a long chain: threads that were asked,
/// were answered by the agent that owed the answer, and are over. They are in `--json` untouched,
/// and `nxc status --thread <root>` prints the whole tree.
fn says_why(t: &crate::facade::StatusThread) -> bool {
    t.depth == 0
        || t.state != crate::facade::ThreadState::Answered
        || t.escalated
        || t.substituted
        // **A thread whose session is ON HOLD** (nxf 6j6v.npy3), which the line above does not
        // catch in the one case that matters most: the boundary can fall just AFTER the answer
        // lands — measured at 0.6 seconds — leaving a discharged thread with a standing hold. That
        // is the row that explains why its operation is listed at all, and folding it into "N
        // answered thread(s) not shown" would hide exactly the finding.
        || t.interrupted.is_some()
        || t.working_tree.is_some()
}

/// **What became of the session that owes this thread an answer** (nxf 6j6v.qmy6) — the terminal
/// half of [`StatusThread::session_state`], and the distinction the whole field exists for: a
/// thread waiting on an agent whose process is gone is HUNG, and until this it read exactly like
/// one whose agent is still thinking.
///
/// **Only on an OPEN thread, and that is not a shortcut.** A discharged thread's session has
/// finished its job whatever its process is doing, so saying anything there would be history in the
/// place a reader looks for a problem. The `--json` field is carried for every thread that names a
/// session — a tool can want the history; a person reading a screen wants the one line that is
/// about now.
///
/// [`SessionState::Unknown`](crate::orchestration::SessionState::Unknown) says nothing here, and
/// deliberately: it is the answer that carries no information, and a host whose worker cannot ask
/// the process question at all reads it on every thread — which is what
/// [`StatusReport::worker_answers_liveness`] is printed once for, rather than repeated as a word
/// that would look like a finding.
///
/// **And it says nothing about a session that ENDED WHILE WAITING on a round of its own** (nxf
/// 6j6v.hw2t, review of PR #460, Integrity & Robustness #1). "Nothing is coming" is a claim about
/// the FUTURE, and it is false exactly there: the session ended its turn because the engine now
/// tells it to, its sub-round is running, and the coordinator will start it again with the answer.
///
/// Without this branch the fix for that item reintroduced the defect it exists to remove, on the
/// very line it exists to correct — measured on a two-consultation run, which is the ORDINARY shape
/// rather than a corner:
///
/// ```text
/// waiting on local/head-of-marketing — waiting on its own sub-round (2 open) — ITS SESSION HAS ENDED, nothing is coming
/// ```
///
/// Two clauses, contradicting each other, on the row a reader uses to tell a stopped chain from a
/// running one. A waiting turn ALWAYS ends its session (the sidecar announces it on every
/// teardown), so this was not an edge case but what every waiting caller would have rendered. The
/// sub-round clause stands alone; whether that round is itself alive is a question about ITS thread,
/// where this same suffix answers it.
fn session_suffix(t: &crate::facade::StatusThread) -> &'static str {
    if t.state != crate::facade::ThreadState::Open {
        return "";
    }
    // **And it says nothing about a session ON HOLD at an availability boundary** (nxf 6j6v.npy3).
    // "Nothing is coming" is a claim about the FUTURE and it is false here for the second time:
    // the session ended because the model went away, it is armed to be taken up, and what it is
    // waiting for is a window, not a person. This is the same correction nxf 6j6v.hw2t had to make
    // on this exact line for a caller waiting on its own sub-round — the hold's own clause is
    // rendered by `interruption_suffix`, and the two would contradict each other on one row.
    if t.interrupted.is_some() {
        return "";
    }
    match t.session_state {
        Some(crate::orchestration::SessionState::Ended) if t.waiting_on_sub_round.is_empty() => {
            " — ITS SESSION HAS ENDED, nothing is coming"
        }
        Some(crate::orchestration::SessionState::Ended) => "",
        Some(crate::orchestration::SessionState::Running) => " (its session is running)",
        Some(crate::orchestration::SessionState::Unknown) | None => "",
    }
}

/// The trailing `  · holding working tree` / `  · waiting for working tree (#n)` a `threads
/// list`/`show` human line adds when a board has a relation to the working-tree lease (nxf
/// 6j6v.qk5b) — empty for the ordinary case, so an unrelated board's line is exactly what it always
/// was.
fn working_tree_suffix(
    status: Option<crate::facade::WorkingTreeStatus>,
    position: Option<i64>,
) -> String {
    match status {
        Some(crate::facade::WorkingTreeStatus::Holding) => "  · holding working tree".to_string(),
        Some(crate::facade::WorkingTreeStatus::Waiting) => match position {
            Some(p) => format!("  · waiting for working tree (#{p})"),
            None => "  · waiting for working tree".to_string(),
        },
        None => String::new(),
    }
}

/// The `working copy: <sha> on <branch>` line `nxc threads show` prints under a message that
/// recorded an anchor (nxf 6j6v.2af2) — the per-message twin of [`anchor_suffix`], which is the same
/// facts as a suffix on a `nxc status` row.
///
/// Two renderings rather than one shared string, because the two reads answer different questions
/// and have different room: a status row is one line per THREAD and gets a terse suffix, while
/// `show` has a line to itself per message and can afford to name the fact. Both are pinned by
/// [`tests::the_two_anchor_renderings_say_the_same_four_things`] — which is also what the guide
/// quotes, since a golden cannot show an anchor at all (its worker names no working copy).
fn anchor_line(a: &crate::anchor::Anchor) -> String {
    let where_ = match a.branch.is_empty() {
        true => "detached HEAD".to_string(),
        false => format!("on {}", a.branch),
    };
    let dirty = match a.dirty {
        // Said at the point of reading, because a reader's assumption is that a commit describes the
        // tree and here it does not: the work was in the tree and no commit anywhere holds it.
        true => " · uncommitted work NOT saved anywhere",
        false => " · clean",
    };
    format!("working copy: {} {where_}{dirty}", a.short())
}

/// `nxc threads show <thread_id> [--consumer <h>]`: one board in full — the quorum fields plus the
/// reply messages in order. `not_found` for an unknown thread, `forbidden` for a non-member.
fn threads_show(
    json: bool,
    db: Option<&str>,
    thread_id: &str,
    consumer: Option<&str>,
) -> Result<()> {
    let store = open(db)?;
    let consumer = resolve_consumer(consumer, db)?;
    // `thread_board` takes an explicit `ChannelPolicy` (6j6v.xrk3, widened from a bare `Visibility`
    // to visibility PLUS the declared `members:` by 6j6v.v39s). Resolve the thread's REAL declared
    // policy (independent review, Code Quality #1 / Integrity #3, High — this used to hardcode
    // `AllMembers` unconditionally, silently never enforcing a declared `requester_only`): the
    // `decl:<name>` reverse lookup mirrors `route_declared_channel_completion`'s own pattern.
    // `ChannelPolicy::undeclared` remains the honest fallback for a thread whose channel isn't a
    // declared one at all (e.g. a plain `nxc channels create`) — nothing to look up, behavior
    // unchanged for it.
    let channel_id = store
        .thread_channel(thread_id)
        .or_else(|| store.thread_channel_via_message(thread_id));
    let channels = crate::channel::load_all_channels(&declaration_dir(db)?)?;
    let policy = crate::channel::declared_policy(&channels, channel_id.as_deref());
    let board = crate::facade::thread_board(&store, thread_id, &resolve_now()?, &consumer, policy)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&board).expect("board serializes")
        );
    } else {
        let q = &board.quorum;
        println!(
            "thread {} in {}",
            q.thread_id,
            q.channel_id.as_deref().unwrap_or("?")
        );
        // A LINE of its own here rather than in place of the channel (nxf 6j6v.e76c), unlike the two
        // listings: `show` is the one form that has room, and a reader looking at ONE board wants
        // both what it is called and where it lives.
        if let Some(name) = &q.name {
            println!("  name:        {name}");
        }
        println!("  expects:     {}", q.expects.join(", "));
        println!("  replied:     {}", q.replied.join(", "));
        println!("  outstanding: {}", q.outstanding.join(", "));
        println!("  complete:    {}", q.complete);
        // Only when it is so (nxf 6j6v.pzkb), like `name` above: every thread this replica's own
        // writers shaped reads exactly as it always did.
        if q.held {
            println!(
                "  held:        an op that shaped this obligation came from nobody this workspace \
                 trusts — nothing here acts on it (see `nxs sync trust list`)"
            );
        }
        if let Some(d) = &q.deadline {
            println!(
                "  deadline:    {d}{}",
                if q.stale { " (stale)" } else { "" }
            );
        }
        println!(
            "  working tree: {}",
            match board.working_tree {
                Some(crate::facade::WorkingTreeStatus::Holding) => "holding".to_string(),
                Some(crate::facade::WorkingTreeStatus::Waiting) => {
                    match board.working_tree_queue_position {
                        Some(p) => format!("waiting (#{p})"),
                        None => "waiting".to_string(),
                    }
                }
                None => "not needed".to_string(),
            }
        );
        for m in &board.messages {
            // A message nobody here can vouch for says so on its own line (nxf 6j6v.pzkb): the name
            // it shows is only what it claims.
            let unvouched = if m.unvouched {
                " (unvouched — no agent action follows it)"
            } else {
                ""
            };
            println!("  · {} {}{unvouched}", m.sender, m.body);
            // **The anchor belongs under the message it is a fact ABOUT** (nxf 6j6v.2af2), not on a
            // summary line: this is the one read where "what did this reviewer actually have in
            // front of it?" is answerable per message, which is the second of the three uses the
            // item names. A message that recorded none prints nothing, so every board in a workspace
            // that takes no anchors reads exactly as it always did.
            if let Some(a) = &m.refs.working_copy {
                println!("   {}", anchor_line(a));
            }
        }
    }
    Ok(())
}

// `threads_expect` was here — the body of the removed `nxc threads expect` verb (6j6v.dvyq §3). It
// did nothing but parse a comma list and call `facade::set_expects`, which is still `pub` and is now
// reached only from inside the engine (the channel supervisor's re-declaration). Nothing about the
// capability moved; the typed entrance did.

// ---- search ----------------------------------------------------------------

/// `nxc search "<q>" [--consumer <h>]`: body substring in the caller's channels.
fn search(json: bool, db: Option<&str>, query: &str, consumer: Option<&str>) -> Result<()> {
    let store = open(db)?;
    let consumer = resolve_consumer(consumer, db)?;
    // Through `facade::search` rather than straight at the store since nxf 6j6v.px98: the declared
    // `visibility` filter lives there, and a CLI that read past it would be the third answer to the
    // question px98 exists to give ONE answer to. Same catalogue source as `threads show` next door.
    // Since nxf 6j6v.cs03 the catalogue decides the caller's MEMBERSHIP there as well — which is why
    // the qualified handle `resolve_consumer` falls back to now finds a declared channel's boards at
    // all, where it used to match no materialised member row and answer "no matches".
    let channels = crate::channel::load_all_channels(&declaration_dir(db)?)?;
    let hits = crate::facade::search(&store, &consumer, query, &channels)?;
    if json {
        println!("{}", serde_json::to_string(&hits).expect("hits serialize"));
    } else if hits.is_empty() {
        println!("no matches");
    } else {
        for h in &hits {
            println!("{}  {}  {}", h.channel_id, h.sender, h.body);
        }
    }
    Ok(())
}

// ---- session ----------------------------------------------------------------

/// `nxc session bind <internal> <real>`: fill in the real Claude Agent SDK
/// session id for a previously-minted internal session (T4's `bind_session`). A contract another
/// already-committed piece of code depends on (T1's sidecar `main.mjs` shells out to this) — argument
/// order (`internal` then `real`) and the `--json` shape are load-bearing.
fn session_cmd(json: bool, db: Option<&str>, action: &SessionAction) -> Result<()> {
    match action {
        SessionAction::Bind { internal, real } => {
            let mut store = open(db)?;
            store.bind_session(internal, real)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "internal": internal, "bound": true })
                );
            } else {
                println!("bound {internal}");
            }
        }
        // A full orchestration context, unlike its sibling above, and for one reason: the
        // announcement can RELEASE a step of a declared flow, so the same supervisor a `reply`
        // drives has to run here (nxf 6j6v.10yb). Everything it may then do — start the next step,
        // consolidate, wake a requester — is reported on the receipt, because the caller is an
        // exiting sidecar and nobody is reading this process's stderr.
        SessionAction::Ended { internal } => {
            let cli_ctx = CliCtx::resolve(db, hop())?;
            let mut store = open(db)?;
            let r = crate::orchestration::session_ended(&cli_ctx.ctx(), &mut store, internal)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&r).expect("session-ended receipt serializes")
                );
            } else {
                println!("session {} ended: {}", r.session, r.reason);
                for w in &r.warnings {
                    println!("  ! {}", w.detail);
                }
            }
        }
        // A full orchestration context for `Ended`'s reason: this verb arms a deadline, so it needs
        // the timer the context carries (nxf 6j6v.npy3).
        SessionAction::Interrupted {
            internal,
            limit,
            until,
            detail,
        } => {
            let cli_ctx = CliCtx::resolve(db, hop())?;
            let mut store = open(db)?;
            let r = crate::orchestration::session_interrupted(
                &cli_ctx.ctx(),
                &mut store,
                internal,
                limit,
                until.as_deref(),
                detail,
            )?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&r).expect("interruption receipt serializes")
                );
            } else {
                let when = match &r.until {
                    Some(until) => format!(", until {until}"),
                    // Said, not left blank: a reader has to know that nothing will start this on
                    // its own, or the silence reads as "it is handled".
                    None => ", with no reset instant stated — nothing will take it up on its own"
                        .to_string(),
                };
                println!(
                    "session {} is on hold at an availability boundary ({}{when})",
                    r.session, r.limit
                );
                for w in &r.warnings {
                    println!("  ! {}", w.detail);
                }
            }
        }
        // A full orchestration context for `Ended`'s reason and one more: this verb STARTS a
        // session — it is the resume the whole collecting queue exists to produce (nxf 6j6v.gn8b).
        SessionAction::Deliver { internal } => {
            let cli_ctx = CliCtx::resolve(db, hop())?;
            let mut store = open(db)?;
            let r = crate::orchestration::deliver_held(&cli_ctx.ctx(), &mut store, internal)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&r).expect("delivery receipt serializes")
                );
            } else if r.delivered == 0 {
                println!("nothing was held for session {}", r.session);
            } else {
                println!(
                    "delivered {} message(s) to session {}{}",
                    r.delivered,
                    r.session,
                    match r.woke.is_some() {
                        true => "",
                        // The queue is untouched in this case, which is the fact an operator needs:
                        // nothing was lost, and the next settle carries it.
                        false => " — the resume did not land, so they are still held",
                    }
                );
                if !r.outstanding.is_empty() {
                    println!("  still outstanding: {}", r.outstanding.join(", "));
                }
                for w in &r.warnings {
                    println!("  ! {}", w.detail);
                }
            }
        }
        // The worker is ASKED a question and never told to do anything — which is why this reads
        // the store directly through `CliCtx`'s worker instead of taking an orchestration context
        // it has no use for. `CliCtx::resolve` is what puts that worker at the WORKSPACE ROOT (nxf
        // 6j6v.npn0), and that matters more here than almost anywhere: a `state` answered from the
        // wrong directory would report every live session as `unknown`, which is precisely the
        // wrong answer to the question this verb exists for.
        SessionAction::State { internal, thread } => {
            let cli_ctx = CliCtx::resolve(db, hop())?;
            let scope =
                match (internal, thread) {
                    (_, Some(thread)) => crate::orchestration::SessionScope::Thread(thread),
                    (Some(internal), None) => crate::orchestration::SessionScope::Session(internal),
                    // `clap`'s `required_unless_present` has already refused this; the arm is here
                    // because the types cannot know that, and a `panic!` for a state the parser
                    // guarantees away would be a worse answer than the sentence itself.
                    (None, None) => return Err(NxfError::validation(
                        "name a session, or pass `--thread <id>` to ask about a thread's sessions",
                    )),
                };
            let report = crate::orchestration::session_state(&open(db)?, &cli_ctx.worker, scope)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&report).expect("session-state report serializes")
                );
            } else if report.sessions.is_empty() {
                println!("no session has run on this thread");
            } else {
                for s in &report.sessions {
                    let state = match s.state {
                        crate::orchestration::SessionState::Running => "running".to_string(),
                        crate::orchestration::SessionState::Ended => match &s.ended {
                            Some(at) => format!("ended {at}"),
                            None => "ended".to_string(),
                        },
                        // The two `unknown`s, told apart in words (nxf 6j6v.t41e): the report
                        // now carries whether the worker could be asked at all, and an operator
                        // reading "no live process answers for it" about a worker that never
                        // looked is being told something about the session that nobody
                        // established.
                        crate::orchestration::SessionState::Unknown
                            if !report.worker_answers_liveness =>
                        {
                            "unknown — this worker cannot answer whether a session is running, so \
                             nothing was asked"
                                .to_string()
                        }
                        crate::orchestration::SessionState::Unknown => {
                            "unknown — nothing announced an end and no live process answers for it"
                                .to_string()
                        }
                    };
                    // **The hold, on the same line as the state and not instead of it** (nxf
                    // 6j6v.npy3). An interrupted session HAS ended and `ended <at>` is true of it;
                    // what this adds is the half that is not in the word "ended" — that the model
                    // went away rather than the work finishing, and when it is back. Without it a
                    // reader of this verb sees the same clean ending that made the interruption
                    // unfindable in the first place.
                    let hold = match &s.interrupted {
                        None => String::new(),
                        Some(h) => {
                            let when = match &h.until {
                                Some(until) => format!(", back at {until}"),
                                None => ", and NOTHING will take it up on its own".to_string(),
                            };
                            format!(
                                "  · ON HOLD since {} — the model is not available to it ({}{when}): {}",
                                h.at, h.limit, h.detail
                            )
                        }
                    };
                    println!("{}  {}  {}{hold}", s.session, s.role, state);
                }
            }
        }
    }
    Ok(())
}

// ---- transcript --------------------------------------------------------------

/// The `nxc transcript` verbs (nxf epic 6wt2) — the write half and the read half of one session's
/// agent transcript:
///
/// - `append --session <internal>` is the SIDECAR'S callback: JSON-lines in on stdin,
///   one [`TranscriptEntry`] per line, handed to [`crate::facade::transcript_append`] as ONE batch
///   (see [`TranscriptAction::Append`] for why the flag name and the framing are load-bearing).
///   What stays HERE is the stdin framing and the per-line error coordinate; the write itself is on
///   the seam since nxf 6j6v.c6e8, so a host driving its own role runtime needs no process;
/// - `show <internal>` is the operator's/agent's read: [`crate::facade::transcript_page`]'s
///   nested [`TranscriptView`](crate::facade::TranscriptView), rendered as an indented timeline or
///   printed verbatim under `--json` (see [`print_transcript`]). Read-only, whole session by
///   default; `--from-seq`/`--limit` window it (nxf 6j6v.t7pa);
/// - `prune` is the operator's half of retention: [`crate::facade::prune_transcripts`], the lever
///   this table had no version of until 6j6v.t7pa. The automatic half needs no verb — it rides the
///   first flush of every new session, inside `append_transcript`.
///
/// `append` reads whole-stdin-then-parses rather than streaming line by line: the batch is atomic in the store, so
/// a malformed line at the end must reject the lines before it too. NOT because a retry would
/// otherwise double them — the sidecar does not retry, it clears its buffer before the write
/// (`main.mjs:69-77`) precisely so a failure cannot double-write — but because a stored prefix of a
/// flush that failed is indistinguishable on read from a flush that succeeded, while the rest is
/// gone with no marker. The sidecar flushes every `FLUSH_THRESHOLD` entries, so this buffer stays
/// small.
/// Backstop for one `nxc transcript append` batch (PR #258 review, Integrity #1). Deliberately
/// generous rather than tight: the sidecar flushes every 32 entries and `tool_use`'s `data.input`
/// is uncapped by design (a `Write` of a large file lands whole), so a legitimate batch can be
/// megabytes. 64 MiB is far above any real agent turn and far below what threatens the process —
/// it exists to turn an OOM into a diagnosable error, not to enforce a size policy.
const MAX_TRANSCRIPT_BATCH_BYTES: u64 = 64 * 1024 * 1024;

fn transcript_cmd(json: bool, db: Option<&str>, action: &TranscriptAction) -> Result<()> {
    let mut store = open(db)?;
    match action {
        TranscriptAction::Append { session } => {
            // Bounded read (PR #258 review, Integrity #1). The producer caps thinking at 4000 and
            // tool_result at 8000 chars, and `tool_use`'s `data.input` is deliberately uncapped —
            // but ALL of that is the SIDECAR's discipline, on the far side of a process boundary.
            // Trusting it with an unbounded `read_to_string` means one pathological line OOM-kills
            // `nxc`, and because the sidecar's mid-run flush propagates, that aborts the whole role
            // turn with nothing diagnosable in the log. This is the backstop, NOT a policy: a cap
            // that ever bites belongs in `transcript.mjs` next to the other two (it sees the value
            // pre-serialization and saves the pipe cost). Read one byte past the limit so an
            // over-long batch is a loud `validation` error naming the limit, never a silent
            // truncation that would store a torn prefix as if it were the whole batch.
            // Read BYTES, not a String: `take` cuts at the byte limit, which can land mid-codepoint
            // on an over-long batch, and `read_to_string` would then fail with an opaque "stream did
            // not contain valid UTF-8" instead of the size error that is actually the point. Check
            // the length first, decode second, so the over-limit case always reports as over-limit.
            let mut buf = Vec::new();
            let mut bounded = std::io::Read::take(std::io::stdin(), MAX_TRANSCRIPT_BATCH_BYTES + 1);
            std::io::Read::read_to_end(&mut bounded, &mut buf)
                .map_err(|e| NxfError::io(format!("reading transcript entries from stdin: {e}")))?;
            if buf.len() as u64 > MAX_TRANSCRIPT_BATCH_BYTES {
                return Err(NxfError::validation(format!(
                    "transcript batch exceeds {MAX_TRANSCRIPT_BATCH_BYTES} bytes — refusing to \
                     read further; cap the payload in agent-sidecar/src/transcript.mjs (see \
                     MAX_THINKING_CHARS / MAX_TOOL_RESULT_CHARS) rather than raising this backstop"
                )));
            }
            let raw = String::from_utf8(buf).map_err(|e| {
                NxfError::validation(format!("transcript batch is not valid UTF-8: {e}"))
            })?;
            let mut entries = Vec::new();
            for (i, line) in raw.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let entry: TranscriptEntry = serde_json::from_str(line)
                    .map_err(|e| NxfError::validation(format!("transcript line {}: {e}", i + 1)))?;
                // `append_transcript` runs this same check (it is the invariant of the TABLE, and an
                // embedder reaching it through `facade::transcript_append` must not get past it) —
                // but it can only report a batch INDEX, and blank framing lines make those diverge
                // from line numbers. Calling the SHARED `validate` here, rather than
                // re-implementing the predicate, is what keeps a future second invariant reported
                // with a line number on this path too.
                entry.validate().map_err(|e| {
                    NxfError::validation(format!("transcript line {}: {}", i + 1, e.msg))
                })?;
                entries.push(entry);
            }
            let appended = crate::facade::transcript_append(&mut store, session, &entries)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "session": session, "appended": appended })
                );
            } else {
                println!("appended {appended} entries to {session}");
            }
        }
        TranscriptAction::Show {
            session,
            from_seq,
            limit,
        } => {
            // `-1` is "from the start" — `seq` starts at 0, so "after -1" is "all", the same
            // sentinel `transcript_rows` itself uses.
            let view =
                crate::facade::transcript_page(&store, session, from_seq.unwrap_or(-1), *limit)?;
            if json {
                println!("{}", view.to_value());
            } else {
                print_transcript(&view);
            }
        }
        TranscriptAction::Prune { keep_days, dry_run } => {
            // The flag wins over the environment; with neither, the configured default. `off` is a
            // legitimate answer for the AUTOMATIC pass and a dead end for this one — an operator who
            // typed `prune` has asked for a window, so say what is missing instead of silently
            // doing nothing.
            let keep_days = match keep_days {
                Some(n) => *n,
                None => crate::transcript::configured_keep_days()?.ok_or_else(|| {
                    NxfError::validation(format!(
                        "{} is `off` — pass --keep-days to prune anyway",
                        crate::transcript::KEEP_DAYS_ENV
                    ))
                })?,
            };
            let report =
                crate::facade::prune_transcripts(&mut store, &resolve_now()?, keep_days, *dry_run)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_value(&report)
                        .map_err(|e| { NxfError::io(format!("serializing prune report: {e}")) })?
                );
            } else {
                print_transcript_prune(&report, keep_days);
            }
        }
    }
    Ok(())
}

/// The human `nxc transcript prune`: what went (or would go), then the sessions retention could not
/// date. Both counts are named even when zero — "pruned 0 transcripts" is the answer an operator
/// wanting to know whether anything was stale is asking for, and silence would read as a failure.
fn print_transcript_prune(report: &crate::transcript::TranscriptPruneReport, keep_days: i64) {
    let verb = if report.dry_run {
        "would prune"
    } else {
        "pruned"
    };
    println!(
        "{verb} {} transcript(s), {} entries (keeping {keep_days} days)",
        report.sessions.len(),
        report.entries
    );
    for s in &report.sessions {
        println!("  {s}");
    }
    if !report.undated.is_empty() {
        println!(
            "kept {} session(s) with no readable timestamp:",
            report.undated.len()
        );
        for s in &report.undated {
            println!("  {s}");
        }
    }
}

/// Max characters of an entry's `data` rendered into the HUMAN timeline. A `tool_use`'s
/// `data.input` is unbounded by design (the store keeps every byte — see
/// [`ChatStore::append_transcript`]'s doc comment), so one `Write` of a large file would otherwise
/// scroll the whole session off the screen. This bounds the RENDERING only; `--json` stays whole.
const TRANSCRIPT_GIST_CHARS: usize = 120;

/// The human `nxc transcript show`: a header line naming the session (plus its role / real SDK id
/// when `session_map` knows them and the total entry count including nested ones), then one line
/// per entry with each subagent's sub-timeline indented under the `tool_use` that spawned it.
fn print_transcript(view: &crate::facade::TranscriptView) {
    let mut head = format!("transcript {}", view.session);
    if let Some(role) = &view.role {
        head.push_str(&format!("  role={role}"));
    }
    if let Some(real) = &view.real_sdk_id {
        head.push_str(&format!("  real={real}"));
    }
    let total: usize = view.entries.iter().map(|e| 1 + e.subagent.len()).sum();
    println!("{head}  ({total} entries)");
    for e in &view.entries {
        print_transcript_entry(e, 0);
        // One level deep by contract (`facade::transcript`), so a plain nested loop is the whole
        // tree — no recursion to write and none to read.
        for sub in &e.subagent {
            print_transcript_entry(sub, 1);
        }
    }
}

/// One timeline line: `#<seq> <at> <kind> [<subagent_type>] <gist of data>`, indented `depth`
/// levels. The gist is the entry's opaque `data` as compact JSON, truncated on a char boundary —
/// deliberately shape-agnostic: a per-`kind` renderer would re-learn the sidecar's payload shapes in
/// a third place, and JSON escaping already guarantees the one-line-per-entry property (a newline
/// inside a payload string renders as `\n`).
fn print_transcript_entry(e: &crate::facade::TranscriptEntryView, depth: usize) {
    let pad = "  ".repeat(depth);
    let at = e.at.as_deref().unwrap_or("-");
    let subagent = match &e.subagent_type {
        Some(t) => format!(" [{t}]"),
        None => String::new(),
    };
    let compact = e.data.to_string();
    let gist = if compact.chars().count() > TRANSCRIPT_GIST_CHARS {
        format!(
            "{}…",
            compact
                .chars()
                .take(TRANSCRIPT_GIST_CHARS)
                .collect::<String>()
        )
    } else {
        compact
    };
    println!("{pad}#{} {at} {}{subagent} {gist}", e.seq, e.kind);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An anchor as the engine records one — full sha, a real branch, and a fingerprint.
    fn anchored(branch: &str, dirty: bool) -> crate::anchor::Anchor {
        crate::anchor::Anchor {
            commit: "9f2c1ab4e7d0c5b3a1f8e6d4c2b0a9f8e7d6c5b4".to_string(),
            branch: branch.to_string(),
            dirty,
            fingerprint: "f".repeat(64),
        }
    }

    #[test]
    fn the_two_anchor_renderings_say_the_same_four_things() {
        // **The guide quotes these two strings**, and a golden cannot: the golden corpus runs with a
        // worker that names no working copy, so no `nxc` invocation in it can produce an anchor at
        // all. This test is what stands in for "example = test" there, which is why it pins the
        // bytes rather than a substring.
        assert_eq!(
            anchor_suffix(Some(&anchored("feat/parser", true))),
            "  · tree 9f2c1ab4e7d0 on feat/parser, uncommitted work NOT saved"
        );
        assert_eq!(
            anchor_line(&anchored("feat/parser", true)),
            "working copy: 9f2c1ab4e7d0 on feat/parser · uncommitted work NOT saved anywhere"
        );
        // A clean tree: the status suffix stays silent about it (there is nothing to warn of) while
        // `show` says `clean`, because a line that exists to describe the tree must not leave the
        // most important half of it to be inferred from an absence.
        assert_eq!(
            anchor_suffix(Some(&anchored("main", false))),
            "  · tree 9f2c1ab4e7d0 on main"
        );
        assert_eq!(
            anchor_line(&anchored("main", false)),
            "working copy: 9f2c1ab4e7d0 on main · clean"
        );
    }

    #[test]
    fn a_thread_with_no_anchor_adds_nothing_at_all_to_its_status_line() {
        // Every workspace whose runtime names no working copy, and every message written before this
        // existed: the row has to be byte-identical to what it always was, or a feature nobody asked
        // for changes every line of a read they depend on.
        assert_eq!(anchor_suffix(None), "");
    }

    #[test]
    fn a_detached_head_is_rendered_as_the_state_it_is_and_not_as_a_branch() {
        // `Anchor::branch` is empty for a detached HEAD, and an empty branch name printed after
        // "on " would read as a rendering bug — or worse, as a branch.
        assert_eq!(
            anchor_suffix(Some(&anchored("", false))),
            "  · tree 9f2c1ab4e7d0 (detached)"
        );
        assert_eq!(
            anchor_line(&anchored("", true)),
            "working copy: 9f2c1ab4e7d0 detached HEAD · uncommitted work NOT saved anywhere"
        );
    }

    /// **The `nxc status` line for a withdrawn holder names who withdrew it, when, and who still
    /// pins it** (nxf 6j6v.b9nf, Integrity #2 of this item's review) — the human half of
    /// [`crate::facade::StatusOperation::withdrawn`], pinned directly the way
    /// [`the_two_anchor_renderings_say_the_same_four_things`] pins its sibling: a golden corpus runs
    /// with the dry worker, which never produces a marker at all, so no golden line stands in for
    /// this one.
    #[test]
    fn the_withdrawn_line_names_who_withdrew_it_and_who_still_pins_it() {
        let pinned = crate::facade::WithdrawnHolderStatus {
            withdrawn_at: "2026-09-19T09:10:00Z".to_string(),
            by: "local/carsten".to_string(),
            pinned_by: vec!["s-1".to_string()],
        };
        assert_eq!(
            withdrawn_line(&pinned),
            "  withdrawn by local/carsten at 2026-09-19T09:10:00Z — waiting to park; still \
             pinned by session(s) s-1"
        );

        let clear = crate::facade::WithdrawnHolderStatus {
            withdrawn_at: "2026-09-19T09:10:00Z".to_string(),
            by: "local/carsten".to_string(),
            pinned_by: Vec::new(),
        };
        assert_eq!(
            withdrawn_line(&clear),
            "  withdrawn by local/carsten at 2026-09-19T09:10:00Z — waiting to park; nothing is \
             pinning it any more, the next tick parks it"
        );

        // Two sessions still pinning it, both named.
        let both = crate::facade::WithdrawnHolderStatus {
            withdrawn_at: "2026-09-19T09:10:00Z".to_string(),
            by: "local/carsten".to_string(),
            pinned_by: vec!["s-1".to_string(), "s-2".to_string()],
        };
        assert!(withdrawn_line(&both).ends_with("session(s) s-1, s-2"));
    }

    #[test]
    fn parse_refs_builds_scalars_and_repeated_nxf_ids() {
        let r = parse_refs(&[
            "session_id=s1".into(),
            "nxf_ids=6j6v.a".into(),
            "nxf_ids=6j6v.b".into(),
        ])
        .unwrap();
        assert_eq!(r.session_id.as_deref(), Some("s1"));
        assert_eq!(r.nxf_ids, vec!["6j6v.a", "6j6v.b"]);
    }

    #[test]
    fn parse_refs_rejects_an_unknown_key() {
        assert!(parse_refs(&["mystery=x".into()]).is_err());
    }

    /// A `stream_options_from` lookup over a fixed pair — no process env, so these run in parallel
    /// with everything else in this binary.
    fn env<'a>(
        idle: Option<&'a str>,
        poll: Option<&'a str>,
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| match key {
            "NXC_STREAM_IDLE" => idle.map(str::to_string),
            "NXC_STREAM_POLL" => poll.map(str::to_string),
            other => panic!("unexpected lookup of {other}"),
        }
    }

    #[test]
    fn stream_timings_read_seconds_and_keep_the_knock_inside_the_window() {
        let opts = stream_options_from(env(Some("60"), Some("0.5"))).unwrap();
        assert_eq!(opts.idle, std::time::Duration::from_secs(60));
        assert_eq!(opts.poll, std::time::Duration::from_millis(500));
        assert_eq!(
            opts.knock_lead,
            std::time::Duration::from_secs(10),
            "the knock fits inside the window it warns about"
        );

        let unset = stream_options_from(env(None, Some(""))).unwrap();
        assert_eq!(
            unset,
            crate::stream::StreamOptions::default(),
            "unset and empty both mean the default, not zero"
        );
    }

    #[test]
    fn a_stream_timing_that_is_not_a_duration_is_a_validation_error_not_a_panic() {
        // The bug this test exists for: `"-1"` and `"nan"` PARSE as f64 and then panicked inside
        // `Duration::from_secs_f64`, aborting the process (exit 101) where the doc promises a
        // validation error. Every rejection is checked on BOTH variables, since they share one
        // closure and a fix applied to one only would read as green here.
        for bad in ["-1", "-0.5", "nan", "inf", "-inf", "1e400", "soon", "5s"] {
            for (idle, poll) in [(Some(bad), None), (None, Some(bad))] {
                let which = if idle.is_some() { "IDLE" } else { "POLL" };
                match stream_options_from(env(idle, poll)) {
                    Err(e) => assert_eq!(
                        e.kind,
                        crate::error::ErrorKind::Validation,
                        "{which}={bad:?} is a bad input, not a broken workspace"
                    ),
                    Ok(opts) => panic!("{which}={bad:?} was accepted as {opts:?}"),
                }
            }
        }
    }

    #[test]
    fn a_poll_interval_below_the_floor_is_raised_to_it_rather_than_busy_looping() {
        // `NXC_STREAM_POLL=0` turns the loop's only sleep into a no-op. Clamped, not rejected —
        // see `MIN_STREAM_POLL` for why the line is drawn at a floor and not at zero.
        for spun in ["0", "0.0", "0.001"] {
            let opts = stream_options_from(env(None, Some(spun))).unwrap();
            assert_eq!(opts.poll, MIN_STREAM_POLL, "{spun} raised to the floor");
        }
        assert_eq!(
            stream_options_from(env(None, Some("0.2"))).unwrap().poll,
            std::time::Duration::from_millis(200),
            "a value above the floor is left exactly as asked"
        );
    }

    /// **The park classes are reports, not failures** (fix round 1 of nxf 6j6v.8bv9). Both classes
    /// document in as many words that they do not change a verb's exit code, and since this item
    /// made `WorkHandedOnUnparked` reachable from the most ORDINARY drain — a workspace that is not
    /// a git repository refuses every park permanently — the rule and the code had to be made to
    /// agree before `nxc tick` started exiting 1 on every routine queue hand-off.
    #[test]
    fn a_park_that_was_refused_is_reported_without_failing_the_verb() {
        use crate::orchestration::{ConsequenceClass, FailedConsequence};
        let held = FailedConsequence::work_not_parked(Some("t1"), "a merge is in progress");
        let unparked =
            FailedConsequence::work_handed_on_unparked(Some("t1"), "this is not a git repository");
        assert_eq!(held.class, ConsequenceClass::WorkNotParked);
        assert_eq!(unparked.class, ConsequenceClass::WorkHandedOnUnparked);
        assert!(
            !changes_the_exit_code(std::slice::from_ref(&held)),
            "the claim stayed and the retry is armed — the verb did what it was asked"
        );
        assert!(
            !changes_the_exit_code(std::slice::from_ref(&unparked)),
            "the copy went on and the finding is the whole of what went differently"
        );
        assert!(!changes_the_exit_code(&[held.clone(), unparked.clone()]));
        // …and the rule the exception is carved out of still stands: a class that DOES mean "a step
        // of this call did not happen" is still red, beside either of them.
        assert!(changes_the_exit_code(&[
            held,
            unparked,
            FailedConsequence::step_skipped(
                Some("t1"),
                Some("s1"),
                crate::orchestration::WakeSkipReason::Unresolved,
                "nobody was put to work",
            ),
        ]));
    }

    /// A sweep as `tick` reports one: `started` empty means the compare-and-swap behind the park
    /// matched nothing, so the lease never moved.
    fn a_sweep(started: bool, parked: bool) -> crate::orchestration::WorkingTreeSweep {
        crate::orchestration::WorkingTreeSweep {
            scope: "thread:t1".to_string(),
            expired: "2026-09-08T11:00:00Z".to_string(),
            parked: parked.then(|| crate::park::Parked {
                branch: "nxs/park/t1".to_string(),
                commit: "abc1234".to_string(),
                base_branch: "release/1.2".to_string(),
                base_commit: "def5678".to_string(),
                created_branch: true,
                committed: true,
            }),
            unparked: None,
            promotions: crate::orchestration::Promotions {
                started: started
                    .then(|| crate::orchestration::PromotedTrigger {
                        thread: "t2".to_string(),
                        role: "solo".to_string(),
                        session: "s2".to_string(),
                    })
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
        }
    }

    /// **A park is not a reclaim** (fix round 1 of nxf 6j6v.8bv9). Since the sweep parks BEFORE the
    /// compare-and-swap, it can report a `WorkingTreeSweep` whose promotions are empty — the holder
    /// renewed while its work was being saved, so the copy never moved. The line that opens the
    /// block used to say "reclaimed this workspace's working copy from …" over exactly that.
    #[test]
    fn a_sweep_that_saved_the_work_but_moved_nothing_does_not_claim_a_reclaim() {
        assert_eq!(
            sweep_headline(&a_sweep(true, true)),
            "reclaimed this workspace's working copy from thread:t1 (its lease ran out at \
             2026-09-08T11:00:00Z)"
        );
        let stayed = sweep_headline(&a_sweep(false, true));
        assert!(
            !stayed.contains("reclaimed"),
            "nothing was reclaimed: {stayed}"
        );
        assert!(
            stayed.contains("thread:t1") && stayed.contains("2026-09-08T11:00:00Z"),
            "it still names the operation and the bound it ran past: {stayed}"
        );
        assert!(
            stayed.contains("still holds") || stayed.contains("keeps"),
            "…and says the copy stayed with it: {stayed}"
        );
    }

    #[test]
    fn command_tree_parses() {
        // A cheap smoke test that the clap tree is well-formed (no duplicate args / bad defaults).
        command().debug_assert();
    }

    // The `render_unread_blocks` unit tests moved with that function into `facade.rs` (nxf
    // 6j6v.r5a2) — the CLI no longer formats anything itself — and were then DELETED with the
    // renderer (nxf 6j6v.4d2z). `render_opener_wake`'s made the same move and went the same way one
    // ticket earlier (nxf 6j6v.1gm9); what stands in their place is a golden,
    // `tests/prime_golden.rs`, asserting that a completed board adds not one byte to the session
    // start.
}

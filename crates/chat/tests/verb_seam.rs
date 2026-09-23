//! nxc's half of the verb-seam omission gate (nexus-flow-6j6v.vtvs).
//!
//! `crates/chat/tests/parity.rs` next door drives the same script through the CLI and the
//! [`Engine`](nexus_chat::engine::Engine) and compares op logs and read results byte for byte. It
//! is blind to a verb that exists on only one side — which is how `nxc prime` came to read the
//! store directly, split its own catch-up list and decide `is_spawned_context` inside `cli.rs`,
//! all with that suite green. This gate compares the two verb LISTS instead; both are derived (the
//! `clap` tree, the seam sources), and only the difference is declared below.
//!
//! chat carries the repo's only [`Waiver::KnownGap`] entry, and it is down to one: `tick`, whose
//! seam twin was removed on purpose. The other was the transcript WRITE, which acted straight on
//! the store from the CLI until nxf 6j6v.c6e8 lifted it onto the facade — the one KnownGap here
//! that was a real omission rather than a decision, and it was paid off rather than reworded. Each
//! entry carries its reason; the gate stops accepting a waiver the moment its verb disappears.
//!
//! See [`nxs_test_support::verb_seam`] for what each waiver kind claims and how the gate holds it
//! to that claim.

use nxs_test_support::verb_seam::{assert_verb_seam, crate_relative, Waiver};

/// nxc's declared difference between the two verb lists. Every entry carries its reason: an
/// exception without one is a finding, not a free pass.
const WAIVERS: &[Waiver] = &[
    // ---- the command line's own business -----------------------------------
    Waiver::CliOnly {
        verb: "init",
        reason: "Sets up the HOST, not the workspace: ensures `.nxs/`, registers the chat module \
                 and delegates the shared agent file + the SessionStart hook to the `nxs` \
                 assembler. An embedding app is installed by its own host; it does not bootstrap a \
                 checkout. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "agent-manifest",
        reason: "Emits chat's DECLARED contribution to the shared agent file (spec §6.2) — the \
                 data the `nxs` umbrella assembles from. It is about the files on disk around the \
                 workspace, not about messages, and its only consumer is the umbrella running the \
                 binary. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "guide",
        reason: "Renders the CLI's own manual — markdown baked into the binary via `include_dir!` \
                 (`crates/chat/docs/guide/en`), written about typing `nxc …`. An embedding app has \
                 no `nxc` to explain and ships its own documentation; there is no workspace \
                 capability behind this verb to lift. Same shape as flow's, and the same shared \
                 mechanism (`nxs-guide`, 6j6v.9e3r).",
    },
    // ---- on the seam, under a name of its own -------------------------------
    // `read` -> `mark_read` was the first entry here. DROPPED by nxf 6j6v.1gm9 with the verb: this
    // gate asks only whether every verb an agent types is reachable from the seam, so a verb that
    // no longer exists has nothing to waive and the gate refuses a stale waiver outright. The WRITE
    // outlived the waiver by a fortnight (`facade::mark_read` → `Engine::mark_read`) and went with
    // the whole unread apparatus in nxf 6j6v.4d2z. Both moves are recorded next door in
    // `seam_disposition.rs`, which is the gate for the opposite direction — what becomes of the
    // seam when an entrance goes.
    Waiver::Alias {
        verb: "threads list",
        seam: "threads",
        reason: "`facade::threads` is the verb body's one call, named for the collection rather \
                 than the verb path. It reached `Engine::threads` too until nxf 6j6v.yr59 folded \
                 that method into `Engine::status`; the COMPUTE half is what this gate asks for, \
                 and it is untouched.",
    },
    Waiver::Alias {
        verb: "threads show",
        seam: "thread_board",
        reason: "`facade::thread_board`: the seam names the record — a quorum board with its \
                 replies — and the CLI names the act of looking at it. Its handle method went with \
                 `threads` in nxf 6j6v.yr59 (quorum half into `Engine::status`, message half into \
                 `Engine::thread`, which took the `visibility` policy with it); the compute layer \
                 this gate reads is unchanged.",
    },
    Waiver::Alias {
        verb: "threads name",
        seam: "name_thread",
        reason: "`facade::name_thread` / `Engine::name_thread` (nxf 6j6v.e76c): the seam names the \
                 ACT on a thread, which is how every other write on it reads (`set_expects`, \
                 `reply_thread`), while the CLI groups it under the collection the way \
                 `threads list`/`show` are grouped. Same shape and same reason as those two \
                 aliases; the compute half this gate asks for is `pub` and tested at both seams.",
    },
    // `threads expect` had an `Alias` waiver here until 6j6v.cg8g/6j6v.dvyq removed the VERB. The
    // waiver had to go with it — this gate rejects a waiver for a verb that no longer exists, which
    // is exactly the property that keeps this table honest. `facade::set_expects` is untouched and
    // still `pub`; it simply no longer has a CLI path to be waived FOR.
    //
    // `release` -> `release_working_tree` had the same `Alias` waiver here until nxf 6j6v.b9nf
    // removed BOTH the verb and the seam method it named (owner decision, 2026-09-17: `release`
    // leaves the user surface). Same rule as `threads expect` above: a waiver for a verb that no
    // longer exists is a stale claim, not a decision this gate can go on holding true, so it goes
    // with the verb rather than standing as dead prose. `seam_disposition.rs` carries the removal's
    // own row, with the owner's safety reason and both dates.
    Waiver::Alias {
        verb: "session bind",
        seam: "bind_runtime_session",
        reason: "`Engine::bind_runtime_session` makes the identical `store.bind_session` call. It \
                 exists precisely BECAUSE the CLI route does not reach far enough: a local session \
                 can shell out to `nxc`, a remote runtime cannot, so the handle grew the verb for \
                 the host that drives its own worker.",
    },
    Waiver::Alias {
        verb: "session deliver",
        seam: "deliver_held",
        reason: "`Engine::deliver_held` (nxf 6j6v.gn8b) IS this verb's body — one call, the same \
                 `DeliveryReceipt` `--json` prints. The names differ because the two seams name \
                 different things: the CLI files it under `session` beside `bind`/`ended`/`state`, \
                 which is where somebody looking for what one may do TO a session finds it, while \
                 `session_deliver` would be a poor method name on a handle where the object is the \
                 held QUEUE and not the session. A host that runs its own sessions knows when one \
                 settles, which is precisely why this exists on the handle at all: on the CLI the \
                 background service arms it, and a host has no such arming to ride on.",
    },
    Waiver::Alias {
        verb: "resume",
        seam: "resume_interrupted",
        reason: "`Engine::resume_interrupted` (nxf 6j6v.npy3) IS this verb's body — one call, the \
                 same `ResumeReceipt` `--json` prints, including every check it made before \
                 deciding not to start anything. The names differ because the two seams address \
                 different readers: a person types the short word at a terminal, where the object \
                 is obvious from `nxc status`; on a handle that also resumes a ROLE \
                 (`role_resume`) and hands a settled caller its held answers, a bare `resume` \
                 would not say which of the three it is. The handle needs it for the reason \
                 `session bind` gives: the background service shells out to `nxc`, and a host that \
                 drives its own runtime has no such shell to ride on.",
    },
    Waiver::Alias {
        verb: "list",
        seam: "directory",
        reason: "`Engine::directory` (nxf 6j6v.p6m1) returns the same `persona::Directory` the \
                 verb prints — `--json` IS that value. The seam names the record (who can be \
                 addressed, and what for) where the CLI keeps the short word an agent types, and \
                 `list` alone would be a poor method name on a handle that also lists channels \
                 and threads.",
    },
    Waiver::Alias {
        verb: "transcript show",
        seam: "transcript",
        reason: "`facade::transcript` returns the same `TranscriptView` the verb renders as a \
                 timeline; `--json` IS that value verbatim. The verb's `--from-seq`/`--limit` \
                 window is `facade::transcript_page` (→ `Engine::transcript_page`), which \
                 `transcript` itself delegates to — which is why nxf 6j6v.yr59 removed \
                 `Engine::transcript` as the pure overload it was, leaving the windowed read as \
                 the handle's one transcript verb. The unwindowed BODY stays, and is what this \
                 waiver names.",
    },
    Waiver::Alias {
        verb: "transcript prune",
        seam: "prune_transcripts",
        reason: "`facade::prune_transcripts` (→ `Engine::prune_transcripts`) is the verb body's one \
                 call and returns the `TranscriptPruneReport` `--json` prints verbatim. The CLI \
                 keeps the short word; the seam names what it acts on, since `prune` alone would be \
                 a poor method name on a handle that will one day prune more than transcripts.",
    },
    // ---- real gaps, filed rather than absorbed ------------------------------
    //
    // The three `agents` entries (6j6v.1xdd) were here and are gone with their VERBS, not with
    // their debt: 6j6v.dvyq §3 retires `agents list`/`register`/`search` because a team is
    // DECLARED, so there was never anything to lift onto the seam. 6j6v.1xdd was closed on
    // 2026-08-13 for exactly that reason. `seam_disposition.rs` carries the decision now.
    // Six `channels` waivers stood here — the two `Alias` rows for `list`/`public` (whose seam
    // reads are named for what they return rather than for the path typed to reach them) and the
    // four `KnownGap` rows for `create`/`dm`/`join`/`leave` (store-direct in the CLI, 6j6v.hcq1).
    // The whole verb group went with 6j6v.dvyq §3, and this gate REJECTS a waiver for a verb that
    // no longer exists — which is the property that keeps the table honest, and it is what sent
    // this edit here rather than leaving six stale excuses behind.
    //
    // `facade::channels` and `facade::public_channels` are untouched and still `pub`; they simply
    // no longer have a CLI path to be waived FOR. Their HANDLE methods are gone too since nxf
    // 6j6v.yr59 — `public_channels` folded into `Engine::directory`, `channels` left as a named
    // loss — which this gate cannot see in either direction and does not need to. The disposition
    // record next door (`tests/seam_disposition.rs`) is where that fact is held, which is the
    // division of labour between the two gates: this one asks whether an agent's verbs are all
    // reachable from the seam, that one asks what became of the seam when a verb left.
    Waiver::KnownGap {
        verb: "tick",
        ticket: "6j6v.dvyq",
        reason: "NO SEAM TWIN BY DECISION, not by omission. `Engine::workflow_tick` was on §5's \
                 closed list and went with the run engine (6j6v.dvyq §3, recorded in \
                 `seam_disposition.rs`); the VERB stayed because a declared channel's `timeout` \
                 schedules a one-shot `at` job and that job invokes this binary. An embedding app \
                 does not drive a board's deadline by hand — it has its own scheduler, and what it \
                 schedules is this command. \
                 \
                 Hidden from `--help`, and this gate counts it anyway, which is the point: a \
                 hidden verb is still a verb an agent could type, so it may not slip out of the \
                 comparison merely by being invisible.",
    },
    // `withdraw` was waived `CliOnly` here — DROPPED by nxf 6j6v.0djn, because the waiver's
    // premise was measured and found false. It read: "a device-local queue over one checkout,
    // which is exactly the thing an embedding app does not have … an app that runs its roles
    // elsewhere has no such queue and therefore nothing to withdraw FROM."
    //
    // An embedding host reaches the queue. `working_tree: exclusive` is a DECLARATION, not a
    // command-line flag; the lease gate that reads it sits in `orchestration::trigger_role`, the
    // one funnel `Engine::send_to` passes through as much as `nxc send --to` does. Proven rather
    // than argued, by a test that existed before this waiver did:
    // `embed_surface.rs::engine_send_to_a_busy_exclusive_persona_reports_the_queue_instead_of_a_started_session`
    // opens an `Engine`, calls `send_to` twice against two `exclusive` personas and touches no CLI,
    // and the second receipt comes back `queue_position: Some(1)`.
    //
    // That made the standing state the class this house otherwise closes: `SendToReceipt` handed an
    // app `queue_position`/`queued_behind` — a state a surface is told to branch on (`nxc guide
    // limits-and-safety`) — with no call on the handle to answer it. `Engine::withdraw` now exists
    // as the thin `with_orchestration` forwarding the waiver's own last paragraph anticipated ("if
    // a host ever does hold a lease this becomes a seam method rather than a second
    // implementation"), so the verb needs no waiver at all: the gate finds it by name.
    // `embed_surface.rs::engine_withdraw_takes_back_the_commission_the_queue_receipt_reported`
    // drives the round trip through the handle.
    // `transcript append` was the last `KnownGap` here, and it is GONE WITH ITS DEBT rather than
    // with its verb (nxf 6j6v.c6e8, closed 2026-09-01). The waiver named an addressee —
    // app-foundations' `crates/agent-runtime` — and that host now has the call it was waiting for:
    // `facade::transcript_append` → `Engine::transcript_append`, which the CLI verb itself goes
    // through. The gate finds it by name, so there is nothing left to declare.
];

#[test]
fn every_nxc_verb_is_reachable_from_the_library_seam() {
    // The seam an embedding app links: the long-lived handle plus the compute layer under it. The
    // store below is deliberately NOT seam — reaching past the facade into the store is the very
    // thing E5c calls a violation (it is the finding that opened epic 6j6v.fjrc), so a capability
    // that exists only there is an omission, not a counterpart.
    let sources = ["src/engine.rs", "src/facade.rs"]
        .map(|rel| crate_relative(env!("CARGO_MANIFEST_DIR"), rel))
        .to_vec();
    assert_verb_seam("nxc", &nexus_chat::cli::command(), &sources, WAIVERS);
}

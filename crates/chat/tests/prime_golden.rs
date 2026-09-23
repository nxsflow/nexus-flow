//! The byte-exact `nxc prime` contract (nxf 6j6v.r5a2, epic 6j6v.fjrc).
//!
//! `nxc prime` is a SessionStart-hook output: `nxs prime` fans out to it and a host injects the
//! Markdown verbatim as session context. Its bytes are therefore a CONTRACT, not an implementation
//! detail — and lifting the assembly out of `cli.rs` into the shared facade layer is a MOVE, not a
//! reformulation. `contract.rs` asserts the *meaning* and `prime_validation.rs` the roster
//! partition; this file pins the exact stdout of every section — head, the requester wake (both
//! the complete list and the stale one), the address book, and the interactive-only tail: the
//! `writing-declarations` pointer, the declaration WARNINGS and the declaration errors (nxf
//! 6j6v.9w08 added the first two) — so the lift can be proven byte-for-byte rather than argued. It pinned the unread
//! catch-up too, in both dispositions, until nxf 6j6v.4mmk stopped rendering it and nxf 6j6v.4d2z
//! removed it. **"## Declared Team" no longer renders** (nxf h4d3, task 3): see `HEAD`'s doc
//! comment and `render_declared_team`'s in `crates/chat/src/facade.rs`.
//!
//! Written against the pre-lift implementation deliberately: if the shared-layer renderer changes so
//! much as a space, these fail.

use assert_cmd::Command;
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs};
use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-07-10T10:00:00Z";

/// A chat workspace with deterministic ids, seeded WITHOUT `nxc init` (no banner to strip) —
/// mirrors `prime_validation.rs`'s own helper.
fn workspace() -> TempDir {
    std::env::set_var("NXF_DETERMINISTIC_IDS", "1");
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    tmp
}

/// The base `nxc` invocation: `local/alice`, pinned clock and ids. `NXC_ACTOR` being SET makes this
/// a SPAWNED context (see `is_spawned_context`) — the interactive variant unsets all three signals.
fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// The explicitly-interactive variant: a human at the keyboard, no spawned-context signal at all.
/// Unsetting `NXC_ACTOR` also unpins the caller handle (it would fall back to `$USER`), so callers
/// pass `--consumer local/alice` to keep the rendered handle byte-stable.
fn nxc_interactive(tmp: &TempDir) -> Command {
    let mut c = nxc(tmp);
    c.env_remove("NXC_ACTOR")
        .env_remove("NXC_WORKER")
        .env_remove("NXC_SESSION");
    c
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success().get_output().clone();
    String::from_utf8(out.stdout).expect("utf8")
}

fn prime(tmp: &TempDir) -> String {
    stdout_of(nxc(tmp).arg("prime"))
}

/// The `general` channel with alice and bob in it.
///
/// Written to the store since 6j6v.dvyq §3 removed `nxc channels create`/`join` — a channel is a
/// declaration now, and nothing mints one from the command line. The ops are the ones those verbs
/// emitted, so the ids stay on the same deterministic sequence the goldens below are written
/// against.
fn seed_channel(tmp: &TempDir) -> String {
    let mut store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    store.set_wall_clock(NOW);
    let cid = store.mint_channel_id();
    store.set_channel_field(&cid, "name", "general", "local/alice");
    store.set_channel_field(&cid, "kind", "group", "local/alice");
    store.set_channel_field(&cid, "origin", "local", "local/alice");
    store.add_member(&cid, "local/alice", "local/alice");
    store.add_member(&cid, "local/bob", "local/alice");
    cid
}

/// Open a board as `actor`, expecting `expect`, and return its thread id — the fixture `nxc ask`
/// was (6j6v.dvyq §3 removed the verb; `facade::ask`, the write, is untouched).
fn seed_board_as(
    tmp: &TempDir,
    channel: &str,
    actor: &str,
    expect: &str,
    body: &str,
    deadline: Option<&str>,
) -> String {
    let mut store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    nexus_chat::facade::ask(
        &mut store,
        nexus_chat::facade::AskRequest {
            now: NOW,
            origin: "local",
            actor,
            channel,
            body,
            expect: &[expect.to_string()],
            deadline,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
    )
    .expect("the board opens")
    .thread_id
}

/// A previous session of `role` that has ENDED — the watermark a session start derives its notice
/// from (nxf 6j6v.2hx9).
///
/// Without one there is no window and no notice at all, which is the correct answer for a caller
/// that has never run here and the wrong fixture for a test about what the notice CONTAINS.
///
/// Earlier than every instant these fixtures use — [`NOW`] itself AND the deadlines they set on a
/// stale board — so everything they seed lands inside the window. A watermark between the two would
/// hide the stale half and leave a test passing for the wrong reason.
const PREVIOUS_SESSION_ENDED: &str = "2026-07-08T00:00:00Z";

fn a_previous_session_of(tmp: &TempDir, role: &str) {
    let mut store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let id = format!("s-{role}");
    store.create_pending_session(&id, role).expect("mint");
    store
        .mark_session_ended(&id, PREVIOUS_SESSION_ENDED)
        .expect("end it");
}

/// [`seed_board_as`] for the ordinary direction: alice opens it, bob owes the reply.
fn seed_board(tmp: &TempDir, channel: &str, body: &str, deadline: Option<&str>) -> String {
    seed_board_as(tmp, channel, "alice", "local/bob", body, deadline)
}

/// Post as bob into `channel` — the fixture `nxc send <channel> <body>` was, before the raw
/// channels went (6j6v.dvyq §3). What these goldens are about is what `prime` RENDERS, so the
/// traffic has to exist and does not have to be typed.
fn post_from_bob(tmp: &TempDir, channel: &str, body: &str, disposition: Disposition) {
    let mut store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    store.set_wall_clock(NOW);
    store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: channel.into(),
        sender: "local/bob".into(),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition,
        thread_id: None,
        refs: Refs::default(),
        body: body.into(),
    });
}

/// Post bob's awaited reply straight into the store. Under `NXF_DETERMINISTIC_IDS` a thread id and a
/// message id come from the SAME sequence, so `nxc reply <thread>` would resolve the id to the
/// like-numbered MESSAGE instead — the same reason `parity.rs` posts its completing reply this way.
fn post_reply(tmp: &TempDir, channel: &str, thread: &str, body: &str) {
    let mut store = Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    store.set_wall_clock(NOW);
    store.post_message(&MessageEnvelope {
        origin: "local".into(),
        channel_id: channel.into(),
        sender: "local/bob".into(),
        kind: MessageKind::Report,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread_id: Some(thread.into()),
        refs: Refs::default(),
        body: body.into(),
    });
}

/// The `## Declarations` block `prime` renders for a workspace that declares NOTHING (nxf
/// 6j6v.dvyq): the empty case says so, and says where a declaration belongs.
///
/// Interpolated rather than baked into the constants beside it, because the one thing it carries
/// that a reader needs is the workspace-ABSOLUTE path — and the `nxc` subprocess reaches the
/// workspace through the process cwd, which on macOS resolves the `TempDir` symlink. Everything
/// else about the block is fixed text and is asserted verbatim.
fn nothing_declared(tmp: &TempDir) -> String {
    let dir = std::fs::canonicalize(tmp.path())
        .unwrap_or_else(|_| tmp.path().to_path_buf())
        .join(".nxs-personas");
    format!(
        "\n\n## Declarations\n\nnobody is declared here yet — declare one as \
         `{d}/<handle>.yaml` (a persona) or `{d}/channels.yaml` (a channel).",
        d = dir.display()
    )
}

fn write_role(tmp: &TempDir, handle: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: be helpful\n"),
    )
    .unwrap();
}

/// The full head every `nxc prime` emits: title, the intro paragraph (the coordination rule folded
/// in as a prohibition, task 3), and "## How a conversation moves" — the verbs task 3 kept, less
/// `withdraw`, which nxf 6j6v.ezbr took out again (a person's verb), as literal shell examples with
/// a trailing `#` comment each.
///
/// **Cut to this shape by nxf h4d3 (task 3, q3fh session-start-budget), from a head that ran
/// through a Context Recovery blockquote, a separately labelled Coordination rule, eight commands
/// under "## Chat Commands", and a when-stuck paragraph.** That older shape MEASURED 1.938 B against
/// this same fixture (a bare, empty workspace — the fixed prose is the whole output there); this one
/// measures 720 B, byte-for-byte the owner's own edited target draft (`nxc-static-target.md`). What
/// an agent can pull on demand via `nxc --help`/`nxc guide` left the block; the one thing neither
/// page teaches — the prohibition on coordinating through scratch files — stayed, folded into the
/// intro paragraph below instead of a standalone labelled rule.
///
/// **It runs to the END of the fixed block now, not to a `## Unread` heading** (nxf 6j6v.4mmk).
/// There is no unread section any more: what a session pays for at every start has to be something
/// it cannot get otherwise and needs before it acts, and message bodies are neither.
const HEAD: &str = "\
# nexus-chat — the channel between agents

`nxc` is how the agents in this workspace coordinate: `send --to` starts something, `reply --thread` \
answers it, and every message is durable and replayable. **Never coordinate through scratch files or \
ad-hoc notes** — they are not delivered, not synced, and never replayed here. `nxc --help` lists the \
commands, `nxc guide` the topics behind them.

## How a conversation moves

    nxc send --to <handle> --ref nxf_ids=<id> -         # start one; a thread id comes back
    nxc reply --thread <id> -                           # answer — you are finished
    nxc reply --thread <id> --escalate -                # you need help or a DECISION first

`-` reads the message from STDIN — `… - <<'EOF'`, your text, then `EOF` on its own line — so the
shell evaluates nothing in it. A one-line message may be an argument instead.

`--escalate` is not only \"I cannot\": it is also how you ask for a DECISION that is not yours to
make — one you could act on either way, but may not settle. It goes UP, to whoever commissioned
you. Put that question in an ordinary reply instead and it travels DOWN to the next step, which
may not settle it either, and the work runs on around it.";

/// The persona identity block, byte for byte — `## You are …`, its declared fields, and the four
/// `nxc` rules in order. Composed from [`PERSONA_RULES`] so the ONE thing this file exists to pin
/// (bytes a host injects verbatim) is not restated twice in one const.
const PERSONA_HEAD: &str = "\
## You are `pm`
- **Role:** Product manager
- **Job:** Plans the release.
- **Expected output:** A dated plan.
- **Stage:** senior

### How you use `nxc`
";

/// The four rules, each already prefixed with its bullet. Rule ONE teaches the id-free `nxc reply`
/// first and names the condition that ends it in the same breath (nxf 6j6v.dq59) — a shorthand
/// taught without its precondition is a shorthand reached for in the one state the engine has to
/// refuse. Rule two ends with the RESUME SENTENCE
/// (nxf 6j6v.z6f9) — the one line that tells a persona a question does not cost it its context, and
/// the reason this golden exists at all: it replaced a ritual of state-trailer lines and a
/// mandatory `nxc threads show` per turn that five declarations in an external testbed had grown
/// against an amnesia that does not exist. A rewrite that drops it now fails here by name.
const PERSONA_RULES: &str = "\
- Answer in the conversation you were handed: `nxc reply - <<'EOF'`, your text, then `EOF` on its \
own line. The `-` reads the message from STDIN, where the shell evaluates nothing in it — a \
one-line answer may be an argument instead (`nxc reply \"lgtm\"`), longer text must not, because \
backticks, `$(…)` and quotes in it are rewritten before nxc sees them. While that is the only \
conversation open for you, no thread id is needed; open a second — commission somebody yourself — \
and `nxc reply --thread <thread-id> -` says which, and the message that wakes you names each one \
with the command it takes.
- Need something before you can answer? Same move — reply into the thread and say what you need. \
Whoever is waiting sees it and answers you back in the same thread. Asking and ending your turn is \
safe: while the round is open, the answer resumes you with everything you already know — once it \
has been answered and handed on, that thread is closed and replying into it is refused, so start \
the next thing with `nxc send`.
- Starting something NEW with someone else: `nxc send --to <persona|channel> -`, with the body on \
STDIN exactly as above (or a short one as an argument). It returns a thread id; that thread is \
where their answer arrives.
- `nxc list` shows who you can address and what for.
";

#[test]
fn prime_for_a_persona_puts_its_identity_and_nxc_rules_between_the_rule_and_the_commands() {
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: be helpful\njob_title: Product manager\n\
         job_description: Plans the release.\nexpected_output: A dated plan.\nstage: senior\n",
    )
    .unwrap();

    // The identity block sits BETWEEN the intro paragraph and the command reference — "who am I"
    // anchors how everything after it is read. Splitting [`HEAD`] rather than restating it keeps
    // the head single-sourced, so a change to it reddens every golden in this file at once.
    let (before_commands, commands) = HEAD.split_once("## How a conversation moves").unwrap();
    // No trailing section here (nxf h4d3, task 3): "## Declared Team" is gone outright (the
    // duplication note on `render_declared_team`'s own doc comment), pm's own address book is empty
    // (nobody else is declared for it to address), and `render_declaration_source` stays silent for
    // the ordinary case — a `.nxs-personas/` declaration with no legacy folder.
    assert_eq!(
        stdout_of(nxc(&tmp).args(["prime", "--persona", "pm"])),
        format!(
            "{before_commands}{PERSONA_HEAD}{PERSONA_RULES}\n\
             ## How a conversation moves{commands}\n"
        )
    );
}

#[test]
fn prime_with_no_unread_is_byte_exact() {
    let tmp = workspace();
    assert_eq!(prime(&tmp), format!("{HEAD}{}\n", nothing_declared(&tmp)));
}

/// **The acceptance of nxf 6j6v.4mmk's second half**, and the inverse of the golden that stood
/// here (`prime_replays_both_dispositions_byte_exact`): unread used to be replayed in full text and
/// now is not replayed at all. Two unread messages, both dispositions, and the block is byte for
/// byte the one an empty workspace gets.
#[test]
fn prime_renders_no_message_text_whatever_is_unread() {
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    post_from_bob(&tmp, &cid, "please review PR 42", Disposition::InTurn);
    post_from_bob(&tmp, &cid, "look next session", Disposition::NextSession);

    let out = prime(&tmp);
    assert_eq!(out, format!("{HEAD}{}\n", nothing_declared(&tmp)));
    for body in ["please review PR 42", "look next session"] {
        assert!(
            !out.contains(body),
            "the block still carries {body:?}:\n{out}"
        );
    }
}

/// **The size of the block does not grow with the number of unread messages** — the property the
/// item asks a test to hold, because the measured failure was not "the section is ugly" but "the
/// section grows with the operating time of the workspace": 199.491 bytes, 98 % message text, back
/// tenfold in one working day.
///
/// Stated as an EQUALITY, not a bound: with the bodies gone the block is literally identical, so a
/// re-introduction of any per-message rendering — a counter, a signpost, one line each — reddens
/// here rather than passing under a generous threshold.
#[test]
fn prime_size_is_independent_of_how_much_is_unread() {
    let empty = {
        let tmp = workspace();
        prime(&tmp)
    };
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    for i in 0..25 {
        post_from_bob(
            &tmp,
            &cid,
            &format!("message {i}: {}", "x".repeat(500)),
            Disposition::InTurn,
        );
    }
    assert_eq!(
        prime(&tmp).len(),
        empty.len(),
        "25 unread messages of ~500 bytes each must not add a single byte to the block"
    );
}

// `the_unread_fields_are_untouched_although_the_block_no_longer_shows_them` stood here — nxf
// 6j6v.4mmk's own acceptance test, asserting that `prime --json` still carried `count`,
// `in_turn[0].body`/`disposition` and `next_session[0]` after the human block stopped drawing them.
// The fields were left standing precisely because the decision about them belonged to another
// ticket, which the test named: nxf 6j6v.4d2z. That ticket removed them, so the test goes with its
// subject. Its TWIN survives and now carries the whole "a rendering cut is not a data cut" claim on
// its own — see `the_wake_field_is_untouched_although_the_block_no_longer_shows_it` below.

/// **The acceptance of nxf 6j6v.1gm9**, and the inverse of the golden that stood here
/// (`prime_threads_you_opened_is_byte_exact`): a completed board used to replay its whole
/// conversation under `## Threads you opened`, and now nothing of it reaches the block at all.
///
/// Both halves of the old section are seeded — a board past its deadline (the `stale` advisory) and
/// one bob completed (the `complete` list with its message text) — because the ticket removed the
/// section, not just its heaviest half. Asserted as an EQUALITY against the empty workspace, so a
/// re-introduction of any per-board rendering (a counter, a signpost, one line each) reddens here
/// rather than passing under a generous bound.
#[test]
fn prime_renders_no_board_text_however_many_threads_completed() {
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    // A board past its deadline with nobody replying (stale) …
    seed_board(
        &tmp,
        &cid,
        "review the migration plan",
        Some("2026-07-09T00:00:00Z"),
    );
    // … and a board bob completed (finished inside the caller's window).
    let thread = seed_board(&tmp, &cid, "sign off on the release", None);
    post_reply(&tmp, &cid, &thread, "lgtm");

    let out = prime(&tmp);
    assert_eq!(out, format!("{HEAD}{}\n", nothing_declared(&tmp)));
    assert!(
        !out.contains("Threads you opened"),
        "the section itself is gone, heading included:\n{out}"
    );
    for body in [
        "review the migration plan",
        "sign off on the release",
        "lgtm",
        "past deadline",
    ] {
        assert!(
            !out.contains(body),
            "the block still carries {body:?}:\n{out}"
        );
    }
}

/// **The size of the block does not grow with the number of COMPLETED BOARDS** — the property the
/// item asks a test to hold by name ("die Groesse von `prime` waechst nicht mehr linear mit der Zahl
/// abgeschlossener Bretter … gemessen, nicht als Schranke mit grossem Spielraum"), and the twin of
/// [`prime_size_is_independent_of_how_much_is_unread`] above.
///
/// The measured failure was a cliff, not a price: in `watch-bundestag` this section was 46.487 of a
/// 63.787-byte composed session start — 74 %, six completed boards out of 24 threads — and above the
/// host's SessionStart cut-off a session is handed a 2 KB preview and a file path instead of its
/// board, its vocabulary and its memories. Twelve boards of ~1 KB each here would have been +12 KB
/// under the old renderer; the assertion is that they are +0.
#[test]
fn prime_size_is_independent_of_how_many_boards_completed() {
    let empty = {
        let tmp = workspace();
        prime(&tmp)
    };
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    for i in 0..12 {
        let thread = seed_board(
            &tmp,
            &cid,
            &format!("commission {i}: {}", "x".repeat(500)),
            None,
        );
        post_reply(
            &tmp,
            &cid,
            &thread,
            &format!("report {i}: {}", "y".repeat(500)),
        );
    }
    assert_eq!(
        prime(&tmp).len(),
        empty.len(),
        "12 completed boards of ~1 KB each must not add a single byte to the block"
    );
}

/// **Why nothing replaced the section** (nxf 6j6v.1gm9), as a test rather than as an assurance.
///
/// The one fact worth keeping was named while the removal was being cut: *a thread awaiting a reply
/// from THIS session is a fact it must know* — the dividing line being "does this session owe
/// something here", not "is this unread". This pins the finding that settled it: **that fact was
/// never in this section.** The wake is keyed on `ChatStore::threads_opened_by`, so it is the
/// OPENER's view; on every board it carries, `expects_reply_from` names OTHER handles, never the
/// caller. So the removal loses none of it, and a line for it would be a NEW derivation over
/// `expects_reply_from`, argued on its own merits. What the session gets instead is the push it
/// always got: `send` puts the body in the prompt that starts it, `reply` resumes it with the
/// reply's body.
///
/// **The board here is deliberately STALE**, which is what makes this a test of the opener keying
/// rather than of the shape of the two lists. A board nobody has answered is neither complete nor —
/// without a deadline — stale, so it would be absent from anyone's wake for a second, unrelated
/// reason, and the assertion would pass while proving nothing. Past its deadline it is exactly the
/// kind of board the wake DOES surface: it is in bob's, who opened it, and not in alice's, who owes
/// the reply. Counter-probed by widening `threads_opened_by` to "opened OR expected", which turns
/// alice's wake red here and nowhere else.
#[test]
fn the_wake_never_carried_what_this_session_owes() {
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    // Both callers have a previous session end, so both HAVE a window — which is what makes the
    // assertion below about WHOSE board it is rather than about who has a watermark.
    a_previous_session_of(&tmp, "alice");
    a_previous_session_of(&tmp, "bob");
    // Bob opens a board and alice owes the reply — the "this session owes something here" case —
    // and it is past its deadline, so the wake would surface it if it surfaced this side at all.
    let owed = seed_board_as(
        &tmp,
        &cid,
        "bob",
        "local/alice",
        "alice, can you confirm the cut-off?",
        Some("2026-07-09T00:00:00Z"),
    );

    // Alice, who OWES the reply: nothing. Not one line, in either view.
    let alice_json: serde_json::Value =
        serde_json::from_str(stdout_of(nxc(&tmp).args(["--json", "prime"])).trim())
            .expect("valid json");
    assert!(
        alice_json.get("threads_you_opened").is_none(),
        "the wake is the opener's view, and alice opened nothing: {alice_json}"
    );
    assert!(
        !prime(&tmp).contains(&owed),
        "and the block never named the board she owes"
    );

    // Bob, who OPENED it: the same board, in his wake's stale list. This is the control — the board
    // IS of a kind the derivation surfaces, so alice's empty wake is about WHOSE it is.
    let bob_json: serde_json::Value = serde_json::from_str(
        stdout_of(nxc(&tmp).args(["--json", "prime", "--consumer", "local/bob"])).trim(),
    )
    .expect("valid json");
    let stale = bob_json["threads_you_opened"]["stale"]
        .as_array()
        .unwrap_or_else(|| panic!("bob opened it, so it is in his wake: {bob_json}"));
    assert_eq!(stale.len(), 1, "{bob_json}");
    assert_eq!(stale[0]["thread_id"], owed, "{bob_json}");
    assert_eq!(
        stale[0]["outstanding"],
        serde_json::json!(["local/alice"]),
        "…and what it says is that ALICE owes it — the fact she is never told here: {bob_json}"
    );
}

/// **The wake RECORD stays** (nxf 6j6v.1gm9), and this is the test that item asked for by name.
/// Dropping a rendering is not dropping a contract: `threads_you_opened` is what `Engine::prime_as`
/// carries to an app, and that ticket's scope was the AGENT surface.
///
/// **It is the only half of that pair left.** 6j6v.4mmk left the unread fields standing on the same
/// argument and named nxf 6j6v.4d2z as what would decide them; 4d2z decided, and they went with the
/// read cursor, the ack and the per-channel counts. So this test now carries the whole claim that a
/// rendering cut is not a data cut — the wake is the record that proves it, because it is the one
/// that survived being measured.
#[test]
fn the_wake_field_is_untouched_although_the_block_no_longer_shows_it() {
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    a_previous_session_of(&tmp, "alice");
    let thread = seed_board(&tmp, &cid, "sign off on the release", None);
    post_reply(&tmp, &cid, &thread, "lgtm");

    let json: serde_json::Value =
        serde_json::from_str(stdout_of(nxc(&tmp).args(["--json", "prime"])).trim())
            .expect("valid json");
    let complete = json["threads_you_opened"]["complete"]
        .as_array()
        .unwrap_or_else(|| panic!("the record still carries the wake: {json}"));
    assert_eq!(complete.len(), 1, "{json}");
    assert_eq!(complete[0]["thread_id"], thread, "{json}");
    assert_eq!(
        complete[0]["messages"][1]["body"], "lgtm",
        "the bodies are still on the seam, they are only not rendered: {json}"
    );
}

#[test]
fn prime_address_book_warnings_and_errors_are_byte_exact_and_interactive_gated() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    write_role(&tmp, "coder");
    // Two channels, one of them naming a member nobody declared — so the declaration errors are
    // non-empty and the interactive/spawned split below has something to be a split ABOUT. It was a
    // clean and a broken WORKFLOW declaration until 6j6v.dvyq §3 removed those; the channel
    // partition is the same shape, with one difference worth knowing: channel validation is
    // WHOLE-FILE scoped, so one broken entry excludes every channel this file declares from the
    // roster. The ADDRESS BOOK is unaffected — it renders the declarations, which are read without
    // referential validation — which is exactly why both are pinned here.
    std::fs::write(
        tmp.path().join(".nxs-personas/channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n  description: the whole team's daily sync\n\
         - name: ghosts\n  members: [ghost]\n",
    )
    .unwrap();

    // Whom the caller may address (nxf 6j6v.p6m1, re-cut in 6j6v.frek) — how to address anyone said
    // ONCE in the header, then one paragraph per target: who they are, the handle to type, and what
    // they are for. A caller with no persona of its own sees the whole declared team, which is the
    // acceptance's own first step: a human runs `prime` and sees who is there and what for. It
    // precedes the roster, which stays the terse name list it has always been.
    //
    // THIS IS THE PROOF THAT `prime` AND `nxc list` CANNOT DRIFT (frek acceptance 5): both render
    // `Directory::render_markdown`, so this byte-exact golden pins the persona's view of the
    // directory and the human's at the same time. A channel's `description` reaches it too — the
    // field that made the channel half of the list sayable at all.
    let address_book = "\n## Who you can address\n\n\
                        Address any of them the same way: `nxc send --to <handle> -` — the `-` reads the message from STDIN, and a one-line message may be an argument instead.\n\n\
                        **Coder** (handle: `coder`)\n\n\
                        **Pm** (handle: `pm`)\n\n\
                        **Standup** (handle: `standup`, members: pm, coder) — the whole team's daily sync\n\n\
                        **Ghosts** (handle: `ghosts`, members: ghost)\n";
    let caught_up = format!("{HEAD}\n");

    // The interactive-only tail (nxf 6j6v.9w08), in the order a reader works through it: the one
    // line pointing at the topic that would have prevented the rest, then what is wrong with the
    // declarations here — WARNINGS (it loads and will work badly) before ERRORS (it does not
    // resolve, and something the roster would have offered is excluded).
    let writing_declarations = "\n> **Before you write or change a persona or a channel:** \
                                `nxc guide writing-declarations`.\n";
    // Three of them, and each is a real defect of this fixture rather than a contrivance: neither
    // persona declares a `job_description` (`write_role` writes the two-line minimum), and `ghosts`
    // declares no `description`. Roles first in load order, then channels in declaration order.
    // `standup` is absent on purpose — its `description` is a real one, which is what proves the
    // check is about the field being usable rather than about it being present.
    let warnings = "\n## Declaration Warnings\n\n\
                    - `coder.yaml`: `job_description` is not declared — that is the one thing a \
                    calling agent is shown when it decides whom to address\n\
                    - `pm.yaml`: `job_description` is not declared — that is the one thing a \
                    calling agent is shown when it decides whom to address\n\
                    - `channels.yaml`: channel \"ghosts\" declares no `description` — that is the \
                    one thing a calling agent is shown when it decides whom to address\n";

    // "## Declared Team" no longer renders (nxf h4d3, task 3): the bare roster it printed read as
    // noise beside the address book right above, which says who each one is AND what for — not a
    // strict superset (a channel-only persona is a channel-entry line there, not a roster line),
    // but this fixture's roster (`coder`, `pm`) is exactly its address book's roles either way, so
    // the removal costs nothing here. Interactive (a human at the keyboard): the address book, the
    // guide pointer, the declaration warnings AND the declaration errors.
    assert_eq!(
        stdout_of(nxc_interactive(&tmp).args(["prime", "--consumer", "local/alice"])),
        format!(
            "{caught_up}{address_book}{writing_declarations}{warnings}\n\
             ## Declaration Errors\n\n\
             - `channels.yaml`: channel \"ghosts\": unknown member handle \"ghost\" — it names \
             neither a declared role nor a declared channel\n"
        )
    );
    // Spawned (a role session): the same address book, and NOTHING else — the errors, the warnings
    // and the pointer are silently excluded, because nobody is there to act on any of them and a
    // session start is not the place to spend bytes on its author's job. Byte-identical to what
    // this assertion said before 6j6v.9w08 added two of the three.
    assert_eq!(prime(&tmp), format!("{caught_up}{address_book}"));
}

#[test]
fn prime_json_is_byte_exact() {
    let tmp = workspace();
    let cid = seed_channel(&tmp);
    post_from_bob(&tmp, &cid, "please review PR 42", Disposition::InTurn);

    // `serde_json::Value`'s own key order (a BTreeMap — no `preserve_order` in the build graph), so
    // every object's keys are alphabetical and stable; `threads_you_opened`/`roles`/`channels`/
    // `workflows`/`declaration_errors` are absent entirely.
    //
    // `count`, `in_turn` and `next_session` stood in this golden between `coordination_rule` and
    // `when_stuck` — `next_session` as an empty array, present, unlike the omitted-when-empty five
    // above, and `in_turn` carrying the seeded message with its five projected fields. All three
    // went with the unread apparatus (nxf 6j6v.4d2z). The message below is still SEEDED, so the
    // golden is over a workspace with traffic rather than an empty one: what it now says is that
    // traffic adds nothing to this object at all.
    //
    // `declarations` is ALWAYS present, unlike those five (nxf 6j6v.dvyq): its whole job is to make
    // the empty case say something, and a field that vanished exactly when the workspace had
    // nothing to show would answer the question only when nobody was asking it. `legacy_path`
    // inside it is the omitted-when-absent kind — this workspace never had a legacy `roles/`
    // folder.
    //
    // Unlike the human views above, this one GREW under 6j6v.r5a2 — additively. Every pre-existing
    // key is untouched (including the omitted-when-empty convention); `context_recovery`,
    // `coordination_rule` and `commands` joined them because the JSON is now a projection of the
    // whole report rather than a second, thinner assembly. Same change, same reason, as `nxm`'s.
    assert_eq!(
        stdout_of(nxc(&tmp).args(["--json", "prime"])),
        format!(
            "{{\"commands\":[\
             {{\"invocations\":[\"nxc list\"],\
             \"summary\":\"who can be addressed here, and what for\"}},\
             {{\"invocations\":[\"nxc send --to <persona|channel> --ref nxf_ids=<id> -\"],\
             \"summary\":\"start a conversation; you get a thread id back — say what it is \
             about, or `--no-ref`. `-` reads the body from STDIN\"}},\
             {{\"invocations\":[\"nxc reply --thread <id> -\"],\
             \"summary\":\"answer in that thread — `--escalate` says you cannot reach the \
             result and need help or a decision, and hands the round UP to whoever commissioned \
             you. `-` reads the body from STDIN, so the shell evaluates nothing in it\"}},\
             {{\"invocations\":[\"nxc status [--thread <id>] [--all]\"],\
             \"summary\":\"what is still going on here; `--thread` for one operation, `--all` \
             for the finished ones too\"}},\
             {{\"invocations\":[\"nxc threads show <thread-id>\"],\
             \"summary\":\"one board in full: who still owes a reply, and whether it holds the \
             WORKING COPY (`holding` / `waiting (#N)` / `not needed`)\"}},\
             {{\"invocations\":[\"nxc search \\\"<text>\\\"\"],\
             \"summary\":\"find a message again, across your channels\"}},\
             {{\"invocations\":[\"nxc transcript show <session>\"],\
             \"summary\":\"what a session actually did, step by step\"}}],\
             \"consumer\":\"local/alice\",\
             \"context_recovery\":\"re-run `nxs prime` after a context compaction to reload this \
             block.\",\
             \"coordination_rule\":\"**Coordination rule:** agent-to-agent coordination in this \
             workspace goes through `nxc` — `send --to` to start something, `reply --thread` to \
             answer it. **Do not** coordinate through ad-hoc notes or scratch files: they are not \
             delivered, not synced, and never replayed here. `nxc` is the one durable channel \
             between agents.\",\
             \"declarations\":{{\"count\":0,\"kind\":\"none\",\"path\":\"{d}\",\
             \"read_dir\":\"{d}\"}},\
             \"when_stuck\":\"**When something does not move:** a commission that has not \
             started yet is parked behind the WORKING COPY, and `nxc threads show <thread-id>` is \
             the only place that state is visible (`working tree: holding` / `waiting (#N)` / \
             `not needed`). What holds it is the chain that took it — most often an escalation \
             nobody has answered, because an escalated round keeps the working copy until a reply \
             lands in that thread. So: answer the holding thread \
             (`nxc reply --thread <id> -`) if the answer is yours to give; if it is not, escalate \
             on your own thread (`nxc reply --thread <id> --escalate -`), which goes UP, towards \
             whoever may decide what becomes of the round holding the copy. An abandoned lease is \
             reclaimable by the next chain that asks after 2h.\"}}\n",
            d = std::fs::canonicalize(tmp.path())
                .unwrap_or_else(|_| tmp.path().to_path_buf())
                .join(".nxs-personas")
                .display()
        )
    );
}

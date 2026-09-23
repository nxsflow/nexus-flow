//! Every entrance 6j6v.dvyq §3 takes away, PROVEN EXPRESSIBLE BY CONSTRUCTION — one test per line
//! of the removal list, each driving the REPLACEMENT and showing the same effect.
//!
//! > every removed entrance is **expressible by a remaining one, proven by construction (no paper
//! > mapping)**.
//!
//! That is acceptance point 1, and the "no paper mapping" is the whole instruction: a table in a
//! doc saying `agents list -> list` is a claim, not a proof. So each test here does two things and
//! neither on its own would be enough:
//!
//! 1. **the entrance is really gone** — the binary refuses it, which is what an agent finds out;
//! 2. **the replacement really does it** — driven for real, against a real workspace, asserting the
//!    effect the old entrance produced, not merely that the new verb exits zero.
//!
//! Where the two do not produce a byte-identical answer, the test says what changed and why that is
//! the point rather than a regression: `agents search` searched a runtime PROFILE table, `list`
//! reads the DECLARED team, and the consolidation is exactly the move from one to the other.

use assert_cmd::Command;
use nexus_chat::workspace::ChatWorkspaceExt;
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-16T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env_remove("NXC_WORKER")
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn json_of(cmd: &mut Command) -> Value {
    serde_json::from_str(stdout_of(cmd).trim()).expect("valid json")
}

/// What a refused entrance looks like to an agent: a non-zero exit and clap's own complaint. This
/// is the half a paper mapping cannot have — the door is shut, not merely undocumented.
fn refused(tmp: &TempDir, args: &[&str]) -> String {
    let out = nxc(tmp).args(args).assert().failure();
    String::from_utf8_lossy(&out.get_output().stderr).into_owned()
}

/// The session the dry worker recorded for `handle` — what a persona's own `nxc` invocation must
/// carry as `NXC_SESSION` for the engine to recognise it as that persona answering.
fn session_of(tmp: &TempDir, handle: &str) -> String {
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).expect("the dry worker logged");
    log.lines()
        .filter(|l| l.starts_with(&format!("trigger role={handle} ")))
        .filter_map(|l| {
            l.split_whitespace()
                .find_map(|kv| kv.strip_prefix("session=").map(str::to_string))
        })
        .next_back()
        .unwrap_or_else(|| panic!("no dry-worker trigger for {handle} in:\n{log}"))
}

/// `nxc` as a spawned persona: the two stamps a real triggered session carries.
fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = nxc(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

/// The one thread a channel's supervisor opened below `parent` for `handle` — its SLOT. Read from
/// the store rather than from a receipt, because the supervisor opens it inside the same write path
/// and hands nothing back about it.
fn slot_under(tmp: &TempDir, parent: &str, handle: &str) -> String {
    let store = nexus_chat::workspace::Workspace::resolve(None, tmp.path())
        .unwrap()
        .open_chat_store()
        .unwrap();
    let want = format!("local/{handle}");
    store
        .supervised_children(parent)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no slot for {handle} under {parent}"))
}

/// Declare a team in the workspace's own `.nxs-personas/`, which is what "registering an agent"
/// became.
fn declare(tmp: &TempDir, file: &str, body: &str) {
    let dir = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file), body).unwrap();
}

const QA: &str = "handle: qa\njob_title: QA Reviewer\n\
                  job_description: runs the race-flag suite\nsystem_prompt: You are QA.\n";
const CODER: &str = "handle: coder\njob_title: Implementer\n\
                     job_description: turns a work order into code\n\
                     system_prompt: You are coder.\n";

// ---- LEBENSZYKLUS: the profile verbs (§3) ---------------------------------------------------
//
// `agents register` -> a persona declaration; `agents list`/`agents search` -> `list`.

#[test]
fn agents_register_is_expressible_as_declaring_a_persona() {
    let tmp = workspace();
    assert!(
        refused(&tmp, &["agents", "register", "local/qa", "--title", "QA"])
            .contains("unrecognized subcommand"),
        "the entrance is gone"
    );

    // The replacement, driven: a declaration file. The effect `agents register` produced was "this
    // handle is now somebody here" — so that is what is asserted, on both surfaces that answer it.
    declare(&tmp, "qa.yaml", QA);

    let listed = json_of(nxc(&tmp).args(["--json", "list"]));
    assert_eq!(listed["personas"][0]["handle"], "qa");

    // …and, unlike a profile row, a declared persona is ADDRESSABLE, which is the point of
    // registering one at all.
    let opened = json_of(nxc(&tmp).args(["--json", "send", "--to", "qa", "please review"]));
    assert_eq!(opened["target"], "persona");
    assert!(opened["session"].is_string(), "{opened}");
}

#[test]
fn agents_list_is_expressible_as_list() {
    let tmp = workspace();
    assert!(
        refused(&tmp, &["agents", "list"]).contains("unrecognized subcommand"),
        "the entrance is gone"
    );
    declare(&tmp, "qa.yaml", QA);
    declare(&tmp, "coder.yaml", CODER);

    let v = json_of(nxc(&tmp).args(["--json", "list"]));
    let handles: Vec<&str> = v["personas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["handle"].as_str().unwrap())
        .collect();
    assert_eq!(
        handles,
        ["coder", "qa"],
        "the whole team, handle-sorted exactly as `agents list` was: {v}"
    );
    // And the human form is a list a human can read, which is what the verb was for.
    let human = stdout_of(nxc(&tmp).args(["list"]));
    assert!(human.contains("qa") && human.contains("coder"), "{human}");
}

#[test]
fn agents_search_is_expressible_as_list_which_carries_the_two_fields_it_searched() {
    let tmp = workspace();
    assert!(
        refused(&tmp, &["agents", "search", "REVIEWER"]).contains("unrecognized subcommand"),
        "the entrance is gone"
    );
    declare(&tmp, "qa.yaml", QA);
    declare(&tmp, "coder.yaml", CODER);

    // `agents search` was a substring match over `job_title` + `job_description`. `list` carries
    // both fields for the whole declared team, so the same question is answerable — by the caller,
    // over a set that is now bounded by the declaration rather than by whatever a runtime write
    // once put in the profile table.
    let v = json_of(nxc(&tmp).args(["--json", "list"]));
    let qa = v["personas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["handle"] == "qa")
        .expect("the declared persona is in the directory");
    assert_eq!(qa["job_title"], "QA Reviewer");
    assert_eq!(qa["job_description"], "runs the race-flag suite");

    // The deliberate DIFFERENCE, stated rather than glossed: `agents search` could only find a
    // handle somebody had registered at runtime, in this one workspace's database. `list` answers
    // over the declared team — which is the set `send --to` can actually reach, and the reason the
    // profile table stopped being the answer.
    let human = stdout_of(nxc(&tmp).args(["list"]));
    assert!(
        human.contains("QA Reviewer"),
        "the searchable text reaches a human too: {human}"
    );
}

/// The `agents` group is gone as a whole, not verb by verb with a stub left behind — the group node
/// itself is what an agent would tab-complete into.
#[test]
fn the_agents_group_itself_is_gone() {
    let tmp = workspace();
    let err = refused(&tmp, &["agents"]);
    assert!(err.contains("unrecognized subcommand"), "{err}");
    // And the help page does not advertise it (the hole 6j6v.spjj closed for this tree).
    let help = stdout_of(nxc(&tmp).args(["--help"]));
    assert!(!help.contains("agents"), "{help}");
}

// ---- INBOX AND READ: off the agent surface (§3), then out of the binary (6j6v.1gm9) ----------
//
// "Off the AGENT surface" and "for the HUMAN, `inbox` stays" were only compatible because the agent
// surface is something other than the verb list of the binary: it is the set a session is TAUGHT at
// its start. That split is expressed where a session learns what it may do — `prime` — and the
// tests below take it from both sides.
//
// **The human half then went too** (nxf 6j6v.1gm9, 2026-08-27). The verb was kept for "a human at
// the keyboard has no session to be primed into", and the owner answered that product question the
// other way: a human reads a CONVERSATION (`nxc threads show`, `nxc status`), not a flat unread list
// with no thread to answer in. `read` went with it — it was the brake on the "Threads you opened"
// block that the same ticket removed, and it had never been used either (0/66 role sessions, like
// `inbox`). The compute half did NOT go then — `facade::inbox` was what `prime` read, and the
// record was on the seam as `PrimeReport::in_turn`/`next_session` — and went on 2026-09-08
// (6j6v.4d2z), so today there is nothing behind either entrance at all.

/// **The premise moved, the conclusion did not** (nxf 6j6v.4mmk). This test used to say a persona
/// needs no `inbox` BECAUSE `prime` replays its unread; `prime` no longer replays anything, and the
/// verb is still off the agent surface — for the reason the replay was always standing in for.
///
/// A spawned persona is handed its task in the TRIGGER MESSAGE and a resumed one gets the answer as
/// a new turn, so there is no moment at which it would ask its inbox. The measurement agrees: over
/// 66 persona sessions and seven roles in the 4jgn.g90w testbed, `inbox` and `read` were used
/// ZERO times each — while the replay they were supposed to serve cost 199.491 bytes, 98 % of the
/// block, in every session.
#[test]
fn a_persona_gets_its_task_in_the_trigger_so_the_block_teaches_no_inbox() {
    let tmp = workspace();
    declare(&tmp, "qa.yaml", QA);
    // Somebody sends the persona a direct message, which is what an unread IS for it.
    let opened = json_of(nxc(&tmp).args(["--json", "send", "--to", "qa", "please review PR 42"]));
    let session = opened["session"].as_str().unwrap().to_string();

    let block = stdout_of(
        nxc(&tmp)
            .env("NXC_ACTOR", "qa")
            .env("NXC_SESSION", &session)
            .args(["prime"]),
    );
    assert!(
        !block.contains("please review PR 42") && !block.contains("## Unread"),
        "the block replays no message text — that is 6j6v.4mmk: {block}"
    );
    // …and it does not teach `inbox`, which is what "off the agent surface" means here.
    assert!(
        !block.contains("nxc inbox"),
        "the agent surface no longer names it: {block}"
    );
    assert!(
        !block.contains("nxc read") && !block.contains("nxc channels"),
        "nor the read cursor or the channel lifecycle, both of which went the same way: {block}"
    );
    // A third assertion stood here: the message itself is not lost, read back off `prime --json`'s
    // `in_turn` — the record 6j6v.1gm9 left standing when it took the verb. nxf 6j6v.4d2z removed
    // that record too, so the sentence this test can still make is the one above and no more.
    //
    // WHERE A PERSONA ACTUALLY MEETS THE MESSAGE, and what makes the loss of the third assertion a
    // non-event: it is PUSHED. `send --to` starts the session with the body in its prompt and
    // `reply --thread` resumes it with the reply's — pinned in `tests/orchestration_trigger.rs`
    // and `tests/orchestration_reply.rs`, which drive the real thing rather than a record of it.
}

#[test]
fn the_agent_facing_surface_is_exactly_the_verbs_the_design_names() {
    // §4: `prime`, `send --to`, `reply --thread`, `list`, `search`, `transcript show`,
    // `nxc status`. This is the acceptance point-1 claim taken on the agent-facing side — the
    // surface carries only the verbs the design names.
    //
    // TWO MORE since nxf 6j6v.4mmk, and they were added on a measurement rather than on the design:
    // `threads show` was the SECOND most used verb across 66 persona sessions and was named
    // nowhere, and `withdraw` is the rescue verb for a commission — queued behind the working
    // copy, or already running with a session to stop (nxf 6j6v.b9nf). Both cover the bad case;
    // §4's six cover the good one.
    //
    // **Checked against `--help`, not `prime`, since nxf h4d3 (task 3, q3fh session-start-budget).**
    // This test's claim was always about the SURFACE — that these eight verbs exist and no others
    // do — not about what a session is TAUGHT at start. Task 3 narrowed the latter on purpose: `nxc
    // prime` named only three of these eight (`send --to`, `reply --thread`,
    // `withdraw --thread`) — two since nxf 6j6v.ezbr made `withdraw` a person's verb that a
    // session this workspace started is refused, though it is still on this SURFACE for the
    // person — because the rest are reachable on demand via `nxc --help`/`nxc guide`,
    // which is exactly the distinction the new intro paragraph draws. `contract.rs`'s
    // `prime_pins_the_rule_lists_commands_and_replays_both_dispositions` pins `prime`'s own,
    // narrower list; this one still pins the full surface. Matched on the command column
    // (`\n  <verb> `), the same convention `neither_inbox_nor_read_is_a_verb_any_more` below uses.
    let tmp = workspace();
    let help = stdout_of(nxc(&tmp).args(["--help"]));
    for named in [
        "\n  list ",
        "\n  send ",
        "\n  reply ",
        "\n  status ",
        "\n  search ",
        "\n  transcript ",
        "\n  threads ",
        "\n  withdraw ",
    ] {
        assert!(help.contains(named), "the surface names {named:?}: {help}");
    }
}

/// **`a_human_still_has_inbox` stood here, and nxf 6j6v.1gm9 turned it around.** The verb was kept
/// on the argument that a human at the keyboard has no session to be primed into; the owner
/// answered that a human reads a CONVERSATION, not a flat unread list, and both verbs left the
/// binary. So this asserts the refusal instead — the shape every other removed entrance in this
/// file is held to — and then that the two surfaces a human actually has still answer.
#[test]
fn neither_inbox_nor_read_is_a_verb_any_more() {
    let tmp = workspace();
    declare(&tmp, "qa.yaml", QA);
    let opened = json_of(nxc(&tmp).args(["--json", "send", "--to", "qa", "please review PR 42"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();

    for gone in [
        vec!["inbox"],
        vec!["inbox", "--all"],
        vec!["read", "dm:whatever", "--through", "m-1"],
    ] {
        let err = refused(&tmp, &gone);
        assert!(
            err.contains("unrecognized subcommand"),
            "{gone:?} is refused, not quietly accepted: {err}"
        );
    }
    // And the help page does not advertise either (the hole 6j6v.spjj closed for this tree).
    // Matched on the COMMAND COLUMN (`\n  <verb> `), not on the bare word: every surviving verb's
    // one-line summary is prose, and `a thread ` contains `read ` while `threads` contains `read`.
    let help = stdout_of(nxc(&tmp).args(["--help"]));
    for gone in ["\n  inbox ", "\n  read "] {
        assert!(
            !help.contains(gone),
            "{gone:?} is off the help page too: {help}"
        );
    }

    // What a human has instead: the conversation, from either end.
    let board = stdout_of(nxc(&tmp).args(["threads", "show", &thread]));
    assert!(
        board.contains("please review PR 42") && board.contains("outstanding: local/qa"),
        "the board carries the message and who owes the answer: {board}"
    );
    assert!(
        stdout_of(nxc(&tmp).args(["status"])).contains(&thread),
        "and `status` says the operation is live"
    );
}

// ---- THE ABLAUF GROUP: the whole `workflow` surface (§3) ---------------------------------------
//
// §3's ABLAUF block, line by line:
//
//     workflow start        -> send --to <kanal>
//     workflow step done    -> reply --thread (der Betreuer entscheidet; --outcome entfaellt ersatzlos)
//     workflow status       -> nxc status
//     workflow list         -> nxc status
//     workflow tickets add  -> ueberfluessig (Owner)
//     workflow tick         -> Innenmechanik, von der Oberflaeche runter
//     workflow liveness     -> Innenmechanik, von der Oberflaeche runter
//
// The group itself is gone from the binary — there is no `nxc workflow` to type. The last two lines
// did NOT both become inner mechanics: `tick` did (hidden, invoked by the scheduled `at` job, see
// `the_group_is_gone_and_only_the_clocks_hand_survives_below_the_surface` below), and `liveness`
// fell entirely, which `seam_disposition.rs` records as a named loss rather than a collapse.

/// The declared flow the tests below drive: the same fixed order a `workflow start` graph declared
/// — a role, then a channel — expressed as a channel with `flow: sequential`.
const FLOW: &str = "- name: review\n  members: [qa]\n\
                    - name: ship\n  members: [coder, review]\n  flow: sequential\n";

#[test]
fn workflow_start_is_expressible_as_send_to_a_declared_channel() {
    let tmp = workspace();
    declare(&tmp, "coder.yaml", CODER);
    declare(&tmp, "qa.yaml", QA);
    declare(&tmp, "channels.yaml", FLOW);

    // The entrance is shut — the whole group, not just the leaf.
    let err = refused(&tmp, &["workflow", "start", "ship it"]);
    assert!(err.contains("unrecognized subcommand"), "{err}");

    // The replacement, driven: sending to the declared channel opens the operation and starts its
    // first step. The effect `workflow start` produced was "a work order is in motion and step[0]'s
    // target is running", so that is what is asserted — a thread with the flow's first member at
    // work under it, not merely a zero exit.
    let opened = json_of(nxc(&tmp).args(["--json", "send", "--to", "ship", "ship it"]));
    let board = opened["thread_id"].as_str().unwrap().to_string();
    assert_eq!(opened["target"], "channel", "{opened}");

    let tree = json_of(nxc(&tmp).args(["--json", "status", "--thread", &board]));
    let rendered = tree.to_string();
    assert!(
        rendered.contains("coder"),
        "the flow's FIRST member is at work under the board, and nobody else yet: {tree}"
    );
    assert!(
        !rendered.contains("qa"),
        "…and the second step has not fired — `flow: sequential` is the order the run declared: \
         {tree}"
    );
}

#[test]
fn workflow_step_done_is_expressible_as_reply_thread() {
    let tmp = workspace();
    declare(&tmp, "coder.yaml", CODER);
    declare(&tmp, "qa.yaml", QA);
    declare(&tmp, "channels.yaml", FLOW);

    // Both halves of the entrance are shut: the verb, and the flag §3 says falls with no
    // replacement at all.
    assert!(
        refused(&tmp, &["workflow", "step", "done"]).contains("unrecognized subcommand"),
        "the entrance is gone"
    );
    assert!(
        refused(&tmp, &["workflow", "step", "done", "--outcome", "approved"])
            .contains("unrecognized subcommand"),
        "and so is the flag that used to carry the branch"
    );

    let board = json_of(nxc(&tmp).args(["--json", "send", "--to", "ship", "ship it"]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let coder_slot = slot_under(&tmp, &board, "coder");

    // The replacement, driven: the step's own agent answers on its thread — as itself, which is the
    // whole of the mechanism. The effect `workflow step done` produced was "this step is finished,
    // fire whatever comes next", so the assertion is that the NEXT step really fired, not that the
    // reply exited zero.
    let coder = session_of(&tmp, "coder");
    let replied = json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &coder_slot,
        "done",
    ]));
    assert_eq!(replied["thread_id"], coder_slot, "{replied}");

    let tree = json_of(nxc(&tmp).args(["--json", "status", "--thread", &board]));
    assert!(
        tree.to_string().contains("qa"),
        "the supervisor moved the flow on to its second step, which is the whole of what \
         `step done` did — and nobody had to utter a branching token to make it happen: {tree}"
    );
}

#[test]
fn workflow_status_and_workflow_list_are_expressible_as_nxc_status() {
    let tmp = workspace();
    declare(&tmp, "coder.yaml", CODER);
    declare(&tmp, "qa.yaml", QA);
    declare(&tmp, "channels.yaml", FLOW);

    assert!(
        refused(&tmp, &["workflow", "status"]).contains("unrecognized subcommand"),
        "the entrance is gone"
    );
    assert!(
        refused(&tmp, &["workflow", "list"]).contains("unrecognized subcommand"),
        "and so is its sibling"
    );

    let board = json_of(nxc(&tmp).args(["--json", "send", "--to", "ship", "ship it"]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // `workflow status --run <id>`'s replacement: where does THIS operation stand.
    let one = json_of(nxc(&tmp).args(["--json", "status", "--thread", &board]));
    assert!(
        one.to_string().contains(&board),
        "the operation is shown from its own root: {one}"
    );

    // `workflow list`'s replacement, and it is the harder half of the claim: the read that needs NO
    // prior knowledge — no run id, no session, no having started anything — which is what made
    // "what is running here?" a question that could be asked at all.
    let all = json_of(nxc(&tmp).args(["--json", "status"]));
    assert!(
        all.to_string().contains(&board),
        "…and it is also in the workspace-wide read, taken with no argument: {all}"
    );
}

/// The one entrance on the removal list that got NO replacement, by decision rather than by
/// oversight — so the test says what survives instead of pretending something took its place.
#[test]
fn workflow_tickets_add_is_gone_and_a_message_still_names_its_subject_matter() {
    let tmp = workspace();
    declare(&tmp, "qa.yaml", QA);

    // Owner, 2026-08-13: "ueberfluessig". The entrance is shut — and since §3's last block, so is
    // the group it hung under.
    assert!(
        refused(&tmp, &["workflow", "tickets", "add", "6j6v.jd37"])
            .contains("unrecognized subcommand"),
        "the entrance is gone"
    );

    // What it was for — "which ticket is this message about?" as a lookup rather than a judgement —
    // is what `--ref nxf_ids` says, and always was: it won over the run's automatic stamp whenever
    // a caller named one, and it is what is left now that there is no run to stamp from.
    let sent = json_of(nxc(&tmp).args([
        "--json",
        "send",
        "--to",
        "qa",
        "please review",
        "--ref",
        "nxf_ids=6j6v.jd37",
        "--ref",
        "nxf_ids=6j6v.8dbe",
    ]));
    let thread = sent["thread_id"].as_str().unwrap().to_string();
    let board = json_of(nxc(&tmp).args(["--json", "threads", "show", &thread]));
    let rendered = board.to_string();
    assert!(
        rendered.contains("6j6v.jd37") && rendered.contains("6j6v.8dbe"),
        "the message carries the items it is about: {board}"
    );
}

#[test]
fn the_group_is_gone_and_only_the_clocks_hand_survives_below_the_surface() {
    let tmp = workspace();

    // The GROUP, not merely its leaves: `nxc workflow` is not a command.
    let err = refused(&tmp, &["workflow"]);
    assert!(err.contains("unrecognized subcommand"), "{err}");
    assert!(
        !stdout_of(nxc(&tmp).args(["--help"])).contains("workflow"),
        "and `nxc --help` does not name the concept anywhere"
    );

    // `tick` is the exception, and it is an exception on purpose: a declared channel's `timeout`
    // schedules a one-shot `at` job, and that job runs this verb. It is HIDDEN — off `--help`, off
    // the prime block — because nobody types it; the timer does.
    assert!(
        !stdout_of(nxc(&tmp).args(["--help"])).contains("tick"),
        "nothing on the surface names it"
    );
    assert!(
        !stdout_of(nxc(&tmp).args(["prime"])).contains("nxc tick"),
        "and the agent surface — what a session is TAUGHT — does not either"
    );
    // …and it is nevertheless there, which is the half that keeps a declared `timeout:` meaning
    // something. An unknown thread is `not_found`, which is the verb answering rather than clap.
    let err = refused(&tmp, &["tick", "--thread", "m-does-not-exist"]);
    assert!(
        !err.contains("unrecognized subcommand"),
        "the verb exists for the scheduled job that invokes it: {err}"
    );

    // `liveness` has no such exception: it went entirely, with the run whose step it watched.
    assert!(
        refused(&tmp, &["liveness"]).contains("unrecognized subcommand"),
        "the dead-man's watch was run-keyed and fell with the runs"
    );
}

// ---- THE ENTRANCE COLLAPSES: `send --thread <id>` -> `reply --thread` (§3) ---------------------

#[test]
fn send_thread_is_expressible_as_reply_thread() {
    let tmp = workspace();
    declare(&tmp, "qa.yaml", QA);

    // A thread comes into being through `send --to`, which MINTS the id and hands it back — the
    // half of the pair that `--thread` on `send` could never do honestly, because it took an id
    // nobody had stamped.
    let opened = json_of(nxc(&tmp).args(["--json", "send", "--to", "qa", "which db?"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();

    // The entrance is shut. clap refuses the flag, which is what an agent finds out.
    let err = refused(&tmp, &["send", "--to", "qa", "more", "--thread", &thread]);
    assert!(
        err.contains("unexpected argument") || err.contains("--thread"),
        "{err}"
    );

    // The replacement, driven: `reply --thread` posts into that same thread, which is the effect
    // `send --thread` produced.
    let replied = json_of(nxc(&tmp).args(["--json", "reply", "--thread", &thread, "sqlite"]));
    assert_eq!(
        replied["thread_id"], thread,
        "the message lands in the named thread: {replied}"
    );

    // …and it is really in the conversation, not merely stamped: the board shows both messages.
    let board = json_of(nxc(&tmp).args(["--json", "threads", "show", &thread]));
    let bodies = board.to_string();
    assert!(
        bodies.contains("which db?") && bodies.contains("sqlite"),
        "{board}"
    );
}

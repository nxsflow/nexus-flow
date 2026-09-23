//! **A conversation gets a name, and answering it stops needing an id** (nxf 6j6v.e76c, 6j6v.dq59).
//!
//! Both items are about what a HUMAN or an AGENT reads at one specific moment — the conversation
//! list of an app, and the message that wakes a session — so what is driven here is the whole path
//! that produces those two texts, not the units under them (`naming.rs` and `addressees.rs` carry
//! their own).

use std::sync::Mutex;

use assert_cmd::Command;
use nexus_chat::definitions::Definitions;
use nexus_chat::naming::Namer;
use nexus_chat::orchestration::Ctx;
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::worker::{TriggerRequest, Worker};
use tempfile::TempDir;

/// A worker that records what it was handed instead of spawning anything — the wake TEXT a session
/// is started with is `TriggerRequest::message`, which is the whole observable of 6j6v.dq59's
/// second half.
#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}

/// A namer that records the commissions instead of starting a process, and derives the message's
/// own first words — the observable of 6j6v.e76c's "the coordinator commissions the name".
#[derive(Default)]
struct RecordingNamer {
    commissioned: Mutex<Vec<(String, String)>>,
}

impl Namer for RecordingNamer {
    fn commission(&self, run: nexus_chat::naming::Naming<'_>) -> nexus_chat::error::Result<()> {
        self.commissioned
            .lock()
            .unwrap()
            .push((run.thread_id.to_string(), run.db_path.to_string()));
        Ok(())
    }
    fn derive(&self, body: &str) -> Option<String> {
        nexus_chat::naming::sanitize(body)
    }
}

/// A namer whose commission FAILS — `current_exe` unanswerable, `claude` missing, a fork that hit
/// the process limit; every one of them reaches the coordinator as an `Err` from this half.
///
/// It exists because "a broken namer must not fail `send`" is property 2 of `naming.rs` and was
/// asserted only by a doc comment (review of PR #452, Test Quality · Medium). A doc comment does not
/// red when somebody turns that `eprintln!` into a `?`.
struct FailingNamer;

impl Namer for FailingNamer {
    fn commission(&self, _run: nexus_chat::naming::Naming<'_>) -> nexus_chat::error::Result<()> {
        Err(nexus_chat::error::NxfError::io(
            "no naming run could be started",
        ))
    }
    fn derive(&self, _body: &str) -> Option<String> {
        None
    }
}

static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!("handle: {handle}\nsystem_prompt: do the thing\n"))
        .expect("test role parses")
}

fn ctx<'a>(
    defs: &'a Definitions,
    worker: &'a dyn Worker,
    namer: &'a dyn Namer,
    now: &'a str,
) -> Ctx<'a> {
    Ctx {
        now,
        origin: "local",
        actor: "human",
        session: None,
        hop: 0,
        defs,
        worker,
        timer: &NO_TIMER,
        namer,
        db_path: "/w/.nxs/db.sqlite",
        project_claude_md: None,
        module_primes: None,
        machines: None,
    }
}

// ---- 6j6v.e76c: the conversation has a name ----------------------------------------------------

#[test]
fn a_send_to_a_persona_commissions_a_name_for_the_thread_it_opened() {
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let namer = RecordingNamer::default();
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &namer, "2026-09-08T10:00:00Z"),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "Please review the retry loop in the sync driver",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send");
    assert_eq!(
        *namer.commissioned.lock().unwrap(),
        vec![(receipt.thread_id.clone(), "/w/.nxs/db.sqlite".to_string())],
        "the thread the coordinator OPENED is commissioned, with the workspace it lives in"
    );
    // …and the thread is a perfectly valid thread while nothing has named it yet, which is what
    // "the derivation may not hold up or fail a send" costs a reader.
    assert_eq!(
        store.thread_name(&receipt.thread_id).unwrap(),
        None,
        "nothing is named synchronously"
    );
}

#[test]
fn the_commissioned_run_names_the_thread_once_and_every_thread_read_carries_it() {
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let namer = RecordingNamer::default();
    let now = "2026-09-08T10:00:00Z";
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &namer, now),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "Please review the retry loop in the sync driver",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send");

    // What the commissioned run does, driven directly — it is a second process in production and a
    // call here, and the same function either way.
    let named = nexus_chat::surface::name_thread_from_its_opening_message(
        &ctx(&defs, &worker, &namer, now),
        &mut store,
        &receipt.thread_id,
    )
    .expect("name");
    assert!(named.named);
    assert_eq!(
        named.name.as_deref(),
        Some("Please review the retry loop in the"),
        "at most seven words, derived from the message that opened the thread"
    );

    // It rides on the read every thread surface is built on — which is the whole point, because an
    // app's conversation list could otherwise only show `dm:<24 hex>`.
    let quorum = store
        .thread_quorum(&receipt.thread_id, now)
        .unwrap()
        .expect("thread");
    assert_eq!(
        quorum.name.as_deref(),
        Some("Please review the retry loop in the")
    );
    let board = nexus_chat::facade::thread_board(
        &store,
        &receipt.thread_id,
        now,
        "local/human",
        nexus_chat::channel::ChannelPolicy::undeclared(),
    )
    .expect("board");
    assert_eq!(
        board.quorum.name.as_deref(),
        Some("Please review the retry loop in the")
    );

    // ONCE. A later message never renames it, because a name that moves under a reader is worse
    // than no name at all.
    let again = nexus_chat::facade::name_thread(
        &mut store,
        "2026-09-08T11:00:00Z",
        "local/human",
        &receipt.thread_id,
        "Something else entirely",
    )
    .expect("second attempt");
    assert!(!again.named, "the second attempt writes nothing");
    assert_eq!(
        again.name.as_deref(),
        Some("Please review the retry loop in the"),
        "and reports the name the thread already carries"
    );
}

#[test]
fn a_thread_nobody_could_name_stays_a_valid_thread() {
    // The namer declined (no model, no `claude`, an app that never asked for names): the receipt
    // says nothing was written, no error is raised, and every read renders the thread as it did
    // before names existed.
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let now = "2026-09-08T10:00:00Z";
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &nexus_chat::naming::DisabledNamer, now),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "Please review the retry loop",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send");
    let named = nexus_chat::surface::name_thread_from_its_opening_message(
        &ctx(&defs, &worker, &nexus_chat::naming::DisabledNamer, now),
        &mut store,
        &receipt.thread_id,
    )
    .expect("naming never fails the caller");
    assert!(!named.named);
    assert_eq!(named.name, None);
    assert_eq!(
        store
            .thread_quorum(&receipt.thread_id, now)
            .unwrap()
            .unwrap()
            .name,
        None
    );
}

// ---- 6j6v.dq59: the coordinator names the addressees at the wake -------------------------------

#[test]
fn the_commission_wake_says_the_thread_id_may_be_left_out() {
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let namer = RecordingNamer::default();
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &namer, "2026-09-08T10:00:00Z"),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "review the retry loop",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send");
    let seen = worker.seen.lock().unwrap();
    let wake = &seen.last().expect("one trigger").message;
    assert!(wake.starts_with("review the retry loop"), "{wake}");
    assert!(
        wake.contains("needs no thread id"),
        "one conversation is open, so the shorthand is offered: {wake}"
    );
    assert!(
        wake.contains(&format!("nxc reply --thread {}", receipt.thread_id)),
        "and the full form is spelled out, because these lines are meant to be run: {wake}"
    );
    assert!(
        wake.contains("the answer goes to local/human"),
        "the coordinator NAMES whom the answer reaches: {wake}"
    );
    assert!(wake.contains("--escalate"), "{wake}");
}

#[test]
fn a_session_with_two_open_conversations_is_told_to_name_the_thread() {
    // The owner's own case (2026-08-25): the coder owes its commissioner an answer AND has
    // consulted somebody who has not come back. The wake that resumes it must name both, each with
    // the verb it takes — and must NOT offer the id-free form, because the engine would have to
    // guess.
    let defs = Definitions::new(vec![role("coder"), role("frontend")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let namer = RecordingNamer::default();
    let now = "2026-09-08T10:00:00Z";
    let mut store = ChatStore::open_in_memory(1);
    // The human commissions the coder…
    let commission = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &namer, now),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "review the retry loop",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("commission");
    // …and the coder consults the frontend engineer, from its own session.
    let coder_session = commission.session.clone().expect("a session was minted");
    let mut coder_ctx = ctx(&defs, &worker, &namer, now);
    coder_ctx.actor = "coder";
    coder_ctx.session = Some(&coder_session);
    let consultation = nexus_chat::surface::send_to(
        &coder_ctx,
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "frontend",
            body: "which hook owns the retry?",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("consultation");

    let seen = worker.seen.lock().unwrap();
    let wake = &seen.last().expect("the frontend was started").message;
    // The FRONTEND has exactly one open conversation, so it gets the shorthand…
    assert!(wake.contains("needs no thread id"), "{wake}");

    // …while the CODER, asked for guidance right now, has two.
    drop(seen);
    let block = nexus_chat::addressees::render_for_a_wake(
        &nexus_chat::addressees::open_for(&store, "local/coder", None, now).expect("addressees"),
    );
    assert!(block.contains("2 conversations are open"), "{block}");
    assert!(
        !block.contains("needs no thread id"),
        "ambiguity must not come back through the wake text: {block}"
    );
    assert!(
        block.contains(&format!("nxc reply --thread {}", commission.thread_id)),
        "the commissioner's thread, by name and with the verb: {block}"
    );
    assert!(
        block.contains(&format!("nxc reply --thread {}", consultation.thread_id)),
        "and the consultation's: {block}"
    );
    assert!(block.contains("ask local/frontend back"), "{block}");
    assert!(block.contains("the answer goes to local/human"), "{block}");
}

#[test]
fn a_namer_that_cannot_start_its_run_does_not_fail_the_send() {
    // Property 2 of `naming.rs`: the derivation may not hold `send` up and may not fail it. Both
    // halves of that were asserted only by a doc comment until this (review of PR #452, Test
    // Quality · Medium) — `commission_thread_name` reports the error and carries on, and nothing
    // red if somebody turned that `eprintln!` into a `?`.
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let now = "2026-09-08T10:00:00Z";
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &FailingNamer, now),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "review the retry loop",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("a namer that cannot start its run is not a send that failed");

    // Everything the send owed happened: the thread is open, the message is in it, and the persona
    // was started. Only nobody worked out what to call it.
    assert_eq!(
        store.messages_in_thread(&receipt.thread_id).unwrap().len(),
        1
    );
    assert_eq!(worker.seen.lock().unwrap().len(), 1, "the persona ran");
    assert_eq!(store.thread_name(&receipt.thread_id).unwrap(), None);
    assert!(
        receipt.warnings.is_empty(),
        "a name was never OWED, so its failure is not a failed consequence either: {:?}",
        receipt.warnings
    );
}

#[test]
fn an_already_named_thread_is_never_derived_a_second_time() {
    // The once-only rule lives at the write, where a call site cannot forget it — but until the
    // review of PR #452 (Integrity & Robustness · High) it only discarded the RESULT, after a real
    // `claude` subprocess had already run. `nxc threads name <id>` is un-gated by opener on
    // purpose, so a caller could run it in a loop against one named thread and buy unbounded model
    // calls for no effect.
    #[derive(Default)]
    struct CountingNamer {
        derived: Mutex<usize>,
    }
    impl Namer for CountingNamer {
        fn commission(
            &self,
            _run: nexus_chat::naming::Naming<'_>,
        ) -> nexus_chat::error::Result<()> {
            Ok(())
        }
        fn derive(&self, body: &str) -> Option<String> {
            *self.derived.lock().unwrap() += 1;
            nexus_chat::naming::sanitize(body)
        }
    }

    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let namer = CountingNamer::default();
    let now = "2026-09-08T10:00:00Z";
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &namer, now),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "review the retry loop",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send");
    let mut run = || {
        nexus_chat::surface::name_thread_from_its_opening_message(
            &ctx(&defs, &worker, &namer, now),
            &mut store,
            &receipt.thread_id,
        )
        .expect("naming never fails the caller")
    };
    assert!(run().named, "the first run names it");
    for _ in 0..5 {
        let again = run();
        assert!(!again.named);
        assert_eq!(
            again.name.as_deref(),
            Some("review the retry loop"),
            "and reports the name it already has, so the receipt shape is unchanged"
        );
    }
    assert_eq!(
        *namer.derived.lock().unwrap(),
        1,
        "the model is asked ONCE, however often the verb is run"
    );
}

#[test]
fn a_caller_that_may_not_read_the_thread_may_not_have_it_named_either() {
    // The read this makes is a thread-content read like any other, and it hands what it finds to an
    // external model — so it goes through the same gate `nxc threads show` does (review of PR #452,
    // Integrity & Robustness · Medium). A stranger to a members-only channel is refused BEFORE the
    // opening message is read.
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    let worker = RecordingWorker::default();
    let namer = RecordingNamer::default();
    let now = "2026-09-08T10:00:00Z";
    let mut store = ChatStore::open_in_memory(1);
    let receipt = nexus_chat::surface::send_to(
        &ctx(&defs, &worker, &namer, now),
        &mut store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "coder",
            body: "the credentials rotate on friday",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send");

    let mut stranger = ctx(&defs, &worker, &namer, now);
    stranger.actor = "stranger";
    let refused = nexus_chat::surface::name_thread_from_its_opening_message(
        &stranger,
        &mut store,
        &receipt.thread_id,
    )
    .expect_err("a stranger to the conversation is refused");
    assert_eq!(refused.kind.as_str(), "forbidden", "{refused:?}");
    assert_eq!(
        store.thread_name(&receipt.thread_id).unwrap(),
        None,
        "and nothing was named on the way to the refusal"
    );
}

// ---- both verbs through the real binary --------------------------------------------------------
//
// The CLI adapter is its own layer and had no black-box coverage at all (review of PR #452, Test
// Quality · Medium-High): `Option<&str>` for `--thread`, the `parent_thread_of` lookup that supplies
// `standing_in`, the derivation `threads name` runs without a name argument, and how a refusal comes
// back as an exit code. A wiring bug in any of them would have shipped green under the seam tests
// above.

const CLI_NOW: &str = "2026-09-08T10:00:00Z";

fn cli_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    for handle in ["coder", "frontend"] {
        std::fs::write(
            personas.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    tmp
}

/// The human at a terminal: no session, so nothing supplies `standing_in`.
fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "human")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", CLI_NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_TIMER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn as_persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = nxc(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn json_of(cmd: &mut Command) -> serde_json::Value {
    let out = cmd.assert().success();
    serde_json::from_slice(&out.get_output().stdout).expect("valid json")
}

#[test]
fn the_cli_derives_a_name_for_a_thread_and_leaves_an_already_named_one_alone() {
    let tmp = cli_workspace();
    let sent = json_of(nxc(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--no-ref",
        "Please review the retry loop in the sync driver",
    ]));
    let thread = sent["thread_id"].as_str().unwrap().to_string();

    // No name argument: the verb derives one through the namer this process resolves — `dry` here
    // (`nxs_test_support::cargo_bin` pins it, so no suite can reach a real model by forgetting a
    // line), which answers with the message's own first words.
    let named = json_of(nxc(&tmp).args(["--json", "threads", "name", &thread]));
    assert_eq!(named["named"], true, "{named}");
    assert_eq!(
        named["name"], "Please review the retry loop in the",
        "{named}"
    );

    // Run it again — the once-only rule, through the binary. The receipt reports the name it has
    // and `named: false`, and it exits 0, because a retried commission must stay idempotent.
    let again = json_of(nxc(&tmp).args(["--json", "threads", "name", &thread]));
    assert_eq!(again["named"], false, "{again}");
    assert_eq!(
        again["name"], "Please review the retry loop in the",
        "{again}"
    );

    // The human-readable rendering says so in words — the plain `{name}` print site.
    let plain = nxc(&tmp)
        .args(["threads", "name", &thread])
        .assert()
        .success();
    let plain = String::from_utf8(plain.get_output().stdout.clone()).unwrap();
    assert!(
        plain.contains("Please review the retry loop in the (already named — left as it is)"),
        "{plain}"
    );

    // …and the name rides on the list read every surface is built on.
    let listed = nxc(&tmp)
        .args(["threads", "list", "--consumer", "local/coder"])
        .assert()
        .success();
    let listed = String::from_utf8(listed.get_output().stdout.clone()).unwrap();
    assert!(
        listed.contains("Please review the retry loop in the"),
        "{listed}"
    );

    // A thread nobody minted is `not_found`, and that reaches the shell as a failing exit code
    // rather than as a silently created register.
    let missing = nxc(&tmp)
        .args(["threads", "name", "t-nothing"])
        .assert()
        .failure();
    let missing = String::from_utf8(missing.get_output().stderr.clone()).unwrap();
    assert!(missing.contains("no such thread"), "{missing}");
}

#[test]
fn the_cli_answers_without_a_thread_id_while_exactly_one_is_open() {
    let tmp = cli_workspace();
    let sent = json_of(nxc(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--no-ref",
        "review the retry loop",
    ]));
    let commission = sent["thread_id"].as_str().unwrap().to_string();
    let coder = sent["session"].as_str().unwrap().to_string();

    // ONE conversation is open for the coder — the obligation it was commissioned with — so the
    // bare form resolves to it. This is the whole of 6j6v.dq59's first half, through the adapter.
    let answered = json_of(as_persona(&tmp, "coder", &coder).args(["--json", "reply", "done"]));
    assert_eq!(answered["posted"], true, "{answered}");
    assert_eq!(answered["thread_id"], commission, "{answered}");

    // The obligation is discharged, and the bare form still resolves — to the SAME thread, because
    // the coder is STANDING in it. That third shape is the one no register produces: it comes from
    // `parent_thread_of` over the session, which is the CLI wiring this file exists to drive.
    let follow_up =
        json_of(as_persona(&tmp, "coder", &coder).args(["--json", "reply", "one more thing"]));
    assert_eq!(follow_up["thread_id"], commission, "{follow_up}");

    // A caller that is standing nowhere and owes nothing has nothing to resolve — refused, and the
    // refusal points at the id-bearing form rather than guessing.
    let mut stranger = nxc(&tmp);
    stranger.env("NXC_ACTOR", "frontend");
    let nothing_open = stranger.args(["reply", "hello?"]).assert().failure();
    let nothing_open = String::from_utf8(nothing_open.get_output().stderr.clone()).unwrap();
    assert!(
        nothing_open.contains("nxc reply --thread <id>"),
        "{nothing_open}"
    );
}

#[test]
fn the_cli_refuses_the_id_free_form_when_two_conversations_are_open_and_names_them() {
    // The owner's own case: the coder owes its commissioner an answer AND has consulted somebody
    // who has not come back. The engine would have to guess, so it refuses — and the refusal names
    // both, each with the verb that answers it.
    let tmp = cli_workspace();
    let sent = json_of(nxc(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--no-ref",
        "review the retry loop",
    ]));
    let commission = sent["thread_id"].as_str().unwrap().to_string();
    let coder = sent["session"].as_str().unwrap().to_string();
    let consulted = json_of(as_persona(&tmp, "coder", &coder).args([
        "--json",
        "send",
        "--to",
        "frontend",
        "--no-ref",
        "which hook owns the retry?",
    ]));
    let consultation = consulted["thread_id"].as_str().unwrap().to_string();
    let frontend = consulted["session"].as_str().unwrap().to_string();

    let refused = as_persona(&tmp, "coder", &coder)
        .args(["reply", "done"])
        .assert()
        .failure();
    let stderr = String::from_utf8(refused.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("2 conversations are open"), "{stderr}");
    assert!(
        stderr.contains(&format!("nxc reply --thread {commission}")),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("nxc reply --thread {consultation}")),
        "{stderr}"
    );

    // …and the id-BEARING form is always allowed AS A FORM, which is what this item decides. What
    // it does not decide is whether the answer itself is true, and since nxf 6j6v.hw2t a second
    // rule stands beside this one and says it is not: the coder is waiting on a round it
    // commissioned itself, so "done" is a claim it cannot make yet. THIS TEST ASSERTED THE
    // OPPOSITE — the same call, expected to post — and the change is deliberate rather than a
    // regression: the state it was written in is exactly the false-completion 6j6v.hw2t refuses.
    // Naming a thread is still never what gets refused here; the refusal is about the round, and
    // it says so.
    let too_early = as_persona(&tmp, "coder", &coder)
        .args(["reply", "--thread", &commission, "done"])
        .assert()
        .failure();
    let too_early = String::from_utf8(too_early.get_output().stderr.clone()).unwrap();
    assert!(
        too_early.contains("waiting on a round you commissioned yourself"),
        "the id-bearing form is refused for the ROUND's sake, not the id's: {too_early}"
    );
    // Said as a NEGATIVE too, because the positive alone leaves the claim to be inferred from one
    // message not being another (review of PR #460, Test Quality #3): whatever else changes here,
    // naming the thread must never be what gets refused — that is this test's own subject.
    assert!(
        !too_early.contains("conversations are open"),
        "…and NOT for the ambiguity this test is about, which naming the thread already settled: \
         {too_early}"
    );

    // The consultation comes back, and with it the last reason to refuse. Nothing is reset and
    // nobody says "I am done waiting" — the register empties and the rule stops holding.
    json_of(as_persona(&tmp, "frontend", &frontend).args([
        "--json",
        "reply",
        "--thread",
        &consultation,
        "the sync driver's own hook owns it",
    ]));
    let answered = json_of(as_persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &commission,
        "done",
    ]));
    assert_eq!(answered["posted"], true, "{answered}");
}

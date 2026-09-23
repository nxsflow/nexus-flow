//! **The executing machine** (nxf 6j6v.1c6k): a persona chat runs on exactly ONE machine, every
//! other machine sees it and starts nothing, and the designated machine picks it up after the pull.
//! See `docs/specs/E4-executing-machine.md`.
//!
//! Two replicas of one stream in one process: `laptop` and `studio`, each its own in-memory store
//! with its own signing key, exchanging ops the way a relay would (`export` → `apply`) and trusting
//! each other's key the way `nxs sync trust add` does. Each replica gets its own [`FixedMachines`],
//! which is the host's half — which machine this is, who is online, the per-machine claims.

use std::sync::Mutex;

use nexus_chat::definitions::Definitions;
use nexus_chat::machine::{FixedMachines, MachineRef, MachineSeen, Machines};
use nexus_chat::orchestration::{self, Ctx};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker};

const NOW: &str = "2026-09-22T10:00:00Z";

#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl RecordingWorker {
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    fn last(&self) -> TriggerRequest {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a trigger")
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
}

static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn coder(machine: Option<&str>) -> RoleDecl {
    let line = machine
        .map(|m| format!("machine: {m}\n"))
        .unwrap_or_default();
    serde_yaml::from_str(&format!(
        "handle: coder\nsystem_prompt: You are coder.\n{line}"
    ))
    .expect("role parses")
}

fn defs(machine: Option<&str>) -> Definitions {
    Definitions::new(vec![coder(machine)], vec![]).expect("definitions")
}

fn laptop() -> MachineRef {
    MachineRef {
        machine_id: "01jlaptop".into(),
        name: "laptop".into(),
    }
}

fn studio() -> MachineRef {
    MachineRef {
        machine_id: "01jstudio".into(),
        name: "studio".into(),
    }
}

fn seen(m: &MachineRef, online: bool, age: u64) -> MachineSeen {
    MachineSeen {
        machine_id: m.machine_id.clone(),
        name: m.name.clone(),
        online,
        age_secs: age,
    }
}

/// Both machines online, as a relay would report them.
fn both_online() -> Vec<MachineSeen> {
    vec![seen(&laptop(), true, 4), seen(&studio(), true, 30)]
}

fn machines(here: MachineRef, presence: Vec<MachineSeen>) -> FixedMachines {
    FixedMachines::new(Some(here), Ok(presence))
}

fn ctx<'a>(
    defs: &'a Definitions,
    worker: &'a dyn Worker,
    machines: Option<&'a dyn Machines>,
    actor: &'a str,
    session: Option<&'a str>,
    db_path: &'a str,
) -> Ctx<'a> {
    Ctx {
        now: NOW,
        origin: "o",
        actor,
        session,
        hop: 0,
        defs,
        worker,
        timer: &NO_TIMER,
        namer: &nexus_chat::naming::DryNamer,
        db_path,
        project_claude_md: None,
        module_primes: None,
        machines,
    }
}

fn send<'a>(body: &'a str, machine: Option<&'a str>) -> SendToRequest<'a> {
    SendToRequest {
        to: "coder",
        body,
        refs: SendToRefs::ExplicitlyNone,
        machine,
    }
}

fn reply<'a>(thread: &'a str, body: &'a str, machine: Option<&'a str>) -> ReplyThreadRequest<'a> {
    ReplyThreadRequest {
        thread,
        body,
        escalate: false,
        needs_rework: false,
        accept: false,
        machine,
    }
}

/// `to` pulls everything `from` holds, and trusts `from`'s key — the state after
/// `nxs sync trust add` and one pass.
fn pull_trusting(from: &ChatStore, to: &mut ChatStore) {
    to.trust_key(from.key_id(), "peer", NOW).unwrap();
    to.apply(&from.export());
}

#[test]
fn a_chat_started_here_runs_here_and_names_this_machine() {
    let d = defs(None);
    let w = RecordingWorker::default();
    let m = machines(laptop(), both_online());
    let mut store = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        send("Add a --since flag.", None),
    )
    .unwrap();
    assert!(r.spawned, "{r:?}");
    assert_eq!(r.handed_to, None);
    assert_eq!(w.count(), 1);
    let chat = store.thread_machine(&r.thread_id).unwrap().unwrap();
    assert_eq!((chat.machine_id.as_str(), chat.acts), ("01jlaptop", true));
    assert!(
        m.claimed(&r.message_id),
        "the order is claimed on this machine, so the service's pickup does not start it again"
    );
}

#[test]
fn a_host_that_names_no_machine_records_nothing_and_starts_here() {
    let d = defs(Some("studio"));
    let w = RecordingWorker::default();
    let mut store = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &w, None, "alice", None, "/laptop.db"),
        &mut store,
        send("go", None),
    )
    .unwrap();
    assert!(r.spawned);
    assert_eq!(store.thread_machine(&r.thread_id).unwrap(), None);
}

#[test]
fn a_chat_for_another_machine_starts_nothing_here_and_says_where() {
    let d = defs(Some("studio"));
    let w = RecordingWorker::default();
    let m = machines(laptop(), both_online());
    let mut store = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        send("go", None),
    )
    .unwrap();
    assert_eq!(
        w.count(),
        0,
        "nothing starts on the machine that is not designated"
    );
    assert!(!r.spawned);
    assert_eq!(r.session, None);
    let to = r.handed_to.expect("the receipt names the machine");
    assert_eq!(to.machine_id, "01jstudio");
    assert_eq!(to.online, Some(true));
    assert_eq!(
        store
            .thread_machine(&r.thread_id)
            .unwrap()
            .unwrap()
            .machine_id,
        "01jstudio"
    );
    assert!(
        store
            .thread_quorum(&r.thread_id, NOW)
            .unwrap()
            .unwrap()
            .outstanding
            .contains(&"o/coder".to_string()),
        "the chat still owes the persona's answer — that is what the pickup reads"
    );
}

#[test]
fn a_chat_for_an_absent_machine_asks_and_writes_nothing() {
    let d = defs(Some("studio"));
    let w = RecordingWorker::default();
    let m = machines(
        laptop(),
        vec![seen(&laptop(), true, 4), seen(&studio(), false, 7200)],
    );
    let mut store = ChatStore::open_in_memory(1);
    let before = store.export().len();
    let err = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        send("go", None),
    )
    .unwrap_err();
    assert!(
        err.msg.contains("studio (01jstudio), which is not online"),
        "{}",
        err.msg
    );
    assert!(
        err.msg.contains("laptop (01jlaptop"),
        "offers the online ones: {}",
        err.msg
    );
    assert_eq!(store.export().len(), before, "asking writes nothing");
    assert_eq!(w.count(), 0);

    // The answer is the same call with the machine named: this one, which is online.
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        send("go", Some("laptop")),
    )
    .unwrap();
    assert!(r.spawned && r.handed_to.is_none());
}

#[test]
fn the_designated_machine_picks_the_chat_up_after_the_pull_exactly_once() {
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let mut stu = ChatStore::open_in_memory(2);

    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("Add a --since flag.", Some("studio")),
    )
    .unwrap();
    assert_eq!(lw.count(), 0);

    pull_trusting(&lap, &mut stu);
    let sctx = ctx(&d, &sw, Some(&sm), "service", None, "/studio.db");
    let first = orchestration::pick_up(&sctx, &mut stu).unwrap();
    assert_eq!(first.served.len(), 1, "{first:?}");
    assert_eq!(first.served[0].thread, r.thread_id);
    assert!(first.served[0].started);
    assert_eq!(sw.count(), 1);
    assert!(sw.last().message.contains("Add a --since flag."));

    let again = orchestration::pick_up(&sctx, &mut stu).unwrap();
    assert!(again.served.is_empty(), "{again:?}");
    assert_eq!(sw.count(), 1, "once per message, never on arrival");

    // The laptop sees the chat and starts nothing, however often it looks.
    let lctx = ctx(&d, &lw, Some(&lm), "service", None, "/laptop.db");
    assert!(orchestration::pick_up(&lctx, &mut lap)
        .unwrap()
        .served
        .is_empty());
    assert_eq!(lw.count(), 0);
}

#[test]
fn an_order_from_an_origin_nobody_here_trusts_is_not_picked_up() {
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let mut stu = ChatStore::open_in_memory(2);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("rm -rf everything", Some("studio")),
    )
    .unwrap();
    stu.apply(&lap.export()); // pulled, but the laptop's key is not on studio's trust list
    let sctx = ctx(&d, &sw, Some(&sm), "service", None, "/studio.db");
    let report = orchestration::pick_up(&sctx, &mut stu).unwrap();
    assert!(report.served.is_empty(), "{report:?}");
    assert_eq!(sw.count(), 0);
    assert!(!sm.claimed(&r.message_id));

    // Trusting the key is what lets the same order act — nothing else changed.
    stu.trust_key(lap.key_id(), "laptop", NOW).unwrap();
    assert_eq!(
        orchestration::pick_up(&sctx, &mut stu)
            .unwrap()
            .served
            .len(),
        1
    );
}

#[test]
fn two_replicas_of_one_stream_on_one_machine_start_it_once() {
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online()); // ONE machine, shared by both replicas
    let mut lap = ChatStore::open_in_memory(1);
    let mut clone_a = ChatStore::open_in_memory(2);
    let mut clone_b = ChatStore::open_in_memory(3);
    surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("go", Some("studio")),
    )
    .unwrap();
    pull_trusting(&lap, &mut clone_a);
    pull_trusting(&lap, &mut clone_b);
    let a =
        orchestration::pick_up(&ctx(&d, &sw, Some(&sm), "s", None, "/a.db"), &mut clone_a).unwrap();
    let b =
        orchestration::pick_up(&ctx(&d, &sw, Some(&sm), "s", None, "/b.db"), &mut clone_b).unwrap();
    assert_eq!(a.served.len() + b.served.len(), 1, "{a:?} {b:?}");
    assert_eq!(sw.count(), 1);
}

#[test]
fn a_reply_from_elsewhere_resumes_the_persona_on_its_machine() {
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let mut stu = ChatStore::open_in_memory(2);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("first", Some("studio")),
    )
    .unwrap();
    pull_trusting(&lap, &mut stu);
    let sctx = ctx(&d, &sw, Some(&sm), "service", None, "/studio.db");
    orchestration::pick_up(&sctx, &mut stu).unwrap();
    let session = sw.last().internal_session.clone();

    // The persona answers on the studio…
    surface::reply_in_thread(
        &ctx(&d, &sw, Some(&sm), "coder", Some(&session), "/studio.db"),
        &mut stu,
        reply(&r.thread_id, "done, what next?", None),
    )
    .unwrap();
    pull_trusting(&stu, &mut lap);

    // …and the person answers from the laptop: posted there, started nowhere there.
    let rr = surface::reply_in_thread(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        reply(&r.thread_id, "now the tests", None),
    )
    .unwrap();
    assert_eq!(lw.count(), 0);
    assert_eq!(
        rr.handed_to.map(|m| m.machine_id).as_deref(),
        Some("01jstudio")
    );

    pull_trusting(&lap, &mut stu);
    let report = orchestration::pick_up(&sctx, &mut stu).unwrap();
    assert_eq!(report.served.len(), 1, "{report:?}");
    assert!(
        !report.served[0].started,
        "the persona's own session is resumed"
    );
    assert_eq!(sw.count(), 2);
    let resumed = sw.last();
    assert_eq!(resumed.internal_session, session);
    assert!(resumed.message.contains("now the tests"));
}

#[test]
fn a_chat_handed_to_another_machine_starts_fresh_there() {
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let mut stu = ChatStore::open_in_memory(2);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("first", None),
    )
    .unwrap();
    assert_eq!(lw.count(), 1, "started on the laptop, where it was written");
    let session = lw.last().internal_session.clone();
    surface::reply_in_thread(
        &ctx(&d, &lw, Some(&lm), "coder", Some(&session), "/laptop.db"),
        &mut lap,
        reply(&r.thread_id, "first part done", None),
    )
    .unwrap();
    let rr = surface::reply_in_thread(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        reply(&r.thread_id, "carry on over there", Some("studio")),
    )
    .unwrap();
    assert_eq!(lw.count(), 1, "handing over starts nothing here");
    assert_eq!(
        rr.handed_to.map(|m| m.machine_id).as_deref(),
        Some("01jstudio")
    );

    pull_trusting(&lap, &mut stu);
    let report =
        orchestration::pick_up(&ctx(&d, &sw, Some(&sm), "s", None, "/studio.db"), &mut stu)
            .unwrap();
    assert_eq!(report.served.len(), 1, "{report:?}");
    assert!(
        report.served[0].started,
        "no session of the persona lives here yet"
    );
    let fresh = sw.last();
    assert!(
        fresh.message.contains("carry on over there"),
        "{}",
        fresh.message
    );
    assert!(
        !fresh.message.contains("first part done"),
        "only what is owed since the persona's last answer is handed over"
    );
    assert!(
        fresh.message.contains(&format!(
            "read it first: `nxc threads show {}`",
            r.thread_id
        )),
        "a fresh session is told where the conversation so far is: {}",
        fresh.message
    );
}

#[test]
fn a_reply_into_a_chat_whose_machine_is_gone_asks() {
    let d = defs(None);
    let lw = RecordingWorker::default();
    let lm = machines(laptop(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("first", Some("studio")),
    )
    .unwrap();
    let gone = machines(
        laptop(),
        vec![seen(&laptop(), true, 4), seen(&studio(), false, 9000)],
    );
    let before = lap.export().len();
    let err = surface::reply_in_thread(
        &ctx(&d, &lw, Some(&gone), "alice", None, "/laptop.db"),
        &mut lap,
        reply(&r.thread_id, "still there?", None),
    )
    .unwrap_err();
    assert!(err.msg.contains("not online"), "{}", err.msg);
    assert_eq!(lap.export().len(), before, "asking writes nothing");
}

#[test]
fn a_session_cannot_send_a_chat_to_another_machine() {
    // A persona's own sub-chat answers back into that persona's session, which lives on ITS
    // machine; handing the sub-chat elsewhere would leave the answer with nobody to wake. So a
    // session's chat runs where the session runs, whatever the target persona declares.
    let reviewer: RoleDecl =
        serde_yaml::from_str("handle: reviewer\nsystem_prompt: You review.\nmachine: studio\n")
            .unwrap();
    let d = Definitions::new(vec![coder(None), reviewer], vec![]).unwrap();
    let w = RecordingWorker::default();
    let m = machines(laptop(), both_online());
    let mut store = ChatStore::open_in_memory(1);
    let caller = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        send("go", Some("laptop")),
    )
    .unwrap();
    let session = caller.session.unwrap();
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "coder", Some(&session), "/laptop.db"),
        &mut store,
        SendToRequest {
            to: "reviewer",
            ..send("sub-task", None)
        },
    )
    .unwrap();
    assert!(r.handed_to.is_none() && r.spawned, "{r:?}");
    assert!(surface::send_to(
        &ctx(&d, &w, Some(&m), "coder", Some(&session), "/laptop.db"),
        &mut store,
        SendToRequest {
            to: "reviewer",
            ..send("sub-task", Some("studio"))
        },
    )
    .is_err());
}

#[test]
fn a_chat_redirected_by_an_origin_nobody_trusts_is_not_picked_up_there() {
    // The order itself is trusted — the laptop wrote it — but the register that would send it to
    // the studio was written by a third replica nobody trusts. Authority follows the op that wrote
    // the register, never its value.
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let mut stu = ChatStore::open_in_memory(2);
    let mut evil = ChatStore::open_in_memory(3);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("go", None),
    )
    .unwrap();
    evil.apply(&lap.export());
    evil.set_thread_machine(&r.thread_id, "01jstudio", "o/alice");
    pull_trusting(&lap, &mut stu);
    stu.apply(&evil.export());
    assert_eq!(
        stu.thread_machine(&r.thread_id)
            .unwrap()
            .unwrap()
            .machine_id,
        "01jstudio",
        "the register converged on the redirect…"
    );
    let report =
        orchestration::pick_up(&ctx(&d, &sw, Some(&sm), "s", None, "/studio.db"), &mut stu)
            .unwrap();
    assert!(
        report.served.is_empty(),
        "…and nothing acts on it: {report:?}"
    );
    assert_eq!(sw.count(), 0);
    assert!(stu.orders_owed("01jstudio").unwrap().is_empty());
}

#[test]
fn the_orders_owed_are_the_messages_after_the_personas_last_answer() {
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(laptop(), both_online());
    let sm = machines(studio(), both_online());
    let mut lap = ChatStore::open_in_memory(1);
    let mut stu = ChatStore::open_in_memory(2);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("go", Some("studio")),
    )
    .unwrap();
    pull_trusting(&lap, &mut stu);
    assert_eq!(
        stu.orders_owed("01jstudio").unwrap(),
        vec![r.message_id.clone()]
    );
    assert!(stu.orders_owed("01jlaptop").unwrap().is_empty());
    orchestration::pick_up(&ctx(&d, &sw, Some(&sm), "s", None, "/studio.db"), &mut stu).unwrap();
    let session = sw.last().internal_session.clone();
    surface::reply_in_thread(
        &ctx(&d, &sw, Some(&sm), "coder", Some(&session), "/studio.db"),
        &mut stu,
        reply(&r.thread_id, "done", None),
    )
    .unwrap();
    assert!(stu.orders_owed("01jstudio").unwrap().is_empty(), "answered");
}

/// What `nxs` does for a workspace bound to no stream: this machine, and no claims recorded —
/// nothing is ever picked up there, so there is nothing for a claim to guard against.
#[derive(Debug)]
struct Unbound(MachineRef);

impl Machines for Unbound {
    fn here(&self) -> Option<MachineRef> {
        Some(self.0.clone())
    }
    fn presence(&self, _db: &str) -> Result<Vec<MachineSeen>, String> {
        Err("bound to no stream".into())
    }
    fn claim_order(&self, _db: &str, _id: &str) -> Result<bool, String> {
        Ok(true)
    }
    fn release_order(&self, _db: &str, _id: &str) {}
    fn claim_thread(&self, _t: &str, holder: &str) -> Result<String, String> {
        Ok(holder.to_string())
    }
}

#[test]
fn a_second_reply_before_the_persona_answers_is_handed_over_alone_even_with_nothing_claimed() {
    let d = defs(None);
    let w = RecordingWorker::default();
    let m = Unbound(laptop());
    let mut store = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/local.db"),
        &mut store,
        send("first", None),
    )
    .unwrap();
    for body in ["one more thing", "and another"] {
        surface::reply_in_thread(
            &ctx(&d, &w, Some(&m), "alice", None, "/local.db"),
            &mut store,
            reply(&r.thread_id, body, None),
        )
        .unwrap();
    }
    assert_eq!(w.count(), 3);
    let last = w.last().message;
    assert!(last.starts_with("and another"), "{last}");
    assert!(!last.contains("one more thing"), "delivered twice: {last}");
    assert!(!last.contains("first"), "delivered twice: {last}");
}

#[test]
fn a_hand_back_into_a_chat_on_its_machine_still_announces_itself() {
    let d = defs(None);
    let w = RecordingWorker::default();
    let m = machines(laptop(), both_online());
    let mut store = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        send("first", None),
    )
    .unwrap();
    surface::reply_in_thread(
        &ctx(&d, &w, Some(&m), "alice", None, "/laptop.db"),
        &mut store,
        ReplyThreadRequest {
            escalate: true,
            ..reply(&r.thread_id, "I cannot decide this", None)
        },
    )
    .unwrap();
    assert_eq!(w.count(), 2);
    assert!(
        w.last()
            .message
            .starts_with(nexus_chat::channel::ESCALATION_NOTICE),
        "{}",
        w.last().message
    );
}

#[test]
fn a_machine_chosen_while_it_is_offline_gets_the_chat_when_it_is_back() {
    // "Late, not lost" (spec §2.2, row 4): a person who names a machine that is not online is not
    // asked again — the chat is written, handed over, and waits in the log until that machine
    // pulls it in.
    let d = defs(None);
    let (lw, sw) = (RecordingWorker::default(), RecordingWorker::default());
    let lm = machines(
        laptop(),
        vec![seen(&laptop(), true, 4), seen(&studio(), false, 9000)],
    );
    let mut lap = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &lw, Some(&lm), "alice", None, "/laptop.db"),
        &mut lap,
        send("for when you are back", Some("studio")),
    )
    .unwrap();
    assert_eq!(lw.count(), 0, "nothing starts on the laptop");
    let to = r.handed_to.clone().expect("handed over, not refused");
    assert_eq!(
        (to.machine_id.as_str(), to.online, to.age_secs),
        ("01jstudio", Some(false), Some(9000))
    );
    let chat = lap.thread_machine(&r.thread_id).unwrap().unwrap();
    assert_eq!((chat.machine_id.as_str(), chat.acts), ("01jstudio", true));
    assert_eq!(
        lap.acting_messages_in_thread(&r.thread_id).unwrap().len(),
        1,
        "the message is posted"
    );

    // The studio comes back, pulls, and picks it up.
    let mut stu = ChatStore::open_in_memory(2);
    pull_trusting(&lap, &mut stu);
    let sm = machines(studio(), both_online());
    let report =
        orchestration::pick_up(&ctx(&d, &sw, Some(&sm), "s", None, "/studio.db"), &mut stu)
            .unwrap();
    assert_eq!(report.served.len(), 1, "{report:?}");
    assert!(sw.last().message.contains("for when you are back"));
}

/// A host whose claims say another replica on this machine already serves every chat — the
/// second clone of a stream, standing where the first one got to the chat first.
#[derive(Debug)]
struct ServedElsewhere(MachineRef);

impl Machines for ServedElsewhere {
    fn here(&self) -> Option<MachineRef> {
        Some(self.0.clone())
    }
    fn presence(&self, _db: &str) -> Result<Vec<MachineSeen>, String> {
        Ok(both_online())
    }
    fn claim_order(&self, _db: &str, _id: &str) -> Result<bool, String> {
        Ok(true)
    }
    fn release_order(&self, _db: &str, _id: &str) {}
    fn claim_thread(&self, _t: &str, _holder: &str) -> Result<String, String> {
        Ok("/the-other-clone.db".to_string())
    }
}

#[test]
fn a_reply_in_a_replica_that_does_not_serve_the_chat_wakes_nobody_there() {
    // The narrow case the reply fallback's second guard exists for: the chat runs on THIS machine,
    // the caller is not the persona, and serving finds nothing to do here — because another clone
    // of the stream serves it. Without the guard, `reply --thread`'s return-address fallback would
    // resume the persona's session in THIS replica, around the claims.
    let d = defs(None);
    let w = RecordingWorker::default();
    let m = ServedElsewhere(laptop());
    let mut store = ChatStore::open_in_memory(1);
    let r = surface::send_to(
        &ctx(&d, &w, Some(&m), "alice", None, "/this-clone.db"),
        &mut store,
        send("first", None),
    )
    .unwrap();
    assert_eq!(w.count(), 1);
    let session = w.last().internal_session.clone();
    surface::reply_in_thread(
        &ctx(&d, &w, Some(&m), "coder", Some(&session), "/this-clone.db"),
        &mut store,
        reply(&r.thread_id, "done", None),
    )
    .unwrap();
    let rr = surface::reply_in_thread(
        &ctx(&d, &w, Some(&m), "alice", None, "/this-clone.db"),
        &mut store,
        reply(&r.thread_id, "one more thing", None),
    )
    .unwrap();
    assert!(rr.handed_to.is_none(), "the chat runs on this machine");
    assert_eq!(
        w.count(),
        1,
        "the replica that serves the chat takes it, not this one"
    );
    assert!(!rr.resumed && rr.woke.is_none(), "{rr:?}");
}

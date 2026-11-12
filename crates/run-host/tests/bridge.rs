//! The bridge entry points as library functions over a temp store (§6.4 item 1, §10). The CLI wiring is ticket
//! b7ck's; these are the checks it will stand on: refused before anything is written.

mod common;

use nexus_run_contracts::{
    Decision, EventBody, Origin, ReasonCode, RunId, RunStore, SessionFact, LOCAL_TENANT,
};
use nexus_run_host::bridge::{
    answer, commission, live_sessions, open_store, reply, session_of, status, withdraw, Caller,
    Refused, ReplyFlags,
};
use nexus_run_host::mint_owner_id;
use nexus_run_host::ports::Refs;
use nexus_run_host::test_support::scripted_backend::{pass, Cue, SessionInfo};
use nexus_run_host::test_support::{
    replies_pass, short_policy, store_path, PortLine, Rig, CLOCK_START, REQUESTER, WORKSPACE,
};

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

fn running(dir: &std::path::Path) -> (Rig, RunId, String, String) {
    let mut rig = Rig::scripted(dir, CLOCK_START, Box::new(silent));
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(300, move |rig| {
        rig.events(&check).iter().any(|event| {
            matches!(
                event.body,
                EventBody::SessionFact {
                    fact: SessionFact::Started { .. },
                    ..
                }
            )
        })
    });
    let key = format!("{run}/build/v1/coder/a1");
    (rig, run, receipt.root_thread, key)
}

fn inbox_depth(rig: &Rig, run: &RunId) -> usize {
    rig.store.waiting_inputs(run).unwrap().len()
}

fn coder(key: &str) -> Caller {
    Caller {
        actor: "coder".to_owned(),
        session: Some(key.to_owned()),
    }
}

fn thread_of(rig: &Rig, key: &str) -> String {
    session_of(&rig.bridge, key)
        .unwrap()
        .expect("the session is known")
        .session_key
        .map(|_| {
            String::from_utf8(rig.store.get(&format!("session/{key}")).unwrap().unwrap()).unwrap()
        })
        .unwrap()
}

#[test]
fn a_missing_file_is_the_one_way_to_say_not_v2() {
    let dir = tempfile::tempdir().unwrap();
    assert!(open_store(&dir.path().join("run.sqlite"))
        .unwrap()
        .is_none());
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    drop(rig);
    assert!(open_store(&store_path(dir.path())).unwrap().is_some());
}

#[test]
fn a_store_that_does_not_open_refuses_and_never_reads_as_not_v2() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    std::fs::write(
        &path,
        b"this is not a database, and it is long enough to be read as one",
    )
    .unwrap();
    assert!(open_store(&path).is_err());
}

/// A file whose existence cannot be told — its directory not searchable — is not "not v2" either: only
/// `NotFound` is (6j6v.b8tj review, IR #1). `Path::exists` answered `false` here, and a v2 session fell through
/// to the old lookup.
#[cfg(unix)]
#[test]
fn a_store_whose_directory_cannot_be_searched_refuses_and_never_reads_as_not_v2() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let nxs = dir.path().join(".nxs");
    std::fs::create_dir(&nxs).unwrap();
    std::fs::write(nxs.join("run.sqlite"), b"").unwrap();
    std::fs::set_permissions(&nxs, std::fs::Permissions::from_mode(0o000)).unwrap();
    let answer = open_store(&nxs.join("run.sqlite"));
    std::fs::set_permissions(&nxs, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        answer.is_err(),
        "unknown is not absent: {:?}",
        answer.map(|s| s.is_some())
    );
}

/// The one transaction of §6.4 item 1, and H4: `tenant: local`.
#[test]
fn a_commission_writes_one_transaction_and_posts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let refs = Refs {
        nxf_ids: vec!["6j6v.m8we".to_owned()],
    };
    let receipt = rig.commission_refs("the text", &refs);
    let run = &receipt.run;
    assert!(receipt.root_thread.starts_with("m-"));
    assert_eq!(
        rig.ports.lines(),
        Vec::new(),
        "nothing is posted before the log can answer for it"
    );
    assert_eq!(rig.events(run), Vec::new(), "the log waits for the owner");

    let inbox = rig.store.next_input(run).unwrap().expect("the inbox row");
    assert_eq!(inbox.origin, Origin::Cli);
    let EventBody::Commissioned {
        tenant,
        workspace_id,
        requester,
        root_thread,
        flow,
        ..
    } = &inbox.body
    else {
        panic!("{:?}", inbox.body);
    };
    assert_eq!(tenant, LOCAL_TENANT);
    assert_eq!(workspace_id, WORKSPACE);
    assert_eq!(requester, REQUESTER);
    assert_eq!(*root_thread, receipt.root_thread);
    let wire = serde_json::to_string(&inbox).unwrap();
    assert!(!wire.contains("the text"), "the body never reaches the log");

    let pending = rig.store.get(&format!("pending/{run}")).unwrap().unwrap();
    let pending = String::from_utf8(pending).unwrap();
    assert!(
        pending.contains("the text") && pending.contains("6j6v.m8we"),
        "{pending}"
    );
    assert!(rig
        .store
        .get(&format!("thread/{}", receipt.root_thread))
        .unwrap()
        .is_some());
    let stored = rig
        .store
        .get(&format!("flow/{}", flow.hash))
        .unwrap()
        .unwrap();
    let def: nexus_run_engine::FlowDef = serde_json::from_slice(&stored).unwrap();
    assert_eq!(def.flow_ref().hash, flow.hash, "content-addressed");
    assert_eq!(def.name, "coding");
}

#[test]
fn a_second_commission_reuses_the_flow_and_opens_its_own_run() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let first = rig.commission("one");
    let second = rig.commission("two");
    assert_ne!(first.run, second.run);
    assert_ne!(first.root_thread, second.root_thread);
}

#[test]
fn a_channel_that_does_not_compile_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let refused = commission(
        &rig.bridge,
        "review",
        "work",
        &Refs::default(),
        REQUESTER,
        short_policy(),
        rig.now(),
    );
    assert!(matches!(refused, Err(Refused::Compile(_))), "{refused:?}");
    assert_eq!(rig.store.open_runs().unwrap(), Vec::new());
}

#[test]
fn only_the_slots_own_session_can_reply_on_a_step_thread() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run, _, key) = running(dir.path());
    let thread = thread_of(&rig, &key);
    let posts = rig.ports.lines().len();
    let before = inbox_depth(&rig, &run);
    let wrong_handle = Caller {
        actor: "verifier".to_owned(),
        session: Some(key.clone()),
    };
    let wrong_session = Caller {
        actor: "coder".to_owned(),
        session: Some(format!("{run}/build/v1/coder/a9")),
    };
    let no_session = Caller {
        actor: "coder".to_owned(),
        session: None,
    };
    for caller in [
        &wrong_handle,
        &wrong_session,
        &no_session,
        &Rig::requester(),
    ] {
        let refused = reply(
            &rig.bridge,
            &thread,
            caller,
            "text",
            ReplyFlags::default(),
            rig.now(),
        );
        assert!(
            matches!(&refused, Err(Refused::NotTheSlot { owed_by }) if owed_by == "coder"),
            "{caller:?}: {refused:?}"
        );
    }
    assert_eq!(
        rig.ports.lines().len(),
        posts,
        "refused before anything was written"
    );
    assert_eq!(inbox_depth(&rig, &run), before);

    reply(
        &rig.bridge,
        &thread,
        &coder(&key),
        "done",
        ReplyFlags::default(),
        rig.now(),
    )
    .unwrap();
    assert_eq!(inbox_depth(&rig, &run), before + 1);
    let waiting = rig.store.next_input(&run).unwrap().unwrap();
    assert_eq!(waiting.origin, Origin::Cli);
    assert!(waiting.key.starts_with(&format!("input/{run}/")));
    assert!(
        rig.ports.lines().iter().any(|line| matches!(
            line,
            PortLine::Post { key: k, .. } if *k == waiting.key
        )),
        "the message carries the input's key"
    );
}

#[test]
fn a_reply_on_a_root_or_an_unknown_thread_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, _, root, key) = running(dir.path());
    for thread in [root.as_str(), "m-unknown"] {
        let refused = reply(
            &rig.bridge,
            thread,
            &coder(&key),
            "x",
            ReplyFlags::default(),
            rig.now(),
        );
        assert!(
            matches!(refused, Err(Refused::NotARunThread { .. })),
            "{refused:?}"
        );
    }
}

#[test]
fn a_finished_run_refuses_every_verb_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let receipt = rig.commission("work");
    let run = receipt.run;
    rig.run_to_finished(&run);
    let key = format!("{run}/build/v1/coder/a1");
    let thread = thread_of(&rig, &key);
    let posts = rig.ports.lines().len();
    let now = rig.now();
    let requester = Rig::requester();
    assert_eq!(
        reply(
            &rig.bridge,
            &thread,
            &coder(&key),
            "late",
            ReplyFlags::default(),
            now
        ),
        Err(Refused::RunFinished)
    );
    assert_eq!(
        answer(
            &rig.bridge,
            &receipt.root_thread,
            &requester,
            "late",
            false,
            now
        ),
        Err(Refused::RunFinished)
    );
    assert_eq!(
        withdraw(&rig.bridge, &receipt.root_thread, &requester, "late", now),
        Err(Refused::RunFinished)
    );
    assert_eq!(rig.ports.lines().len(), posts);
}

#[test]
fn only_the_requester_at_the_terminal_can_answer_or_withdraw() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run, root, key) = running(dir.path());
    let before = inbox_depth(&rig, &run);
    let now = rig.now();
    let session = coder(&key);
    let someone = Caller {
        actor: "someone".to_owned(),
        session: None,
    };
    let as_a_session = Caller {
        actor: REQUESTER.to_owned(),
        session: Some(key.clone()),
    };
    for caller in [&session, &someone, &as_a_session] {
        assert_eq!(
            withdraw(&rig.bridge, &root, caller, "stop", now),
            Err(Refused::NotTheRequester),
            "{caller:?}"
        );
        assert_eq!(
            answer(&rig.bridge, &root, caller, "yes", false, now),
            Err(Refused::NotTheRequester),
            "{caller:?}"
        );
    }
    assert_eq!(inbox_depth(&rig, &run), before);
}

#[test]
fn an_answer_with_no_open_question_is_refused_so_a_question_about_a_run_restarts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, _, root, _) = running(dir.path());
    assert_eq!(
        answer(
            &rig.bridge,
            &root,
            &Rig::requester(),
            "why?",
            false,
            rig.now()
        ),
        Err(Refused::NoOpenQuestion)
    );
}

/// Two answers before a fold address two questions, and `--accept` is refused on an escalation (§6.4 items 6, 8).
#[test]
fn two_answers_before_a_fold_address_two_questions_in_arrival_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(
        dir.path(),
        CLOCK_START,
        Box::new(|info| {
            if info.step == "judge" && info.member != "__synth__" && info.attempt == 1 {
                if info.member == "integrity" {
                    Cue::Silent
                } else {
                    Cue::Reply {
                        text: format!("{} asks", info.member),
                        flags: ReplyFlags {
                            escalate: true,
                            needs_rework: false,
                        },
                    }
                }
            } else if info.step == "judge" && info.attempt > 1 {
                Cue::Silent
            } else {
                pass()
            }
        }),
    );
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(800, move |rig| {
        rig.state(&check).open_questions().count() == 2
    });
    let root = &receipt.root_thread;
    let now = rig.now();
    let requester = Rig::requester();
    assert_eq!(
        answer(&rig.bridge, root, &requester, "accepted", true, now),
        Err(Refused::AcceptAtACeilingOnly)
    );
    answer(&rig.bridge, root, &requester, "first", false, now).unwrap();
    answer(&rig.bridge, root, &requester, "second", false, now).unwrap();
    assert_eq!(
        answer(&rig.bridge, root, &requester, "third", false, now),
        Err(Refused::NoOpenQuestion),
        "both questions are addressed by a pending answer"
    );
    let asked: Vec<String> = rig
        .state(&run)
        .open_questions()
        .map(|q| q.id.clone())
        .collect();
    let answered: Vec<String> = rig
        .store
        .waiting_inputs(&run)
        .unwrap()
        .into_iter()
        .filter_map(|input| match input.ok()?.body {
            EventBody::Answer { question, .. } => Some(question),
            _ => None,
        })
        .collect();
    assert_eq!(answered, asked);
}

fn at_a_ceiling() -> impl Fn(&SessionInfo) -> Cue + Send + Sync {
    |info: &SessionInfo| {
        if info.step == "verify" {
            Cue::Reply {
                text: "still not".to_owned(),
                flags: ReplyFlags {
                    needs_rework: true,
                    escalate: false,
                },
            }
        } else {
            pass()
        }
    }
}

#[test]
fn at_a_ceiling_accept_sends_the_run_on_and_the_question_was_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(at_a_ceiling()));
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(4_000, move |rig| rig.has(&check, "ceiling"));
    let check = run.clone();
    rig.run_until(50, move |rig| rig.has(&check, "delivered"));
    let question = rig.ports.lines().into_iter().find_map(|line| match line {
        PortLine::Post { key, body, .. } if key.starts_with("question/") => Some(body),
        _ => None,
    });
    let question = question.expect("the ceiling asked");
    assert!(
        question.contains("verify") && question.contains("still not"),
        "{question}"
    );
    assert!(question.contains("--accept"), "{question}");
    assert!(question.contains("nxc withdraw"), "{question}");
    answer(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "take it",
        true,
        rig.now(),
    )
    .unwrap();
    rig.backend.set_scenario(Box::new(|_| pass()));
    rig.run_to_finished(&run);
    let finished = rig.events(&run).iter().find_map(|event| match &event.body {
        EventBody::Decision(Decision::Finished { outcome, .. }) => Some(outcome.clone()),
        _ => None,
    });
    assert_eq!(finished, Some(nexus_run_contracts::Outcome::Delivered));
}

/// A run withdrawn in `Created` has no thread, and delivers nothing.
#[test]
fn a_withdrawal_before_the_root_exists_posts_nothing_and_finishes_with_the_body_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let receipt = rig.commission("work");
    let posted = withdraw(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    assert_eq!(posted, None);
    rig.run_to_finished(&receipt.run);
    assert_eq!(
        rig.store.get(&format!("pending/{}", receipt.run)).unwrap(),
        None
    );
    assert!(rig.ports.lines().iter().all(|line| !matches!(
        line,
        PortLine::Post { key, .. } if key.starts_with("result/")
    )));
}

/// H2: `local` is around the pure projection, with `observed_at`, and absent as a whole without the host.
#[test]
fn status_adds_local_around_the_pure_projection() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run, _, _) = running(dir.path());
    let pure = nexus_run_engine::status(&rig.state(&run));
    assert_eq!(pure.local, None);
    let wire = serde_json::to_value(&pure).unwrap();
    assert!(wire.get("local").is_none(), "absent as a whole: {wire}");
    assert!(wire.get("tree").is_none());

    let now = rig.now();
    let hosted = status(&rig.bridge, &run, now).unwrap().unwrap();
    let local = hosted
        .local
        .clone()
        .expect("the machine that holds the run adds it");
    assert_eq!(local.observed_at, now);
    assert!(local.owner.attended, "a host ticked within two expiries");
    assert_eq!(
        local.tree.path,
        rig.bridge.layout.v2_dir().to_string_lossy()
    );
    assert!(local.tree.base.is_some(), "the base comes from the state");
    assert_eq!(local.tree.commit, None, "the directory is missing");
    assert_eq!(local.tree.dirty, None);
    assert_eq!(local.tree.in_flight, None);
    // The rest of it is exactly the projection.
    let mut stripped = hosted;
    stripped.local = None;
    assert_eq!(stripped, pure);
}

#[test]
fn status_says_no_host_attends_when_none_has_been_seen() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let run = rig.commission("work").run;
    let local = status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap()
        .local
        .unwrap();
    assert!(!local.owner.attended);
    assert_eq!(local.owner.last_seen, None);
    assert_eq!(local.pending_inputs, 1, "the commission waits in the inbox");
}

#[test]
fn a_replied_session_winding_down_is_still_a_live_session() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, _, key) = running(dir.path());
    let thread = thread_of(&rig, &key);
    assert_eq!(live_sessions(&rig.bridge).unwrap().len(), 1);
    reply(
        &rig.bridge,
        &thread,
        &coder(&key),
        "done",
        ReplyFlags::default(),
        rig.now(),
    )
    .unwrap();
    rig.host.tick(rig.now());
    rig.host.tick(rig.now());
    let live = live_sessions(&rig.bridge).unwrap();
    assert!(
        live.iter().any(|session| session.session_key == key),
        "{live:?}"
    );
    let route = session_of(&rig.bridge, &key).unwrap().unwrap();
    assert_eq!(route.run, run);
    assert_eq!(session_of(&rig.bridge, "r-x/unknown").unwrap(), None);
}

/// M6: `origin` is set from where the input really came, never taken from what the row says.
#[test]
fn origin_is_set_by_the_host_never_taken_from_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, _, key) = running(dir.path());
    let forged = nexus_run_contracts::Event::new(
        run.clone(),
        0,
        rig.now(),
        format!("input/{run}/forged"),
        Origin::Reducer,
        EventBody::Reply {
            session_key: key.clone(),
            message: "m-1".to_owned(),
            escalate: false,
            needs_rework: false,
        },
    );
    rig.store.enqueue(&run, &forged, &[]).unwrap();
    let facts = rig.events(&run);
    assert!(facts
        .iter()
        .all(|event| event.origin != Origin::Cli
            || matches!(event.body, EventBody::Commissioned { .. })));
    rig.host.tick(rig.now());
    let logged = rig
        .events(&run)
        .into_iter()
        .find(|event| event.key == forged.key)
        .expect("it folded");
    assert_eq!(logged.origin, Origin::Cli, "the row said reducer");

    let by_kind = rig.events(&run);
    let origin_of = |kind: &str| {
        by_kind
            .iter()
            .find(|event| event.body.kind() == kind)
            .map(|event| event.origin.clone())
    };
    assert_eq!(origin_of("commissioned"), Some(Origin::Cli));
    assert_eq!(origin_of("root_opened"), Some(Origin::Host));
    assert_eq!(origin_of("session_fact"), Some(Origin::Adapter));
    assert_eq!(origin_of("step_started"), Some(Origin::Reducer));
}

/// An unknown kind whose row claims the reducer wrote it can never pass for an unknown decision, which would
/// refuse ownership of the run on the next fold.
#[test]
fn an_unknown_input_kind_folds_to_nothing_and_never_wedges_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rig, run, _, _) = running(dir.path());
    let raw = serde_json::json!({ "kind": "from_a_newer_writer" });
    let unknown = nexus_run_contracts::Event::new(
        run.clone(),
        0,
        rig.now(),
        format!("input/{run}/newer"),
        Origin::Reducer,
        EventBody::Unknown(nexus_run_contracts::UnknownBody {
            kind: "from_a_newer_writer".to_owned(),
            raw,
        }),
    );
    rig.store.enqueue(&run, &unknown, &[]).unwrap();
    rig.host.tick(rig.now());
    assert_eq!(rig.store.waiting_inputs(&run).unwrap().len(), 0, "consumed");
    rig.clock.advance(31_000);
    rig.restart();
    rig.tick();
    let status = status(&rig.bridge, &run, rig.now()).unwrap().unwrap();
    assert_ne!(status.waiting_for.as_deref(), Some("a newer nxs"));
    assert_eq!(status.reason, None::<ReasonCode>);
}

/// H5: `owner_id` is an opaque minted id, with no user or machine name in it.
#[test]
fn owner_id_is_opaque_and_names_nobody() {
    let id = mint_owner_id();
    assert!(id.starts_with("h-"), "{id}");
    assert_eq!(id.len(), 2 + 26, "h- and a ULID: {id}");
    assert_ne!(mint_owner_id(), id, "minted per host");
    for name in ["USER", "LOGNAME", "HOSTNAME", "HOME"] {
        if let Ok(value) = std::env::var(name) {
            if value.len() >= 3 {
                assert!(!id.contains(&value), "{name} is in {id}");
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let run = rig.commission("work").run;
    let mut rig = rig;
    rig.tick();
    let stored = rig.store.get(&format!("owner/{run}")).unwrap().unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&stored).unwrap();
    assert_eq!(stored["owner_id"], rig.host.owner_id());
    assert_eq!(
        stored.as_object().unwrap().len(),
        3,
        "owner_id, epoch, expires_at: {stored}"
    );
}

//! What a session is started with (§6.4 item 3, §5, §8.2): the turn composed by the port from ids, the back edge
//! carrying its verdict, the obligation per step, the v2 prime, and the session's record.

mod common;

use nexus_run_contracts::{EventBody, RunId, RunStore, StartSpec, TimerPurpose};
use nexus_run_engine::{slot_timer, SlotState, SUPERVISOR, SYNTH};
use nexus_run_host::bridge::{self, ReplyFlags};
use nexus_run_host::test_support::scripted_backend::{pass, Cue, SessionInfo};
use nexus_run_host::test_support::{scenario_named, PortLine, Rig, CLOCK_START};

fn spec(rig: &Rig, key: &str) -> StartSpec {
    let bytes = std::fs::read(rig.dir.join("agent-logs/v2").join(key).join("spec.json"))
        .unwrap_or_else(|_| panic!("no spec for {key}"));
    serde_json::from_slice(&bytes).unwrap()
}

fn finished_rig(dir: &std::path::Path, scenario: &str) -> (Rig, RunId) {
    let mut rig = Rig::scripted(dir, CLOCK_START, scenario_named(scenario));
    let run = rig.commission("the commission").run;
    rig.run_to_finished(&run);
    (rig, run)
}

/// The declaration's permission mode reaches the session; the consolidator has no persona, and so none.
#[test]
fn the_personas_permission_mode_reaches_the_start_spec() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "happy");
    let build = spec(&rig, &format!("{run}/build/v1/coder/a1"));
    assert_eq!(build.permissions.as_deref(), Some("acceptEdits"));
}

#[test]
fn the_back_edge_turn_carries_the_verdict_of_the_step_that_sent_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "rework");
    let second_build = spec(&rig, &format!("{run}/build/v2/coder/a1"));
    assert!(
        second_build.turn.text.contains("not yet"),
        "the verdict is in the turn: {}",
        second_build.turn.text
    );
    // First build: the commission, and no verdict that does not exist yet.
    let first_build = spec(&rig, &format!("{run}/build/v1/coder/a1"));
    assert!(first_build.turn.text.contains("the commission"));
    assert!(!first_build.turn.text.contains("not yet"));
    // A back edge's input is the verdict, framed by the port, by id.
    let asked = rig.ports.asked.lock().unwrap().clone();
    assert!(
        asked
            .iter()
            .any(|line| line.starts_with("turn coder reply of verifier on verify m-")),
        "{asked:?}"
    );
}

#[test]
fn a_back_edge_resumes_the_personas_backend_session_of_this_run() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "rework");
    assert_eq!(spec(&rig, &format!("{run}/build/v1/coder/a1")).resume, None);
    assert_eq!(
        spec(&rig, &format!("{run}/build/v2/coder/a1"))
            .resume
            .as_deref(),
        Some("b-coder-1")
    );
    assert_eq!(
        spec(&rig, &format!("{run}/verify/v1/verifier/a1")).resume,
        None
    );
}

#[test]
fn the_turn_is_composed_by_the_port_from_ids_with_the_task_and_obligation_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "happy");
    let reviewer = spec(&rig, &format!("{run}/judge/v1/integrity/a1"));
    let text = &reviewer.turn.text;
    let framed = text.find("<commission>").expect("the port framed the root");
    let task = text.find("Judge the change").expect("the step's task");
    let obligation = text
        .find("nxc reply --thread")
        .expect("the host's obligation");
    assert!(framed < task && task < obligation, "{text}");
    // verify's `input: []` hands nothing.
    let verifier = spec(&rig, &format!("{run}/verify/v1/verifier/a1"));
    assert!(
        !verifier.turn.text.contains("<reply"),
        "{}",
        verifier.turn.text
    );
    // The finisher is handed verify and judge, and judge's voice is its synthesis.
    let finisher = spec(&rig, &format!("{run}/finish/v1/finisher/a1"));
    assert!(finisher.turn.text.contains("reply of verifier on verify"));
    assert!(finisher.turn.text.contains("reply of __synth__ on judge"));
}

#[test]
fn the_turn_is_the_message_the_thread_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "happy");
    let key = format!("{run}/build/v1/coder/a1");
    let started = spec(&rig, &key);
    let posted = rig.ports.lines().into_iter().find_map(|line| match line {
        PortLine::Post {
            key: k,
            body,
            sender,
            thread,
            ..
        } if k == format!("turn/{key}") => Some((body, sender, thread)),
        _ => None,
    });
    let (body, sender, thread) = posted.expect("the turn is posted");
    assert_eq!(body, started.turn.text);
    assert_eq!(sender, SUPERVISOR);
    assert_eq!(thread, started.turn.thread);
}

/// What a v2 session receives holds no way to commission: `prompt` asked for `V2`, and the block is the host's.
#[test]
fn a_v2_session_is_primed_for_v2_and_nxc_send_is_absent_from_what_it_receives() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, _) = finished_rig(dir.path(), "rework");
    let asked = rig.ports.asked.lock().unwrap().clone();
    let prompts: Vec<&String> = asked.iter().filter(|l| l.starts_with("prompt ")).collect();
    assert!(!prompts.is_empty());
    assert!(
        prompts.iter().all(|line| line.ends_with(" V2")),
        "{prompts:?}"
    );
    assert!(
        !prompts.iter().any(|line| line.contains(SYNTH)),
        "the consolidator has no persona"
    );
    for key in rig.spawned() {
        let started = spec(&rig, &key);
        for text in [&started.turn.text, &started.prompt.system_prompt] {
            assert!(!text.contains("nxc send"), "{key} received: {text}");
        }
        assert!(
            started.turn.text.contains("cannot commission"),
            "{key} is told it cannot commission"
        );
    }
}

/// `--needs-rework` exactly where the step declares `on_needs_rework`, asserted per step (§6.4 item 3).
#[test]
fn the_obligation_offers_needs_rework_exactly_where_the_step_declares_a_back_edge() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "happy");
    let cases = [
        ("build/v1/coder", false),
        ("verify/v1/verifier", true),
        ("judge/v1/code-quality", true),
        ("judge/v1/test-quality", true),
        ("judge/v1/integrity", true),
        ("judge/v1/__synth__", true),
        ("finish/v1/finisher", true),
    ];
    for (seat, offered) in cases {
        let started = spec(&rig, &format!("{run}/{seat}/a1"));
        assert_eq!(
            started.turn.text.contains("--needs-rework"),
            offered,
            "{seat}: {}",
            started.turn.text
        );
        assert!(started.turn.text.contains("--escalate"), "{seat}");
        assert!(
            started
                .turn
                .text
                .contains(&format!("--thread {}", started.turn.thread)),
            "{seat} names its thread"
        );
    }
}

#[test]
fn a_session_is_started_with_its_identity_in_the_environment_and_nothing_outside_the_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "happy");
    let key = format!("{run}/build/v1/coder/a1");
    let started = spec(&rig, &key);
    let env = |name: &str| {
        started
            .env
            .iter()
            .find(|var| var.name == name)
            .map(|var| var.value.as_str())
    };
    assert_eq!(env("NXC_SESSION"), Some(key.as_str()));
    assert_eq!(env("NXC_ACTOR"), Some("coder"));
    assert_eq!(env("NXC_SIDECAR"), Some("/nonexistent/sidecar.mjs"));
    for var in &started.env {
        assert!(
            nexus_run_contracts::ENV_ALLOWLIST.contains(&var.name.as_str()),
            "{} is not on the allowlist",
            var.name
        );
    }
    assert!(started.cwd.ends_with("root-v2"), "{}", started.cwd);
    assert!(started.ledger_dir.ends_with(&key), "{}", started.ledger_dir);
}

/// `started` is the session's fact: `persona_session` is written when it folds, and `start_deadline` is armed by
/// `tree_prepared`, never by the spawn and never while the tree is unprepared (§6.5).
#[test]
fn start_deadline_is_armed_by_tree_prepared_and_persona_session_when_started_folds() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(|_| Cue::Silent));
    let ledger_tree = std::time::Duration::from_secs(5);
    rig.tree.script(vec![
        nexus_run_host::test_support::scripted_tree::TreeStep::Delay(ledger_tree),
    ]);
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(50, move |rig| rig.has(&check, "step_started"));
    let slot = nexus_run_contracts::Slot {
        step: "build".to_owned(),
        visit: 1,
        member: "coder".to_owned(),
        attempt: 1,
    };
    let timer = slot_timer(&TimerPurpose::StartDeadline, &slot);
    let key = slot.session_key(&run);
    let state = rig.state(&run);
    assert!(!state.timers.contains_key(&timer), "armed with no tree");
    assert_eq!(state.slots[0].state, SlotState::Pending);
    assert!(rig.spawned().is_empty(), "a session before the tree");

    // Host ticks only from here: the session reports nothing until the rig moves it.
    let handle_key = format!("persona_session/{run}/coder");
    rig.clock.advance(6_000);
    common::tree::until("the session spawned", || {
        let now = rig.now();
        rig.host.tick(now);
        if rig.spawned().contains(&key) {
            return true;
        }
        // The worker's delay starts when its thread gets there, so the clock has to keep moving.
        rig.clock.advance(1_000);
        false
    });
    assert!(rig.has(&run, "tree_prepared"));
    assert!(
        rig.state(&run).timers.contains_key(&timer),
        "armed by tree_prepared"
    );
    assert!(rig.spawned().contains(&key));
    assert_eq!(
        rig.store.get(&handle_key).unwrap(),
        None,
        "the spawn records nothing"
    );
    assert!(rig
        .dir
        .join("agent-logs/v2")
        .join(&key)
        .join("child.json")
        .exists());
    let check = run.clone();
    rig.run_until(30, move |rig| {
        rig.events(&check).iter().any(|event| {
            matches!(
                event.body,
                EventBody::SessionFact {
                    fact: nexus_run_contracts::SessionFact::Started { .. },
                    ..
                }
            )
        })
    });
    assert_eq!(
        rig.store.get(&handle_key).unwrap().as_deref(),
        Some(b"b-coder-1".as_slice())
    );
    assert!(
        !rig.state(&run).timers.contains_key(&timer),
        "`started` clears it"
    );
}

fn escalating(info: &SessionInfo) -> Cue {
    if info.step == "verify" && info.attempt == 1 {
        Cue::Reply {
            text: "which branch?".to_owned(),
            flags: ReplyFlags {
                escalate: true,
                needs_rework: false,
            },
        }
    } else {
        pass()
    }
}

/// A `Question` is templated from state and posted as the supervisor, once; the answer reaches the same seat.
#[test]
fn a_question_is_delivered_once_as_the_supervisor_and_the_answer_resumes_the_asking_seat() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(escalating));
    let receipt = rig.commission("work");
    let run = receipt.run;
    let root = receipt.root_thread;
    let check = run.clone();
    rig.run_until(400, move |rig| rig.has(&check, "delivered"));
    for _ in 0..10 {
        rig.tick();
    }
    let questions: Vec<(String, String)> = rig
        .ports
        .lines()
        .into_iter()
        .filter_map(|line| match line {
            PortLine::Post {
                key,
                sender,
                body,
                thread,
                ..
            } if key.starts_with("question/") && thread == root => Some((sender, body)),
            _ => None,
        })
        .collect();
    assert_eq!(questions.len(), 1, "owed once");
    let (sender, body) = &questions[0];
    assert_eq!(sender, SUPERVISOR);
    assert!(body.contains("which branch?"), "{body}");
    assert!(body.contains("verifier"), "{body}");

    bridge::answer(
        &rig.bridge,
        &root,
        &Rig::requester(),
        "the feature branch",
        false,
        rig.now(),
    )
    .unwrap();
    let answer_turn = format!("{run}/verify/v1/verifier/a2");
    let probe = answer_turn.clone();
    rig.run_until(400, move |rig| rig.spawned().contains(&probe));
    let started = spec(&rig, &answer_turn);
    assert!(
        started.turn.text.contains("the feature branch"),
        "{}",
        started.turn.text
    );
    assert_eq!(
        started.resume.as_deref(),
        Some("b-verifier-1"),
        "the same session"
    );
    rig.run_to_finished(&run);
}

/// A restarted last step is told that an earlier attempt may have pushed (§8.3).
#[test]
fn a_restarted_finisher_is_told_an_earlier_attempt_may_have_pushed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(
        dir.path(),
        CLOCK_START,
        Box::new(|info| {
            if info.step == "finish" && info.attempt == 1 {
                Cue::End(nexus_run_contracts::EndCause::Lost)
            } else {
                pass()
            }
        }),
    );
    let run = rig.commission("work").run;
    rig.run_to_finished(&run);
    let first = spec(&rig, &format!("{run}/finish/v1/finisher/a1"));
    let second = spec(&rig, &format!("{run}/finish/v1/finisher/a2"));
    assert!(!first.turn.text.contains("earlier attempt"));
    assert!(
        second.turn.text.contains("earlier attempt"),
        "{}",
        second.turn.text
    );
    let timeline = common::Timeline(rig.backend.timeline());
    assert!(timeline.no_two_sessions_alive_for_one(("finish", "finisher")));
}

/// The consolidator is a session of its own: no persona, its prompt from the definition, the three replies framed
/// through the port, its reply accepted from `__synth__` with its own key (§6.4 item 7).
#[test]
fn the_synthesis_turn_is_the_three_replies_and_the_definitions_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let (rig, run) = finished_rig(dir.path(), "happy");
    let started = spec(&rig, &format!("{run}/judge/v1/{SYNTH}/a1"));
    for member in ["code-quality", "test-quality", "integrity"] {
        assert!(
            started
                .turn
                .text
                .contains(&format!("reply of {member} on judge")),
            "{}",
            started.turn.text
        );
    }
    assert!(started
        .turn
        .text
        .contains("Consolidate the three verdicts."));
    assert_eq!(started.resume, None);
    assert_eq!(started.tools, ["Bash(nxc reply:*)"]);
}

#[test]
fn the_delivered_result_is_the_finishers_reply_on_the_root_thread_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(
        dir.path(),
        CLOCK_START,
        Box::new(|info| {
            if info.step == "finish" {
                Cue::Reply {
                    text: "the pull request is open".to_owned(),
                    flags: ReplyFlags::default(),
                }
            } else {
                pass()
            }
        }),
    );
    let receipt = rig.commission("work");
    rig.run_to_finished(&receipt.run);
    let results: Vec<(String, String)> = rig
        .ports
        .lines()
        .into_iter()
        .filter_map(|line| match line {
            PortLine::Post {
                key,
                sender,
                body,
                thread,
                ..
            } if key == format!("result/{}", receipt.run) && thread == receipt.root_thread => {
                Some((sender, body))
            }
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, SUPERVISOR);
    assert_eq!(results[0].1, "the pull request is open");
}

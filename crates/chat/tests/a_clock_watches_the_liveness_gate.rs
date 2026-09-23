//! **A step whose session never announces its end no longer holds the channel for good** (nxf
//! 6j6v.858n).
//!
//! Since nxf 6j6v.10yb a step of a `working_tree: exclusive` channel is over when it has answered
//! AND its session is over. The second fact normally arrives on its own — the sidecar's teardown
//! calls `nxc session ended` as its last act. **The gap is what happens when it does not**: a
//! sidecar killed before that step, an older `nxc`, a host runtime with no `Engine::session_ended`,
//! or a process that simply hangs. Then the gate declines, and — until this item — nothing ever
//! looked again:
//!
//! * `rearm_member_window` schedules the next instant a member's REPLY window falls due, and a
//!   member that has replied is `complete`, so it is not a candidate. On the commonest shape of all,
//!   one member per step, there is no candidate at all and no job was scheduled.
//! * `supervisor_consider_set` — the PUSH path, which is where the decline actually happens when the
//!   reply arrives — armed nothing whatsoever.
//! * The channel's declared `timeout:` cannot end it: `stale` needs a non-empty `outstanding`, and
//!   this member has answered. It is a deadline for an ANSWER, not for a process.
//!
//! So the way out was `nxc tick --thread <id>`, typed by a human who first had to work out that
//! this was the state they were in.
//!
//! **What this adds is one clock, in both paths**: a decline whose ONLY remaining blocker is a live
//! session arms a re-check of that same board, [`SESSION_LIVENESS_RECHECK`] later. A hard-killed
//! session is then noticed by the next re-check and the flow moves on by itself; a genuinely hung
//! process keeps declining and keeps re-arming, which is the same stall as before and no worse — but
//! now a visible one, with something looking at it.
//!
//! **Why this could not be built with nxf 6j6v.10yb**, in that ticket's own words: it hangs on the
//! same one-shot job planner as the channel `timeout:`, and on macOS that planner did not fire at
//! all (6j6v.p3sm) — on exactly the machine 6j6v.10yb was measured on. That reason went away on
//! 2026-08-25 with the one nexus-flow service (6j6v.8see), which is why this is buildable now.
//!
//! Time is controlled by `NXC_NOW`; nothing here sleeps or polls. What was ARMED is read off
//! `DryTimer`'s log, which is the record of what a real clock would have been asked to do.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::{Caller, SESSION_LIVENESS_RECHECK_SECS};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

/// The measured instant of nxf 6j6v.10yb's timeline: the coder answers while it is still writing.
const NOW: &str = "2026-08-24T00:16:33Z";
/// `NOW` plus [`SESSION_LIVENESS_RECHECK_SECS`] — when the board must be looked at again.
const RECHECK_AT: &str = "2026-08-24T00:17:33Z";

/// `NXC_TIMER_LOG` is process-global env, so every test in this binary that reads it serializes
/// against the others — `timer.rs`'s own unit tests take exactly this precaution for exactly this
/// reason.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A `DryTimer` pointed at a fresh log, for the duration of one test.
struct TimerLog<'a> {
    #[allow(dead_code)]
    lock: MutexGuard<'a, ()>,
    path: std::path::PathBuf,
}

impl TimerLog<'_> {
    fn open(dir: &std::path::Path) -> TimerLog<'_> {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.join("timer.log");
        std::env::set_var("NXC_TIMER_LOG", &path);
        TimerLog { lock, path }
    }

    /// Every instant a tick has been scheduled for on `thread`, in the order they were armed.
    fn armed_for(&self, thread: &str) -> Vec<String> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("schedule "))
            .filter(|l| l.contains(&format!("thread={thread} ")))
            .filter_map(|l| {
                l.split_whitespace()
                    .find_map(|f| f.strip_prefix("deadline="))
                    .map(str::to_string)
            })
            .collect()
    }
}

impl Drop for TimerLog<'_> {
    fn drop(&mut self) {
        std::env::remove_var("NXC_TIMER_LOG");
    }
}

#[derive(Default)]
struct LiveSessionWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
}

impl LiveSessionWorker {
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    fn only_for(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
            .clone()
    }
}

impl Worker for LiveSessionWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// The measured `coding` channel: ordered over two steps, and it needs the working copy alone.
const EXCLUSIVE_CODING: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\nworking_tree: exclusive\n";
/// The same flow WITHOUT the exclusive claim — the control. No liveness gate, so no clock for it.
const SHARED_CODING: &str = "name: coding\nmembers: [coder, finisher]\nflow: sequential\n";

fn team(tmp: &TempDir, channel_yaml: &str) -> (Engine, Arc<LiveSessionWorker>) {
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder"), role("finisher")],
        vec![channel(channel_yaml)],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(LiveSessionWorker::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (engine, worker)
}

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

fn as_session<'a>(session: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

/// Open the round and hand back (the coder's session, its slot thread, the channel thread).
fn open_round(engine: &Engine, worker: &LiveSessionWorker) -> (String, String, String) {
    let opened = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens");
    let coder = worker.only_for("coder");
    let slot = coder
        .reply_thread
        .clone()
        .expect("a commissioned step is told which thread it owes an answer on");
    (coder.internal_session.clone(), slot, opened.thread_id)
}

fn answer(engine: &Engine, session: &str, thread: &str) {
    engine
        .reply_thread(
            as_session(session),
            ReplyThreadRequest {
                machine: None,
                thread,
                body: "ZWISCHENSTAND — nicht der Abschluss.",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");
}

// ---- the PUSH path ---------------------------------------------------------------------------

#[test]
fn a_reply_declined_by_the_liveness_gate_arms_a_re_check() {
    let tmp = TempDir::new().unwrap();
    let log = TimerLog::open(tmp.path());
    let (engine, worker) = team(&tmp, EXCLUSIVE_CODING);
    let (coder, slot, channel_thread) = open_round(&engine, &worker);
    worker.running.lock().unwrap().insert(coder.clone());

    answer(&engine, &coder, &slot);

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "premise: the gate declined, so there is something to come back to"
    );
    assert_eq!(
        log.armed_for(&channel_thread).last().map(String::as_str),
        Some(RECHECK_AT),
        "the PUSH path armed nothing at all before this item — and it is the path a real reply \
         takes, so a lost end announcement was a stall with no clock on it. Armed: {:?}",
        log.armed_for(&channel_thread)
    );
}

// ---- the control -----------------------------------------------------------------------------

#[test]
fn a_channel_that_does_not_claim_the_working_copy_gets_no_liveness_clock() {
    // The gate only exists on a channel that declared it needs the checkout alone, so the clock
    // that watches the gate must not appear anywhere else. Nothing in this item may put a job on a
    // board that was never going to wait for a process.
    let tmp = TempDir::new().unwrap();
    let log = TimerLog::open(tmp.path());
    let (engine, worker) = team(&tmp, SHARED_CODING);
    let (coder, slot, channel_thread) = open_round(&engine, &worker);
    worker.running.lock().unwrap().insert(coder.clone());

    answer(&engine, &coder, &slot);

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "finisher".to_string()],
        "premise: a shared channel advances on the answer, which is unchanged by this item"
    );
    assert!(
        !log.armed_for(&channel_thread)
            .iter()
            .any(|d| d == RECHECK_AT),
        "no liveness re-check belongs on a board with no liveness gate. Armed: {:?}",
        log.armed_for(&channel_thread)
    );
}

#[test]
fn the_window_is_one_named_constant_rather_than_a_number_in_two_places() {
    // Both paths arm the same window, and the value is stated once. A minute is short enough that
    // a lost end announcement costs a minute rather than a working day, and long enough that a
    // step which takes its eighteen measured minutes costs eighteen cheap store reads.
    assert_eq!(SESSION_LIVENESS_RECHECK_SECS, 60);
}

// ---- and the PULL path, through the shipped path with a real process -------------------------
//
// `tick` has no seam twin by decision (nxf 6j6v.dvyq — an app drives its own scheduler and what it
// schedules is this command), so the re-check itself can only be driven the way the clock drives
// it: by running `nxc tick --thread <id>`. That is also where the acceptance of this item lives —
// a REAL process, killed without announcing anything, and a channel that frees itself.

mod cli {
    use assert_cmd::Command;
    use serde_json::Value;
    use tempfile::TempDir;

    use nexus_chat::workspace::{chat_config, setup};

    /// The instant the round opens, and the two the clock lands on after it.
    const OPEN: &str = "2026-08-24T00:16:33Z";
    const RECHECK_AT: &str = "2026-08-24T00:17:33Z";
    const AND_AGAIN_AT: &str = "2026-08-24T00:18:33Z";

    fn workspace(channels: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles = tmp.path().join(".nxs-personas");
        std::fs::create_dir_all(&roles).unwrap();
        for handle in ["coder", "finisher"] {
            std::fs::write(
                roles.join(format!("{handle}.yaml")),
                format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
            )
            .unwrap();
        }
        std::fs::write(roles.join("channels.yaml"), channels).unwrap();
        std::fs::write(
            tmp.path().join("stub-sidecar.mjs"),
            "setTimeout(() => {}, 30_000);\n",
        )
        .unwrap();
        tmp
    }

    const EXCLUSIVE_CODING: &str = "- name: coding\n  members: [coder, finisher]\n  \
                                    flow: sequential\n  working_tree: exclusive\n";

    fn nxc(tmp: &TempDir, now: &str) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", now)
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", tmp.path().join("stub-sidecar.mjs"))
            .env("NXC_TIMER", "dry")
            .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
        c
    }

    fn persona<'a>(cmd: &'a mut Command, handle: &str, session: &str) -> &'a mut Command {
        cmd.env("NXC_ACTOR", handle).env("NXC_SESSION", session)
    }

    fn json_of(cmd: &mut Command) -> Value {
        let out = cmd.assert().success();
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
            .expect("valid json")
    }

    fn started_sessions(tmp: &TempDir) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(tmp.path().join(".nxs/agent-logs"))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter_map(|e| {
                        e.file_name()
                            .to_str()
                            .and_then(|n| n.strip_suffix(".spec.json"))
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    fn armed_for(tmp: &TempDir, thread: &str) -> Vec<String> {
        std::fs::read_to_string(tmp.path().join("timer.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("schedule "))
            .filter(|l| l.contains(&format!("thread={thread} ")))
            .filter_map(|l| {
                l.split_whitespace()
                    .find_map(|f| f.strip_prefix("deadline="))
                    .map(str::to_string)
            })
            .collect()
    }

    /// Kill every stub sidecar this workspace started, and **wait until the kernel agrees they are
    /// gone** (nxf 6j6v.jg09).
    ///
    /// The wait is the whole point and it was missing. `kill -9` returns as soon as the signal is
    /// DELIVERED, not once the process is reaped — and what every caller does next is tick, which
    /// asks the liveness gate whether that process still exists. On a fast machine (and in
    /// `--release`, where the gap between the two is smaller) the gate could still find it, and the
    /// tick answered `waiting_for_a_session` instead of `advanced`. That reddened `main` on
    /// 2026-08-26 and went green on a bare re-run, which is exactly what a race looks like from the
    /// outside.
    ///
    /// Polling `pgrep` rather than `kill(pid, 0)` on purpose: `pgrep` is what asks the same
    /// question the gate asks — is there a process matching this script — so the wait ends when the
    /// gate's own answer has changed, not when a neighbouring one has. Bounded, and a breach is
    /// LOUD: a `reap` that silently gave up would put the flake back with an extra second of
    /// latency.
    fn reap(tmp: &TempDir) {
        let pattern = format!("{}", tmp.path().join("stub-sidecar.mjs").display());
        let alive = || {
            std::process::Command::new("pgrep")
                .args(["-f", &pattern])
                .output()
                .map(|out| {
                    String::from_utf8_lossy(&out.stdout)
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        for pid in alive() {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid])
                .status();
        }
        for _ in 0..500 {
            if alive().is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!(
            "a stub sidecar survived `kill -9` for five seconds ({pattern}) — the liveness gate \
             would read it as live, and every assertion after this one would be about the wrong \
             state"
        );
    }

    /// Open the round and answer from inside the coder's own session, while its process lives.
    fn a_round_stuck_on_a_live_session(tmp: &TempDir) -> (String, String) {
        let opened =
            json_of(nxc(tmp, OPEN).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
        let channel_thread = opened["thread_id"].as_str().unwrap().to_string();
        let coder = started_sessions(tmp)
            .into_iter()
            .next()
            .expect("step one started");
        let spec: Value = serde_json::from_str(
            &std::fs::read_to_string(
                tmp.path()
                    .join(".nxs/agent-logs")
                    .join(format!("{coder}.spec.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let slot = spec["replyThread"].as_str().unwrap().to_string();
        json_of(persona(
            nxc(tmp, OPEN).args([
                "--json",
                "reply",
                "--thread",
                &slot,
                "INTERIM - not the finish",
            ]),
            "coder",
            &coder,
        ));
        assert_eq!(
            started_sessions(tmp),
            vec![coder.clone()],
            "premise: the gate declined — the coder's process is still writing"
        );
        (coder, channel_thread)
    }

    #[test]
    fn a_session_killed_without_announcing_frees_the_channel_on_the_next_re_check() {
        // THE ACCEPTANCE. Before this item the only way out of here was a human working out that
        // this was the state they were in and typing `nxc tick --thread <id>` themselves.
        let tmp = workspace(EXCLUSIVE_CODING);
        let (coder, channel_thread) = a_round_stuck_on_a_live_session(&tmp);
        assert_eq!(
            armed_for(&tmp, &channel_thread).last().map(String::as_str),
            Some(RECHECK_AT),
            "the reply put a re-check on the books: {:?}",
            armed_for(&tmp, &channel_thread)
        );

        // Killed hard — no `session ended`, no teardown, nothing announced at all.
        reap(&tmp);

        // What the service runs when that deadline falls due, verbatim (`Job::ChatTick`).
        let ticked =
            json_of(nxc(&tmp, RECHECK_AT).args(["--json", "tick", "--thread", &channel_thread]));
        assert_eq!(
            ticked["reason"], "advanced",
            "the re-check found no process and moved the flow on: {ticked}"
        );
        let after = started_sessions(&tmp);
        assert_eq!(after.len(), 2, "the finisher started: {after:?}");
        assert!(after.contains(&coder), "{after:?}");
        assert!(
            ticked["delivered"].is_null(),
            "…and it ADVANCED rather than folding the round: the flow is not over, so nothing is \
             delivered to the requester yet: {ticked}"
        );
        assert_eq!(
            ticked["warnings"].as_array().map(Vec::len),
            Some(0),
            "no finding, because nothing went wrong: the step ANSWERED and the flow was only ever \
             waiting for its process to end. A `step_unanswered` here would be a false alarm on the \
             commonest path this branch takes: {ticked}"
        );

        reap(&tmp);
    }

    #[test]
    fn a_failed_step_start_is_not_retried_by_a_later_tick() {
        // The half of the widening the review of PR #375 asked about (Code Quality #2): now that a
        // step which ANSWERED can advance the flow from here, does a start that FAILED get retried
        // by the next tick, where before this item the board would quietly have consolidated?
        //
        // For a ROLE step it does not, and by construction rather than by a guard: `open_flow_step`
        // writes the slot thread BEFORE it attempts the trigger, so a failed start leaves an
        // unanswered slot behind, the set is no longer settled, and `tick_move` answers `Waiting`.
        //
        // **THE MECHANISM CHANGED WITH nxf 6j6v.n92p, and the reason is worth reading.** This used
        // to break the start by DELETING `finisher.yaml` between the answer and the tick. That no
        // longer breaks anything: an operation is now bound to the declarations it opened under, so
        // a persona deleted mid-chain still resolves — which is nby8's question 2, answered on
        // purpose, and pinned by `declaration_freeze.rs`'s own suite. So the start is broken here
        // where it is genuinely fragile and always was: the WORKER. An unknown `NXC_WORKER` makes
        // the spawn fail for a reason that has nothing to do with liveness and nothing to do with
        // declarations, which is exactly what this test needs from it.
        let tmp = workspace(EXCLUSIVE_CODING);
        let (_coder, channel_thread) = a_round_stuck_on_a_live_session(&tmp);
        reap(&tmp);

        let first = json_of(
            nxc(&tmp, RECHECK_AT)
                .env("NXC_WORKER", "no-such-worker")
                .args(["--json", "tick", "--thread", &channel_thread]),
        );
        assert_eq!(
            first["reason"], "advanced",
            "the liveness gate is clear, so the flow does move on — this is the premise: {first}"
        );
        assert_eq!(
            started_sessions(&tmp).len(),
            1,
            "…and the step could not be started, because its role is gone"
        );
        let skipped = first["warnings"]
            .as_array()
            .expect("warnings is an array")
            .iter()
            .find(|w| w["class"] == "step_skipped")
            .unwrap_or_else(|| panic!("the step that never started is reported: {first}"));
        assert!(
            skipped["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("finisher"),
            "the finding names the step: {skipped}"
        );

        // The second tick: the slot thread that was opened is still unanswered, so the set is not
        // settled and there is nothing to advance. No second attempt, no second session — and the
        // worker is healthy again here, so "nothing was started" is the flow declining to retry
        // rather than the same breakage twice.
        let again =
            json_of(nxc(&tmp, AND_AGAIN_AT).args(["--json", "tick", "--thread", &channel_thread]));
        assert_ne!(
            again["reason"], "advanced",
            "a failed start is reported once, not retried on every look: {again}"
        );
        assert_eq!(
            started_sessions(&tmp).len(),
            1,
            "and nothing was started a second time"
        );
    }

    #[test]
    fn a_re_check_that_declines_again_puts_the_next_one_on_the_books() {
        // A one-shot job fires ONCE. A re-check that declined without arming the next would give a
        // hung process exactly one look and then go quiet — `rearm_member_window`'s own argument
        // (nxf 6j6v.nf38), applied to the clock this item adds.
        let tmp = workspace(EXCLUSIVE_CODING);
        let (_coder, channel_thread) = a_round_stuck_on_a_live_session(&tmp);

        // The process is deliberately left alive: this is the hung half.
        let ticked =
            json_of(nxc(&tmp, RECHECK_AT).args(["--json", "tick", "--thread", &channel_thread]));
        assert_eq!(ticked["reason"], "waiting_for_a_session", "{ticked}");
        assert_eq!(
            armed_for(&tmp, &channel_thread).last().map(String::as_str),
            Some(AND_AGAIN_AT),
            "measured from the instant the re-check RAN, so the looking does not stop: {:?}",
            armed_for(&tmp, &channel_thread)
        );

        reap(&tmp);
    }
}

//! **Whoever puts a role under an obligation grants it the means to discharge it** — nxf 6j6v.kffm.
//!
//! The chain the acceptance run of nxf 6j6v.dbwn walked into, and the reason its first smoke came
//! back GREEN having never reached the model:
//!
//! 1. every role trigger that registers an expectation composes the forced ending into the system
//!    prompt — *end this task with `nxc reply --thread <id>`* (nxf 6j6v.enrs);
//! 2. the sidecar mapped the role's `tools:` onto the SDK's AUTO-APPROVAL list, so a role that
//!    declares no `tools:` at all — the default, and what every role written before the role runtime
//!    does — got the full base toolset and an EMPTY approval list;
//! 3. its own `nxc reply` came back "This command requires approval". The role COULD NOT do what it
//!    had been ordered to do;
//! 4. the teardown then settled the debt in its name. The expectation was discharged, the working
//!    copy was handed on, and from the outside the round looked healthy.
//!
//! **Nothing checked the pairing.** It had been fixed once before, per call site — nxf 6j6v.04es
//! granted the `summarize` synthesizer its `Bash` by hand — and it came straight back at the next
//! spawn path, which is what says that repair sat at the wrong height.
//!
//! So this file asserts the pairing at the two seams it is made of, and never by reading:
//!
//! * the ENGINE seam ([`Worker::trigger`]) — a request that carries an obligation carries the grant,
//!   whichever coordinator admitted it, and one that carries no obligation grants nothing;
//! * the SPEC the shipped sidecar actually reads — `grantedTools`, written by `SidecarWorker`.
//!   What the sidecar DOES with it (the union into the SDK's two separate lists) is
//!   `agent-sidecar/test/spec-helpers.test.mjs`, where those lists are observable.
//!
//! The mutation that puts the bug back is deleting `grant_the_means_for_the_obligation`, and every
//! test below goes red on it.

mod common;

use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::{RoleDecl, REPLY_OBLIGATION_TOOL};
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{
    RoleSpec, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup};
use tempfile::TempDir;

const NOW: &str = "2026-08-30T10:00:00Z";

/// Records every request the engine hands it, verbatim — the seam itself, with nothing built by
/// hand on the way in.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl Recorder {
    /// **ONE guard, and both answers derived from it.** The first cut locked again inside
    /// `unwrap_or_else` while the outer guard was still alive — `std::sync::Mutex` is not reentrant,
    /// so a lookup that FAILED deadlocked instead of panicking. That is the worst possible place for
    /// it: the hang only ever happens when a test is already failing, and a hung suite says nothing
    /// about which assertion broke. Found when its twin in `a_substituted_reply_says_so.rs` hung a
    /// whole test binary at 0% CPU (second review round of PR #397).
    fn for_role(&self, handle: &str) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .find(|r| r.role.handle == handle)
            .cloned()
            .unwrap_or_else(|| {
                let handles: Vec<&str> = seen.iter().map(|r| r.role.handle.as_str()).collect();
                panic!("no trigger for {handle}, only: {handles:?}")
            })
    }
}

impl Worker for Recorder {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
}

/// A role declared EXACTLY as the trap needs it: no `tools:` key at all. That is the default and
/// what every role file written before the role runtime looks like — the state whose auto-approval
/// list was empty.
fn undeclared_tools(handle: &str) -> RoleDecl {
    let decl: RoleDecl = serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses");
    assert_eq!(
        decl.tools, None,
        "the trap needs a role that declares NO tools — an empty list is a different state"
    );
    decl
}

fn declared_tools(handle: &str, tools: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\ntools: {tools}\n"
    ))
    .expect("test role parses")
}

fn team(
    tmp: &TempDir,
    roles: Vec<RoleDecl>,
    channels: Vec<ChannelDecl>,
) -> (Engine, Arc<Recorder>) {
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(roles, channels).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(Recorder::default());
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

fn send_to(engine: &Engine, to: &str) {
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to,
                body: "do the thing",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the target is addressed");
}

// ---- the ENGINE seam --------------------------------------------------------------------------

#[test]
fn a_persona_ordered_to_reply_is_granted_what_running_that_reply_takes() {
    // The trap's own shape: `nxc send --to <persona>` against a role that declares no `tools:`.
    // The trigger registers the expectation and the prompt orders the reply, so the request that
    // reaches the worker has to carry the means as well.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, vec![undeclared_tools("coder")], vec![]);
    send_to(&engine, "coder");

    let req = worker.for_role("coder");
    assert!(
        req.reply_thread.is_some(),
        "the premise: this trigger DEMANDS a reply — {req:?}"
    );
    assert!(
        req.role
            .granted_tools
            .iter()
            .any(|t| t == REPLY_OBLIGATION_TOOL),
        "…so it must also grant the means: {:?}",
        req.role.granted_tools
    );
    assert_eq!(
        req.role.tools, None,
        "and the role's own declaration is untouched — the grant is a SEPARATE list, so an \
         undeclared role keeps the SDK's full default base toolset instead of being narrowed to \
         the grant"
    );
}

#[test]
fn a_channel_member_ordered_to_reply_is_granted_it_too() {
    // The SECOND coordinator, and the reason the grant sits at the funnel rather than at a call
    // site: a member of a declared channel reaches the worker through the supervisor, not through
    // `send --to <persona>`. Fixing one entrance is what nxf 6j6v.04es did, and the defect came
    // back at the next one.
    let tmp = TempDir::new().unwrap();
    let review: ChannelDecl =
        serde_yaml::from_str("name: review\nmembers: [checker]\n").expect("channel parses");
    let (engine, worker) = team(&tmp, vec![undeclared_tools("checker")], vec![review]);
    send_to(&engine, "review");

    let req = worker.for_role("checker");
    assert!(req.reply_thread.is_some(), "{req:?}");
    assert!(
        req.role
            .granted_tools
            .iter()
            .any(|t| t == REPLY_OBLIGATION_TOOL),
        "{:?}",
        req.role.granted_tools
    );
}

#[test]
fn a_role_that_declared_its_own_tools_keeps_them_and_the_grant_stays_beside_them() {
    // The grant never rewrites a declaration. A role that declared `tools: [Read]` reaches the
    // worker with exactly that, plus a grant the sidecar unions in — which is what lets an
    // explicitly narrow role discharge an obligation without its author having had to foresee it.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, vec![declared_tools("narrow", "[Read]")], vec![]);
    send_to(&engine, "narrow");

    let req = worker.for_role("narrow");
    assert_eq!(req.role.tools, Some(vec!["Read".to_string()]));
    assert_eq!(
        req.role.granted_tools,
        vec![REPLY_OBLIGATION_TOOL.to_string()]
    );
}

#[test]
fn the_one_spawn_that_owes_nobody_an_answer_is_granted_nothing() {
    // The other direction, and it is what keeps this from being a blanket capability handout: the
    // grant is the MEANS FOR AN OBLIGATION, so a spawn that imposes none carries an empty list.
    //
    // The `summarize` synthesizer is that spawn, and it is the right one to assert on: it is the
    // very site nxf 6j6v.04es repaired BY HAND, it reaches the worker through the same funnel, and
    // it owes no thread an answer through the register — its instruction to post comes from the
    // channel's own `summary_prompt`, which is why its narrow `tools: [Bash]` declaration is the
    // whole of its toolset and has to stay so.
    let tmp = TempDir::new().unwrap();
    let review: ChannelDecl = serde_yaml::from_str(
        "name: review\nmembers: [checker]\non_complete: summarize\nsummary_prompt: Summarize it.\n",
    )
    .expect("channel parses");
    let (engine, worker) = team(&tmp, vec![undeclared_tools("checker")], vec![review]);
    send_to(&engine, "review");

    let member = worker.for_role("checker");
    engine
        .reply_thread(
            Caller {
                session: Some(&member.internal_session),
                actor: None,
                now: Some(NOW),
            },
            nexus_chat::surface::ReplyThreadRequest {
                machine: None,
                thread: member
                    .reply_thread
                    .as_deref()
                    .expect("the member owes an answer"),
                body: "looks fine",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply settles the set and starts the consolidation");

    let synth = worker.for_role(nexus_chat::channel::SYNTHESIS_HANDLE);
    assert_eq!(
        synth.reply_thread, None,
        "the premise: nothing is DEMANDED of the synthesizer through the register — {synth:?}"
    );
    assert!(
        synth.role.granted_tools.is_empty(),
        "…so nothing is granted to it either: {:?}",
        synth.role.granted_tools
    );
    assert_eq!(
        synth.role.tools,
        Some(vec!["Bash".to_string()]),
        "and its own narrow declaration is untouched — the per-call-site grant nxf 6j6v.04es made          is what it still runs on"
    );
}

// ---- the SPEC the shipped sidecar reads -------------------------------------------------------

#[test]
fn the_spec_the_sidecar_reads_carries_the_grant_as_its_own_key() {
    // The engine's half ends at the spec file; `agent-sidecar/test/spec-helpers.test.mjs` picks it
    // up from there. Two keys, not one, and that is the whole design: `tools` is the author's
    // declaration and drives the SDK's BASE toolset, `grantedTools` is the coordinator's and is
    // unioned into the AUTO-APPROVAL list — widening `tools` itself would have shrunk an undeclared
    // role's base toolset from "everything" to "Bash".
    let tmp = TempDir::new().unwrap();
    let worker = nexus_chat::worker::SidecarWorker {
        sidecar: tmp.path().join("no-such-sidecar.mjs"),
        cwd: tmp.path().to_path_buf(),
    };
    let _ = worker.trigger(TriggerRequest {
        internal_session: "s-grant".to_string(),
        resume_real: None,
        message: "go".to_string(),
        role: RoleSpec {
            handle: "coder".to_string(),
            system_prompt: "You are coder.".to_string(),
            use_claude_code_preset: false,
            tools: None,
            granted_tools: vec![REPLY_OBLIGATION_TOOL.to_string()],
            permissions: None,
            model: None,
            declaration_hash: None,
        },
        reply_thread: Some("th-1".to_string()),
        coordinator: nexus_chat::worker::Coordinator::Persona,
        terms: Default::default(),
        env: Default::default(),
    });

    let spec: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".nxs/agent-logs/s-grant.spec.json"))
            .expect("the spec is written before the spawn"),
    )
    .expect("valid json");
    assert_eq!(
        spec["grantedTools"],
        serde_json::json!([REPLY_OBLIGATION_TOOL]),
        "{spec}"
    );
    assert_eq!(
        spec["tools"],
        serde_json::Value::Null,
        "and the declaration stays what the author wrote — absent: {spec}"
    );
}

// ---- the live acceptance: a real session, a real model, a role that declares no tools ----------

/// **The one thing no deterministic test can show: the model actually answers** (nxf 6j6v.kffm).
///
/// Everything above proves the pieces line up — the grant reaches the worker, the spec file carries
/// it, `main.mjs` puts it in the SDK's approval list. What none of them can prove is the fact the
/// item is about: that a persona declared the way EVERY role written before the role runtime is
/// declared — no `tools:` key at all — reaches a real Claude session, runs its own
/// `nxc reply --thread <id>` through the SDK's Bash tool, and answers.
///
/// It is the shape that used to come back GREEN having never reached the model, so the assertions
/// are on the ANSWER and not on the state: a `sidecar:`-prefixed body and `substituted: true` are
/// exactly what a passing-looking failure produces here.
///
/// `#[ignore]`d for [`smoke_v3.rs`]'s reasons: a real SDK session, real subscription auth, real
/// wall-clock seconds. Run it by hand, from a checkout where `claude` is authenticated:
///
/// ```console
/// $ cargo build -p nxs
/// $ cargo test -p nexus-chat --test an_obligation_comes_with_its_means -- --ignored --nocapture
/// ```
#[test]
#[ignore = "live: spawns a real Claude Agent SDK session through the real sidecar"]
fn live_a_persona_that_declares_no_tools_answers_its_own_thread() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits at <root>/crates/chat")
        .to_path_buf();
    let sidecar = repo.join("agent-sidecar/src/main.mjs");
    assert!(sidecar.is_file(), "live run: no sidecar at {sidecar:?}");
    assert!(
        std::process::Command::new("sh")
            .args(["-c", "command -v claude"])
            .output()
            .is_ok_and(|o| o.status.success()),
        "live run: `claude` is not on PATH — the SDK needs the CLI, authenticated (run `claude` \
         once interactively first)"
    );
    assert!(
        std::env::var_os("ANTHROPIC_API_KEY").is_none(),
        "live run: ANTHROPIC_API_KEY is set; the SDK would prefer it over this machine's own \
         authenticated `claude` session"
    );
    nxs_test_support::assert_multicall_binary_fresh();
    let nxc = std::env::current_exe()
        .expect("the test binary has a path")
        .ancestors()
        .nth(2)
        .expect("the test binary lives in target/<profile>/deps")
        .join(format!("nxc{}", std::env::consts::EXE_SUFFIX));
    assert!(
        nxc.exists(),
        "the `nxc` argv[0] symlink is missing at {nxc:?}"
    );

    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    // NO `tools:` key. That is the whole premise, and it is asserted rather than trusted: an
    // accidental `tools: [Bash]` here would make this test pass for the wrong reason, which is the
    // one way a live smoke can go quietly hollow.
    let yaml = "handle: answerer\njob_title: Live acceptance persona\nsystem_prompt: |\n  \
                You are a persona in a live acceptance run. Read, write or change NO files.\n  \
                Run exactly one command: the `nxc reply --thread <id>` you were asked for, with\n  \
                the single word `ready` as its body. Then stop.\n";
    let decl: RoleDecl = serde_yaml::from_str(yaml).expect("the live role parses");
    assert_eq!(decl.tools, None, "the premise: this role declares no tools");
    std::fs::write(roles.join("answerer.yaml"), yaml).unwrap();

    let live = |args: &[&str]| {
        let mut c = std::process::Command::new(&nxc);
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env_remove("NXC_NOW")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    repo.join("target/debug").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_WORKER", "sidecar")
            // The clock is not what this proves, and the real backend would leave a one-shot job
            // behind in the runner's own login session, due against a `TempDir` that is gone.
            .env("NXC_TIMER", "dry")
            .env("NXC_SIDECAR", &sidecar)
            .args(args);
        let out = c.output().expect("nxc runs");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        serde_json::from_str::<serde_json::Value>(stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "no --json receipt from `nxc {}`: {e}\nstdout: {stdout}\nstderr: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    };

    let receipt = live(&[
        "--json",
        "send",
        "--to",
        "answerer",
        "--no-ref",
        "Acceptance run. Reply with the single word: ready.",
    ]);
    let thread = receipt["thread_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the send opened a thread: {receipt:#}"))
        .to_string();

    // Generously bounded: a real session is seconds to a couple of minutes, and the failure this
    // guards is a session that never answers at all — which without a bound is a hang, not a test.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let body = loop {
        let board = live(&["--json", "threads", "show", &thread]);
        let answer = board["messages"]
            .as_array()
            .expect("a thread carries messages")
            .iter()
            .find(|m| m["sender"].as_str() == Some("local/answerer"))
            .and_then(|m| m["body"].as_str())
            .map(str::to_string);
        if let Some(body) = answer {
            break body;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the persona never answered thread {thread}; the live board was:\n{board:#}"
        );
        std::thread::sleep(std::time::Duration::from_secs(3));
    };

    assert!(
        !body.starts_with("sidecar:"),
        "THE DEFECT, verbatim: the answer on this thread is the runtime's fallback for a session \
         that never posted one, not the model's — which is exactly what a role declaring no \
         `tools:` produced before nxf 6j6v.kffm, while every other field read like a healthy \
         round: {body}"
    );
    assert!(
        body.to_lowercase().contains("ready"),
        "…and it is the answer that was asked for: {body}"
    );

    let status = live(&["--json", "status", "--thread", &thread]);
    let row = status["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .flat_map(|op| op["threads"].as_array().expect("threads").iter())
        .find(|t| t["thread_id"] == thread.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("the thread is in the report: {status:#}"));
    assert_eq!(
        row["substituted"], false,
        "the AGENT answered, not the runtime standing in for it: {row:#}"
    );
    assert_eq!(row["escalated"], false, "{row:#}");

    // And the spec the sidecar was actually handed says why it could: the declaration is absent and
    // the grant is present, which is the two-key split this item exists for.
    let spec: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".nxs/agent-logs").join(format!(
            "{}.spec.json",
            receipt["session"].as_str().unwrap()
        )))
        .expect("the spec the live session ran on"),
    )
    .expect("valid json");
    assert_eq!(spec["tools"], serde_json::Value::Null, "{spec}");
    assert_eq!(
        spec["grantedTools"],
        serde_json::json!([REPLY_OBLIGATION_TOOL]),
        "{spec}"
    );
}

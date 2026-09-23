//! **A spawned persona is handed the suite's session start** (nxf 6j6v.k8zq), proven on a REAL
//! spawn rather than on a command's output.
//!
//! That distinction is the whole reason this file exists. `agent-sidecar/src/spec-helpers.mjs` sets
//! `settingSources: []` (SDK isolation, 6j6v.93hz), so no `.claude/settings.json` loads, so the
//! SessionStart hook never fires, so `nxs prime` — and `nxc prime` with it — has NEVER run inside a
//! persona session. Everything a spawned persona knew about `nxc` was four hand-written verbs in
//! `role.rs::nxc_usage_block`.
//!
//! nxf 6j6v.z6f9 is the measured cost of not checking this: it put the sentence "Asking and ending
//! your turn is safe: when the answer comes you are resumed" into `PERSONA_INSTRUCTIONS` and
//! accepted it against `nxc prime --persona <name>` — a path no spawned persona takes. The sentence
//! never reached the sessions whose self-invented memory ritual it was written to retire. So the
//! assertions below read the spec JSON the real `SidecarWorker` wrote to disk, which is the file the
//! model is actually started from.

use assert_cmd::Command;
use nexus_chat::workspace::{chat_config, setup};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-26T10:00:00Z";
const ORIGIN: &str = "local";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard, wired to the REAL `SidecarWorker` over a NONEXISTENT sidecar script:
/// `node` still spawns and fails fast, but the spec JSON is already on disk by then. The pattern
/// `channel_consolidator.rs` established, and the only way to observe a composed system prompt
/// exactly as the model would receive it.
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", ORIGIN)
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "sidecar")
        .env(
            "NXC_SIDECAR",
            tmp.path().join("does-not-exist").join("main.mjs"),
        );
    c
}

fn write_persona(tmp: &TempDir, handle: &str, extra: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!(
            "handle: {handle}\nsystem_prompt: THE ROLE PROMPT\njob_title: Coder\n\
             job_description: Implements features.\n{extra}"
        ),
    )
    .unwrap();
}

/// Every spec JSON the real `SidecarWorker` wrote in this workspace.
fn specs(tmp: &TempDir) -> Vec<Value> {
    std::fs::read_dir(tmp.path().join(".nxs/agent-logs"))
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with(".spec.json"))
                .map(|p| serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap())
                .collect()
        })
        .unwrap_or_default()
}

/// Summon `handle` and return the system prompt the sidecar was started with.
fn summon(tmp: &TempDir, handle: &str) -> String {
    human(tmp)
        .args(["send", "--no-ref", "--to", handle, "do the thing"])
        .assert()
        .success();
    let specs = specs(tmp);
    assert_eq!(specs.len(), 1, "one persona summoned: {specs:?}");
    specs[0]["systemPrompt"]
        .as_str()
        .expect("a composed system prompt")
        .to_string()
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success().get_output().clone();
    String::from_utf8(out.stdout).expect("utf8")
}

/// **THE ACCEPTANCE.** A spawned persona's system prompt carries the block, and it carries it as the
/// SAME BYTES `nxc prime --persona <handle>` prints — not a second assembly of the same pieces,
/// which is how two surfaces start teaching different things.
#[test]
fn a_spawned_persona_is_handed_the_same_block_the_prime_verb_prints() {
    let tmp = workspace();
    write_persona(&tmp, "coder", "");

    let composed = summon(&tmp, "coder");
    let printed = stdout_of(
        nxs_test_support::cargo_bin("nxc")
            .current_dir(tmp.path())
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", ORIGIN)
            .env("NXC_NOW", NOW)
            .args(["prime", "--consumer", "local/coder", "--persona", "coder"]),
    );
    assert!(
        composed.starts_with(printed.trim_end()),
        "the block must LEAD the prompt, byte for byte.\n--- printed ---\n{printed}\n\
         --- composed ---\n{composed}"
    );
    assert!(
        composed.ends_with("THE ROLE PROMPT"),
        "…and the role's own prompt still stands last:\n{composed}"
    );
}

/// What the block actually gives a persona that it never had: its own identity, and the command
/// reference nxf 6j6v.4mmk measured into shape (later cut further by nxf h4d3, task 3). Named
/// individually rather than as one prefix check, so a regression says WHICH of them went.
#[test]
fn the_block_carries_the_identity_and_escalation_but_never_withdraw() {
    let tmp = workspace();
    write_persona(&tmp, "coder", "");
    let composed = summon(&tmp, "coder");

    for expected in [
        // its own identity — the thing `nxc_usage_block` never carried
        "## You are `coder`",
        "- **Role:** Coder",
        // how it answers, and that asking costs it nothing (nxf 6j6v.z6f9's sentence, which never
        // reached a spawned persona before this item)
        "Asking and ending your turn is safe",
        // what an agent does when it cannot or may not settle something — the way UP, in the
        // "## How a conversation moves" shell-example style (`<id>`, not `<thread-id>`). This line
        // named `nxc withdraw --thread <id>` as the rescue verb for a held working copy until nxf
        // 6j6v.ezbr: `withdraw` is a person's verb since the owner's decision of 2026-09-20, and a
        // session this workspace started is refused it — see the absent list below.
        "nxc reply --thread <id> --escalate -",
    ] {
        assert!(
            composed.contains(expected),
            "the composed prompt must carry {expected:?}:\n{composed}"
        );
    }
    assert!(
        !composed.contains("How to use `nxc`:"),
        "…and NOT the hand-written four-verb block it replaced:\n{composed}"
    );
    // `threads show` did NOT survive task 3's cut (nxf h4d3) — it left with `list`, `status`,
    // `search` and `transcript show`, all five now taught on demand via `nxc --help`/`nxc guide`
    // instead. Nor did the when-stuck paragraph, which moved to `nxc guide limits-and-safety`.
    for absent in [
        "nxc threads show <thread-id>",
        "When something does not move",
        "working tree: holding",
    ] {
        assert!(
            !composed.contains(absent),
            "task 3 cut {absent:?} from every session start:\n{composed}"
        );
    }
    // …and the verb a session this workspace started is refused (nxf 6j6v.ezbr) is taught nowhere
    // in what it reads at its start — not as a command, not as a rescue, not in passing.
    assert!(
        !composed.contains("withdraw"),
        "a summoned persona is never offered `withdraw`:\n{composed}"
    );
}

/// `prime: false` still means what it meant: no block at all — and, just as before, it does NOT
/// opt out of the forced ending (nxf 6j6v.553s part 1), which is the engine's rule and not a
/// declaration's to drop. The new block obeys the old switch, on the real spawn path.
#[test]
fn prime_false_gets_no_block_but_keeps_the_forced_ending() {
    let tmp = workspace();
    write_persona(&tmp, "coder", "prime: false\n");
    let composed = summon(&tmp, "coder");
    assert!(
        composed.starts_with("Obligation: thread "),
        "the forced ending survives `prime: false`:\n{composed}"
    );
    assert!(
        composed.ends_with("THE ROLE PROMPT"),
        "…and the role's own prompt stands last:\n{composed}"
    );
    for absent in [
        "## You are `coder`",
        "## How a conversation moves",
        "Job title: Coder",
    ] {
        assert!(
            !composed.contains(absent),
            "an unprimed persona gets no block: {absent:?} is in\n{composed}"
        );
    }
}

/// The filter, on the spawn path and per persona: switching chat off takes the identity block away
/// and puts the declared identity back in its place, so a declaration cannot make a session
/// anonymous by switching a service off.
#[test]
fn a_persona_that_filters_chat_out_still_knows_who_it_is() {
    let tmp = workspace();
    write_persona(&tmp, "coder", "prime:\n  chat: false\n");
    let composed = summon(&tmp, "coder");
    assert!(
        !composed.contains("## You are `coder`")
            && !composed.contains("## How a conversation moves"),
        "chat's block is filtered out:\n{composed}"
    );
    assert!(
        composed.contains("Job title: Coder") && composed.contains("Job description: Implements"),
        "…and the declared identity stands in for it:\n{composed}"
    );
}

/// **The trap this item creates, sprung end to end** (nxf 6j6v.k8zq): a persona that filters the
/// memories out AND inherits no `CLAUDE.md` has the project's rules from nowhere at all — and
/// `memory: false` reads like "does not need memories". The prompt says so, in the session.
#[test]
fn a_persona_with_neither_memories_nor_claude_md_is_told_it_has_no_project_rules() {
    let tmp = workspace();
    write_persona(
        &tmp,
        "coder",
        "claude_md: ignore\nprime:\n  memory: false\n",
    );
    let composed = summon(&tmp, "coder");
    assert!(
        composed.contains("neither the project's memories nor its `CLAUDE.md`")
            && composed.contains("nxm prime"),
        "the session is told, rather than left to find out:\n{composed}"
    );
}

/// **The half of nxf 6j6v.9w08 that only a REAL spawn can prove.** The declaration warnings and the
/// `nxc guide writing-declarations` pointer are for whoever WRITES declarations, so they are
/// interactive-only — the same cut `personas.md` already states for referential errors ("a spawned
/// persona is not shown its author's mistakes; a human at a keyboard is").
///
/// Asserted here rather than against `nxc prime` with `NXC_ACTOR` set, for this file's own founding
/// reason: a persona's block is composed IN PROCESS by `facade::compose_persona_prime`, which never
/// consults the environment at all — it passes `false` itself. A test that only sniffed env would
/// have said nothing about the path a spawned persona actually takes, which is exactly how 6j6v.z6f9
/// accepted a sentence against a surface no persona reads.
#[test]
fn a_spawned_persona_is_not_shown_its_authors_declaration_warnings_or_the_guide_pointer() {
    let tmp = workspace();
    // A declaration that trips both checks at once: it rebuilds the engine's own answering rule,
    // and its `job_description` cannot tell a caller when to call it.
    let personas = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(
        personas.join("coder.yaml"),
        "handle: coder\njob_title: Coder\njob_description: Codes.\n\
         system_prompt: THE ROLE PROMPT. When done, `nxc reply --thread <id>`.\n",
    )
    .unwrap();

    let composed = summon(&tmp, "coder");
    for absent in [
        "## Declaration Warnings",
        "nxc guide writing-declarations",
        "the declaration spells",
    ] {
        assert!(
            !composed.contains(absent),
            "a spawned persona is not shown its author's mistakes: {absent:?} is in\n{composed}"
        );
    }

    // …and the same workspace, read by a human at the keyboard, carries both. Without this half the
    // assertions above would pass on a feature that never worked at all.
    let printed = stdout_of(
        nxs_test_support::cargo_bin("nxc")
            .current_dir(tmp.path())
            .env_remove("NXC_ACTOR")
            .env_remove("NXC_WORKER")
            .env_remove("NXC_SESSION")
            .env("NXC_ORIGIN", ORIGIN)
            .env("NXC_NOW", NOW)
            .args(["prime", "--consumer", "local/carsten"]),
    );
    assert!(
        printed.contains("## Declaration Warnings")
            && printed.contains("`nxc guide writing-declarations`")
            && printed.contains("the declaration spells `nxc reply`")
            && printed.contains("`job_description` is \"Codes.\""),
        "the author IS shown both, and what is wrong with which file:\n{printed}"
    );
}

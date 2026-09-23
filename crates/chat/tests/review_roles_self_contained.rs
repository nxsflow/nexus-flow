//! Regression guard for ticket `6j6v.93hz`'s mandate (already closed, binding going forward): no
//! review-quorum role's `system_prompt` may reference or depend on any plugin skill/slash command —
//! the SDK sessions these roles run in (headless `query()` calls) have no slash-command mechanism
//! to invoke one. Loads the four review-quorum role YAMLs shipped under
//! `examples/role-runtime-v2/.nxs-personas/` (`general`, `code-quality`, `test-quality`,
//! `integrity` — ticket `6j6v.zebd`) via [`nexus_chat::role::load_role`] and asserts none of their
//! `system_prompt` text contains a forbidden pattern.
//!
//! The second test proves the check is a real regression guard, not a tautology: it runs the
//! exact same detector against a deliberately non-self-contained fixture (the kind of prose a
//! naive verbatim copy-paste of a plugin's own skill-invocation framing would produce) and
//! confirms it is flagged.
//!
//! Extended per the FINAL whole-branch review of epic `6j6v.nmpk`: the same mandate applies to
//! `pm.yaml`, `coding.yaml`, and `workflow.yaml` too — `workflow.yaml`'s step `instructions` text
//! is folded directly into the PM/coder's own system prompt at trigger time via
//! [`nexus_chat::workflow::compose_workflow_block`], and that role runs with
//! `settingSources: []` (SDK isolation, `6j6v.93hz`) the same as the four review roles — so a
//! plugin/skill reference there is exactly as unreachable. This test file originally checked only
//! the four review roles, which is why a "the nxf-workflow review-pr skill's Step-5 shape" phrase
//! in `pm_summarize`'s instructions slipped through uncaught. One nuance: `pm.yaml`/`coding.yaml`/
//! `workflow.yaml` are legitimately ALLOWED to mention `nxf create` (the PM and coder ARE the
//! roles permitted to file tickets, per `6j6v.93hz`'s own ticket-creation policy) — only the four
//! review roles are forbidden from ever mentioning it — so these three files are checked with the
//! ticket-mention rule turned off, everything else (plugin/skill references, leading-slash
//! commands, `.nxf/review`/`status.jsonl` paths) still forbidden.

use nexus_chat::channel::load_all_channels;
use nexus_chat::role::load_role;
use std::path::{Path, PathBuf};

const REVIEW_ROLE_HANDLES: &[&str] = &["general", "code-quality", "test-quality", "integrity"];
const PM_CODING_ROLE_FILES: &[&str] = &["pm.yaml", "coding.yaml"];

fn personas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/role-runtime-v2/.nxs-personas")
}

// ---- v3 (nxf ticket 6j6v.3nha) -------------------------------------------------------------
//
// v3 extends the SAME `6j6v.93hz` self-containment mandate to `examples/role-runtime-v3/`: a
// separate, additive example (v2's own example/tests above are untouched). v3 adds two things v2
// never had to cover — a `channels.yaml` (the `review` quorum's `summary_prompt`) and the
// SDD-hardening content (brief `6j6v.3nha` §5): a per-reviewer merge verdict, anti-rubber-stamp
// discipline, and "critique the plan too". The self-containment DETECTOR functions themselves
// (`self_containment_violations`/`self_containment_violations_allowing_ticket_mentions`) are
// reused verbatim from above — no new detector logic — only new call sites pointed at v3's files,
// plus new (separate) content-presence checks for the SDD-hardening requirements, which have no
// v2 analogue to reuse.

const V3_PM_CODER_ROLE_FILES: &[&str] = &["pm.yaml", "coder.yaml"];

fn v3_personas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/role-runtime-v3/.nxs-personas")
}

/// Case-insensitive substring check for the new per-reviewer merge-verdict field (brief §5): "Ready
/// to merge? [yes | no | with-fixes]". Deliberately loose (just "ready to merge") so it doesn't
/// over-fit exact bracket/casing wording while still being a real content assertion, not "the file
/// is non-empty".
fn has_merge_verdict_field(text: &str) -> bool {
    text.to_lowercase().contains("ready to merge")
}

/// Case-insensitive substring check for the anti-rubber-stamp discipline paragraph (brief §5):
/// every reviewer prompt must name the concept explicitly (adapted wording is fine; the concept
/// itself, spelled with a hyphen or a space, must be present).
fn has_anti_rubber_stamp_language(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("rubber-stamp") || lower.contains("rubber stamp")
}

/// Case-insensitive substring check for "critique the plan, not just the implementation" (brief
/// §5's third requirement) — the instruction that a real design/approach flaw is in scope, not
/// just surface-level diff correctness.
fn has_critique_the_plan_instruction(text: &str) -> bool {
    text.to_lowercase().contains("critique the plan")
}

#[test]
fn v3_all_six_role_files_are_self_contained() {
    // Mirrors `review_quorum_roles_are_self_contained` (strict variant) for the four reviewers...
    for handle in REVIEW_ROLE_HANDLES {
        let path = v3_personas_dir().join(format!("{handle}.yaml"));
        let role = load_role(&path).unwrap_or_else(|e| panic!("loading role file {path:?}: {e}"));
        assert_eq!(
            role.handle, *handle,
            "role file {path:?} must declare handle: {handle}"
        );
        let violations = self_containment_violations(&role.system_prompt);
        assert!(
            violations.is_empty(),
            "v3 {handle}.yaml's system_prompt must be self-contained but found: \
{violations:?}\n---\n{}",
            role.system_prompt
        );
    }
    // ...and `pm_coding_and_workflow_roles_are_self_contained` (ticket-mention-allowing variant)
    // for pm/coder, which ARE allowed to mention `nxf create` per `6j6v.93hz`'s own policy.
    for file in V3_PM_CODER_ROLE_FILES {
        let path = v3_personas_dir().join(file);
        let role = load_role(&path).unwrap_or_else(|e| panic!("loading role file {path:?}: {e}"));
        let violations = self_containment_violations_allowing_ticket_mentions(&role.system_prompt);
        assert!(
            violations.is_empty(),
            "v3 {file}'s system_prompt must be self-contained but found: \
{violations:?}\n---\n{}",
            role.system_prompt
        );
    }
}

/// New to v3 (v2 declared no channels at all): `channels.yaml`'s `review` channel's
/// `summary_prompt` is folded into the ephemeral synthesizer's own trigger message
/// (`channel::compose_synthesis_trigger`) and run through the exact same headless-SDK, no-slash-
/// command constraints as any role's `system_prompt` — so it must pass the identical
/// self-containment check. Uses the strict variant (no ticket-mention allowance): the review
/// synthesis is not one of the two roles `6j6v.93hz` permits to mention `nxf create`.
#[test]
fn v3_channels_summary_prompt_is_self_contained() {
    let channels = load_all_channels(&v3_personas_dir())
        .unwrap_or_else(|e| panic!("loading v3 channels.yaml: {e}"));
    let review = channels
        .iter()
        .find(|c| c.name == "review")
        .expect("v3 channels.yaml must declare a review channel");
    let prompt = review
        .summary_prompt
        .as_deref()
        .expect("review channel must declare a summary_prompt");
    let violations = self_containment_violations(prompt);
    assert!(
        violations.is_empty(),
        "v3 channels.yaml's review summary_prompt must be self-contained but found: \
{violations:?}\n---\n{prompt}"
    );
}

// `v3_workflow_steps_are_self_contained` stood here — the same guard pointed at the example's
// declared workflow steps. REMOVED with the run engine (6j6v.dvyq §3): nothing composes a step's
// `instructions` into a prompt any more, so there is no text left to hold to the rule.

/// The new SDD-hardening content (brief §5) — genuinely absent from every v2 reviewer prompt, and
/// the whole point of this ticket's "hardening" half: each of the four reviewer prompts must carry
/// a merge verdict field, anti-rubber-stamp discipline, and the "critique the plan too" instruction.
/// A real content assertion (not "the file is non-empty") — see the next test for proof these
/// helpers are genuine detectors, not tautologies.
#[test]
fn v3_reviewer_prompts_contain_the_sdd_hardening_content() {
    for handle in REVIEW_ROLE_HANDLES {
        let path = v3_personas_dir().join(format!("{handle}.yaml"));
        let role = load_role(&path).unwrap_or_else(|e| panic!("loading role file {path:?}: {e}"));
        assert!(
            has_merge_verdict_field(&role.system_prompt),
            "{handle}.yaml must declare a 'Ready to merge?' verdict field"
        );
        assert!(
            has_anti_rubber_stamp_language(&role.system_prompt),
            "{handle}.yaml must state anti-rubber-stamp discipline"
        );
        assert!(
            has_critique_the_plan_instruction(&role.system_prompt),
            "{handle}.yaml must instruct critiquing the plan, not just the implementation"
        );
    }
}

/// Proves the three SDD-hardening helpers above are real detectors, not vacuous ones: run them
/// against a fixture shaped like a pre-hardening (v2-style) reviewer prompt — structured
/// Grade/Justification/Findings output, but none of v3's new requirements — and confirm every one
/// is correctly reported as MISSING.
#[test]
fn sdd_hardening_check_catches_a_plausible_pre_hardening_fixture() {
    let pre_hardening_fixture = "\
You are a code quality reviewer.

## Output Format

**Grade: [A/B/C/D/F]**

**Justification:** [1-2 sentences]

**Findings:**

1. [Severity] [title] - [description]

If there are no findings, write \"No issues found.\"
";
    assert!(
        !has_merge_verdict_field(pre_hardening_fixture),
        "pre-hardening fixture has no verdict field and must be reported as missing"
    );
    assert!(
        !has_anti_rubber_stamp_language(pre_hardening_fixture),
        "pre-hardening fixture has no anti-rubber-stamp language and must be reported as missing"
    );
    assert!(
        !has_critique_the_plan_instruction(pre_hardening_fixture),
        "pre-hardening fixture has no critique-the-plan instruction and must be reported as missing"
    );
}

/// True if `text` contains a `/`-prefixed slash-command reference such as `/review-pr` or
/// `/mitigate-pr-review-findings` — a `/` immediately followed by a letter, with no word
/// character immediately before it. This deliberately does NOT flag a `/` used as a path
/// separator (`crates/chat/src/cli.rs`, always preceded by a word char) or as a prose separator
/// (`off-by-one / null`, `` `test/` / `tests/` ``, always spaced on both sides) — both patterns
/// appear legitimately in the ported review-dimension content.
///
/// Best-effort, not a proof (final independent review of PR #254, Code Quality #2): a
/// parenthesized reference (`(/foo)`) or one preceded by punctuation other than a word
/// character still matches and is caught, but this is still a pattern heuristic, not an
/// exhaustive parse — a future author extending the forbidden-pattern list should keep that
/// in mind rather than treating a clean run here as a formal guarantee.
fn has_slash_command_reference(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        if b != b'/' {
            return false;
        }
        let prev_is_word = i > 0 && (bytes[i - 1] as char).is_alphanumeric();
        let next_is_letter = i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic();
        !prev_is_word && next_is_letter
    })
}

/// Forbidden patterns per `6j6v.93hz` + the ticket-policy note (review roles never create
/// tickets/artifacts). Returns a human-readable label per violation found, empty when clean.
fn self_containment_violations(system_prompt: &str) -> Vec<&'static str> {
    self_containment_core(system_prompt, true)
}

/// Same detector, but with the "nxf create" ticket-mention rule turned OFF: `pm.yaml`/
/// `coding.yaml`/`workflow.yaml` are legitimately allowed to mention `nxf create` (the PM and
/// coder ARE the roles permitted to file tickets, per `6j6v.93hz`'s own ticket-creation policy —
/// only the four review roles are forbidden from ever mentioning it). Every other rule — plugin/
/// skill references, leading-slash commands, `.nxf/review`/`status.jsonl` paths — is universal
/// and stays on.
fn self_containment_violations_allowing_ticket_mentions(text: &str) -> Vec<&'static str> {
    self_containment_core(text, false)
}

fn self_containment_core(text: &str, forbid_ticket_mentions: bool) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut hits = Vec::new();
    if has_slash_command_reference(text) {
        hits.push("a leading-slash slash-command reference");
    }
    if lower.contains("plugin") {
        hits.push("the literal word 'plugin'");
    }
    if lower.contains("skill") {
        hits.push("the literal word 'skill'");
    }
    if lower.contains(".nxf/review") {
        hits.push("a mention of .nxf/review");
    }
    if lower.contains("status.jsonl") {
        hits.push("a mention of status.jsonl");
    }
    if forbid_ticket_mentions && (lower.contains("nxf create") || lower.contains("nxf_create")) {
        hits.push("a mention of nxf create/nxf_create");
    }
    hits
}

#[test]
fn review_quorum_roles_are_self_contained() {
    for handle in REVIEW_ROLE_HANDLES {
        let path = personas_dir().join(format!("{handle}.yaml"));
        let role = load_role(&path).unwrap_or_else(|e| panic!("loading role file {path:?}: {e}"));
        assert_eq!(
            role.handle, *handle,
            "role file {path:?} must declare handle: {handle}"
        );
        let violations = self_containment_violations(&role.system_prompt);
        assert!(
            violations.is_empty(),
            "{handle}.yaml's system_prompt must be self-contained (ticket 6j6v.93hz) but found: \
{violations:?}\n---\n{}",
            role.system_prompt
        );
    }
}

/// Extends the same `6j6v.93hz` mandate to `pm.yaml`, `coding.yaml`, and `workflow.yaml` (final
/// whole-branch review of epic `6j6v.nmpk`): none of these ever ran through this guard before,
/// which is exactly how `pm_summarize`'s "the nxf-workflow review-pr skill's Step-5 shape" phrase
/// slipped through. `pm.yaml`/`coding.yaml` are role files with a `system_prompt`, checked via
/// `load_role` same as the review roles. `workflow.yaml` has no `system_prompt` — its per-step
/// `instructions` are what actually gets folded into the PM/coder's system prompt at trigger time
/// (`compose_workflow_block`), so that's what's checked here. All three
/// use the ticket-mention-allowing variant of the detector (see
/// `self_containment_violations_allowing_ticket_mentions`'s doc comment for why).
#[test]
fn pm_and_coding_roles_are_self_contained() {
    for file in PM_CODING_ROLE_FILES {
        let path = personas_dir().join(file);
        let role = load_role(&path).unwrap_or_else(|e| panic!("loading role file {path:?}: {e}"));
        let violations = self_containment_violations_allowing_ticket_mentions(&role.system_prompt);
        assert!(
            violations.is_empty(),
            "{file}'s system_prompt must be self-contained (ticket 6j6v.93hz) but found: \
{violations:?}\n---\n{}",
            role.system_prompt
        );
    }

    // The declared workflow's step `instructions` were held to the same rule here until
    // 6j6v.dvyq §3 removed the run engine and the declaration with it. What a persona is told about
    // the work it is part of now comes from its own `system_prompt`, checked above, and from the
    // channel it was summoned through.
}

/// Proves the ticket-mention-allowing variant is still a real guard for pm/coding/workflow content
/// (not weakened into a tautology by turning off the "nxf create" rule) AND that it does not
/// false-positive on the legitimate `nxf create` mentions those files actually contain: a fixture
/// shaped like the CURRENT (fixed) `pm_summarize` step — including its genuine "does NOT file any
/// `nxf create` tickets" clause — plus a re-introduced plugin/skill reference of the exact kind
/// that slipped through pre-fix (`"the nxf-workflow review-pr skill's Step-5 shape"`) must catch
/// the skill reference while leaving the `nxf create` mention unflagged.
#[test]
fn pm_coding_workflow_check_catches_a_plausible_violation_but_allows_nxf_create() {
    let fixture = "\
Read every reviewer's individual result, then render ONE unified summary in the nxf-workflow
review-pr skill's Step-5 shape. This step does NOT file any `nxf create` tickets itself — the
checklist is a mitigation REQUEST handed to the coder.
";
    let violations = self_containment_violations_allowing_ticket_mentions(fixture);
    assert!(
        violations.contains(&"the literal word 'skill'"),
        "expected the plugin-skill reference to be caught, got: {violations:?}"
    );
    assert!(
        !violations.contains(&"a mention of nxf create/nxf_create"),
        "pm/coding/workflow's legitimate `nxf create` mention must NOT be flagged, got: \
{violations:?}"
    );
}

/// Proves `self_containment_violations` is a real detector, not a vacuous one: run it against a
/// fixture shaped like a naive verbatim copy-paste of a plugin skill's own invocation framing
/// (the mistake ticket 6j6v.93hz forbids) and confirm each distinct violation is caught.
#[test]
fn self_containment_check_catches_a_plausible_violation() {
    let naive_copy_paste = "\
You are a code quality reviewer, part of the review-pr plugin.

Fetch the full prompt from the skill's bundled prompts/code-quality.md, then run
/review-pr to gather the diff and quality-gates log the orchestrating skill prepares for you.

Once findings are ready, file them with `nxf create --type issue --title \"…\"` and log the
outcome to .nxf/review/ + status.jsonl for mitigate-pr-review-findings to pick up later.

## Output Format
**Grade: [A/B/C/D/F]**
";

    let violations = self_containment_violations(naive_copy_paste);
    assert!(
        violations.contains(&"a leading-slash slash-command reference"),
        "expected the /review-pr invocation to be caught, got: {violations:?}"
    );
    assert!(
        violations.contains(&"the literal word 'plugin'"),
        "expected the word 'plugin' to be caught, got: {violations:?}"
    );
    assert!(
        violations.contains(&"the literal word 'skill'"),
        "expected the word 'skill' to be caught, got: {violations:?}"
    );
    assert!(
        violations.contains(&"a mention of .nxf/review"),
        "expected .nxf/review to be caught, got: {violations:?}"
    );
    assert!(
        violations.contains(&"a mention of status.jsonl"),
        "expected status.jsonl to be caught, got: {violations:?}"
    );
    assert!(
        violations.contains(&"a mention of nxf create/nxf_create"),
        "expected nxf create to be caught, got: {violations:?}"
    );
}

/// Companion sanity check: legitimate prose that merely *uses* a `/` as a path separator or a
/// list-style alternation must NOT be flagged — otherwise the guard above would be too blunt to
/// survive on the ported review-dimension content, which contains both patterns.
#[test]
fn self_containment_check_does_not_flag_legitimate_slash_usage() {
    let legitimate = "\
See crates/chat/src/cli.rs for the grammar. Off-by-one / null / async errors matter.
Directories like `test/` / `tests/` / `__tests__/` / `spec/` are all valid test homes.
";
    let violations = self_containment_violations(legitimate);
    assert!(
        violations.is_empty(),
        "legitimate path/list slash usage must not trip the guard, got: {violations:?}"
    );
}

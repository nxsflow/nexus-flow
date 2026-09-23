//! Quality warnings over a workspace's declarations (nxf 6j6v.9w08) — the checkable part of
//! `nxc guide writing-declarations`.
//!
//! **A second stage beside [`crate::channel::validate_channels`], not an extension of it.** That
//! function answers "does this declaration RESOLVE" and its findings are
//! [`ValidationError`](crate::channel::ValidationError)s: a channel with one excludes every channel
//! its file declares from the prime roster. These findings answer "is this declaration written so
//! that it works", they exclude nothing, and a declaration that trips one loads and runs exactly as
//! before. Two questions, two types, so no caller can treat one as the other by accident.
//!
//! **Why there is a checker at all, when most of the seven error classes are judgements.** The guide
//! topic is the substance; these three checks are the sliver of it a file can be held to on its
//! own — and only two of them are genuinely DECIDABLE (engine text is spelled or it is not; a
//! routing surface is declared or it is not). The third, a stub-length description, is a heuristic
//! floor and says so at [`ROUTING_SURFACE_MIN_CHARS`]. They are worth having because the class they
//! catch is the one that happens to people who know about it. The item's own note records the case: the person specifying this ticket wrote
//! eleven good practices into a `process-designer` persona's prompt on the same day — which IS
//! error class 1, made inside the persona that exists to warn about it, and caught within the hour
//! by somebody reading the file rather than by anything that ran.
//!
//! **What is deliberately NOT checked**, because a rule would be wrong more often than the author:
//! project knowledge nailed into a role, an `expected_output` that forbids its own answer, a
//! nailed-down language, and a diluting round size — classes 4 to 7 of the guide. All four are
//! real and all four need a reader. A warning that cries wolf trains its reader to skim, which
//! costs more than the four it would catch, so they live in the guide and in its checklist
//! instead.
//!
//! **Where they surface:** `nxs prime`, in an interactive context only, beside the referential
//! errors and under the same rule — a spawned persona is not shown its author's mistakes, a human
//! at a keyboard is ([`crate::facade::PrimeReport::render_markdown`]).

use crate::channel::ChannelDecl;
use crate::role::RoleDecl;

/// One quality finding about a declaration, in [`ValidationError`](crate::channel::ValidationError)'s
/// `{file, what}` shape so the renderer beside it needs no second layout — and a DISTINCT type, so
/// the compiler refuses to let a warning reach the list that partitions the roster.
///
/// `file` is the declaration surface as a reader meets it: `channels.yaml` for a channel (the one
/// file [`crate::channel::load_all_channels`] reads), and `<handle>.yaml` for a persona. The second
/// is DERIVED from the handle rather than discovered: `load_all_roles` returns declarations, not
/// paths, and the filename stem is the folder's convention while the `handle:` field is the source
/// of truth. A persona whose file is named otherwise is therefore named here by its handle, which
/// is still the thing to search the folder for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationWarning {
    pub file: String,
    pub what: String,
}

const CHANNELS_FILE: &str = "channels.yaml";

/// HOW FAR one engine-written invocation reaches — which is the whole of whether a declaration
/// that spells it out is a duplicate or is doing the engine's job for it.
///
/// **The distinction is load-bearing, and bundling it away was a real defect** (review of PR #465,
/// Code Quality #1, High). The first cut gated every `nxc` invocation on `prime.chat` in one
/// clause, reasoning that a role denied [`crate::persona::PERSONA_INSTRUCTIONS`] has to teach
/// itself how to answer. That holds for `send` and `list` and is FALSE for `reply`: the forced
/// ending is composed by [`crate::role`] whatever the declaration says, because the engine depends
/// on it — this crate's own `prime_false_gets_no_block_but_keeps_the_forced_ending` is the proof.
/// So a `prime: false` role spelling out `nxc reply` was the exact class-1 copy this module exists
/// to catch, and the check was looking the other way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Written into EVERY turn that owes a reply, `prime: false` included — the forced ending.
    Always,
    /// Written at session start by [`crate::persona::PERSONA_INSTRUCTIONS`], so only into a
    /// persona primed with chat.
    WhenChatIsPrimed,
    /// Taught by `nxm prime`'s own "How a memory is written" block, so only where the memories are
    /// primed in.
    WhenMemoryIsPrimed,
}

impl Reach {
    /// Whether a persona declaring `services` is handed this invocation by the engine anyway.
    fn reaches(self, services: crate::role::PrimeServices) -> bool {
        match self {
            Reach::Always => true,
            Reach::WhenChatIsPrimed => services.chat,
            Reach::WhenMemoryIsPrimed => services.memory,
        }
    }
}

/// Every invocation the engine writes into a session itself, with the reach that decides when a
/// declaration repeating it is a copy. ONE table, so the two callers below cannot disagree about
/// the set, and the order here is the order a warning names them in.
///
/// Spelled with the binary so the terms cannot match ordinary prose: `reply`, `send` and `list` are
/// among the most ordinary words a declaration contains.
///
/// Deliberately absent: `nxc guide` / `nxc status` / `nxc threads show`, and every `nxm` verb
/// outside the three the prime block teaches (`nxm guide`, `nxm index`, …). Pointing a role at a
/// guide is not a copy of anything, and a verb the engine does not inject is one a declaration ADDS
/// rather than duplicates.
const ENGINE_INVOCATIONS: &[(&str, Reach)] = &[
    ("nxc send", Reach::WhenChatIsPrimed),
    ("nxc reply", Reach::Always),
    ("nxc list", Reach::WhenChatIsPrimed),
    ("nxm remember", Reach::WhenMemoryIsPrimed),
    ("nxm recall", Reach::WhenMemoryIsPrimed),
    ("nxm memories", Reach::WhenMemoryIsPrimed),
];

/// A LENGTH FLOOR under the routing surface, and no more than that — a description this short
/// cannot carry a situation and a boundary, so it is a stub rather than a sentence.
///
/// **It is a heuristic, not a decision, and the honest limits are these** (review of PR #465, Code
/// Quality #2 and Integrity #2). It cannot tell a complete short description from a stub —
/// `"Reviews PRs; no merges."` is 23 characters and would trip — and it cannot see that a long one
/// says nothing, which is the "package insert" half of guide class 3 and is left entirely to the
/// reader. A description padded to length with zero-width characters clears it too; declarations
/// are self-authored, so that is a limit rather than a hole, and the guide rule is what actually
/// covers this class.
///
/// What it IS good for is the case it was MEASURED against: the shortest `job_description` shipped
/// anywhere in this repository is 41 characters ("Hunts silent failures and unhandled edges.",
/// `examples/role-runtime-v3`) and the shortest channel `description` is 44, so 24 sits under every
/// real one by a wide margin and fires only on text that is not a sentence at all. A warning that
/// fired on a real description would teach its reader to skim past the stub it exists for, which
/// costs more than the stub does.
const ROUTING_SURFACE_MIN_CHARS: usize = 24;

/// Every quality finding over one workspace's declarations: each role's, in the order
/// [`crate::role::load_all_roles`] returns them (handle-sorted), then each channel's in declaration
/// order. Pure — no I/O, no notion of interactive-vs-spawned, exactly like
/// [`crate::channel::validate_channels`] beside it.
pub fn warn_declarations(roles: &[RoleDecl], channels: &[ChannelDecl]) -> Vec<DeclarationWarning> {
    let mut warnings = Vec::new();
    for role in roles {
        warn_role(role, &mut warnings);
    }
    for channel in channels {
        warn_channel(channel, &mut warnings);
    }
    warnings
}

/// One persona's findings: engine text copied into it (guide class 1), then a routing surface that
/// cannot route (guide class 3).
fn warn_role(role: &RoleDecl, out: &mut Vec<DeclarationWarning>) {
    let file = format!("{}.yaml", role.handle);
    let text = [
        Some(role.system_prompt.as_str()),
        role.job_description.as_deref(),
        role.expected_output.as_deref(),
    ];

    // **Gated PER INVOCATION on what this persona is actually handed**, not per binary — see
    // [`Reach`] for the defect that distinction fixes. A role declared `prime: false` teaches
    // itself how to `send` and `list`, so spelling those out is the declaration doing the job the
    // engine was told not to do; the forced ending reaches it regardless, so spelling `nxc reply`
    // out is a copy whatever it declared.
    let services = role.prime.services();
    let admitted: Vec<&str> = ENGINE_INVOCATIONS
        .iter()
        .filter(|(_, reach)| reach.reaches(services))
        .map(|(invocation, _)| *invocation)
        .collect();
    let found = spelled(&text, &admitted);
    if !found.is_empty() {
        out.push(DeclarationWarning {
            file: file.clone(),
            what: format!(
                "the declaration spells {} — the engine supplies that already, in this persona's \
                 own session start and in the forced ending of every turn. A copy drifts, and the \
                 copy is the half nobody maintains",
                quoted(&found)
            ),
        });
    }

    match role.job_description.as_deref().map(str::trim) {
        None => out.push(routing_surface(&file, "`job_description` is not declared")),
        Some(text) if text.chars().count() < ROUTING_SURFACE_MIN_CHARS => out.push(
            routing_surface(&file, &format!("`job_description` is {:?}", text)),
        ),
        Some(_) => {}
    }

    // **The DEPRECATED channel list** (nxf 6j6v.g0yn). It still parses and still means "nobody
    // directly", so nothing breaks — but it is the second of the two places a channel's cast used
    // to be written down, and the one that could disagree with the channel itself. A warning and
    // not an error, deliberately: every declaration written before this change carries it.
    let via = role.addressable.channels();
    if !via.is_empty() {
        out.push(DeclarationWarning {
            file: file.clone(),
            what: format!(
                "`addressable: [{}]` is the old spelling of \"nobody directly\" — write \
                 `addressable: none` instead and let the channel that casts this persona be the \
                 route. Naming the channels here is the second place a channel's cast is written \
                 down, and the one that can disagree with the channel",
                via.join(", ")
            ),
        });
    }
}

/// One channel's findings: engine text copied into anything it declares that reaches a model (guide
/// class 1), a step target repeated under `members:` (nxf 6j6v.g0yn), then a routing surface that
/// cannot route (guide class 3).
fn warn_channel(channel: &ChannelDecl, out: &mut Vec<DeclarationWarning>) {
    // **The redundancy the derived cast leaves behind** (nxf 6j6v.g0yn). A step target IS in the
    // channel now ([`crate::channel::cast`]), so repeating it under `members:` says nothing — and
    // the two lists are then two places one fact is written, which is what the ticket set out to
    // end. A warning rather than the error the item proposed: the union means every existing
    // declaration keeps working, and refusing them would be a migration nothing else needs.
    let repeated: Vec<&str> = channel
        .members
        .iter()
        .filter(|m| channel.steps.iter().any(|s| &&s.target == m))
        .map(String::as_str)
        .collect();
    if !repeated.is_empty() {
        out.push(DeclarationWarning {
            file: CHANNELS_FILE.to_string(),
            what: format!(
                "channel {:?} lists {} under `members:` as well as in its steps — a step \
                 target already takes part, so the member entry adds nothing. `members:` is for a \
                 seat no step names (a person following the round)",
                channel.name,
                quoted(&repeated)
            ),
        });
    }

    // Everything a channel declares that a model reads: what a caller is shown, what the
    // synthesizer is given, and what each step hands its target.
    let mut text: Vec<Option<&str>> = vec![
        channel.description.as_deref(),
        channel.summary_prompt.as_deref(),
    ];
    text.extend(channel.steps.iter().map(|s| s.task.as_deref()));

    // Every `nxc` invocation in the table, whatever its [`Reach`], and no `nxm` one — DERIVED from
    // the same list rather than restated, because a second copy of the set is the defect this
    // module is named for. A channel has no `prime:` for an author to have switched off: the
    // ephemeral synthesizer is told the one delivery command it may run whatever the channel
    // declares, and a member is handed the forced ending whatever IT declares. `nxm` is absent for
    // the mirror-image reason — the synthesizer gets no memory block, so a `summary_prompt` naming
    // `nxm recall` is adding rather than duplicating.
    let admitted: Vec<&str> = ENGINE_INVOCATIONS
        .iter()
        .map(|(invocation, _)| *invocation)
        .filter(|invocation| invocation.starts_with("nxc "))
        .collect();
    let found = spelled(&text, &admitted);
    if !found.is_empty() {
        out.push(DeclarationWarning {
            file: CHANNELS_FILE.to_string(),
            what: format!(
                "channel {:?} spells {} — the engine supplies that already, to the synthesizer and \
                 to every member it commissions. A copy drifts, and the copy is the half nobody \
                 maintains",
                channel.name,
                quoted(&found)
            ),
        });
    }

    // The same rule as a persona's `job_description`, deliberately: it is the same surface, read by
    // the same caller in the same list, and a channel entry with nothing after its members is the
    // case that made this worth checking — a round nobody can tell from another round.
    let what = match channel.description.as_deref().map(str::trim) {
        None => format!("channel {:?} declares no `description`", channel.name),
        Some(text) if text.chars().count() < ROUTING_SURFACE_MIN_CHARS => format!(
            "channel {:?} has the `description` {:?}",
            channel.name, text
        ),
        Some(_) => return,
    };
    out.push(routing_surface(CHANNELS_FILE, &what));
}

/// The shared second half of warnings (b) and (c): WHY this field is worth a sentence of thought.
/// One wording, because it is one fact about one rendering — `Directory::render_markdown` puts a
/// persona's `job_description` and a channel's `description` in the same place, for the same
/// reader, at the same moment.
fn routing_surface(file: &str, what: &str) -> DeclarationWarning {
    DeclarationWarning {
        file: file.to_string(),
        what: format!(
            "{what} — that is the one thing a calling agent is shown when it decides whom to \
             address"
        ),
    }
}

/// Which of `needles` appears in any of `haystacks`, in `needles`' own order and without repeats —
/// so one declaration produces one warning naming every invocation it copied, rather than one
/// warning per occurrence.
fn spelled<'a>(haystacks: &[Option<&str>], needles: &[&'a str]) -> Vec<&'a str> {
    needles
        .iter()
        .filter(|needle| {
            haystacks
                .iter()
                .flatten()
                .any(|text| text.contains(**needle))
        })
        .copied()
        .collect()
}

/// `` `a`, `b` `` — the invocations as a reader meets them in the declaration, in code spans,
/// matching how every other line of the prime block spells a command.
fn quoted(found: &[&str]) -> String {
    found
        .iter()
        .map(|f| format!("`{f}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(yaml: &str) -> RoleDecl {
        serde_yaml::from_str(yaml).expect("role parses")
    }

    fn channel(yaml: &str) -> ChannelDecl {
        serde_yaml::from_str(yaml).expect("channel parses")
    }

    /// A declaration that says nothing the engine says, and describes itself well enough to be
    /// chosen from a list, produces NOTHING. The silent case is the one a warning has to get right
    /// first: everything below is a departure from it.
    #[test]
    fn a_well_written_pair_warns_about_nothing() {
        let roles = [role(
            "handle: coder\n\
             job_description: Implements a work order on a branch and merges it.\n\
             system_prompt: |\n  \
             You are the coder. Do what the message asks, then say what you did.\n",
        )];
        let channels = [channel(
            "name: review\n\
             members: [coder]\n\
             description: the review quorum — one round, one verdict back\n",
        )];
        assert_eq!(warn_declarations(&roles, &channels), vec![]);
    }

    /// Guide class 1, the persona half: the three `nxc` invocations `PERSONA_INSTRUCTIONS` already
    /// carries are named ONCE, in the list's own order, however many fields spelled them.
    #[test]
    fn a_persona_that_rebuilds_the_answering_rules_is_named_with_the_verbs_it_copied() {
        let roles = [role(
            "handle: coder\n\
             job_description: Implements a work order on a branch and merges it.\n\
             expected_output: Answer with `nxc reply`.\n\
             system_prompt: |\n  \
             When you are done, `nxc reply --thread <id>`. `nxc list` shows who is there.\n",
        )];
        let warnings = warn_declarations(&roles, &[]);
        assert_eq!(warnings.len(), 1, "one per declaration: {warnings:?}");
        assert_eq!(warnings[0].file, "coder.yaml");
        assert!(
            warnings[0].what.contains("`nxc reply`, `nxc list`"),
            "names what it found, in the list's order, deduplicated: {}",
            warnings[0].what
        );
        assert!(
            !warnings[0].what.contains("`nxc send`"),
            "and names nothing it did not find: {}",
            warnings[0].what
        );
    }

    /// The gate is PER INVOCATION, not per binary: a role the engine does not prime still has to be
    /// told how to `send` and `list`, so writing those out is the declaration taking over a job the
    /// engine was told to drop — not a copy.
    #[test]
    fn a_persona_that_opted_out_of_priming_may_spell_the_session_start_commands_itself() {
        let opted_out = [role(
            "handle: coder\n\
             prime: false\n\
             job_description: Implements a work order on a branch and merges it.\n\
             system_prompt: |\n  \
             Start something with `nxc send --to pm`, and `nxc list` shows who is there.\n",
        )];
        assert_eq!(warn_declarations(&opted_out, &[]), vec![]);

        // …and the same file with chat priming back on is warned about, so the assertion above is
        // about the gate rather than about some other difference.
        let primed = [role(
            "handle: coder\n\
             prime:\n  chat: true\n\
             job_description: Implements a work order on a branch and merges it.\n\
             system_prompt: |\n  \
             Start something with `nxc send --to pm`, and `nxc list` shows who is there.\n",
        )];
        assert_eq!(warn_declarations(&primed, &[]).len(), 1);
    }

    /// **`nxc reply` is the one that is NEVER exempt** (review of PR #465, Code Quality #1, High).
    /// The forced ending is composed whatever the declaration says — `role.rs`'s own
    /// `prime_false_gets_no_block_but_keeps_the_forced_ending` is the proof — so a `prime: false`
    /// role that spells the answering command out is copying text it is handed at every single
    /// turn. The first cut of this module bundled `reply` in with `send`/`list` behind
    /// `prime.chat` and reported such a declaration clean.
    #[test]
    fn the_forced_endings_command_is_a_copy_even_where_nothing_else_is_primed() {
        for opted_out in ["prime: false\n", "prime:\n  chat: false\n"] {
            let roles = [role(&format!(
                "handle: coder\n{opted_out}\
                 job_description: Implements a work order on a branch and merges it.\n\
                 system_prompt: |\n  \
                 When you are done, `nxc reply --thread <id>`.\n"
            ))];
            let warnings = warn_declarations(&roles, &[]);
            assert_eq!(warnings.len(), 1, "{opted_out:?} -> {warnings:?}");
            assert!(
                warnings[0].what.contains("`nxc reply`"),
                "{opted_out:?} -> {}",
                warnings[0].what
            );
        }
    }

    /// The same gate on the memory half, both ways round: `nxm recall` is a duplicate under
    /// `prime.memory` (which plays the memories in AND teaches the three verbs) and is honest
    /// guidance without it.
    #[test]
    fn the_memory_verbs_are_a_copy_only_where_the_memories_are_primed_in() {
        let primed = [role(
            "handle: coder\n\
             job_description: Implements a work order on a branch and merges it.\n\
             system_prompt: |\n  \
             Look the project's conventions up with `nxm recall <key>` before you start.\n",
        )];
        let warnings = warn_declarations(&primed, &[]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].what.contains("`nxm recall`"), "{warnings:?}");

        let unprimed = [role(
            "handle: coder\n\
             prime:\n  memory: false\n\
             job_description: Implements a work order on a branch and merges it.\n\
             system_prompt: |\n  \
             Look the project's conventions up with `nxm recall <key>` before you start.\n",
        )];
        assert_eq!(warn_declarations(&unprimed, &[]), vec![]);
    }

    /// Guide class 3, the persona half: absent and stub-length are the same defect and are told
    /// apart in the wording, because the fix differs (write one / write a real one).
    #[test]
    fn a_missing_or_stub_job_description_is_named_as_the_routing_surface_it_is() {
        let missing = [role("handle: coder\nsystem_prompt: be helpful\n")];
        let warnings = warn_declarations(&missing, &[]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            warnings[0].what,
            "`job_description` is not declared — that is the one thing a calling agent is shown \
             when it decides whom to address"
        );

        let stub = [role(
            "handle: coder\njob_description: Codes.\nsystem_prompt: be helpful\n",
        )];
        let warnings = warn_declarations(&stub, &[]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0]
                .what
                .starts_with("`job_description` is \"Codes.\""),
            "the stub is quoted back, so the author can see what was judged: {}",
            warnings[0].what
        );
    }

    /// The threshold is under every real description shipped in this repository — the property
    /// that makes it safe to warn on. Pinned here rather than argued in a comment, so a later
    /// change to the constant has to look at this.
    #[test]
    fn the_shortest_description_this_repository_ships_is_well_clear_of_the_threshold() {
        for shipped in [
            "Hunts silent failures and unhandled edges.",
            "Judges the change as a whole and grades it.",
            "Implements a work order on a branch and merges it.",
            "the declared order a work order runs through",
        ] {
            assert!(
                shipped.chars().count() >= ROUTING_SURFACE_MIN_CHARS,
                "{shipped:?} would warn, and it is a real description"
            );
        }
    }

    /// Guide class 3, the channel half — and the channel `summary_prompt` half of class 1 in the
    /// same declaration, so the per-declaration order (engine copy first, routing surface second)
    /// is pinned too.
    #[test]
    fn a_channel_is_warned_about_its_summary_prompt_and_its_missing_description() {
        let channels = [channel(
            "name: review\n\
             members: [a, b]\n\
             on_complete: summarize\n\
             summary_prompt: |\n  \
             Fold both reviews and deliver with `nxc reply --thread <id>`.\n",
        )];
        let warnings = warn_declarations(&[], &channels);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(warnings[0].file, "channels.yaml");
        assert!(
            warnings[0]
                .what
                .starts_with("channel \"review\" spells `nxc reply`"),
            "{}",
            warnings[0].what
        );
        assert!(
            warnings[1]
                .what
                .starts_with("channel \"review\" declares no `description`"),
            "{}",
            warnings[1].what
        );
    }

    /// The channel half of the stub case — the mirror of
    /// `a_missing_or_stub_job_description_is_named_as_the_routing_surface_it_is` above, which had
    /// no counterpart until the review of PR #465 (Test Quality #2) found the branch untested.
    #[test]
    fn a_channel_with_a_stub_description_is_named_the_same_way_a_persona_is() {
        let channels = [channel(
            "name: review\nmembers: [a, b]\ndescription: the round\n",
        )];
        let warnings = warn_declarations(&[], &channels);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            warnings[0].what,
            "channel \"review\" has the `description` \"the round\" — that is the one thing a \
             calling agent is shown when it decides whom to address"
        );
    }

    // ---- the two places one fact was written down (nxf 6j6v.g0yn) ----------------------------

    #[test]
    fn a_persona_still_carrying_the_channel_list_is_told_what_to_write_instead() {
        let roles = [role(
            "handle: reviewer\nsystem_prompt: p\naddressable: [review]\n",
        )];
        let warnings = warn_declarations(&roles, &[]);
        let found = warnings
            .iter()
            .find(|w| w.what.contains("old spelling"))
            .unwrap_or_else(|| panic!("{warnings:?}"));
        assert_eq!(found.file, "reviewer.yaml");
        assert!(found.what.contains("`addressable: none`"), "{found:?}");
    }

    #[test]
    fn the_new_spellings_carry_no_deprecation_warning() {
        for spelling in ["general", "none", "{personas: [pm]}", "{humans: true}"] {
            let roles = [role(&format!(
                "handle: r\nsystem_prompt: p\naddressable: {spelling}\n"
            ))];
            assert!(
                !warn_declarations(&roles, &[])
                    .iter()
                    .any(|w| w.what.contains("old spelling")),
                "{spelling} must not be deprecated"
            );
        }
    }

    #[test]
    fn a_step_target_repeated_under_members_is_reported_as_redundant() {
        let channels = [channel(
            "name: coding\n\
             members: [coder, ckoch]\n\
             description: turns a work order into a merged change, step by step\n\
             steps:\n  \
             - id: build\n    target: coder\n",
        )];
        let warnings = warn_declarations(&[], &channels);
        let found = warnings
            .iter()
            .find(|w| w.what.contains("as well as in its steps"))
            .unwrap_or_else(|| panic!("{warnings:?}"));
        assert!(found.what.contains("`coder`"), "{found:?}");
        assert!(
            !found.what.contains("`ckoch`"),
            "a seat no step names is exactly what `members:` is FOR: {found:?}"
        );
    }

    /// A step's `task:` is declaration text that reaches a model exactly as a `system_prompt` does
    /// — and where the step's target is a whole channel it reaches every member of it, which is
    /// precisely where a copied ending does the most damage.
    #[test]
    fn a_steps_task_is_declaration_text_too() {
        // `members: []` and not the two step targets: the steps ARE the cast since nxf 6j6v.g0yn,
        // so listing them again is its own (separate) warning and would make this case assert two
        // things at once.
        let channels = [channel(
            "name: build\n\
             members: []\n\
             description: the declared order a work order runs through\n\
             steps:\n  \
             - id: build\n    target: coder\n    next: check\n    task: |\n      \
             Build it, then `nxc reply --thread <id>` with what you did.\n  \
             - id: check\n    target: review\n",
        )];
        let warnings = warn_declarations(&[], &channels);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].what.contains("`nxc reply`"), "{warnings:?}");
    }

    /// Roles first in load order, then channels in declaration order — the order the renderer
    /// prints and therefore the order a reader works through.
    #[test]
    fn the_order_is_roles_then_channels() {
        let roles = [
            role("handle: a\nsystem_prompt: x\n"),
            role("handle: b\nsystem_prompt: x\n"),
        ];
        let channels = [
            channel("name: one\nmembers: [a]\n"),
            channel("name: two\nmembers: [b]\n"),
        ];
        let files: Vec<String> = warn_declarations(&roles, &channels)
            .into_iter()
            .map(|w| w.file)
            .collect();
        assert_eq!(
            files,
            ["a.yaml", "b.yaml", "channels.yaml", "channels.yaml"]
        );
    }
}

//! The texts the host writes itself, from ids and state: the obligation block, the delivered question, the
//! discharge and the park notice. Nothing here goes into the log.

use nexus_run_contracts::{MessageId, Outcome, ParkOutcome};
use nexus_run_engine::{Asked, Question, StepDef};

/// How a session ends its turn (§6.4 item 3). It always names the thread, because the id-free short form
/// resolves through the bridge's own lookup. `--needs-rework` appears exactly where the step declares a
/// back edge. It says what a commission would be refused for without naming the verb: what a v2 session
/// receives does not contain that command at all (§5).
pub(crate) fn obligation(thread: &str, step: &StepDef) -> String {
    let mut block = format!(
        "When you are done, answer on your thread: `nxc reply --thread {thread} - <<'EOF'`, your text, \
         then `EOF` on its own line.\n"
    );
    if step.on_needs_rework.is_some() {
        block.push_str(&format!(
            "If the work has to go back, add `--needs-rework`: `nxc reply --thread {thread} --needs-rework -`.\n"
        ));
    }
    block.push_str(&format!(
        "If you cannot reach a result, or need a decision, add `--escalate`: \
         `nxc reply --thread {thread} --escalate -`.\n\
         This session cannot commission anybody. Whatever you need, ask for it with `--escalate`, never \
         in a plain reply, which would advance the run.\n"
    ));
    block
}

pub(crate) const REMINDER: &str =
    "You ended your turn without replying on your thread. Finish the work and reply now.";

/// Told to a restarted last step: "no duplicate PR" is the finisher's own first move (§8.3).
pub(crate) const EARLIER_ATTEMPT: &str =
    "An earlier attempt of this step may already have pushed or opened a pull request. Check that first.";

/// The question `Deliver` owes the commissioner: templated from state and posted as the supervisor.
pub(crate) fn question(question: &Question, root: &str, last_verdict: Option<&str>) -> String {
    match &question.asked {
        Asked::Escalation { slot, .. } => {
            let said = last_verdict.unwrap_or("(the message could not be read)");
            format!(
                "{} on step {} asks for a decision:\n\n{said}\n\nAnswer with `nxc reply --thread {root} -`; \
                 `nxc withdraw --thread {root}` ends the run.",
                slot.member, slot.step
            )
        }
        Asked::Ceiling { step, pass, .. } => {
            let said = last_verdict.unwrap_or("(the message could not be read)");
            format!(
                "Step {step} sent the work back {} times, its ceiling. The last verdict:\n\n{said}\n\n\
                 `nxc reply --thread {root} -` continues with exactly one more pass; \
                 `nxc reply --thread {root} --accept -` accepts the verdict as it stands; \
                 `nxc withdraw --thread {root}` ends the run.",
                pass.saturating_sub(1)
            )
        }
    }
}

pub(crate) fn discharge(
    outcome: Option<&Outcome>,
    step: Option<&str>,
    pass: u32,
    verdict: Option<&MessageId>,
    parked: Option<&ParkOutcome>,
    dir: &str,
) -> String {
    let what = match outcome {
        Some(Outcome::Withdrawn) => "This run was withdrawn",
        Some(Outcome::Failed) => "This run failed",
        _ => "This run ended",
    };
    let mut text = match step {
        Some(step) => format!("{what} at step {step}, pass {pass}."),
        None => format!("{what} before any step started."),
    };
    if let Some(verdict) = verdict {
        text.push_str(&format!(" The last verdict is message {verdict}."));
    }
    if let Some(
        ParkOutcome::Committed { branch, commit } | ParkOutcome::OnOwnBranch { branch, commit },
    ) = parked
    {
        text.push_str(&format!(
            " Its uncommitted work is on branch {branch} at {commit}, in {dir}."
        ));
    }
    text
}

pub(crate) fn park_notice(branch: &str, commit: &str, dir: &str) -> String {
    format!("This run left a local, unpushed commit: {commit} on branch {branch}, in {dir}.")
}

#!/usr/bin/env bash
# The walking-skeleton end-to-end smoke (nxf epic 6j6v.zenf, Task 8): a scratch workspace + two
# roles (pm, coding) + a seeded channel, kicked off with a real Claude Agent SDK session per role
# (NXC_WORKER=sidecar). This is a LIVE SMOKE — it spawns real detached processes and costs real
# tokens — not CI. See agent-sidecar/README.md "Skeleton smoke — observed run" for a recorded run.
set -euo pipefail

# --- fresh-binary PATH (the same gotcha every prior live-smoke task hit): this machine can have a
# stale, separately-installed `nxc`/`nxs` under ~/.local/bin ahead of the one just built from this
# branch. Prepend target/debug so THIS shell's `nxc`/`nxs` resolve the fresh binary — and, because
# `Command::new("node")` in crates/chat/src/worker.rs inherits this process's env (never clears
# it), and main.mjs's `options.env` is `{ ...process.env, ...spec.env }` (spec.env never carries
# PATH), every in-session `nxc` call the spawned SDK sessions make below also resolves the same
# fresh binary — PROVIDED this script's own first `nxc`/`nxs` call inherits this PATH, which it
# does since the export happens before anything else runs.
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export PATH="$REPO_ROOT/target/debug:$PATH"

# No ANTHROPIC_API_KEY is required: this machine has an authenticated `claude` CLI session (OAuth
# credentials under ~/.claude/), which @anthropic-ai/claude-agent-sdk picks up automatically
# (confirmed live in Task 1/Task 2 — apiKeySource:"none"). The brief's literal skeleton hard-required
# the env var (`: "${ANTHROPIC_API_KEY:?...}"`), which would abort this script on this machine before
# spawning anything — dropped rather than faked, since setting a bogus key would be worse (it would
# make the SDK PREFER the (invalid) key over the working CLI auth).
export NXC_WORKER=sidecar
export NXC_SIDECAR="$(cd "$(dirname "$0")/.." && pwd)/src/main.mjs"

WS=$(mktemp -d); cd "$WS"
# Bug 2 (brief pre-flight finding): `nxs init --json` with NO --module flags defaults to flow ONLY
# ("nxs init --help": "non-interactively (--json/piped) with none given defaults to flow") — the
# brief's own "# flow + chat" comment on the bare form is misleading. Name both explicitly.
nxs init --json --module flow --module chat >/dev/null

# `nxs init --module chat` already creates `.nxs-personas/` (nxf 6j6v.dvyq step 4) — this is only
# the belt-and-braces for a chat build that predates it.
mkdir -p .nxs-personas

# NO CHANNEL IS MINTED HERE ANY MORE (nxf 6j6v.dvyq §3). What stood here captured the `m-…` id of
# a `nxc channels create work`, interpolated it into both role prompts through a placeholder, and
# then joined both roles to it by hand — three steps that existed only because a channel could be
# something nobody declared.
#
# It is all gone with the raw channels: a target is a NAME resolved against the declarations, and
# `send --to <persona>` opens the direct conversation and its thread itself. The two roles below
# address each other by handle. If this demo ever wants a real fan-in board instead, that is a
# `channels.yaml` beside the two persona files — declared, reviewable, and it survives the run.

# --- .nxs-personas/pm.yaml -----------------------------------------------------------
#
# Bug 3 (brief pre-flight finding, PM's half): the `send`/`reply` trigger path hands the
# spawned/resumed session ONLY the raw message body as its prompt (crates/chat/src/cli.rs:
# `trigger_role(..., body, ...)` -> `TriggerRequest.message = body`) — no message id is ever
# attached. So before pm can `nxc reply` to route an answer back to coding, it must first discover
# ITS OWN incoming message's id (same mechanism as coding's half below).
#
# That finding is CLOSED, and the prompts below no longer work around it. It read: "`reply()`
# (unlike `send()`) does NOT auto-stamp the ambient NXC_SESSION into the REPLY's own
# `refs.session_id` […] so a multi-hop bounce only stays resumable at EVERY hop if each reply
# explicitly restamps its own return address via `--ref session_id=$NXC_SESSION`". Both prompts did
# exactly that, on every reply expecting a follow-up.
#
# `orchestration::reply` stamps it from the caller's own session now (`with_return_address`, and
# only where the caller passed none), so the hand restamp bought nothing — and since nxf 6j6v.ckeq
# it is not expressible either: `reply` takes no `--ref` at all, because a reply INHERITS its
# subject from the thread. The refs obligation lives on `send --to`, which is where a conversation
# says what it is about.
cat > .nxs-personas/pm.yaml <<'YAML'
handle: pm
system_prompt: |
  You are the PM role in a nexus-chat workspace. Coordinate ONLY through `nxc` (Bash tool) — you
  have no other tools, and must never try to write files yourself; that is coding's job.

  You address people by NAME. There is no channel id to remember: `--to` takes a declared persona
  or a declared channel, and the conversation opens itself.

  CASE A — this is a FRESH kickoff (you have no earlier turns in this conversation): the text you
  were just given is a human's work order. Delegate ONE tiny, concrete task to the coding role,
  phrased in YOUR OWN words (not copy-pasted verbatim from the human's order, so it is
  distinguishable later when someone searches for it):
    nxc send --to coding --no-ref "<the task for coding, in your own words>"
  Do nothing else this turn.
  (`--ref nxf_ids=<id>` is how you say what a first message is ABOUT; this fixture has no ticket,
  so it says `--no-ref` — saying nothing at all is warned about.)

  CASE B — you are RESUMED with a follow-up from coding (you already have earlier turns): the text
  you were just given IS that follow-up's raw content — either a clarifying QUESTION, or a final
  REPORT that the work is done. Handle it:
    1. You do not have to go looking for an id. The text you were handed ENDS with the exact
       command to answer it, naming the thread — and it gives the body on STDIN
       (`nxc reply --thread <thread-id> -`), because the shell evaluates nothing on that path.
       Use that thread id. For the one-line answers below an argument is fine.
    2. If it was a QUESTION: answer it plainly, then reply so coding's session can resume with your
       answer. Your own session is stamped as the reply's return address automatically, so a LATER
       follow-up from coding routes back to you again:
         nxc reply --thread <that-thread-id> "<your answer>"
       Then stop — you are done for this turn.
    3. If it was a final REPORT (coding says the work is done): post ONE terse closing reply
       summarizing the outcome, for a human to read later:
         nxc reply --thread <that-thread-id> "<terse summary of what got done>"
base_prompt: claude_code
tools: [Bash]
YAML
# --- .nxs-personas/coding.yaml --------------------------------------------------------
cat > .nxs-personas/coding.yaml <<'YAML'
handle: coding
system_prompt: |
  You are the coding role in a nexus-chat workspace. Coordinate through `nxc` (Bash tool); use
  Read/Write ONLY for the actual work product — never to communicate with pm, that always goes
  through `nxc`.

  Every turn, the text you were just given is either pm's original order (a fresh turn), or, on a
  later RESUMED turn, pm's answer to a question you asked. Either way it ENDS with the exact command
  to answer it, naming the thread — and it gives the body on STDIN
  (`nxc reply --thread <thread-id> -`), because the shell evaluates nothing on that path. Use that
  thread id. For the one-line answers below an argument is fine. You do not have to go looking for
  an id, and there is no channel to remember.

  This is a deliberate two-turn test of the messaging/session-resume mechanism itself, not a
  realistic judgment call about ambiguity — so follow the rule below EXACTLY, regardless of how
  clear pm's order seems to you. Tell which turn you're on from your OWN conversation history (a
  truly fresh session has no earlier turn of yours in it at all).

  CASE A — TURN 1 (a fresh session: this is the very first message you have ever seen in this
  conversation, i.e. pm's order): do NOT create any file yet, no matter how clear the order seems.
  Ask exactly ONE clarifying question back about the greeting (e.g. its exact wording, tone, or who
  it should be addressed to) — this is MANDATORY on turn 1, not optional, so pm's session can
  resume with the answer. Your own session is stamped as the reply's return address automatically,
  so pm's eventual answer routes back to resume THIS session, not a fresh one:
    nxc reply --thread <that-thread-id> "<your one question>"
  Then stop for this turn — wait to be resumed with the answer.

  CASE B — TURN 2 (a RESUMED turn: you already have an earlier turn of yours in this conversation —
  the text you were just given is pm's answer to your question): now do the actual work — create
  hello.txt via the Write tool, containing a greeting using pm's answer to decide its exact wording
  — then report back so pm's session can resume and close out:
    nxc reply --thread <that-thread-id> "<short report of what you did>"
base_prompt: claude_code
tools: [Bash, Read, Write]
YAML
# Kickoff from the human terminal (no NXC_SESSION set): summon the PM. `--no-ref`, because this
# fixture is about the messaging mechanism and not about a ticket — since nxf 6j6v.ckeq a first
# message either names what it is about or says explicitly that there is nothing, and omitting both
# is warned about. There is no return address either: a bare-terminal kickoff has no ambient session
# to route a follow-up back to (by design: crates/chat/src/cli.rs's `session()` returns None here,
# so `refs.session_id` is deliberately left unset rather than stamped with a placeholder).
#
# LIVE-RUN FINDING (T8): the brief's original wording here ("Ambiguity is expected") relied on the
# MODEL deciding, on its own judgment, that the order was ambiguous enough to ask about — a live
# run showed pm reliably pre-empts that by resolving the ambiguity itself when delegating to coding
# (e.g. "a short greeting, your choice of wording"), so coding never asks anything and the resume
# path a `--role`/`reply` round-trip is supposed to prove never fires. The forced question is now a
# DETERMINISTIC rule in coding's own system_prompt (turn 1 always asks, turn 2 always does the work
# — see below) rather than left to the model's ambiguity judgment; the wording here no longer needs
# to manufacture ambiguity itself.
nxc send --to pm --no-ref "Kick off: have coding create a file hello.txt containing a greeting." --json
echo "workspace: $WS"

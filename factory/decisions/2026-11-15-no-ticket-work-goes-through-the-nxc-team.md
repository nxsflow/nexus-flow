# 2026-11-15 · no ticket · Never build tickets yourself with your own subagents: implementation goes to the channel coding, planning to pm or planning; only pure configuration or cleanup tickets are done directly.

- Decided by: recorded in the project memory (`nxm`, key `arbeit-ueber-das-team`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, process, chain

## The record

Owner decision of 2026-10-12: since 12 October every workspace on the machine sees the coding team from the user-level folder ~/.nexusflow/personas (a link to the repository nexusflow-personas): pm, coder, verifier, finisher, merger, the three reviewers and the channels planning, coding, review, merging, process. 'nxc list' shows them with 'from the user-level folder'.

HOW THE WORK IS DONE:
- Building a ticket: nxc send --to coding --ref nxf_ids=<id> - (the brief by heredoc: what, why, acceptance, pitfalls). The coder builds test-first, the verifier measures the gates, the review round judges, the finisher opens the PR. The session writes the commission, watches the run and reports the result; it does NOT build itself.
- Cutting a plan into tickets: nxc send --to planning (the design is reviewed), or for something small or an opinion directly --to pm.
- Merging: the channel merging, with the finished ticket, the next ticket and the branch.
- NO LONGER for ticket work: superpowers:subagent-driven-development, or implementer and reviewer subagents of one's own. The Superpowers plugin asks for its skills at every start; for ticket work this rule takes precedence.
- EXCEPTION: pure configuration or cleanup tickets, for which the full chain is too heavy, are done directly and the PR is shown to the owner. Research, measurements, acceptance checks and deploy checks are done by the session itself.
- Before every send: check nxf search for duplicates and nxc status --json for a commission already running for the same ticket.
- NEVER create a persona or a channel with one of these names in this repository's .nxs-personas/: it hides the central version. Changes to the team belong in the repository nexusflow-personas.

Model: agents has worked this way since 2026-09-20, nexflowit-app since 2026-10-12.

FOR THIS REPOSITORY: the coder pushes and starts the CI run; the verifier reads it on the commit and starts none. The procedure is in gates-run-in-ci-not-locally and need not go into the brief to coding. Runs that test nxc itself (fixture personas such as coder/review/test in temporary workspaces) are not affected.

REVIEW (owner decision of 2026-10-12, 'the chain replaces review'): the review round of the coding chain (code quality, test quality, integrity, in separate sessions) IS the independent review. After it there is NO additional review-pr run, no stop for an owner review, no mitigation round: after the finisher's PR, commission the channel merging (finished ticket, next ticket, branch), then a beta release where it makes sense, then a pm_report. Promotion to stable stays owner-gated, and where the repository has a merge rule of its own, that rule still applies.

(Translated from German on 2026-11-15; the memory's key was `arbeit-ueber-das-team`.)

# 2026-11-15 · no ticket · Never launch two worktree-isolated agents in ONE message — a subagent checkout -b can flip the shared checkout; stagger and forbid checkout.

- Decided by: recorded in the project memory (`nxm`, key `parallel-worktree-agents-race`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: git, worktrees, subagents

## The record

Launching two `Agent` calls with `isolation: "worktree"` in the SAME message (concurrently) raced:
one subagent's `git checkout -b <branch>` executed against the SHARED main checkout instead of its
own worktree, flipping the main checkout off `main` (its worktree stayed on the auto
`worktree-agent-<id>` branch). This is the "worktree isolation can collapse into branch switching in
a shared checkout" trap Carsten warns about for diamond/parallel phases.

**Why:** concurrent worktree creation plus an in-agent `git checkout -b` is not reliably cwd-pinned
to the agent's worktree at launch time.

**Safe recipe for parallel worktree subagents:**
1. **Stagger** the launches — launch agent A, confirm its worktree via `git worktree list` (main
   still `[main]`), THEN launch agent B. Do not launch both in one message.
2. Give each agent a **hard-abort preflight**: verify `git rev-parse --git-dir` !=
   `--git-common-dir` AND that `git rev-parse --show-toplevel` is under `.claude/worktrees/`; if
   not, STOP and run nothing.
3. Tell agents **never** to `git checkout` / `checkout -b` / `switch` — work on the
   isolation-provided `worktree-agent-<id>` branch and report the final commit **SHA**.
4. **Collect by SHA** (merge or cherry-pick the reported SHA onto an integration branch), since the
   worktree and its branch may be auto-cleaned when the agent finishes.
5. After each launch, re-check `git worktree list` and that `git branch --show-current` on main is
   still `main`.

Verified working for a two-task parallel diamond in the nexus-chat milestone work. A shared
checkout has a second, independent failure mode worth reading alongside this one:
[review-session-in-a-shared-checkout-mistakes-the-work-for-its-own](2026-11-15-no-ticket-review-session-in-a-shared-checkout-mistakes-the-work-for-its-own.md). See also
[the-board-is-local-and-not-in-git](2026-11-15-no-ticket-the-board-is-local-and-not-in-git.md).

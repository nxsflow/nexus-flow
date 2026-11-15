# 2026-11-15 · no ticket · The chat write-seam gate joins calls by BARE NAME — a private helper called resolve or send turns eleven read verbs red; name helpers specifically.

- Decided by: recorded in the project memory (`nxm`, key `call-graph-gate-joins-on-bare-names`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, gates, naming

## The record

The write-seam gate in `crates/chat/tests/read_surface.rs`
(`nothing_on_this_seam_starts_work_that_was_not_already_commissioned`) does NOT decide by types or
modules which calls can start a session. It builds a call graph over ALL of `chat/src` and joins
nodes by **BARE FUNCTION NAME** (`nxs_test_support::verb_seam::call_graph` records every
`ExprMethodCall` and every `ExprCall` under its LAST path segment). For this gate two functions
with the same name are ONE function — `Workspace::resolve` and a private `WorkingCopy::resolve`
are the same node, and `tx.send(buf)` on an mpsc channel is the same node as `facade::send`.

THE CONSEQUENCE, measured 2026-09-05 while building the escalation-parking feature: a private
helper `fn resolve(&self, rev: &str)` in `crates/chat/src/park.rs`, in a module that shells out to
git via `std::process::Command` and whose pipe readers return their result through `tx.send(...)`,
turned ELEVEN READ VERBS of the engine seam red — `status`, `directory`, `search`, `thread`,
`prime_as`, `persona_prime`, `open`, `open_with`, `origin`, `definitions`, `withdraw`. The path the
gate found:

    status -> open -> resolve -> git -> send -> send_to -> channel_open -> ... -> trigger_and_bind

Every hop is real IN THE GRAPH and none is real in the code. The gate's error message names only
the eleven lines and says "the source is the one that is true" — it does NOT name the PATH, and
without it you look in the wrong place.

THE RULE THAT FOLLOWS: a private helper in `chat/src` must not share a name with something the
read seam calls — above all not `resolve`, `send`, `open`, `status`, `thread`, `search`,
`definitions`, `origin`. If you work in a module that spawns a subprocess or uses a channel, name
specifically: `resolve_commit`, not `resolve`. This is not cosmetics; it is the condition under
which this gate can say anything at all.

HOW TO FIND THE PATH when it happens anyway: a throwaway test in `crates/chat/tests/` that runs
`call_graph` over `crates/chat/src`, reverses the edges (callee -> caller) and runs a BFS backwards
from `trigger_and_bind` with a predecessor table; then print the path to the reported verb. Two
minutes, and delete it again afterwards.

# 2026-11-15 · no ticket · nxm remember has no - STDIN form: piping a body stores a literal hyphen and reports success — pass it as an argument and verify with recall.

- Decided by: recorded in the project memory (`nxm`, key `nxm-remember-has-no-stdin-dash`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxm, cli

## The record

NXM AND NXF DO NOT SPELL "READ FROM STDIN" THE SAME WAY, AND THE DIFFERENCE IS SILENT.

`nxf note add <id> -` and `nxf create --description -` read the body from STDIN. `nxm remember`
does NOT: its `<TEXT>` is positional, so

    nxm remember - --key k --introduction '...' < body.md

stores the literal string `-` as the whole memory, reports `remembered k`, and lists the key in the
index with its introduction. The BODY is one hyphen and nothing says so. Caught on 2026-09-05 only
because `NEXUS_MEMORY.md` is committed and the regenerated diff showed an empty section — an agent
that does not review that diff loses the fact entirely, at exactly the moment it was trying to make
it durable. An open board item tracks giving `nxm remember` the same STDIN form `nxf` has.

UNTIL THEN, pass the body as an argument:

    nxm remember "$(cat body.md)" --key k --introduction '...'

and then ALWAYS verify with `nxm recall <key>`. One command, and it is the only thing that can tell
you the write landed.

TWO MECHANICAL DETAILS THAT BITE WHEN SCRIPTING THIS:
- A body that begins with `- ` (a markdown list) is parsed as a FLAG. Put the option arguments
  first and separate the text with `--`: `nxm remember --key k --introduction '...' -- "$BODY"`.
- `"$(cat file)"` is safe, but a bare double-quoted literal is not — the shell evaluates backticks
  and angle brackets inside it first. See [shell-evaluates-long-descriptions](2026-11-15-no-ticket-shell-evaluates-long-descriptions.md).

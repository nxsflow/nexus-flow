# 2026-11-15 · no ticket · Never pass a long description in double quotes — the shell evaluates backticks and angle brackets first; use a quoted heredoc and read it back.

- Decided by: recorded in the project memory (`nxm`, key `shell-evaluates-long-descriptions`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: shell, cli

## The record

A TRAP WHEN CREATING BOARD AND MEMORY ENTRIES FROM THE SHELL: `nxf create --description "..."` and
`nxm remember "..."` in DOUBLE quotes let the shell evaluate the text before `nxf`/`nxm` ever sees
it. In this repository that is not an edge case but the normal case: descriptions constantly quote
identifiers in backticks and placeholders in angle brackets.

PROVEN 2026-09-02 while creating a board item: `nxc reply --thread <id> "..."` became "the human
types ." — the shell read the angle brackets as a redirection. And the word `nxs guide` in
backticks became the COMPLETE OUTPUT of `nxs guide`, written into the middle of the item
description. The command reported "created" and looked successful; only the two lines
"(eval):1: no such file or directory: id" before it gave it away. Anyone who does not read back
never notices.

THE FORM THAT HOLDS — a quoted heredoc inside a command substitution, which leaves the body
literal:

    nxf create --type feature --title "..." --description "$(cat <<'EOF'
    Text with `backticks`, <placeholders> and $variables stays untouched.
    EOF
    )"

The single quote around EOF is the decisive part; without it the shell evaluates the heredoc body
just the same. Passing a file is equally safe: `"$(cat body.md)"` does not evaluate its contents.

REPAIR, if it has already happened: `nxf update <id> --set description="$(cat <<'EOF' … EOF
)"` — `description`, `design` and `completion_criterion` are ordinary `--set` fields. For `nxm` the
same form with `nxm remember --key <key>`, which replaces the entry in place.

READING BACK IS PART OF THE STEP: after every create/remember with a longer text, run
`nxf show <id>` or `nxm recall <key>` once and check that what you wrote is what is there. See
[nxm-remember-has-no-stdin-dash](2026-11-15-no-ticket-nxm-remember-has-no-stdin-dash.md) for the neighbouring trap on the same surface.

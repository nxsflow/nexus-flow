# DECISIONS.md — where the decision log lives

The decision log is the folder [`factory/decisions/`](decisions/), one file per decision. FACTORY.md defines it; this file says how to read and write it in this repository.

## Reading it

```sh
cargo xtask decisions                      # every decision in full, in date order
cargo xtask decisions --index              # one line each: date, ticket, title, superseded by …
cargo xtask decisions --current            # leave out superseded decisions
cargo xtask decisions --ticket NEX-22      # only this ticket's decisions
cargo xtask decisions --topic tracker      # only this topic's decisions
cargo xtask decisions --since 2026-11-01   # only decisions made on or after this date
```

The filters combine with each other and with `--index` and `--current`. `ls factory/decisions/` lists them by date too.

## Writing one

Add a new file, `factory/decisions/YYYY-MM-DD-<ticket>-<slug>.md`, or `YYYY-MM-DD-no-ticket-<slug>.md` when there is no ticket. When the heading names several tickets, the file name takes the first. The slug is the title in lowercase words joined by `-`.

```markdown
# 2026-11-14 · NEX-22 · Tickets live in Linear

- Decided by: Carsten
- Why: one line
- Instead of: the option not taken, one line
- Topics: tracker, factory
- Supersedes: 2026-11-07-no-ticket-some-older-decision.md
```

- **Topics** is required: one or more lowercase tags, separated by commas, so a reader can find every decision on a subject.
- **Supersedes** is optional: the file names of earlier decisions this one replaces, separated by commas. Nobody edits an old entry; a decision is superseded when a later file names it here.

CI checks every file (`cargo xtask decisions --check`, in the `lint` job): the name pattern, the heading's date and first ticket against the file name, the four required fields, lowercase topics, and that each `Supersedes:` file exists. The folder holds decision files only: any other file, a `.gitkeep` included, fails the name pattern.

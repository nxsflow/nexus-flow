# RFC 0004: Genesis storage

Status: Decided (2026-11-20)

This RFC designs where the `genesis` module keeps a team's product description, its GENESIS, and how nexus-flow installs the template and the guide it ships beside it. It follows `factory/GENESIS.md`, "What we build" (the `genesis` row: the product, what we build, for whom, principles, no-gos, trade-offs and what goes back to the owner, stored in git, shipped with a template, a guide and `nxs genesis create`) and the no-go "Never overwrite a team's own GENESIS … on install or upgrade". It says where a GENESIS lives and which copy is the truth, how a change to it reaches the owner, how an agent reads it, how the rules on what comes back to the owner are kept so the merge gate can find them, and how `nxs genesis create` installs the template and the guide without ever writing over a team's own text. Once it is decided, a coding ticket can build `nxs genesis create` and its library counterpart.

## What it solves

A team that lets agents work for days needs its product written down where every agent reads it and the owner reviews every change (GENESIS.md, "What we build"). Today each repository that runs a factory keeps such a file by hand: this one in `factory/GENESIS.md`, nocentraldb in its own `factory/`. v1 has no genesis module; v2 makes it one, which is mostly its template and guide and starts with a single command (decision of 2026-11-15, NEX-33, "genesis is a template and a guide, with a small module that starts with nxs genesis create").

The owner and the agents need from the genesis storage:

- **One file, at one path, in every repository**, so that every role's instructions can point at it and every agent finds it without asking.
- **A change only through the owner's review.** GENESIS says what to build and what comes back to the owner, so an agent must not be able to change it on its own (principle 1).
- **Reads that never wait for the network** (principle 3).
- **Rules on what comes back to the owner that a script can find**, so the merge gate checks every one of them and can tell when one is missing (GENESIS.md, "The merge gate": "The rules in a team's GENESIS on what comes back to the owner are checked the same way").
- **A template that never overwrites a team's own text**, on install and on every upgrade of nexus-flow.

The owner decided the main choice before this RFC was written, and the text below carries it: a team's GENESIS stays a file in git, and the file is the truth; `nxs` reads it from the working tree, and NoCentralDB holds nothing of it for now (NEX-14; decision of 2026-11-15, "A team's GENESIS and factory files stay in git, and the files are the truth"). The owner decided the two choices this RFC left open on 2026-11-20: the rules on what comes back to the owner are the list after a marker comment (NEX-70), and `nxs genesis create` installs the guide as a shipped default under RFC 0006's rules (NEX-71).

Out of scope, each elsewhere: the text of the template and the guide, which are default templates the owner decides in their own ticket (GENESIS.md, "What has to come back through me"); the merge gate's rules, their JSON schema and the script that decides, and how that script reads the rules this RFC keeps (RFC 0010, the merge gate, NEX-72); the factory's files, `nxs factory install` and the rules for shipped defaults, which this RFC applies to the guide (RFC 0006); and the frame of spaces, members and `nexus-flow.toml` (RFC 0001). A mirror of GENESIS into the repository space, which NEX-14 left open for when manufakt.io or a coordination space needs to read it, is not part of v2: no v2 reader needs it yet, and a later RFC can add it without changing which copy is the truth.

This RFC builds on RFC 0001 ("Creating and finding a space", for the repository's root and `nexus-flow.toml`) and on RFC 0006 ("Which copy is the truth", "Installing the shipped defaults"), and cites NoCentralDB's RFCs as "NoCentralDB RFC", by number and section.

## The design

### Where a GENESIS lives

A repository's GENESIS is one Markdown file at a fixed path, relative to the repository's root, the folder that holds `nexus-flow.toml` (RFC 0001, "Creating and finding a space"):

```
factory/
  GENESIS.md          # the team's product description; written once by `nxs genesis create`, the team's own from then on
  genesis-guide.md    # the guide for filling it in, as nexus-flow ships it; written and updated by `nxs genesis create`
```

**The path is fixed,** next to the factory's files, because every role file and every session's instructions point at it, and the repositories nexus-flow follows already keep it there (this one and nocentraldb). The `factory/` folder is RFC 0006's; GENESIS.md and the guide are neighbours in it, owned by this module, and `nxs factory install` neither writes nor reads them (RFC 0006, "The factory's files").

*Not taken:* `GENESIS.md` at the repository's root (it would sit apart from the files the owner reviews with it, and a root `GENESIS.md` is easy to mistake for a project's README); a setting in `nexus-flow.toml` that names another path (a setting earns its place only when a real team needs it, GENESIS.md, "Every default belongs to the team", and one can be added later without breaking anyone); and several files, one per section (one file is what agents and the owner read in one go, and how a GENESIS is structured inside is the team's call, GENESIS.md, "Strong opinions on what, open on how").

**nexus-flow reads one thing inside the file:** the rules on what comes back to the owner (see "The rules on what comes back to the owner"). Everything else is text for agents and people, and its shape is the team's.

### Which copy is the truth

The file in git is the truth (NEX-14). nexus-flow keeps no copy of it in NoCentralDB, no cache of it anywhere, and declares no collection for it. Two kinds of reader use two copies, as for the factory's files (RFC 0006, "Which copy is the truth"):

- **A session reads its working tree.** An agent reads `factory/GENESIS.md` as it is checked out where the session runs, local changes included. A branch can therefore draft a change to GENESIS and work under it before it merges, and a session on an old branch works under the GENESIS of that branch.
- **What judges a pull request reads the base branch.** Whatever checks a pull request against the rules in GENESIS reads `factory/GENESIS.md` from the commit the base branch points at (`[git] base` in `nexus-flow.toml`, RFC 0001), never from the pull request's head. A pull request that loosens a rule on what comes back to the owner is judged by the rule it would replace.

### How a change reaches the owner

GENESIS changes only through a pull request, like code. nexus-flow adds no approval of its own: the owner's approval is the code owners' review the forge already enforces, and the merge gate's decision on top of it (GENESIS.md, "The merge gate", step 1). For that, `factory/GENESIS.md` must be under CODEOWNERS with the owner as its code owner, as it is in this repository (`.github/CODEOWNERS`).

**CODEOWNERS is reported, never written.** `nxs genesis create` reads the repository's CODEOWNERS file in the places and order GitHub looks for it (`.github/CODEOWNERS`, `CODEOWNERS`, `docs/CODEOWNERS`), as `nxs factory install` does (RFC 0006), and reports whether a rule in it covers `factory/GENESIS.md`. It never writes CODEOWNERS: that file is the team's own, and a change to it comes back to the owner (GENESIS.md, "What has to come back through me").

An agent may propose a change to GENESIS, preferably after the owner decided it in a ticket (GENESIS.md, "The merge gate"); it merges only with the owner's approving review.

*Not taken:* an approval kept inside nexus-flow, such as an owner's signature on a record in NoCentralDB (NEX-14 decided against it: the owner's approval would have to be rebuilt beside the forge's, which already enforces it).

### How an agent reads it

An agent reads `factory/GENESIS.md` from the working tree with its own file tools, as it reads code. `nxs` has no command that prints it: a file read is already local, deterministic and needs no network (principle 3), and the module starts with `nxs genesis create` alone (NEX-33). A session's instructions, such as a repository's `AGENTS.md` and the role files, name the path.

*Not taken:* `nxs genesis show` (it would print the same bytes a file read returns, and one more command is one more surface the owner decides); and reading GENESIS out of NoCentralDB (NEX-14).

### The rules on what comes back to the owner

A team's GENESIS says which work comes back to the owner before it merges, as this repository's does in "What has to come back through me". The merge gate checks each of these rules the way it checks the merge rules: a sub-agent reports, per rule, whether the pull request meets it, and a script decides (GENESIS.md, "The merge gate"). For the script to know that every rule got a verdict, the list of rules must come out of the file the same way every time, not from a model's reading of it.

**The rules are a marked list in GENESIS.md.** The rules sit in the team's GENESIS, in the words the owner reviews, as one Markdown list: each top-level item is one rule, and items nested under it belong to it. A marker comment on the line before the list marks it, so a script finds it whatever the team calls the section (decided by the owner, NEX-70):

```markdown
## What has to come back through me

<!-- nexus-flow: owner-rules -->
- Anything a user sees first: command names and their output contract, …
- A change to a stored format, or the migration from v1 to NoCentralDB.
```

The marker survives a renamed or translated heading, it sits in the same file and the same diff the owner reviews, and it needs no setting. It does not show in rendered Markdown, so an agent editing the section can delete it without seeing it; the merge gate then reserves every pull request for the owner until it is restored (see below).

**Reading the rules is deterministic.** Given the bytes of `factory/GENESIS.md`, the mark is a line that reads exactly `<!-- nexus-flow: owner-rules -->`, apart from surrounding whitespace, outside a fenced code block, and the rules come from the list that follows the mark directly: a CommonMark list whose first item starts on the first line after the mark that is not blank (decided by the owner, NEX-77; RFC 0010, "The rules from GENESIS"). The rules are its top-level items, in their order, each with its text as written, nested items included. Any other line there, such as a paragraph or a heading, means the mark has no list after it, so a list further down the file is never read as the rules once the list under the mark is deleted. A file with no mark, more than one mark, or a mark with no list after it has no readable rules, and that is a state of its own, not an empty list.

**When the rules cannot be read, the owner reviews.** A missing file, a missing or doubled mark, or a mark with no list must never let a pull request merge without the owner. This RFC sets that as a requirement on RFC 0010, the merge gate's RFC (NEX-72), which designs the script that reads the rules, the command or seam it reads them through, and how its JSON names each rule. The base branch's copy is the one read (see "Which copy is the truth").

*Not taken* (decided by the owner, NEX-70): a fixed heading, the one the template uses (the team could no longer name or translate the section, against "Strong opinions on what, open on how", and this repository's GENESIS would have to rename its own); and the heading named in `nexus-flow.toml` (renaming the section without changing the setting loses the rules, and the owner would review a second file together with GENESIS.md). Also not taken: the gate's sub-agent reading the whole GENESIS and listing the rules it finds (the list itself would be a model's judgement, and a rule could be skipped without anyone seeing it); and a separate rules file beside GENESIS, such as one under `factory/merge-gate/` (the rules would then live apart from the product description that gives them their reasons, and the owner would review two texts that can disagree).

### Installing the template

nexus-flow ships a GENESIS template and a guide for filling it in (GENESIS.md, the `genesis` row). Their text is a default template, which the owner decides in its own ticket; this RFC decides how they are kept and installed.

**Where nexus-flow keeps them.** In nexus-flow's source tree under `templates/genesis/` (`GENESIS.md`, `genesis-guide.md`), embedded in `nxs` at build time, so creating a GENESIS needs no network. Beside them, `templates/genesis/shipped.toml` lists the SHA-256 of every version of the guide nexus-flow has ever shipped, in the format of RFC 0006's `templates/factory/shipped.toml`; the template has no entry there, because nexus-flow never updates a GENESIS. The template carries the mark for the rules on what comes back to the owner, with the template's default rules under it.

**`nxs genesis create`** writes `factory/GENESIS.md` from the template, only where none exists, and installs the guide (see "The guide" below). It is the only thing in nexus-flow that ever writes either path:

| The repository's `factory/GENESIS.md` | State | What `create` does |
| --- | --- | --- |
| Missing from the working tree, and tracked neither in the index, in `HEAD`, nor on the base branch | `written` | Writes the template, creating `factory/` if needed |
| Present, with any content, the template's included | `team` | Nothing |
| Missing from the working tree, but tracked in the index, in `HEAD` or on the base branch | `tracked` | Nothing, and exits non-zero: the file was deleted or not checked out, and writing the template would replace the team's text in the next commit. The error names the file and says to restore it from git |
| A file whose name differs from `GENESIS.md` only in case is present in `factory/` | `team` | Nothing; the report names the file it found |

The write never replaces anything: it creates the file only if the path is free at the moment of writing (an exclusive create, which also fails on a symbolic link, dangling or not), so a GENESIS that appears between the check and the write is kept. It never commits: what it writes lands in the working tree and reaches the base branch through a pull request, which CODEOWNERS sends to the owner.

**A GENESIS is never updated by nexus-flow.** Once the file exists it is the team's, even while it still reads exactly like the template. `create` has no option that writes over it, `--update` included, and no other command, upgrade or migration writes, renames or deletes it: not `nxs init`, not `nxs factory install` (RFC 0006), not `nxs migrate` (RFC 0008). A newer template reaches a team through the release notes, and the team changes its own file in a pull request if it wants to.

*Not taken:* recognising an unchanged template by its hash and replacing it with a newer one on request, as RFC 0006 does for the factory's defaults (a GENESIS that still reads like the template describes no product yet, so there is nothing an update would keep current, and the hash machinery would earn nothing); and merging a team's text with a newer template (the result is a product description nobody wrote).

**The guide.** `nxs genesis create` installs the guide as `factory/genesis-guide.md`, under RFC 0006's rules for shipped defaults (decided by the owner, NEX-71): it compares the repository's file with the guide's shipped versions in `templates/genesis/shipped.toml`, after turning CRLF into LF on both sides, and reports the same states as `nxs factory install` (RFC 0006, "Installing the shipped defaults"). It writes the current guide when the file is missing (`written`); with `--update`, it replaces a file identical to an earlier shipped guide (`older_default`, then `updated`); and it never touches a guide the team changed (`team`), nor one whose path is retired (`retired`). Unlike GENESIS, the guide is nexus-flow's text, which a team rarely changes, so keeping it current is worth that rule, and every agent reads it locally. `--update` applies to the guide only and never to `factory/GENESIS.md`. The guide needs no code owner: it holds no rule.

*Not taken* (decided by the owner, NEX-71): the guide as comments inside the template (it would live in the team's file, never get newer, and be gone once the team deletes the comments); and the guide only in nexus-flow's online documentation (an agent would have to fetch it, which a cloud container's network may not allow, and its version may not match the `nxs` installed). Also not taken: listing the guide among RFC 0006's factory defaults, so that `nxs factory install` writes it (a file of the `genesis` module would be installed by the `factory` module's command, and RFC 0006 would have to change for it).

**`nxs init`** (RFC 0001) runs `nxs genesis create` after `nxs factory install`, without `--update`, so a new factory has a GENESIS to fill in on day one, which its roles and FACTORY.md can point at. On an upgrade of nexus-flow nothing is installed until someone runs `nxs genesis create`.

**Finding the repository.** `create` finds the repository's root as RFC 0001 does, from the working directory up to the folder holding `nexus-flow.toml`. Without one it exits non-zero and says to run `nxs init`. It opens no NoCentralDB database: it touches only files and the local git.

**Output.** JSON by default, Markdown with `--md`, files ordered by path, bytewise:

```json
{
  "files": [
    { "path": "factory/GENESIS.md", "state": "written" },
    { "path": "factory/genesis-guide.md", "state": "written" }
  ],
  "codeowners": {
    "file": ".github/CODEOWNERS",
    "uncovered": ["factory/GENESIS.md"]
  }
}
```

`files` has the shape of `nxs factory install`'s report. The `state` of `factory/GENESIS.md` is `written`, `team` or `tracked`; the guide's is one of RFC 0006's states (`written`, `current`, `older_default`, `updated`, `team`, `retired`). A `team` entry for GENESIS found under another case carries `found`, the path as it is on disk. `codeowners` is `null` when the repository has no CODEOWNERS file, and `uncovered` names `factory/GENESIS.md` when no rule covers it; the guide is not checked. The `tracked` case exits non-zero with this report on standard output, so the caller sees which file is missing.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Command:** `nxs genesis create [--update]` (`--update` for the guide only), which `nxs init` also runs, without `--update`.
- **JSON output** of `nxs genesis create`: `files` (each with `path`, `state`: `written`, `team` or `tracked` for GENESIS, one of RFC 0006's states for the guide, and `found` where a case variant was found), `codeowners` (`file`, `uncovered`, or `null`).
- **Library seam:** `Engine::genesis_create(update: bool)`, the counterpart of the command, returning the same report (the decision on CLI and library parity stays in force).
- **Stored formats:** `factory/GENESIS.md` as the fixed path of a team's GENESIS; the mark `<!-- nexus-flow: owner-rules -->` and the rule that reads the list directly after it; `factory/genesis-guide.md` as the guide's path; `templates/genesis/shipped.toml` in RFC 0006's format.

### What nexus-flow needs from NoCentralDB

Nothing. GENESIS stays in git (NEX-14), so this RFC adds no collection to the model and no requirement on NoCentralDB.

### What the build must prove

The coding ticket that builds `nxs genesis create` proves the no-overwrite guarantee with tests at the seam (`Engine::genesis_create`), each of which must fail if the guarantee is broken: an existing `factory/GENESIS.md` with arbitrary bytes, the template's included, is unchanged after `create`, after `create --update`, after `nxs factory install --update` and after `nxs init`; a guide the team changed is unchanged after `create --update`; a GENESIS deleted from the working tree but tracked in `HEAD`, or only on the base branch, is reported `tracked`, nothing is written, and the exit is non-zero; a dangling symbolic link at the path is not followed; a `factory/genesis.md` on a case-sensitive file system blocks the write; and `nxs init` run twice leaves the file as the first run, or the team, left it.

## What it does not do

- **It does not stop a session from editing GENESIS.** A session reads its working tree, so it can change GENESIS and act on the change at once. The guard is at the merge: the rules are read from the base branch, and the path is under CODEOWNERS. A repository without CODEOWNERS, or on a forge without it, has no such guard; `nxs genesis create` reports it but cannot make one.
- **It does not check what GENESIS says,** beyond finding the rules on what comes back to the owner. A GENESIS that is empty, still the template, or contradicts itself is the team's to find.
- **It does not judge a rule.** It only says how the rules are found; whether a pull request meets one is the merge gate's sub-agent's judgement (GENESIS.md, "Limits").
- **A lost mark sends every pull request to the owner.** If an edit removes or doubles the mark, the merge gate cannot read the rules and must reserve every pull request for the owner until the mark is restored. That is safe, and it is noisy.
- **It does not bring a newer template to a team.** A GENESIS, once created, never changes through nexus-flow; the team reads the release notes and changes its file itself.
- **It does not protect a GENESIS that git does not know.** A file that was never committed, then deleted, is gone from the working tree and from git alike, so `create` writes the template there; nothing of the team's is in git to overwrite.
- **It does not give manufakt.io the GENESIS.** A caller with no checkout cannot read it until a later RFC adds a mirror (NEX-14).
- **A team that changes its base branch changes which rules judge a pull request.** The rules are read from whatever `[git] base` names.

## Migration

Nothing to migrate. v1 has no genesis module and stores no product description. A repository that already keeps a `factory/GENESIS.md`, such as this one or nocentraldb, keeps it: `nxs genesis create` and `nxs init` report it as `team` and write nothing. A repository that keeps its product description elsewhere moves it to `factory/GENESIS.md` itself, in a pull request.

To have the merge gate check its rules, such a repository adds the mark before its list of rules on what comes back to the owner, in a pull request the owner reviews. Until it does, the gate reads no rules from it and reserves every pull request for the owner (see "The rules on what comes back to the owner").

## Open questions

None. The owner answered them on 2026-11-20 (NEX-70, NEX-71), and the answers are written into the text above.

## Decisions

- **The rules are the list directly after the mark** ("The rules on what comes back to the owner"). Decided by the owner on 2026-11-20 (NEX-77), with RFC 0010 (#171). This narrows "the first list after the mark": only blank lines may stand between the mark and the list's first item. Why: if the list under the mark is deleted, the merge gate must read no rules and send every pull request to the owner, not read some later list in GENESIS as the rules, silently. *Not taken:* the first list anywhere after the mark, which matched this RFC's words most literally.

## History

- 2026-11-17: published as a draft in #163.
- 2026-11-20: decided by the owner (NEX-70, NEX-71), in #163.
- 2026-11-20: the list of rules narrowed to the one directly after the mark, from RFC 0010 (see "Decisions"), in #173 (NEX-83).

# RFC 0010: The merge gate

Status: Draft

This RFC designs the merge gate of the `factory` module: the step that decides whether a pull request goes to the owner before it merges, or whether pm may merge it. It follows `factory/GENESIS.md`, "The merge gate" and its limit "The merge gate's verdict is a model's judgement". It says how a team writes its merge rules and in which file, how the rules on what comes back to the owner are read out of GENESIS, what the sub-agent's verdict looks like as JSON, how a deterministic decision is made on top of the verdicts, how that decision becomes a status check that repository rules enforce, which copy of the rules judges a pull request, and the minimal set nexus-flow ships. Once it is decided, a coding ticket can build `nxs merge-gate rules`, `nxs merge-gate decide` and their library counterparts, and a ticket of its own can write the shipped defaults.

## What it solves

Agents work for days without the owner watching. The owner wants to review only what needs them, and to know that no agent can merge what needs them, whatever the agent's prompt says (GENESIS.md, principle 1 and the first no-go). GENESIS.md splits the gate into five steps: repository rules decide what they can; the team writes merge rules for the rest; a sub-agent reports a verdict per rule as JSON; a script decides from that JSON whether pm merges or the owner reviews; and the gate reports its decision as a required status check, which repository rules enforce.

The owner and the agents need from the gate:

- **Rules the owner wrote and reviewed,** in files under CODEOWNERS, so no agent can move the boundary of the owner's review on its own (decision of 2026-11-15, NEX-33, "The merge gate is a required status check, and its rules are under CODEOWNERS"; NEX-32).
- **Every rule judged, and a gap seen.** A rule without a verdict, a verdict nobody can read, or rules nobody can read must send the pull request to the owner, never let it through.
- **A decision that comes out the same every time** from the same verdicts. Only the verdict per rule is a model's judgement.
- **A pull request judged by the rules it would replace,** not by the rules it brings (RFC 0006, "Which copy is the truth").
- **Work that carries out a decision the owner already made merges without them** (GENESIS.md, "Decide first, then build").
- **A minimal set that runs on day one,** which the team then owns (GENESIS.md, "Every default belongs to the team").

Two decided RFCs set what this one builds on. RFC 0006 keeps the merge gate's files under `factory/merge-gate/`, installs nexus-flow's minimal set there with `nxs factory install` under its rules for shipped defaults, and fixes that whatever judges a pull request reads the factory's files from the base branch. RFC 0004 keeps the rules on what comes back to the owner as the list after the mark `<!-- nexus-flow: owner-rules -->` in `factory/GENESIS.md` (NEX-70), and requires of this RFC that a pull request goes to the owner whenever that list cannot be read, and that this RFC designs the command or seam that reads it and how its JSON names each rule.

Out of scope, each elsewhere: the text of the shipped rules, prompt and workflow, which are default templates the owner decides in their own ticket (GENESIS.md, "What has to come back through me"); the repository rules themselves (CODEOWNERS, the branch rules and rulesets), which are the team's and, here, the owner's; pm's own review of a pull request, which stays in `roles/pm.md`; the field that links a ticket to its pull request (RFC 0002); and a fast decision model in place of the sub-agent, which GENESIS.md names as a later option outside v2's scope.

This RFC builds on RFC 0001 ("Creating and finding a space", for the repository's root and `[git] base` in `nexus-flow.toml`), RFC 0004 ("The rules on what comes back to the owner") and RFC 0006 ("The factory's files", "Which copy is the truth", "Installing the shipped defaults").

## The design

### The parts and where each runs

The gate is five parts. Two are files the team owns, two are `nxs` commands, and one is a workflow on the forge:

| Part | What it is | Judgement or rule |
| --- | --- | --- |
| The rules | `factory/merge-gate/gate.toml` and the marked list in `factory/GENESIS.md` | Written by the team, reviewed by the owner |
| The rules reader | `nxs merge-gate rules`: reads every rule from the base branch and gives each an id | Deterministic |
| The verdict | A sub-agent judges the pull request against each rule and writes JSON in the format of `factory/merge-gate/verdict.schema.json` | A model's judgement |
| The decision | `nxs merge-gate decide`: reads the rules again, the verdict, the changed paths and the reviews, and decides | Deterministic |
| The status check | The workflow posts the decision as the commit status `merge-gate` on the pull request's head commit; a ruleset requires it | Enforced by the forge |

`nxs` never touches the network (GENESIS.md, principle 3): it reads files and the local git, and its input about the forge, the reviews, comes in as a file. The workflow fetches what the forge knows and posts what `nxs` decided.

### Which copy of the rules judges a pull request

The base branch's, never the pull request's head (RFC 0006). Both commands read the rules from the commit given as `--base`, which the workflow sets to the commit the pull request's base branch points at when the event fires. A pull request that loosens a merge rule, removes a rule from GENESIS or deletes the mark is judged by the rules it would replace, and it touches paths under CODEOWNERS, so it needs the owner's review anyway.

The workflow that judges is read from the base branch too: it runs on `pull_request_target`, which takes the workflow file and the `nxs` it installs from the base branch, and it never checks out or runs the head's code. It fetches the head's commits only as data, to diff them. *Not taken:* `pull_request`, which runs the workflow file of the pull request's own merge ref, so a pull request could change how it is judged. GitHub has no counterpart of `pull_request_target` for reviews, so the run a review starts takes its workflow file from the pull request's merge ref; it never judges, and a pull request that changes it changes `.github/`, which CODEOWNERS holds for the owner (see "The workflow").

### The merge rules

A team's merge rules live in one file, `factory/merge-gate/gate.toml`, beside the gate's settings:

```toml
format = 1

# What the gate decides when no rule says otherwise: "owner" or "pm".
default = "owner"

# The forge logins whose approving review counts as the owner's.
owners = ["cabcookie"]

[[rule]]
id = "decided-design"
effect = "pm"
text = """
The pull request only carries out decisions the owner has already made,
such as a decided RFC, and departs from none of them.
"""

[[rule]]
id = "stored-format"
effect = "owner"
text = """
The pull request changes a stored format.
"""
```

- **`format`** is `1`. It fixes how `nxs merge-gate decide` reads this file and decides; a release of nexus-flow never changes what format 1 decides, so an upgrade of `nxs` cannot change which pull requests reach the owner. A new decision procedure comes as a new format, which a team adopts in a pull request the owner reviews.
- **`default`** is what the gate decides when no rule says otherwise: `owner` or `pm`.
- **`owners`** lists the forge logins whose approving review satisfies the gate when it reserves a pull request for the owner (see "The status check"). It must not be empty. CONFIG.md names the owner for people; this list is the value a script reads (RFC 0006: CONFIG.md holds no value a script reads).
- **Each `[[rule]]`** has an `id` matching `^[a-z][a-z0-9-]{0,62}$`, unique in the file, an `effect`, and a `text` in Markdown that the sub-agent judges. Rules keep their order in the file.

A rule has one of two effects:

- **`owner`: the pull request goes to the owner when it falls under the rule,** unless all it does under the rule is carry out a decision the owner already made. The rules in GENESIS on what comes back to the owner are all of this effect.
- **`pm`: pm may merge when the pull request meets the rule.** It is an exception to `default = "owner"`, such as this repository's "a pull request that only implements a decided RFC". It never overrides an `owner` rule the pull request falls under.

**One TOML file, with ids the team chooses.** The decision must find every rule the same way every time, see which rules have no verdict, and tell the two effects apart; TOML gives each rule an explicit id and effect, an empty set of exceptions is written as no `[[rule]]` of that effect rather than as a missing mark, and the ids stay the same when a rule is added before them. The rules are short, and the owner reviews them in the same diff as the settings. *Not taken:* a Markdown file with marked lists, as GENESIS has (an empty kind could not be told from a lost mark, and the ids would be positions); one file per rule (more files for the owner to review together, and the effect would need a header format of its own); and the rules in `nexus-flow.toml` (that file holds the space ids and is not under CODEOWNERS everywhere, RFC 0001).

**Reading `gate.toml` is deterministic.** The file is read from the base commit. It cannot be read, and the gate decides `owner`, when it is missing, is not valid TOML, has a `format` other than `1`, a `default` other than `owner` or `pm`, an empty or missing `owners`, a rule whose `id` does not match the pattern or repeats, whose `effect` is not `owner` or `pm`, or whose `text` is empty. Unknown keys are ignored, so a team may keep notes beside the rules.

### The rules from GENESIS

`nxs merge-gate rules` reads the rules on what comes back to the owner from `factory/GENESIS.md` at the base commit, as RFC 0004 defines the mark and the list:

- The mark is a line that reads exactly `<!-- nexus-flow: owner-rules -->`, apart from surrounding whitespace, outside a fenced code block.
- The list is a CommonMark list whose first item starts on the first line after the mark that is not blank. Any other line there, such as a paragraph or a heading, means the mark has no list after it (see "Open questions", question 3).
- Each top-level item of that list is one rule, in order, with its text as written, nested items included, from its marker to the line before the next top-level item or the end of the list.

Each GENESIS rule has the effect `owner` and the id `genesis.<n>`, its 1-based position in the list. The dot keeps these ids apart from the ids in `gate.toml`, which cannot contain one.

**When the GENESIS rules cannot be read, the gate decides `owner`** (RFC 0004): when the file is missing at the base commit, has no mark, has more than one, or has no list after the mark.

*Not taken:* ids from the rule's text, such as a hash (an edit of a comma would change the id, and the owner reviews text, not hashes); ids the team writes into GENESIS (one more thing to keep in step in a file whose shape is the team's, and RFC 0004 decided the list as plain list items).

### `nxs merge-gate rules`

`nxs merge-gate rules [--base <ref>]` prints every rule the gate judges, read from `<ref>`; without `--base`, from the branch `[git] base` names in `nexus-flow.toml` (RFC 0001). It finds the repository's root as RFC 0001 does, opens no NoCentralDB database, and reads only the local git. JSON by default, Markdown with `--md`:

```json
{
  "base": "4f1c2e…",
  "readable": true,
  "default": "owner",
  "owners": ["cabcookie"],
  "rules": [
    { "id": "genesis.1", "effect": "owner", "source": "factory/GENESIS.md", "text": "Anything a user sees first: …" },
    { "id": "genesis.2", "effect": "owner", "source": "factory/GENESIS.md", "text": "A change to a stored format, …" },
    { "id": "decided-design", "effect": "pm", "source": "factory/merge-gate/gate.toml", "text": "The pull request only carries out …" }
  ],
  "errors": []
}
```

`base` is the full object id `<ref>` resolved to. The GENESIS rules come first, in their order, then the rules of `gate.toml`, in theirs. When anything cannot be read, `readable` is `false`, `rules` is empty, `default` and `owners` are `null`, and `errors` names each problem as `{ "source": <path>, "code": <code> }`, with the codes `file_missing`, `mark_missing`, `mark_doubled`, `list_missing`, `toml_invalid`, `format_unknown`, `default_invalid`, `owners_empty`, `rule_id_invalid`, `rule_id_duplicate`, `effect_invalid` and `text_empty`; a rule's error also carries `rule`, its id or position. The command then exits non-zero, with the report on standard output, as `nxs genesis create` does for `tracked` (RFC 0004). A team runs it to check its rules before it asks the owner to review them; the workflow runs it to brief the sub-agent.

### The verdict

The sub-agent receives the rules from `nxs merge-gate rules`, the diff between the base and the head, and the prompt `factory/merge-gate/prompt.md`, and writes one JSON document, the verdict, in the format `factory/merge-gate/verdict.schema.json` describes (JSON Schema, draft 2020-12):

```json
{
  "format": 1,
  "base": "4f1c2e…",
  "head": "9a07bd…",
  "verdicts": [
    { "rule": "genesis.1", "outcome": "decided", "reason": "Adds `nxs merge-gate rules` exactly as RFC 0010 names it.",
      "decision": { "path": "docs/rfc/0010-merge-gate.md", "section": "nxs merge-gate rules" } },
    { "rule": "genesis.2", "outcome": "not_concerned", "reason": "No stored format changes." },
    { "rule": "decided-design", "outcome": "met", "reason": "Only implements RFC 0010's sections …" }
  ]
}
```

- **`base` and `head`** are the full object ids the sub-agent judged.
- **One entry per rule,** each with `rule` (the id), `outcome` and `reason` (a non-empty sentence for people).
- **The outcome of an `owner` rule** is `not_concerned` (the pull request does nothing the rule names), `decided` (it falls under the rule, but all it does there carries out a decision the owner already made, which `decision` names), `applies` (it falls under the rule and needs the owner: no decision covers it, or it departs from one) or `unsure`.
- **The outcome of a `pm` rule** is `met`, `not_met` or `unsure`.
- **`decision`** is present exactly when the outcome is `decided`: `path` is a file in the repository at the base commit that holds the decision, such as a decided RFC, and `section`, optional, names the part of it.

The verdict carries the two questions GENESIS.md asks of the sub-agent: whether and how the pull request meets each rule (`outcome`, `reason`), and whether it carries out the owner's decision or departs from it (`decided` with its `decision`, or `applies`).

**A decision is cited by a path in the repository.** The gate's decision runs where the repository is checked out at the base commit and no NoCentralDB database is open, so the one thing it can check about a cited decision is that its file exists there. The decisions in RFCs, which are where the owner decides a design before it is built, are in the repository. *Not taken:* citing a decision record or a ticket by its handle (RFC 0003, RFC 0002), which the workflow cannot read; a pull request whose only cover is such a record goes to the owner, unless the team's own `pm` rule covers it.

The schema and the prompt are the team's files: the team may tighten the schema or reword the prompt. `nxs merge-gate decide` does not read either; it reads format 1 as this section defines it, and a verdict that does not fit it sends the pull request to the owner.

### `nxs merge-gate decide`

`nxs merge-gate decide --base <sha> --head <sha> [--verdict <file>] [--reviews <file>]` decides, and prints the decision. It opens no NoCentralDB database and reads only the local git and the two files, so the same commits and the same file bytes give the same output bytes.

It reads the rules from `--base` as `nxs merge-gate rules` does, takes the changed paths from the diff between the merge base of `--base` and `--head` and `--head` (both paths of a rename count), and reads the repository's CODEOWNERS file from the base commit, in the places and order GitHub looks for it and with RFC 0006's matcher. Then, in this order:

1. **The rules cannot be read:** `owner`, with the reason `rules_unreadable` and the errors of `nxs merge-gate rules`. Nothing else is checked.
2. **The verdict is not valid:** `owner`, with the reason `verdict_invalid` and what is wrong. A verdict is valid when the file is JSON with `format` 1; its `base` and `head` equal `--base` and `--head`; it has exactly one entry for each rule id and none for an id that is not a rule; each outcome is one its rule's effect allows; each `reason` is a non-empty string; and `decision` is present exactly for `decided`, with a `path` that names a file at the base commit. Without `--verdict`, or with a file that does not exist, there is no valid verdict. Unknown fields are ignored.
3. **Every reason the pull request needs the owner is collected,** in this order: `codeowners`, with the changed paths a CODEOWNERS rule covers; then, for each rule in order, `unsure` for a rule whose outcome is `unsure`, and `applies` for an `owner` rule whose outcome is `applies`. If any is found, the decision is `owner`.
4. **Otherwise the pull request is pm's** when `default` is `pm`, with the reason `default`, or when a `pm` rule is `met`, with the reason `pm_rule` naming each such rule. When neither holds, the decision is `owner`, with the reason `default`.

A rule outcome of `not_concerned`, `decided` or `not_met` adds no reason of its own.

**CODEOWNERS is part of the decision** although the forge enforces it already (GENESIS.md, "The merge gate", step 1): a pull request the forge holds for a code owner's review cannot be merged by pm either, and the gate's decision should say so instead of reading `pm`. *Not taken:* leaving it to the forge alone, which would let the check say `pm` for a pull request pm cannot merge.

**The owner's approval.** `--reviews` is the pull request's reviews as GitHub's "list reviews for a pull request" endpoint returns them, all pages as one JSON array; the gate reads `user.login`, `state`, `commit_id` and `submitted_at`. The owner has approved when, for some login in `owners` (compared without regard to ASCII case), the latest of that login's reviews in the state `APPROVED`, `CHANGES_REQUESTED` or `DISMISSED`, by `submitted_at` and then by position in the array, is `APPROVED` and its `commit_id` is `--head`. Without `--reviews`, the owner has not approved. An approval of an earlier commit does not count, as the forge's dismissal of stale approvals would have it.

**Output.** JSON by default, Markdown with `--md`:

```json
{
  "base": "4f1c2e…",
  "head": "9a07bd…",
  "decision": "owner",
  "reasons": [
    { "code": "codeowners", "paths": [".github/workflows/ci.yml"] },
    { "code": "applies", "rule": "genesis.2", "reason": "Adds a field to the ticket record." }
  ],
  "owner_approved": false,
  "status": "pending",
  "description": "Waits for the owner: CODEOWNERS, genesis.2"
}
```

- **`decision`** is `owner` or `pm`.
- **`reasons`** lists why, with `code` one of `rules_unreadable` (with `errors`), `verdict_invalid` (with `detail`, one line per problem), `codeowners` (with `paths`, sorted bytewise), `unsure` and `applies` (with `rule` and the verdict's `reason`), `pm_rule` (with `rules`), and `default`.
- **`owner_approved`** says whether the owner approved the head commit.
- **`status`** is the state to post: `success` when `decision` is `pm` or the owner approved, otherwise `pending`.
- **`description`** is a line of at most 140 characters for the status, built from `decision` and the reasons' codes and rule ids.

The command exits 0 whenever it decided, `owner` included, and non-zero only when it cannot decide: a flag is missing, or `--base` or `--head` is not a commit in the local git. Then it posts nothing, and the required status check stays missing, which blocks the merge.

### The status check

The workflow posts `status` and `description` as the commit status with the context `merge-gate` on the head commit, and writes the full decision into the run's summary, where pm and the owner read the reasons.

A ruleset on the base branch requires the status check `merge-gate`, with its source pinned to the app that runs the workflow (GitHub Actions), so a status posted with any other token, such as an agent account's, does not count. Then:

- **A pull request the gate gives to pm** has `merge-gate` green. pm merges it once its own review in `roles/pm.md` passes and every other required check is green; the gate allows the merge, it does not make it.
- **A pull request the gate reserves for the owner** has `merge-gate` pending, and nobody can merge it, pm included. When the owner approves its head commit, the review event runs the gate again, `owner_approved` is `true`, and the status turns green.
- **A new push** runs the gate again on the new head: a new verdict, a new decision, and an approval of the old head no longer counts.

The owner's approval satisfies the gate through the gate's own status, because a ruleset cannot require a review only for some pull requests: requiring the owner's review on every pull request would leave nothing for pm to merge, and requiring it on none would leave the gate's decision unenforced. *Not taken:* a failing status while the pull request waits for the owner (waiting is not an error, and pm and the owner would have to tell a broken gate from a waiting one by the description); and enforcing the decision through pm's prompt alone, which NEX-33 decided against.

### The workflow

The shipped workflow, which the team copies into `.github/workflows/` (see "The minimal set nexus-flow ships"), runs on `pull_request_target` when a pull request is opened, reopened, marked ready for review or pushed to, and on `pull_request_review` when a review is submitted or dismissed. It skips draft pull requests, which cannot merge, so a coding agent's pushes during its work cost no verdict. On a pull request event, in one job, it:

1. checks out the base commit and fetches the head's commits as data, without checking them out;
2. installs `nxs` at a version the workflow pins, with its checksum;
3. runs `nxs merge-gate rules --base <base>` and, when the rules are readable, runs the sub-agent with the rules, the diff and `prompt.md`, which writes `verdict.json`;
4. fetches the pull request's reviews into `reviews.json`;
5. runs `nxs merge-gate decide --base <base> --head <head> --verdict verdict.json --reviews reviews.json` and posts its `status` as `merge-gate`.

On a review event it does not judge again. It runs only for a review by a login in `owners` whose state is `APPROVED`, `CHANGES_REQUESTED` or `DISMISSED`, and runs `nxs merge-gate decide` without `--verdict`, so the decision is `owner` and the status is green exactly when the owner approved the head commit. An owner's request for changes therefore turns a green `merge-gate` pending, and the next push judges again.

The sub-agent's step is the agent framework's (GENESIS.md, principle 6, "Bring your own agents"): the shipped workflow names one framework's action as its example, and a team uses its own. Where the sub-agent runs is the first open question.

### The minimal set nexus-flow ships

`nxs factory install` installs these under `factory/merge-gate/`, as RFC 0006's shipped defaults (`written`, `current`, `older_default`, `updated`, `team`, `retired`), and adds their versions to `templates/factory/shipped.toml`:

| File | What it holds |
| --- | --- |
| `gate.toml` | `format = 1`, `default = "owner"`, an `owners` list the team fills in, and one `pm` rule: the pull request only carries out decisions the owner has already made and departs from none of them |
| `verdict.schema.json` | The verdict's format 1 as a JSON Schema |
| `prompt.md` | The sub-agent's instructions: judge each rule against the diff, cite the decision a `decided` outcome rests on, answer `unsure` rather than guess, and write a verdict that fits the schema |
| `github-workflow.yml` | The workflow above, for the team to copy into `.github/workflows/merge-gate.yml` |

Together with the rules in the GENESIS template (RFC 0004), this minimal gate sends to the owner every pull request that does anything other than carry out the owner's decisions, every one that touches a rule in GENESIS without a decision behind it, and every one the gate cannot judge. The text of each file is a default template, which the owner decides in its own ticket.

**`nxs` never writes outside `factory/`.** The workflow is shipped as a file in `factory/merge-gate/` for the team to copy, because `.github/` and the rulesets are the team's own and come back to the owner (GENESIS.md, "What has to come back through me"), and because RFC 0006 installs nothing outside `factory/`. `nxs factory install` reports which of the gate's files no CODEOWNERS rule covers (RFC 0006). *Not taken:* writing `.github/workflows/merge-gate.yml` from `nxs factory install` (nexus-flow would write a file that runs with the repository's secrets, outside the folder it owns).

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:** `nxs merge-gate rules [--base <ref>]` and `nxs merge-gate decide --base <sha> --head <sha> [--verdict <file>] [--reviews <file>]`, both with `--md`. The noun is `merge-gate`, not `gate`, because "gate" already names CI's gate command in a factory's CONFIG.md.
- **JSON output** of `nxs merge-gate rules`: `base`, `readable`, `default`, `owners`, `rules` (each with `id`, `effect`, `source`, `text`), `errors` (each with `source`, `code`, and `rule` where it concerns one), with the error codes above. Of `nxs merge-gate decide`: `base`, `head`, `decision`, `reasons` (each with `code` and, by code, `errors`, `detail`, `paths`, `rule`, `reason`, `rules`), `owner_approved`, `status`, `description`.
- **Library seam:** `Engine::merge_gate_rules(base: &str)`, returning the rules report, and `Engine::merge_gate_decide(base: &str, head: &str, verdict: Option<&[u8]>, reviews: Option<&[u8]>)`, returning the decision report: the counterparts of the commands (the decision on CLI and library parity stays in force).
- **Stored formats:** `factory/merge-gate/gate.toml` (format 1: `format`, `default`, `owners`, `[[rule]]` with `id`, `effect`, `text`); the GENESIS rule ids `genesis.<n>`; the verdict's format 1 and `factory/merge-gate/verdict.schema.json`; `factory/merge-gate/prompt.md` and `factory/merge-gate/github-workflow.yml` as shipped files.
- **Forge surface:** the commit status context `merge-gate` and its states `success` and `pending`; the reviews input in GitHub's format.

### What nexus-flow needs from NoCentralDB

Nothing. The gate reads files and git at the base commit, and its input from the forge comes as a file, so this RFC adds no collection to the model and no request to NoCentralDB.

### What the build must prove

The coding ticket that builds the two commands proves at the seam (`Engine::merge_gate_rules`, `Engine::merge_gate_decide`) that the gate never gives a pull request to pm when it should not, with tests that each fail if the guarantee is broken: the decision is `owner` and `status` `pending` when GENESIS is missing at the base, has no mark, two marks, or a paragraph between the mark and the list; when `gate.toml` is missing, invalid, of another format, or has no owners; when the verdict file is missing, not JSON, of another format, for another base or head, missing a rule, carrying an unknown rule, an outcome the rule's effect does not allow, or a `decision` whose path does not exist at the base; when any rule is `unsure`; when an `owner` rule `applies` even though a `pm` rule is `met`; when a changed path, the old path of a rename included, is covered by CODEOWNERS at the base; and when a rule was loosened only on the head. It proves that the owner's approval counts only on the head commit and only when it is the owner's latest deciding review, that a login outside `owners` never counts, and that the same inputs give the same output bytes.

## What it does not do

- **It does not make the verdict right.** The verdict per rule is a model's judgement (GENESIS.md, "Limits"). A sub-agent can misjudge a rule, and text in the diff can try to steer it. The gate makes the decision on top of the verdicts deterministic and fails safe on every gap it can see; it cannot see a confident wrong verdict.
- **It checks only that a cited decision exists.** `decided` needs a file at the base commit; whether that file is a decided design and whether the pull request follows it is the sub-agent's judgement.
- **It does not set up the repository rules.** The guarantee holds only while a ruleset requires `merge-gate` from the app that runs the workflow, and while CODEOWNERS covers `.github/`, `factory/merge-gate/` and `factory/GENESIS.md` with the owner. `nxs` reads no forge settings, so it cannot check the ruleset; `nxs factory install` reports the CODEOWNERS gaps (RFC 0006). Without these, an agent with write access can post the status itself, or change the workflow or the rules in the pull request they judge.
- **It does not judge again when the rules change.** A verdict is made against the base commit at the time of the event. If the base branch's rules change later, an open pull request keeps its status until its next push, review or a re-run.
- **It does not merge.** A green `merge-gate` allows pm to merge; pm's own review and the other required checks still apply.
- **It does not check what the owner read.** An approving review by an owner on the head commit satisfies the gate, whatever the decision's reasons were.
- **GENESIS rule ids are positions.** `genesis.2` is the second rule at that base commit; a rule inserted above it renumbers the rest, so ids from different commits cannot be compared.
- **It speaks GitHub.** The reviews input, the workflow and the status check are GitHub's. Another forge needs its own workflow and a reviews input of its shape, in a later format.
- **The review run is not read from the base branch.** It takes its workflow file from the pull request's merge ref, so a pull request that edits the workflow could change what a review run posts; only CODEOWNERS on `.github/` keeps such a pull request with the owner.
- **It does not judge drafts.** The shipped workflow skips them, so a draft's `merge-gate` is missing until it is marked ready.

## Migration

Nothing to migrate from v1. v1 has no merge gate; its step chains, whose last step could be a judge, lapse with the rest of chat (NEX-20).

For this repository, the gate takes effect in steps that each change a path under CODEOWNERS or a ruleset, so each is a pull request or a setting the owner approves: add `/factory/merge-gate/` to `.github/CODEOWNERS`; add the mark before the list in GENESIS.md's "What has to come back through me"; add `factory/merge-gate/` with this repository's rules, among them the exception for a pull request that only implements a decided RFC that CONFIG.md and `roles/pm.md` state today; add the workflow, which on `v2` builds `nxs` from the base commit, since `v2` cuts no release; require `merge-gate` in `v2`'s ruleset; and rewrite CONFIG.md's merge rule and `roles/pm.md` to point at the gate. Until the last step, the merge rule in CONFIG.md applies as it does today.

## Open questions

1. **Where the sub-agent runs.**
   - *In the gate's workflow (recommended).* The workflow calls a model with a key kept as a repository secret. No agent whose pull request is judged can write the verdict, because every agent here works under the same agent account. Cost: a model call per ready pull request and per push to one, on a key for a paid service, which comes back to the owner under "Money".
   - *In pm's session.* pm runs a sub-agent and posts the verdict as a pull request comment; the workflow reads the newest verdict comment from the agent account for the head commit. No key in CI and no cost beyond pm's session, but any session on the agent account, the coder whose pull request is judged included, could post a verdict, so the gate's input rests on a prompt.

2. **Who owns the decision's code.**
   - *Built into `nxs` (recommended),* as `nxs merge-gate decide`, with what each `format` decides fixed for good. It is tested in nexus-flow's suite, runs the same on every platform without another runtime, and an upgrade cannot change it. The team owns the rules, the schema, the prompt and its workflow, and a team that wants another procedure replaces the decide step in its workflow with its own script that reads the same JSON. This reads GENESIS.md's "the script belongs to the team" as "the team chooses it", not "the team edits nexus-flow's".
   - *A script file under `factory/merge-gate/`* that nexus-flow ships and the team edits, such as a Node script. Closer to the letter of GENESIS.md, but each team then runs and tests its own copy of the one deterministic step, and a broken edit can let pull requests through.

3. **How close the list must follow the mark in GENESIS.** RFC 0004 reads "the top-level items of the first list after the mark".
   - *Directly after it, with only blank lines between (recommended),* as written above. If the list under the mark is deleted, the gate reads no rules and sends every pull request to the owner.
   - *The first list anywhere after the mark.* Matches RFC 0004's words most literally, but a deleted list makes the gate read some later list in GENESIS as the rules, silently.

## History

- 2026-11-20: published as a draft in #171.

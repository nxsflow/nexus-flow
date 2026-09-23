# Command-Line Help for `nxf`

This document contains the help content for the `nxf` command-line program.

**Command Overview:**

* [`nxf`↴](#nxf)
* [`nxf init`↴](#nxf-init)
* [`nxf agent-manifest`↴](#nxf-agent-manifest)
* [`nxf create`↴](#nxf-create)
* [`nxf update`↴](#nxf-update)
* [`nxf claim`↴](#nxf-claim)
* [`nxf archive`↴](#nxf-archive)
* [`nxf unarchive`↴](#nxf-unarchive)
* [`nxf close`↴](#nxf-close)
* [`nxf list`↴](#nxf-list)
* [`nxf blocked`↴](#nxf-blocked)
* [`nxf deferred`↴](#nxf-deferred)
* [`nxf closed`↴](#nxf-closed)
* [`nxf archived`↴](#nxf-archived)
* [`nxf recap`↴](#nxf-recap)
* [`nxf next`↴](#nxf-next)
* [`nxf dep`↴](#nxf-dep)
* [`nxf dep add`↴](#nxf-dep-add)
* [`nxf dep remove`↴](#nxf-dep-remove)
* [`nxf mention`↴](#nxf-mention)
* [`nxf mention add`↴](#nxf-mention-add)
* [`nxf mention remove`↴](#nxf-mention-remove)
* [`nxf mention list`↴](#nxf-mention-list)
* [`nxf contributes`↴](#nxf-contributes)
* [`nxf contributes add`↴](#nxf-contributes-add)
* [`nxf contributes remove`↴](#nxf-contributes-remove)
* [`nxf contributes list`↴](#nxf-contributes-list)
* [`nxf thread`↴](#nxf-thread)
* [`nxf thread link`↴](#nxf-thread-link)
* [`nxf thread unlink`↴](#nxf-thread-unlink)
* [`nxf thread list`↴](#nxf-thread-list)
* [`nxf label`↴](#nxf-label)
* [`nxf label add`↴](#nxf-label-add)
* [`nxf label remove`↴](#nxf-label-remove)
* [`nxf label list`↴](#nxf-label-list)
* [`nxf note`↴](#nxf-note)
* [`nxf note add`↴](#nxf-note-add)
* [`nxf note list`↴](#nxf-note-list)
* [`nxf search`↴](#nxf-search)
* [`nxf show`↴](#nxf-show)
* [`nxf schema`↴](#nxf-schema)
* [`nxf setup`↴](#nxf-setup)
* [`nxf setup claude`↴](#nxf-setup-claude)
* [`nxf guide`↴](#nxf-guide)

## `nxf`

nexus-flow agent CLI

**Usage:** `nxf [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `init` — Initialize a `.nxs` workspace in the current directory: set up flow, then delegate the shared agent file + one SessionStart hook per active module to the `nxs` assembler. To set up the whole suite (and pick tools interactively), use `nxs init` instead
* `agent-manifest` — Emit nxf's declared contribution to the shared agent file (the AGENTS.md section, the prime command, the SessionStart hook) as data. The `nxs` umbrella assembles the shared files from each active module's manifest; `--json` is the machine contract
* `create` — Create a new item — a complete item in one call. Title, description, and priority are required; design, the definition of done, a parent, and dependencies are optional. Run `nxf schema` for the active plugin's field model (labels, constraints, required fields)
* `update` — Update item fields via `--set field=value` (repeatable). The long-form `description`/`design`/`completion_criterion` are ordinary `--set` fields. Run `nxf schema` for the settable fields and their plugin names. Convention: correct the field model once shortly after creation; once it is stable, record what you learn as append-only notes (`nxf note add`) rather than further field edits
* `claim` — Claim an item: mark it in progress (and optionally assign it)
* `archive` — Archive one or more items — "closed and put away". Cascades DOWN: each root and its whole `belongs_to` subtree are archived, but only when the root is closed and every descendant is closed (already-archived counts as closed). Atomic per root, independent between roots; the result reports each root's outcome plus the full list of ids actually archived (incl. cascaded). Archiving is reversible — see `unarchive`
* `unarchive` — Unarchive one or more items, making them visible again. Cascades UP only: each item and its ancestor chain resurface, but its children/siblings stay archived. Partial per root; the result reports each root's outcome plus the full list of ids actually unarchived
* `close` — Close an item, recording why. Closing is final (no hard delete), so a reason is required
* `list` — List items, optionally filtered by status and/or type. Id-ordered by default (so the `--json` record stays plugin-independent); override with `--sort rank`
* `blocked` — List blocked items (open, with an open blocker or in a cycle). Ranked by default; override with `--sort id`
* `deferred` — List deferred items (open, unblocked, with a future defer date) — the lane that is neither ready nor blocked. Ordered by defer-date ascending (soonest first); override with `--sort`
* `closed` — List closed items (excluding archived). Ordered by close-date descending (most recent first); override with `--sort`
* `archived` — List archived items (any status). Ordered by archive-date descending (most recent first); override with `--sort`
* `recap` — Recap the most recently closed work — "what got finished lately" — newest close first, archived closes INCLUDED. A recency recall view, distinct from `closed` (the lane, which drops archived). `--limit` caps the count (default 10); `--since` keeps only later closes
* `next` — List actionable work — ready plus already-claimed, unblocked, not deferred — ranked by the active plugin's `next` policy and tiered to finish before starting: work you can close now, then each started epic with its open children, then the backlog. `--sort id` gives the flat, untiered order; `--limit` shows only the head of it, and says so
* `dep` — Add or remove a dependency edge. Direction: `dep add A B` makes A depend on B, so B blocks A — A cannot be ready until B closes
* `mention` — Add, remove, or list reference edges (free-text short-id citations)
* `contributes` — Add, remove, or list contributes-to edges: `from` contributes to `to` (n:m). Like a mention it never blocks — it records a structural "feeds into" relation, not a dependency
* `thread` — Link a chat thread to a board item, or list what a thread is about. This is how the conversation that did the work and the ticket it was about stop being two artefacts: a later reader of the item sees that there are conversations, without having to go looking
* `label` — Add, remove, or list user labels (free-text tags) on an item. A label is user vocabulary, distinct from the plugin's display type; filter `list`/`next` with `--label`
* `note` — Add or list notes (the change/worklog stream) on an item
* `search` — Search items by substring over title, description, design, DoD, and notes. Results are grouped by lane priority (next+in-progress → blocked → deferred → closed), each group in its natural order; archived items are excluded by default
* `show` — Show one item with its dependencies and notes
* `schema` — Describe the active plugin's field model — vocabulary, priorities, and per field whether it is required on create / settable on update and how. `--json` is the machine contract; run it to learn what `create`/`update` accept in this workspace (the static `--help` cannot carry the active plugin's vocabulary)
* `setup` — Wire host-specific agent integration (idempotent, merge-only). `nxf init` stays tool-agnostic; this opts a project into a host's deterministic delivery
* `guide` — Print an embedded, offline guide. Omit the topic to list the available ones

###### **Options:**

* `--json` — Emit machine-readable JSON instead of human output. `priority` and `type` carry canonical keys (stable ordinals / declared type keys), not display labels; the labels are available via `nxf schema` and, on `show`/`list`/`next`, the additive `priority_label`/`type_label` fields. `create`/`update` accept either the key or the label
* `--db <DB>` — Use this sqlite db file directly instead of discovering `.nxs/`



## `nxf init`

Initialize a `.nxs` workspace in the current directory: set up flow, then delegate the shared agent file + one SessionStart hook per active module to the `nxs` assembler. To set up the whole suite (and pick tools interactively), use `nxs init` instead

**Usage:** `nxf init [OPTIONS]`

###### **Options:**

* `--plugin <PLUGIN>` — Plugin to activate (vocabulary + ranking): `issue-tracker` | `personal-todo`. Required when non-interactive (`--json`/piped/`--quiet`); on a TTY, omitting it prompts
* `--quiet` — Driven/quiet mode: set up the workspace and delegate the agent files + the SessionStart hooks, but render no banner or prompt. The seam the `nxs` umbrella uses to drive `nxf init` without its output bleeding through; implies a non-interactive plugin choice (`--plugin`)
* `--from-beads` — Consent to the beads → nxs migration: in a project that still uses beads, import its tickets + memories and roll back beads' config. Routed through the `nxs` umbrella, which owns the migration



## `nxf agent-manifest`

Emit nxf's declared contribution to the shared agent file (the AGENTS.md section, the prime command, the SessionStart hook) as data. The `nxs` umbrella assembles the shared files from each active module's manifest; `--json` is the machine contract

**Usage:** `nxf agent-manifest`



## `nxf create`

Create a new item — a complete item in one call. Title, description, and priority are required; design, the definition of done, a parent, and dependencies are optional. Run `nxf schema` for the active plugin's field model (labels, constraints, required fields)

**Usage:** `nxf create [OPTIONS] [-]`

###### **Arguments:**

* `<->` — `-` reads the whole item as one JSON object from STDIN (every field in one piped payload — no escaping, no flag combinatorics; pair with `--json` for JSON output too, the canonical form `nxf create --json -`). The only accepted value is `-`, and it is mutually exclusive with the field flags

###### **Options:**

* `--type <TY>` — Item type, from the active plugin's own vocabulary (under `issue-tracker`: `epic`, `feature`, `bug`, `chore`, `decision`). The set is plugin-declared, so run `nxf schema` for the one this workspace actually accepts
* `--title <TITLE>` — Item title (required)
* `--description <DESCRIPTION>` — Description — why this item exists and its goal (required). Pass `-` to read it from STDIN (escaping-free, multi-line; e.g. `cat desc.md | nxf create … --description -`), or supply it via `--description-file` instead
* `--description-file <DESCRIPTION_FILE>` — Read the description from this file (UTF-8, verbatim); `-` reads STDIN. Mutually exclusive with `--description`
* `--priority <PRIORITY>` — Priority — the plugin's named variant (e.g. P0..P4) OR its canonical ordinal key (0..4, the form `--json` emits), required; run `nxs prime`/`nxf schema` to see the set. Validated against it; ranks `next` by the variant order
* `--design <DESIGN>` — Design — the path to the goal; can be filled in later (optional). `-` reads it from STDIN
* `--design-file <DESIGN_FILE>` — Read the design from this file (UTF-8, verbatim); `-` reads STDIN
* `--dod <DOD>` — Definition of done — what "done" means for this item (optional). `-` reads it from STDIN
* `--dod-file <DOD_FILE>` — Read the definition of done from this file (UTF-8, verbatim); `-` reads STDIN
* `--due <DUE>` — Due date (ISO-8601 / RFC3339)
* `--defer <DEFER>` — Defer-until date (ISO-8601 / RFC3339)
* `--parent <PARENT>` — Parent item id (sets belongs-to); must exist
* `--depends-on <DEPENDS_ON>` — This item depends on <id> (repeatable): <id> must exist and blocks this item until it closes. Same direction as `dep add <this> <id>`
* `--set <SET>` — Set a plugin CUSTOM field: `name=value` (repeatable). Canonical fields use their own flags above; `--set` carries the active plugin's declared custom fields (run `nxf schema` for the set). Validated against the field's type; a value of `-` reads STDIN
* `--set-file <SET_FILE>` — Set a custom field from a file: `name=path` (UTF-8, verbatim); `path` of `-` reads STDIN. Repeatable. A field may be set by `--set` or `--set-file`, not both
* `-q`, `--id-only` — Print ONLY the new item's id — one line, no JSON, no output framing — so it can be captured straight into the next command (`id=$(nxf create … -q)`) without parsing JSON. Takes precedence over `--json` (the id wins; `--json` is ignored, not an error)



## `nxf update`

Update item fields via `--set field=value` (repeatable). The long-form `description`/`design`/`completion_criterion` are ordinary `--set` fields. Run `nxf schema` for the settable fields and their plugin names. Convention: correct the field model once shortly after creation; once it is stable, record what you learn as append-only notes (`nxf note add`) rather than further field edits

**Usage:** `nxf update [OPTIONS] <ID> [-]`

###### **Arguments:**

* `<ID>` — Item id
* `<->` — `-` reads the fields to set as one JSON object from STDIN (`{"description":"…", "design":"…"}`) — every field in one piped payload, no escaping. The only accepted value is `-` (canonical form `nxf update <id> --json -`), mutually exclusive with `--set`/`--set-file`

###### **Options:**

* `--set <SET>` — A `field=value` assignment; repeat for multiple fields. An unknown field is rejected with the settable set listed. Aliases (same words `create` uses): `parent`→belongs_to, `defer`→defer_until. A value of `-` reads that field from STDIN (escaping-free, multi-line); at most one field per call may read STDIN
* `--set-file <SET_FILE>` — A `field=path` assignment that reads the field's value from a file (UTF-8, verbatim); `path` of `-` reads STDIN. Repeatable; lets several long-text fields each come from their own file in one call. A field may be set by `--set` or `--set-file`, not both



## `nxf claim`

Claim an item: mark it in progress (and optionally assign it)

**Usage:** `nxf claim [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>` — Item id

###### **Options:**

* `--assignee <ASSIGNEE>` — Assignee to record



## `nxf archive`

Archive one or more items — "closed and put away". Cascades DOWN: each root and its whole `belongs_to` subtree are archived, but only when the root is closed and every descendant is closed (already-archived counts as closed). Atomic per root, independent between roots; the result reports each root's outcome plus the full list of ids actually archived (incl. cascaded). Archiving is reversible — see `unarchive`

**Usage:** `nxf archive <IDS>...`

###### **Arguments:**

* `<IDS>` — Item ids to archive (each a cascade root). At least one is required



## `nxf unarchive`

Unarchive one or more items, making them visible again. Cascades UP only: each item and its ancestor chain resurface, but its children/siblings stay archived. Partial per root; the result reports each root's outcome plus the full list of ids actually unarchived

**Usage:** `nxf unarchive <IDS>...`

###### **Arguments:**

* `<IDS>` — Item ids to unarchive (each surfaces its ancestor chain). At least one is required



## `nxf close`

Close an item, recording why. Closing is final (no hard delete), so a reason is required

**Usage:** `nxf close [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>` — Item id

###### **Options:**

* `--reason <REASON>` — Closing comment — required: why this item is being closed. `-` reads it from STDIN
* `--reason-file <REASON_FILE>` — Read the closing comment from this file (UTF-8, verbatim); `-` reads STDIN. Mutually exclusive with `--reason`



## `nxf list`

List items, optionally filtered by status and/or type. Id-ordered by default (so the `--json` record stays plugin-independent); override with `--sort rank`

**Usage:** `nxf list [OPTIONS]`

###### **Options:**

* `--status <STATUS>` — Filter by status
* `--type <TY>` — Filter by type, using the active plugin's vocabulary (see `nxf schema`)
* `--label <LABEL>` — Filter to items carrying this label (user vocabulary, not the display type)
* `--sort <SORT>` — Sort order: `rank` | `id` (default `id`). An unknown key is a loud error



## `nxf blocked`

List blocked items (open, with an open blocker or in a cycle). Ranked by default; override with `--sort id`

**Usage:** `nxf blocked [OPTIONS]`

###### **Options:**

* `--sort <SORT>` — Sort order: `rank` | `id` (default `rank`). An unknown key is a loud error



## `nxf deferred`

List deferred items (open, unblocked, with a future defer date) — the lane that is neither ready nor blocked. Ordered by defer-date ascending (soonest first); override with `--sort`

**Usage:** `nxf deferred [OPTIONS]`

###### **Options:**

* `--now <NOW>` — Reference time (ISO-8601 / RFC3339); defaults to now. Sets the defer boundary
* `--sort <SORT>` — Sort order: `defer` | `id` | `rank` (default `defer`). An unknown key is a loud error



## `nxf closed`

List closed items (excluding archived). Ordered by close-date descending (most recent first); override with `--sort`

**Usage:** `nxf closed [OPTIONS]`

###### **Options:**

* `--sort <SORT>` — Sort order: `closed` | `id` | `rank` (default `closed`). An unknown key is a loud error



## `nxf archived`

List archived items (any status). Ordered by archive-date descending (most recent first); override with `--sort`

**Usage:** `nxf archived [OPTIONS]`

###### **Options:**

* `--sort <SORT>` — Sort order: `archived` | `id` | `rank` (default `archived`). An unknown key is a loud error



## `nxf recap`

Recap the most recently closed work — "what got finished lately" — newest close first, archived closes INCLUDED. A recency recall view, distinct from `closed` (the lane, which drops archived). `--limit` caps the count (default 10); `--since` keeps only later closes

**Usage:** `nxf recap [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>` — Max items to show (default 10)
* `--since <SINCE>` — Only closes on/after this ISO-8601 date (`YYYY-MM-DD` or RFC3339); filters `closed_at >= since`



## `nxf next`

List actionable work — ready plus already-claimed, unblocked, not deferred — ranked by the active plugin's `next` policy and tiered to finish before starting: work you can close now, then each started epic with its open children, then the backlog. `--sort id` gives the flat, untiered order; `--limit` shows only the head of it, and says so

**Usage:** `nxf next [OPTIONS]`

###### **Options:**

* `--now <NOW>` — Reference time (ISO-8601 / RFC3339); defaults to now
* `--label <LABEL>` — Filter to ready items carrying this label (user vocabulary, not the display type)
* `--sort <SORT>` — Sort order: `rank` | `id` (default `rank`). An unknown key is a loud error
* `--limit <LIMIT>` — Max items to show (default: no limit). Applied last — after `--sort` and `--label` — and never silently: a truncated list is headed `showing <n> of <total>`, and under `--json` the flag wraps the records as `{"items": [...], "total": <n>}` so a consumer reads the untruncated total instead of inferring it from the array's length



## `nxf dep`

Add or remove a dependency edge. Direction: `dep add A B` makes A depend on B, so B blocks A — A cannot be ready until B closes

**Usage:** `nxf dep <COMMAND>`

###### **Subcommands:**

* `add` — Make `from` depend on `to`: `to` blocks `from`, so `from` stays blocked until `to` closes. Rejected if it would create a cycle
* `remove` — Remove the dependency where `from` depends on `to`



## `nxf dep add`

Make `from` depend on `to`: `to` blocks `from`, so `from` stays blocked until `to` closes. Rejected if it would create a cycle

**Usage:** `nxf dep add <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`



## `nxf dep remove`

Remove the dependency where `from` depends on `to`

**Usage:** `nxf dep remove <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`



## `nxf mention`

Add, remove, or list reference edges (free-text short-id citations)

**Usage:** `nxf mention <COMMAND>`

###### **Subcommands:**

* `add` — Record that `from` cites the short-id `to` in its free text (no blocking)
* `remove` — Remove the `from -> to` reference
* `list` — List the short-ids `id` mentions



## `nxf mention add`

Record that `from` cites the short-id `to` in its free text (no blocking)

**Usage:** `nxf mention add <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`



## `nxf mention remove`

Remove the `from -> to` reference

**Usage:** `nxf mention remove <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`



## `nxf mention list`

List the short-ids `id` mentions

**Usage:** `nxf mention list <ID>`

###### **Arguments:**

* `<ID>`



## `nxf contributes`

Add, remove, or list contributes-to edges: `from` contributes to `to` (n:m). Like a mention it never blocks — it records a structural "feeds into" relation, not a dependency

**Usage:** `nxf contributes <COMMAND>`

###### **Subcommands:**

* `add` — Record that `from` contributes to `to` (both must exist; never blocks)
* `remove` — Remove the `from -> to` contributes-to edge
* `list` — List the items `id` contributes to



## `nxf contributes add`

Record that `from` contributes to `to` (both must exist; never blocks)

**Usage:** `nxf contributes add <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`



## `nxf contributes remove`

Remove the `from -> to` contributes-to edge

**Usage:** `nxf contributes remove <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`



## `nxf contributes list`

List the items `id` contributes to

**Usage:** `nxf contributes list <ID>`

###### **Arguments:**

* `<ID>`



## `nxf thread`

Link a chat thread to a board item, or list what a thread is about. This is how the conversation that did the work and the ticket it was about stop being two artefacts: a later reader of the item sees that there are conversations, without having to go looking.

n:m in both directions, and never blocking — a link carries no dependency semantics at all. It may be made at any point in the thread's life, changed, and taken back again.

**Usage:** `nxf thread <COMMAND>`

###### **Subcommands:**

* `link` — Link `thread` to `item`. Re-running with different attributes UPDATES the link rather than adding a second one — that is how a link firms up once a passing mention turns out to be the subject
* `unlink` — Unlink `thread` from `item`, whatever attributes the link currently carries
* `list` — List the board items `thread` is linked to



## `nxf thread link`

Link `thread` to `item`. Re-running with different attributes UPDATES the link rather than adding a second one — that is how a link firms up once a passing mention turns out to be the subject

**Usage:** `nxf thread link [OPTIONS] <THREAD> <ITEM>`

###### **Arguments:**

* `<THREAD>` — The chat thread's own id. Not resolved or shape-checked: the thread lives in the chat store's id space, which flow holds the address of but does not own
* `<ITEM>` — The board item

###### **Options:**

* `--relation <RELATION>` — What the link consists in: `worked_on` (this thread is working on the item) or `cited` (it refers to the item without working on it). Default `worked_on` — the deterministic case, the one a work order's ticket set produces with no judgement involved

  Default value: `worked_on`
* `--weight <WEIGHT>` — How much the thread is about the item: `bearing` (the item is its subject) or `passing` (it came up, no more). Default `bearing`. Only `bearing` links are advertised by `next`; a `passing` one shows on the item itself, so a wandering conversation cannot flood the work list with items it merely touched

  Default value: `bearing`



## `nxf thread unlink`

Unlink `thread` from `item`, whatever attributes the link currently carries

**Usage:** `nxf thread unlink <THREAD> <ITEM>`

###### **Arguments:**

* `<THREAD>`
* `<ITEM>`



## `nxf thread list`

List the board items `thread` is linked to

**Usage:** `nxf thread list <THREAD>`

###### **Arguments:**

* `<THREAD>`



## `nxf label`

Add, remove, or list user labels (free-text tags) on an item. A label is user vocabulary, distinct from the plugin's display type; filter `list`/`next` with `--label`

**Usage:** `nxf label <COMMAND>`

###### **Subcommands:**

* `add` — Attach a label to an item (idempotent). The label is trimmed; a blank label is rejected
* `remove` — Detach a label from an item (observed-remove)
* `list` — List an item's labels (sorted)



## `nxf label add`

Attach a label to an item (idempotent). The label is trimmed; a blank label is rejected

**Usage:** `nxf label add <ID> <LABEL>`

###### **Arguments:**

* `<ID>`
* `<LABEL>`



## `nxf label remove`

Detach a label from an item (observed-remove)

**Usage:** `nxf label remove <ID> <LABEL>`

###### **Arguments:**

* `<ID>`
* `<LABEL>`



## `nxf label list`

List an item's labels (sorted)

**Usage:** `nxf label list <ID>`

###### **Arguments:**

* `<ID>`



## `nxf note`

Add or list notes (the change/worklog stream) on an item

**Usage:** `nxf note <COMMAND>`

###### **Subcommands:**

* `add` — Append a note to an item. Pass `-` as the text to read the note body from STDIN (escaping-free, multi-line)
* `list` — List an item's notes



## `nxf note add`

Append a note to an item. Pass `-` as the text to read the note body from STDIN (escaping-free, multi-line)

**Usage:** `nxf note add <ID> <TEXT>`

###### **Arguments:**

* `<ID>`
* `<TEXT>`



## `nxf note list`

List an item's notes

**Usage:** `nxf note list <ID>`

###### **Arguments:**

* `<ID>`



## `nxf search`

Search items by substring over title, description, design, DoD, and notes. Results are grouped by lane priority (next+in-progress → blocked → deferred → closed), each group in its natural order; archived items are excluded by default

**Usage:** `nxf search [OPTIONS] <QUERY>`

###### **Arguments:**

* `<QUERY>` — Query string

###### **Options:**

* `--status <STATUS>` — Filter by status
* `--type <TY>` — Filter by type, using the active plugin's vocabulary (see `nxf schema`)
* `--now <NOW>` — Reference time (ISO-8601 / RFC3339); defaults to now. Sets the defer boundary used to group deferred matches
* `--include-archived` — Append the archived group (lowest priority) instead of excluding it
* `--archived-only` — Search ONLY the archive; the live lanes are skipped
* `--sort <SORT>` — Flatten the lane grouping into one order by this key (`rank` | `id` | …). Unknown = loud error



## `nxf show`

Show one item with its dependencies and notes

**Usage:** `nxf show <ID>`

###### **Arguments:**

* `<ID>` — Item id



## `nxf schema`

Describe the active plugin's field model — vocabulary, priorities, and per field whether it is required on create / settable on update and how. `--json` is the machine contract; run it to learn what `create`/`update` accept in this workspace (the static `--help` cannot carry the active plugin's vocabulary)

**Usage:** `nxf schema`



## `nxf setup`

Wire host-specific agent integration (idempotent, merge-only). `nxf init` stays tool-agnostic; this opts a project into a host's deterministic delivery

**Usage:** `nxf setup <COMMAND>`

###### **Subcommands:**

* `claude` — Wire Claude Code: (re)assemble the shared agent file (AGENTS.md) and one SessionStart hook per active module (`nxf prime`, `nxm prime`, `nxc prime`) plus the `nxs`/module permission allowlist, merged into `.claude/settings.json` (idempotent, never overwriting). The canonical verb is `nxs setup claude`; this delegates to it (host setup is an umbrella responsibility)



## `nxf setup claude`

Wire Claude Code: (re)assemble the shared agent file (AGENTS.md) and one SessionStart hook per active module (`nxf prime`, `nxm prime`, `nxc prime`) plus the `nxs`/module permission allowlist, merged into `.claude/settings.json` (idempotent, never overwriting). The canonical verb is `nxs setup claude`; this delegates to it (host setup is an umbrella responsibility)

**Usage:** `nxf setup claude`



## `nxf guide`

Print an embedded, offline guide. Omit the topic to list the available ones

**Usage:** `nxf guide [TOPIC]`

###### **Arguments:**

* `<TOPIC>` — Guide topic; omit to list topics



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>

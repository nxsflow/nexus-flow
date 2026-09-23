# Befehle

Eine Tour durch die meistgenutzten `nxf`-Befehle, in der Reihenfolge, in der eine Sitzung sie
gewöhnlich verwendet. Führen Sie `nxf <command> --help` aus für die knappe, generierte Referenz
(Flags, Typen, Voreinstellungen); dieser Leitfaden fügt die Erzählung hinzu. Kombinieren Sie jeden
Befehl mit `--json` für deterministische, agentenfreundliche Ausgabe.

Die Sitzung unten setzt den [Erste-Schritte](nxf-getting-started)-Workspace unter dem
`issue-tracker`-Plugin fort.

## Inspizieren

`show` ist das vollständige menschliche Detail eines Items — Felder, Beschreibung, Abhängigkeiten und
Notizen:

```console
$ nxf show ab12.0002

0002 P1 Write the CLI
=====================

TYPE: feature
STATUS: in progress
PARENT: 0001

DESCRIPTION
-----------

Build the command-line tool

DEFINITION OF DONE
------------------



DESIGN
------

Thin CLI over the core.

NOTES
-----

- 2026-06-23 started on the command layer

This item has the following parents: ab12.0001. URGENT RECOMMENDATION: ALSO READ THESE ITEMS TO GET THE COMPLETE PICTURE!!!
```

`list` gibt jedes Item aus; mit `--json` ist es der kanonische, byte-stabile Schnappschuss
(id-geordnet, nicht gerankt):

```console
$ nxf list --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Cut the first release","design":null,"due":null,"id":"ab12.0001","priority":"1","priority_label":"P1","status":"in_progress","title":"Ship v1","type":"epic","type_label":"epic","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":"dev","belongs_to":"ab12.0001","closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Build the command-line tool","design":"Thin CLI over the core.","due":"2026-12-31","id":"ab12.0002","priority":"1","priority_label":"P1","status":"in_progress","title":"Write the CLI","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":"2026-06-23T00:00:00Z","closing_comment":"approved","completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Approve the final spec","design":null,"due":null,"id":"ab12.0003","priority":"2","priority_label":"P2","status":"closed","title":"Spec sign-off","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

`search` matcht über Titel, Beschreibung und Notizen:

```console
$ nxf search "thin cli" --json
[{"archived":null,"assignee":"dev","belongs_to":"ab12.0001","closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Build the command-line tool","design":"Thin CLI over the core.","due":"2026-12-31","id":"ab12.0002","priority":"1","priority_label":"P1","status":"in_progress","title":"Write the CLI","type":"feature","type_label":"feature"}]
```

## Ein Item bearbeiten

`claim` markiert ein Item als in Bearbeitung und weist es zu:

```console
$ nxf claim ab12.0002 --assignee dev --json
{"archived":null,"assignee":"dev","belongs_to":"ab12.0001","closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Build the command-line tool","design":null,"due":"2026-12-31","id":"ab12.0002","priority":"1","status":"in_progress","title":"Write the CLI","type":"feature"}
```

`update` bearbeitet Felder — hier das `design`:

```console
$ nxf update ab12.0002 --set "design=Thin CLI over the core." --json
{"archived":null,"assignee":"dev","belongs_to":"ab12.0001","closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Build the command-line tool","design":"Thin CLI over the core.","due":"2026-12-31","id":"ab12.0002","priority":"1","status":"in_progress","title":"Write the CLI","type":"feature"}
```

`note add` hängt eine Worklog-Notiz an; `note list` liest sie zurück:

```console
$ nxf note add ab12.0002 "started on the command layer" --json
{"body":"started on the command layer","id":"[..]"}

```

```console
$ nxf note list ab12.0002

- 2026-06-23 started on the command layer


```

`close` schließt ein Item ab und hält einen Schließkommentar fest (das *Warum*):

```console
$ nxf close ab12.0002 --reason done --json
{"archived":null,"assignee":"dev","belongs_to":"ab12.0001","closed_at":"2026-06-23T00:00:00Z","closing_comment":"done","completion_criterion":null,"defer_until":null,"deleted":null,"description":"Build the command-line tool","design":"Thin CLI over the core.","due":"2026-12-31","id":"ab12.0002","priority":"1","status":"closed","title":"Write the CLI","type":"feature"}
```

## Die Arbeit strukturieren

`dep add` / `dep remove` verwalten die *muss-zuerst-fertig*-Kanten, die das Blockieren steuern
(siehe [Kernkonzepte](nxf-core-concepts)):

```console
$ nxf dep add ab12.0002 ab12.0003 --json
{"msg":"ab12.0002 -> ab12.0003","ok":true}

```

```console
$ nxf dep remove ab12.0002 ab12.0003 --json
{"msg":"removed ab12.0002 -> ab12.0003","ok":true}

```

`mention add` / `mention list` / `mention remove` halten Freitext-Referenzen per Short-id fest —
ein Verweis, der nie blockiert:

```console
$ nxf mention add ab12.0002 ab12.0001 --json
{"msg":"ab12.0002 mentions ab12.0001","ok":true}

```

```console
$ nxf mention list ab12.0002 --json
["ab12.0001"]

```

```console
$ nxf mention remove ab12.0002 ab12.0001 --json
{"msg":"removed mention ab12.0002 -> ab12.0001","ok":true}

```

## Einen Agenten bootstrappen

`prime` ist ein einziger Aufruf, der einem Agenten die plugin-bestimmte Zweckbeschreibung von nxf
(`purpose`), die Arbeitsregeln, die gerankte `next`-Empfehlung (Top 7, laufende Arbeit zuerst),
einen leverage-bewussten `blocked`-Schnappschuss, ein `create`-Beispiel mit dem Typ-Vokabular und
der Prioritäts-Spanne des aktiven Plugins sowie die Befehlsreferenz (deren `dep`-Eintrag die
Abhängigkeitsrichtung ausbuchstabiert) übergibt. Der deterministische Einstiegspunkt für eine
automatisierte Sitzung:

```console
$ nxf prime --json
{"blocked":[],"commands":[{"group":"Finding work","items":[{"name":"next","summary":"what to work on (start here): work you can finish now, then started epics with their children, then the backlog"},{"name":"blocked","summary":"list blocked work"},{"name":"deferred","summary":"list deferred items (open, unblocked, future defer date); defer needs a real date — to wait on an event/delivery use a WAIT: chore dependents dep on, not a placeholder date (nxf guide deferring-and-waiting)"},{"name":"show <id>","summary":"item detail with deps and notes"}]},{"group":"Creating & updating","items":[{"name":"create","summary":"create an item — see the `create` section above"},{"name":"update <id> --set k=v","summary":"edit fields"},{"name":"claim <id>","summary":"mark in progress"},{"name":"close <id> --reason","summary":"close with a comment"},{"name":"schema","summary":"introspect this plugin's field model (--json) before create/update"}]},{"group":"Dependencies & references","items":[{"name":"dep add <from> <to>","summary":"<from> depends on <to> (so <to> blocks <from> and must close first)"},{"name":"mention add <from> <to>","summary":"record a free-text short-id reference"}]},{"group":"Notes & search","items":[{"name":"note add <id> <text>","summary":"append a worklog note"},{"name":"search <query>","summary":"search title/description/design/DoD/notes"}]}],"context_recovery":"Run `nxs prime` after a context compaction, /clear, or a new session — hosts auto-call it in Claude Code when a nexus-flow workspace is resolved.","create":{"example":"nxf create --type <bug|chore|decision|epic|feature> --title /".../" --priority <P0|P1|P2|P3|P4>","long_text_hint":"Long text without shell escaping: pipe a field via STDIN (`--description -`), read it from a file (`--description-file <path>`), or pipe the whole item as JSON (`nxf create --json -`).","recommendation":"Always set --priority (named variants, highest first: P0 … P4); an item created without a priority ranks last in `next`."},"next":[{"id":"ab12.0001","parent":null,"priority":"1","status":"in_progress","title":"Ship v1","type":"epic"}],"next_total":1,"purpose":"nexus-flow is a software issue tracker for epics and issues. You record items, the dependencies between them, due/defer dates, and priority; `next` and `blocked` are then derived deterministically from that graph rather than stored, so the work list is always consistent.","recently_closed":[{"archived":false,"closed_at":"2026-06-23T00:00:00Z","closing_notes":"done","id":"ab12.0002","notes_truncated":false,"title":"Write the CLI"},{"archived":false,"closed_at":"2026-06-23T00:00:00Z","closing_notes":"approved","id":"ab12.0003","notes_truncated":false,"title":"Spec sign-off"}],"rules":["Track all work in nexus-flow itself: open an item for every task rather than keeping a separate TODO list or scratch notes — the board is the single source of truth.","Find what to work on next: `nxf next`.","Claim work before starting it: `nxf claim <id>`.","Close with a reason: `nxf close <id> --reason <text>`.","Use `--json` everywhere for deterministic, machine-readable output.","Choose the containment edge deliberately: `parent` is gating — a child rests when its container rests (a deferred, blocked, or closed parent propagates down and hides or masks the child). For a loose association that must NOT gate the child, use `contributes_to` (e.g. cream belongs to the shopping list but only contributes to the birthday plan, so deferring the birthday never hides the cream).","Defer only for a real calendar date — a day before which the item genuinely cannot start (`--defer <date>` on create, or `nxf update <id> --set defer=<date>`); a placeholder date for /"someday, once X ships/" is an anti-pattern that hides the item on a false promise. To wait on an external DELIVERY instead — there are no cross-workspace dependencies — model the delivery as an open WAIT chore in this workspace (title it `WAIT: <what ships>`, e.g. `WAIT: acme-api v2`), have the dependents `nxf dep add <id> <wait-chore>` onto it, and CLOSE the chore — with the delivered version in the reason — to release the whole chain. Each workspace keeps its own anchor; see `nxf guide deferring-and-waiting`.","Correct an item's fields once shortly after creating it (e.g. to fold in a review); after that keep the fields stable and record what you learn while working as append-only notes (`nxf note add <id> <text>`), not field edits. The original title and description are preserved on purpose: paired with the closing comment they form the intent-vs-outcome pair you learn from, so when an item no longer fits, open a new one and close the old with a reason instead of rewriting it past recognition.","When you cite a task's short-id in free text (body or note), also record the reference: `nxf mention add <this-item> <cited-id>`. It keeps the citation resolvable if ids are remapped on sync, and never blocks (it is not a dependency)."],"session_close":["Capture unfinished work as a note so the next session has the context: `nxf note add <id> <text>`.","Close finished items with the reason they're done: `nxf close <id> --reason <text>`.","If the project is under version control, commit and push your code changes."],"workflows":[{"name":"Starting work","steps":["nxf next","nxf show <id>","nxf claim <id>"]},{"name":"Completing work","steps":["nxf close <id> --reason /".../"","nxf next   # pick up the next unblocked item"]},{"name":"Creating dependent work","steps":["nxf create --type <type> --title /".../" --priority <P…>","nxf create --type <type> --title /".../" --priority <P…>","nxf dep add <child> <prereq>   # child depends on prereq; prereq must close first"]}]}
```

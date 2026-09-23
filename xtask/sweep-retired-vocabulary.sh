#!/usr/bin/env bash
# Find files that still name a RETIRED verb or path and that a given change never opened.
#
#   bash xtask/sweep-retired-vocabulary.sh [<base-ref>]     # default base: origin/main
#
# ---------------------------------------------------------------------------------------------
# WHAT THIS IS, AND WHAT IT IS NOT
#
# It is a REVIEW AID, not a gate. It exits 0 whether it finds anything or not, it is not wired into
# CI, and every line it prints is for a human to judge. Do not read a clean run as "the text is
# correct" — read it as "no file outside this change still says these words".
#
# It exists because of a failure mode this repo keeps meeting (nexus-flow-6j6v.dvyq, fix rounds 1
# and 2): **a removal leaves no identifier to grep for.** After you delete a verb, `cargo check` and
# every gate stay green while the prose that names it rots in place — and the prose is the surface
# an agent actually reads. So the sweep has to run over the vocabulary of what was DELETED, which
# only a human can supply, and that list is the whole content of this file.
#
# The specific question it answers is narrower than "is any text stale", and deliberately so: *did
# this change open every file that names the thing it removed?* Two shapes of miss caused four
# Important findings on 6j6v.dvyq, and this catches the first of them:
#
#   shape 1  files the change never opened at all              <- THIS SCRIPT
#   shape 2  lines ADJACENT to lines the change edited         <- `git diff -U8 | grep '^ '`, below
#   shape 3  a doc whose own supersession note is scoped too   <- by hand; no mechanism known
#            narrowly to cover the whole file
#
# For shape 2, run:
#   git diff -U8 <base>..HEAD | grep -E '^[ ]' | grep -nE "$(IFS='|'; echo "${VOCABULARY[*]}")"
#
# TURNING THIS INTO A REAL GATE is a separate job and is NOT done here. A fail-closed version needs
# a rule for "this occurrence is deliberately historical" (a supersession note, a changelog
# fragment, a frozen plan) — otherwise it reddens on every honest record of a removal, of which
# this branch wrote dozens. The obvious rule (accept an occurrence whose neighbourhood has a
# removal
# marker) is loose enough to produce false all-clears, which is worse than no gate. Filed rather
# than guessed at.
# ---------------------------------------------------------------------------------------------

set -uo pipefail

BASE="${1:-origin/main}"

# ---- THE VOCABULARY: what was removed, spelled as a reader would meet it ----------------------
#
# Verbs AND the path, because the path is what produced two of the four Important findings on
# 6j6v.dvyq — a sweep that carried only the verbs reported "clean" while seventeen sites still
# named the old folder. Add a line here whenever you remove something; that is what it costs.
#
# Terms are EXTENDED regular expressions (`grep -E`), which is also what the shape-2 alternation
# above builds out of them. Most are plain literals; where a term carries metacharacters it is
# deliberate — a path anchor, a `(a|b)` alternation, an escaped dot, a trailing "not an identifier
# character" guard — and the reason is written down: at the term itself, or in the block above the
# pair it belongs to (several go in twos, `send`/`reply`, `hop`/`origin`, `prime`/`transcript`, and
# one comment covers both). The single term whose metacharacter is NOT deliberate says so on its
# own line.
#
# THIS SENTENCE USED TO CARRY A COUNT ("all but one … the exception is the folder path") and the
# count was wrong: it was already wrong before 6j6v.sgfr, and that change added two more terms
# without noticing. Replacing it with a LIST would have rotted the same way — measured while
# fixing it, there are twelve such terms, not the four a first draft of this paragraph named. So
# it states the rule instead, and the rule is enforceable where it matters: at each term.
VOCABULARY=(
  # The declaration folder before 6j6v.dvyq step 4; now `.nxs-personas/`. PATH-ANCHORED rather than
  # the bare substring `roles/`, which also matched "roles" as an ordinary word in front of a slash
  # — `nxf-* roles/buckets/SSM` (AWS IAM, docs/specs/release-management.md), and eleven more
  # `roles/channels`, `roles/workflow` prose hits inside files this branch happened to open. The
  # folder is never written bare in prose: it is quoted (`roles/`), or it carries its parent
  # (`<project-root>/roles/`), so requiring a path-ish or quoting character in front of it — or the
  # start of a line, for a path that opens one — drops all twelve false positives and keeps every
  # one of the 36 real hits.
  '(^|[`/"'"'"'])roles/'
  'nxc agents'        # 6j6v.dvyq §3 — a team is declared, not registered
  'agents register'
  'agents search'
  'agents list'
  'workflow tickets'  # 6j6v.dvyq §3 — owner: "ueberfluessig", no replacement
  'tickets add'
  'set_definitions'   # 6j6v.dvyq step 6 — the injection path
  'DefinitionSource'
  'from_roles_dir'    # renamed to `Definitions::from_dir` in the same step
  'roles_dir'         # `Workspace::roles_dir()`; now `personas_dir`/`declaration_dir`
  'threads expect'    # 6j6v.cg8g/dvyq §3 — the seam write `facade::set_expects` stays
  'send --model'      # 6j6v.e9qj/dvyq §3 — the flag went, the request field stayed
  'send --thread'     # 6j6v.dvyq §3 — `send --to` mints it, `reply --thread` posts into it
  # ---- the RAW CHANNELS, 6j6v.dvyq §3 block (a) ------------------------------------------------
  # A channel is a declaration or it is nothing. Everything below either minted a channel nobody
  # declared, or addressed one.
  'nxc ask'           # collapses into `send --to <channel>`; the declaration carries `expects`
  'ask --expect'      # a per-call override of a question the declaration answers
  'ask --deadline'    # MOVED, not removed: the flag is `send --deadline` now
  'nxc channels'      # the whole verb group: list/public/create/dm/join/leave
  'channels create'
  'channels dm'
  'channels join'
  'channels leave'
  'channels list'
  'channels public'   # the ONE with no CLI successor — discovery left the agent surface
  'Engine::ask'       # the flat handle method; `facade::ask` (the write) stays
  'Engine::channel_open'
  'TargetKind::Conversation'  # the resolution that named a raw channel as a target
  # ---- THE SECOND AND THIRD WAY TO SUMMON, 6j6v.dvyq §3 block (b) ------------------------------
  # `send --to` is the one entrance; §5's closed list takes both seam methods with it.
  'send --role'       # collapses into `send --to <persona>` — same body, plus a thread to answer in
  'send --session'    # a LOSS, not a collapse: no successor. The residual is nxf 6j6v.xr3z
  # Qualified, NOT the bare symbol — WHEN THIS WAS WRITTEN. `orchestration::role_trigger` and
  # `role_trigger_with` stayed then and are gone now (nxf 6j6v.ntp9 renamed the family to
  # `coordinator_commission`), so the bare symbol is listed in that item's own block below and this
  # pair is left qualified only because these two seam methods are what 6j6v.dvyq removed.
  # `orchestration::role_resume` does still stay.
  'Engine::role_trigger'
  'Engine::role_resume'
  # ---- THE PERSONA COORDINATOR, 6j6v.ntp9 -----------------------------------------------------
  # A commission for one persona now goes through ONE named place, `coordinator_commission`, the
  # twin of `supervisor_*` for channels. What went is the four near-identical names it replaced and
  # the two pieces of the coordinator's job that used to sit above it on the surface.
  #
  # BARE here, unlike the pair above: nothing named `role_trigger` survives, so every hit is either
  # an honest historical record (a changelog fragment, a supersession note) or prose to correct.
  # `trigger_role` — the spawn primitive under the coordinator — is a DIFFERENT symbol and stays;
  # note the word order before reading a hit as stale.
  'role_trigger'          # `role_trigger`/`role_trigger_with`/`role_trigger_in`
  'RoleTriggerRequest'    # the request type; now `Commission`
  'MAX_REPLY_REMINDERS'   # the sidecar constant; the bound is `TurnTerms` in the engine now
  # ---- THE WHOLE RUN ENGINE, 6j6v.dvyq §3 block (c)/(d) ---------------------------------------
  # A channel IS the flow. `send --to <channel>` starts one, `reply --thread` moves it on, and
  # `nxc status` is where it stands. What went is the second mechanism beside all three, together
  # with the record it wrote — so these terms are the group, its verbs, its seam and its schema.
  'nxc workflow'      # the whole command group, including the two that had no successor
  'workflow start'
  'workflow step'     # covers `workflow step done` and its `--outcome` flag
  'workflow status'
  'workflow list'
  'workflow liveness'
  'workflow tick'     # MOVED, not removed: the verb is `nxc tick --thread`, hidden from `--help`
  'workflow_runs'     # the record, and the five side tables named after it
  'workflow_reducer'
  'workflow_events'   # acceptance point 4: the stream, its cursor and its subscription
  'subscribe_workflow'
  'WorkflowDecl'      # `.nxs-personas/workflow.yaml` and `workflows/*.yaml` are read by nothing now
  # PATH-ANCHORED to the declaration folder, for the same reason `roles/` above is: the bare
  # `workflows/` matches `.github/workflows/`, `infra/`'s CI trees and `npm/`'s constants — eleven
  # files that have nothing to do with this removal, which is exactly how a sweep trains a reader
  # to skim past it.
  '(personas|roles)/workflows/'
  'WORKFLOW_RUN_ADDR_PREFIX'
  'fire_role_step'    # the two step-firing primitives and the advance between them
  'fire_channel_step'
  'advance_and_fire'
  'step_liveness'     # the dead-man's watch, a NAMED LOSS with no successor
  'Requester::'       # the two-variant requester; a completion has a return address now
  'parse_outcome_token'
  'advance_failed'    # the two `FailedConsequence` classes that were about a run
  # CamelCase, not the snake_case constructor: `no_verdict` also matches the ordinary English of
  # `xtask`'s own `check_asks_for_no_verdict_when_the_pr_touches_no_consumed_surface`, which has
  # nothing to do with this vocabulary.
  'NoVerdict'
  # ---- THE INFRASTRUCTURE TREE AND ITS FIVE WORKFLOWS, 6j6v.sgfr ------------------------------
  # The delivery stack, the updater Lambda, the CI trust gates and the E2E suite left this repo
  # for `nxsflow-landing-page`; `infra/` and the workflows that deployed it are deleted. This is
  # the vocabulary-shaped half of that removal: a WORKFLOW NAME is the purest case the header
  # describes — delete the file and there is no identifier left anywhere for `cargo check` to
  # miss, only prose in four OTHER workflows that still tells a reader to go look at it.
  #
  # PATH-ANCHORED for the same reason `roles/` above is, and it matters more here: the bare
  # substring `infra` is inside "infrastructure", which this repo's own README and specs use as
  # ordinary English a dozen times. Requiring a path-ish or quoting character in front (or the
  # start of a line) keeps `\`infra/\``, `nexus-flow/infra/` and `"infra/lambda/…"` and drops
  # every prose hit.
  '(^|[`/"'"'"'])infra/'
  'infra-ci'          # the five deleted workflows. `infra-staging` also covers
  'infra-staging'     # `infra-staging-release`; listed once so one file does not print twice.
  'infra-prod'
  'staging-gate'      # the label-lease gate AND its pure logic, `infra/ci/staging-gate.ts`
  'deploy-staging-'   # covers both leases, `deploy-staging-infra` and `deploy-staging-site`
  'nxs-infra-deploy'  # the OIDC role the deploy workflows assumed — in an account that had not
                      # owned it since the delivery move, which is what made them permanently red
  '/nxs/infra/'       # the SSM namespace: the deploy marker and the staging lease
  'NxsDelivery'       # the two CDK stacks
  'NxsGitHubDeploy'
  # The cross-account pin and the E2E wiring: repo secrets/vars that address nothing after the
  # teardown. Anchored on the distinguishing tail rather than the `NXS_` prefix, which
  # `NXS_STAGING_ACCOUNT_ID` and `NXS_PROD_ACCOUNT_ID` share — and those two STAY (release.yml,
  # promote.yml and publish-content.yml build role and bucket names out of them at run time).
  'LANDING_DISTRIBUTION_ARN'
  'NXS_E2E_'
  'NXS_STAGING_PUBLIC_URL'
  'NXS_PROD_PUBLIC_URL'
  # The updater's pure decision module. It was read by `cargo xtask platforms check` and
  # `cargo xtask channels check` until this removal — the one consumer of `infra/` that lived
  # OUTSIDE it, and the reason a naive teardown would have reddened ci.yml on every pull request.
  # The dot is ESCAPED so the term cannot also match `decideXts`-shaped identifiers; it costs
  # nothing and keeps the term saying exactly the filename it means.
  'decide\.ts'
  # ---- THE WRITING SEAM CUT TO TWO VERBS, 6j6v.ckeq -------------------------------------------
  # `send_to` and `reply_thread` are the only way to communicate, and each takes three statements.
  # Both removed methods had been off the CLI since 6j6v.dvyq §3 and lived only on the app seam, so
  # the vocabulary here is mostly SEAM names and FLAG spellings rather than verbs.
  #
  # ANCHORED so they do not match their own successors: a bare `Engine::send` matches
  # `Engine::send_to` and a bare `Engine::reply` matches `Engine::reply_thread`, which are exactly
  # the two things that STAYED — a sweep that flags every honest mention of the survivor trains a
  # reader to skim past the one that went.
  'Engine::send($|[^_a-zA-Z])'
  'Engine::reply($|[^_a-zA-Z])'
  # NOT `SendRequest`, deliberately: `facade::SendRequest` STAYS and is what every post in the crate
  # is built out of, so the term would flag the survivor at each of its honest call sites — the same
  # trap `role_trigger` is anchored against above. `Engine::send` is the thing that went, and the
  # anchored form above catches it.
  'send --kind'        # the three caller options that fell together, on both verbs
  'send --priority'
  'send --disposition'
  'send --deadline'    # MOVED here from `ask` by dvyq §3 and REMOVED by ckeq; `timeout:` declares it
  'reply --kind'
  'reply --priority'
  'reply --disposition'
  'reply --ref'        # a reply INHERITS its subject; the refs obligation is `send --to`'s
  'kind question'      # the one `--kind` value with a rule behind it — a NAMED LOSS (6j6v.1xw1)
  'escalate_or_kind'   # `--escalate` is a boolean now; `surface::escalation_kind` is the mapping
  # The FIELD, not the flag: `nxc reply --if-unanswered` STAYS (the sidecar teardown is its one
  # caller). What left is the request field an app filled in, so the term is spelled as the field.
  'ReplyThreadRequest::if_unanswered'
  # The only term here whose metacharacter is INCIDENTAL rather than chosen: the `.` is Rust field
  # access, and as an ERE it also matches any single character. Harmless — the literal it is meant
  # to find matches it — and left as written rather than escaped, because escaping it would suggest
  # a regex intention that was never there. Noted so the paragraph above stays true: every
  # metacharacter in this list is either deliberate or says, here, that it is not.
  'if_unanswered: req.if_unanswered'
  # ---- WHAT A CALLER MAY NO LONGER SAY, 6j6v.07me ---------------------------------------------
  # `Caller` carried five ambient values and carries `{ session, actor, now }`. The three that left
  # have no CLI counterpart to retire — `NXC_ORIGIN`/`NXC_HOP`/`NXC_NOW` are all still read by
  # `cli.rs` — so this vocabulary is seam FIELD spellings, the prose that counted them, and the one
  # constant the owner's decision of 2026-08-21 deleted outright.
  #
  # QUALIFIED, never the bare field names: `origin`, `actor`, `hop` and `now` are all still live
  # `Ctx` fields (the CLI fills every one of them), and `session`/`actor`/`now` are still live
  # `Caller` fields. A sweep on the bare words would flag several hundred honest mentions of the
  # survivors, which is precisely how a reader is trained to skim past the real hit.
  'Caller(\.|::)hop'
  'Caller(\.|::)origin'
  'origin, actor, session, hop'   # the five-field shape, as prose and code both spelled it
  'five ambient values'           # and the prose that counted them
  # The constant that briefly WAS the answer. The first build of 07me made `origin` the literal
  # `"local"` behind `workspace::LOCAL_ORIGIN`; the owner overruled that on 2026-08-21 in favour of
  # the replica prefix, and the constant was deleted with no successor — `workspace::origin_of(&ws)`
  # is a function over a workspace, not a name a second file can restate. This is exactly the shape
  # the sweep exists for: nothing fails to compile when a doc keeps linking a deleted `pub const`,
  # and an intra-doc link to one is a dangling link `cargo doc` alone does not redden.
  'LOCAL_ORIGIN'
  # NOT on this list, deliberately: `NXC_ORIGIN` and the bare string `"local"`. The variable STAYS
  # as the explicit override, and `local` is an ordinary English word (and a real, correct pin in
  # the 21 CLI-driven chat suites that set `NXC_ORIGIN=local` so their handle literals keep meaning
  # something). What changed is a DEFAULT, which has no spelling of its own; `parity.rs`'s
  # `both_adapters_resolve_one_origin_for_one_workspace` is the mechanism that holds it.
  # NOT on this list, deliberately: `Caller::now` and `Caller::actor`. Both fields still EXIST —
  # what changed is that one became optional and the other conditional, and neither has a spelling
  # of its own that a grep could tell from the survivor. The rule they carry is held by a test
  # instead (`caller_seam.rs`), which is the stronger of the two mechanisms anyway.
  # ---- THE READING SEAM CUT TO SEVEN VERBS, 6j6v.yr59 -----------------------------------------
  # `status`, `directory`, `prime_as`, `thread`, `transcript_page`, `subscribe` — plus `search`,
  # which the item never covered and which the OWNER RULED STAYS on 2026-08-21 (app-foundations
  # consumes it). Ten reads left `Engine`; their `facade::` bodies mostly STAYED (the CLI verbs that
  # reach them are still verbs), so this vocabulary is SEAM-QUALIFIED throughout — a bare `inbox`,
  # `messages` or `channels` would flag several hundred honest mentions of the surviving
  # compute-layer read and of the CLI verb, which is exactly how a sweep trains a reader to skim
  # past the real hit.
  'Engine::messages'          # the channel-keyed reader; `Engine::thread` is the one that stayed
  'Engine::channels'          # named LOSS on the unread half; `Engine::directory` on the other
  'Engine::public_channels'   # absorbed by `Engine::directory`, front doors and all
  'Engine::inbox'             # the catch-up `Engine::prime_as` already carried
  'Engine::opener_wake'       # by its own doc the same derivation `prime` renders
  'Engine::threads'           # folded into `Engine::status`; NOT `Engine::thread`, which stayed
  'Engine::thread_board'      # quorum half -> `status`, message half -> `thread`
  'Engine::thread_quorums'    # a PARAMETER now: `StatusScope::Threads`
  # ANCHORED against their own successors, the shape `Engine::send`/`Engine::reply` needed above:
  # a bare `Engine::prime` matches `Engine::prime_as` and a bare `Engine::transcript` matches
  # `Engine::transcript_page` — the two verbs that STAYED.
  # KNOWN COLLISION, and it is written down rather than left for a reader to rediscover:
  # `nexus_memory::engine::Engine::prime` is a LIVE verb with the same spelling (memory's own
  # session bootstrap, untouched by yr59), so this term reports three memory files on every run.
  # They are expected, not defects. The term is kept anyway: chat's prose says `Engine::prime` too,
  # and three labelled false positives are cheaper than no sweep over the real one.
  'Engine::prime($|[^_a-zA-Z])'
  'Engine::transcript($|[^_a-zA-Z])'
  # The scope variant that was REPLACED rather than joined: one id became a set, so a reader who
  # meets the old spelling is reading a false map. The paren is what tells it from `Threads(`.
  'StatusScope::Thread\('
  # NOT on this list, deliberately: `open_with_poll_interval`. chat's third constructor went (it is
  # `EngineConfig::poll_interval` now), but `nexus_flow_facade::engine::Engine`, `nexus_memory` and
  # `nxs_foundation` all still HAVE one under that exact name, and the term cannot tell them apart.
  # What holds chat's removal is `tests/seam_disposition.rs`'s `Leaves` row, proven load-bearing.
  # NOT on this list, deliberately: `Engine::search`. It STOOD HERE while the build had it removed,
  # and the owner's correction of 2026-08-21 put the method back — app-foundations consumes it, so
  # the identifier is live on both halves of the seam and is not retired vocabulary at all. A term
  # for it would flag every honest mention of a method that exists, which is the exact failure mode
  # this file's anchoring rules exist to avoid. The decision is `tests/seam_disposition.rs`'s
  # `search` row; the count is `tests/read_surface.rs`, which holds SEVEN.
  # NOT on this list, deliberately: `ThreadListEntry`, `ThreadBoardView`, `OpenerWake` and
  # `ThreadQuorum`. Every one of them is still a live type `facade::threads`/`thread_board`/
  # `opener_wake`/`thread_quorums` returns to `nxc`; what went is the handle method, not the record.
  # NOT on this list, deliberately: the bare word `workflow`. It is an ordinary English noun this
  # repo uses for CI workflows, for nxf's own `guide` workflows and in every honest historical
  # note about what was removed — a term that matches all of those trains a reader to skim.
  # ---- THE DOCS LAYER SPLIT INTO THREE BLOCKS, 6j6v.9e3r --------------------------------------
  # Almost nothing was REMOVED here — `nxs guide`, `nxm guide` and `nxc guide` are additions — so
  # this section is short on purpose. What went is the assumption that there is ONE guide list.
  'TOPIC_ORDER'       # content/lib/docs.mjs's flat list; now `BUILDING_BLOCKS[].order`
  # NOT on this list, deliberately: `crates/cli/docs/guide`. The path still EXISTS and is correct
  # wherever it names flow's own tree — what changed is that it is no longer the ONLY one, and a
  # term cannot tell "flow's guides live here" (true) from "the suite's guides live here" (now
  # false). Both readings appear in this repo's prose, so the sweep was done BY HAND for it: the
  # three prose sites (`NEXUS_MEMORY.md`, `docs/specs/nxsflow-com-migration.md`, `content/README.md`)
  # and the two workflow path filters were opened and corrected in the same change.
  # ---- THE CHANNEL-LEVEL SESSION POLICY, 6j6v.fepb -------------------------------------------
  # `ChannelDecl.member_session: fresh | resume` and its `MemberSession` type. It named a policy it
  # never steered — the fan-out mints a fresh session either way — and its only two consumers were
  # a preflight that rejected `resume` by name and an `unreachable!()` behind it. The two levels
  # (6j6v.pf6j) answer its question by construction: a member is addressed as a PERSONA on its own
  # thread, and a reply into that thread resumes the session through the return address.
  #
  # BOTH spellings, because neither finds the other: the YAML key and the prose say
  # `member_session`, the Rust type says `MemberSession`, and a reader meets each in a different
  # file. Both are unambiguous — nothing else in this repo is spelled either way — so no anchoring
  # is needed, unlike the seam terms above.
  'member_session'
  'MemberSession'
  # NOT on this list, deliberately: `RoleDecl.session` / `SessionPolicy`. The persona-level twin
  # asks the same question and is equally unread, but it STAYS (the decision is recorded on
  # `SessionPolicy` itself): it refuses nothing and steers nothing, so it is documented intent in
  # the guide's "declared but not yet read" list rather than retired vocabulary. A term for it
  # would flag every honest mention of a field that exists — and `session` is besides one of the
  # most ordinary words in this repo.
  # ---- ONE LOOKUP FOR A CHANNEL'S DECLARED POLICY, 6j6v.v39s ---------------------------------
  # `channel::declared_visibility` resolved a declared channel's `visibility` for the two adapters
  # that read a thread (`cli.rs`'s `threads show`, `Engine::thread`). The READ GATE now has to read
  # the same declaration's `members:` — for a declared channel the file IS the membership — so the
  # two facts travel together as `ChannelPolicy` out of ONE lookup, `channel::declared_policy`, and
  # the old function is gone. `Visibility` itself STAYS and is a field on the new type, so only the
  # function name is retired.
  'declared_visibility'
  # NOT on this list, deliberately: `Engine::thread`'s own `declared_visibility_of` helper, which
  # was RENAMED (`declared_policy_of`) rather than removed — the substring above already catches any
  # prose that still spells the old one, and a second term for the same letters would only
  # double-report it.
  # ---- THE SECOND CLOCK, 6j6v.8see -----------------------------------------------------------
  # The owner's decision of 2026-08-25 was one word: ONE. So the per-deadline launchd backend went
  # — the type, the `NXC_TIMER` value, the label prefix its agents carried and the log they wrote —
  # and the sync daemon's own label was renamed on its way to becoming the service.
  #
  # This is the purest case the header describes twice over. A launchd LABEL and a PLIST FILE NAME
  # leave no identifier behind at all: delete the module and `cargo check` is green while a guide
  # still tells a reader to look in `~/Library/LaunchAgents` for something that is never written
  # there again. And the reader in question is often an agent, following a false map.
  #
  # NOT on this list, deliberately: the bare word `launchd`. It is MORE live than before — the one
  # background agent is a launchd agent, `crates/service/src/launchd.rs` is its home, and the guides
  # explain it by name. A term on it would flag the survivor everywhere.
  'LaunchdTimer'
  'NXC_TIMER=launchd'
  'TimerConfig::Launchd'
  'timer/launchd.rs'      # the deleted module, as prose cites a path
  'com.nxsflow.nxc.tick'  # the per-deadline label prefix — nothing writes one now
  'com.nxsflow.nxs.sync'  # the service's label before it was named for the product
  'nxc-tick.log'          # the per-deadline job's log; its output is the service's log now
  'sync-daemon.log'       # renamed with the label, same reason
  # NOT on this list, deliberately: `sync-daemon.json` and `sync-daemon.lock`. Both files KEPT
  # their names through 6j6v.8see and are written under them today — renaming them would have
  # stranded the heartbeat and the lock of a service already installed on a machine, which is a live
  # upgrade hazard for a cosmetic gain. `ServiceHome` says so at the constants.
  # NOT on this list, deliberately: the bare docs slugs (`getting-started`, `commands`, …). They
  # are still the CLI's own topic names — `nxf guide getting-started` is unchanged — and only the
  # PUBLISHED slug gained its `nxf-` prefix. A term on the bare names would flag every correct CLI
  # mention. What holds the rename is `checkCrossLinks` in `content/lib/docs.mjs`, which resolves
  # every link target exactly as the landing does and fails the build on one that no longer lands.
  # ---- WHAT A PERSONA IS TOLD, 6j6v.4mmk / 6j6v.k8zq ------------------------------------------
  # Three copies of the same `nxc` guidance became one, and the session-start block stopped
  # dumping the inbox. Both are the shape this file's header describes: the identifiers vanish,
  # `cargo check` stays green, and the prose that named them is a false map of a text a session
  # reads every time.
  'nxc_usage_block'   # the four hand-written verbs a spawned persona used to read; the composed
                      # `nxs prime --persona` block is the one source now
  'PRIME_CAUGHT_UP'   # the "caught up — no unread" line; there is no unread section to be empty
  # ANCHORED to the heading, and it stays anchored even though the fields it protected are gone
  # too now (6j6v.4d2z, whose own block below carries them): `unread` is an ordinary English word
  # this repo uses in every honest historical note about the removal. What THIS term is about is the
  # rendered SECTION, which only ever appeared as this heading.
  '## Unread'
  # The three-argument form. `compose_system_prompt` STAYS and is the one composer, so the bare
  # symbol would flag every honest caller; what a stale doc gets wrong is the argument list, and
  # the fourth argument is the whole of 6j6v.k8zq at that seam.
  'compose_system_prompt\(role, project_claude_md, reply_thread\)'
  # `render_unread_blocks` was NOT on this list while it was public, tested and merely uncalled by
  # `prime`. It went with the fields it rendered (6j6v.4d2z), so it IS retired vocabulary now and is
  # listed in that item's own block below rather than here — it belongs to the removal that took it,
  # not to the one that stopped calling it.
  # ---- THE OTHER HALF OF THE SESSION START, 6j6v.1gm9 -----------------------------------------
  # The requester wake stopped being rendered and the agent's two PULL verbs left the binary. Same
  # shape as the block above and the same reason it is the shape this file exists for: the renderer
  # and the two `clap` variants vanish, `cargo check` is green, and what rots is a guide telling a
  # reader to type a verb that is not there — an agent-read guide, following a false map.
  'render_opener_wake'    # the block renderer, with `render_completed_thread`/`render_stale_thread`
                          # under it. BARE: nothing survives under that name, so every hit is either
                          # a supersession note or prose to correct.
  '## Threads you opened' # ANCHORED to the heading, exactly as `## Unread` above is: the DERIVATION
                          # stays (`facade::opener_wake`, `PrimeReport::wake`, `prime --json`'s
                          # `threads_you_opened`, all live and all correct to mention), and the
                          # heading is the only spelling the removed RENDERING ever had.
  'nxc inbox'             # the two verbs, spelled as a reader types them. QUALIFIED with `nxc `,
  'nxc read($|[^a-z])'    # never bare: `inbox` and `read` are two of the most ordinary words in
                          # this repo's prose. (It said "`facade::inbox` and `facade::mark_read`
                          # both stay and are named honestly all over the crate" until 6j6v.4d2z
                          # removed them; the terms for those two are in its block below, and the
                          # anchoring here is still needed for the English words.) The trailing
                          # guard on `read` is what keeps it off `nxc reads …` — and it no longer
                          # has to keep it off `nxc read_cursor`-shaped text, since the block below
                          # sweeps `read_cursor` deliberately.
  # `read_cursor`, `read_cursors` and `mark_read` were NOT on this list under 6j6v.1gm9, which
  # removed a RENDERING and two CLI entrances and left every one of them live. 6j6v.4d2z removed
  # them, and they are listed in its own block below.
  #
  # STILL NOT on this list, deliberately: `opener_wake`, `OpenerWake`, `CompletedThread`,
  # `StaleThread`. All four are live and correct to mention — the derivation stayed through both
  # tickets; what changed under it (6j6v.2hx9) is which watermark it reads, which has no spelling of
  # its own that a grep could tell from the survivor.
  # ---- THE UNREAD APPARATUS, 6j6v.4d2z ---------------------------------------------------------
  # The read cursor, the ack that moved it, the set it gated and every count derived from it. This
  # is the shape the header describes at its purest and the one the item's own history proves: the
  # 2026-08-25 measurement that made the removal look free named ONE consumer of the cursor and
  # missed a second, because a removal leaves no identifier to grep for and a named getter beside a
  # table reads like a helper rather than a consumer. The gate that caught it (`read_surface.rs`)
  # and the replacement that cleared it (6j6v.2hx9) are both recorded; what is left for a SWEEP is
  # the prose, and there is a lot of it — two specs, four guide pages in two languages, and the
  # engine's own supersession notes.
  #
  # BARE, unlike the `Engine::`-qualified terms of the yr59 block above: nothing survives under any
  # of these names, on either half of the seam, so every hit is either an honest historical record
  # (a changelog fragment, a supersession note, one of the two spec sections that carries the
  # removal) or prose to correct.
  'read_cursor'           # covers `read_cursors`, `fold_read_cursor`, `advance_read_cursor`,
                          # `read_cursor_seen` and the `read_cursor` op kind in one term
  'mark_read'             # `Engine::mark_read`, `facade::mark_read`, and `markRead` is not this
                          # repo's spelling — the two apps carry their own
  'ReadReceipt'
  'render_unread_blocks'  # public and tested until this removal; see the 6j6v.4mmk block above
  'InboxEntry'            # the five-field projection `in_turn`/`next_session` carried
  'unread_next_session'   # the `ChannelView` field. Its twin `unread` is NOT listed: it is a bare
                          # English word here, and this term plus `ChannelView`'s own row in
                          # `tests/seam_reads.rs` is what holds the pair.
  # ANCHORED as JSON keys, not as bare words: `in_turn` and `next_session` are still live
  # `Disposition` values on every message, on the wire and in the model — a bare term would flag
  # several hundred honest mentions of a field that exists. What went is the two `prime --json`
  # KEYS, and a key is only ever written with its quotes and colon.
  '"in_turn":'
  '"next_session":'
  # NOT on this list, deliberately: the SPACED form `read cursor`. It is ordinary English, and this
  # removal wrote dozens of honest historical sentences containing it ("the gate was C's channel read
  # cursor until nxf 6j6v.2hx9"); a term for it would flag every one of them and train a reader to
  # skim past the real hit. WHAT IT COST, recorded because the sweep did miss something and the miss
  # is the useful part: `content/lib/docs.mjs`'s copy of chat's guide summaries said "and the read
  # cursor", spaced, so the underscored term above could not see it. It was caught by the gate that
  # exists for exactly that pair — `content-ci`'s "every English guide summary in the feed is the
  # string `nxs guide --json` prints" (6j6v.5cxr) — which is the right division of labour: a gate
  # holds a two-copy invariant, a sweep looks for prose nothing holds.
  # NOT on this list, deliberately: `Disposition`, `in_turn`, `next_session`, `count`. The first
  # three are live (see above); `count` is one of the most ordinary words in this repo AND a live
  # method on several other types. What holds the `--json` key's removal is `prime_golden.rs`'s
  # byte-exact golden, which is stronger than a grep anyway.
  # NOT on this list, deliberately: `ChatStore::inbox` / `facade::inbox` / `Engine::inbox`. The
  # last is already in the yr59 block above; the other two are covered by `nxc inbox` and by that
  # same block's reasoning about bare `inbox`, which is an ordinary word this repo uses honestly.
  # ---- `nxf prime` RESHAPED FOR THE SESSION-START BUDGET, nxf xe2z task 2 -----------------------
  # The HUMAN view (not `--json`) of `nxf prime` shed six whole sections and most of its Core
  # Rules bullets to clear the host's 10.240 B SessionStart hook ceiling — the fixed prose alone
  # had been 5.570 B of a ~9.719 B block. `PrimeReport`'s fields (`commands`, `workflows`,
  # `session_close`, `sync`, `context_recovery`, `create.recommendation`,
  # `create.long_text_hint`, and all nine `rules`) are UNTOUCHED and still render in full under
  # `prime --json` — the stable record the MCP server and embedding apps read — so what is
  # "retired" here is specifically the RENDERED MARKDOWN, not the underlying data or any CLI verb.
  # An agent-facing doc that still describes what `nxf prime`'s plain output shows is the false
  # map this sweep exists to catch; one that describes `prime --json`'s fields is still correct.
  '## Essential Commands'  # heading-anchored: the JSON `commands` array (and `nxf --help`'s own
                            # subcommand list) both still name every command it used to group —
                            # only this rendered section is gone, so a bare `Essential Commands`
                            # or `commands` term would flag the survivors.
  '## Common Workflows'    # heading-anchored, same reason: `read::common_workflows` and `--json`'s
                            # `workflows` array both stay; a bare `Common Workflows` would flag them.
  '## Session close'       # heading-anchored: `report.session_close` and `--json`'s `session_close`
                            # key both stay; `session_close`/`session close` alone are not listed
                            # because the snake_case JSON key and ordinary lifecycle prose elsewhere
                            # both use those exact words honestly.
  '## Core Rules'          # heading-anchored: `report.rules`/`--json`'s `rules` array keep all nine
                            # entries; only the heading (and 8 of the 9 bullets under it) left the
                            # human view. `Core Rules` bare would flag docs describing --json.
  '## IDs'                 # heading-anchored: the paragraph's CONTENT (prefix + suffix/full-id
                            # convention) survives, folded under the title with no heading of its
                            # own — only this specific heading text is gone.
  '^## Create$'            # LINE-anchored, not just heading-anchored: a bare `## Create` (or even
                            # `## Create`-as-substring) matches
                            # `crates/cli/docs/guide/en/getting-started.md`'s LIVE, unrelated
                            # `## Create your first items` heading. `create.example` and
                            # `create.recommendation`/`create.long_text_hint` all stay in `--json`;
                            # `create_example` still feeds `## How work moves` too (a DIFFERENT
                            # heading) — only the standalone `## Create` heading text is gone.
  'md_code_placeholders'   # the Markdown-code-span helper, deleted outright with its one call site
                            # (Essential Commands' summaries) — nothing survives under this name.
  'is a gh-ism'            # the `--jq`-does-not-exist-here aside's distinctive close; `--jq` itself
                            # is not listed bare (it never existed as a flag, so nothing to collide
                            # with, but the phrase is more exact about which sentence left).
  'create "..." -q'        # the `-q` id-capture example line, spelled exactly as prime showed it —
                            # `-q` bare is not listed: it is `create`'s OWN short flag and stays.
  # NOT on this list, deliberately: `## Sync`, `--description-file`, `nxf create --json -`, and
  # `nxf mention add`. `## Sync` collides with three unrelated live headings —
  # `crates/chat/docs/guide/{de,en}/core-concepts.md` (`nxc`'s own sync concept) and
  # `crates/cli/docs/guide/en/running-a-relay.md` — checked, none about prime. The two create
  # flags are LIVE, working CLI flags used throughout the codebase and its docs — only their
  # mention inside prime's human view left, not the flags themselves. "`nxf mention add`" is a
  # live, working verb; only ONE Core Rules bullet that taught it inside `prime`'s human view is
  # gone (`--json`'s `rules[8]` still teaches it, unchanged) — the verb itself was never on the
  # chopping block.
  #
  # **"Context Recovery" USED TO BE HERE TOO, and is corrected rather than left** (nxf q065, task
  # 4). This note said `nxm`'s own retirement "has NOT landed yet, so it still renders under its
  # own, still-live blockquote" — true when task 2 wrote it, false now: task 4 (below) drops the
  # blockquote from `nxm prime`'s human view too, so all three modules (`nxf`, `nxc`, `nxm`) have
  # now retired it and no live rendering carries the label "Context Recovery" anywhere. The term
  # itself moved into the VOCABULARY, in task 4's own block below.
  # ---- `nxc prime` RESHAPED FOR THE SESSION-START BUDGET, nxf h4d3 task 3 -----------------------
  # The HUMAN view (not `--json`) of `nxc prime` shed the Context Recovery blockquote, five of its
  # eight command-reference entries, the when-stuck paragraph and the "## Declared Team" section to
  # clear the host's 10.240 B SessionStart hook ceiling — the fixed prose alone had been 1.938 B of
  # a 3.394 B block in `watch-bundestag`. `PrimeReport`'s fields (`context_recovery`,
  # `coordination_rule`, `when_stuck`, all eight `commands`, and the roster's `roles`/`channels`)
  # are UNTOUCHED and still render in full under `prime --json` — the stable record an embedding
  # app reads — so what is "retired" here is specifically the RENDERED MARKDOWN, exactly the same
  # shape as `nxf xe2z` task 2 above. An agent-facing doc that still describes what `nxc prime`'s
  # plain output shows is the false map this sweep exists to catch; one that describes
  # `prime --json`'s fields is still correct.
  '## Chat Commands'   # heading-anchored: `PRIME_COMMANDS` (all eight entries) still serializes
                        # under `--json`'s `commands` key — only this rendered section, replaced by
                        # "## How a conversation moves", is gone.
  '## Declared Team'    # heading-anchored: `roster.roles`/`roster.channels` still serialize under
                        # `--json`, and `render_declared_team` stays public, callable, and unit
                        # tested directly (its own doc comment covers why removing the RENDER call
                        # was still right even though the bare roster it prints is not a strict
                        # subset of "## Who you can address" — a channel-only persona is a
                        # channel-entry line there, not a roster line) — only this rendered section
                        # is gone.
  # NOT on this list, deliberately: `nxc list`, `nxc status`, `nxc threads show`, `nxc search`, and
  # `nxc transcript show` — the five verbs task 3's brief names as cut from the command reference.
  # All five are live, working verbs (`nxc --help` lists them, `consolidated_entrances.rs`'s
  # `the_agent_facing_surface_is_exactly_the_verbs_the_design_names` pins them); only their mention
  # inside `prime`'s human view is gone, exactly the shape `nxf mention add` above already
  # explains. A bare term on any of the five would flag hundreds of honest mentions of a verb that
  # still exists — the exact failure mode this file's anchoring rules exist to avoid. Nor is "When
  # something does not move" listed: `PRIME_WHEN_STUCK`'s exact text is unchanged and still
  # serializes under `--json`'s `when_stuck` key, so the phrase is not retired, only unrendered by
  # default — the same reasoning that keeps `## Core Rules`'s own bullet TEXT off task 2's list
  # above (only that heading is listed, never the rules' wording).
  # ---- `nxm prime` RESHAPED FOR THE SESSION-START BUDGET, nxf q065 task 4 -----------------------
  # The HUMAN view (not `--json`) of `nxm prime` shed the Context Recovery blockquote, the
  # `NEXUS_MEMORY.md`/`CLAUDE.md` explanation paragraph, and `nxm classify` from the fixed command
  # list, and replaced "## Memory Commands" with "## How a memory is written" (three verbs instead
  # of six, as literal shell examples) to clear the host's 10.240 B SessionStart hook ceiling — the
  # fixed prose alone had been 2.285 B of a 3.629 B block in `watch-bundestag`. `PrimeReport`'s
  # fields (`context_recovery`, `memory_rule`, `context_document`, `introduction_rule`, and all six
  # `commands`, both `nxm classify` entries included) are UNTOUCHED and still render in full under
  # `prime --json` — the stable record an embedding app reads — so what is "retired" here is
  # specifically the RENDERED MARKDOWN, exactly the same shape as `nxf xe2z` task 2 and `nxc h4d3`
  # task 3 above. An agent-facing doc that still describes what `nxm prime`'s plain output shows is
  # the false map this sweep exists to catch; one that describes `prime --json`'s fields is still
  # correct.
  '## Memory Commands'   # heading-anchored: `PRIME_COMMANDS` (all six entries, including both
                          # `nxm classify` invocations) still serializes under `--json`'s
                          # `commands` key — only this rendered section, replaced by "## How a
                          # memory is written", is gone.
  'Context Recovery'     # now genuinely retired everywhere (see the corrected note in task 2's
                          # block above): no module's human view renders this label any more. Not
                          # anchored — `PRIME_CONTEXT_RECOVERY`'s own STRING never contained these
                          # two words (only the removed `"> **Context Recovery:** {}"` format
                          # wrapper did), so nothing live in `--json` or in prose about the field
                          # can collide with the bare label.
  # NOT on this list, deliberately: `nxm classify` (both fixed-list entries), "CRITICAL — Memory
  # rule", "Writing one:", and "This block IS how the project's memory reaches you". `nxm classify`
  # is a live, working verb (`nxm classify --help`, the guide, `crates/nxs/src/mcp/memory.rs`'s
  # `memory_classify` tool) that only lost its two mentions inside `prime`'s FIXED verb list — it
  # still appears there conditionally, in `introduction_gap`'s own situational note, unaffected by
  # this cut — the exact shape the five `nxc` verbs above already explains. The other three are
  # exact constant TEXT (`PRIME_MEMORY_RULE`, `PRIME_INTRODUCTION_RULE`, `PRIME_CONTEXT_DOCUMENT`)
  # that is UNCHANGED and still serializes verbatim under `--json` — a bare term on any of them
  # would flag that honest, live occurrence, the same reasoning that keeps `PRIME_WHEN_STUCK`'s
  # text off task 3's list above.
  # ---- THE SINGLE SessionStart HOOK SPLIT INTO ONE PER MODULE, nxf n2m6 + a2a1 -----------------
  # `crates/nxs-init/src/assembler.rs` wired ONE SessionStart hook running `nxs prime`, promised as
  # "**exactly one**" in its module doc, in `HOOK_COMMAND`'s own doc and in spec §6.2 / TB-9. The
  # host truncates per hook OUTPUT at 10.240 B (2026-08-28 telemetry: 1.114 events / 947
  # transcripts; three ~8 KB hooks arrived whole, 24.127 B), so the wiring is now one entry per
  # active module — `nxf prime`, `nxm prime`, `nxc prime` — with the `|| cat NEXUS_MEMORY.md`
  # fallback on the FIRST entry only (Ruling R1). A doc that still tells a reader there is exactly
  # one hook, or that the hook runs `nxs prime`, is the false map this sweep exists to catch.
  'HOOK_COMMAND'           # the constant itself is GONE, replaced by `hook_commands(&[binary])` +
                            # `HOOK_FALLBACK`. Anchored on the identifier, so prose about "the hook
                            # command" is not swept — only code or docs still naming the removed
                            # const. NOTE `beads.rs` has its OWN unrelated `HOOK_COMMAND` (`bd
                            # prime`), which is live; it is in the exclusions below.
  'exactly one SessionStart hook'   # the retired promise, spelled as the module doc spelled it.
  'single SessionStart hook'        # …and as `AssembleReport`/`HookWireReport`/spec spelled it.
  'the single `nxs prime` hook'     # …and as the init paths' own comments spelled it.
  #
  # NOT on this list, deliberately:
  #
  # * `nxs prime` — still a LIVE verb, and deliberately so (task-5 brief: "`nxs prime` bleibt als
  #   Befehl für den manuellen Aufruf bestehen; nur der Hook ändert sich"). It is still what
  #   AGENTS.md points at, still what `nxs prime --persona` composes, and still what
  #   `crates/nxs/src/prime.rs` implements. Only its role AS THE WIRED HOOK ended. A bare term here
  #   would flag hundreds of honest, live occurrences.
  # * `nxf prime` / `nxm prime` / `nxc prime` — the OPPOSITE of retired: these are the new hook
  #   commands. `nxf prime` in particular used to be retired vocabulary (the pre-umbrella v0.5.x
  #   hook, stripped by `nxs migrate`) and is now something the assembler itself writes.
  #   That reversal used to be recorded on `migrate.rs`'s `LEGACY_FLOW_HOOK`; nxf 6j6v.sp5v
  #   DELETED that constant along with the bespoke strip around it, because looking for one exact
  #   legacy command was the bug — see the block at the end of this list. Nothing in `migrate.rs`
  #   spells a hook command any more; it reads the assembler's set.
  # * `|| cat NEXUS_MEMORY.md 2>/dev/null` — unchanged and live, only relocated onto the first
  #   per-module entry instead of the single umbrella one.
  # ---- THE COMPOSED-FAN-OUT GATE REPLACED BY A PER-HOOK ONE, nxf qhgw ---------------------------
  # `crates/nxs/tests/the_session_start_ceiling.rs` measured the COMPOSED `nxs prime` against the
  # ceiling and justified it explicitly ("Both measure the COMPOSED fan-out — the thing the host
  # actually truncates"). After the split the composed output is not a host output at all, so the
  # gate measures each module's hook output separately. The test function was renamed with it.
  'the_composed_session_start_[a-z_]*'   # BOTH retired names in one term: the original
                                          # `..._stays_under_the_hosts_cut_off` and task 1's
                                          # inverted `..._exceeds_the_corrected_cut_off_pending_task_6`.
                                          # The new name is `each_modules_hook_output_stays_under_
                                          # the_hosts_cut_off`, which this pattern does not match.
  'the thing the host actually truncates' # the retired justification, verbatim.
  #
  # NOT on this list, deliberately: "COMPOSED fan-out" and "composed" on their own. The composed
  # fan-out still EXISTS and is still rendered, measured and tested — it is what `nxs prime`
  # produces by hand and what a persona's prompt is composed from; what changed is only that it is
  # no longer compared to the host's per-hook ceiling.
  # ---- migrate STOPPED SPELLING HOOK COMMANDS ITSELF, nxf 6j6v.sp5v ----------------------------
  # `nxs migrate` used to carry its own copy of the wiring logic: one hard-coded legacy command, a
  # bespoke strip over `.claude/settings.json`, and a SECOND atomic settings writer beside the
  # assembler's. All three are gone — the condition is now "settings.json names any command in
  # `assembler::managed_hook_commands`", and `wire_session_hook` does the rest. The removal is what
  # closed the bug: a workspace on the intermediate single `nxs prime` hook carries no `nxf prime`,
  # so the exact-match strip found nothing and migrate silently did nothing (19 of 19 real
  # workspaces, measured 2026-08-29).
  'LEGACY_FLOW_HOOK'
  'strip_legacy_flow_hook'
  'rewrite_legacy_hook'          # renamed to `converge_session_hook` in the same change
  'legacy `nxf prime` hook rewritten'  # the retired success line; now "outdated SessionStart
                                       # wiring rewritten", because naming the one shape it used to
                                       # find would be wrong for the shape it now finds most often.
  # ---- THE MEMORY INDEX GAINED A BOUND, nxf 6j6v.5jm3 ------------------------------------------
  # `nxm prime`'s index was one line per memory with nothing bounding it. It is rendered within
  # `PRIME_BLOCK_BUDGET_BYTES` now, and a cut block says so. Nothing was RENAMED here, so there is
  # no identifier to sweep — what rots instead is the CLAIM, and a claim has no compiler. These are
  # the phrasings that asserted the index was complete; each surviving hit needs reading, because
  # several are correct about `--json`, about `NEXUS_MEMORY.md`, or about a small fixture.
  'every active memory as a context block'
  'the same one `nx[a-z] prime` replays'   # the document notice's retired equality claim
  'reads in full on EXACTLY one surface'   # the partition property, restated without "in full"
  #
  # NOT on this list, deliberately: `render_memory_index` and `## Memories (`. Both are live — the
  # renderer is what the bound calls and what `NEXUS_MEMORY.md` still calls unbounded, and the
  # heading simply gained a second form. `ceiling_warning` still takes
  # `composed_bytes` and still says "composed", correctly.
  # ---- THE SERVICE STOPPED BEING A SINGLETON, nxf 6j6v.gd9p -----------------------------------
  # A machine can run several named service instances now, so what rots is partly an identifier and
  # mostly a CLAIM. The label, the home directory and the alias name all came out of constants;
  # three of those constants are gone and the fourth (`.nexusflow` in `endpoint.rs`) was a fifth
  # copy of a string `home.rs` claimed was "named once".
  'launchd::LABEL'    # the public const; every caller asks `Instance::label()` now
  'REGISTRY_DIR'      # `endpoint.rs`'s own copy of the home directory name
  # The claims. Each surviving hit needs READING rather than replacing: several are still true of
  # the production instance, and one — the plist's `ProgramArguments` — is a constraint this change
  # deliberately keeps.
  'ONE background agent'
  'one process per machine'
  'There is one of it'
  'Es gibt genau einen'
  #
  # NOT on this list, deliberately: `RETIRED_LABELS`, `render_plist`, `install_with` and
  # `uninstall_with` are all live — the three functions gained an argument rather than a successor,
  # and a signature change leaves an identifier the compiler follows, which is the one shape this
  # sweep is not for.
  # ---- A REPLY GAINED A THIRD ANSWER, nxf 6j6v.553s (a)/(d) -----------------------------------
  # `--needs-rework` is the third way a turn can end, so what rots is one identifier and — far more
  # of it — a COUNT that was written out in prose in a dozen places. A count has no compiler.
  'escalation_kind'   # the one mapping; it takes both bits now and is `surface::reply_kind`
  # The claims. Each surviving hit needs READING: several are still correct, because a step that
  # declares no `on_needs_rework:` edge really is offered exactly two endings, and the `flow:
  # sequential` half of the guides describes a flow that has no third answer at all.
  'exactly two ways to end'
  'EXACTLY TWO FORMS'
  'SECOND and last thing'
  'second and last thing'
  'zweite und letzte'
  'zwei Antworten'
  'separate step list'          # `steps:` is one; the claim now scopes to `flow: sequential`
  'never recorded'              # a stepped slot RECORDS its run and step (`ChatStore::flow_mark`)
  #
  # NOT on this list, deliberately: `flow: sequential`, `members:`, `flow_steps`, `slot_target` and
  # `current_pass` are all live and unchanged for the flow they serve. `steps:` is a second shape
  # beside them, not a successor — reading a `flow: sequential` sentence as stale is the error this
  # note exists to prevent.
  # ---- THE BLOCK BOUNDARY CLOSED THE FORGERY GAP, nxf 6j6v.k1gb --------------------------------
  # Two identifiers went, and — as with the count above — far more of what rots is a CLAIM: for two
  # items the consolidator's own docs, its notices and both guides told a reader that `from="…"` was
  # not authenticated and that a member could forge a block. That is now false in the other
  # direction, and no compiler follows a sentence.
  'PASS_THROUGH_FORGERY_CAVEAT'  # the constant; it is `ATTRIBUTION_NOTICE` and says the opposite
  'join_replies'                 # the fold's `sender: body` renderer; both forms use `render_blocks`
  # The claims. Each surviving hit needs READING: a supersession note that says the attribution USED
  # to be forgeable is correct, and this file's own history of the gap is one of them.
  'not authenticated'
  'nicht authentifiziert'
  'are CLAIMS'
  'is FORGED'
  'forged `</message>`'
  #
  # NOT on this list, deliberately: `untrusted_channel_replies` and `UNTRUSTED_REPLIES_FRAMING` are
  # LIVE and unchanged in meaning — the framing answers "may these words be obeyed", which the
  # boundary does not touch. Reading a framing sentence as stale is the error this note prevents.
  # ---- CLAUDE.md LEFT THE ASSEMBLER'S DESTINATIONS, 6j6v.q6e3 ---------------------------------
  # The managed block goes to AGENTS.md and nowhere else, so three report categories, an import
  # directive and four helpers went with it. The purest case this header describes: nothing that
  # remains is named `Imported`, so the compiler has no way to notice the twelve prose sites that
  # still described the old behaviour — a spec section, a manifest contract, two module docs.
  'ClaudeAction::Imported'   # QUALIFIED: bare `Imported` is ordinary English in half the tree
  'AlreadyImporting'
  'BlockInlined'
  'block_inlined'            # the `--json` spellings of the two removed categories
  'already_importing'
  'CLAUDE_IMPORT'            # the `@AGENTS.md` directive and the three helpers around it
  'claude_imports_agents'
  'with_import_prepended'
  'contains_any_managed_block'
  # The CLAIMS, and every surviving hit needs READING rather than fixing: a supersession note that
  # says the block USED to be written to both files is correct, and the changelog fragment for this
  # very change is another. What is stale is a sentence in the PRESENT tense.
  'AGENTS.md/CLAUDE.md'
  'AGENTS.md, CLAUDE.md'
  # The SPACED spelling, added after it cost a round (2026-09-02). While this change was in review,
  # `nxs`'s brand-new guide landed on main writing the pair as `AGENTS.md / CLAUDE.md` — the same
  # claim, one space wider, and invisible to the two terms above. It survived the rebase and would
  # have shipped as a guide page telling users their CLAUDE.md gets a managed section. That is the
  # header's own lesson arriving from the other direction: not a file the change failed to open,
  # but a file that did not exist when the change was written.
  'AGENTS.md */ *CLAUDE.md'
  #
  # NOT on this list, deliberately: `@NEXUS_MEMORY.md` and `NEXUS_MEMORY.md` are LIVE. The binding
  # did not go anywhere — it stayed in AGENTS.md, which is the whole point of the change, and the
  # hook's `|| cat NEXUS_MEMORY.md` fallback is untouched. Reading a hit on either as stale is the
  # error this note prevents.
  # ---- `unconfirmed` LOST ONE OF ITS TWO READINGS, 6j6v.d43g ----------------------------------
  # Not a verb and not a path, which is why it belongs here rather than in spite of it: what was
  # removed is a MEANING, and a meaning leaves even less to grep for than a deleted identifier.
  # `ServiceState::Unconfirmed` used to cover two things — a handed-on process id, and a service
  # that stamped its own start late — because the probe could not tell them apart. It can now, so
  # the second reads as `running`, and every sentence still OFFERING it as a reading of
  # `unconfirmed` is a false map: it sends a reader looking for a hang that cannot be there.
  'stamped itself late'
  'verspätet gestempelt'
  'stamped .{0,20}late'      # QUALIFIED: the claim survives rewording; the sentence is what matters
  #
  # NOT on this list, deliberately: `START_TIME_SLACK` and `born_together` are LIVE and keep their
  # names — what changed is which DIRECTION the slack governs, not that either went away.
  # ---- WHAT A WAITING ROLE USED TO BE TOLD, 6j6v.hw2t -----------------------------------------
  # No identifier went here at all: what was retired is three SENTENCES, in three surfaces that each
  # told a role to escalate (or to answer) rather than wait — the engine's own forced ending, the
  # sidecar's reminder, and the channels guide. That is the purest form of the failure this file
  # exists for, one step further out than a deleted verb: nothing to grep for and nothing that stops
  # compiling, while the prose an agent reads still names the behaviour that produced the measured
  # false alarm. So the terms are the exact bytes those sentences carried.
  #
  # Distinctive enough to be spelled SHORT: no surviving text in this repo says "wait for work you
  # commissioned" or "say what you handed out" for any other reason, and the guide claim is quoted
  # with its own punctuation in both languages.
  'wait for work you commissioned'   # the forced ending, `role.rs::reply_obligation`
  'say what you handed out'          # the sidecar reminder's commissioned-work paragraph
  'no way for a step to say'         # the channels guide's own statement of the gap (EN)
  'nicht .noch nicht. sagen'         # …and DE. The dots stand in for the typographic quotes
  # THE SAME CLAIM, WORDED OTHERWISE — and the four terms above did not catch it. The review of
  # PR #460 (Code Quality #1) found `commands.md` still asserting it in both languages, a few dozen
  # lines from the paragraph that same change had just added, because that file says "there is no
  # spelling for that" where `channels.md` says "no way for a step to say". A vocabulary tuned to
  # ONE surface's phrasing reports clean about the next surface's, which is the sweep failing in the
  # quietest way it can. Anchored on the words each file actually carries, not on a paraphrase.
  #
  # NOT on this list, deliberately: "it is not a way to say 'not yet'" / "es ist kein Weg, 'noch
  # nicht' zu sagen". That is the `--escalate` bullet's own heading and it is STILL TRUE — escalating
  # is not how you wait, and the sentence survives this change saying so. What retired is the clause
  # under it that went on to claim no spelling exists at all. Listing the heading would flag the
  # survivor at every honest mention, which is the trap `Engine::send` above is anchored against.
  'no spelling for that'             # `commands.md`, EN
  'keine Schreibweise'               # `commands.md`, DE
  # ---- FOUND MISPLACED, AND MOVED HERE BY 6j6v.hw2t -------------------------------------------
  # Everything from here to the end of this array stood in EXCLUDE until 2026-09-10, where terms do
  # nothing at all: `excluded()` matches an entry against a FILE PATH, and no path contains
  # `PendingWrite` or `facade::Landing`. So the whole 6j6v.xbnh list was inert — the sweep reported
  # clean about a vocabulary it was never searching for, which is the exact failure this file's own
  # header warns about, arrived at by putting a list in the wrong array rather than by omitting it.
  # Moved verbatim, comments and all; nothing about the terms themselves is changed. It prints four
  # files that 6j6v.xbnh's own change never opened, and they are that item's to judge, not this one's
  # (nxf 6j6v.9p5r carries them, opened rather than fixed here: bringing the tool back is one line
  # of structure, while rewording the prose needs to know what replaced the ceiling).
  # ---- THE TWO-CLASS REPLAY AND ITS BUDGET, 6j6v.xbnh -----------------------------------------
  # nxf 6j6v.waq9 split the session start into a full-text class (`rules`, plus the unjudged) and an
  # index class, and capped the first at 48 KiB. Both halves are gone: there is no full-text channel
  # at all now, so the ceiling that governed one has nothing left to govern. This is the purest
  # shape the header describes — the constants and functions vanish, every gate stays green, and
  # what rots is the prose that explains a mechanism nobody can find any more.
  'PRIME_ALWAYS_BUDGET_BYTES'  # the 48 KiB ceiling, and the two names it was computed over
  'PRIME_ALWAYS_CATEGORIES'
  'replayed_in_full'
  'always_bytes'
  'check_always_budget'        # the write-time gate, its predicate and its pending-write shape
  'counts_toward_always'
  'PendingWrite'
  'split_by_presence'          # the renderer's half of the split, and the sentence that headed it
  'PRIME_PRESENCE_RULE'
  'INDEX_SUMMARY_CHARS'        # the derivation this item replaced: first line, cut at 110 chars
  'the_session_start_budget'   # the acceptance file; now `the_written_introduction.rs`
  'session-start budget'       # the refusal's own wording, as prose quotes it
  # The two SECTION HEADINGS the split rendered. Spelled with their `###` and their trailing ` (`
  # so they cannot match ordinary prose about rules or about recalling something — these are the
  # exact bytes a doc or a golden would have quoted.
  '### Rules \('
  '### On recall \('
  # QUALIFIED, never the bare word: `Landing` is ordinary English, and the removed thing is one
  # `pub(crate)` struct in memory's facade.
  'facade::Landing'
  # ---- THE ESCAPING-FREE INPUT SEAM MOVED OUT OF FLOW, 6j6v.s46h ------------------------------
  # `crates/cli/src/field_input.rs` — a private module of `nxf` — became
  # `nxs_foundation::text_input`, because `nxc send`/`nxc reply` needed the same `-`/`--*-file`
  # notation for a message BODY, which is not a field of anything, and chat may not depend on flow.
  # A pure move: every function and its contract is unchanged, so nothing here is a LOSS — what
  # rots is prose that sends a reader to a path and a module name that no longer exist.
  #
  # The module name is spelled bare because nothing named `field_input` survives anywhere; the file
  # path is listed separately because a doc is as likely to quote the path as the name.
  'field_input'
  'cli/src/field_input'
  # ---- THE CALLER-BLIND `addressable:`, 6j6v.st83 / 6j6v.g0yn ----------------------------------
  # `Addressable` had two states and one question ("may anybody send directly?"). It now has four
  # and the question takes a caller, so the method that answered the old one is gone — and the
  # CHANNEL LIST form, while it still parses, no longer means "these are the channels I take part
  # in": a channel's cast is declared at the channel.
  #
  # `allows_direct` is spelled with a trailing "not an identifier character" guard because the two
  # methods that replaced it BEGIN with it — `allows_direct_from`, `allows_direct_from_anyone` —
  # and without the guard every honest use of the successors reports as a stale hit.
  'allows_direct([^_]|$)'
  # The old prose for the list form, in both languages. There is no identifier for either: what
  # rots is a sentence telling a reader that naming channels here is how a persona says where it
  # takes part. Anchored on `addressable` so the ordinary English of "via channels" / "über
  # Kanäle" elsewhere in the guides does not print.
  'addressable: via channels'
  'addressable:.*only through these channels'
  'addressable:.*nur über diese Kanäle'
  # The LIST FORM ITSELF, spelled as prose quotes it. Added after the review of PR #472 found the
  # one file the three terms above missed: `tests/golden/messaging.trycmd` still told the reader
  # "each declares `addressable: [review]`" over a fixture that had been migrated to `none` — the
  # fixture is a `.yaml` this change opened, the sentence describing it is a `.trycmd` it did not.
  # A declaration that legitimately still carries the list trips this too; that is the deprecation
  # showing up, which is the point.
  'addressable: \['
  # ---- `release` LEAVES THE USER SURFACE, 6j6v.b9nf — owner 2026-09-17: release leaves the user
  # surface --------------------------------------------------------------------------------------
  # `nxc release --thread <id>` and `Engine::release_working_tree` are gone: offered to an agent,
  # the verb could take the working copy away from a running coding operation, and on a
  # `WorkerConfig::Custom` worker that never implemented `Worker::session_is_running` it was an
  # unconditional release with no guard at all. Every hand-off now parks first; a held copy of a
  # dead chain is the sweep's, a held copy of a live one is `nxc withdraw --thread <id>`'s.
  #
  # BARE `nxc release`, not qualified: nothing on the CLI is spelled that way any more, so every
  # hit is either an honest historical record or prose to correct.
  #
  # The seam method is swept as the BARE symbol with a trailing guard, `($|[^_a-zA-Z])`, and the
  # guard is the whole of why one term can stand for both removed spellings. Two things went: the
  # free function `orchestration::release_working_tree` (what `nxc release` called) and the handle
  # method `Engine::release_working_tree` — and prose names either one as `release_working_tree`,
  # in a rustdoc link, after a `::`, or in backticks, so a term anchored on a qualifier would miss
  # most of the text it exists to find. What the bare form must NOT flag is the survivor:
  # `ChatStore::release_working_tree_and_take_next` (working_tree.rs) is the transaction every
  # hand-off still goes through, and `release_working_tree_if_scope_is_done` (orchestration.rs) is
  # the reply path's own private release. Both continue with an identifier character, which the
  # guard excludes; every removed spelling is followed by a backtick, a `(`, a quote, punctuation
  # or the end of the line, which it admits. This replaced an earlier pair of qualified terms whose
  # comment claimed two spellings were listed while only `Engine::release_working_tree` was
  # (review of this branch, Minor #4) — the sentence and the list now say the same thing.
  'nxc release'
  'release_working_tree($|[^_a-zA-Z])'
  # ---- THE README'S VOCABULARY REACHED THE SHIPPED SURFACES, 6j6v.chap / 6j6v.8tq9 --------------
  # Nothing was removed that a compiler could follow: what retired is a CATEGORY CLAIM. The README
  # (PR #481) calls the suite "the board, the memory and the channel your agents work from" — one
  # binary, three tools; chat is the channel between you and your agents. The old framing — nxs as
  # a "platform", chat as "agent-to-agent" coordination, "three products", the engine line — lived
  # on in help text, the init chooser, guides and the homepage feed. The review of PR #482 found
  # the first sweep had searched only the literal strings and missed the paraphrases; they are here
  # too. Each surviving hit needs READING: vision docs carry supersession notes, and specs record
  # what was decided under the old name.
  'platform umbrella'
  'platform operation'           # a line-wrapped "platform / operation" (it was one in `nxs sync`
  'Plattform-Operation'          # --help) is invisible to a line grep; shape 2 below sees it
  'agent-to-agent messaging'
  'Agent-zu-Agent|von Agent zu Agent'
  '`?nxc`?: agent-to-agent coordination'   # the category LABEL form only — see the note below
  'agents talking to each other'
  'miteinander reden'
  'how agents coordinate'
  'agents coordinate (over|on)'
  'nexus-chat.*coming soon'      # anchored: bare "coming soon" is ordinary English
  'three products'
  'drei Produkte'
  'engine for projects'
  # The beads comparison 8tq9 corrected. beads syncs with `bd dolt push/pull` and settles a
  # same-field collision last-write-wins by `updated_at`; the JSONL is an export, not the sync.
  'closer to last-write-wins'
  'a Dolt export'
  'git-style sync'
  #
  # NOT on this list, deliberately: bare `agent-to-agent coordination`, `coordinate` and
  # `coordination`. The `nxs prime` chat block keeps them ON PURPOSE — `PRIME_COORDINATION_RULE`
  # (the `coordination_rule` `--json` contract, pinned byte for byte in `prime_golden.rs`),
  # `PRIME_INTRO` and the chat guide line that labels the field. Those are instructions to agents,
  # where "coordination" names the MECHANISM, not the product category. `coordinator` is live
  # vocabulary for a role besides. Also not listed: `nxs platform` — `docs/specs/nxs-platform-
  # foundation.md` is named that, and every spec citing it would print.
  #
  # nxf 6j6v.ezbr + 6j6v.s2cj (2026-09-21) — `withdraw` is a person's verb, and a withdrawal
  # interrupts the chain below it. The VERB stays, so it is not listed; what left is the agent
  # surface's offer of it and the rule that only a fully withdrawn round was discharged.
  'take back a commission, queued or running'   # the `nxc prime` cheatsheet line
  'discharge_a_fully_withdrawn_round'            # now `interrupt_the_chain_above`
  'any thread above the parked commissions'      # the old `withdraw --thread` help
  'of a fully withdrawn round'                   # 7me0's rule, in prose
  # NOT listable, and the review of PR #483 found it by hand: a verb that STAYS but may no longer
  # be offered to an agent. `git grep 'nxc withdraw' -- crates/chat/src` is that sweep.
)

# ---- THE EXCLUSIONS, each with the reason it is not a defect ----------------------------------
#
# Written down rather than implied. An unexplained exclusion is how a sweep comes to report "clean"
# about a set it quietly stopped looking at.
EXCLUDE=(
  'docs/specs/plans/'   # dated implementation plans: frozen records of what was built THAT DAY.
                        # Rewriting one would falsify the record it exists to be.
  'docs/reviews/'       # the same class, arrived at the same way (6j6v.sgfr): a completed review
                        # names the files it FOUND THINGS IN, at a stated date and with a stated
                        # verdict. `2026-07-13-beads-provenance-review.md` cites four `infra/`
                        # paths as evidence; the tree is gone, and editing the citation would
                        # leave a gate report that no longer says what was examined.
  'release-notes.json'  # shipped-version entries. Those releases really did carry those verbs;
                        # editing them would make the published history lie.
  'target/'             # build output.
  'node_modules/'       # vendored dependencies; not our prose.
  'content/dist/'       # generated bundle; its source is `content/`, which IS swept.
  'sweep-retired-vocabulary.sh'  # this file: the vocabulary list would match every one of its own
                        # terms, and a tool that always reports itself trains a reader to skim.
  'require-green-ci.test.sh'     # GitHub's Actions API names its own payload `workflow_runs`, which
                        # is the same string chat's removed run table had. Nothing in that file has
                        # ever been about this vocabulary; it is a collision, not a leftover.
  'crates/nxs-init/src/beads.rs'  # it has its OWN `HOOK_COMMAND`, and that one is LIVE: `bd prime`,
                        # the beads hook this suite detects and rolls back. Same identifier, a
                        # different constant in a different module, matched by the `HOOK_COMMAND`
                        # term added for nxf n2m6 + a2a1. A collision, not a leftover — and it must
                        # keep its name, since it is what `migrate_beads` greps settings.json for.
  'docs/superpowers/plans/'  # the SDD plan documents. Same REASON as `docs/specs/plans/` above —
                        # a plan records what was decided and measured on its date — but not the
                        # same standing: `docs/superpowers/` is untracked in this repo today, so
                        # this entry matches nothing and is here for when such a plan is committed. `q3fh-session-start-budget.md` quotes the retired
                        # single-hook wiring and the "Context Recovery" line precisely because they
                        # are what its tasks set out to change; rewriting them would erase the
                        # before-state the plan exists to record.
)

# `.github` JOINED THE ROOTS with 6j6v.sgfr, and `infra` left them. The teardown removed five
# WORKFLOWS as well as a source tree, and four surviving workflows named them in comments — none
# of which a sweep rooted only at source directories would ever have looked at. `infra` is gone
# from the list for the plainer reason that the directory is gone.
ROOTS=(crates xtask docs content agent-sidecar npm release .github examples tests changes
       README.md AGENTS.md CLAUDE.md NEXUS_MEMORY.md)

# Every file this change opened: committed since BASE, staged, or dirty in the working tree.
changed() {
  { git diff --name-only "$BASE"...HEAD
    git diff --name-only
    git diff --name-only --cached
  } 2>/dev/null | sort -u
}

CHANGED_LIST="$(changed)"
excluded() {
  local f="$1" e
  for e in "${EXCLUDE[@]}"; do [[ "$f" == *"$e"* ]] && return 0; done
  return 1
}

echo "retired-vocabulary sweep — files naming a retired term that ${BASE}..HEAD never opened"
echo

found=0
for term in "${VOCABULARY[@]}"; do
  while IFS= read -r f; do
    [ -z "$f" ] && continue
    excluded "$f" && continue
    grep -qxF "$f" <<<"$CHANGED_LIST" && continue
    printf '  UNSWEPT  %-60s <- %s\n' "$f" "$term"
    found=$((found + 1))
  done < <(grep -rIlE --exclude-dir=.git -- "$term" "${ROOTS[@]}" 2>/dev/null | sort -u)
done

echo
if [ "$found" -eq 0 ]; then
  echo "  nothing — every file naming a retired term was opened by this change."
else
  echo "  $found occurrence(s). Each is a file to READ, not necessarily a defect: a supersession"
  echo "  note, a changelog fragment or a historical record may name a retired term correctly."
fi
exit 0

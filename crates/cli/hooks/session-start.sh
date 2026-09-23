#!/usr/bin/env sh
# Thin SessionStart hook wrapper: it only calls `nxs prime` and passes the output
# through. `nxs prime` is the umbrella fan-out — it runs the prime of each active
# module (flow/memory/…) with one shared clock, so this single hook stays the same
# however many modules a workspace has. All logic lives in the command, so this stays
# host-agnostic and trivial. Wire it into your host (e.g. a Claude Code SessionStart
# hook) to inject project context at session start.
exec nxs prime

# chat — the channel

The channel between you and the agents working in one workspace. `nxc send --to <handle>` starts
something, `nxc reply --thread <id>` answers it, and `--escalate` is how an agent says *I need help,
or a decision* instead of guessing — it hands the round up to whoever commissioned it.

Chat is a **substrate, not a chat app**, and the difference is the whole reason it exists. A message
is not in flight: it is a row in the same workspace store flow and memory write to, so it is durable,
it syncs with everything else, and it is replayed rather than remembered. There is no daemon to run
and no server to point at. That is also the rule agents are given at session start — never
coordinate through scratch files or ad-hoc notes: they are not delivered, not synced, and never
replayed.

Who may be addressed, and how a group of agents proceeds, is **declared** rather than configured. A
persona file per agent under `.nxs-personas/` gives it an identity, its prompt layers, a model band,
its tools, and who is allowed to address it; a channel declares a group — and an ordered one
(`flow: sequential`) *is* the workflow, not a description of one.

```bash
nxc send --to <handle> -   # a thread id comes back
nxc reply --thread <id> -  # the body on STDIN: `… - <<'EOF'`, your text, then `EOF`
```

The `-` is what keeps the message intact: a verdict or a report is full of backticks and `$(…)`,
and passing one as a shell argument hands it to the shell before `nxc` ever sees it. A one-line
message may still be an argument.

Because agents run unattended, the limits are part of the design and not an afterthought: a hop cap
so a conversation cannot ring forever, a human gate in front of what an agent does not decide, and a
lease on the working copy so two agents cannot edit one checkout at the same time.

## Where to go from here

- [Getting started](nxc-getting-started) — declare a persona and get your first answer back.
- [Personas](nxc-personas) and [channels](nxc-channels) — the two declarations, and the ordered
  channel that carries a flow.
- [Limits and safety](nxc-limits-and-safety) — the hop cap, the human gate, and what an agent does
  *not* decide.

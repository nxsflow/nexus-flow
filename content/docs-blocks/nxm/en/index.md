# memory — what the project knows

Durable facts that outlive the session which learned them: a convention, a gotcha, the reason a
decision went the way it did. `nxm remember` writes one, `nxm recall <key>` reads it back in full,
and a stable key lets a fact be **corrected in place** rather than accumulating contradictory
copies of itself.

The point is not storage but **replay**. Memories are handed back at the start of every session, so
the next agent begins already knowing what the last one learned. Nothing about that is a search
problem: there is no model on the write path, no embedding and no similarity ranking, so the same
store answers the same way today and in six months.

The first step is one line, and the discipline it starts is a hard rule the session banner repeats:
durable project knowledge goes into `nxm remember` and nowhere else. A `MEMORY.md`, or any other
file invented for the purpose, is never read back — the knowledge in it is silently lost.

```bash
nxm remember "The four cargo gates run in CI, not locally" --key gates
nxm memories                  # the index — one line per memory
nxm recall gates              # one memory, in full
```

What arrives is that **index**, not the memories themselves: one line each, saying what the memory
*says*. That is why the line is worth writing carefully — nobody looks up a rule before breaking
it, so the index line has to state it.

## Where to go from here

- [Getting started](nxm-getting-started) — what memory needs before it can run, and your first
  remembered fact.
- [Core concepts](nxm-core-concepts) — keys and auto-keys, the three registers, the retrieval rule,
  and the tombstone.
- [Agents and MCP](nxm-agents-and-mcp) — the `memory_*` tools an MCP host gets, and where an agent
  writes.

# 2026-11-28 · NEX-110 · nxs ticket create prints the ticket as show prints it, and with -q its key alone

- Decided by: coder on NEX-110
- Why: RFC 0002 lists `-q` for `nxs ticket create` without saying what it prints, nor what `create` prints without it. `create` returns what `Engine::ticket` reads right after the write, so a writer sees the minted key and every field in one call, as `nxs decision add` prints the decision in full. `-q` prints the key as `nxs` shows it, on one line in both formats, so `key=$(nxs ticket create … -q)` needs no JSON parser, as v1's `-q` printed the item id; the key, not the object id, because branch names and commit titles carry it.
- Instead of: printing only the key and the object id, which would make every writer run `show` next; or printing the object id with `-q`, which no person or commit uses.
- Topics: tickets, nxs, v2

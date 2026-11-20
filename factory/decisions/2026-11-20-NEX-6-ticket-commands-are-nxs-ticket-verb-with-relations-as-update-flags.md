# 2026-11-20 · NEX-6 · Ticket commands are nxs ticket <verb>, with relations as flags of update

- Decided by: Carsten
- Why: the commands stay two words deep, like RFC 0001's flat commands, and one `update` changes fields and relations in one change (RFC 0002, "Commands, JSON and the seam"). Answer to NEX-57.
- Instead of: nouns of their own (`nxs dep add`, `nxs note add`), or three words (`nxs ticket dep add`).
- Topics: tickets, cli

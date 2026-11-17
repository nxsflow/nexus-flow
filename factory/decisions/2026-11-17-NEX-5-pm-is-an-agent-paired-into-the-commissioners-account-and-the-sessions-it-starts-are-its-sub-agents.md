# 2026-11-17 · NEX-5 · pm is an agent paired into the commissioner's account, and the sessions it starts are its sub-agents

- Decided by: Carsten
- Why: NoCentralDB has built accounts, pairing and sub-agents (NoCentralDB RFC 0010 "Accounts"; RFC 0009 "Sub-agents and ad-hoc spaces"), so pm is paired once into the commissioning person's account and pm's `members.add` signs each coder's certificate; the chain person → pm → coder is signed by NoCentralDB, and the person does not act per session (RFC 0001, "Members"). The coder's invitation still reaches pm as a session message (answer 4a). Decided on 2026-11-17 in chat with pm, replacing answer 7a in NEX-38.
- Instead of: answer 7a, a session as an agent of its own unconfirmed account, tied to the person only by a `spaceMember` record its commissioner signs, until sub-agent certificates were built.
- Topics: members, identity, nocentraldb

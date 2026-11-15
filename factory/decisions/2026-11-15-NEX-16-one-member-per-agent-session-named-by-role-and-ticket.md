# 2026-11-15 · NEX-16 · One NoCentralDB member per agent session: a role on a ticket, commissioned by a person

- Decided by: Carsten
- Why: the signature then says exactly which session wrote a change; the member is the role on a ticket ("coder on NEX-42"), started by the developer who commissioned it, so every change shows the role that did the work and the person who commissioned it. nexus-flow is an open-source tool for any team, so nothing is specific to one person. Ticket ids must stay readable for people and link when mentioned; how, in a local-first store, is RFC 0002's question.
- Instead of: one member per role shared by its sessions, or one per machine with the role as an unsigned field.
- Topics: nocentraldb, members, identity

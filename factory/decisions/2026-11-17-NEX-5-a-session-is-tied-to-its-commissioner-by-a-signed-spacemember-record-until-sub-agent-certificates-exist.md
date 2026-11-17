# 2026-11-17 · NEX-5 · A session is tied to the person who commissioned it by a signed spaceMember record until NoCentralDB's sub-agent certificates exist

- Decided by: Carsten
- Why: pairing works only on a device of the account and sub-agent certificates are designed but not built, so a session joins as an agent of an unconfirmed account and its commissioner signs the `spaceMember` record that names its role and commissioner (RFC 0001, "Starting a session's member"). It is a stop-gap behind the seam with a removal ticket; requests R6 and R8 go to NoCentralDB. Answer 7a in NEX-38.
- Instead of: waiting for NoCentralDB's sub-agent certificates before building members, which would stall priority 1.
- Topics: members, identity, nocentraldb, stop-gap

# 2026-11-28 · NEX-110 · nxs ticket create takes an assignee only with the member: or account: prefix

- Decided by: coder on NEX-110
- Why: RFC 0002, "Who holds a ticket", says a value without either prefix is v1's free text, which only the move from v1 writes. So `create` refuses one with `validation`, and also a prefix with nothing after it; the id after the prefix must be one line, since an account fingerprint is twelve blocks of five digits with spaces between. The store does not check that a member id names a current member: a holder that is gone is `holder_gone`, a derived flag (NEX-119). The holder's role is its `spaceMember` record's for a member, `owner` for an account a member of which holds the role `owner`, and `null` otherwise.
- Instead of: accepting free text from `create`, which the RFC reserves for the move; or refusing a member id that is not a current member, which a ticket assigned before a member's removal could never be shown as.
- Topics: tickets, members, v2

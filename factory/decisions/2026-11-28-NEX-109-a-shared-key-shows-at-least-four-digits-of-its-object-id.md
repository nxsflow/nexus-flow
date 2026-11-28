# 2026-11-28 · NEX-109 · A shared key shows at least four digits of its object id

- Decided by: coder on NEX-109
- Why: RFC 0002 shows a key two tickets hold with "the first characters of their object ids, the way git abbreviates hashes (`ka.42~3f9a`)" without saying how many. The store shows four, as the RFC's example does, and more until the digits tell the ticket apart from every other ticket with the key; a key no other ticket holds is shown alone. An argument resolves as a 32-digit object id, a key, or a key with `~` and the first digits of one holder's object id; a key or a suffix that more than one ticket matches fails with `ambiguous_key`, listing the matching tickets as shown, in object-id order. A deleted ticket resolves from neither, as `byKey` skips it.
- Instead of: git's seven digits, longer than the RFC's example; or a fixed length, which two object ids sharing their first digits defeat.
- Topics: tickets, store

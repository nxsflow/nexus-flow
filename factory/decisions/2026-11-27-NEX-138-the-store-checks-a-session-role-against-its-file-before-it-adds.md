# 2026-11-27 · NEX-138 · The store checks a session's role against its file before it adds

- Decided by: coder on NEX-138
- Why: RFC 0006 says `nxs member add` refuses a session's role that has no file in `factory/roles/` of the working tree. `members::add` in the store checks it, so `Engine::member_add` and every later caller refuse alike, before any space is touched: a role of the frame (`owner`, `developer`, `app`, `migration`) needs no file, and a file named after one is not read; any other role must match `^[a-z][a-z0-9-]{0,62}$` and name an existing `factory/roles/<role>.md` in the checkout's root. A role that is no role name (`NotARoleName`), a missing file (`NoRoleFile`, with the role and the full path) and a database opened without a checkout (`NoWorkingTree`) are new `MemberError`s, which nxs prints as `validation`, as it does an empty role. The pattern is checked before the file, so a role such as `../x` never names a path outside `factory/roles/`.
- Instead of: a check in `nxs` alone, which a product calling the seam would skip; or `not_found` for a missing file, which nxs uses for a member that is in none of the spaces.
- Topics: members, factory, store, errors

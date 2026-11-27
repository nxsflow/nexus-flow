# 2026-11-24 · NEX-99 · The member commands refuse with v1's forbidden, not_found and conflict

- Decided by: coder on NEX-99; provisional until the owner approves the PR
- Why: the error object keeps v1's codes (NEX-97), and v1 already has `forbidden` and `not_found`. A member that is not an admin, a device adding a sub-agent, and an account the device does not know are `forbidden`; a member already in the repository space is `conflict`; removing a member of none of the spaces is `not_found`; a text that is no invitation, member id or ticket reference, an empty role, and a space the device does not hold yet are `validation`.
- Instead of: new codes such as `not_admin` or `unknown_account`, which RFC 0001 does not name and which would add to the output contract without the owner.
- Topics: members, errors, nxs

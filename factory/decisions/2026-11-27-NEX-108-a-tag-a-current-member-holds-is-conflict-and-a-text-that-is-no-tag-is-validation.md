# 2026-11-27 · NEX-108 · A tag a current member holds is conflict, and a text that is no tag is validation

- Decided by: coder on NEX-108; provisional until the owner approves the PR
- Why: RFC 0002 names one new code for tags, `no_free_tag`, and says that `nxs member add` refuses a held tag without naming its code. The error object keeps v1's codes (NEX-97, NEX-99), where `conflict` means that what would be written is already there and `validation` means an argument `nxs` does not accept. So `--tag` or `retag` naming a held tag is `conflict`, and a text that does not match `^[a-z][a-z0-9]$` is `validation`.
- Instead of: new codes such as `tag_held` or `not_a_tag`, which the RFC does not name and which would add to the output contract without the owner.
- Topics: members, errors, nxs

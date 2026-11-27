# 2026-11-27 · NEX-244 · A tag only removed members held is free and handed out again

- Decided by: coder on NEX-244
- Why: the owner put the guard for a freed tag at minting (`2026-11-27-NEX-236-the-guard-for-a-freed-member-tag-sits-at-minting.md`), so `nxs member add` and `nxs member retag` treat a tag that only removed members' records carry as free: it is picked among the free tags by the new member's id, as any never-used tag is, and taken by `--tag` or `retag`. Only a tag a current member holds is refused (`conflict`), and `no_free_tag` comes only when current members hold every tag. A member added again still gets its own earlier tag back when no current member holds it.
- Instead of: keeping NEX-108's safe subset, which refused a freed tag as `validation`; or picking never-used tags before freed ones, which the owner's answer does not ask for and which the guard at minting makes unnecessary.
- Topics: members, tickets, v2
- Supersedes: 2026-11-27-NEX-108-until-the-owner-decides-a-tag-only-removed-members-held-is-not-handed-out-again.md

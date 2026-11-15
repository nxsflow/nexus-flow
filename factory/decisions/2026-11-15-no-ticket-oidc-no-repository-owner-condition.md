# 2026-11-15 · no ticket · A GitHub OIDC role trust in the nxsflow accounts must NOT carry a repository_owner condition — it breaks AssumeRoleWithWebIdentity.

- Decided by: recorded in the project memory (`nxm`, key `oidc-no-repository-owner-condition`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: aws, ci, security

## The record

GitHub OIDC role trusts in the shared nxsflow AWS accounts must **NOT** include a
`repository_owner` `StringEquals` condition — it breaks `sts:AssumeRoleWithWebIdentity` with "Not
authorized" even when `aud`, `sub` and every other claim match. Proven 2026-06-11: every working
role omits it; only the two roles that carried it failed. Pin via the `sub` allow-list alone
(`repo:nxsflow/<repo>:…`), which already constrains the owner.

WHERE THE FIX LIVES NOW: it was originally made in this repo's `infra/lib/github-oidc.ts`. That
whole tree was deleted when the delivery stack moved into the nxsflowcom accounts on 2026-08-23;
the OIDC and deploy stacks now live in the `nxsflow-landing-page` repository. **The rule is
unchanged and still applies to every role in those accounts** — including the ones this repo still
assumes: `nxs-<env>-release` and `nxs-<env>-content-publish`. Only the file pointer moved.

WHAT THIS REPO STILL OWNS is the RELEASE, not the delivery: `release.yml`, `promote.yml`,
`publish-content.yml` and the two publish/promote scripts. They address the other side purely by
convention — role `nxs-<env>-release`, bucket `nxs-<env>-artifacts-<account>`, both built at
RUNTIME from the secrets `NXS_STAGING_ACCOUNT_ID` / `NXS_PROD_ACCOUNT_ID`. That is exactly why the
change of accounts reached them without a single line of edit, and why they survived the teardown
untouched.

A SECOND, INDEPENDENT GITHUB FACT worth knowing: `workflow_dispatch`, tag, `workflow_run` and
`pull_request`-closed workflows only become triggerable once the workflow file is on the DEFAULT
branch (`main`) — so verify deploy/release workflows post-merge, via a dispatch. Its mirror image
holds too: DELETING such a workflow only stops it once the deletion is on `main`, so a teardown's
proof ("no run of it appears any more") is a post-merge observation, never a pre-merge one.

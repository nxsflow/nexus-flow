# 2026-11-15 · no ticket · Never put an AWS account id in code, a PR or the git log — it comes from a GitHub secret at runtime, and tests use the example id 123456789012.

- Decided by: recorded in the project memory (`nxm`, key `aws-account-ids-never-in-the-repo`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: aws, security

## The record

TWO RULES THAT OUTLIVED THE INFRASTRUCTURE THEY WERE LEARNED ON. The `infra/` tree was deleted
from this repository on 2026-08-23 when the delivery stack moved into the nxsflowcom accounts (it
now lives in `nxsflow-landing-page`), but both of these are properties of the accounts and of the
release workflows this repo still owns.

1. **NEVER write an AWS account id into code, a pull request, or the git log.** This repository is
   going public, and the git history is being kept rather than rewritten, so an id committed today
   is an id that is published forever. Account ids are supplied at RUNTIME from the GitHub secrets
   `NXS_STAGING_ACCOUNT_ID` / `NXS_PROD_ACCOUNT_ID`; tests and fixtures use the documented example
   id `123456789012`. This is why the release workflows address the other account purely by
   convention (`nxs-<env>-release`, `nxs-<env>-artifacts-<account>`) and needed no edit when the
   accounts changed.

2. **A TRUST ANCHOR IS DEPLOYED BY HAND, never by a workflow.** A workflow cannot create the role
   it assumes — that is a bootstrap circle, not a policy preference. So the role a deploy workflow
   assumes is applied manually and verified against live IAM in each account; only what it can
   already assume belongs in automation.

See [oidc-no-repository-owner-condition](2026-11-15-no-ticket-oidc-no-repository-owner-condition.md) for the trust-policy condition that breaks such a role.

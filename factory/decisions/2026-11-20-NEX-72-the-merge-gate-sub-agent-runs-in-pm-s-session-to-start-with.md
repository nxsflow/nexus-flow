# 2026-11-20 · NEX-72 · The merge gate's sub-agent runs in pm's session to start with, and pm posts its verdict as a pull request comment

- Decided by: Carsten
- Why: no model key in CI and no cost beyond pm's session; `nxs merge-gate decide --comments` reads the newest verdict comment by a login in `verdict_authors`, and the sub-agent can move into the workflow later without a format change (RFC 0010, "Where the verdict comes from"). Answer (b) in NEX-75; the limit that the verdict then rests on a prompt is named in RFC 0010, "What it does not do".
- Instead of: the sub-agent in the gate's workflow, with a model key as a repository secret, which pm and the RFC recommended.
- Topics: merge-gate, factory, owner

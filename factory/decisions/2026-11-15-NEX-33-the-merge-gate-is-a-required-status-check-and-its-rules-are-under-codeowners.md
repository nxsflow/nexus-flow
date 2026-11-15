# 2026-11-15 · NEX-33 · The merge gate is a required status check, and its rules are under CODEOWNERS

- Decided by: Carsten
- Why: repository rules, not an agent's prompt, then enforce the owner's review; the merge rules and the role files that define exceptions to them are under CODEOWNERS, so agents cannot move the boundary of that review on their own; GENESIS.md says plainly that the verdict per rule is a model's judgement and only the decision on top of it is deterministic. Answer to the review in NEX-30.
- Instead of: a gate enforced only by pm's prompt, with merge rules agents may change without the owner's review.
- Topics: merge-gate, factory, codeowners

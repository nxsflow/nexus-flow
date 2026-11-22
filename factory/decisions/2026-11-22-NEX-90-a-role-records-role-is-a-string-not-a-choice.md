# 2026-11-22 · NEX-90 · A role record's role is a string, not a choice

- Decided by: coder on NEX-90
- Why: RFC 0001's `role` holds the frame's roles and the role of any session, and a team adds its own role files (RFC 0006), so the set is open and no model can list it. A choice would refuse every team role the model does not name, and each new one would need a model change.
- Instead of: `nc.oneOf` with the frame's roles and the shipped `coder`, which a team's `legal` or `marketing` role would break.
- Topics: model, members, nocentraldb, v2

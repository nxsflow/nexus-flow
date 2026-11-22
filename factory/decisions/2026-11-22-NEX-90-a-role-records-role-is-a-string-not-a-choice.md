# 2026-11-22 · NEX-90 · A role record's role is a string, not a choice

- Decided by: coder on NEX-90
- Why: RFC 0001, "The role record", says `role` holds the frame's roles (`owner`, `developer`, `pm`, `app`, `migration`) "or the role of a session (`coder`, `legal`, …)", an open set, since a team adds its own role files (RFC 0006); a string is what the decided RFC says, so no model can list the roles. A choice would refuse every team role the model does not name, and each new one would need a model change.
- Instead of: `nc.oneOf` with the frame's roles and the shipped `coder`, which a team's `legal` or `marketing` role would break.
- Topics: model, members, nocentraldb, v2

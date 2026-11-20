# 2026-11-20 · NEX-9 · A reader without the coordination space gets focus null, and its next leaves the focus key out

- Decided by: Carsten
- Why: nothing derived is stored, and a copy of the focus in each repository space would be stale until pm rewrote it. Since role sessions read the coordination space (RFC 0009, NEX-47), the case is a session not yet added there, or a member a team keeps out of it (RFC 0005, "A member who reads one space but not the other"). Answer to NEX-63.
- Instead of: pm keeps a copy of the goals in focus in the repository space, so every member computes the same focus.
- Topics: goals, ranking, spaces, derived-state

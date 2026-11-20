# 2026-11-20 · NEX-9 · Goal and request keys share one numbering per tag in the coordination space

- Decided by: coder on NEX-9
- Why: RFC 0002 has a key unique within its space; with goals and requests both in the coordination space, a member's number counts across both collections, so `cb.3` is never both a goal and a request (RFC 0005, "Goal ids").
- Instead of: a numbering per collection, under which the goal `cb.3` and the request `cb.3` could both exist.
- Topics: goals, ids, requests

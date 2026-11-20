# 2026-11-20 · NEX-9 · The goals in focus have an order the owner sets, and goal 1's ready work goes before goal 2's

- Decided by: Carsten
- Why: agents can work on several goals in parallel when they have capacity; when nothing of goal 1 is free and nothing will be soon, work goes on with goal 2. Each goal in focus carries a `rank`, and `next` orders the new work by the position of the ticket's goal in focus (RFC 0005, "The order of the goals in focus", "Ranking"). Answer to NEX-59.
- Instead of: no order among the goals in focus, with priority alone ordering their work.
- Topics: goals, ranking, tickets

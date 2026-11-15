# 2026-11-15 · NEX-18 · v2 keeps a library seam for manufakt.io; nexflow.it is not a v2 consumer

- Decided by: Carsten
- Why: manufakt.io is the view onto what happens in nexus-flow and needs its API. nexflow.it is likely to build directly on NoCentralDB instead of nexus-flow (not settled), so nexus-flow becomes a tool for coding work and v2 does not design for nexflow.it, which stays on v1 until it decides its own move.
- Instead of: designing v2's seam for both consumers, which cost detours in both apps under v1.
- Topics: nocentraldb, library-seam, manufakt, nexflowit, v2

# 2026-11-28 · NEX-250 · pm fills lanes with the first local v2 test's path first

- Decided by: Carsten
- Why: a first test of v2 on a real repository (a throwaway copy of nocentraldb, one device, local, with Linear still the source of truth) shows early whether v2 holds up in daily work, before the remaining modules are built on it. So pm fills coding lanes first with that test's path: the ticket core (NEX-110 create and show, NEX-111 update, NEX-113 claim, close and reopen, NEX-115 notes, NEX-119 readiness, NEX-120 and NEX-121 lists and lanes) and reading decisions (NEX-128 derived state, NEX-129 list and show). Goals (RFC 0005), status notes (RFC 0007), requests (RFC 0009) and the move from v1 (RFC 0008) wait until that path is built. Nothing is dropped, only ordered.
- Instead of: (b) taking ready tickets across all epics in plan order, which reaches a first test only as the epics complete.
- Topics: planning, v2

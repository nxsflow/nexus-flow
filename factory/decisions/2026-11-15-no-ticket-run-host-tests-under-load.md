# 2026-11-15 · no ticket · Test an exhausted tick with set_budget(ZERO); never stand the injected clock still while waiting on a tree worker, whose Delay starts when its thread gets there.

- Decided by: recorded in the project memory (`nxm`, key `run-host-tests-under-load`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, testing

## The record

To test what a Host::tick does when its budget is exhausted, use host.set_budget(Duration::ZERO), not a slow machine or sleeps: it is deterministic and is the machine whose lease step ate the budget. And the scripted tree's Delay starts when the worker thread gets there, reading the injected clock then: a test that jumps the clock past a delay and then stands still can hang forever under load. Keep the clock moving while waiting on a worker, and find this shape with a loop of the test binary under eight busy loops on Linux release: idle loops passed it 25 of 25 while load hung it 9 of 25.

# 2026-11-23 · NEX-102 · A sync timeout is the error kind timeout, and its message lists what is behind

- Decided by: pm, on coder's recommendation; provisional until the owner approves the PR
- Why: RFC 0001 has `nxs sync --timeout` report "what is still behind" and exit non-zero, and v1's error kinds have none that fits. An agent can match the new kind `timeout` and run the sync again. The message lists, for a person:
  - the relay connection, with the SDK's last error code;
  - how many changes are unconfirmed;
  - each space that is not held yet, has not finished its initial sync, or is behind.

  The exit code is 1, as for every other error.
- Instead of: reusing `io`, which an agent could not tell apart from a failed read of the disk.
- Topics: sync, nxs, errors

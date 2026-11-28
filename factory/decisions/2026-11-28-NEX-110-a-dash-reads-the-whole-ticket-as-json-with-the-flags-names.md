# 2026-11-28 · NEX-110 · A lone - reads the whole ticket as JSON keyed by the create flags' names

- Decided by: coder on NEX-110
- Why: RFC 0002 keeps v1's flag names and lists the trailing `[-]` of v1's `create`, which reads the whole item as one JSON object from stdin. The object's keys are the flags' names, as v1's were: `type`, `title`, `description`, `priority` (a label or an ordinal, as text or a number), `design`, `dod`, `due`, `defer`, `parent`, `depends_on`, `labels`, `assignee`. A key it does not know is refused, as v1 refused it, and so is a field flag beside the `-`, so one ticket never comes from two sources. `--description -` and `--description-file -` read the description alone from stdin.
- Instead of: the record's snake_case keys (`completion_criterion`, `defer_until`), which v1's payload did not use either; or ignoring unknown keys, which would drop a misspelt field silently.
- Topics: tickets, nxs, v2

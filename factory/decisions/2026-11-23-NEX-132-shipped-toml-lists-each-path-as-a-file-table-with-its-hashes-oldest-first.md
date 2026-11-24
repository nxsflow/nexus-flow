# 2026-11-23 · NEX-132 · shipped.toml lists each path as a file table with its hashes, oldest first

- Decided by: coder on NEX-132
- Why: RFC 0006 says what `shipped.toml` holds (every SHA-256 per path, oldest first, `retired = true`) but not its shape. Each path is a `[[file]]` table with `path`, relative to the repository's `factory/`, `sha256`, a list of 64 lowercase hex digits, and `retired`, false when left out; a hash is taken of the text after CRLF becomes LF. The last hash is the current default, so a release that ships an earlier text again lists its hash again, and a repeated hash is allowed. An unknown key, an empty hash list, a path listed twice and a path that is absolute or holds an empty, `.` or `..` segment or a backslash are refused. The file has no `format` key: it is nexus-flow's own source, read only by the `nxs` it is embedded in.
- Instead of: a table keyed by path (`[files."roles/pm.md"]`), whose quoted keys are easy to get wrong in review, or a `format` key, which no reader built from another release ever sees.
- Topics: templates, factory, seam, v2

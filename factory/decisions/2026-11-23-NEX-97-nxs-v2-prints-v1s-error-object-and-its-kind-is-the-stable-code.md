# 2026-11-23 · NEX-97 · nxs-v2 prints v1's error object, and its kind is the stable code

- Decided by: coder on NEX-97
- Why: RFC 0001 and RFC 0002 say an error is "v1's error object" and speak of its codes; v1's object is `{"error":{"kind":"<code>","msg":"<message>"}}`, so the stable code the ticket asks for is `kind`, and the object keeps v1's two keys. With JSON the default, it goes to stdout with exit code 1, as v1's `--json` does; with `--md`, `error: <msg>` goes to stderr, as v1's human output does. The codes are v1's: `no_workspace` for no `nexus-flow.toml`, `validation` for a setting, file or argument `nxs` does not accept (a command line clap cannot parse included), `conflict` for a file `nxs` will not overwrite, and `io` for the disk or the database. Help and the version print as clap writes them, with exit code 0.
- Instead of: a new `code` key beside or in place of `kind`, which would make v2's object differ from v1's, or a `usage` code of its own for parse errors, a code v1 does not have.
- Topics: v2, output

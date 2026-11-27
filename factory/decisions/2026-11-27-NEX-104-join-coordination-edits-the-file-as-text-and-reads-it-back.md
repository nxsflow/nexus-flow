# 2026-11-27 · NEX-104 · Joining a coordination space edits nexus-flow.toml as text and reads the result back before it replaces the file

- Decided by: coder on NEX-104
- Why: the file is the team's configuration (GENESIS.md: never overwrite it), and rewriting it from `Config` would drop its comments and layout. The store inserts `coordination = "<id>"` after the `repository` line of `[space]` (after the header when there is none), with the file's own line ending, and keeps every other byte. Before it writes, it parses the edited text, which must name exactly what the old file did plus the coordination space; a layout where that fails, such as `space = { … }` inline, is refused with the line to add by hand. The new text is written beside the file with its permissions and renamed over it, so a reader never sees half a file. A format-preserving TOML editor (`toml_edit`) was not taken: adding it changes the code-owned lockfile for one line of text.
- Instead of: rewriting the whole file from `Config`, which loses the team's comments; or adding `toml_edit` as a dependency of the store.
- Topics: storage, coordination, v2

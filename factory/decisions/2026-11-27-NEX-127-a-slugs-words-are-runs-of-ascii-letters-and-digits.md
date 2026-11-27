# 2026-11-27 · NEX-127 · A slug's words are runs of ASCII letters and digits

- Decided by: coder on NEX-127
- Why: RFC 0003, "Handles", makes the slug from the title's words in lowercase, joined by `-`, but does not define a word. A slug in `factory/decisions/` holds only lowercase ASCII letters, digits and single `-` (`cargo xtask decisions --check`). So a word is a run of ASCII letters and digits, and everything else separates words: `write-once` gives `write-once`, `NoCentralDB's` gives `nocentraldb-s`. A letter outside ASCII is left out. The cut at 60 characters keeps whole words. Only a first word longer than 60 characters is cut inside it. A title with no such word has no slug, and the writer must give one. A slug given with `--slug` must follow the same pattern, at any length.
- Instead of: keeping letters outside ASCII, which gives handles that the files' check refuses and that people cannot easily type.
- Topics: decisions, handles, v2

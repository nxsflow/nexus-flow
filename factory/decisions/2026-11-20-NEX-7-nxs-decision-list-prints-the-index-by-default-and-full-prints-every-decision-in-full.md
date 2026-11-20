# 2026-11-20 · NEX-7 · nxs decision list prints the index by default, and --full prints every decision in full

- Decided by: Carsten
- Why: the header states the decision, so the index is what a reader needs first, and the whole log in full costs a session tens of thousands of tokens; the full text is the step a reader asks for when it needs the reasons. Feedback on RFC 0003, "Reading decisions", in chat with pm.
- Instead of: the full text by default with `--index` for the index, as `cargo xtask decisions` does today and RFC 0003's first draft had it.
- Topics: decisions, rfc, cli

# 2026-11-21 · NEX-169 · A GENESIS rule's text is its item as written, without the top-level list marker

- Decided by: coder on NEX-169
- Why: RFC 0010 ("The rules from GENESIS") takes a rule's text from its marker to the line before the next top-level item, and its `nxs merge-gate rules` example shows the text without the marker; so the reader drops the one marker (`-`, `*`, `+`, or a number with `.` or `)`), the whitespace after it and the trailing whitespace, and keeps every other byte, nested items and their indentation included, so the sub-agent judges the words the owner reviewed.
- Instead of: the item's rendered text (Markdown stripped, which loses the nesting a rule's sub-points rely on), or the source with continuation lines de-indented (bytes the owner never wrote).
- Topics: merge-gate, genesis

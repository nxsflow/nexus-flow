# 2026-11-20 · NEX-12 · A redacted v1 note's text is not carried into the space

- Decided by: coder on NEX-12
- Why: redaction was v1's only way to take text back, possibly personal data, and NoCentralDB cannot erase an unmarked value once it is in every member's log (R13); the note moves as a deleted note with an empty body, its record keeps the op's metadata and signature, and the v1 data on disk keeps the text, so nothing is lost (RFC 0008, "Redacted notes").
- Instead of: replaying the note and then deleting it, which writes the text into every member's log for good.
- Topics: migration, nocentraldb, personal-data

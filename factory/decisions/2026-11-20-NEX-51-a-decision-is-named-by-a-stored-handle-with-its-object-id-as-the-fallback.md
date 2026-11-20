# 2026-11-20 · NEX-51 · A decision is named by a stored handle, YYYY-MM-DD-<ticket>-<slug>, with its object id as the fallback

- Decided by: Carsten
- Why: a handle reads in a prompt, and every file in factory/decisions/ and every v1 memory keeps its name through the move, so `Supersedes:` lines and references in texts still lead to it. Two decisions written concurrently with the same date, ticket and title can share a handle; a command then refuses it and names the object ids. Each index line ends with the handle, or with the object id where the handle is shared. RFC 0003, "Handles", option (a).
- Instead of: the object id only, or a short prefix of it as git abbreviates commits: never or rarely ambiguous, but unreadable, and the moved files would lose their names.
- Topics: decisions, rfc, cli

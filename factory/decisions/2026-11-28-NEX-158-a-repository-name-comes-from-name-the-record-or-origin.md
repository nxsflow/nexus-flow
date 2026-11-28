# 2026-11-28 · NEX-158 · A repository's name is --name, else its record's, else origin's, and a name two records hold is ambiguous

- Decided by: coder on NEX-158
- Why: RFC 0009 takes the name from the last path segment of `origin` without `.git`, and has a join run again with `--name` rename it. So a join without `--name` keeps the name the repository's record has, and only a new record takes `origin`'s; the segment is what follows the url's last `/` or `:`, after a trailing `/`. A checkout without `origin` is refused with a message to give `--name`, not named after its folder. A name is any text without a space or a control character. The name check that addressing needs, `repository_name_ambiguous` with the matching repositories, is built now as the store's `requests::named`, and the CLI maps it; `nxs request create --to` will be its first caller.
- Instead of: the folder's name without `origin`, which guesses at a name the group addresses requests by; or renaming the record to `origin`'s on every join, which undoes a `--name`.
- Topics: coordination, store, v2

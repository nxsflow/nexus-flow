# 2026-11-21 · NEX-153 · A secret pattern matches a whole token, not its prefix alone

- Decided by: coder on NEX-153
- Why: RFC 0007 lists the patterns by their prefixes but not where a match ends, and a note may well say "classic tokens start with ghp_". So a GitHub, Slack or Linear prefix counts only with at least 20 token characters after it, which every real token has; an AWS key id is a whole word of exactly 20 upper-case letters or digits; a JSON web token is three non-empty base64url parts whose first starts `eyJ` at the start of a word; and a private key block is any `-----BEGIN <label>-----` whose label holds `PRIVATE KEY`, so PGP's `PRIVATE KEY BLOCK` is caught too. When a text matches several patterns, the first in the RFC's order is named. The scan uses no `regex` dependency.
- Instead of: matching the bare prefix, which refuses notes that only talk about tokens; or each provider's exact token length, which misses a token a provider lengthens.
- Topics: status, secrets

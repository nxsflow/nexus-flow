# 2026-11-21 · NEX-153 · The secret patterns are named in snake case after what they find

- Decided by: Carsten
- Why: RFC 0007 says `secret_in_note` names the pattern but does not name the patterns, and agents and apps read the name in the error. The six names are `private_key`, `github_token`, `aws_access_key_id`, `slack_token`, `json_web_token` and `linear_api_key`: snake case, like the error codes, and named after the kind of secret, not after a prefix. Approved in NEX-218, question 2.
- Instead of: names after the prefixes (`ghp`, `akia`, `xox`), which a pattern with several prefixes cannot carry; or numbered patterns, which an agent cannot read.
- Topics: status, secrets, errors

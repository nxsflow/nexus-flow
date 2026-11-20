# 2026-11-20 · NEX-8 · nxs genesis create installs the GENESIS guide as factory/genesis-guide.md, under RFC 0006's rules for shipped defaults

- Decided by: Carsten
- Why: the guide is nexus-flow's text, which every agent reads locally and a team rarely changes, so it is written when missing, recognised by the hashes in `templates/genesis/shipped.toml`, and replaced only with `--update` while unchanged; `--update` never touches `factory/GENESIS.md` (RFC 0004, "Installing the template"). Answer (a) in NEX-71.
- Instead of: the guide as comments inside the template, or only in nexus-flow's online documentation.
- Topics: genesis, templates, cli

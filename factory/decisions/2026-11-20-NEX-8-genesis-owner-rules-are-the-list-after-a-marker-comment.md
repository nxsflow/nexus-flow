# 2026-11-20 · NEX-8 · The rules in GENESIS on what comes back to the owner are the list after a marker comment

- Decided by: Carsten
- Why: the merge gate's script must find the rules the same way every time, whatever the team calls the section and in whatever language; `<!-- nexus-flow: owner-rules -->` on the line before the list sits in the file and the diff the owner reviews and needs no setting, and a lost or doubled marker reserves every pull request for the owner (RFC 0004, "The rules on what comes back to the owner"). Answer (a) in NEX-70.
- Instead of: a fixed heading the template uses, or the heading named in `nexus-flow.toml`.
- Topics: genesis, merge-gate, owner

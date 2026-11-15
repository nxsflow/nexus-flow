# 2026-11-15 · NEX-14 · A team's GENESIS and factory files stay in git, and the files are the truth

- Decided by: Carsten
- Why: the owner's review of GENESIS, roles, FACTORY, CONFIG and merge rules runs through PRs and CODEOWNERS, and the factory rests on that gate; `nxs` reads them from the working tree. A mirror into the repository space can follow when manufakt.io or a coordination space needs to read them.
- Instead of: records in NoCentralDB as the truth, which would need the owner's approval rebuilt inside NoCentralDB.
- Topics: nocentraldb, genesis, factory, storage

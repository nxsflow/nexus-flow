# 2026-11-24 · NEX-137 · The install reads every shipped path and CODEOWNERS before it writes

- Decided by: coder on NEX-137
- Why: RFC 0006 says what `install` writes but not what happens when a read fails partway. `install::factory::install(root, &defaults, update)` reads every path the shipped list names, then the CODEOWNERS file, and writes only after both, so a read that fails writes nothing. A link at a shipped path counts as present and is not replaced unless it is an earlier default and `--update` is given; a new file is created with `create_new`, so a file that appeared since the read, or a dangling link, is not written through. The function takes its defaults as an argument, so tests can install defaults with earlier and retired versions, and `nxs init` (NEX-139) can call it. A seam error outside a repository is `FactoryError::Root`, printed as `no_workspace`; one reading or writing a file is `FactoryError::Install`, printed as `io`. The form of `Engine::factory_install` is the owner's, in NEX-232.
- Instead of: writing each file as soon as its state is known, which leaves a half-installed factory when a later read fails.
- Topics: factory, seam, v2

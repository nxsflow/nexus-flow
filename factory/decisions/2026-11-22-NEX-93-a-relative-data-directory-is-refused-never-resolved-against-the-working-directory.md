# 2026-11-22 · NEX-93 · A relative data directory is refused, never resolved against the working directory

- Decided by: coder on NEX-93
- Why: RFC 0001, "The local data on disk", keeps the member databases outside the repository, and a relative path would resolve against the working directory, which is usually a working tree. So a relative `NXS_DATA_DIR` or `HOME` is an error that names the variable, and a relative `XDG_DATA_HOME` is ignored, as the XDG base directory spec says. An empty variable counts as unset. A session id, member id or run that is not one plain name (`..`, `a/b`, empty) is refused too, so no folder of the layout lies outside its parent.
- Instead of: resolving a relative path against the working directory, or ignoring a relative `NXS_DATA_DIR` silently and falling back to the platform default.
- Topics: storage, nocentraldb, v2

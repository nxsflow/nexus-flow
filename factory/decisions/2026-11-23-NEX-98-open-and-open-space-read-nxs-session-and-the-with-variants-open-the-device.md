# 2026-11-23 · NEX-98 · open and open_space read NXS_SESSION, and the _with variants open the device

- Decided by: coder on NEX-98
- Why: `Engine::open` and `Engine::open_space` read the environment already (`NXS_DATA_DIR`), so they also read `NXS_SESSION`, first, and run as the session's member when it names one. They never touch the device's database or key store then, so a session works on a machine where the device has none. `open_with` and `open_space_with` take explicit `Options` and open the device's member, and `as_session` switches any engine to a session. This keeps the owner-approved `Options { data, keys }` unchanged. An empty `NXS_SESSION` counts as unset, as an empty `NXS_DATA_DIR` does.
- Instead of: a `session` field in `Options`, which would change a seam type the owner approved in NEX-96; or opening the device first and then switching with `as_session`, which would need the device's keys on every session machine.
- Topics: seam, sessions, v2

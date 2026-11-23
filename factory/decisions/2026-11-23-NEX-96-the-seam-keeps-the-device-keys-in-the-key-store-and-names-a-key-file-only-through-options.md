# 2026-11-23 · NEX-96 · The seam keeps the device's keys in the key store, and names a key file only through Options

- Decided by: coder on NEX-96
- Why: NEX-94 left to the seam which `DeviceKeys` it passes. `Engine::open` and `Engine::open_space` keep the RFC's signatures and pass `DeviceKeys::KeyStore` with `DataDir::from_env`; a caller with no key store, or one that keeps its device elsewhere, such as manufakt.io or a test, names both in `Options` and calls `Engine::open_with` or `Engine::open_space_with`. No environment variable chooses a key file: RFC 0001 names only `NXS_SESSION` and `NXS_DATA_DIR`, and a new one is a public surface for the owner. Both opens are `async`, since the SDK's open is, and the seam brings no runtime of its own.
- Instead of: a key file chosen by a new environment variable, or `Engine::open` taking the options as a second parameter, which would change the RFC's signature.
- Topics: seam, storage, nocentraldb, v2

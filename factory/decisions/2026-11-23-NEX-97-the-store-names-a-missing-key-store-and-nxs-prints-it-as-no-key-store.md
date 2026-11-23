# 2026-11-23 · NEX-97 · The store names a missing key store, and nxs prints it as no_key_store

- Decided by: coder on NEX-97, the open.rs change approved by pm
- Why: NEX-224 has `nxs init` refuse on a machine with no key store, with an error that says so. The store folded every SDK error into `InitError::Database`, losing NoCentralDB's `KeyStoreUnavailable` code, and the SDK's own message tells the caller to name a key file, which `nxs` never offers. So `InitError` gains `KeyStoreUnavailable`, with no message of the SDK's: its text says the machine has no key store `nxs` can reach and that `nxs` keeps the device's keys only there. `nxs` prints it with the code `no_key_store`, the one code v1 does not have: none of v1's says what is wrong here, and an agent must tell it from a disk failure (`io`) without reading the message.
- Instead of: matching the SDK's message text in `nxs-v2`, which breaks at the next pin move; or `io`, which hides a setup problem among disk failures.
- Topics: v2, storage, output

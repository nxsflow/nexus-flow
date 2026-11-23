# 2026-11-23 · NEX-94 · The device's keys stay in the platform key store unless the caller names a key file

- Decided by: coder on NEX-94
- Why: RFC 0001, "Members", keeps a device's keys in its key store, and NoCentralDB never falls back to a key file the app did not name, so `DeviceKeys::KeyStore` is the store's default and a machine without a key store, such as a test or a headless container, must say `DeviceKeys::File(path)` itself. Which of the two `nxs` passes, and from what setting, is the seam's call (NEX-96), not the store's.
- Instead of: a key file beside the database by default, which only file permissions protect.
- Topics: storage, nocentraldb, v2

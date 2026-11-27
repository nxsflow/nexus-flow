# 2026-11-24 · NEX-126 · A write-once object takes all its fields in its create

- Decided by: coder on NEX-126
- Why: R12 (RFC 0003, "Requirements") lets a write-once object take field writes in the change that creates it. The store's `Write::create` takes every field at once, and the SDK returns the new id in the same change, so the topic, subject and supersession objects link to their decision in that change without a later set. The guard therefore refuses every `update`, `delete` and `restore` of a write-once collection, without tracking which objects the change itself creates.
- Instead of: allowing a set of an object created earlier in the same change, which needs the guard to track created ids and gives no writer anything `create` does not.
- Topics: store, decisions, v2

# 2026-11-28 · NEX-241 · Only the creation of the space writes the owner's record

- Decided by: coder on NEX-241
- Why: RFC 0001 has `nxs init` write the record of the device that creates the space, with role `owner` and a free tag (NEX-240). The store's `space::init` writes it right after it creates the space and its marker, through the store's one write path, and before `nexus-flow.toml` names the space, so a failed write leaves no file naming a space without the record. A run whose `nexus-flow.toml` already names a space writes nothing: the device running it may be another developer's, which is not the owner, and a member that created the space cannot be told apart from one added later. So `init` run twice writes no second record. A space created before this change keeps its device without a record, and `nxs member retag` still gives it a tag.
- Instead of: writing the record on every run for a device that has none, which would make a second developer's device `owner`; or writing it after `nexus-flow.toml`, which leaves a named space without the record when the write fails.
- Topics: init, members, store, v2

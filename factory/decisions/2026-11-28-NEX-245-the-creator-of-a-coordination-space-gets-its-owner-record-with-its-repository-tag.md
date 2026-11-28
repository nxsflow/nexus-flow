# 2026-11-28 · NEX-245 · The creator of a coordination space gets its owner record with its repository tag

- Decided by: coder on NEX-245
- Why: RFC 0001's "Roles in a space" says nothing on tags; RFC 0009, "Request ids", gives every member of the coordination space a tag under RFC 0002's rules, and RFC 0002 has the device that creates a space get its `owner` record with a free tag (NEX-240), so the creator of a coordination space needs one to mint goal and request keys. `nxs member add` gives a member one tag in every space, so the creator takes the tag its record in the repository space carries, and a free tag picked by its id only when it has none there. The store's `requests::create` writes the record right after it creates and opens the space, before the repository record and `nexus-flow.toml`, through the same `members::write_owner` that `nxs init` uses, so only the creation writes it and a join writes none.
- Instead of: no tag in the coordination space, which leaves its creator unable to mint a goal or request key; or a free tag picked anew, which could give the device two tags across its spaces where `nxs member add` gives one.
- Topics: coordination, members, tags, v2

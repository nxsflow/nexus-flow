# 2026-11-28 · NEX-242 · Only the creation of a space writes its marker

- Decided by: coder on NEX-242
- Why: the marker decides which writes a space takes, so a write that adds a second marker or changes the first would change the space's kind. The store writes it in `space_marker::create`, in the change right after NoCentralDB creates the space (the SDK creates a space and writes into it in two calls), and `Opened::write` refuses every create, set, delete and restore of `spaceKind` with `WriteError::Marker`. `nxs init` creates its space this way, and `space::create` makes a marked space of a kind its caller names, for the coordination and ad-hoc spaces, whose commands do not exist yet. The stored values are RFC 0001's words: `repository`, `coordination`, `ad-hoc`; an index `byKind` finds a space's markers.
- Instead of: letting the marker through the write path like any frame collection, which lets any caller rewrite a space's kind; or declaring it write-once only, which still lets a caller add a second marker.
- Topics: spaces, store, model, v2

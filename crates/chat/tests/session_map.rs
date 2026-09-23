use nexus_chat::error::ErrorKind;
use nexus_chat::store::ChatStore;

#[test]
fn pending_bind_resolve_role() {
    let mut s = ChatStore::open_in_memory(1);
    s.create_pending_session("s-1", "coding").unwrap();
    assert_eq!(s.resolve_real("s-1").unwrap(), None);
    assert_eq!(s.session_role("s-1").unwrap().as_deref(), Some("coding"));
    s.bind_session("s-1", "real-abc").unwrap();
    assert_eq!(s.resolve_real("s-1").unwrap().as_deref(), Some("real-abc"));
    // Test Quality #4: tightened from a bare `.is_err()` to the specific error KIND — an unknown
    // `internal_id` must map to `not_found` (mirrors `session_bind_of_an_unknown_internal_id_is_
    // not_found` in `tests/verbs.rs`, which already checks this at the CLI layer).
    let err = s.bind_session("s-nope", "x").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

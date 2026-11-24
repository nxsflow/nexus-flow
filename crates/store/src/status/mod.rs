//! Daily status notes in the repository space (RFC 0007).

/// The status module's write-once collections (RFC 0007, "Correcting a note"): a note is
/// corrected by a new note, never rewritten, and its lines and its goals' verdicts are written
/// once, in the same change as the note.
pub const WRITE_ONCE: &[&str] = &["statusNote", "statusItem", "statusGoal"];

pub mod secrets;

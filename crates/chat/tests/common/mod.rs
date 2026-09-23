//! Shared test harness for the chat integration suites.
//!
//! This module is compiled into each integration-test binary that declares `mod common;`, so not
//! every helper is used by every binary — allow the resulting dead-code noise rather than annotate
//! each item.
#![allow(dead_code)]

use std::path::Path;

use nexus_chat::definitions::Definitions;
use nexus_chat::workspace::PERSONAS_DIR;

/// Write a [`Definitions`] into a workspace's `.nxs-personas/` folder, in the shape the loaders
/// read it back from.
///
/// **Why this exists: nxf 6j6v.dvyq step 6 removed the injection path.** `EngineConfig` used to
/// take a `DefinitionSource::Supplied(defs)` and an app could hand a catalogue straight to the
/// handle; `Engine::set_definitions` could then replace it mid-flight. Both are gone — personas and
/// channels come from the folder, for an app as much as for the CLI, so the only way to give a test
/// workspace a team is to declare one in it. This is that, in one place, so forty call sites did
/// not each grow their own YAML string.
///
/// **This is not a shortcut around the folder — it IS the folder.** Every catalogue written here
/// goes through the same `serde_yaml` shape `role::load_all_roles` / `channel::load_all_channels`
/// parse, so a test that used to inject a `Definitions` now proves the same behaviour over the real
/// read path. Round-tripping is deliberate: if a declaration
/// stopped surviving the file, these suites would notice, where injection could never have.
///
/// Layout, matching what the loaders expect:
///
/// * one `<handle>.yaml` per persona;
/// * `channels.yaml` — a YAML LIST of channel declarations (written only when there is one, so an
///   empty catalogue leaves an empty folder rather than an empty list file).
pub fn write_declarations(root: &Path, defs: &Definitions) {
    let dir = root.join(PERSONAS_DIR);
    std::fs::create_dir_all(&dir).expect("create the declaration folder");
    for role in defs.roles() {
        write_yaml(&dir.join(format!("{}.yaml", role.handle)), role);
    }
    if !defs.channels().is_empty() {
        write_yaml(&dir.join("channels.yaml"), &defs.channels());
    }
}

/// Replace a workspace's declarations wholesale: clear the folder first, then write.
///
/// The mid-flight edit `Engine::set_definitions` used to perform, done the way it is actually done
/// now — a user (or a role editor in an app) changes the files, and the next OPERATION reads them.
///
/// "The next operation" and not "the next verb" since nxf 6j6v.n92p: a chain already running is
/// bound to the declarations it opened under, which is why several suites call this MID-FLIGHT and
/// then assert that nothing changed.
pub fn replace_declarations(root: &Path, defs: &Definitions) {
    let dir = root.join(PERSONAS_DIR);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear the declaration folder");
    }
    write_declarations(root, defs);
}

/// Write one declaration file ATOMICALLY: into a sibling temp file, then `rename` over the target.
///
/// A plain `fs::write` truncates first, so a reader that happens to look in that instant sees an
/// empty declaration — and the engine now re-reads the folder on EVERY verb, so "that instant" is
/// reachable by any concurrent call. `rename` within one directory is atomic on POSIX, which makes
/// a reader see either the old file or the new one and never half of either. The temp name carries
/// the thread id and a counter so two writers never collide on it.
fn write_yaml<T: serde::Serialize>(path: &Path, value: &T) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let yaml = serde_yaml::to_string(value).expect("a declaration serializes");
    let tmp = path.with_extension(format!(
        "tmp-{:?}-{}",
        std::thread::current().id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, yaml).unwrap_or_else(|e| panic!("writing {}: {e}", tmp.display()));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("renaming onto {}: {e}", path.display()));
}

// ---- reads that left the handle in nxf 6j6v.yr59 ----------------------------
//
// `Engine::messages`, `Engine::channels`, `Engine::inbox`, `Engine::threads`, `Engine::thread_board`
// and `Engine::transcript` are gone from the seam; their `facade::` bodies are not, because the CLI
// verbs that reach them are still verbs. A suite that asserts "what is in this channel" or "what is
// unread" is asking a question the seam no longer offers as a method, and it still has to be able
// to ask it — otherwise the cut would take coverage of the WRITES with it.
//
// So these open the workspace's own store and read it there, which is what `cli.rs` does. They are
// deliberately NOT wrappers pretending to be seam calls: a test that needs the SEAM uses
// `Engine::thread`/`status`/`directory`/`prime_as`, and a test that only needs to look at what a
// write left behind uses one of these.

/// One channel's messages, read off the workspace store (`Engine::messages`' body).
pub fn channel_messages(
    root: &Path,
    channel: &str,
    as_handle: &str,
) -> nexus_chat::error::Result<Vec<nexus_chat::facade::MessageView>> {
    nexus_chat::facade::messages(&open_store(root), channel, as_handle)
}

/// One handle's channels (`Engine::channels`' body).
pub fn channels_of(
    root: &Path,
    handle: &str,
) -> nexus_chat::error::Result<Vec<nexus_chat::facade::ChannelView>> {
    nexus_chat::facade::channels(&open_store(root), handle)
}

/// The caller's quorum boards (`Engine::threads`' body).
pub fn boards_of(
    root: &Path,
    consumer: &str,
    now: &str,
    channel: Option<&str>,
) -> nexus_chat::error::Result<Vec<nexus_chat::facade::ThreadListEntry>> {
    nexus_chat::facade::threads(&open_store(root), consumer, now, channel)
}

/// One board in full (`Engine::thread_board`' body). `visibility` is explicit, as it always was at
/// this layer; a suite whose channel is not a declared one passes `AllMembers`, which is what the
/// adapter above resolves for it.
pub fn board_of(
    root: &Path,
    thread_id: &str,
    now: &str,
    as_handle: &str,
    visibility: nexus_chat::channel::Visibility,
) -> nexus_chat::error::Result<nexus_chat::facade::ThreadBoardView> {
    nexus_chat::facade::thread_board(&open_store(root), thread_id, now, as_handle, visibility)
}

/// The bulk quorum read for an explicit set (`Engine::thread_quorums`' body, now
/// `StatusScope::Threads` on the seam).
pub fn quorums_of(
    root: &Path,
    thread_ids: &[&str],
    now: &str,
) -> nexus_chat::error::Result<Vec<nexus_chat::store::ThreadQuorum>> {
    nexus_chat::facade::thread_quorums(&open_store(root), thread_ids, now)
}

/// A whole session's transcript (`Engine::transcript`' body; the seam offers the WINDOWED read,
/// `transcript_page(s, -1, None)`, which is the same value).
pub fn whole_transcript(
    root: &Path,
    internal_session: &str,
) -> nexus_chat::error::Result<nexus_chat::facade::TranscriptView> {
    nexus_chat::facade::transcript(&open_store(root), internal_session)
}

/// The workspace's own chat store, opened fresh — a second connection beside whatever handle the
/// test holds, which is exactly what a `nxc` subprocess is.
pub fn open_store(root: &Path) -> nexus_chat::store::ChatStore {
    use nexus_chat::workspace::ChatWorkspaceExt;
    nexus_chat::workspace::Workspace::resolve(None, root)
        .expect("resolve the workspace")
        .open_chat_store()
        .expect("open the chat store")
}

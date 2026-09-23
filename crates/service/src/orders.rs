//! **Which orders this machine has taken** (nxf 6j6v.1c6k) — the claims that make a persona chat
//! run once per message on the machine it is designated to.
//!
//! A signed order proves where it came from, never that it is new (`docs/specs/E4-auth-identity-
//! and-signed-ops.md` §2.5), so the machine that executes it must take it exactly once. The claim
//! lives HERE, in the service home, and not in a workspace: one machine can hold two replicas of
//! one stream (two clones of a repository derive the same stream id), both pull the same order,
//! and a claim per workspace would start it twice (6j6v.f0b5, note 3).
//!
//! `<home>/orders/` holds two kinds of file, each created with an exclusive create, so exactly one
//! process on this machine wins it whichever asks first:
//!
//! - `msg-<message id>` — this machine took the message. The local `nxc send`/`nxc reply` takes it
//!   too, which is what stops the service picking up what was just started here.
//! - `thread-<thread id>` — which replica of this machine serves the chat (its workspace db path,
//!   the file's content). Every later turn goes to that replica, where the persona's session is.
//!
//! A file per message, never pruned in this slice: a few bytes per picked-up message. Pruning, and
//! recovering a claim whose start never happened (a pickup process that died between claiming and
//! starting), are `6j6v.avwz`.

use std::path::PathBuf;

use nxs_foundation::error::{NxfError, Result};

use crate::home::ServiceHome;

impl ServiceHome {
    /// `<home>/orders/` — this machine's claims.
    pub fn orders(&self) -> PathBuf {
        self.root().join("orders")
    }

    /// Claim `message_id` for execution on this machine: `true` exactly once, for the first caller.
    pub fn claim_order(&self, message_id: &str) -> Result<bool> {
        crate::machine::create_new(&self.order_file("msg", message_id), "", &unique())
    }

    /// Whether `message_id` has been claimed on this machine.
    pub fn order_claimed(&self, message_id: &str) -> bool {
        self.order_file("msg", message_id).exists()
    }

    /// Give a claim back — the start it was taken for failed, and the next pickup should try again.
    /// A claim that is already gone is not an error; one that cannot be removed IS, because it
    /// keeps that message from ever being picked up on this machine again (review of PR #492,
    /// Integrity #1).
    pub fn release_order(&self, message_id: &str) -> Result<()> {
        let path = self.order_file("msg", message_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(NxfError::io(format!(
                "could not give back the claim {} — that message will not be picked up on this \
                 machine until the file is removed: {e}",
                path.display()
            ))),
        }
    }

    /// Which replica serves `thread_id` on this machine: the first `holder` to claim it keeps it,
    /// and that holder is what every caller gets back.
    pub fn claim_thread(&self, thread_id: &str, holder: &str) -> Result<String> {
        let path = self.order_file("thread", thread_id);
        if crate::machine::create_new(&path, holder, &unique())? {
            return Ok(holder.to_string());
        }
        std::fs::read_to_string(&path)
            .map_err(|e| NxfError::io(format!("reading {}: {e}", path.display())))
    }

    fn order_file(&self, kind: &str, id: &str) -> PathBuf {
        self.orders().join(format!("{kind}-{}", file_safe(id)))
    }
}

/// An id as a file name: ids are ULID-shaped (`m-01j…`, `<prefix>-…`), which pass through
/// unchanged; anything else is hex-encoded, so no id can name a path outside the directory.
fn file_safe(id: &str) -> String {
    let plain = !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if plain {
        id.to_string()
    } else {
        let hex: String = id.bytes().map(|b| format!("{b:02x}")).collect();
        format!("x{hex}")
    }
}

fn unique() -> String {
    ulid::Ulid::new().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn home() -> (TempDir, ServiceHome) {
        let dir = TempDir::new().unwrap();
        let home = ServiceHome::at(dir.path().join(".nexusflow"));
        (dir, home)
    }

    #[test]
    fn a_message_is_claimed_once_and_can_be_given_back() {
        let (_d, home) = home();
        assert!(!home.order_claimed("m-01jabc"));
        assert!(home.claim_order("m-01jabc").unwrap());
        assert!(!home.claim_order("m-01jabc").unwrap());
        assert!(home.order_claimed("m-01jabc"));
        home.release_order("m-01jabc").unwrap();
        home.release_order("m-01jabc")
            .expect("a claim already gone is not an error");
        assert!(home.claim_order("m-01jabc").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_claim_that_cannot_be_given_back_is_an_error_not_a_silence() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, home) = home();
        assert!(home.claim_order("m-stuck").unwrap());
        let dir = home.orders();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let released = home.release_order("m-stuck");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let err = released.expect_err("a claim that stays behind blocks the message for good");
        assert!(err.msg.contains("will not be picked up"), "{}", err.msg);
        assert!(home.order_claimed("m-stuck"));
    }

    #[test]
    fn a_thread_keeps_the_replica_that_claimed_it_first() {
        let (_d, home) = home();
        assert_eq!(
            home.claim_thread("m-t", "/a/.nxs/db").unwrap(),
            "/a/.nxs/db"
        );
        assert_eq!(
            home.claim_thread("m-t", "/b/.nxs/db").unwrap(),
            "/a/.nxs/db"
        );
    }

    #[test]
    fn two_homes_on_one_computer_are_two_machines_with_their_own_claims() {
        let dir = TempDir::new().unwrap();
        let prod = ServiceHome::at(dir.path().join(".nexusflow"));
        let dev = ServiceHome::at(dir.path().join(".nexusflow-dev"));
        assert!(prod.claim_order("m-1").unwrap());
        assert!(dev.claim_order("m-1").unwrap());
    }

    #[test]
    fn an_id_that_is_not_a_plain_name_cannot_leave_the_directory() {
        let (_d, home) = home();
        assert!(home.claim_order("../../etc/passwd").unwrap());
        let names: Vec<String> = std::fs::read_dir(home.orders())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("msg-x"), "{names:?}");
    }
}

//! Workspace resolution is foundation-owned (the `.nxs/` directory belongs to the platform, not
//! chat); this module re-exports it so `nexus_chat::workspace::*` paths resolve, and adds the chat
//! half: opening the *chat* store (registers the message reducer + creates the six views) and
//! building chat's foundation config. Mirrors flow's/memory's `WorkspaceExt`.

pub use nxs_foundation::workspace::*;

use crate::error::{NxfError, Result};
use crate::store::ChatStore;
use std::path::{Path, PathBuf};

/// chat's module key in the foundation config's `active_modules` (spec §7). The `nxc init` that
/// *activates* it is T3; T2 only needs the constant + store-open + a fresh-workspace config.
pub const CHAT_MODULE: &str = "chat";

/// The MINTING WORKSPACE IDENTITY (spec §2.2) every message, thread and channel written in `ws` is
/// stamped with, and the `<origin>` half of every qualified handle `<origin>/<agent>` minted there:
/// **the workspace's own replica prefix**.
///
/// **A per-workspace value, and M1 says so in as many words**: "in M1 it is the local workspace
/// identity"; the org/repo-qualified form was left to the federation bridge (nxf 6j6v.kz8p).
/// One string per workspace, the same for every caller in it.
///
/// **The workspace's NAME is not this** (nxf 6j6v.q32p). The `<owner>/<repo>` an address from
/// another workspace starts with is stored beside the replica (`nxs_foundation::workspace_name`),
/// and the origin stays the prefix: a message copied across the border keeps the origin its author
/// signed, and a status row maps the other side to its name through the border record
/// (`crate::border`), never by rewriting a handle.
///
/// **Written once because BOTH seams read it** (nxf 6j6v.07me). `cli.rs`'s `origin()` falls back to
/// it when `NXC_ORIGIN` is unset, and [`crate::engine::Engine::origin`] answers with it for every
/// call through the handle — so the two seams cannot end up naming the same workspace differently,
/// which is the failure that would make an app's `<prefix>/coder` and a terminal's `local/coder`
/// two different members of one declared channel. `tests/parity.rs`'s
/// `both_adapters_resolve_one_origin_for_one_workspace` is the gate on that agreement; there is no
/// second derivation of this rule anywhere for it to drift from.
///
/// **It was the constant `"local"` until the owner ruled otherwise** (2026-08-21, on 6j6v.07me):
/// the first build kept the CLI's old default for behaviour-equality, and the owner chose the
/// replica prefix instead, following the item's own wording — `Engine::workspace()` already holds
/// the replica identity, so a workspace-independent literal made every workspace on a machine mint
/// under one name. The user-visible consequence is that an agent's qualified handle is
/// `<prefix>/coder`, not `local/coder`.
///
/// **The prefix is REASSIGNABLE, and that hazard is accepted rather than solved here.**
/// `foundation::workspace::adopt_prefix` rewrites `replica.prefix` when sync finds a colliding one,
/// and every `<prefix>/<agent>` already written then names a workspace identity that no longer
/// exists. The owner took that knowingly; it is tracked as nxf 6j6v.dnzt. Do not reach for
/// `replica_uuid` here to dodge it — that is 26 characters no surface ever shows, and the trade was
/// made with both sides on the table.
pub fn origin_of(ws: &Workspace) -> &str {
    &ws.replica.prefix
}

/// Where a workspace's persona/channel/flow declarations belong: `<workspace-root>/.nxs-personas`,
/// sibling to the `.nxs/` directory itself (nxf 6j6v.dvyq — THE break of that item).
///
/// The dot prefix and the product name are both deliberate: the folder is nxsflow's own, it sits in
/// the USER's repository next to `.nxs/`, and a bare `roles/` at a project root is a name any
/// project might already have taken for something else entirely.
pub const PERSONAS_DIR: &str = ".nxs-personas";

/// The LEGACY declaration folder, `<workspace-root>/roles`, which this workspace still READS (and
/// reports) but never writes (nxf 6j6v.dvyq step 4). See [`crate::definitions::DeclarationSource`]
/// for the resolution order and for why the engine does not migrate the folder itself.
pub const LEGACY_ROLES_DIR: &str = "roles";

/// Opens chat's [`ChatStore`] over a resolved [`Workspace`]. WAL mode lets parallel `nxc` processes
/// serialize their short write transactions instead of failing with "database is locked". Bring this
/// trait into scope to call `ws.open_chat_store()`.
pub trait ChatWorkspaceExt {
    fn open_chat_store(&self) -> Result<ChatStore>;

    /// The project root this workspace sits in: `Workspace.dir` IS the `.nxs/` directory (confirmed
    /// by `Workspace::db_path` joining `db.sqlite` straight onto `dir`), so the root is its parent.
    fn workspace_root(&self) -> Result<&Path>;

    /// Where a declaration BELONGS in this workspace: `<workspace-root>/.nxs-personas`
    /// ([`PERSONAS_DIR`]). This is the answer every surface renders when there is nothing declared
    /// — it names the place, not what happens to be there today, so it is well-defined even for a
    /// workspace that has no declaration folder at all.
    fn personas_dir(&self) -> Result<PathBuf>;

    /// The legacy `<workspace-root>/roles` folder ([`LEGACY_ROLES_DIR`]) — the location
    /// declarations lived at before 6j6v.dvyq. Resolved, not necessarily existing.
    fn legacy_roles_dir(&self) -> Result<PathBuf>;

    /// The workspace's OWN folder declarations are read from — only the workspace's half since nxf
    /// 6j6v.7k58: the catalogue also merges the user-level folder, so a reader that wants the team
    /// resolves [`crate::definitions::Definitions::resolve`] rather than loading this folder alone.
    /// Resolved over both workspace locations — see
    /// [`crate::definitions::DeclarationSource::locate`], which owns the rule. Shared by every
    /// declared-persona/channel/flow loader, so the CLI and [`crate::engine::Engine`] cannot
    /// resolve it differently (independent review, Integrity #3/Code Quality #1, High).
    fn declaration_dir(&self) -> Result<PathBuf>;
}

impl ChatWorkspaceExt for Workspace {
    fn open_chat_store(&self) -> Result<ChatStore> {
        let path = self.db_path_str()?;
        let store = ChatStore::open(&path, self.replica.site_id)
            .map_err(|e| NxfError::io(format!("opening chat store at {path}: {e}")))?;
        store
            .connection()
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| NxfError::io(format!("failed to set sqlite pragmas: {e}")))?;
        Ok(store)
    }

    fn workspace_root(&self) -> Result<&Path> {
        self.dir.parent().ok_or_else(|| {
            NxfError::io(format!(
                "workspace directory has no parent to resolve {PERSONAS_DIR}/ against"
            ))
        })
    }

    fn personas_dir(&self) -> Result<PathBuf> {
        Ok(self.workspace_root()?.join(PERSONAS_DIR))
    }

    fn legacy_roles_dir(&self) -> Result<PathBuf> {
        Ok(self.workspace_root()?.join(LEGACY_ROLES_DIR))
    }

    fn declaration_dir(&self) -> Result<PathBuf> {
        Ok(crate::definitions::DeclarationSource::locate(self.workspace_root()?).read_dir)
    }
}

/// Build the foundation config for a FRESH chat-only workspace: register `chat` as the sole active
/// module. When chat joins an EXISTING workspace, `foundation::setup` loads that config unchanged
/// (the `nxc init` that registers chat via `activate_module` is T3); this seeds only a brand-new
/// `.nxs/` (used by the T2 verb tests to stand up a workspace without the T3 `init` verb).
pub fn chat_config() -> WorkspaceConfig {
    WorkspaceConfig {
        active_modules: vec![CHAT_MODULE.to_string()],
        ..Default::default()
    }
}

/// chat's key, under its `[chat]` section, for the liveness policy (6j6v.pszd, design 6.1).
pub const LIVENESS_KEY: &str = "liveness";
/// Where the policy is read from, as the warnings name it.
const LIVENESS_FILE: &str = ".nxs/config.toml";

/// **The workspace's liveness policy** — the `[chat.liveness]` section of `.nxs/config.toml`
/// (6j6v.pszd, design 6.1): two knobs, not one named policy.
///
/// ```toml
/// [chat.liveness]
/// dead_resumes = 1              # 1 to 3: recovery attempts on one owed turn before a dead verdict escalates
/// notice_at_first_stop = false  # true: a non-escalating notice to the commissioner at the first stop
/// ```
///
/// The trial compares A = `3`/`false`, **B = `1`/`false` (the default when the section or a key is
/// absent)** and C = `3`/`true`; `2` is valid and has no letter, and `1`/`true` is a valid fourth
/// setting. It is **read at every look**: the CLI resolves it in every `nxs chat tick` process, so an
/// edit needs no service restart; an embedding app's [`crate::engine::Engine`] supplies the policy
/// of the workspace it loaded, for its lifetime. Nothing inside the ladder reads the file — the
/// ladder reads [`crate::orchestration::Ctx::liveness`]. `.nxs/` is device-local and not synced, so
/// the setting is per machine: the trial runs over three workspaces on this one machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LivenessPolicy {
    /// Recovery attempts on one owed turn before a dead verdict escalates: 1 to
    /// [`crate::ladder::MAX_RECOVERY_ATTEMPTS`].
    pub dead_resumes: u32,
    /// Post a non-escalating notice to the commissioner at the episode's first stop.
    pub notice_at_first_stop: bool,
    /// At least one key was invalid and its default was used — what rung E's summary repeats.
    pub fell_back: bool,
}

impl Default for LivenessPolicy {
    /// Policy B.
    fn default() -> Self {
        Self {
            dead_resumes: crate::ladder::DEFAULT_DEAD_RESUMES,
            notice_at_first_stop: false,
            fell_back: false,
        }
    }
}

impl LivenessPolicy {
    /// `A`, `B` or `C` for the three combinations the trial compares; `None` for any other pair.
    pub fn letter(self) -> Option<char> {
        match (self.dead_resumes, self.notice_at_first_stop) {
            (3, false) => Some('A'),
            (1, false) => Some('B'),
            (3, true) => Some('C'),
            _ => None,
        }
    }
}

/// [`liveness_policy`]'s answer: the policy in force together with what the read had to say about
/// the file — so a host that runs the look also reports it (the tick's receipt carries the
/// warnings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLiveness {
    pub policy: LivenessPolicy,
    /// One line per invalid value or unknown key: `<file>: [chat.liveness] <key> = <value> — <why>;
    /// the default <d> is used`.
    pub warnings: Vec<String>,
    /// Unknown keys under `[chat.liveness]`, verbatim (the liveness report lists them).
    pub unknown_keys: Vec<String>,
}

/// Read the liveness policy from a workspace's config (6j6v.pszd, design 6.1), in the shape of
/// flow's `flow_plugin`. **An invalid value never skips a look**: a value out of `1 to 3` or of the
/// wrong type, for either key, runs the DEFAULT for that key, sets
/// [`LivenessPolicy::fell_back`] and adds a warning naming the file, the key and the value; an
/// unknown key is listed and warned about, and changes nothing.
pub fn liveness_policy(config: &WorkspaceConfig) -> ResolvedLiveness {
    let mut out = ResolvedLiveness {
        policy: LivenessPolicy::default(),
        warnings: Vec::new(),
        unknown_keys: Vec::new(),
    };
    let Some(raw) = config
        .products
        .get(CHAT_MODULE)
        .and_then(|c| c.as_table())
        .and_then(|c| c.get(LIVENESS_KEY))
    else {
        return out;
    };
    let default = LivenessPolicy::default();
    let Some(section) = raw.as_table() else {
        out.policy.fell_back = true;
        out.warnings.push(format!(
            "{LIVENESS_FILE}: [chat.liveness] = {raw} — must be a table; the defaults \
             dead_resumes = {} and notice_at_first_stop = {} are used",
            default.dead_resumes, default.notice_at_first_stop
        ));
        return out;
    };
    let max = crate::ladder::MAX_RECOVERY_ATTEMPTS;
    for (key, value) in section {
        match key.as_str() {
            "dead_resumes" => match value
                .as_integer()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (1..=max).contains(n))
            {
                Some(n) => out.policy.dead_resumes = n,
                None => {
                    out.policy.fell_back = true;
                    out.warnings.push(format!(
                        "{LIVENESS_FILE}: [chat.liveness] dead_resumes = {value} — must be 1 to \
                         {max}; the default {} is used",
                        default.dead_resumes
                    ));
                }
            },
            "notice_at_first_stop" => match value.as_bool() {
                Some(b) => out.policy.notice_at_first_stop = b,
                None => {
                    out.policy.fell_back = true;
                    out.warnings.push(format!(
                        "{LIVENESS_FILE}: [chat.liveness] notice_at_first_stop = {value} — must be \
                         true or false; the default {} is used",
                        default.notice_at_first_stop
                    ));
                }
            },
            other => {
                out.unknown_keys.push(other.to_string());
                out.warnings.push(format!(
                    "{LIVENESS_FILE}: [chat.liveness] {other} is not a known key and is ignored"
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(section: &str) -> WorkspaceConfig {
        let text = format!("{section}\n");
        let mut cfg = chat_config();
        let table: toml::value::Table = toml::from_str(&text).unwrap();
        cfg.products.extend(table);
        cfg
    }

    #[test]
    fn no_section_is_policy_b() {
        let r = liveness_policy(&chat_config());
        assert_eq!(r.policy, LivenessPolicy::default());
        assert_eq!(r.policy.letter(), Some('B'));
        assert!(r.warnings.is_empty());
        assert!(r.unknown_keys.is_empty());
        assert!(!r.policy.fell_back);
    }

    #[test]
    fn a_and_c_and_the_unlettered_pairs() {
        let a = liveness_policy(&with(
            "[chat.liveness]\ndead_resumes = 3\nnotice_at_first_stop = false",
        ))
        .policy;
        assert_eq!(
            (a.dead_resumes, a.notice_at_first_stop, a.letter()),
            (3, false, Some('A'))
        );
        let c = liveness_policy(&with(
            "[chat.liveness]\ndead_resumes = 3\nnotice_at_first_stop = true",
        ))
        .policy;
        assert_eq!(c.letter(), Some('C'));
        let two = liveness_policy(&with("[chat.liveness]\ndead_resumes = 2"));
        assert_eq!(
            (
                two.policy.dead_resumes,
                two.policy.letter(),
                two.warnings.len()
            ),
            (2, None, 0)
        );
        let one_true =
            liveness_policy(&with("[chat.liveness]\nnotice_at_first_stop = true")).policy;
        assert_eq!(
            (
                one_true.dead_resumes,
                one_true.notice_at_first_stop,
                one_true.letter()
            ),
            (1, true, None)
        );
    }

    #[test]
    fn an_invalid_value_runs_the_default_and_warns() {
        for bad in ["0", "4", "5", "\"three\"", "-1", "2.5"] {
            let r = liveness_policy(&with(&format!("[chat.liveness]\ndead_resumes = {bad}")));
            assert_eq!(r.policy.dead_resumes, 1, "{bad}");
            assert!(r.policy.fell_back, "{bad}");
            assert_eq!(r.warnings.len(), 1, "{bad}");
            assert!(
                r.warnings[0].contains(".nxs/config.toml")
                    && r.warnings[0].contains("dead_resumes")
                    && r.warnings[0].contains(bad.trim_matches('"'))
                    && r.warnings[0].contains("1 to 3"),
                "{}",
                r.warnings[0]
            );
        }
        let r = liveness_policy(&with("[chat.liveness]\nnotice_at_first_stop = 1"));
        assert!(
            !r.policy.notice_at_first_stop
                && r.policy.fell_back
                && r.warnings[0].contains("notice_at_first_stop"),
            "{r:?}"
        );
        // One bad key falls back for THAT key only: the other valid key still applies.
        let r = liveness_policy(&with(
            "[chat.liveness]\ndead_resumes = 3\nnotice_at_first_stop = \"yes\"",
        ));
        assert_eq!(
            (
                r.policy.dead_resumes,
                r.policy.notice_at_first_stop,
                r.policy.fell_back
            ),
            (3, false, true)
        );
        let r = liveness_policy(&with("[chat.liveness]\ndead_resume = 3"));
        assert_eq!(r.unknown_keys, vec!["dead_resume".to_string()]);
        assert_eq!(r.policy.dead_resumes, 1);
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("dead_resume"), "{}", r.warnings[0]);
    }

    #[test]
    fn a_liveness_that_is_not_a_table_runs_b_and_warns() {
        let r = liveness_policy(&with("[chat]\nliveness = 3"));
        assert_eq!(r.policy.dead_resumes, 1);
        assert!(r.policy.fell_back);
        assert_eq!(r.warnings.len(), 1);
        assert!(
            r.warnings[0].contains("[chat.liveness]"),
            "{}",
            r.warnings[0]
        );
    }

    #[test]
    fn chat_config_registers_only_chat() {
        assert_eq!(chat_config().active_modules, vec!["chat".to_string()]);
    }

    #[test]
    fn open_chat_store_materializes_views_over_a_fresh_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = setup(tmp.path(), &chat_config()).unwrap();
        let store = ws.open_chat_store().unwrap();
        // The `messages` view exists after open (the store applied the DDL + registered the reducer).
        let n: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}

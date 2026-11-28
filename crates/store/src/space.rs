//! Creating the repository space (RFC 0001, "Creating and finding a space", "Migration"): the
//! store half of `nxs init`. A repository whose `nexus-flow.toml` already names a space keeps
//! it, and the report names it; otherwise the device member creates the space as its first admin
//! and records its id in a new `nexus-flow.toml` at the root. Every space the store creates gets
//! the marker that names its kind (`crate::space_marker`): `init` a repository space, [`create`]
//! a space of the kind its caller names.
//!
//! A v1 workspace (`.nxs/`) beside it is never opened or written: the space starts empty, and
//! the report says the v1 data was not moved (RFC 0008 designs the move).

use std::path::Path;

use crate::config::Config;
use crate::data_dir::DataDir;
use crate::id::SpaceId;
use crate::member_select::RunAs;
use crate::open::{open_in, open_member, Network};
use crate::repo_root::{RepoRoot, RootError};
use crate::space_kind::SpaceKind;
use crate::space_marker;

pub use crate::open::{DeviceKeys, InitError};

/// v1's workspace folder at the repository's root.
const V1_WORKSPACE: &str = ".nxs";

/// How the device's database reaches other members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// No relay: the space is local to this device (RFC 0001, "Sync", **Island mode.**).
    Island,
    /// The device's settings name a relay, and the database syncs through it while it is open
    /// (RFC 0001, "Sync", **One relay per member database.**).
    Relay,
}

/// What `init` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// The repository space's id, as `nexus-flow.toml` names it.
    pub space: String,
    /// Whether this call created the space; `false` when the file already named one.
    pub created: bool,
    /// How the space syncs.
    pub sync: SyncMode,
    /// Whether the root holds a v1 workspace, `.nxs/`, which was left as it was.
    pub v1_workspace: bool,
}

impl InitReport {
    /// The report's lines, for a person to read.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.created {
            lines.push(format!(
                "created the repository space {} and recorded it in nexus-flow.toml; commit the file",
                self.space
            ));
        } else {
            lines.push(format!(
                "nexus-flow.toml already names the repository space {}; nothing was created",
                self.space
            ));
        }
        match self.sync {
            SyncMode::Island => lines.push(
                "no relay is set, so the space stays on this device in island mode; nxs sync --relay <url> connects it"
                    .to_owned(),
            ),
            SyncMode::Relay => lines.push(
                "the space syncs through the relay set on this device; nxs sync waits until nothing is behind or unconfirmed"
                    .to_owned(),
            ),
        }
        if self.v1_workspace {
            lines.push(format!(
                "the v1 data in {V1_WORKSPACE}/ was not moved into the space and is left as it was"
            ));
        }
        if self.created {
            lines.push(
                "this device is the space's only admin: if it is lost, nobody can add or remove members again; add a second admin device"
                    .to_owned(),
            );
        }
        lines
    }
}

/// Opens the device member's database and gives the repository at or above `dir` its space.
/// When no `nexus-flow.toml` is found from `dir` upward, `dir` becomes the root.
pub async fn init(dir: &Path, data: &DataDir, keys: DeviceKeys) -> Result<InitReport, InitError> {
    let (root, existing) = match RepoRoot::find(dir) {
        Ok(root) => {
            let config = Config::read(&root)?;
            (root, Some(config.repository))
        }
        Err(RootError::NotFound { .. }) => (RepoRoot::at(dir), None),
        Err(e) => return Err(e.into()),
    };
    let (db, relay) = open_in(&root, data, RunAs::Device(keys), Network::Relay).await?;
    let (space, created) = match existing {
        Some(space) => (space, false),
        None => {
            let space = space_marker::create(&db, SpaceKind::Repository)
                .await?
                .id()
                .to_string();
            Config::new(&space).write_new(root.path())?;
            (space, true)
        }
    };
    Ok(InitReport {
        space,
        created,
        sync: match relay {
            Some(_) => SyncMode::Relay,
            None => SyncMode::Island,
        },
        v1_workspace: root.path().join(V1_WORKSPACE).symlink_metadata().is_ok(),
    })
}

/// Creates a space of kind `kind`, with the device member as its first admin and the marker that
/// names its kind, and returns its id: a coordination space, or the ad-hoc space pm gives a role
/// session. [`init`] creates the repository space the same way.
pub async fn create(
    data: &DataDir,
    keys: DeviceKeys,
    kind: SpaceKind,
) -> Result<SpaceId, InitError> {
    let (db, _) = open_member(data, RunAs::Device(keys), Network::Relay).await?;
    Ok(SpaceId(space_marker::create(&db, kind).await?.id()))
}

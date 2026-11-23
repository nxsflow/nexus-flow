//! The device member's database, open on the spaces a caller reads (RFC 0001, "Reading and
//! writing", "The library seam"): what `Engine::open` and `Engine::open_space` hold.
//!
//! A repository opens on the spaces its `nexus-flow.toml` names, and has a checkout: the root
//! and the base branch "on main" is checked against. A space opened by id has a checkout only
//! when the caller names one, and without it every "on main" value is `null` ("Derived state").
//!
//! A named space the database does not hold yet, such as on a device added but not yet synced,
//! is open as not held: no read touches the network, so a read answers from what the database
//! holds, which is nothing of that space.

use std::fmt;
use std::path::{Path, PathBuf};

use nocentraldb::{Db, ErrorCode, Space};

use crate::config::Config;
use crate::data_dir::DataDir;
use crate::id::SpaceId;
use crate::open::{open_db, open_device, DeviceKeys, InitError};
use crate::repo_root::RepoRoot;

/// The local git a caller reads "on main" from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The repository's root, the folder that holds `nexus-flow.toml`.
    pub path: PathBuf,
    /// The team's base branch, from `[git] base`.
    pub base: String,
}

/// The device member's database, open on some spaces.
pub struct Opened {
    /// Kept open for as long as the spaces are: sync runs only while a database is open
    /// (RFC 0001, "Sync").
    _db: Db,
    spaces: Vec<(SpaceId, Option<Space>)>,
    checkout: Option<Checkout>,
}

impl Opened {
    /// Whether the space `id` is held: `None` when it is not one of the open spaces, `Some(false)`
    /// when it is but the database holds none of it yet.
    pub fn held(&self, id: &SpaceId) -> Option<bool> {
        self.spaces
            .iter()
            .find(|(open, _)| open == id)
            .map(|(_, space)| space.is_some())
    }

    /// The checkout "on main" is read from; `None` when there is none, so "on main" is `null`.
    pub fn checkout(&self) -> Option<&Checkout> {
        self.checkout.as_ref()
    }
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opened")
            .field("spaces", &self.spaces)
            .field("checkout", &self.checkout)
            .finish_non_exhaustive()
    }
}

/// Opens the device member's database for the repository at or above `dir`, on the spaces its
/// `nexus-flow.toml` names. Outside a repository the error says to run `nxs init`, and nothing
/// is created.
pub async fn repository(dir: &Path, data: &DataDir, keys: DeviceKeys) -> Result<Opened, InitError> {
    let root = RepoRoot::find(dir)?;
    let config = Config::read(&root)?;
    let mut ids = vec![parse(&config.repository, &root)?];
    if let Some(coordination) = &config.coordination {
        ids.push(parse(coordination, &root)?);
    }
    let db = open_device(&root, data, keys).await?;
    let checkout = Checkout {
        path: root.path().to_path_buf(),
        base: config.base,
    };
    open(db, ids, Some(checkout)).await
}

/// Opens the device member's database on the space `id`. With `git`, the checkout is the
/// repository at or above it, whose `nexus-flow.toml` gives the base branch; without it there
/// is none, and "on main" is `null`.
pub async fn space(
    id: SpaceId,
    git: Option<&Path>,
    data: &DataDir,
    keys: DeviceKeys,
) -> Result<Opened, InitError> {
    let (db, checkout) = match git {
        Some(dir) => {
            let root = RepoRoot::find(dir)?;
            let config = Config::read(&root)?;
            let db = open_device(&root, data, keys).await?;
            let checkout = Checkout {
                path: root.path().to_path_buf(),
                base: config.base,
            };
            (db, Some(checkout))
        }
        None => (open_db(data, keys).await?, None),
    };
    open(db, vec![id], checkout).await
}

async fn open(db: Db, ids: Vec<SpaceId>, checkout: Option<Checkout>) -> Result<Opened, InitError> {
    let mut spaces = Vec::with_capacity(ids.len());
    for id in ids {
        let space = match db.spaces().get(id.0).await {
            Ok(space) => Some(space),
            Err(e) if e.code() == ErrorCode::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        spaces.push((id, space));
    }
    Ok(Opened {
        _db: db,
        spaces,
        checkout,
    })
}

fn parse(text: &str, root: &RepoRoot) -> Result<SpaceId, InitError> {
    text.parse().map_err(|error| InitError::SpaceId {
        file: root.config_file(),
        error,
    })
}

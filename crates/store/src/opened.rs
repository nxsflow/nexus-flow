//! The device member's database, open on the spaces a caller reads (RFC 0001, "Reading and
//! writing", "The library seam"): what `Engine::open` and `Engine::open_space` hold.
//!
//! A repository opens on the spaces its `nexus-flow.toml` names, and has a checkout: the root
//! and the base branch "on main" is checked against. A space opened by id has a checkout only
//! when the caller names one, and without it every "on main" value is `null` ("Derived state").
//!
//! A space's kind comes from `nexus-flow.toml` too: the space it names as `repository` or
//! `coordination`, in the repository opened or the checkout named. A space no file names, such
//! as one opened by id without a checkout, is of no known kind, and every write checks
//! [`Opened::check_write`] first, so it takes no write of a collection restricted to some kinds.
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
use crate::space_kind::{self, SpaceKind, WrongSpace};

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
    /// The kinds `nexus-flow.toml` names, for the spaces it names.
    kinds: Vec<(SpaceId, SpaceKind)>,
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

    /// The kind of the open space `id`, as `nexus-flow.toml` names it; `None` when no file the
    /// store read names it.
    pub fn kind(&self, id: &SpaceId) -> Option<SpaceKind> {
        self.held(id)?;
        self.kinds
            .iter()
            .find(|(named, _)| named == id)
            .map(|&(_, kind)| kind)
    }

    /// Whether `collection` may be written into the open space `id` (RFC 0001, "The model"):
    /// every write checks this first.
    pub fn check_write(&self, id: &SpaceId, collection: &str) -> Result<(), WrongSpace> {
        space_kind::check(collection, self.kind(id))
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
            .field("kinds", &self.kinds)
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
    let kinds = named_kinds(&config, &root)?;
    let ids = kinds.iter().map(|&(id, _)| id).collect();
    let db = open_device(&root, data, keys).await?;
    let checkout = Checkout {
        path: root.path().to_path_buf(),
        base: config.base,
    };
    open(db, ids, kinds, Some(checkout)).await
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
    let (db, kinds, checkout) = match git {
        Some(dir) => {
            let root = RepoRoot::find(dir)?;
            let config = Config::read(&root)?;
            let kinds = named_kinds(&config, &root)?;
            let db = open_device(&root, data, keys).await?;
            let checkout = Checkout {
                path: root.path().to_path_buf(),
                base: config.base,
            };
            (db, kinds, Some(checkout))
        }
        None => (open_db(data, keys).await?, Vec::new(), None),
    };
    open(db, vec![id], kinds, checkout).await
}

/// The spaces `config` names, with their kinds: the repository space first.
fn named_kinds(config: &Config, root: &RepoRoot) -> Result<Vec<(SpaceId, SpaceKind)>, InitError> {
    let mut kinds = vec![(parse(&config.repository, root)?, SpaceKind::Repository)];
    if let Some(coordination) = &config.coordination {
        kinds.push((parse(coordination, root)?, SpaceKind::Coordination));
    }
    Ok(kinds)
}

async fn open(
    db: Db,
    ids: Vec<SpaceId>,
    kinds: Vec<(SpaceId, SpaceKind)>,
    checkout: Option<Checkout>,
) -> Result<Opened, InitError> {
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
        kinds,
        checkout,
    })
}

fn parse(text: &str, root: &RepoRoot) -> Result<SpaceId, InitError> {
    text.parse().map_err(|error| InitError::SpaceId {
        file: root.config_file(),
        error,
    })
}

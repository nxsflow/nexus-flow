//! Creating, joining and leaving the coordination space from a checkout (RFC 0009, "Creating
//! it", "Repositories in the space"): each checks the file's edit and the name first, writes the
//! repository record, then `nexus-flow.toml`.

use crate::config::Config;
use crate::coordination::{plan_join, plan_leave, JoinError};
use crate::id::SpaceId;
use crate::open::InitError;
use crate::opened::Opened;
use crate::repo_root::RepoRoot;
use crate::space_kind::SpaceKind;
use crate::space_marker;

use super::repositories::{self, Record};
use super::{name, Coordination, CoordinationError, Repository};

/// A space id no create writes, to check before the space exists that the file takes the line.
const PLACEHOLDER: &str = "00000000000000000000000000000000";

/// The checkout's root and its repository space's id, from `nexus-flow.toml` as it is now.
fn checkout(opened: &Opened) -> Result<(RepoRoot, String), CoordinationError> {
    let path = &opened.checkout().ok_or(CoordinationError::NoCheckout)?.path;
    let root = RepoRoot::find(path).map_err(JoinError::Root)?;
    let config = Config::read(&root).map_err(JoinError::Config)?;
    Ok((root, config.repository))
}

/// Creates a coordination space with the member as its first admin, writes the checkout's record
/// named `name`, or by `origin`, and names the space in `nexus-flow.toml`. A file that already
/// names a coordination space is refused, and nothing is created.
pub async fn create(
    opened: &mut Opened,
    name: Option<&str>,
) -> Result<Coordination, CoordinationError> {
    let (root, own) = checkout(opened)?;
    let file = root.config_file();
    if let Some(recorded) = Config::read(&root).map_err(JoinError::Config)?.coordination {
        return Err(JoinError::OtherCoordination { file, recorded }.into());
    }
    let placeholder: SpaceId = PLACEHOLDER.parse().expect("a space id");
    match plan_join(&root, placeholder) {
        Ok(_) | Err(JoinError::OwnSpace { .. }) => {}
        Err(e) => return Err(e.into()),
    }
    let name = match name {
        Some(text) => name::check(text)?,
        None => name::from_origin(root.path())?,
    };
    let space = SpaceId(
        space_marker::create(opened.db(), SpaceKind::Coordination)
            .await?
            .id(),
    );
    opened.open_coordination(space).await?;
    repositories::upsert(opened, &space, &[], &own, &name).await?;
    if let Some(edit) = plan_join(&root, space)? {
        edit.apply()?;
    }
    Ok(Coordination {
        coordination: space,
        written: true,
        repository: Some(Repository {
            name,
            space: own,
            left: false,
        }),
    })
}

/// Joins the coordination space `space`, which the database must hold: writes the checkout's
/// record, or clears `left` on and renames the records whose `space` is the repository's, and
/// names the space in `nexus-flow.toml`. Without `name`, a record keeps its name, and a new one
/// takes `origin`'s. A name another repository's record holds is refused.
pub async fn join(
    opened: &mut Opened,
    space: SpaceId,
    name: Option<&str>,
) -> Result<Coordination, CoordinationError> {
    let (root, own) = checkout(opened)?;
    let edit = plan_join(&root, space)?;
    let given = name.map(name::check).transpose()?;
    let was_open = opened.held(&space).is_some();
    opened.open_coordination(space).await?;
    if opened.held(&space) != Some(true) {
        if !was_open {
            opened.close(&space);
        }
        return Err(CoordinationError::NotHeld { space });
    }
    let records = repositories::all(opened, &space).await?;
    let mine: Vec<&Record> = records
        .iter()
        .filter(|r| r.repository.space == own)
        .collect();
    let kept = mine.first().map(|r| r.repository.name.clone());
    let name = match (given, &kept) {
        (Some(given), _) => given,
        (None, Some(kept)) => kept.clone(),
        (None, None) => name::from_origin(root.path())?,
    };
    if kept.as_deref() != Some(name.as_str()) {
        let holders: Vec<Repository> = records
            .iter()
            .filter(|r| r.repository.name == name && r.repository.space != own)
            .map(|r| r.repository.clone())
            .collect();
        if !holders.is_empty() {
            return Err(CoordinationError::NameTaken { name, holders });
        }
    }
    let mut written = repositories::upsert(opened, &space, &mine, &own, &name).await?;
    if let Some(edit) = edit {
        edit.apply()?;
        written = true;
    }
    Ok(Coordination {
        coordination: space,
        written,
        repository: Some(Repository {
            name,
            space: own,
            left: false,
        }),
    })
}

/// Leaves the coordination space `nexus-flow.toml` names: sets `left` on the repository's records,
/// which stay, and takes the space out of the file.
pub async fn leave(opened: &mut Opened) -> Result<Coordination, CoordinationError> {
    let (root, own) = checkout(opened)?;
    let (recorded, edit) = plan_leave(&root)?.ok_or(CoordinationError::NotJoined)?;
    let space: SpaceId = recorded.parse().map_err(|error| InitError::SpaceId {
        file: root.config_file(),
        error,
    })?;
    opened.open_coordination(space).await?;
    if opened.held(&space) != Some(true) {
        return Err(CoordinationError::NotHeld { space });
    }
    let records = repositories::all(opened, &space).await?;
    let mine: Vec<&Record> = records
        .iter()
        .filter(|r| r.repository.space == own)
        .collect();
    repositories::set_left(opened, &space, &mine).await?;
    edit.apply()?;
    opened.close(&space);
    Ok(Coordination {
        coordination: space,
        written: true,
        repository: mine.first().map(|r| Repository {
            left: true,
            ..r.repository.clone()
        }),
    })
}

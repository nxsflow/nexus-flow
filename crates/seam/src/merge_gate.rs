//! The merge gate on the seam (RFC 0010, "Public surfaces this RFC brings"):
//! [`Engine::merge_gate_rules`], the counterpart of `nxs merge-gate rules`, and the types of its
//! report.
//!
//! It reads only the local git, from the repository's root as `Engine::open` finds it, and opens no
//! database, so it runs on a machine without a key store, such as the gate's workflow.

use std::path::Path;

use nexus_flow_store::config::Config;
use nexus_flow_store::repo_root::RepoRoot;
use nexus_flow_store::space::InitError;

pub use nexus_flow_merge_gate::gate_toml::{Effect, RuleRef};
pub use nexus_flow_merge_gate::rules::{
    GitError, Rule, RulesError, RulesReport, Settings, GATE_TOML_PATH, GENESIS_PATH,
};

use crate::engine::{Engine, Error};

impl Engine {
    /// Every rule the gate judges, read from the commit `base` names, or, without `base`, from the
    /// branch `[git] base` names in `nexus-flow.toml`, for the repository at or above
    /// `repository`. Rules that cannot be read are a report, not an error; a base that names no
    /// commit is an error.
    pub fn merge_gate_rules(repository: &Path, base: Option<&str>) -> Result<RulesReport, Error> {
        let root = RepoRoot::find(repository).map_err(InitError::Root)?;
        let base = match base {
            Some(base) => base.to_string(),
            None => Config::read(&root).map_err(InitError::Config)?.base,
        };
        Ok(nexus_flow_merge_gate::rules::rules_at(root.path(), &base)?)
    }
}

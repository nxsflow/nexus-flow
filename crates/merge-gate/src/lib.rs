//! The merge gate of the factory module (RFC 0010): reads the rules and decides whether pm or the owner merges.

pub mod gate_toml;
pub mod genesis_rules;
mod git;
mod markdown;
pub mod rules;
pub mod verdict;
pub mod verdict_comments;

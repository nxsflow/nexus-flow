//! Installing the shipped defaults (RFC 0006, "Installing the shipped defaults"; RFC 0004,
//! "Installing the template"): the states of a shipped file, the CODEOWNERS report, and the
//! install of the factory's defaults. It touches only files, never NoCentralDB, and never writes
//! CODEOWNERS.

pub mod codeowners;
pub mod factory;
pub mod shipped;

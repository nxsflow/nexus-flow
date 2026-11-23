//! Installing the shipped defaults (RFC 0006, "Installing the shipped defaults"; RFC 0004,
//! "Installing the template"): the states of a shipped file, and the CODEOWNERS report. It touches
//! only files, never NoCentralDB, and never writes CODEOWNERS.

pub mod codeowners;
pub mod shipped;

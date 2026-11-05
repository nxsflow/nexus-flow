//! `NXR_CRASH_AT=<boundary>:<nth>`: abort at the nth occurrence of a boundary in this process. Under
//! `NXR_BOUNDARY_COUNTS=<file>` every occurrence is appended to that file, which is how the harness learns how
//! often a clean run passes each boundary, so it can cover occurrences as well as kinds.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::boundary::Boundary;

fn seen() -> &'static Mutex<HashMap<Boundary, u32>> {
    static SEEN: OnceLock<Mutex<HashMap<Boundary, u32>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn target() -> Option<&'static (Boundary, u32)> {
    static TARGET: OnceLock<Option<(Boundary, u32)>> = OnceLock::new();
    TARGET
        .get_or_init(|| {
            let spec = std::env::var("NXR_CRASH_AT").ok()?;
            let (name, nth) = spec.split_once(':')?;
            Some((Boundary::parse(name)?, nth.parse().ok()?))
        })
        .as_ref()
}

fn journal(line: &str) {
    if let Ok(path) = std::env::var("NXR_BOUNDARY_COUNTS") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            // One write per line: the worker's thread reaches boundaries too.
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
    }
}

/// A write of the effect executor, journalled between the boundaries.
pub(crate) fn wrote(what: &str) {
    journal(&format!("write {what}"));
}

pub(crate) fn reached(boundary: Boundary) {
    journal(boundary.name());
    let Some((wanted, nth)) = target() else {
        return;
    };
    if *wanted != boundary {
        return;
    }
    let mut seen = seen().lock().unwrap_or_else(PoisonError::into_inner);
    let count = seen.entry(boundary).or_insert(0);
    *count += 1;
    if *count == *nth {
        std::process::abort();
    }
}

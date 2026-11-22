//! nexus-flow's NoCentralDB model (RFC 0001, "The model"). The source is `model/model.ts` at the
//! repository root; `ncdb model build` writes `model/model.lock.json` from it, and this crate
//! reads only that committed lock, compiled in, so building `nxs` needs neither Node nor `ncdb`.

/// The text of the committed `model/model.lock.json`, which every member database opens with.
pub const LOCK: &str = include_str!("../../../model/model.lock.json");

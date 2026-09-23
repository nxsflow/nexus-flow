//! The declarative plugin seam (vocabulary, priority, ranking, presentation) — now owned by
//! `nexus-flow-facade` (E5 #9t7.1) because the `next` ranking is *compute*, shared by the CLI
//! and any embedding app. The two embedded configs (`issue-tracker`, `personal-todo`) live in
//! the facade alongside it. Re-exported so `crate::plugin::…` paths resolve unchanged.

pub use nexus_flow_facade::plugin::*;

//! flow re-exports the foundation's [`Reducer`](nxs_foundation::reducer::Reducer) trait so
//! existing `nexus_flow_core::reducer::Reducer` paths keep resolving. The reducer seam itself —
//! the data axis the substrate dispatches on — is domain-agnostic and owned by the foundation;
//! flow's concrete reducer is [`crate::task_reducer::TaskReducer`].

pub use nxs_foundation::reducer::Reducer;

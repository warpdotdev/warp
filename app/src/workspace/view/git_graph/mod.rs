//! Git Graph panel: a read-only commit-DAG visualization (see specs/git-graph).
//!
//! Module layering:
//! - [`data`]       commit data types + parsing of `git log` output (pure
//!   functions) + async data loading.
//! - [`layout`]     arranges a commit sequence into a per-row lane layout (pure
//!   functions, the core algorithm).
//! - [`row_canvas`] custom drawing element for a single row of lanes (vertical
//!   lines / dots / orthogonal polylines).
//! - [`view`]       the Git Graph view in the left panel.
//!
//! Later phases will add commit details and paginated lazy loading.

pub(crate) mod data;
pub(crate) mod layout;
pub(crate) mod row_canvas;
pub(crate) mod view;

#[cfg(not(target_family = "wasm"))]
pub(crate) use view::GitGraphEvent;
pub(crate) use view::{init, GitGraphView};

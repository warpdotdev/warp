//! Git Graph panel: a read-only commit-DAG visualization (see specs/git-graph).
//!
//! Module layering:
//! - [`data`]       commit data types + parsing of `git log` output (pure
//!   functions) + async data loading.
//! - [`file_tree`]  turns a commit's flat changed-file list into a collapsible
//!   directory tree for the detail area (pure functions).
//! - [`ops`]        write-operation layer: mutating git actions reachable from
//!   the right-click context menus (pure arg-builders + async runners), gated by
//!   [`warp_features::FeatureFlag::GitGraphWrite`].
//! - [`menu`]       builds the per-target right-click context menus (pure).
//! - [`layout`]     arranges a commit sequence into a per-row lane layout (pure
//!   functions, the core algorithm).
//! - [`row_canvas`] custom drawing element for a single row of lanes (vertical
//!   lines / dots / orthogonal polylines).
//! - [`view`]       the Git Graph view in the left panel.
//! - [`auto_refresh`] keeps the graph in sync with git operations made outside
//!   the panel (terminal / IDE / external git) by subscribing to the selected
//!   repo's `.git` changes via `repo_metadata` (`local_fs`); a graph-affecting
//!   change triggers a position-preserving reload.

pub(crate) mod auto_refresh;
pub(crate) mod data;
pub(crate) mod file_tree;
pub(crate) mod layout;
pub(crate) mod menu;
pub(crate) mod ops;
pub(crate) mod row_canvas;
pub(crate) mod view;

#[cfg(not(target_family = "wasm"))]
pub(crate) use view::GitGraphEvent;
pub(crate) use view::{init, GitGraphView};

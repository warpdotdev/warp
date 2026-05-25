//! Test seams that re-export LSP plumbing so integration tests in the
//! `integration` crate can drive the custom-LSP path end-to-end. The
//! underlying `code` and `ai` modules are private to the warp crate;
//! everything here is intentionally test-only.

pub use crate::ai::persisted_workspace::PersistedWorkspace;
pub use crate::code::footer::CodeFooterView;
pub use crate::code::lsp_dispatch::{resolve_server_for_path, ResolvedLspServer};

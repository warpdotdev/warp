//! File → LSP-server resolution for the editor.
//!
//! Centralizes the "which LSP server handles this file?" decision so every
//! call site dispatches identically. Custom descriptors from
//! `[[editor.language_servers]]` take precedence over the frozen built-in
//! `LanguageId` → `LSPServerType` mapping; when several custom descriptors
//! match the same file, the first in settings order wins.
//!
//! Most call sites take a [`ResolvedLspServer`] and hand it to a downstream
//! API (e.g. `PersistedWorkspace::enable_and_spawn_lsp_server`) without
//! looking at the variant. The kind-aware accessors on this type
//! ([`display_name`], [`log_file_path`]) absorb the rest of the dispatch
//! so call sites stay free of `match` blocks.

use std::path::{Path, PathBuf};

use lsp::descriptor::LspServerDescriptor;
use lsp::supported_servers::LSPServerType;
use lsp::LanguageId;
use warpui::{AppContext, SingletonEntity};

use crate::code::lsp_logs::{custom_log_file_path, log_file_path};
use crate::settings::LanguageServersSettings;

/// Identity of the LSP server claiming a file, returned by
/// [`resolve_server_for_path`]. The `Custom` arm carries an owned clone of the
/// descriptor so callers can construct a [`lsp::CustomLspServerConfig`]
/// without re-borrowing settings; the `BuiltIn` arm just carries the
/// `LSPServerType`. The descriptor is boxed because it's substantially
/// larger than `LSPServerType` — keeps the enum's stack size small.
#[derive(Debug, Clone)]
pub enum ResolvedLspServer {
    BuiltIn(LSPServerType),
    Custom(Box<LspServerDescriptor>),
}

impl ResolvedLspServer {
    /// User-facing display name for this server — `binary_name()` for
    /// built-ins, `descriptor.name` for customs. Used in footer labels and
    /// log messages so call sites don't have to match on the kind to render
    /// the right string.
    pub fn display_name(&self) -> &str {
        match self {
            Self::BuiltIn(st) => st.binary_name(),
            Self::Custom(d) => &d.name,
        }
    }

    /// Absolute path to this server's log file for the given workspace,
    /// dispatching to the kind-appropriate naming helper.
    pub fn log_file_path(&self, workspace_path: &Path) -> PathBuf {
        match self {
            Self::BuiltIn(st) => log_file_path(*st, workspace_path),
            Self::Custom(d) => custom_log_file_path(&d.name, workspace_path),
        }
    }
}

/// Resolves which LSP server should handle `path` for the editor.
///
/// Walks the user's `[[editor.language_servers]]` array first; if any custom
/// descriptor's `filetypes` matcher claims the path, returns that descriptor.
/// Otherwise falls back to the built-in `LanguageId` → `LSPServerType` map.
/// Returns `None` when neither side claims the path (e.g. an unknown
/// extension with no matching custom entry).
pub fn resolve_server_for_path(path: &Path, ctx: &AppContext) -> Option<ResolvedLspServer> {
    if let Some(matched) = LanguageServersSettings::as_ref(ctx).match_for_path(path) {
        return Some(ResolvedLspServer::Custom(Box::new(
            matched.descriptor.clone(),
        )));
    }
    LanguageId::from_path(path).map(|id| ResolvedLspServer::BuiltIn(id.server_type()))
}

#[cfg(test)]
#[path = "lsp_dispatch_tests.rs"]
mod tests;

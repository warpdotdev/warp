mod config;
mod manager;
mod model;
pub mod supported_servers;
pub mod types;

pub use config::{LanguageId, LspServerConfig};
pub use lsp_types::{Position, Range};
pub use manager::{LspManagerModel, LspManagerModelEvent};
pub use model::{
    BackgroundTaskInfo, DocumentDiagnostics, LanguageServerId, LspEvent, LspServerModel, LspState,
};
pub use types::{HoverContents, HoverResult, MarkupKind, ReferenceLocation};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LspServerLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LspServerLogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        };
        f.write_str(level)
    }
}

pub fn init(app: &mut warpui_core::AppContext) {
    app.add_singleton_model(|_| LspManagerModel::new());
}

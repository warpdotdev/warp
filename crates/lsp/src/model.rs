use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use instant::Instant;
use lsp_types::FormattingOptions;
use warpui_core::{Entity, ModelContext};

use crate::config::LanguageId;
use crate::supported_servers::LSPServerType;
use crate::types::{
    DefinitionLocation, DocumentVersion, HoverResult, Location, ReferenceLocation,
    TextDocumentContentChangeEvent, TextEdit, WatchedFileChangeEvent,
};
use crate::{LspServerConfig, LspServerLogLevel};

static NEXT_LANGUAGE_SERVER_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LanguageServerId(usize);

impl LanguageServerId {
    pub fn new() -> Self {
        Self(NEXT_LANGUAGE_SERVER_ID.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for LanguageServerId {
    fn default() -> Self {
        Self::new()
    }
}

pub enum LspState {
    Stopped { manually_stopped: bool },
    Starting,
    Stopping { manually_stopped: bool },
    Available {},
    Failed { error: String },
}

impl LspState {
    pub fn name(&self) -> &str {
        match self {
            Self::Stopped { .. } => "stopped",
            Self::Starting => "starting",
            Self::Stopping { .. } => "stopping",
            Self::Available { .. } => "available",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn can_auto_start(&self) -> bool {
        false
    }
}

pub struct LspServerModel {
    id: LanguageServerId,
    server_state: LspState,
    config: LspServerConfig,
}

#[derive(Debug, Clone)]
pub struct BackgroundTaskInfo {
    pub task_token: String,
    pub message: Option<String>,
    pub finished: bool,
    pub updated_at: Instant,
}

impl BackgroundTaskInfo {
    pub fn to_display_message(&self) -> String {
        let message_part = if let Some(message) = &self.message {
            format!("{} {}", self.task_token, message)
        } else {
            self.task_token.clone()
        };

        if self.finished {
            format!("finished: {message_part}")
        } else {
            message_part
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocumentDiagnostics {
    pub diagnostics: Vec<lsp_types::Diagnostic>,
    pub version: Option<i32>,
    pub published_at: Instant,
}

#[derive(Debug)]
pub enum LspEvent {
    Starting,
    BackgroundTaskUpdated,
    Idle,
    Stopped,
    Failed(anyhow::Error),
    Started,
    DiagnosticsUpdated { path: PathBuf },
}

type LspFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>;

fn disabled() -> anyhow::Error {
    anyhow::anyhow!("LSP support has been removed")
}

impl LspServerModel {
    pub fn id(&self) -> LanguageServerId {
        self.id
    }

    pub fn server_type(&self) -> LSPServerType {
        self.config.server_type()
    }

    pub fn server_name(&self) -> String {
        self.config.server_name()
    }

    pub fn state(&self) -> &LspState {
        &self.server_state
    }

    pub fn log_to_server_log(&self, _level: LspServerLogLevel, _message: impl Into<String>) {}

    pub fn latest_progress_update(&self) -> Option<&BackgroundTaskInfo> {
        None
    }

    pub fn is_ready_for_requests(&self) -> bool {
        false
    }

    pub fn has_started(&self) -> bool {
        false
    }

    pub fn has_pending_tasks(&self) -> bool {
        false
    }

    pub fn supports_language(&self, _lang: &LanguageId) -> bool {
        false
    }

    pub fn initial_workspace(&self) -> &Path {
        self.config.initial_workspace()
    }

    pub fn can_auto_start(&self) -> bool {
        false
    }

    pub fn stop(&mut self, manually_stopped: bool, _ctx: &mut ModelContext<Self>) -> Result<()> {
        self.server_state = LspState::Stopped { manually_stopped };
        Ok(())
    }

    pub fn manual_start(&mut self, _ctx: &mut ModelContext<Self>) -> Result<()> {
        Err(disabled())
    }

    pub fn restart(&mut self, _ctx: &mut ModelContext<Self>) {}

    pub fn document_is_open(&self, _path: &PathBuf) -> Result<bool> {
        Err(disabled())
    }

    pub fn last_synced_version(&self, _path: &PathBuf) -> Result<Option<usize>> {
        Err(disabled())
    }

    pub fn did_open_document(
        &self,
        _path: PathBuf,
        _content: String,
        _initial_version: usize,
    ) -> Result<LspFuture<()>> {
        Err(disabled())
    }

    pub fn did_close_document(&self, _path: PathBuf) -> Result<LspFuture<()>> {
        Err(disabled())
    }

    pub fn did_change_document(
        &self,
        _path: PathBuf,
        _version: DocumentVersion,
        _deltas: Vec<TextDocumentContentChangeEvent>,
    ) -> Result<LspFuture<()>> {
        Err(disabled())
    }

    pub fn did_change_watched_files(&self, _events: Vec<WatchedFileChangeEvent>) -> Result<()> {
        Err(disabled())
    }

    pub fn goto_definition(
        &self,
        _path: PathBuf,
        _position: Location,
    ) -> Result<LspFuture<Vec<DefinitionLocation>>> {
        Err(disabled())
    }

    pub fn format_document(
        &self,
        _path: PathBuf,
        _options: FormattingOptions,
    ) -> Result<LspFuture<Option<Vec<TextEdit>>>> {
        Err(disabled())
    }

    pub fn hover(
        &self,
        _path: PathBuf,
        _position: Location,
    ) -> Result<LspFuture<Option<HoverResult>>> {
        Err(disabled())
    }

    pub fn diagnostics_for_path(&self, _path: &Path) -> Result<Option<&DocumentDiagnostics>> {
        Ok(None)
    }

    pub fn find_references(
        &self,
        _path: PathBuf,
        _position: Location,
    ) -> Result<LspFuture<Vec<ReferenceLocation>>> {
        Err(disabled())
    }
}

impl Entity for LspServerModel {
    type Event = LspEvent;
}

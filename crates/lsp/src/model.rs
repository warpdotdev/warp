use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Error, Result};
use instant::Instant;
use jsonrpc::ServerNotificationEvent;
use lsp_types::notification::{self, Notification};
use lsp_types::{
    FormattingOptions, NumberOrString, ProgressParams, ProgressParamsValue,
    PublishDiagnosticsParams, WorkDoneProgress,
};
#[cfg(not(target_arch = "wasm32"))]
use simple_logger::manager::LogManager;
#[cfg(not(target_arch = "wasm32"))]
use warp_core::features::FeatureFlag;
use warpui::r#async::executor::Background;
#[cfg(not(target_arch = "wasm32"))]
use warpui::SingletonEntity;
use warpui::{Entity, ModelContext};

use crate::config::{lsp_uri_to_path, LanguageId};
use crate::server_repo_watcher::LspRepoWatcher;
use crate::supported_servers::LSPServerType;
use crate::types::{
    DefinitionLocation, DocumentVersion, HoverResult, Location, ReferenceLocation,
    TextDocumentContentChangeEvent, TextEdit, WatchedFileChangeEvent,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::{spawn_lsp_service, LspServiceInitializationResult};
use crate::{LspServerConfig, LspServerLogLevel, LspService};

static NEXT_LANGUAGE_SERVER_ID: AtomicUsize = AtomicUsize::new(0);

/// Unique identifier for a running language server instance.
/// This is used to track which LSP server is associated with external files
/// that were navigated to via goto-definition.
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

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum LspState {
    Stopped {
        manually_stopped: bool,
    },
    Starting,
    Stopping {
        manually_stopped: bool,
    },
    Available {
        service: Arc<LspService>,
        background_executor: Arc<Background>,
    },
    Failed {
        error: String,
    },
}

impl LspState {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn name(&self) -> &str {
        match self {
            Self::Stopped { .. } => "stopped",
            Self::Starting => "starting",
            Self::Stopping { .. } => "stopping",
            Self::Available { .. } => "available",
            Self::Failed { .. } => "failed",
        }
    }

    /// Returns whether this server can be auto-started.
    /// Returns false if the server was manually stopped by the user.
    pub fn can_auto_start(&self) -> bool {
        match self {
            Self::Stopped { manually_stopped } | Self::Stopping { manually_stopped } => {
                !manually_stopped
            }
            _ => true,
        }
    }
}

pub struct LspServerModel {
    id: LanguageServerId,
    server_state: LspState,
    config: LspServerConfig,
    // This tracks all in-progress background tasks from the server.
    // Tasks are keyed by their progress token and removed when they finish.
    in_progress_tasks: HashMap<String, BackgroundTaskInfo>,
    diagnostics_by_path: HashMap<PathBuf, DocumentDiagnostics>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) repo_watcher: LspRepoWatcher,
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
    /// This is roundtripped back from the server to client.
    pub version: Option<i32>,
    pub published_at: Instant,
}

#[derive(Debug)]
pub enum LspEvent {
    Starting,
    BackgroundTaskUpdated,
    Idle,
    Stopped,
    Failed(Error),
    Started,
    DiagnosticsUpdated { path: PathBuf },
}

// Determines whether to accept an incoming diagnostic update based on version.
//
// An incoming version of None means the server is sending diagnostics not tied to a
// specific document version (e.g. transitive workspace updates from gopls). We accept
// these so that stale diagnostics do not persist indefinitely. The caller is responsible
// for preserving the existing version when the incoming version is None, so the
// render-side version check can still filter out diagnostics that don't match the
// current buffer.
fn should_accept_publish_diagnostics_version(existing: Option<i32>, incoming: Option<i32>) -> bool {
    match (existing, incoming) {
        (Some(_), None) => true,
        (None, None) => true,
        (None, Some(_)) => true,
        (Some(existing), Some(incoming)) => incoming >= existing,
    }
}

impl LspServerModel {
    pub(crate) fn new(config: LspServerConfig) -> Self {
        Self {
            id: LanguageServerId::new(),
            server_state: LspState::Stopped {
                manually_stopped: false,
            },
            config,
            in_progress_tasks: HashMap::new(),
            diagnostics_by_path: HashMap::new(),
            repo_watcher: LspRepoWatcher::new(),
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn repo_watcher_mut(&mut self) -> &mut LspRepoWatcher {
        &mut self.repo_watcher
    }

    /// Returns the unique identifier for this language server instance.
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

    pub fn log_to_server_log(&self, level: LspServerLogLevel, message: impl Into<String>) {
        if let LspState::Available { service, .. } = &self.server_state {
            service.log_to_server_log(level, message);
        }
    }

    pub fn latest_progress_update(&self) -> Option<&BackgroundTaskInfo> {
        self.in_progress_tasks
            .values()
            .max_by_key(|task| task.updated_at)
    }

    pub fn is_ready_for_requests(&self) -> bool {
        matches!(&self.server_state, LspState::Available { .. })
    }

    pub fn has_started(&self) -> bool {
        !matches!(&self.server_state, LspState::Stopped { .. })
    }

    pub fn has_pending_tasks(&self) -> bool {
        !self.in_progress_tasks.is_empty()
    }

    pub fn supports_language(&self, lang: &LanguageId) -> bool {
        self.config.languages().contains(lang)
    }

    /// Returns the initial workspace path for this server.
    pub fn initial_workspace(&self) -> &Path {
        self.config.initial_workspace()
    }

    /// Returns whether this server can be auto-started by LspManagerModel::start_all.
    /// Returns false if the server was manually stopped by the user.
    pub fn can_auto_start(&self) -> bool {
        self.server_state.can_auto_start()
    }

    fn service(&self) -> Result<Arc<LspService>> {
        match &self.server_state {
            LspState::Available { service, .. } => Ok(service.clone()),
            LspState::Starting => Err(anyhow::anyhow!("Server is starting")),
            LspState::Stopped { .. } => Err(anyhow::anyhow!("Server is stopped")),
            LspState::Stopping { .. } => Err(anyhow::anyhow!("Server is stopping")),
            LspState::Failed { error } => Err(anyhow::anyhow!("Server has failed: {error}")),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start(&mut self, ctx: &mut ModelContext<Self>) -> Result<()> {
        match &self.server_state {
            LspState::Stopped { .. } => {
                self.server_state = LspState::Starting;
                ctx.emit(LspEvent::Starting);
                let server_name = self.server_name();

                let config = self.config.clone();
                let executor = ctx.background_executor();
                let logger = match config.log_relative_path().cloned() {
                    Some(log_relative_path) => {
                        match LogManager::handle(ctx).update(ctx, |manager, _| {
                            manager.register_namespace("lsp", true);
                            manager.register("lsp", &log_relative_path, executor.clone())
                        }) {
                            Ok(logger) => Some(logger),
                            Err(e) => {
                                log::warn!(
                                    "Failed to register LSP log file for {server_name}; continuing without file logging: {e:#}"
                                );
                                None
                            }
                        }
                    }
                    None => None,
                };
                ctx.spawn(
                    async move { spawn_lsp_service(config, executor, logger).await },
                    move |me, result, ctx| match result {
                        Ok(LspServiceInitializationResult { service, channel }) => {
                            ctx.spawn_stream_local(
                                channel,
                                |me, notification, ctx| {
                                    me.handle_server_notification(notification, ctx);
                                },
                                |_, _| {},
                            );

                            // At this point, the server has started and been initialized with its workspace folders
                            // but it is likely still in the "bootstrapping" phase where it will respond to most
                            // requests with `null` responses. We capture a reference to the service here
                            // and set our state to available. We consider it "bootstrapped" when the server notifies us
                            // that the bootstrap task is complete.
                            me.server_state = LspState::Available {
                                service: Arc::new(service),
                                background_executor: ctx.background_executor(),
                            };

                            if FeatureFlag::LSPAsATool.is_enabled() {
                                me.repo_watcher.ensure(&me.config, ctx);
                            }

                            ctx.emit(LspEvent::Started);
                        }
                        Err(e) => {
                            log::error!("Failed to start LSP server: {e}");
                            let error = format!("{e:#}");
                            me.server_state = LspState::Failed { error };
                            ctx.emit(LspEvent::Failed(e));
                        }
                    },
                );
            }
            _ => {
                log::warn!(
                    "Unable to start LSP server in state: {}",
                    self.server_state.name()
                );
            }
        }

        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn stop(&mut self, manually_stopped: bool, ctx: &mut ModelContext<Self>) -> Result<()> {
        match &self.server_state {
            LspState::Available { service, .. } => {
                if FeatureFlag::LSPAsATool.is_enabled() {
                    self.repo_watcher.teardown(ctx);
                }

                let service = service.clone();
                self.server_state = LspState::Stopping { manually_stopped };
                ctx.spawn(async move { service.shutdown().await }, |me, _, ctx| {
                    // Only transition to Stopped if still in Stopping state.
                    // This prevents race conditions if manual_start was called while shutdown was in flight.
                    if let LspState::Stopping { manually_stopped } = me.server_state {
                        me.server_state = LspState::Stopped { manually_stopped };
                        ctx.emit(LspEvent::Stopped);
                    }
                });
            }
            _ => {
                log::debug!(
                    "Unable to stop LSP server in state: {}",
                    self.server_state.name()
                );
            }
        }
        Ok(())
    }

    /// Manually starts the server and clears the manually_stopped flag.
    /// This should be called when the user explicitly wants to start the server.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn manual_start(&mut self, ctx: &mut ModelContext<Self>) -> Result<()> {
        match &self.server_state {
            LspState::Stopped { .. } | LspState::Failed { .. } => {
                // Clear the manually_stopped flag and start
                self.server_state = LspState::Stopped {
                    manually_stopped: false,
                };
                self.start(ctx)
            }
            LspState::Stopping {
                manually_stopped: true,
            } => {
                // Server is still shutting down from a manual stop.
                // Clear the manually_stopped flag so can_auto_start returns true,
                // then use start_all to trigger start after shutdown completes.
                self.server_state = LspState::Stopping {
                    manually_stopped: false,
                };
                Ok(())
            }
            _ => {
                log::debug!(
                    "Unable to manually start LSP server in state: {}",
                    self.server_state.name()
                );
                Ok(())
            }
        }
    }

    /// Manually starts the server (WASM stub).
    #[cfg(target_arch = "wasm32")]
    pub fn manual_start(&mut self, _ctx: &mut ModelContext<Self>) -> Result<()> {
        Err(anyhow::anyhow!(
            "Start is not supported in WASM environments"
        ))
    }

    /// Restarts the LSP server by stopping it and starting it again.
    /// The server will emit `LspEvent::Stopped` followed by `LspEvent::Started` on success.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn restart(&mut self, ctx: &mut ModelContext<Self>) {
        log::info!("Restarting LSP server: {}", self.config.server_name());

        match &self.server_state {
            LspState::Available { service, .. } => {
                if FeatureFlag::LSPAsATool.is_enabled() {
                    self.repo_watcher.teardown(ctx);
                }

                let service = service.clone();
                self.server_state = LspState::Stopping {
                    manually_stopped: false,
                };
                ctx.spawn(async move { service.shutdown().await }, |me, _, ctx| {
                    // Only transition if still in Stopping state
                    if let LspState::Stopping { manually_stopped } = me.server_state {
                        me.server_state = LspState::Stopped { manually_stopped };
                        // Immediately start the server again
                        if let Err(e) = me.start(ctx) {
                            log::warn!("Failed to restart LSP server: {e}");
                        }
                    }
                });
            }
            LspState::Failed { .. } | LspState::Stopped { .. } => {
                // Server was in Failed or Stopped state, just start it
                self.server_state = LspState::Stopped {
                    manually_stopped: false,
                };
                if let Err(e) = self.start(ctx) {
                    log::warn!("Failed to restart LSP server: {e}");
                }
            }
            LspState::Starting | LspState::Stopping { .. } => {
                log::debug!(
                    "Unable to restart LSP server in state: {}",
                    self.server_state.name()
                );
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn restart(&mut self, _ctx: &mut ModelContext<Self>) {}

    /// Different from stop -- on terminate, we won't update the server state and emit events based on server response.
    fn terminate(&mut self) {
        match &self.server_state {
            LspState::Available {
                service,
                background_executor,
            } => {
                let service = service.clone();
                let executor = background_executor.clone();
                // Assume the server state is stopped.
                self.server_state = LspState::Stopped {
                    manually_stopped: false,
                };
                executor
                    .spawn(async move {
                        let _ = service.shutdown().await;
                    })
                    .detach();
            }
            _ => {
                log::debug!(
                    "Unable to stop LSP server in state: {}",
                    self.server_state.name()
                );
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start(&mut self, _ctx: &mut ModelContext<Self>) -> Result<()> {
        Err(anyhow::anyhow!(
            "Start is not supported in WASM environments"
        ))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn stop(&mut self, _manually_stopped: bool, _ctx: &mut ModelContext<Self>) -> Result<()> {
        Ok(())
    }

    pub fn document_is_open(&self, path: &PathBuf) -> Result<bool> {
        let service = self.service()?;
        service.text_document().document_is_open(path)
    }

    /// Returns the last synced buffer version for the document, if it is open.
    pub fn last_synced_version(&self, path: &PathBuf) -> Result<Option<usize>> {
        let service = self.service()?;
        service.text_document().last_synced_version(path)
    }

    pub fn did_open_document(
        &self,
        path: PathBuf,
        content: String,
        initial_version: usize,
    ) -> Result<impl Future<Output = Result<()>>> {
        let service = self.service()?;
        Ok(async move {
            service
                .text_document()
                .did_open(&path, content, initial_version)
                .await
        })
    }

    pub fn did_close_document(&self, path: PathBuf) -> Result<impl Future<Output = Result<()>>> {
        let service = self.service()?;
        Ok(async move { service.text_document().did_close(&path).await })
    }

    pub fn did_change_document(
        &self,
        path: PathBuf,
        version: DocumentVersion,
        deltas: Vec<TextDocumentContentChangeEvent>,
    ) -> Result<impl Future<Output = Result<()>>> {
        let service = self.service()?;
        Ok(async move {
            service
                .text_document()
                .did_change(&path, version.as_i32(), deltas)
                .await
        })
    }

    pub fn did_change_watched_files(&self, events: Vec<WatchedFileChangeEvent>) -> Result<()> {
        let service = self.service()?;
        service.workspace_watched_files_changed(events)
    }

    pub fn goto_definition(
        &self,
        path: PathBuf,
        position: Location,
    ) -> Result<impl Future<Output = Result<Vec<DefinitionLocation>>>> {
        let service = self.service()?;
        Ok(async move {
            let result = service
                .text_document()
                .definition(&path, position.into_lsp())
                .await?;
            Ok(result
                .into_iter()
                .filter_map(|location| DefinitionLocation::try_from(location).ok())
                .collect())
        })
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn handle_server_notification(
        &mut self,
        notification: ServerNotificationEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let ServerNotificationEvent { method, params } = notification;
        match method.as_str() {
            notification::Progress::METHOD => {
                if let Ok(progress_params) = serde_json::from_value::<ProgressParams>(params) {
                    self.handle_progress_update(progress_params, ctx);
                }
            }
            notification::PublishDiagnostics::METHOD => {
                match serde_json::from_value::<PublishDiagnosticsParams>(params) {
                    Ok(params) => self.handle_publish_diagnostics(params, ctx),
                    Err(e) => log::warn!("Failed to parse PublishDiagnostics params: {e}"),
                }
            }
            _ => {
                log::warn!("Received unhandled notification {method}");
            }
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn handle_progress_update(
        &mut self,
        progress_params: ProgressParams,
        ctx: &mut ModelContext<Self>,
    ) {
        let task_token = match progress_params.token {
            NumberOrString::String(token) => token,
            NumberOrString::Number(token) => token.to_string(),
        };

        let ProgressParamsValue::WorkDone(work_done_progress) = progress_params.value;

        match work_done_progress {
            WorkDoneProgress::Begin(report) => {
                self.in_progress_tasks.insert(
                    task_token.clone(),
                    BackgroundTaskInfo {
                        task_token,
                        message: report.message.clone(),
                        finished: false,
                        updated_at: Instant::now(),
                    },
                );
                ctx.emit(LspEvent::BackgroundTaskUpdated);
            }
            WorkDoneProgress::Report(report) => {
                if let Some(task) = self.in_progress_tasks.get_mut(&task_token) {
                    task.message = report.message.clone();
                    task.updated_at = Instant::now();
                } else {
                    // If we get a report without a begin, create the task
                    self.in_progress_tasks.insert(
                        task_token.clone(),
                        BackgroundTaskInfo {
                            task_token: task_token.clone(),
                            message: report.message.clone(),
                            finished: false,
                            updated_at: Instant::now(),
                        },
                    );
                }
                ctx.emit(LspEvent::BackgroundTaskUpdated);
            }
            WorkDoneProgress::End(_) => {
                log::debug!("LSP server finished {task_token}");
                self.in_progress_tasks.remove(&task_token);
                ctx.emit(LspEvent::BackgroundTaskUpdated);
            }
        };
    }

    fn handle_publish_diagnostics(
        &mut self,
        params: PublishDiagnosticsParams,
        ctx: &mut ModelContext<Self>,
    ) {
        let uri = params.uri;

        let path = match lsp_uri_to_path(&uri) {
            Ok(path) => path,
            Err(e) => {
                log::warn!(
                    "PublishDiagnostics contained invalid URI {}: {e}",
                    uri.as_str()
                );
                return;
            }
        };

        let existing_version = self
            .diagnostics_by_path
            .get(&path)
            .and_then(|diagnostics| diagnostics.version);

        let incoming_version = params.version;
        let incoming_count = params.diagnostics.len();

        if !should_accept_publish_diagnostics_version(existing_version, incoming_version) {
            self.log_to_server_log(
                LspServerLogLevel::Info,
                format!(
                    "publishDiagnostics <- server: DROPPED file={} incoming_version={incoming_version:?} existing_version={existing_version:?} diag_count={incoming_count}",
                    path.display()
                ),
            );
            return;
        }

        self.log_to_server_log(
            LspServerLogLevel::Debug,
            format!(
                "publishDiagnostics <- server: ACCEPTED file={} version={incoming_version:?} diag_count={incoming_count}",
                path.display()
            ),
        );

        // When the incoming version is None (unversioned transitive update), preserve
        // the existing version so the render-side version check can still guard against
        // showing diagnostics that don't match the current buffer version.
        let stored_version = incoming_version.or(existing_version);

        self.diagnostics_by_path.insert(
            path.clone(),
            DocumentDiagnostics {
                diagnostics: params.diagnostics,
                version: stored_version,
                published_at: Instant::now(),
            },
        );
        ctx.emit(LspEvent::DiagnosticsUpdated { path });
    }

    pub fn format_document(
        &self,
        path: PathBuf,
        options: FormattingOptions,
    ) -> Result<impl Future<Output = Result<Option<Vec<TextEdit>>>>> {
        let service = self.service()?;
        Ok(async move { service.text_document().format(&path, options).await })
    }

    pub fn hover(
        &self,
        path: PathBuf,
        position: Location,
    ) -> Result<impl Future<Output = Result<Option<HoverResult>>>> {
        let service = self.service()?;
        Ok(async move {
            service
                .text_document()
                .hover(&path, position.into_lsp())
                .await
        })
    }

    pub fn diagnostics_for_path(&self, path: &Path) -> Result<Option<&DocumentDiagnostics>> {
        Ok(self.diagnostics_by_path.get(path))
    }

    pub fn find_references(
        &self,
        path: PathBuf,
        position: Location,
    ) -> Result<impl Future<Output = Result<Vec<ReferenceLocation>>>> {
        let service = self.service()?;
        Ok(async move {
            service
                .text_document()
                .references(&path, position.into_lsp())
                .await
        })
    }
}

impl Entity for LspServerModel {
    type Event = LspEvent;
}

impl Drop for LspServerModel {
    fn drop(&mut self) {
        self.terminate();
    }
}

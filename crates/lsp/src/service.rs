use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use globset::{Glob, GlobMatcher};
use jsonrpc::{JsonRpcService, RequestId, ServerNotificationEvent};
use lsp_types::notification::{self, Notification};
use lsp_types::request::{self, Request};
use lsp_types::{
    CancelParams, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, FileChangeType, FileSystemWatcher,
    FormattingOptions, GlobPattern, GotoDefinitionParams, GotoDefinitionResponse, HoverParams,
    InitializeParams, InitializedParams, NumberOrString, OneOf, Position, ReferenceParams,
    RegistrationParams, RelativePattern, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, UnregistrationParams, VersionedTextDocumentIdentifier, WatchKind,
};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use simple_logger::SimpleLogger;
use warp_errors::report_error;
use warp_util::on_cancel::OnCancelFutureExt;

use crate::config::{lsp_uri_to_path, path_to_lsp_uri, LanguageId};
use crate::types::{
    HoverResult, LspDefinitionLocation, ReferenceLocation, TextDocumentContentChangeEvent,
    TextEdit, WatchedFileChangeEvent,
};
use crate::LspServerLogLevel;

/// Tracks the sync state for an open document.
#[derive(Debug, Clone)]
pub struct DocumentSyncState {
    /// The last buffer version that was successfully synced with the LSP server.
    /// None means the document was opened but no subsequent changes have been synced yet.
    pub last_synced_version: Option<usize>,
}

pub struct LspService {
    jsonrpc_service: JsonRpcService,
    server_capabilities: Option<lsp_types::ServerCapabilities>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    open_documents: Arc<Mutex<HashMap<PathBuf, DocumentSyncState>>>,
    watched_files_registry: Arc<Mutex<WatchedFilesRegistry>>,
    notify_tx: async_channel::Sender<ServerNotificationEvent>,
    #[cfg(not(target_arch = "wasm32"))]
    logger: Option<SimpleLogger>,
}

struct LspServerRequestHandler {
    watched_files_registry: Arc<Mutex<WatchedFilesRegistry>>,
}

impl LspServerRequestHandler {
    fn handle_request(&self, method: &str, params: Value, id: RequestId) -> Result<()> {
        match method {
            "client/registerCapability" => {
                let params = match serde_json::from_value::<RegistrationParams>(params) {
                    Ok(params) => params,
                    Err(e) => {
                        log::warn!(
                            "Failed to parse client/registerCapability params (id: {id:?}): {e}"
                        );
                        return Ok(());
                    }
                };

                for registration in params.registrations {
                    if registration.method != notification::DidChangeWatchedFiles::METHOD {
                        continue;
                    }

                    let Some(register_options) = registration.register_options else {
                        log::debug!(
                            "Ignoring didChangeWatchedFiles registration without options (id: {})",
                            registration.id
                        );
                        continue;
                    };

                    let options = match serde_json::from_value::<
                        DidChangeWatchedFilesRegistrationOptions,
                    >(register_options)
                    {
                        Ok(options) => options,
                        Err(e) => {
                            log::warn!(
                                "Failed to parse didChangeWatchedFiles registration options (id: {}): {e}",
                                registration.id
                            );
                            continue;
                        }
                    };

                    let mut registry = self
                        .watched_files_registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    registry.register(registration.id, options);
                }
            }
            "client/unregisterCapability" => {
                let params = match serde_json::from_value::<UnregistrationParams>(params) {
                    Ok(params) => params,
                    Err(e) => {
                        log::warn!(
                            "Failed to parse client/unregisterCapability params (id: {id:?}): {e}"
                        );
                        return Ok(());
                    }
                };

                let mut registry = self
                    .watched_files_registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());

                for unregistration in params.unregisterations {
                    if unregistration.method == notification::DidChangeWatchedFiles::METHOD {
                        registry.unregister(&unregistration.id);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}

impl LspService {
    /// Creates a new LspService with the given JsonRpcService.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn new(
        jsonrpc_service: JsonRpcService,
        notify_tx: async_channel::Sender<ServerNotificationEvent>,
        workspace_root: PathBuf,
        #[cfg(not(target_arch = "wasm32"))] logger: Option<SimpleLogger>,
    ) -> Result<Self> {
        let watched_files_registry =
            Arc::new(Mutex::new(WatchedFilesRegistry::new(workspace_root)));

        let server_request_handler = Arc::new(LspServerRequestHandler {
            watched_files_registry: watched_files_registry.clone(),
        });

        let server_request_handler_for_closure = server_request_handler.clone();
        jsonrpc_service.set_server_request_handler(move |method, params, id| {
            server_request_handler_for_closure.handle_request(&method, params, id)
        });

        let service = Self {
            jsonrpc_service,
            server_capabilities: None,
            open_documents: Arc::new(Mutex::new(HashMap::new())),
            watched_files_registry,
            notify_tx,
            #[cfg(not(target_arch = "wasm32"))]
            logger,
        };

        Ok(service)
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) async fn initialize(&mut self, params: InitializeParams) -> Result<()> {
        let response = self.send_request::<request::Initialize>(params).await?;
        self.server_capabilities = Some(response.capabilities);

        self.send_notification::<notification::Initialized>(InitializedParams {})?;
        self.subscribe::<notification::Progress>().await;
        self.subscribe::<notification::PublishDiagnostics>().await;

        log::info!("LSP initialized successfully and will now run startup tasks");
        Ok(())
    }

    pub(crate) async fn subscribe<N: notification::Notification>(&self) {
        self.jsonrpc_service
            .subscribe(N::METHOD.to_string(), self.notify_tx.clone())
            .await;
    }

    pub fn server_capabilities(&self) -> &Option<lsp_types::ServerCapabilities> {
        &self.server_capabilities
    }

    pub fn log_to_server_log(&self, level: LspServerLogLevel, message: impl Into<String>) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(logger) = &self.logger {
                logger.log(format!("[{level}] {}", message.into()));
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = (level, message.into());
        }
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        // Send LSP shutdown request first
        log::debug!("Sending LSP shutdown request");
        match self.send_request::<request::Shutdown>(()).await {
            Ok(_) => log::debug!("LSP shutdown request completed successfully"),
            Err(e) => {
                log::warn!("LSP shutdown request failed: {e}");
                // Continue with exit notification even if shutdown request fails
            }
        }

        // Send exit notification
        log::debug!("Sending LSP exit notification");
        if let Err(e) = self.send_notification::<notification::Exit>(()) {
            log::warn!("Failed to send LSP exit notification: {e}");
        }

        // Finally, shutdown the transport (kill process if needed)
        self.jsonrpc_service
            .shutdown(std::time::Duration::from_secs(5))
            .await?;

        log::debug!("LSP shutdown sequence completed");
        Ok(())
    }

    /// Get a handle to text-document related operations and requests
    pub fn text_document(&self) -> TextDocumentService<'_> {
        TextDocumentService {
            service: self,
            open_documents: self.open_documents.clone(),
        }
    }

    pub fn workspace_watched_files_changed(
        &self,
        events: Vec<WatchedFileChangeEvent>,
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let watched_files_registry = self
            .watched_files_registry
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire lock on watched files registry"))?;

        // If the server hasn't registered any watchers yet, don't send notifications.
        if watched_files_registry.is_empty() {
            return Ok(());
        }

        let mut changes = Vec::new();
        for event in events {
            if !watched_files_registry.matches(&event) {
                continue;
            }

            match event.into_lsp() {
                Ok(event) => changes.push(event),
                Err(e) => log::warn!("Failed to convert file event: {e}"),
            }
        }

        drop(watched_files_registry);

        if changes.is_empty() {
            return Ok(());
        }

        self.send_notification::<notification::DidChangeWatchedFiles>(DidChangeWatchedFilesParams {
            changes,
        })
    }

    async fn send_request_internal(&self, method: String, params: Value) -> Result<Value> {
        let request_id = self.jsonrpc_service.next_id();

        let request = self
            .jsonrpc_service
            .send_request(request_id, method, params);

        request
            .on_cancel(move || {
                self.cancel_request(request_id);
            })
            .await
    }

    fn send_notification<N: Notification>(&self, params: N::Params) -> Result<()> {
        let params = serde_json::to_value(params)?;
        self.jsonrpc_service
            .send_notification(N::METHOD.to_string(), params)
    }

    async fn send_request<R: Request>(&self, params: R::Params) -> Result<R::Result> {
        let params = serde_json::to_value(params)?;
        let request = self.send_request_internal(R::METHOD.to_string(), params);
        let response = request.await?;
        serde_json::from_value::<R::Result>(response)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {e}"))
    }

    fn cancel_request(&self, request_id: RequestId) {
        let cancel = serde_json::to_value(CancelParams {
            id: NumberOrString::Number(request_id),
        })
        .expect("Failed to serialize cancel params");

        if let Err(e) = self
            .jsonrpc_service
            .send_notification(notification::Cancel::METHOD.to_string(), cancel)
        {
            report_error!(e.context("Failed to send cancel notification"));
        }
    }
}

struct WatchedFilesRegistry {
    workspace_root: PathBuf,
    registrations: HashMap<String, Vec<GlobFileMatcher>>,
}

impl WatchedFilesRegistry {
    fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            registrations: HashMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }

    fn register(
        &mut self,
        registration_id: String,
        options: DidChangeWatchedFilesRegistrationOptions,
    ) {
        let watchers = options
            .watchers
            .into_iter()
            .filter_map(|watcher| self.compile_watcher(watcher))
            .collect();

        self.registrations.insert(registration_id, watchers);
    }

    fn unregister(&mut self, registration_id: &str) {
        self.registrations.remove(registration_id);
    }

    fn matches(&self, event: &WatchedFileChangeEvent) -> bool {
        let Ok(path_relative) = event.path.strip_prefix(&self.workspace_root) else {
            return false;
        };

        let path_relative = warp_util::path::normalize_relative_path_for_glob(path_relative);

        self.registrations
            .values()
            .flatten()
            .any(|watcher| watcher.matches(&path_relative, event.typ))
    }

    fn compile_watcher(&self, watcher: FileSystemWatcher) -> Option<GlobFileMatcher> {
        let FileSystemWatcher { glob_pattern, kind } = watcher;

        let pattern = self.pattern_for_glob_pattern(glob_pattern)?;

        let matcher = match Glob::new(&pattern) {
            Ok(glob) => {
                let matcher: GlobMatcher = glob.compile_matcher();
                matcher
            }
            Err(e) => {
                log::warn!("Failed to compile watched-files glob pattern {pattern:?}: {e}");
                return None;
            }
        };

        Some(GlobFileMatcher { matcher, kind })
    }

    fn pattern_for_glob_pattern(&self, glob_pattern: GlobPattern) -> Option<String> {
        match glob_pattern {
            GlobPattern::String(pattern) => Some(pattern),
            GlobPattern::Relative(relative_pattern) => {
                self.pattern_for_relative_pattern(relative_pattern)
            }
        }
    }

    fn pattern_for_relative_pattern(&self, relative_pattern: RelativePattern) -> Option<String> {
        let RelativePattern { base_uri, pattern } = relative_pattern;

        let base_path = match base_uri {
            OneOf::Left(folder) => folder.uri,
            OneOf::Right(uri) => uri,
        };

        let base_path = match lsp_uri_to_path(&base_path) {
            Ok(path) => path,
            Err(e) => {
                let base_uri = base_path.as_str();
                log::warn!("Failed to resolve relativePattern baseUri {base_uri}: {e}");
                return None;
            }
        };

        let Ok(base_relative) = base_path.strip_prefix(&self.workspace_root) else {
            return None;
        };

        // Normalize to forward slashes so glob patterns and event paths are comparable across platforms (esp. Windows).
        let prefix = warp_util::path::normalize_relative_path_for_glob(base_relative);

        if prefix.is_empty() {
            Some(pattern)
        } else {
            Some(format!("{prefix}/{pattern}"))
        }
    }
}

#[derive(Debug)]
struct GlobFileMatcher {
    matcher: GlobMatcher,
    kind: Option<WatchKind>,
}

impl GlobFileMatcher {
    fn matches(&self, path_relative: &str, change_type: FileChangeType) -> bool {
        if let Some(kind) = self.kind {
            if let Some(required) = watch_kind_for_change_type(change_type) {
                if !kind.contains(required) {
                    return false;
                }
            }
        }

        self.matcher.is_match(path_relative)
    }
}

fn watch_kind_for_change_type(change_type: FileChangeType) -> Option<WatchKind> {
    match change_type {
        FileChangeType::CREATED => Some(WatchKind::Create),
        FileChangeType::CHANGED => Some(WatchKind::Change),
        FileChangeType::DELETED => Some(WatchKind::Delete),
        _ => None,
    }
}

/// Encapsulates text-document related operations and requests. This exists only for bookkeeping
/// and discoverability.
pub struct TextDocumentService<'a> {
    service: &'a LspService,
    open_documents: Arc<Mutex<HashMap<PathBuf, DocumentSyncState>>>,
}

impl<'a> TextDocumentService<'a> {
    pub fn document_is_open(&self, path: &PathBuf) -> Result<bool> {
        let open_documents = self
            .open_documents
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire lock on open documents"))?;
        Ok(open_documents.contains_key(path))
    }

    /// Returns the last synced buffer version for the document, if it is open.
    /// Returns None if the document is not open.
    pub fn last_synced_version(&self, path: &PathBuf) -> Result<Option<usize>> {
        let open_documents = self
            .open_documents
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire lock on open documents"))?;
        Ok(open_documents
            .get(path)
            .and_then(|state| state.last_synced_version))
    }

    pub async fn did_open(
        &self,
        path: &Path,
        content: String,
        initial_version: usize,
    ) -> Result<()> {
        {
            // Drop the guard before await
            let mut open_documents = self
                .open_documents
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire lock on open documents"))?;
            // Use entry API to check if already present
            if open_documents.contains_key(&path.to_path_buf()) {
                return Ok(());
            }
            open_documents.insert(
                path.to_path_buf(),
                DocumentSyncState {
                    last_synced_version: Some(initial_version),
                },
            );
        }

        self.service.log_to_server_log(
            LspServerLogLevel::Debug,
            format!(
                "didOpen -> server: file={} version={initial_version}",
                path.display()
            ),
        );

        // Determine language ID from the file path
        let language_id = LanguageId::from_path(path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Could not determine language ID for file: {}",
                    path.display()
                )
            })?
            .lsp_language_identifier()
            .to_owned();

        let did_open_params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: path_to_lsp_uri(path)?,
                language_id,
                version: initial_version as i32,
                text: content,
            },
        };

        self.service
            .send_notification::<notification::DidOpenTextDocument>(did_open_params)
    }

    pub async fn did_close(&self, path: &Path) -> Result<()> {
        {
            // Drop the guard before await
            let mut open_documents = self
                .open_documents
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire lock on open documents"))?;
            if open_documents.remove(&path.to_path_buf()).is_none() {
                return Ok(());
            }
        }

        let did_close_params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: path_to_lsp_uri(path)?,
            },
        };

        self.service
            .send_notification::<notification::DidCloseTextDocument>(did_close_params)
    }

    pub async fn did_change(
        &self,
        path: &Path,
        version: i32,
        deltas: Vec<TextDocumentContentChangeEvent>,
    ) -> Result<()> {
        {
            // Check if document is open
            let mut open_documents = self
                .open_documents
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire lock on open documents"))?;

            let Some(state) = open_documents.get_mut(path) else {
                return Ok(());
            };

            state.last_synced_version = Some(version as usize);
        }

        let did_change_params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: path_to_lsp_uri(path)?,
                version,
            },
            content_changes: deltas.into_iter().map(|delta| delta.into_lsp()).collect(),
        };

        self.service
            .send_notification::<notification::DidChangeTextDocument>(did_change_params)
    }

    pub async fn definition(
        &self,
        path: &Path,
        position: Position,
    ) -> anyhow::Result<Vec<LspDefinitionLocation>> {
        let uri = path_to_lsp_uri(path)?;

        let definition_params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .service
            .send_request::<request::GotoDefinition>(definition_params)
            .await;

        if let Err(e) = &result {
            self.service.log_to_server_log(
                LspServerLogLevel::Error,
                format!("textDocument/definition failed: {e}"),
            );
        }

        // The LSP spec says textDocument/definition can return null when no definition is found
        // Handle this case explicitly since GotoDefinitionResponse doesn't deserialize null properly
        let Some(response) = result? else {
            return Err(anyhow::anyhow!("No definition found or LSP busy"));
        };

        match response {
            GotoDefinitionResponse::Scalar(location) => Ok(vec![location.into()]),
            GotoDefinitionResponse::Array(locations) => {
                Ok(locations.into_iter().map(Into::into).collect())
            }
            GotoDefinitionResponse::Link(locations) => {
                Ok(locations.into_iter().map(Into::into).collect())
            }
        }
    }

    pub async fn format(
        &self,
        path: &Path,
        options: FormattingOptions,
    ) -> anyhow::Result<Option<Vec<TextEdit>>> {
        let format_params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: path_to_lsp_uri(path)?,
            },
            options,
            work_done_progress_params: Default::default(),
        };

        let result = self
            .service
            .send_request::<request::Formatting>(format_params)
            .await;

        if let Err(e) = &result {
            self.service.log_to_server_log(
                LspServerLogLevel::Error,
                format!("textDocument/formatting failed: {e}"),
            );
        }

        // The LSP spec says textDocument/formatting can return null when formatting is not supported
        result.map(|edits_option| {
            edits_option.map(|text_edits| text_edits.into_iter().map(Into::into).collect())
        })
    }

    pub async fn hover(
        &self,
        path: &Path,
        position: Position,
    ) -> anyhow::Result<Option<HoverResult>> {
        let uri = path_to_lsp_uri(path)?;

        let hover_params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: Default::default(),
        };

        let result = self
            .service
            .send_request::<request::HoverRequest>(hover_params)
            .await;

        if let Err(e) = &result {
            self.service.log_to_server_log(
                LspServerLogLevel::Error,
                format!("textDocument/hover failed: {e}"),
            );
        }

        Ok(result?.map(Into::into))
    }

    pub async fn references(
        &self,
        path: &Path,
        position: Position,
    ) -> anyhow::Result<Vec<ReferenceLocation>> {
        let uri = path_to_lsp_uri(path)?;

        let reference_params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
        };

        let result = self
            .service
            .send_request::<request::References>(reference_params)
            .await;

        if let Err(e) = &result {
            self.service.log_to_server_log(
                LspServerLogLevel::Error,
                format!("textDocument/references failed: {e}"),
            );
        }

        // The LSP spec says textDocument/references can return null when no references are found
        Ok(result?
            .unwrap_or_default()
            .into_iter()
            .filter_map(|loc| ReferenceLocation::try_from(loc).ok())
            .collect())
    }
}

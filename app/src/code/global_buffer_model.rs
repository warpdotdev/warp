#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use bimap::BiMap;

use futures_util::stream::AbortHandle;
use lsp::types::TextDocumentContentChangeEvent;
use lsp::{LspManagerModel, LspServerLogLevel, LspServerModel};
use string_offset::{ByteOffset, CharOffset};
use vec1::vec1;
use warp_core::{features::FeatureFlag, safe_error};
use warp_editor::content::buffer::{Buffer, ToBufferCharOffset};
use warp_editor::content::diff::{text_diff, TextDiff};
use warp_editor::content::edit::PreciseDelta;
use warp_editor::content::version::BufferVersion;
use warp_util::content_version::ContentVersion;
use warp_util::file::{FileId, FileLoadError, FileSaveError};
use warp_util::host_id::HostId;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle};

use remote_server::manager::RemoteServerManager;

use super::buffer_location::{LocalOrRemotePath, SyncClock};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use lsp::LspManagerModelEvent;
        use warp_files::{FileModelEvent, FileModel};
        use warp_editor::content::text::IndentBehavior;
        use warp_editor::content::text::IndentUnit;
        use warp_editor::content::buffer::EditOrigin;
    }
}

/// State for a shared buffer including the file ID and buffer handle.
#[derive(Debug, Clone)]
pub struct BufferState {
    pub file_id: FileId,
    pub buffer: ModelHandle<Buffer>,
}

impl BufferState {
    pub fn new(file_id: FileId, buffer: ModelHandle<Buffer>) -> Self {
        Self { file_id, buffer }
    }
}

/// Tracks an active background diff parsing operation.
struct PendingDiffParse {
    abort_handle: AbortHandle,
}

/// How long to wait after the last keystroke before sending a batched
/// `BufferEdit` to the remote server. Long enough to coalesce rapid
/// keystrokes, short enough for the remote view to feel responsive.
const REMOTE_EDIT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Accumulates incremental edits for a single remote buffer during a
/// debounce window before sending them as a single `BufferEdit` message.
struct PendingEditBatch {
    /// The server version known when the first edit in this batch was captured.
    expected_server_version: u64,
    /// Accumulated `TextEdit`s — each edit's offsets reference the buffer state
    /// AFTER all previous edits in this batch have been applied.
    edits: Vec<remote_server::proto::TextEdit>,
    /// The client version to send (updated on each append).
    latest_client_version: ContentVersion,
    /// Handle to cancel the debounce timer when a new edit arrives or the
    /// batch is flushed/discarded.
    debounce_timer: Option<AbortHandle>,
}

impl PendingEditBatch {
    /// Flush this batch: send accumulated edits as a single `BufferEdit`
    /// to the remote server and cancel the debounce timer.
    ///
    /// Note: `send_buffer_edit` uses best-effort `try_send` on an unbounded
    /// channel, so it can only fail if the connection is closed (in which
    /// case the subsequent `save_buffer` would also fail).
    fn flush(self, client: &remote_server::client::RemoteServerClient, path: &str) {
        if let Some(timer) = &self.debounce_timer {
            timer.abort();
        }
        if self.edits.is_empty() {
            return;
        }
        log::debug!(
            "[remote-buffer] Flushing batched BufferEdit: path={path} \
             expected_sv={} new_cv={} edit_count={}",
            self.expected_server_version,
            self.latest_client_version.as_u64(),
            self.edits.len()
        );
        client.send_buffer_edit(
            path.to_string(),
            self.expected_server_version,
            self.latest_client_version.as_u64(),
            self.edits,
        );
    }

    /// Discard this batch without sending, cancelling the debounce timer.
    fn discard(self) {
        if let Some(timer) = &self.debounce_timer {
            timer.abort();
            log::debug!(
                "[remote-buffer] Discarded pending batch: \
                 expected_sv={} edit_count={}",
                self.expected_server_version,
                self.edits.len()
            );
        }
    }
}

/// Describes the backing store for a buffer's content.
enum BufferSource {
    /// Backed by the local filesystem (existing behavior).
    Local {
        base_content_version: Option<ContentVersion>,
        /// The first ever content version when the file was loaded.
        /// Used to avoid sending a spurious didChange to LSP on initial load.
        initial_content_version: Option<ContentVersion>,
    },
    /// Backed by a remote filesystem over the remote server protocol.
    Remote {
        remote_path: RemotePath,
        /// `None` while waiting for the `OpenBufferResponse`; `Some` once loaded.
        sync_clock: Option<SyncClock>,
        /// Pending batched edits awaiting the debounce timer. `None` when idle.
        pending_batch: Option<PendingEditBatch>,
    },
    /// Local file managed by the remote-server daemon.
    /// Owns the SyncClock for version tracking. Connection tracking
    /// is handled by ServerModel, not here — the buffer is a file-level
    /// concept shared across connections.
    ServerLocal {
        sync_clock: SyncClock,
        base_content_version: Option<ContentVersion>,
        initial_content_version: Option<ContentVersion>,
    },
}

struct InternalBufferState {
    buffer: WeakModelHandle<Buffer>,
    /// Tracks the latest buffer version we've attempted to sync with LSP.
    /// Used to detect if previous versions were synced successfully.
    latest_buffer_version: Option<usize>,
    /// Tracks any active background diff parsing for auto-reload.
    pending_diff_parse: Option<PendingDiffParse>,
    source: BufferSource,
}

impl InternalBufferState {
    /// Returns the base content version for local buffers, `None` for remote.
    ///
    /// Remote buffers return `None` because they don't use the file-watcher
    /// auto-reload path (which is `local_fs`-only). Version tracking for
    /// remote buffers is handled by `SyncClock` instead.
    fn base_content_version(&self) -> Option<ContentVersion> {
        match &self.source {
            BufferSource::Local {
                base_content_version,
                ..
            }
            | BufferSource::ServerLocal {
                base_content_version,
                ..
            } => *base_content_version,
            BufferSource::Remote { .. } => None,
        }
    }

    /// Sets the base content version. Applicable to Local and ServerLocal buffers.
    fn set_base_content_version(&mut self, version: ContentVersion) {
        match &mut self.source {
            BufferSource::Local {
                base_content_version,
                ..
            }
            | BufferSource::ServerLocal {
                base_content_version,
                ..
            } => {
                *base_content_version = Some(version);
            }
            BufferSource::Remote { .. } => {}
        }
    }

    /// Returns the initial content version for local/server-local buffers,
    /// `None` for remote.
    ///
    /// Remote buffers return `None` because the initial-version guard is only
    /// needed to avoid a spurious LSP `didChange` on first load. Remote
    /// buffers don't interact with the local LSP (it runs on the server side),
    /// so no guard is necessary.
    fn initial_content_version(&self) -> Option<ContentVersion> {
        match &self.source {
            BufferSource::Local {
                initial_content_version,
                ..
            }
            | BufferSource::ServerLocal {
                initial_content_version,
                ..
            } => *initial_content_version,
            BufferSource::Remote { .. } => None,
        }
    }

    /// Sets the initial content version. Applicable to Local and ServerLocal buffers.
    fn set_initial_content_version(&mut self, version: ContentVersion) {
        match &mut self.source {
            BufferSource::Local {
                initial_content_version,
                ..
            }
            | BufferSource::ServerLocal {
                initial_content_version,
                ..
            } => {
                *initial_content_version = Some(version);
            }
            BufferSource::Remote { .. } => {}
        }
    }

    /// Whether this buffer has been loaded (has content).
    fn is_loaded(&self) -> bool {
        match &self.source {
            BufferSource::Local {
                base_content_version,
                ..
            }
            | BufferSource::ServerLocal {
                base_content_version,
                ..
            } => base_content_version.is_some(),
            // Remote buffers are loaded once the OpenBufferResponse arrives
            // and populates the sync clock.
            BufferSource::Remote { sync_clock, .. } => sync_clock.is_some(),
        }
    }
}

pub enum GlobalBufferModelEvent {
    BufferLoaded {
        file_id: FileId,
        content_version: ContentVersion,
    },
    FailedToLoad {
        file_id: FileId,
        error: Rc<FileLoadError>,
    },
    BufferUpdatedFromFileEvent {
        file_id: FileId,
        success: bool,
        content_version: ContentVersion,
    },
    FileSaved {
        file_id: FileId,
    },
    FailedToSave {
        file_id: FileId,
        error: Rc<FileSaveError>,
    },
    /// A remote buffer update conflicted with local edits.
    /// The UI should present a resolution dialog.
    RemoteBufferConflict {
        file_id: FileId,
    },
    /// A server-local buffer was updated from a file-watcher event.
    /// Carries the incremental diff edits for the ServerModel to push
    /// to connected clients as `BufferUpdatedPush`.
    ServerLocalBufferUpdated {
        file_id: FileId,
        /// Incremental edits with 1-indexed character offsets (matching `CharOffset`).
        edits: Vec<CharOffsetEdit>,
        new_server_version: ContentVersion,
        expected_client_version: ContentVersion,
    },
}

impl GlobalBufferModelEvent {
    pub fn file_id(&self) -> FileId {
        match self {
            GlobalBufferModelEvent::BufferLoaded { file_id, .. }
            | GlobalBufferModelEvent::FailedToLoad { file_id, .. }
            | GlobalBufferModelEvent::BufferUpdatedFromFileEvent { file_id, .. }
            | GlobalBufferModelEvent::FileSaved { file_id, .. }
            | GlobalBufferModelEvent::FailedToSave { file_id, .. }
            | GlobalBufferModelEvent::RemoteBufferConflict { file_id, .. }
            | GlobalBufferModelEvent::ServerLocalBufferUpdated { file_id, .. } => *file_id,
        }
    }
}

/// A text edit using 1-indexed character offsets (matching `CharOffset`).
///
/// Used to carry incremental edits in `ServerLocalBufferUpdated` events
/// and `handle_buffer_updated_push` without coupling `GlobalBufferModel`
/// to proto types. Offsets use the same 1-indexed coordinate system as
/// the buffer's `CharOffset`, so no conversion is needed at the boundary.
pub struct CharOffsetEdit {
    pub start: CharOffset,
    pub end: CharOffset,
    pub text: String,
}

/// Global singleton model for managing shared buffers across editors.
///
/// This allows multiple editors to share the same buffer when editing the same file,
/// enabling consistent content synchronization and more efficient memory usage.
pub struct GlobalBufferModel {
    location_to_id: BiMap<LocalOrRemotePath, FileId>,
    buffers: HashMap<FileId, InternalBufferState>,
}

impl GlobalBufferModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        #[cfg(feature = "local_fs")]
        _ctx.subscribe_to_model(&FileModel::handle(_ctx), Self::handle_file_model_events);

        #[cfg(feature = "local_fs")]
        _ctx.subscribe_to_model(
            &LspManagerModel::handle(_ctx),
            Self::handle_lsp_manager_events,
        );

        // Subscribe to remote buffer updates from the RemoteServerManager.
        #[cfg(feature = "local_tty")]
        if FeatureFlag::SshRemoteServer.is_enabled() {
            use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
            let mgr = RemoteServerManager::handle(_ctx);
            _ctx.subscribe_to_model(&mgr, |me, event, ctx| match event {
                RemoteServerManagerEvent::BufferUpdated {
                    host_id,
                    path,
                    new_server_version,
                    expected_client_version,
                    edits,
                } => {
                    let char_edits: Vec<_> = edits
                        .iter()
                        .map(|e| CharOffsetEdit {
                            start: CharOffset::from(e.start_offset as usize),
                            end: CharOffset::from(e.end_offset as usize),
                            text: e.text.clone(),
                        })
                        .collect();
                    me.handle_buffer_updated_push(
                        host_id,
                        path,
                        *new_server_version,
                        *expected_client_version,
                        &char_edits,
                        ctx,
                    );
                }
                RemoteServerManagerEvent::BufferConflictDetected { host_id, path } => {
                    me.handle_buffer_conflict_detected(host_id, path, ctx);
                }
                _ => {}
            });
        }

        Self {
            location_to_id: BiMap::new(),
            buffers: HashMap::new(),
        }
    }

    /// Scan through all buffers and deallocate any that are no longer in use.
    pub fn remove_deallocated_buffers(&mut self, ctx: &mut ModelContext<Self>) {
        let ids_to_remove: HashSet<FileId> = self
            .buffers
            .iter()
            .filter_map(|(id, state)| {
                if state.buffer.upgrade(ctx).is_none() {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        if ids_to_remove.is_empty() {
            return;
        }

        // Collect paths for didClose before removing entries.
        let paths_to_close: Vec<PathBuf> = ids_to_remove
            .iter()
            .filter_map(|id| match self.location_to_id.get_by_right(id) {
                Some(LocalOrRemotePath::Local(path)) => Some(path.clone()),
                Some(LocalOrRemotePath::Remote(_)) | None => None,
            })
            .collect();

        for path in &paths_to_close {
            self.close_document_with_lsp(path, ctx);
        }

        for id in &ids_to_remove {
            self.location_to_id.remove_by_right(id);
        }

        for id in ids_to_remove {
            self.buffers.remove(&id);

            #[cfg(feature = "local_fs")]
            {
                let file_model = FileModel::handle(ctx);
                file_model.update(ctx, |file_model, ctx| {
                    file_model.cancel(id);
                    file_model.unsubscribe(id, ctx);
                });
            }
        }
    }

    pub fn buffer_loaded(&self, file_id: FileId) -> bool {
        self.buffers
            .get(&file_id)
            .map(|state| state.is_loaded())
            .unwrap_or(false)
    }

    fn cleanup_file_id(&mut self, file_id: FileId, _ctx: &mut ModelContext<Self>) {
        // Send didClose before removing the entry.
        if let Some((LocalOrRemotePath::Local(path), _)) =
            self.location_to_id.remove_by_right(&file_id)
        {
            self.close_document_with_lsp(&path, _ctx);
        }

        self.buffers.remove(&file_id);

        #[cfg(feature = "local_fs")]
        {
            let file_model = FileModel::handle(_ctx);
            file_model.update(_ctx, |file_model, ctx| {
                file_model.cancel(file_id);
                file_model.unsubscribe(file_id, ctx);
            });
        }
    }

    /// Returns the buffer handle if it is 1) still exists + active 2) loaded.
    fn buffer_handle_for_id(
        &mut self,
        file_id: FileId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<ModelHandle<Buffer>> {
        let state = self.buffers.get(&file_id)?;

        // If the buffer hasn't been loaded yet, don't return a model handle.
        if !state.is_loaded() {
            log::info!("Cannot return handle for unloaded buffers");
            return None;
        }

        match state.buffer.upgrade(ctx) {
            Some(handle) => Some(handle),
            None => {
                // Clean up deallocated buffers.
                self.cleanup_file_id(file_id, ctx);
                None
            }
        }
    }

    /// Once we finish reading the file's content from the disk, populate the buffer with the content.
    /// For initial load (is_loaded_from_file_system == true), this is synchronous.
    /// For auto-reload (is_loaded_from_file_system == false), this spawns a background task for diff computation.
    /// Exposed as `pub(crate)` so tests can populate buffer content
    /// without going through the async `FileModel` load path.
    pub(crate) fn populate_buffer_with_read_content(
        &mut self,
        file_id: FileId,
        content: &str,
        base_version: ContentVersion,
        new_version: ContentVersion,
        is_initial_load: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.buffers.get_mut(&file_id) else {
            return;
        };

        let Some(buffer) = state.buffer.upgrade(ctx) else {
            self.cleanup_file_id(file_id, ctx);
            log::warn!("Cannot populate buffer with content due to deallocated model handle");
            return;
        };

        if is_initial_load {
            // Initial load: use synchronous replace_all since there's nothing to preserve
            buffer.update(ctx, |buffer, ctx| {
                buffer.replace_all(content, ctx);
                buffer.set_version(new_version);
            });

            state.set_base_content_version(new_version);

            ctx.emit(GlobalBufferModelEvent::BufferLoaded {
                file_id,
                content_version: new_version,
            });
        } else if FeatureFlag::IncrementalAutoReload.is_enabled() {
            // Auto-reload: spawn background task for diff computation
            Self::start_background_diff_parse(
                file_id,
                state,
                buffer,
                content,
                base_version,
                new_version,
                ctx,
            );
        } else {
            // Fallback: synchronous replace_all (non-incremental)
            buffer.update(ctx, |buffer, ctx| {
                buffer.replace_all(content, ctx);
                buffer.set_version(new_version);
            });

            state.set_base_content_version(new_version);

            ctx.emit(GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                file_id,
                success: true,
                content_version: new_version,
            });
        }
    }

    /// Spawns a background task to compute the diff between current buffer content and new content.
    /// On completion, applies the diff edits to the buffer.
    fn start_background_diff_parse(
        file_id: FileId,
        state: &mut InternalBufferState,
        buffer: ModelHandle<Buffer>,
        new_content: &str,
        base_version: ContentVersion,
        new_version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) {
        // Abort any existing diff parse for this file
        if let Some(pending) = state.pending_diff_parse.take() {
            pending.abort_handle.abort();
        }

        // Move owned strings to the background thread
        let old_text = buffer.as_ref(ctx).text().into_string();
        let new_content_owned = new_content.to_string();

        let handle = ctx.spawn(
            async move { text_diff(&old_text, &new_content_owned).await },
            move |me, diff: TextDiff, ctx| {
                me.apply_diff_result(file_id, diff, base_version, new_version, ctx);
            },
        );

        // Store the abort handle so we can cancel if a newer update arrives
        state.pending_diff_parse = Some(PendingDiffParse {
            abort_handle: handle.abort_handle(),
        });
    }

    /// Called when background diff parsing completes. Applies the diff edits to the buffer.
    fn apply_diff_result(
        &mut self,
        file_id: FileId,
        diff: TextDiff,
        base_version: ContentVersion,
        new_version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.buffers.get_mut(&file_id) else {
            return;
        };

        // Clear the pending diff parse state
        state.pending_diff_parse = None;

        let Some(buffer) = state.buffer.upgrade(ctx) else {
            self.cleanup_file_id(file_id, ctx);
            return;
        };

        // Verify the buffer still matches the expected base version.
        // This also correctly handles the case where a client edit arrives
        // during the background diff parse: apply_client_edit modifies the
        // buffer version, so this check will fail and we discard the stale
        // diff rather than incorrectly bumping the server version.
        if !buffer.as_ref(ctx).version_match(&base_version) {
            log::info!("Buffer version changed during diff parsing, aborting apply");
            ctx.emit(GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                file_id,
                success: false,
                content_version: base_version,
            });
            return;
        }

        let is_server_local = matches!(state.source, BufferSource::ServerLocal { .. });

        // For ServerLocal buffers, convert byte-range edits to 1-indexed
        // char-offset edits BEFORE applying the diff, because the byte
        // ranges in diff.edits reference the old (pre-edit) buffer content.
        // Uses the buffer's native byte→char offset conversion.
        let char_offset_edits: Option<Vec<CharOffsetEdit>> = if is_server_local {
            let buffer_ref = buffer.as_ref(ctx);
            Some(
                diff.edits
                    .iter()
                    .map(|(range, text)| {
                        // +1: 0-indexed text byte offset → 1-indexed buffer byte offset
                        let start =
                            ByteOffset::from(range.start + 1).to_buffer_char_offset(buffer_ref);
                        let end = ByteOffset::from(range.end + 1).to_buffer_char_offset(buffer_ref);
                        CharOffsetEdit {
                            start,
                            end,
                            text: text.clone(),
                        }
                    })
                    .collect(),
            )
        } else {
            None
        };

        // Apply the diff edits
        buffer.update(ctx, |buffer, ctx| {
            if diff.is_empty() {
                // No actual changes to content, but still need to update version
                buffer.set_version(new_version);
                return;
            }
            let char_edits = diff.to_char_offset_edits(buffer);
            buffer.insert_at_char_offset_ranges(char_edits, new_version, ctx);
        });

        state.set_base_content_version(new_version);

        if let Some(char_offset_edits) = char_offset_edits {
            // Skip broadcasting empty edits — the file-watcher detected a write
            // but the content is identical (e.g. after a save). Sending an empty
            // BufferUpdatedPush would cause clients to advance base_content_version
            // without updating the buffer version, creating a spurious mismatch.
            if !char_offset_edits.is_empty() {
                if let BufferSource::ServerLocal { sync_clock, .. } = &mut state.source {
                    let new_sv = sync_clock.bump_server();
                    ctx.emit(GlobalBufferModelEvent::ServerLocalBufferUpdated {
                        file_id,
                        edits: char_offset_edits,
                        new_server_version: new_sv,
                        expected_client_version: sync_clock.client_version,
                    });
                }
            }
        } else {
            ctx.emit(GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                file_id,
                success: true,
                content_version: new_version,
            });
        }
    }

    #[cfg(feature = "local_fs")]
    fn handle_file_model_events(&mut self, event: &FileModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            FileModelEvent::FileLoaded {
                content,
                id,
                version,
            } => {
                // Only set the initial_content_version on first file load.
                if let Some(state) = self.buffers.get_mut(id) {
                    state.set_initial_content_version(*version);
                }

                // For initial load, base_version and new_version are the same
                self.populate_buffer_with_read_content(*id, content, *version, *version, true, ctx);
            }
            FileModelEvent::FailedToLoad { id, error } => {
                ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                    file_id: *id,
                    error: error.clone(),
                });
            }
            FileModelEvent::FileUpdated {
                id,
                content,
                base_version,
                new_version,
            } => {
                if let Some(buffer) = self.buffer_handle_for_id(*id, ctx) {
                    if buffer.as_ref(ctx).version_match(base_version) {
                        self.populate_buffer_with_read_content(
                            *id,
                            content,
                            *base_version,
                            *new_version,
                            false,
                            ctx,
                        );
                    } else {
                        // Buffer version doesn't match the event's base_version.
                        // Check if the buffer has no user edits (matches our internal
                        // base_content_version). If so, it's safe to start a fresh
                        // diff parse from the actual buffer version to the new content.
                        let internal_base_version = self
                            .buffers
                            .get(id)
                            .and_then(|state| state.base_content_version());
                        let has_no_user_edits = internal_base_version
                            .is_some_and(|v| buffer.as_ref(ctx).version_match(&v));

                        if has_no_user_edits {
                            // No user edits: safe to reload from the actual buffer
                            // version. This handles both:
                            log::info!(
                                "Starting fresh diff parse for file update (no user edits, \
                                 internal base {:?}, event base {:?})",
                                internal_base_version,
                                *base_version
                            );
                            let actual_version = buffer.as_ref(ctx).version();
                            self.populate_buffer_with_read_content(
                                *id,
                                content,
                                actual_version,
                                *new_version,
                                false,
                                ctx,
                            );
                        } else {
                            log::info!("Not updating global buffer due to version conflict");

                            // Abort any pending diff parse since the buffer has
                            // user edits that we must not overwrite.
                            if let Some(state) = self.buffers.get_mut(id) {
                                if let Some(pending) = state.pending_diff_parse.take() {
                                    pending.abort_handle.abort();
                                }
                            }

                            if internal_base_version != Some(*base_version) {
                                log::warn!(
                                    "Internal global buffer base version {:?} mismatches file model base version {:?}",
                                    internal_base_version,
                                    *base_version
                                );
                            }

                            ctx.emit(GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                                file_id: *id,
                                success: false,
                                content_version: *base_version,
                            });
                        }
                    }
                }
            }
            FileModelEvent::FileSaved { id, version } => {
                // Make sure base content version is updated after a save is performed.
                // This avoids us flagging the incoming update from file watcher as conflict changes.
                if let Some(state) = self.buffers.get_mut(id) {
                    state.set_base_content_version(*version);
                }
                ctx.emit(GlobalBufferModelEvent::FileSaved { file_id: *id });
            }
            FileModelEvent::FailedToSave { id, error } => {
                ctx.emit(GlobalBufferModelEvent::FailedToSave {
                    file_id: *id,
                    error: error.clone(),
                });
            }
        }
    }

    /// Save the content of a tracked buffer.
    ///
    /// For local buffers, saves to disk via `FileModel`.
    /// For remote buffers, flushes any pending edit batch first, then sends
    /// a `SaveBuffer` RPC to the remote server.
    #[cfg(feature = "local_fs")]
    pub fn save(
        &mut self,
        file_id: FileId,
        content: String,
        version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        // Check if this is a remote buffer — save via the remote server RPC.
        if let Some(state) = self.buffers.get_mut(&file_id) {
            if let BufferSource::Remote {
                remote_path,
                pending_batch,
                ..
            } = &mut state.source
            {
                let host_id = remote_path.host_id.clone();
                let path = remote_path.path.as_str().to_string();
                let manager = RemoteServerManager::handle(ctx);
                let Some(client) = manager.as_ref(ctx).client_for_host(&host_id).cloned() else {
                    safe_error!(
                        safe: ("[remote-buffer] No remote server client at buffer save time"),
                        full: ("[remote-buffer] No remote server client for save: host={host_id:?}")
                    );
                    return Err(FileSaveError::RemoteError(
                        "No remote server client available".to_string(),
                    ));
                };

                // Flush any pending edit batch so the server has the latest
                // content before persisting to disk.
                if let Some(batch) = pending_batch.take() {
                    batch.flush(&client, &path);
                }

                ctx.spawn(
                    async move { client.save_buffer(path).await.map_err(|e| format!("{e}")) },
                    move |_me, result, ctx| match result {
                        Ok(()) => {
                            ctx.emit(GlobalBufferModelEvent::FileSaved { file_id });
                        }
                        Err(error) => {
                            log::warn!("Remote save failed: {error}");
                            ctx.emit(GlobalBufferModelEvent::FailedToSave {
                                file_id,
                                error: Rc::new(FileSaveError::RemoteError(error)),
                            });
                        }
                    },
                );
                return Ok(());
            }
        }

        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.save(file_id, content, version, ctx)
        })
    }

    /// Rename a file and save its content via FileModel.
    #[cfg(feature = "local_fs")]
    pub fn rename_and_save(
        &self,
        file_id: FileId,
        new_path: PathBuf,
        content: String,
        version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.rename_and_save(file_id, new_path, content, version, ctx)
        })
    }

    /// Delete a file via FileModel.
    #[cfg(feature = "local_fs")]
    pub fn delete(
        &self,
        file_id: FileId,
        version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.delete(file_id, version, ctx)
        })
    }

    /// Remove a tracked buffer, cleaning up FileModel and LSP state.
    /// Used when a new file is deleted before ever being saved to a permanent location.
    pub fn remove(&mut self, file_id: FileId, ctx: &mut ModelContext<Self>) {
        self.cleanup_file_id(file_id, ctx);
    }

    /// Look up the file path for a tracked buffer.
    pub fn file_path(&self, file_id: FileId) -> Option<&Path> {
        match self.location_to_id.get_by_right(&file_id) {
            Some(LocalOrRemotePath::Local(path)) => Some(path.as_path()),
            _ => None,
        }
    }

    /// Get the base content version (last known on-disk version) for a tracked buffer.
    pub fn base_version(&self, file_id: FileId) -> Option<ContentVersion> {
        self.buffers
            .get(&file_id)
            .and_then(|state| state.base_content_version())
    }

    /// Discard any in progress changes and reload the buffer with the canonical version from the file system.
    #[cfg(feature = "local_fs")]
    pub fn discard_unsaved_changes(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        if let Some(id) = self
            .location_to_id
            .get_by_left(&LocalOrRemotePath::Local(path.to_path_buf()))
            .cloned()
        {
            let path_clone = path.to_path_buf();
            ctx.spawn(
                async move { FileModel::read_content_for_file(&path_clone).await },
                move |me, content, ctx| match content {
                    Ok(content) => {
                        // Consider this reload as a "new" version. This prevents any race condition when there is another
                        // auto-reload while we are reading out the latest content.
                        let new_version = ContentVersion::new();
                        // For discard, we get the current base version from the buffer state
                        let base_version = me
                            .buffers
                            .get(&id)
                            .and_then(|state| {
                                state.buffer.upgrade(ctx).map(|b| b.as_ref(ctx).version())
                            })
                            .unwrap_or(new_version);
                        FileModel::handle(ctx).update(ctx, |file_model, _ctx| {
                            file_model.set_version(id, new_version);
                        });
                        me.populate_buffer_with_read_content(
                            id,
                            &content,
                            base_version,
                            new_version,
                            false,
                            ctx,
                        );
                    }
                    Err(e) => ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                        file_id: id,
                        error: e.into(),
                    }),
                },
            );
        }
    }

    /// Remap an existing buffer from `old_file_id` to a new path, preserving the buffer
    /// content and unsaved edits. Sends didClose for the old path and re-registers the
    /// new path with FileModel and LSP.
    ///
    /// Used for file rename.
    #[cfg(feature = "local_fs")]
    pub fn rename(
        &mut self,
        old_file_id: FileId,
        new_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Option<BufferState> {
        let old_state = self.buffers.remove(&old_file_id)?;
        let buffer = old_state.buffer.upgrade(ctx)?;

        // Send didClose for the old path and remove the mapping.
        // Internal state cleanup is synchronous; only the LSP didClose notification
        // is dispatched asynchronously (with a no-op callback), so there is no race
        // between state removal and the close completing.
        if let Some((LocalOrRemotePath::Local(old_path), _)) =
            self.location_to_id.remove_by_right(&old_file_id)
        {
            self.close_document_with_lsp(&old_path, ctx);
        }

        // Cancel + unsubscribe old FileId from FileModel.
        let file_model = FileModel::handle(ctx);
        file_model.update(ctx, |file_model, ctx| {
            file_model.cancel(old_file_id);
            file_model.unsubscribe(old_file_id, ctx);
        });

        Some(self.register_buffer_for_path(
            new_path,
            buffer,
            old_state.base_content_version(),
            old_state.initial_content_version(),
            ctx,
        ))
    }

    /// Adopt an existing buffer under a new path without reading from disk.
    /// Used by `save_as` to register a newly-created file with GlobalBufferModel.
    #[cfg(feature = "local_fs")]
    pub fn register(
        &mut self,
        path: PathBuf,
        buffer: ModelHandle<Buffer>,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        let buffer_version = buffer.as_ref(ctx).version();
        self.register_buffer_for_path(
            path,
            buffer,
            Some(buffer_version),
            Some(buffer_version),
            ctx,
        )
    }

    /// Shared helper: register `buffer` under `path` with FileModel, subscribe to
    /// buffer events for LSP sync, store internal state, and open the document with LSP.
    #[cfg(feature = "local_fs")]
    fn register_buffer_for_path(
        &mut self,
        path: PathBuf,
        buffer: ModelHandle<Buffer>,
        base_content_version: Option<ContentVersion>,
        initial_content_version: Option<ContentVersion>,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        // If a buffer is already registered for this path, clean up the old entry
        // to avoid orphaning the previous FileId in `self.buffers`.
        if let Some(old_file_id) = self
            .location_to_id
            .get_by_left(&LocalOrRemotePath::Local(path.clone()))
            .copied()
        {
            self.cleanup_file_id(old_file_id, ctx);
        }

        let buffer_version = buffer.as_ref(ctx).version();
        let file_id = FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            let id = file_model.register_file_path(&path, true, ctx);
            file_model.set_version(id, buffer_version);
            id
        });

        self.location_to_id
            .insert(LocalOrRemotePath::Local(path.clone()), file_id);
        self.buffers.insert(
            file_id,
            InternalBufferState {
                buffer: buffer.downgrade(),
                latest_buffer_version: None,
                pending_diff_parse: None,
                source: BufferSource::Local {
                    base_content_version,
                    initial_content_version,
                },
            },
        );

        // Unsubscribe any existing buffer subscription (e.g. from a previous path)
        // before subscribing with the new path.
        ctx.unsubscribe_from_model(&buffer);

        // Subscribe to buffer events for LSP sync.
        let path_clone = path.clone();
        ctx.subscribe_to_model(&buffer, move |me, event, ctx| {
            use warp_editor::content::buffer::BufferEvent;

            let Some(state) = me.buffers.get(&file_id) else {
                return;
            };
            let Some(initial_version) = state.initial_content_version() else {
                return;
            };
            let Some(buffer) = state.buffer.upgrade(ctx) else {
                return;
            };

            if let BufferEvent::ContentChanged {
                delta,
                buffer_version,
                origin,
                ..
            } = event
            {
                let version_matches_initial = buffer.as_ref(ctx).version_match(&initial_version);
                let fid = me
                    .location_to_id
                    .get_by_left(&LocalOrRemotePath::Local(path_clone.clone()))
                    .cloned();
                let previous_version = fid
                    .and_then(|id| me.buffers.get(&id))
                    .and_then(|state| state.latest_buffer_version);

                if let Some(id) = fid {
                    if let Some(state) = me.buffers.get_mut(&id) {
                        state.latest_buffer_version = Some(buffer_version.as_usize());
                    }
                }

                if matches!(origin, EditOrigin::SystemEdit) && version_matches_initial {
                    me.open_or_sync_document_with_lsp(buffer, &path_clone, *buffer_version, ctx);
                    return;
                }

                me.notify_lsp_of_content_change(
                    buffer,
                    &delta.precise_deltas,
                    &path_clone,
                    *buffer_version,
                    previous_version,
                    ctx,
                );
            }
        });

        // Open the document with LSP.
        let buffer_ver = buffer.as_ref(ctx).buffer_version();
        self.open_or_sync_document_with_lsp(buffer.clone(), &path, buffer_ver, ctx);

        BufferState::new(file_id, buffer)
    }

    /// Open a buffer at the given location.
    ///
    /// Dispatches to the appropriate private opener based on the location variant.
    /// If a buffer already exists for this location and is loaded, returns the
    /// existing `BufferState`.
    pub fn open(
        &mut self,
        location: LocalOrRemotePath,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        match location {
            #[cfg(feature = "local_fs")]
            LocalOrRemotePath::Local(path) => self.open_local(path, false, ctx),
            #[cfg(not(feature = "local_fs"))]
            LocalOrRemotePath::Local(_) => {
                unimplemented!("Local buffers require the local_fs feature")
            }
            LocalOrRemotePath::Remote(remote_path) => self.open_remote_buffer(remote_path, ctx),
        }
    }

    /// Open a local buffer for the given file path.
    ///
    /// If a buffer already exists for this path and is loaded, returns the existing BufferState.
    /// If no buffer exists, creates a new Buffer and BufferState using FileModel.
    /// File system updates are automatically subscribed to for all buffers.
    ///
    /// When `is_server_local` is true, the buffer is created with a `ServerLocal`
    /// source (with a `SyncClock`) instead of a plain `Local` source.
    #[cfg(feature = "local_fs")]
    fn open_local(
        &mut self,
        path: PathBuf,
        is_server_local: bool,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        if let Some(id) = self
            .location_to_id
            .get_by_left(&LocalOrRemotePath::Local(path.clone()))
            .cloned()
        {
            debug_assert!(self.buffers.contains_key(&id));
            if let Some(state) = self.buffers.get(&id) {
                if let Some(handle) = state.buffer.upgrade(ctx) {
                    // Only emit buffer loaded if the base content version is set.
                    if state.is_loaded() {
                        ctx.emit(GlobalBufferModelEvent::BufferLoaded {
                            file_id: id,
                            content_version: handle.as_ref(ctx).version(),
                        });
                    }
                    return BufferState::new(id, handle.clone());
                }
            }
        }

        self.create_new_buffer(&path, is_server_local, ctx)
    }

    #[cfg(feature = "local_fs")]
    fn create_new_buffer(
        &mut self,
        path: &Path,
        is_server_local: bool,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        // Open file through FileModel to get FileId
        // Always subscribe to updates for GlobalBufferModel created buffers
        let file_id =
            FileModel::handle(ctx).update(ctx, |file_model, ctx| file_model.open(path, true, ctx));

        // Create new buffer
        let buffer = ctx.add_model(|_| {
            // This sets the default indentation behavior. The editor will override this if it can load the grammar config
            // for the given file path.
            Buffer::new(Box::new(|_, _| {
                IndentBehavior::TabIndent(IndentUnit::Space(4))
            }))
        });

        let path_clone = path.to_path_buf();
        ctx.subscribe_to_model(&buffer, move |me, event, ctx| {
            use warp_editor::content::buffer::BufferEvent;

            let Some(state) = me.buffers.get(&file_id) else {
                me.log_lsp_sync_debug(
                    &path_clone,
                    format!(
                        "lsp-sync: ContentChanged SKIPPED file={} reason=buffer_state_missing",
                        path_clone.display()
                    ),
                    ctx,
                );
                return;
            };

            let Some(initial_version) = state.initial_content_version() else {
                me.log_lsp_sync_debug(
                    &path_clone,
                    format!(
                        "lsp-sync: ContentChanged SKIPPED file={} reason=initial_version_not_set",
                        path_clone.display()
                    ),
                    ctx,
                );
                return;
            };

            let Some(buffer) = state.buffer.upgrade(ctx) else {
                me.log_lsp_sync_debug(
                    &path_clone,
                    format!(
                        "lsp-sync: ContentChanged SKIPPED file={} reason=buffer_handle_deallocated",
                        path_clone.display()
                    ),
                    ctx,
                );
                return;
            };

            if let BufferEvent::ContentChanged {
                delta,
                buffer_version,
                origin,
                ..
            } = event
            {
                let version_matches_initial = buffer.as_ref(ctx).version_match(&initial_version);

                // Read the previous latest_buffer_version before updating it.
                // This is needed to determine if we need a full sync later.
                let file_id = me
                    .location_to_id
                    .get_by_left(&LocalOrRemotePath::Local(path_clone.clone()))
                    .cloned();
                let previous_version = file_id
                    .and_then(|id| me.buffers.get(&id))
                    .and_then(|state| state.latest_buffer_version);

                // Always update the latest buffer version when we receive a ContentUpdated event,
                // even if we early return. This ensures we track versioning correctly.
                if let Some(id) = file_id {
                    if let Some(state) = me.buffers.get_mut(&id) {
                        state.latest_buffer_version = Some(buffer_version.as_usize());
                    }
                }

                // If this is a system edit AND the current buffer version matches the initial version
                // that came from file loading, this is the initial buffer population. Instead of
                // relying on the editor view to send didOpen, we handle it here to ensure the LSP
                // document lifecycle stays in sync with the buffer lifecycle.
                if matches!(origin, EditOrigin::SystemEdit) && version_matches_initial {
                    me.open_or_sync_document_with_lsp(buffer, &path_clone, *buffer_version, ctx);
                    return;
                }

                me.notify_lsp_of_content_change(
                    buffer,
                    &delta.precise_deltas,
                    &path_clone,
                    *buffer_version,
                    previous_version,
                    ctx,
                );
            }
        });

        self.location_to_id
            .insert(LocalOrRemotePath::Local(path.to_path_buf()), file_id);
        let source = if is_server_local {
            BufferSource::ServerLocal {
                sync_clock: SyncClock::new(),
                base_content_version: None,
                initial_content_version: None,
            }
        } else {
            BufferSource::Local {
                base_content_version: None,
                initial_content_version: None,
            }
        };
        self.buffers.insert(
            file_id,
            InternalBufferState {
                buffer: buffer.downgrade(),
                latest_buffer_version: None,
                pending_diff_parse: None,
                source,
            },
        );

        BufferState::new(file_id, buffer)
    }

    fn lsp_server_for_path(
        &self,
        path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> Option<ModelHandle<LspServerModel>> {
        LspManagerModel::as_ref(ctx).server_for_path(path, ctx)
    }

    fn log_lsp_sync_debug(&self, path: &Path, message: String, ctx: &mut ModelContext<Self>) {
        if cfg!(debug_assertions) {
            if let Some(server) = self.lsp_server_for_path(path, ctx) {
                server
                    .as_ref(ctx)
                    .log_to_server_log(LspServerLogLevel::Info, message);
            }
        }
    }

    /// Attempts to retrieve specific lines from an in-memory buffer for the given file path.
    /// Returns `Some(Vec<(usize, String)>)` if the file is loaded in a buffer, `None` otherwise.
    ///
    /// This is a fast, synchronous operation that avoids disk I/O.
    ///
    /// # Arguments
    /// * `path` - Path to the file
    /// * `line_numbers` - A list of 0-based line numbers to retrieve. Supports non-consecutive lines.
    ///
    /// # Returns
    /// A vector of (line_number, line_content) tuples for each requested line that exists.
    /// Lines that don't exist in the buffer are omitted from the result.
    pub fn get_lines_for_file(
        &mut self,
        path: &Path,
        line_numbers: Vec<usize>,
        ctx: &mut ModelContext<Self>,
    ) -> Option<Vec<(usize, String)>> {
        use warp_editor::content::text::LineCount;

        if line_numbers.is_empty() {
            return Some(Vec::new());
        }

        let file_id = self
            .location_to_id
            .get_by_left(&LocalOrRemotePath::Local(path.to_path_buf()))?;
        let buffer = self.buffer_handle_for_id(*file_id, ctx)?;

        let buffer_ref = buffer.as_ref(ctx);
        let total_lines = (buffer_ref.max_point().row + 1) as usize;

        let mut lines = Vec::with_capacity(line_numbers.len());
        for line_idx in line_numbers {
            if line_idx >= total_lines {
                continue;
            }
            // Convert 0-based line index to 1-based LineCount
            let line_count = LineCount::from(line_idx + 1);
            let line_start = buffer_ref.line_start(line_count);
            let line_end = buffer_ref.line_end(line_count);
            let line_text = buffer_ref.text_in_range(line_start..line_end).into_string();
            lines.push((line_idx, line_text));
        }

        Some(lines)
    }

    /// Opens or resyncs a document with the LSP server.
    /// - If the LSP doesn't have the document open yet: sends `didOpen`
    /// - If the LSP already has the document open (buffer recreation): sends a full-content `didChange`
    fn open_or_sync_document_with_lsp(
        &mut self,
        buffer: ModelHandle<Buffer>,
        path: &Path,
        buffer_version: BufferVersion,
        ctx: &mut ModelContext<Self>,
    ) {
        let path_buf = path.to_path_buf();
        let current_version = buffer_version.as_usize();

        let Some(lsp_server) = &self.lsp_server_for_path(path, ctx) else {
            return;
        };

        if !lsp_server.as_ref(ctx).is_ready_for_requests() {
            return;
        };

        let content = buffer.as_ref(ctx).text_with_line_ending().into_string();

        let lsp_already_has_document = lsp_server
            .as_ref(ctx)
            .last_synced_version(&path_buf)
            .ok()
            .flatten()
            .is_some();

        if lsp_already_has_document {
            // Buffer was recreated but LSP still has the document open.
            // Send a full-content didChange to resync.
            lsp_server.as_ref(ctx).log_to_server_log(
                LspServerLogLevel::Info,
                format!(
                    "didChange -> server: RESYNC full-content file={} send_version={current_version}",
                    path.display()
                ),
            );

            let content_changed_events = vec![TextDocumentContentChangeEvent {
                range: None,
                text: content,
            }];

            let Ok(sync_future) = lsp_server.as_ref(ctx).did_change_document(
                path_buf,
                current_version.into(),
                content_changed_events,
            ) else {
                log::warn!("Failed to resync document with LSP server");
                return;
            };

            ctx.spawn(sync_future, |_, _, _| {});
        } else {
            // First time opening this document with the LSP.
            self.log_lsp_sync_debug(
                path,
                format!(
                    "lsp-sync: didOpen from GlobalBufferModel file={} version={current_version}",
                    path.display()
                ),
                ctx,
            );

            let Ok(open_future) =
                lsp_server
                    .as_ref(ctx)
                    .did_open_document(path_buf, content, current_version)
            else {
                log::warn!("Failed to open document with LSP server");
                return;
            };

            ctx.spawn(open_future, |_, _, _| {});
        }
    }

    /// Sends `didClose` to the LSP server for the given path, if the document is open.
    fn close_document_with_lsp(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let Some(lsp_server) = self.lsp_server_for_path(path, ctx) else {
            return;
        };

        let path_buf = path.to_path_buf();
        if !lsp_server
            .as_ref(ctx)
            .document_is_open(&path_buf)
            .is_ok_and(|is_open| is_open)
        {
            return;
        };

        self.log_lsp_sync_debug(
            path,
            format!(
                "lsp-sync: didClose from GlobalBufferModel file={}",
                path.display()
            ),
            ctx,
        );

        let Ok(close_future) = lsp_server.as_ref(ctx).did_close_document(path_buf) else {
            log::warn!("Failed to close document with LSP server");
            return;
        };

        ctx.spawn(close_future, |_, _, _| {});
    }

    /// When an LSP server starts, open all loaded buffers that match its workspace path.
    #[cfg(feature = "local_fs")]
    fn handle_lsp_manager_events(
        &mut self,
        event: &LspManagerModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let LspManagerModelEvent::ServerStarted(workspace_path) = event else {
            return;
        };

        // Collect (path, buffer_handle, version) for all loaded buffers under this workspace.
        let buffers_to_open: Vec<_> = self
            .location_to_id
            .iter()
            .filter_map(|(location, id)| {
                let LocalOrRemotePath::Local(path) = location else {
                    return None;
                };
                if !path.starts_with(workspace_path) {
                    return None;
                }
                let state = self.buffers.get(id)?;
                if !state.is_loaded() {
                    return None;
                }
                let buffer = state.buffer.upgrade(ctx)?;
                let version = buffer.as_ref(ctx).buffer_version();
                Some((path.clone(), buffer, version))
            })
            .collect();

        for (path, buffer, version) in buffers_to_open {
            self.open_or_sync_document_with_lsp(buffer, &path, version, ctx);
        }
    }

    fn notify_lsp_of_content_change(
        &mut self,
        buffer: ModelHandle<Buffer>,
        deltas: &[PreciseDelta],
        path: &Path,
        buffer_version: BufferVersion,
        previous_version: Option<usize>,
        ctx: &mut ModelContext<Self>,
    ) {
        let path_buf = path.to_path_buf();
        let current_version = buffer_version.as_usize();

        let Some(lsp_server) = &self.lsp_server_for_path(path, ctx) else {
            return;
        };

        if !lsp_server.as_ref(ctx).is_ready_for_requests() {
            return;
        };

        // Check if the previous version was successfully synced.
        // If not, we need to fallback to syncing the full buffer content.
        let last_synced = lsp_server
            .as_ref(ctx)
            .last_synced_version(&path_buf)
            .ok()
            .flatten();

        // If we have a previous version that wasn't synced, we need to do a full sync.
        let needs_full_sync = previous_version.is_some_and(|prev| {
            last_synced.is_none() || last_synced.is_some_and(|synced| synced < prev)
        });

        let deltas_len = deltas.len();

        if needs_full_sync {
            lsp_server.as_ref(ctx).log_to_server_log(
                LspServerLogLevel::Info,
                format!(
                    "didChange -> server: FALLBACK full-sync file={} send_version={current_version} prev_version={previous_version:?} last_synced={last_synced:?} deltas={deltas_len}",
                    path.display()
                ),
            );
        } else {
            lsp_server.as_ref(ctx).log_to_server_log(
                LspServerLogLevel::Debug,
                format!(
                    "didChange -> server: file={} send_version={current_version} prev_version={previous_version:?} last_synced={last_synced:?} deltas={deltas_len}",
                    path.display()
                ),
            );
        }

        let line_ending_mode = buffer.as_ref(ctx).line_ending_mode();
        let content_changed_events = if needs_full_sync {
            // Send the full buffer content without a range.
            vec![TextDocumentContentChangeEvent {
                range: None,
                text: buffer.as_ref(ctx).text_with_line_ending().into_string(),
            }]
        } else {
            deltas
                .iter()
                .map(|delta| {
                    let start = lsp::types::Location {
                        line: delta.replaced_points.start.row.saturating_sub(1) as usize,
                        column: delta.replaced_points.start.column as usize,
                    };
                    let end = lsp::types::Location {
                        line: delta.replaced_points.end.row.saturating_sub(1) as usize,
                        column: delta.replaced_points.end.column as usize,
                    };

                    TextDocumentContentChangeEvent {
                        range: Some(lsp::types::Range { start, end }),
                        text: buffer
                            .as_ref(ctx)
                            .text_in_ranges(vec1![delta.resolved_range.clone()], line_ending_mode)
                            .into_string(),
                    }
                })
                .collect()
        };

        let Ok(sync_future) = lsp_server.as_ref(ctx).did_change_document(
            path_buf,
            current_version.into(),
            content_changed_events,
        ) else {
            log::warn!("Failed to sync document with LSP server");
            return;
        };

        ctx.spawn(sync_future, |_, _, _| {});
    }

    /// Look up a remote buffer's `FileId` by host and path string.
    ///
    /// Uses the `location_to_id` BiMap for O(1) lookup instead of scanning
    /// all buffer states.
    fn find_remote_file_id(&self, host_id: &HostId, path: &str) -> Option<FileId> {
        let std_path = StandardizedPath::try_new(path).ok()?;
        let location = LocalOrRemotePath::Remote(RemotePath::new(host_id.clone(), std_path));
        self.location_to_id.get_by_left(&location).copied()
    }

    // ── Remote buffer operations ──────────────────────────────────────

    /// Open a remote buffer identified by a `RemotePath`.
    ///
    /// Sends `OpenBuffer` to the remote server, creates a local `Buffer` model,
    /// and sets up bidirectional sync via `BufferEvent` → `BufferEdit`.
    ///
    /// Returns a `BufferState` immediately (buffer content is populated asynchronously).
    fn open_remote_buffer(
        &mut self,
        remote_path: RemotePath,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        let location = LocalOrRemotePath::Remote(remote_path.clone());

        // Return existing buffer if already open.
        if let Some(id) = self.location_to_id.get_by_left(&location).cloned() {
            if let Some(state) = self.buffers.get(&id) {
                if let Some(handle) = state.buffer.upgrade(ctx) {
                    if state.is_loaded() {
                        ctx.emit(GlobalBufferModelEvent::BufferLoaded {
                            file_id: id,
                            content_version: handle.as_ref(ctx).version(),
                        });
                    }
                    return BufferState::new(id, handle.clone());
                }
            }
        }

        let file_id = FileId::new();
        let buffer = ctx.add_model(|_| Buffer::default());

        // Extract fields before moving remote_path into the buffer source.
        let path_str = remote_path.path.as_str().to_string();
        let host_id = remote_path.host_id.clone();

        // Subscribe to buffer content changes so edits are sent back to the daemon.
        let client_for_sub = {
            let manager = RemoteServerManager::handle(ctx);
            manager.as_ref(ctx).client_for_host(&host_id).cloned()
        };
        log::debug!(
            "[remote-buffer] Setting up edit subscription: path={path_str} has_client={}",
            client_for_sub.is_some()
        );
        if let Some(client) = &client_for_sub {
            let client = client.clone();
            let path_for_edit = path_str.clone();
            ctx.subscribe_to_model(&buffer, move |me, event, ctx| {
                use warp_editor::content::buffer::BufferEvent;
                if let BufferEvent::ContentChanged { delta, origin, .. } = event {
                    // Skip server-originated changes to prevent echo loop.
                    // Server pushes applied via insert_at_char_offset_ranges
                    // emit ContentChanged with SystemEdit origin.
                    if !origin.from_user() {
                        return;
                    }

                    // Build incremental edits from the ContentChanged delta.
                    // Each PreciseDelta carries the replaced range (old buffer
                    // coordinates) and the resolved range (new buffer coordinates)
                    // from which we can read the replacement text.
                    let Some(state) = me.buffers.get(&file_id) else {
                        return;
                    };
                    let Some(buffer) = state.buffer.upgrade(ctx) else {
                        return;
                    };
                    let edits: Vec<remote_server::proto::TextEdit> = delta
                        .precise_deltas
                        .iter()
                        .map(|d| {
                            // Wire offsets are 1-indexed (matching CharOffset).
                            let text = buffer
                                .as_ref(ctx)
                                .text_in_range(d.resolved_range.clone())
                                .into_string();
                            remote_server::proto::TextEdit {
                                start_offset: d.replaced_range.start.as_usize() as u64,
                                end_offset: d.replaced_range.end.as_usize() as u64,
                                text,
                            }
                        })
                        .collect();

                    me.push_edit_to_pending_batch(file_id, edits, ctx);

                    // Schedule (or reschedule) the debounce timer.
                    // Uses the same Timer::after + abort_handle pattern as
                    // LanguageServerShutdownManager::schedule_next_scan.
                    let client_for_flush = client.clone();
                    let path_for_flush = path_for_edit.clone();
                    let handle = ctx.spawn(
                        async {
                            Timer::after(REMOTE_EDIT_DEBOUNCE).await;
                        },
                        move |me, _, _ctx| {
                            let Some(state) = me.buffers.get_mut(&file_id) else {
                                return;
                            };
                            let BufferSource::Remote { pending_batch, .. } = &mut state.source
                            else {
                                return;
                            };
                            if let Some(batch) = pending_batch.take() {
                                batch.flush(&client_for_flush, &path_for_flush);
                            }
                        },
                    );
                    // Re-borrow after ctx.spawn since the closure captured `me`.
                    if let Some(state) = me.buffers.get_mut(&file_id) {
                        if let BufferSource::Remote { pending_batch, .. } = &mut state.source {
                            if let Some(batch) = pending_batch.as_mut() {
                                batch.debounce_timer = Some(handle.abort_handle());
                            }
                        }
                    }
                }
            });
        }

        // Store state with sync_clock = None (set to Some on OpenBufferResponse).
        self.location_to_id.insert(location, file_id);
        self.buffers.insert(
            file_id,
            InternalBufferState {
                buffer: buffer.downgrade(),
                latest_buffer_version: None,
                pending_diff_parse: None,
                source: BufferSource::Remote {
                    remote_path,
                    sync_clock: None,
                    pending_batch: None,
                },
            },
        );

        // Look up the client on the main thread, then send OpenBuffer asynchronously.
        let Some(client) = client_for_sub else {
            safe_error!(
                safe: ("[remote-buffer] No remote server client at buffer open time"),
                full: ("[remote-buffer] No remote server client for host {host_id:?}")
            );
            ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                file_id,
                error: Rc::new(FileLoadError::DoesNotExist),
            });
            return BufferState::new(file_id, buffer);
        };

        log::debug!("[remote-buffer] Sending OpenBuffer for path={path_str} host={host_id:?}");
        ctx.spawn(
            async move {
                client
                    .open_buffer(path_str, false)
                    .await
                    .map_err(|e| format!("{e}"))
            },
            move |me, result, ctx| {
                me.apply_open_buffer_response(file_id, result, ctx);
            },
        );

        BufferState::new(file_id, buffer)
    }

    /// Shared handler for `OpenBuffer` RPC responses.
    ///
    /// On success, replaces the buffer content with the server's latest
    /// on-disk content, resets the `SyncClock`, and emits `BufferLoaded`.
    /// On failure, emits `FailedToLoad`.
    fn apply_open_buffer_response(
        &mut self,
        file_id: FileId,
        result: Result<remote_server::proto::OpenBufferResponse, String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let res = result.and_then(|res| {
            res.result.ok_or_else(|| {
                safe_error!(
                    safe: ("[remote-buffer] No result in OpenBuffer response"),
                    full: ("[remote-buffer] No result in OpenBuffer response for file_id={file_id:?}")
                );
                "No result in OpenBuffer response".to_string()
            })
        });
        match res {
            Ok(remote_server::proto::open_buffer_response::Result::Success(
                remote_server::proto::OpenBufferSuccess {
                    content,
                    server_version,
                },
            )) => {
                log::debug!(
                    "[remote-buffer] OpenBuffer response: content_len={} server_version={}",
                    content.len(),
                    server_version,
                );
                let Some(state) = self.buffers.get_mut(&file_id) else {
                    safe_error!(
                        safe: ("[remote-buffer] Buffer state missing after OpenBuffer response"),
                        full: ("[remote-buffer] Buffer state missing for file_id={file_id:?}")
                    );
                    return;
                };
                if let BufferSource::Remote {
                    sync_clock,
                    pending_batch,
                    ..
                } = &mut state.source
                {
                    *sync_clock = Some(SyncClock::from_wire(server_version, 0));
                    // Discard any pending batch — the server just sent us fresh
                    // content, so any in-flight edits are stale.
                    if let Some(batch) = pending_batch.take() {
                        batch.discard();
                    }
                }
                let Some(buffer) = state.buffer.upgrade(ctx) else {
                    safe_error!(
                        safe: ("[remote-buffer] Buffer handle deallocated before OpenBuffer response"),
                        full: ("[remote-buffer] Buffer handle deallocated for file_id={file_id:?}")
                    );
                    return;
                };
                let version = ContentVersion::new();
                buffer.update(ctx, |buffer, ctx| {
                    buffer.replace_all(&content, ctx);
                    buffer.set_version(version);
                });
                ctx.emit(GlobalBufferModelEvent::BufferLoaded {
                    file_id,
                    content_version: version,
                });
            }
            Ok(remote_server::proto::open_buffer_response::Result::Error(
                remote_server::proto::FileOperationError { message: error },
            ))
            | Err(error) => {
                log::warn!("[remote-buffer] Failed to open remote buffer: {error}");
                ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                    file_id,
                    error: Rc::new(FileLoadError::DoesNotExist),
                });
            }
        }
    }

    // ── Server-local buffer operations (daemon side) ────────────────

    /// Open a server-local buffer for the given file path on the daemon.
    ///
    /// Delegates to `open_local` with `is_server_local = true` so the buffer
    /// is created directly with a `ServerLocal` source and `SyncClock`.
    #[cfg(feature = "local_fs")]
    pub fn open_server_local(
        &mut self,
        path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        self.open_local(path, true, ctx)
    }

    /// Apply a client edit to a server-local buffer.
    ///
    /// If `expected_server_version` matches the buffer's current server version,
    /// the edits are applied to the in-memory buffer (no disk write) and the
    /// client version is updated. Returns `true` if accepted, `false` if rejected
    /// (stale edit — silently discarded).
    ///
    /// **Coordinate convention:** Each `TextEdit` in `edits` uses sequential
    /// coordinates — its offsets reference the buffer state *after* all
    /// preceding edits in the slice have been applied. This matches how the
    /// client constructs edits from `PreciseDelta.replaced_range`, which is
    /// resolved via anchors in intermediate buffer states. Edits are therefore
    /// applied one at a time rather than in a single batch call to
    /// `insert_at_char_offset_ranges` (which expects all offsets in the
    /// original-buffer coordinate space).
    #[cfg(feature = "local_fs")]
    pub fn apply_client_edit(
        &mut self,
        file_id: FileId,
        edits: &[super::super::remote_server::proto::TextEdit],
        expected_server_version: ContentVersion,
        new_client_version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(state) = self.buffers.get_mut(&file_id) else {
            return false;
        };

        let BufferSource::ServerLocal { sync_clock, .. } = &mut state.source else {
            return false;
        };

        if !sync_clock.client_edit_matches(expected_server_version) {
            log::debug!(
                "Rejected client edit: expected S={:?}, actual S={:?}",
                expected_server_version,
                sync_clock.server_version
            );
            return false;
        }

        sync_clock.client_version = new_client_version;

        let Some(buffer) = state.buffer.upgrade(ctx) else {
            return false;
        };

        // Apply each edit sequentially: offsets are in sequential coordinates
        // (each relative to the buffer after all preceding edits), so we must
        // apply one at a time and recompute max_offset for each.
        buffer.update(ctx, |buffer, ctx| {
            for edit in edits {
                let max_offset = buffer.max_charoffset();
                let start =
                    CharOffset::from((edit.start_offset as usize).min(max_offset.as_usize()));
                let end = CharOffset::from((edit.end_offset as usize).min(max_offset.as_usize()));
                buffer.insert_at_char_offset_ranges(
                    vec![(start..end, edit.text.clone())],
                    ContentVersion::new(),
                    ctx,
                );
            }
            // Allocate the final version after all per-edit versions so the
            // monotonic ContentVersion counter moves forward.
            buffer.set_version(ContentVersion::new());
        });
        true
    }

    /// Save a server-local buffer to disk.
    ///
    /// Uses the buffer's current `ContentVersion` (not a fresh one) so that
    /// `FileModel` can detect concurrent modifications between the save
    /// request and the disk write completing.
    #[cfg(feature = "local_fs")]
    pub fn save_server_local(
        &mut self,
        file_id: FileId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        let Some(state) = self.buffers.get(&file_id) else {
            return Err(FileSaveError::RemoteError("Buffer not found".to_string()));
        };
        let Some(buffer) = state.buffer.upgrade(ctx) else {
            return Err(FileSaveError::RemoteError("Buffer deallocated".to_string()));
        };
        let content = buffer.as_ref(ctx).text().into_string();
        let version = buffer.as_ref(ctx).version();
        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.save(file_id, content, version, ctx)
        })
    }

    /// Resolve a conflict by accepting the client's content.
    /// Replaces the buffer content, updates the sync clock, and saves to disk.
    #[cfg(feature = "local_fs")]
    pub fn resolve_conflict(
        &mut self,
        file_id: FileId,
        acknowledged_server_version: ContentVersion,
        current_client_version: ContentVersion,
        client_content: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), FileSaveError> {
        let Some(state) = self.buffers.get_mut(&file_id) else {
            return Err(FileSaveError::RemoteError("Buffer not found".to_string()));
        };

        if let BufferSource::ServerLocal { sync_clock, .. } = &mut state.source {
            sync_clock.server_version = acknowledged_server_version;
            sync_clock.client_version = current_client_version;
        }

        let Some(buffer) = state.buffer.upgrade(ctx) else {
            return Err(FileSaveError::RemoteError("Buffer deallocated".to_string()));
        };

        let new_version = ContentVersion::new();
        buffer.update(ctx, |buffer, ctx| {
            buffer.replace_all(client_content, ctx);
            buffer.set_version(new_version);
        });

        // Save to disk. Note: the buffer content has already been replaced
        // in memory above. If the save fails, memory and disk will diverge.
        // In the local conflict case (handle_file_model_events / FileUpdated),
        // the auto-reload is dropped and the user can retry the save manually.
        // Here we're on the daemon side, so a failed save means the buffer
        // stays diverged until the next file-watcher cycle reconciles it.
        // The synchronous Result is propagated to the caller; async write
        // failures surface via the FailedToSave event.
        let content = client_content.to_string();
        let save_version = ContentVersion::new();
        state.set_base_content_version(save_version);
        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.save(file_id, content, save_version, ctx)
        })
    }

    // ── Public accessors ──────────────────────────────────────────────

    /// Returns the buffer text content for a given `FileId`.
    pub fn content_for_file(&self, file_id: FileId, ctx: &warpui::AppContext) -> Option<String> {
        let state = self.buffers.get(&file_id)?;
        let buffer = state.buffer.upgrade(ctx)?;
        Some(buffer.as_ref(ctx).text().into_string())
    }

    /// Returns a reference to the `SyncClock` for a server-local buffer.
    pub fn sync_clock_for_server_local(&self, file_id: FileId) -> Option<&SyncClock> {
        let state = self.buffers.get(&file_id)?;
        match &state.source {
            BufferSource::ServerLocal { sync_clock, .. } => Some(sync_clock),
            BufferSource::Local { .. } | BufferSource::Remote { .. } => None,
        }
    }

    /// Returns whether a buffer is a `ServerLocal` source.
    #[cfg(test)]
    pub fn is_server_local(&self, file_id: FileId) -> bool {
        self.buffers
            .get(&file_id)
            .is_some_and(|state| matches!(state.source, BufferSource::ServerLocal { .. }))
    }

    /// Force-reload a server-local buffer from disk, discarding any in-memory
    /// edits.
    ///
    /// Reads the file, replaces the buffer content, bumps the server version
    /// in the `SyncClock`, and emits both `BufferLoaded` (so the requesting
    /// connection gets the new content) and `ServerLocalBufferUpdated` (so
    /// other connections receive a `BufferUpdatedPush` with the fresh content).
    #[cfg(feature = "local_fs")]
    pub fn force_reload_server_local(
        &mut self,
        file_id: FileId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        let Some(state) = self.buffers.get(&file_id) else {
            return Err("force_reload: no local path for file_id={file_id:?}".to_string());
        };
        let Some(file_path) =
            self.location_to_id
                .get_by_right(&file_id)
                .and_then(|loc| match loc {
                    LocalOrRemotePath::Local(p) => Some(p.clone()),
                    LocalOrRemotePath::Remote(_) => None,
                })
        else {
            return Err("force_reload: no local path for file_id={file_id:?}".to_string());
        };
        // Capture the current client version before the reload so we can
        // include it in the ServerLocalBufferUpdated event.
        let expected_client_version = match &state.source {
            BufferSource::ServerLocal { sync_clock, .. } => sync_clock.client_version,
            _ => {
                return Err("force_reload called on non-ServerLocal buffer {file_id:?}".to_string());
            }
        };

        ctx.spawn(
            async move { FileModel::read_content_for_file(&file_path).await },
            move |me, content, ctx| match content {
                Ok(content) => {
                    let Some(state) = me.buffers.get_mut(&file_id) else {
                        ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                            file_id,
                            error: Rc::new(FileLoadError::DoesNotExist),
                        });
                        return;
                    };
                    let Some(buffer) = state.buffer.upgrade(ctx) else {
                        ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                            file_id,
                            error: Rc::new(FileLoadError::DoesNotExist),
                        });
                        return;
                    };

                    // Capture the end of the old buffer for the replacement range
                    // BEFORE replacing content.
                    let old_end = buffer.as_ref(ctx).max_charoffset();

                    let new_version = ContentVersion::new();
                    buffer.update(ctx, |buffer, ctx| {
                        buffer.replace_all(&content, ctx);
                        buffer.set_version(new_version);
                    });

                    state.set_base_content_version(new_version);
                    FileModel::handle(ctx).update(ctx, |file_model, _ctx| {
                        file_model.set_version(file_id, new_version);
                    });

                    // Bump the server version in the sync clock.
                    let new_server_version =
                        if let BufferSource::ServerLocal { sync_clock, .. } = &mut state.source {
                            let sv = sync_clock.bump_server();
                            // Reset client version to 0 ("no client edits").
                            // server_version tracks disk state; client_version
                            // tracks user edits. After a force-reload both sides
                            // agree on CV=0 (the client also resets via
                            // apply_open_buffer_response → SyncClock::from_wire).
                            sync_clock.client_version = ContentVersion::from_raw(0);
                            sv
                        } else {
                            return;
                        };

                    // Build a single full-replacement edit so other connections
                    // can apply it via BufferUpdatedPush.
                    let char_offset_edits = vec![CharOffsetEdit {
                        start: CharOffset::from(1usize),
                        end: old_end,
                        text: content,
                    }];

                    // Emit ServerLocalBufferUpdated BEFORE BufferLoaded so that
                    // the ServerModel's handler can peek at pending OpenBuffer
                    // requests to exclude the requesting connection from the
                    // broadcast. BufferLoaded consumes those pending requests.
                    ctx.emit(GlobalBufferModelEvent::ServerLocalBufferUpdated {
                        file_id,
                        edits: char_offset_edits,
                        new_server_version,
                        expected_client_version,
                    });
                    ctx.emit(GlobalBufferModelEvent::BufferLoaded {
                        file_id,
                        content_version: new_version,
                    });
                }
                Err(e) => {
                    log::warn!("[server-local] force_reload failed: {e}");
                    ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                        file_id,
                        error: e.into(),
                    });
                }
            },
        );

        Ok(())
    }

    /// Re-open an existing remote buffer by sending `OpenBuffer` with
    /// `force_reload = true` to the server.
    ///
    /// The server re-reads the file from disk into the existing buffer and
    /// broadcasts a `BufferUpdatedPush` to all other connections. The
    /// requesting connection receives the fresh content via
    /// `OpenBufferResponse`, which is applied by `apply_open_buffer_response`.
    ///
    /// On failure, emits `FailedToLoad` (the caller should keep the current
    /// buffer state so the user can retry).
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    pub fn reopen_remote_buffer(&mut self, file_id: FileId, ctx: &mut ModelContext<Self>) {
        let Some(state) = self.buffers.get(&file_id) else {
            return;
        };
        let BufferSource::Remote { remote_path, .. } = &state.source else {
            return;
        };

        let path_str = remote_path.path.as_str().to_string();
        let host_id = remote_path.host_id.clone();

        let manager = RemoteServerManager::handle(ctx);
        let Some(client) = manager.as_ref(ctx).client_for_host(&host_id).cloned() else {
            log::warn!("[remote-buffer] reopen: no client for host {host_id:?}");
            ctx.emit(GlobalBufferModelEvent::FailedToLoad {
                file_id,
                error: Rc::new(FileLoadError::DoesNotExist),
            });
            return;
        };

        log::debug!("[remote-buffer] Re-opening buffer with force_reload: path={path_str}");
        ctx.spawn(
            async move {
                client
                    .open_buffer(path_str, true)
                    .await
                    .map_err(|e| format!("{e}"))
            },
            move |me, result, ctx| {
                me.apply_open_buffer_response(file_id, result, ctx);
            },
        );
    }

    /// Handle an incoming `BufferConflictDetected` push from the remote server.
    ///
    /// The server detected that the file changed on disk while the client
    /// had unsaved edits. Emits `RemoteBufferConflict` so the UI shows
    /// the conflict resolution banner. Discards any pending edit batch
    /// since conflict resolution will re-sync content.
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    pub(crate) fn handle_buffer_conflict_detected(
        &mut self,
        host_id: &HostId,
        path: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        log::debug!("[remote-buffer] BufferConflictDetected: host={host_id} path={path}");

        let Some(file_id) = self.find_remote_file_id(host_id, path) else {
            safe_error!(
                safe: ("[remote-buffer] BufferConflictDetected for unknown buffer"),
                full: ("[remote-buffer] BufferConflictDetected for unknown buffer: {path}")
            );
            return;
        };

        // Discard any pending batch — conflict resolution handles re-sync.
        if let Some(state) = self.buffers.get_mut(&file_id) {
            if let BufferSource::Remote { pending_batch, .. } = &mut state.source {
                if let Some(batch) = pending_batch.take() {
                    batch.discard();
                }
            }
        }

        ctx.emit(GlobalBufferModelEvent::RemoteBufferConflict { file_id });
    }

    /// Handle an incoming `BufferUpdatedPush` from the remote server.
    ///
    /// Accepts incremental edits (1-indexed char offsets matching `CharOffset`)
    /// and applies them to the local buffer via `insert_at_char_offset_ranges`.
    /// If the expected client version doesn't match, a conflict event is emitted.
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    pub fn handle_buffer_updated_push(
        &mut self,
        host_id: &HostId,
        path: &str,
        new_server_version: u64,
        expected_client_version: u64,
        edits: &[CharOffsetEdit],
        ctx: &mut ModelContext<Self>,
    ) {
        log::debug!(
            "[remote-buffer] BufferUpdatedPush: path={path} new_sv={new_server_version} \
             expected_cv={expected_client_version} edit_count={}",
            edits.len()
        );

        let Some(file_id) = self.find_remote_file_id(host_id, path) else {
            safe_error!(
                safe: ("[remote-buffer] BufferUpdatedPush for unknown remote buffer"),
                full: ("[remote-buffer] BufferUpdatedPush for unknown remote buffer: {path}")
            );
            return;
        };

        let Some(state) = self.buffers.get_mut(&file_id) else {
            return;
        };

        let BufferSource::Remote {
            sync_clock,
            pending_batch,
            ..
        } = &mut state.source
        else {
            return;
        };
        let Some(sync_clock) = sync_clock.as_mut() else {
            return;
        };

        log::debug!(
            "[remote-buffer] SyncClock state: local_sv={:?} local_cv={:?}",
            sync_clock.server_version,
            sync_clock.client_version,
        );

        let expected_cv = ContentVersion::from_raw(expected_client_version as usize);
        if sync_clock.server_push_matches(expected_cv) {
            // Accept the update — apply edits incrementally.
            log::debug!(
                "[remote-buffer] Accepting push: applying {} edits",
                edits.len()
            );
            sync_clock.server_version = ContentVersion::from_raw(new_server_version as usize);

            let Some(buffer) = state.buffer.upgrade(ctx) else {
                return;
            };

            let new_version = ContentVersion::new();
            buffer.update(ctx, |buffer, ctx| {
                let max_offset = buffer.max_charoffset();
                let char_edits: Vec<(std::ops::Range<CharOffset>, String)> = edits
                    .iter()
                    .map(|edit| {
                        let start = std::cmp::min(edit.start, max_offset);
                        let end = std::cmp::min(edit.end, max_offset);
                        (start..end, edit.text.clone())
                    })
                    .collect();
                buffer.insert_at_char_offset_ranges(char_edits, new_version, ctx);
            });

            // Notify LocalCodeEditor so it updates base_content_version.
            // Without this, has_unsaved_changes() would compare the stale
            // initial-load version against the now-different buffer version
            // and incorrectly report unsaved changes.
            ctx.emit(GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                file_id,
                success: true,
                content_version: new_version,
            });
        } else {
            // Check if the push is stale — its server version is already
            // consumed (e.g. via an OpenBufferResponse from a force-reload).
            if new_server_version <= sync_clock.server_version.as_u64() {
                log::info!(
                    "[remote-buffer] Dropping stale BufferUpdatedPush for {path}: \
                     push_sv={new_server_version} <= local_sv={:?}",
                    sync_clock.server_version
                );
                return;
            }
            // Conflict — local edits diverged from server. Discard any
            // pending edit batch since conflict resolution will re-sync.
            if let Some(batch) = pending_batch.take() {
                batch.discard();
            }
            log::info!(
                "[remote-buffer] CONFLICT for {path}: push expected C={expected_client_version}, \
                 but local C={:?}. Emitting RemoteBufferConflict.",
                sync_clock.client_version
            );
            ctx.emit(GlobalBufferModelEvent::RemoteBufferConflict { file_id });
        }
    }
}

impl GlobalBufferModel {
    /// Accumulate edits into the pending batch for a remote buffer.
    ///
    /// Bumps `sync_clock.client_version` immediately so conflict detection
    /// sees the true current C even before the batch is flushed. If no batch
    /// exists yet, creates one capturing the current `server_version` as
    /// `expected_server_version`. Cancels any existing debounce timer —
    /// the caller is responsible for scheduling a new one.
    fn push_edit_to_pending_batch(
        &mut self,
        file_id: FileId,
        edits: Vec<remote_server::proto::TextEdit>,
        _ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.buffers.get_mut(&file_id) else {
            return;
        };
        let BufferSource::Remote {
            sync_clock,
            pending_batch,
            ..
        } = &mut state.source
        else {
            return;
        };
        let Some(sync_clock) = sync_clock.as_mut() else {
            return;
        };

        let new_cv = ContentVersion::new();
        sync_clock.client_version = new_cv;

        let batch = pending_batch.get_or_insert_with(|| PendingEditBatch {
            expected_server_version: sync_clock.server_version.as_u64(),
            edits: Vec::new(),
            latest_client_version: new_cv,
            debounce_timer: None,
        });
        batch.edits.extend(edits);
        batch.latest_client_version = new_cv;

        // Cancel existing debounce timer — caller will schedule a new one.
        if let Some(timer) = batch.debounce_timer.take() {
            timer.abort();
        }
    }
}

impl Entity for GlobalBufferModel {
    type Event = GlobalBufferModelEvent;
}

impl SingletonEntity for GlobalBufferModel {}

#[cfg(test)]
impl GlobalBufferModel {
    /// Test-only: seeds a Remote buffer with the given content and sync clock,
    /// bypassing `open_remote` (which requires `RemoteServerManager`).
    /// `pub(crate)` because it's used by both `buffer_location_tests` and
    /// `global_buffer_model_tests`.
    pub(crate) fn seed_remote_buffer_for_test(
        &mut self,
        host_id: HostId,
        path: warp_util::standardized_path::StandardizedPath,
        content: &str,
        server_version: u64,
        ctx: &mut ModelContext<Self>,
    ) -> BufferState {
        let remote_path = RemotePath::new(host_id, path);
        let location = LocalOrRemotePath::Remote(remote_path.clone());
        let file_id = warp_util::file::FileId::new();
        let buffer = ctx.add_model(|_| Buffer::default());
        let version = ContentVersion::new();
        buffer.update(ctx, |buf, ctx| {
            buf.replace_all(content, ctx);
            buf.set_version(version);
        });
        self.location_to_id.insert(location, file_id);
        self.buffers.insert(
            file_id,
            InternalBufferState {
                buffer: buffer.downgrade(),
                latest_buffer_version: None,
                pending_diff_parse: None,
                source: BufferSource::Remote {
                    remote_path,
                    sync_clock: Some(SyncClock::from_wire(server_version, 0)),
                    pending_batch: None,
                },
            },
        );
        BufferState::new(file_id, buffer)
    }

    /// Test-only: returns the `SyncClock` for a Remote buffer.
    /// `pub(crate)` because it's used by both `buffer_location_tests` and
    /// `global_buffer_model_tests`.
    pub(crate) fn sync_clock_for_remote_test(&self, file_id: FileId) -> Option<&SyncClock> {
        let state = self.buffers.get(&file_id)?;
        match &state.source {
            BufferSource::Remote { sync_clock, .. } => sync_clock.as_ref(),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "global_buffer_model_tests.rs"]
mod global_buffer_model_tests;

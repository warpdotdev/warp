#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

use ai::diff_validation::DiffType;
#[cfg(not(target_family = "wasm"))]
use futures::FutureExt;
#[cfg(not(target_family = "wasm"))]
use futures::{channel::oneshot, future::Shared};
#[cfg(not(target_family = "wasm"))]
use warp_files::FileModel;
use warp_util::file::FileId;
#[cfg(not(target_family = "wasm"))]
use warp_util::file::FileSaveError;
use warp_util::standardized_path::StandardizedPath;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::elements::ChildView;
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

use super::diff_viewer::{DiffViewer, DisplayMode};
use super::editor::NavBarBehavior;
use super::editor::scroll::{ScrollPosition, ScrollTrigger};
use super::editor::view::{CodeEditorEvent, CodeEditorView};
use crate::ai::blocklist::diff_storage::SaveFuture;
#[cfg(not(target_family = "wasm"))]
use crate::ai::blocklist::diff_types::DiffSessionType;
use crate::editor::InteractionState;

pub enum InlineDiffViewEvent {
    DiffStatusUpdated,
    #[cfg(not(target_family = "wasm"))]
    FileLoaded,
    #[cfg(not(target_family = "wasm"))]
    FileSaved,
    #[cfg(not(target_family = "wasm"))]
    FailedToSave {
        error: Arc<FileSaveError>,
    },
    UserEdited,
}

#[cfg(not(target_family = "wasm"))]
enum PersistenceState {
    Unpersisted,
    Persisting(Shared<SaveFuture>),
    Persisted,
    Restoring(Shared<SaveFuture>),
    Restored,
}

/// An inline diff viewer with optional file-backed save support.
///
/// When a backing file is registered (via [`Self::register_file`]), this view supports the full
/// accept/save/revert lifecycle through `FileModel`. Without a registered file, it behaves
/// as a read-only diff viewer (e.g. for WASM or restored conversations).
pub struct InlineDiffView {
    editor: ViewHandle<CodeEditorView>,
    diff_type: Option<DiffType>,
    file_path: Option<StandardizedPath>,
    /// Whether the user has edited the diff content.
    was_edited: bool,
    /// `FileModel` file ID for the backing file. Set via [`Self::register_file`].
    ///
    /// When `Some`:
    /// - The editor is editable (interaction state follows the `DisplayMode` rules).
    /// - Accept, save, and revert operations write through `FileModel`.
    ///
    /// When `None` (WASM, restored conversations, or before registration):
    /// - The editor is selection-only (never editable).
    /// - Accept, save, and revert are no-ops.
    backing_file_id: Option<FileId>,
    #[cfg(not(target_family = "wasm"))]
    session_type: Option<DiffSessionType>,
    #[cfg(not(target_family = "wasm"))]
    persistence: PersistenceState,
}

impl InlineDiffView {
    pub fn new(
        editor: ViewHandle<CodeEditorView>,
        diff_type: Option<DiffType>,
        display_mode: Option<DisplayMode>,
        file_path: Option<StandardizedPath>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_view(&editor, |me, _view, event, ctx| match event {
            CodeEditorEvent::DiffUpdated => {
                ctx.emit(InlineDiffViewEvent::DiffStatusUpdated);
            }
            CodeEditorEvent::ContentChanged { origin } => {
                if origin.from_user() && !me.was_edited {
                    me.was_edited = true;
                    ctx.emit(InlineDiffViewEvent::UserEdited);
                }
            }
            _ => {}
        });

        let model = Self {
            editor,
            diff_type,
            file_path,
            was_edited: false,
            backing_file_id: None,
            #[cfg(not(target_family = "wasm"))]
            session_type: None,
            #[cfg(not(target_family = "wasm"))]
            persistence: PersistenceState::Unpersisted,
        };

        model.apply_diffs_if_any(ctx);
        if let Some(display_mode) = display_mode {
            model.set_display_mode(display_mode, ctx);
        }

        model
    }

    /// Register a file with `FileModel` for save support.
    ///
    /// The `session_type` determines whether the file is local or remote.
    /// For `Local`, the file is registered by path on the local filesystem.
    /// For `Remote`, the file is registered against the remote backend so
    /// that `save()` / `delete()` dispatch over the wire via
    /// `RemoteServerClient`.
    ///
    /// This must be called after construction for non-WASM environments.
    #[cfg(not(target_family = "wasm"))]
    pub fn register_file(&mut self, session_type: &DiffSessionType, ctx: &mut ViewContext<Self>) {
        let Some(file_path) = &self.file_path else {
            return;
        };

        let file_model = FileModel::handle(ctx);
        let file_id = match session_type {
            DiffSessionType::Local => {
                let Some(local_path) = file_path.to_local_path() else {
                    crate::safe_error!(
                        safe: (
                            "Failed to convert StandardizedPath to local path; diff will be \
                            read-only"
                        ),
                        full: (
                            "Failed to convert StandardizedPath to local path: {file_path}; diff \
                            will be read-only"
                        )
                    );
                    return;
                };
                file_model.update(ctx, |file_model, ctx| {
                    file_model.register_file_path(&local_path, false, ctx)
                })
            }
            DiffSessionType::Remote(host_id) => {
                let host_id = host_id.clone();
                let remote_path = file_path.clone();
                file_model.update(ctx, |file_model, _ctx| {
                    file_model.register_remote_file(host_id, remote_path)
                })
            }
        };

        self.session_type = Some(session_type.clone());
        self.finish_file_registration(file_id, ctx);
    }

    /// Sets the backing file ID and initial version after registration.
    #[cfg(not(target_family = "wasm"))]
    fn finish_file_registration(&mut self, file_id: FileId, ctx: &mut ViewContext<Self>) {
        let file_model = FileModel::handle(ctx);

        let version = self.editor.as_ref(ctx).version(ctx);
        file_model.update(ctx, |file_model, _ctx| {
            file_model.set_version(file_id, version);
        });

        self.backing_file_id = Some(file_id);

        ctx.emit(InlineDiffViewEvent::FileLoaded);
    }

    fn apply_diffs_if_any(&self, ctx: &mut ViewContext<Self>) {
        let Some(diff) = self.diff_type.clone() else {
            return;
        };

        let deltas = match diff {
            DiffType::Create { delta } => vec![delta],
            DiffType::Update { mut deltas, .. } => {
                deltas.sort_by_key(|delta| delta.replacement_line_range.start);
                deltas
            }
            DiffType::Delete { delta } => vec![delta],
        };

        if deltas.is_empty() {
            return;
        }

        self.editor.update(ctx, |editor, ctx| {
            editor.apply_diffs(deltas, ctx);
            editor.toggle_diff_nav(None, ctx);
            editor.set_pending_scroll(ScrollTrigger::new(
                ScrollPosition::FocusedDiffHunk,
                editor.buffer_version(ctx),
            ));
        });
    }

    /// Persists the diff operation with the current editor content. `None` when no file is registered.
    #[cfg(not(target_family = "wasm"))]
    fn save_content(&mut self, ctx: &mut ViewContext<Self>) -> Option<SaveFuture> {
        let file_id = self.backing_file_id?;
        match &self.persistence {
            PersistenceState::Persisting(future) => return Some(future.clone().boxed()),
            PersistenceState::Persisted => return Some(futures::future::ready(Ok(())).boxed()),
            PersistenceState::Restoring(_) => {
                return Some(
                    futures::future::ready(Err(Arc::new(FileSaveError::Other(
                        "Cannot accept an edit while restoring it".to_owned(),
                    ))))
                    .boxed(),
                );
            }
            PersistenceState::Unpersisted | PersistenceState::Restored => {}
        }
        let content = self.editor.as_ref(ctx).text(ctx).into_string();
        let version = self.editor.as_ref(ctx).version(ctx);

        match FileModel::handle(ctx).update(ctx, |file_model, ctx| match &self.diff_type {
            Some(DiffType::Delete { .. }) => file_model.delete(file_id, version, ctx),
            Some(DiffType::Update {
                rename: Some(to), ..
            }) if StandardizedPath::try_new(&to.to_string_lossy())
                .ok()
                .as_ref()
                != self.file_path.as_ref() =>
            {
                file_model.rename_and_save(file_id, to.clone(), content, version, ctx)
            }
            Some(DiffType::Update {
                rename: Some(_), ..
            }) => file_model.save(file_id, content, version, ctx),
            Some(DiffType::Create { .. } | DiffType::Update { rename: None, .. }) | None => {
                file_model.save(file_id, content, version, ctx)
            }
        }) {
            Ok(save_future) => Some(self.track_operation(save_future, false, ctx)),
            Err(err) => {
                let error = Arc::new(err);
                ctx.emit(InlineDiffViewEvent::FailedToSave {
                    error: error.clone(),
                });
                Some(futures::future::ready(Err(error)).boxed())
            }
        }
    }

    /// Accepts the diff by persisting its file operation,
    /// returning the save's completion future. `None` when no file is
    /// registered (WASM / restored conversations), in which case nothing is
    /// dispatched.
    pub fn accept_and_save_diff(&mut self, ctx: &mut ViewContext<Self>) -> Option<SaveFuture> {
        #[cfg(not(target_family = "wasm"))]
        {
            self.save_content(ctx)
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = ctx;
            None
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn track_operation(
        &mut self,
        operation: SaveFuture,
        restoring: bool,
        ctx: &mut ViewContext<Self>,
    ) -> SaveFuture {
        let (tx, rx) = oneshot::channel();
        let future = async move {
            rx.await.unwrap_or_else(|_| {
                Err(Arc::new(FileSaveError::Other(
                    "File operation completion was cancelled".to_owned(),
                )))
            })
        }
        .boxed()
        .shared();
        self.persistence = if restoring {
            PersistenceState::Restoring(future.clone())
        } else {
            PersistenceState::Persisting(future.clone())
        };
        ctx.spawn(operation, move |me, result, ctx| {
            me.persistence = match (&result, restoring) {
                (Ok(()), true) => PersistenceState::Restored,
                (Ok(()), false) | (Err(_), true) => PersistenceState::Persisted,
                (Err(_), false) => PersistenceState::Unpersisted,
            };
            match &result {
                Ok(()) => ctx.emit(InlineDiffViewEvent::FileSaved),
                Err(error) => ctx.emit(InlineDiffViewEvent::FailedToSave {
                    error: error.clone(),
                }),
            }
            let _ = tx.send(result);
        });
        future.boxed()
    }

    /// Restores an accepted operation, sharing in-flight completion and leaving failures retryable.
    pub fn restore_and_save_diff(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> Result<SaveFuture, String> {
        #[cfg(target_family = "wasm")]
        {
            let _ = ctx;
            Ok(Box::pin(futures::future::ready(Ok(()))))
        }
        #[cfg(not(target_family = "wasm"))]
        {
            match &self.persistence {
                PersistenceState::Restoring(future) => return Ok(future.clone().boxed()),
                PersistenceState::Unpersisted | PersistenceState::Restored => {
                    return Ok(futures::future::ready(Ok(())).boxed());
                }
                PersistenceState::Persisting(_) => {
                    return Err("File operation is still in flight".to_owned());
                }
                PersistenceState::Persisted => {}
            }
            let file_id = self
                .backing_file_id
                .ok_or_else(|| "Missing backing file".to_owned())?;
            let version = self.editor.as_ref(ctx).version(ctx);
            let file_model = FileModel::handle(ctx);
            let operation = match &self.diff_type {
                Some(DiffType::Create { .. }) => {
                    file_model.update(ctx, |model, ctx| model.delete(file_id, version, ctx))
                }
                diff => {
                    let base_content = self
                        .base_content(ctx)
                        .ok_or_else(|| "Missing base content".to_owned())?;
                    match diff {
                        Some(DiffType::Delete { .. })
                            if matches!(self.session_type, Some(DiffSessionType::Local)) =>
                        {
                            // Local IDs cache canonical paths, so reuse could write a deleted
                            // symlink's referent. Remote IDs already preserve the requested path.
                            let original_path =
                                self.file_path.as_ref().expect("registered file has a path");
                            file_model.update(ctx, |model, ctx| {
                                let restored_id = model.register_file_path(
                                    &PathBuf::from(original_path.as_str()),
                                    false,
                                    ctx,
                                );
                                let result = model.save(restored_id, base_content, version, ctx);
                                model.unsubscribe(restored_id, ctx);
                                result
                            })
                        }
                        Some(DiffType::Update {
                            rename: Some(to), ..
                        }) if StandardizedPath::try_new(&to.to_string_lossy())
                            .ok()
                            .as_ref()
                            != self.file_path.as_ref() =>
                        {
                            let target_id = file_model.update(ctx, |model, ctx| {
                                match self
                                    .session_type
                                    .as_ref()
                                    .expect("registered file has a session")
                                {
                                    DiffSessionType::Local => {
                                        Ok::<_, String>(model.register_file_path(to, false, ctx))
                                    }
                                    DiffSessionType::Remote(host_id) => {
                                        let path = StandardizedPath::try_new(&to.to_string_lossy())
                                            .map_err(|error| error.to_string())?;
                                        Ok(model.register_remote_file(host_id.clone(), path))
                                    }
                                }
                            })?;
                            let original_path =
                                self.file_path.as_ref().expect("registered file has a path");
                            file_model.update(ctx, |model, ctx| {
                                let result = model.rename_and_save(
                                    target_id,
                                    PathBuf::from(original_path.as_str()),
                                    base_content,
                                    version,
                                    ctx,
                                );
                                model.unsubscribe(target_id, ctx);
                                result
                            })
                        }
                        Some(DiffType::Delete { .. } | DiffType::Update { .. }) | None => {
                            file_model.update(ctx, |model, ctx| {
                                model.save(file_id, base_content, version, ctx)
                            })
                        }
                        Some(DiffType::Create { .. }) => unreachable!(),
                    }
                }
            }
            .map_err(|error| error.to_string())?;
            Ok(self.track_operation(operation, true, ctx))
        }
    }
}

impl InlineDiffView {
    pub fn file_path(&self) -> Option<&StandardizedPath> {
        self.file_path.as_ref()
    }

    /// Base content of the diff (from the editor's diff model), if set.
    pub fn base_content(&self, app: &AppContext) -> Option<String> {
        self.editor
            .as_ref(app)
            .model
            .as_ref(app)
            .diff()
            .as_ref(app)
            .base()
            .map(|base| base.to_string())
    }

    pub fn file_name(&self) -> Option<String> {
        self.file_path()
            .map(|p| p.file_name().unwrap_or_default().to_owned())
    }
}

impl DiffViewer for InlineDiffView {
    fn editor(&self) -> &ViewHandle<CodeEditorView> {
        &self.editor
    }

    fn diff(&self) -> Option<&DiffType> {
        self.diff_type.as_ref()
    }

    fn was_edited(&self) -> bool {
        self.was_edited
    }

    fn set_display_mode(&self, mode: DisplayMode, ctx: &mut ViewContext<Self>) {
        let is_delete = matches!(self.diff(), Some(DiffType::Delete { .. }));
        let interaction_state = if self.backing_file_id.is_some() {
            mode.interaction_state(is_delete)
        } else {
            // No file registered (e.g. WASM or restored conversations): always read-only.
            InteractionState::Selectable
        };
        self.editor().update(ctx, |editor, ctx| {
            editor.set_scroll_wheel_behavior(mode.scroll_wheel_behavior());
            editor.set_vertical_expansion_behavior(mode.vertical_expansion_behavior(), ctx);
            editor.set_vertical_scrollbar_appearance(mode.scrollbar_appearance());
            editor.set_horizontal_scrollbar_appearance(mode.scrollbar_appearance());
            editor.set_interaction_state(interaction_state, ctx);
            editor.set_show_nav_bar(mode.show_nav_bar());
            editor.set_nav_bar_behavior(NavBarBehavior::NotClosable, ctx);
        });
    }
}

impl Entity for InlineDiffView {
    type Event = InlineDiffViewEvent;
}

impl View for InlineDiffView {
    fn ui_name() -> &'static str {
        "InlineDiffView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.editor).finish()
    }
}

impl TypedActionView for InlineDiffView {
    type Action = ();
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "inline_diff_tests.rs"]
mod tests;

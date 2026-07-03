use crate::ai::blocklist::inline_action::code_diff_view::DiffSessionType;
use ai::diff_validation::DiffType;
use std::rc::Rc;
use warp_files::{FileModel, FileModelEvent};
use warp_util::file::FileId;
use warp_util::file::FileSaveError;
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::ChildView;
use warpui::SingletonEntity;
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

use super::diff_viewer::DiffViewer;
use super::diff_viewer::DisplayMode;
use super::editor::scroll::{ScrollPosition, ScrollTrigger};
use super::editor::view::{CodeEditorEvent, CodeEditorView};
use super::editor::NavBarBehavior;
use super::DiffResult;
use crate::editor::InteractionState;

pub enum InlineDiffViewEvent {
    DiffStatusUpdated,
    FileLoaded,
    FileSaved,
    FailedToSave { error: Rc<FileSaveError> },
    DiffAccepted { diff: Rc<DiffResult> },
    UserEdited,
}

/// An inline diff viewer with optional file-backed save support.
///
/// When a backing file is registered (via [`Self::register_file`]), this view supports the full
/// accept/save/revert lifecycle through `FileModel`. Without a registered file, it behaves
/// as a read-only diff viewer (e.g. for restored conversations).
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
    /// When `None` (restored conversations or before registration):
    /// - The editor is selection-only (never editable).
    /// - Accept, save, and revert are no-ops.
    backing_file_id: Option<FileId>,
    /// Whether the diff is a new file creation (for revert: delete instead of restore).
    is_new_file: bool,
}

impl InlineDiffView {
    pub fn new(
        editor: ViewHandle<CodeEditorView>,
        diff_type: Option<DiffType>,
        display_mode: Option<DisplayMode>,
        file_path: Option<StandardizedPath>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let is_new_file = matches!(diff_type, Some(DiffType::Create { .. }));

        ctx.subscribe_to_view(&editor, |me, _view, event, ctx| match event {
            CodeEditorEvent::DiffUpdated => {
                ctx.emit(InlineDiffViewEvent::DiffStatusUpdated);
            }
            CodeEditorEvent::UnifiedDiffComputed(diff) => {
                ctx.emit(InlineDiffViewEvent::DiffAccepted { diff: diff.clone() });
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
            is_new_file,
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
    /// Remote sessions are read-only after hosted remote file access removal.
    ///
    /// This must be called after construction for registered local-file environments.
    pub fn register_file(&mut self, session_type: &DiffSessionType, ctx: &mut ViewContext<Self>) {
        let Some(file_path) = &self.file_path else {
            return;
        };

        let file_model = FileModel::handle(ctx);
        let file_id = match session_type {
            DiffSessionType::Local => {
                let Some(local_path) = file_path.to_local_path() else {
                    log::error!(
                        "Failed to convert StandardizedPath to local path: {file_path}; \
                         diff will be read-only",
                    );
                    return;
                };
                file_model.update(ctx, |file_model, ctx| {
                    file_model.register_file_path(&local_path, false, ctx)
                })
            }
            DiffSessionType::Remote(_) => {
                log::debug!("Skipping writable file registration for remote diff {file_path}");
                return;
            }
        };

        self.finish_file_registration(file_id, ctx);
    }

    /// Common registration logic: subscribes to events and sets the
    /// backing file ID after a file has been registered with `FileModel`.
    fn finish_file_registration(&mut self, file_id: FileId, ctx: &mut ViewContext<Self>) {
        let file_model = FileModel::handle(ctx);

        let version = self.editor.as_ref(ctx).version(ctx);
        file_model.update(ctx, |file_model, _ctx| {
            file_model.set_version(file_id, version);
        });

        self.backing_file_id = Some(file_id);

        // Subscribe to FileModel events for this file.
        ctx.subscribe_to_model(&file_model, move |_me, _file_model, event, ctx| {
            if file_id == event.file_id() {
                match event {
                    FileModelEvent::FileSaved { .. } => {
                        ctx.emit(InlineDiffViewEvent::FileSaved);
                    }
                    FileModelEvent::FailedToSave { error, .. } => {
                        ctx.emit(InlineDiffViewEvent::FailedToSave {
                            error: error.clone(),
                        });
                    }
                    _ => {}
                }
            }
        });

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
    fn save_content(&self, ctx: &mut ViewContext<Self>) {
        let Some(file_id) = self.backing_file_id else {
            return;
        };
        let content = self.editor.as_ref(ctx).text(ctx).into_string();
        let version = self.editor.as_ref(ctx).version(ctx);

        if let Err(err) = FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.save(file_id, content, version, ctx)
        }) {
            ctx.emit(InlineDiffViewEvent::FailedToSave {
                error: Rc::new(err),
            });
        }
    }
}

impl InlineDiffView {
    pub fn file_path(&self) -> Option<&StandardizedPath> {
        self.file_path.as_ref()
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
            // No file registered (e.g. restored conversations): always read-only.
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

    fn accept_and_save_diff(&self, ctx: &mut ViewContext<Self>) {
        // No-op when no file is registered (restored conversations).
        if self.backing_file_id.is_none() {
            return;
        }

        // Compute the unified diff (result arrives via CodeEditorEvent::UnifiedDiffComputed).
        if let Some(file_path) = &self.file_path {
            let file_name = file_path.to_string();
            self.editor.update(ctx, |editor, ctx| {
                editor.retrieve_unified_diff(file_name, ctx)
            });
        }
        // Save the current editor content to disk.
        self.save_content(ctx);
    }

    fn restore_diff_base(&mut self, _ctx: &mut ViewContext<Self>) -> Result<(), String> {
        // No-op when no file is registered (restored conversations).
        if self.backing_file_id.is_none() {
            return Ok(());
        }
        {
            let file_id = self
                .backing_file_id
                .expect("backing_file_id must be Some — checked by early return above");

            if self.is_new_file {
                // For newly created files, delete instead of restoring.
                let version = self.editor.as_ref(_ctx).version(_ctx);
                FileModel::handle(_ctx)
                    .update(_ctx, |file_model, ctx| {
                        file_model.delete(file_id, version, ctx)
                    })
                    .map_err(|e| format!("Failed to delete file: {e:?}"))?;
                return Ok(());
            }

            // For existing files, restore the base content from the editor's DiffModel.
            let base_content = self
                .editor
                .as_ref(_ctx)
                .model
                .as_ref(_ctx)
                .diff()
                .as_ref(_ctx)
                .base()
                .ok_or_else(|| "Missing base content".to_string())?
                .to_string();

            let version = self.editor.as_ref(_ctx).version(_ctx);
            FileModel::handle(_ctx)
                .update(_ctx, |file_model, ctx| {
                    file_model.save(file_id, base_content, version, ctx)
                })
                .map_err(|e| format!("Failed to save file: {e:?}"))?;
        }

        Ok(())
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

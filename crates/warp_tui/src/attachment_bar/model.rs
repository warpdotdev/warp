use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose;
use warp::editor::CodeEditorModel;
use warp::tui_export::{
    ActiveSession, BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIInputModel,
    ImageContext, InputType, InputTypeAutoDetectionSource, LLMPreferences,
    MAX_IMAGE_COUNT_FOR_QUERY, MIME_SNIFF_BYTES, PendingAttachmentSummary, ProcessImageResult,
    infer_mime_type, is_supported_image_mime_type, process_image_for_agent,
};
use warp_core::features::FeatureFlag;
use warp_editor::model::CoreEditorModel;
use warpui_core::r#async::SpawnedFutureHandle;
use warpui_core::clipboard::{ClipboardContent, ImageData};
use warpui_core::clipboard_utils::CLIPBOARD_IMAGE_MIME_TYPES;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _};

use crate::input_mode_policy::AI_LOCKED_CONFIG;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiAttachmentSnapshot {
    pub(crate) selected: Option<PendingAttachmentSummary>,
    pub(crate) position: Option<usize>,
    pub(crate) count: usize,
    pub(crate) is_processing: bool,
    pub(crate) selected_is_processing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiAttachmentModelEvent {
    Updated,
    AbortInputDetection,
    RequestInputDetection,
    RestorePastedText(String),
    ShowHint(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiAttachmentPasteDisposition {
    Started,
    Handled,
    NotHandled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachmentModeTransition {
    None,
    LockAgent,
    RestoreAgent { request_detection: bool },
}

pub(crate) struct TuiAttachmentModel {
    context_model: ModelHandle<BlocklistAIContextModel>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    input_editor: ModelHandle<CodeEditorModel>,
    active_session: ModelHandle<ActiveSession>,
    terminal_surface_id: EntityId,
    selected_index: Option<usize>,
    /// Last observed shared-context count. Growth selects the newest item;
    /// shrinkage preserves and clamps the current selection.
    last_attachment_count: usize,
    had_locking_attachment: bool,
    processing_file_name: Option<String>,
    processing_count: usize,
    in_flight: Option<SpawnedFutureHandle>,
}

impl TuiAttachmentModel {
    pub(crate) fn new(
        context_model: ModelHandle<BlocklistAIContextModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        input_editor: ModelHandle<CodeEditorModel>,
        active_session: ModelHandle<ActiveSession>,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let initial_attachment_count = context_model.as_ref(ctx).pending_attachments().len();
        let had_locking_attachment = context_model.as_ref(ctx).has_locking_attachment();
        ctx.subscribe_to_model(&context_model, |model, _, event, ctx| {
            if matches!(event, BlocklistAIContextEvent::UpdatedPendingContext { .. }) {
                model.sync_from_context(ctx);
            }
        });
        Self {
            context_model,
            input_mode,
            input_editor,
            active_session,
            terminal_surface_id,
            selected_index: initial_attachment_count.checked_sub(1),
            last_attachment_count: initial_attachment_count,
            had_locking_attachment,
            processing_file_name: None,
            processing_count: 0,
            in_flight: None,
        }
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> TuiAttachmentSnapshot {
        let summaries = self
            .context_model
            .as_ref(ctx)
            .pending_attachment_summaries();
        let selected_index = self
            .selected_index
            .filter(|index| *index < summaries.len())
            .or_else(|| summaries.len().checked_sub(1));
        if let Some(file_name) = &self.processing_file_name {
            let count = summaries.len() + self.processing_count;
            TuiAttachmentSnapshot {
                selected: Some(PendingAttachmentSummary {
                    index: count.saturating_sub(1),
                    attachment_type: warp::tui_export::AttachmentType::Image,
                    file_name: file_name.clone(),
                }),
                position: Some(count),
                count,
                is_processing: true,
                selected_is_processing: true,
            }
        } else {
            TuiAttachmentSnapshot {
                selected: selected_index.and_then(|index| summaries.get(index).cloned()),
                position: selected_index.map(|index| index + 1),
                count: summaries.len(),
                is_processing: self.is_processing(),
                selected_is_processing: false,
            }
        }
    }

    pub(crate) fn should_render(&self, ctx: &AppContext) -> bool {
        self.is_processing() || self.has_attachments(ctx)
    }

    pub(crate) fn has_attachments(&self, ctx: &AppContext) -> bool {
        !self
            .context_model
            .as_ref(ctx)
            .pending_attachments()
            .is_empty()
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let count = self.context_model.as_ref(ctx).pending_attachments().len();
        if count < 2 {
            return;
        }
        self.selected_index = Some(self.selected_index.unwrap_or_default().wrapping_add(1) % count);
        ctx.emit(TuiAttachmentModelEvent::Updated);
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let count = self.context_model.as_ref(ctx).pending_attachments().len();
        if count < 2 {
            return;
        }
        let selected = self.selected_index.unwrap_or_default();
        self.selected_index = Some(if selected == 0 {
            count - 1
        } else {
            selected - 1
        });
        ctx.emit(TuiAttachmentModelEvent::Updated);
    }

    pub(crate) fn remove_selected(&mut self, ctx: &mut ModelContext<Self>) {
        if self.processing_file_name.is_some() {
            if let Some(in_flight) = self.in_flight.take() {
                in_flight.abort();
            }
            self.finish_processing();
            ctx.emit(TuiAttachmentModelEvent::Updated);
            return;
        }
        let Some(index) = self.selected_index else {
            return;
        };
        self.context_model.update(ctx, |context_model, ctx| {
            context_model.remove_pending_attachment(index, ctx);
        });
    }

    pub(crate) fn try_attach_paste(
        &mut self,
        text: String,
        ctx: &mut ModelContext<Self>,
    ) -> TuiAttachmentPasteDisposition {
        if !FeatureFlag::ImageAsContext.is_enabled() {
            return TuiAttachmentPasteDisposition::NotHandled;
        }
        let Some(paths) = parse_image_paths(&text, &self.current_working_directory(ctx)) else {
            return TuiAttachmentPasteDisposition::NotHandled;
        };
        if let Err(error) = self.validate_new_images(paths.len(), ctx) {
            ctx.emit(TuiAttachmentModelEvent::RestorePastedText(text));
            ctx.emit(TuiAttachmentModelEvent::ShowHint(error));
            return TuiAttachmentPasteDisposition::Handled;
        }

        let processing_file_name = paths
            .last()
            .and_then(|path| path.file_name())
            .map(|file_name| file_name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".to_owned());
        self.start_processing(processing_file_name, paths.len(), ctx);
        let original_text = text;
        self.in_flight = Some(ctx.spawn_abortable(
            process_paths(paths),
            move |model, result, ctx| {
                model.in_flight = None;
                model.finish_processing();
                match result {
                    Ok(images) => {
                        model.context_model.update(ctx, |context_model, ctx| {
                            context_model.append_pending_images(images, ctx);
                        });
                    }
                    Err(error) => {
                        ctx.emit(TuiAttachmentModelEvent::RestorePastedText(
                            original_text.clone(),
                        ));
                        ctx.emit(TuiAttachmentModelEvent::ShowHint(error));
                    }
                }
                ctx.emit(TuiAttachmentModelEvent::Updated);
            },
            |_, _| {},
        ));
        TuiAttachmentPasteDisposition::Started
    }

    pub(crate) fn paste_image_from_clipboard(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        if let Err(error) = self.validate_new_images(1, ctx) {
            ctx.emit(TuiAttachmentModelEvent::ShowHint(error));
            return false;
        }
        self.start_processing("clipboard-image.png".to_owned(), 1, ctx);
        self.in_flight = Some(ctx.spawn_abortable(
            read_and_process_clipboard_image(),
            |model, result, ctx| {
                model.in_flight = None;
                model.finish_processing();
                match result {
                    Ok(image) => {
                        model.context_model.update(ctx, |context_model, ctx| {
                            context_model.append_pending_images(vec![image], ctx);
                        });
                    }
                    Err(error) => ctx.emit(TuiAttachmentModelEvent::ShowHint(error)),
                }
                ctx.emit(TuiAttachmentModelEvent::Updated);
            },
            |_, _| {},
        ));
        true
    }

    fn start_processing(&mut self, file_name: String, count: usize, ctx: &mut ModelContext<Self>) {
        self.processing_file_name = Some(file_name);
        self.processing_count = count;
        ctx.emit(TuiAttachmentModelEvent::Updated);
    }

    fn finish_processing(&mut self) {
        self.processing_file_name = None;
        self.processing_count = 0;
    }

    fn is_processing(&self) -> bool {
        self.processing_count > 0
    }

    fn validate_new_images(&self, count: usize, ctx: &AppContext) -> Result<(), String> {
        if !FeatureFlag::ImageAsContext.is_enabled() {
            return Err("Image attachments are unavailable.".to_owned());
        }
        if self.is_processing() {
            return Err("Wait for the current image attachment to finish.".to_owned());
        }
        let attached_images = self.context_model.as_ref(ctx).pending_images().len();
        if attached_images + count > MAX_IMAGE_COUNT_FOR_QUERY {
            return Err(format!(
                "Image attachment limit is {MAX_IMAGE_COUNT_FOR_QUERY} per query."
            ));
        }
        if !LLMPreferences::as_ref(ctx).vision_supported(ctx, Some(self.terminal_surface_id)) {
            return Err("The selected model does not support image attachments.".to_owned());
        }
        Ok(())
    }

    fn current_working_directory(&self, ctx: &AppContext) -> PathBuf {
        self.active_session
            .as_ref(ctx)
            .current_working_directory()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    fn input_is_empty(&self, ctx: &AppContext) -> bool {
        self.input_editor
            .as_ref(ctx)
            .content()
            .as_ref(ctx)
            .is_empty()
    }

    fn sync_from_context(&mut self, ctx: &mut ModelContext<Self>) {
        let attachment_count = self.context_model.as_ref(ctx).pending_attachments().len();
        self.selected_index = reconciled_selected_index(
            self.last_attachment_count,
            attachment_count,
            self.selected_index,
        );
        self.last_attachment_count = attachment_count;

        let has_locking_attachment = self.context_model.as_ref(ctx).has_locking_attachment();
        let input_is_empty = self.input_is_empty(ctx);
        let autodetection_enabled = self
            .input_mode
            .as_ref(ctx)
            .is_autodetection_enabled_for_current_context(ctx);
        match attachment_mode_transition(
            self.had_locking_attachment,
            has_locking_attachment,
            autodetection_enabled,
            input_is_empty,
        ) {
            AttachmentModeTransition::LockAgent => {
                self.input_mode.update(ctx, |input_mode, ctx| {
                    input_mode.set_input_config(
                        AI_LOCKED_CONFIG,
                        input_is_empty,
                        Some(InputTypeAutoDetectionSource::AttachmentForcedAi),
                        ctx,
                    );
                });
                ctx.emit(TuiAttachmentModelEvent::AbortInputDetection);
            }
            AttachmentModeTransition::RestoreAgent { request_detection } => {
                self.input_mode.update(ctx, |input_mode, ctx| {
                    if autodetection_enabled {
                        input_mode.enable_autodetection(InputType::AI, ctx);
                    } else {
                        input_mode.set_input_config(AI_LOCKED_CONFIG, input_is_empty, None, ctx);
                    }
                });
                if request_detection {
                    ctx.emit(TuiAttachmentModelEvent::RequestInputDetection);
                }
            }
            AttachmentModeTransition::None => {}
        }
        self.had_locking_attachment = has_locking_attachment;
        ctx.emit(TuiAttachmentModelEvent::Updated);
    }
}

impl Entity for TuiAttachmentModel {
    type Event = TuiAttachmentModelEvent;
}

fn reconciled_selected_index(
    previous_count: usize,
    count: usize,
    selected_index: Option<usize>,
) -> Option<usize> {
    if count == 0 {
        None
    } else if count > previous_count {
        Some(count - 1)
    } else {
        Some(selected_index.unwrap_or(count - 1).min(count - 1))
    }
}

fn attachment_mode_transition(
    had_locking_attachment: bool,
    has_locking_attachment: bool,
    autodetection_enabled: bool,
    input_is_empty: bool,
) -> AttachmentModeTransition {
    match (had_locking_attachment, has_locking_attachment) {
        (false, true) => AttachmentModeTransition::LockAgent,
        (true, false) => AttachmentModeTransition::RestoreAgent {
            request_detection: autodetection_enabled && !input_is_empty,
        },
        (false, false) | (true, true) => AttachmentModeTransition::None,
    }
}

fn parse_image_paths(text: &str, cwd: &Path) -> Option<Vec<PathBuf>> {
    let tokens = shell_words::split(text.trim()).ok()?;
    if tokens.is_empty() {
        return None;
    }
    tokens
        .into_iter()
        .map(|token| resolve_image_path(&token, cwd))
        .collect()
}

fn resolve_image_path(token: &str, cwd: &Path) -> Option<PathBuf> {
    let token = token.strip_prefix("file://").unwrap_or(token);
    let path = if token == "~" {
        dirs::home_dir()?
    } else if let Some(rest) = token.strip_prefix("~/") {
        dirs::home_dir()?.join(rest)
    } else {
        PathBuf::from(token)
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp").then_some(path)
}

async fn process_paths(paths: Vec<PathBuf>) -> Result<Vec<ImageContext>, String> {
    let mut images = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = async_fs::metadata(&path)
            .await
            .map_err(|_| format!("Could not read image {}.", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("Image path is not a file: {}.", path.display()));
        }
        let bytes = async_fs::read(&path)
            .await
            .map_err(|_| format!("Could not read image {}.", path.display()))?;
        let mime_type = infer_mime_type(&path, &bytes[..bytes.len().min(MIME_SNIFF_BYTES)]);
        if !is_supported_image_mime_type(&mime_type) {
            return Err(format!(
                "Unsupported image type for {}. Use PNG, JPG, GIF, or WebP.",
                path.display()
            ));
        }
        let data = match process_image_for_agent(&bytes) {
            ProcessImageResult::Success { data } => data,
            ProcessImageResult::TooLarge => {
                return Err(format!("Image is too large: {}.", path.display()));
            }
            ProcessImageResult::Error(_) => {
                return Err(format!("Could not process image {}.", path.display()));
            }
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(format!("Image has no valid filename: {}.", path.display()));
        };
        images.push(ImageContext {
            data: general_purpose::STANDARD.encode(data),
            mime_type,
            file_name: file_name.to_owned(),
            is_figma: false,
        });
    }
    Ok(images)
}

async fn read_and_process_clipboard_image() -> Result<ImageContext, String> {
    let content = blocking::unblock(|| -> Result<ClipboardContent, String> {
        let mut clipboard = warpui::platform::create_system_clipboard()
            .map_err(|_| "The system clipboard is unavailable.".to_owned())?;
        Ok(clipboard.read())
    })
    .await?;
    process_clipboard_content(content)
}

fn process_clipboard_content(content: ClipboardContent) -> Result<ImageContext, String> {
    let images = content
        .images
        .ok_or_else(|| "The clipboard does not contain an image.".to_owned())?;
    let image = CLIPBOARD_IMAGE_MIME_TYPES
        .iter()
        .find_map(|mime_type| {
            images
                .iter()
                .find(|image| image.mime_type == *mime_type)
                .cloned()
        })
        .ok_or_else(|| "The clipboard does not contain a supported image.".to_owned())?;
    process_clipboard_image_data(image)
}

fn process_clipboard_image_data(image: ImageData) -> Result<ImageContext, String> {
    let data = match process_image_for_agent(&image.data) {
        ProcessImageResult::Success { data } => data,
        ProcessImageResult::TooLarge => return Err("The clipboard image is too large.".to_owned()),
        ProcessImageResult::Error(_) => {
            return Err("The clipboard image could not be processed.".to_owned());
        }
    };
    Ok(ImageContext {
        data: general_purpose::STANDARD.encode(data),
        mime_type: image.mime_type,
        file_name: image
            .filename
            .unwrap_or_else(|| "clipboard-image.png".to_owned()),
        is_figma: false,
    })
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

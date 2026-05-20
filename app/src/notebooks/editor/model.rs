use crate::notebooks::file::MarkdownDisplayMode;
use base64::{prelude::BASE64_STANDARD, Engine as _};
use std::{
    any::Any,
    borrow::Cow,
    collections::{HashMap, HashSet},
    ops::Range,
    time::Duration,
};

use itertools::Itertools;
use lazy_static::lazy_static;
use markdown_parser::FormattedText;
use mermaid_to_svg::MermaidTheme;
use num_traits::SaturatingSub;
use regex::Regex;
use url::Url;
use vec1::{vec1, Vec1};
use warpui::{
    accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole},
    clipboard::ClipboardContent,
    AppContext, Entity, ModelAsRef, ModelContext, ModelHandle, SingletonEntity, WindowId,
};

use crate::{
    cloud_object::model::persistence::{CloudModel, CloudModelEvent},
    debounce::debounce,
    editor::InteractionState,
    notebooks::telemetry::BlockInfo,
};
use crate::{
    notebooks::editor::interaction_state_model::InteractionStateModelEvent,
    terminal::ShellLaunchData,
};
use string_offset::CharOffset;
use warp_core::features::FeatureFlag;
use warp_core::semantic_selection::SemanticSelection;
use warp_editor::{
    content::{buffer::ShouldAutoscroll, selection_model::BufferSelectionModel},
    model::BufferUpdateWrapper,
    render::model::{BlockItem, StyleUpdateAction},
};
use warp_editor::{
    content::{
        buffer::{
            AutoScrollBehavior, Buffer, BufferEditAction, BufferEvent, BufferSelectAction,
            EditOrigin, SelectionOffsets,
        },
        text::{
            BlockHeaderSize, BlockType, BufferBlockItem, BufferBlockStyle, BufferTextStyle,
            CodeBlockType, IndentBehavior, IndentUnit, TextStyles, TextStylesWithMetadata,
        },
    },
    model::{CoreEditorModel, RichTextEditorModel},
    render::model::{AutoScrollMode, RenderEvent, RenderState, RichTextStyles},
    search::Searcher,
    selection::{SelectionMode, SelectionModel, TextDirection, TextUnit},
};
use warpui::elements::ListIndentLevel;

use super::{
    super::telemetry::SelectionMode as TelemetrySelectionMode, embedding_model::NotebookEmbed,
    interaction_state_model::InteractionStateModel, notebook_command::NotebookCommand,
    NotebookWorkflow,
};

const DEBOUNCED_RESIZE_PERIOD: Duration = Duration::from_millis(5);

lazy_static! {
    // Specifically match ASCII digits using [[:digit:]], rather than \d, which is Unicode-aware.
    static ref NUMBERED_LIST_SHORTCUT_PREFIX: Regex = Regex::new(r"^([[:digit:]]+)\. $").expect("Markdown shortcut regex should be valid");
    // Note this is slightly different from markdown syntax, which supports interleaving asterisk, dash and underline.
    // For markdown shortcut specifically, this feels more closely mapped to user intention.
    static ref HORIZONTAL_RULE_SHORTCUT_PREFIX: Regex = Regex::new(r"^(\*\*\*+|---+|___+) $").expect("Markdown shortcut regex should be valid");

    static ref ITALIC_INLINE_REGEX: Regex =
        Regex::new(r"\*([^\*]+?)\*$").expect("Markdown shortcut regex should be valid");
    static ref BOLD_INLINE_REGEX: Regex =
        Regex::new(r"\*\*([^\*]+?)\*\*$").expect("Markdown shortcut regex should be valid");
    static ref INLINE_CODE_INLINE_REGEX: Regex =
        Regex::new(r"`([^`]+?)`$").expect("Markdown shortcut regex should be valid");
    static ref BOLD_ITALIC_INLINE_REGEX: Regex =
        Regex::new(r"\*\*\*([^\*]+?)\*\*\*$").expect("Markdown shortcut regex should be valid");
    // Underscore-delimited variants of italic, bold, and bold-italic inline markdown.
    // Matches `_text_`, `__text__`, and `___text___` respectively.
    static ref ITALIC_UNDERSCORE_INLINE_REGEX: Regex =
        Regex::new(r"_([^_]+?)_$").expect("Markdown shortcut regex should be valid");
    static ref BOLD_UNDERSCORE_INLINE_REGEX: Regex =
        Regex::new(r"__([^_]+?)__$").expect("Markdown shortcut regex should be valid");
    static ref BOLD_ITALIC_UNDERSCORE_INLINE_REGEX: Regex =
        Regex::new(r"___([^_]+?)___$").expect("Markdown shortcut regex should be valid");
    static ref STRIKETHROUGH_REGEX: Regex =
        Regex::new(r"~~([^~]+?)~~$").expect("Markdown shortcut regex should be valid");
    static ref LINK_INLINE_REGEX: Regex =
        Regex::new(r"\[(.+?)\]\((.+?)\)$").expect("Markdown shortcut regex should be valid");
    static ref INCOMPLETE_TASKLIST_INLINE_REGEX: Regex =
        Regex::new(r"^\[ ?\] $").expect("Markdown shortcut regex should be valid");
    static ref COMPLETE_TASKLIST_INLINE_REGEX: Regex =
        Regex::new(r"^\[[xX]\] $").expect("Markdown shortcut regex should be valid");
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

/// Model for managing the state of the editor.
pub struct NotebooksEditorModel {
    pub(super) render_state: ModelHandle<RenderState>,
    content: ModelHandle<Buffer>,
    selection_model: ModelHandle<BufferSelectionModel>,
    selection: ModelHandle<SelectionModel>,
    child_models: ChildModels,
    active_text_style: TextStyles,
    active_block_type: BlockType,
    interaction_state: ModelHandle<InteractionStateModel>,
    rte_window_id: Option<WindowId>, // Needed for rendering any subviews from the model directly. Set when pane is attached.
    /// Channel for debounced viewport resizing.
    resize_tx: async_channel::Sender<()>,
    /// Context used to generate clickable file path links for notebooks.
    file_link_resolution_context: Option<FileLinkResolutionContext>,
    default_mermaid_display_mode: MarkdownDisplayMode,
}

#[derive(Clone)]
/// Context used to generate clickable file path links for notebooks.
pub struct FileLinkResolutionContext {
    /// The working directory of the session that the editor is associated with.
    pub working_directory: String,
    /// The shell launch data of the session that the editor is associated with.
    pub shell_launch_data: Option<ShellLaunchData>,
}

#[derive(PartialEq)]
enum InlineStyleAction {
    Insert {
        matched_length: usize,
        text: String,
        style: TextStyles,
        override_text_style: Option<TextStyles>,
    },
    Link {
        matched_length: usize,
        tag: String,
        url: String,
    },
    None,
}

fn mermaid_image_html(svg: &[u8]) -> String {
    format!(
        "<img src=\"data:image/svg+xml;base64,{}\" alt=\"Mermaid diagram\" />",
        BASE64_STANDARD.encode(svg)
    )
}

fn render_mermaid_clipboard_html(source: &str) -> Option<String> {
    let svg = mermaid_to_svg::render_mermaid_to_svg(source, Some(&MermaidTheme::light()))
        .ok()?
        .into_bytes();
    Some(mermaid_image_html(&svg))
}

impl NotebooksEditorModel {
    fn editable_markdown_mermaid_enabled() -> bool {
        FeatureFlag::MarkdownMermaid.is_enabled()
            && FeatureFlag::EditableMarkdownMermaid.is_enabled()
    }

    pub fn new(
        text_styles: RichTextStyles,
        rte_window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::new_internal(text_styles, Some(rte_window_id), ctx)
    }

    /// Create a model that is not yet bound to a window. The window id should be set later via `set_window_id`.
    pub fn new_unbound(text_styles: RichTextStyles, ctx: &mut ModelContext<Self>) -> Self {
        Self::new_internal(text_styles, None, ctx)
    }

    fn new_internal(
        text_styles: RichTextStyles,
        rte_window_id: Option<WindowId>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let content = ctx.add_model(|_| {
            Buffer::new(Box::new(notebook_tab_indentation))
                .with_embedded_item_conversion(super::notebook_embedded_item_conversion)
        });
        ctx.subscribe_to_model(&content, |me, event, ctx| {
            me.handle_content_model_event(event, ctx);
        });

        let selection_model = ctx.add_model(|_ctx| BufferSelectionModel::new(content.clone()));

        let render_state = ctx.add_model(|ctx| RenderState::new(text_styles, false, None, ctx));
        ctx.subscribe_to_model(&render_state, Self::handle_render_model_event);

        let selection = ctx.add_model(|ctx| {
            SelectionModel::new(
                content.clone(),
                render_state.clone(),
                selection_model.clone(),
                None,
                ctx,
            )
        });

        let interaction_state =
            ctx.add_model(|_| InteractionStateModel::new(InteractionState::Editable));
        ctx.subscribe_to_model(
            &interaction_state,
            Self::handle_interaction_state_model_event,
        );

        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, event, ctx| {
            me.handle_cloud_model_event(event, ctx)
        });

        let (resize_tx, resize_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(
            debounce(DEBOUNCED_RESIZE_PERIOD, resize_rx),
            |me, _, ctx| me.rebuild_layout(ctx),
            |_, _| {},
        );

        Self {
            render_state,
            content,
            selection,
            selection_model,
            child_models: ChildModels::new(),
            active_text_style: Default::default(),
            active_block_type: BlockType::Text(BufferBlockStyle::PlainText),
            interaction_state,
            rte_window_id,
            resize_tx,
            file_link_resolution_context: None,
            default_mermaid_display_mode: MarkdownDisplayMode::Raw,
        }
    }

    pub fn interaction_state(&self, ctx: &AppContext) -> InteractionState {
        self.interaction_state.as_ref(ctx).interaction_state()
    }

    pub fn render_state(&self) -> &ModelHandle<RenderState> {
        &self.render_state
    }

    pub fn markdown_table_count(&self, ctx: &impl ModelAsRef) -> usize {
        self.render_state.as_ref(ctx).markdown_table_count()
    }

    pub fn set_interaction_state(
        &mut self,
        new_state: InteractionState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.interaction_state
            .update(ctx, |interaction_state, ctx| {
                interaction_state.set_interaction_state(new_state, ctx)
            });
    }

    /// Set the window this editor model is associated with. Should be called when the pane attaches.
    pub fn set_window_id(&mut self, window_id: WindowId, _ctx: &mut ModelContext<Self>) {
        self.rte_window_id = Some(window_id);
    }

    /// Get the context for the session and working directory associated with this editor, if any.
    /// Used to generate clickable file path links.
    pub fn file_link_resolution_context(&self) -> Option<&FileLinkResolutionContext> {
        self.file_link_resolution_context.as_ref()
    }

    /// Set the context for the session and working directory associated with this editor, if any.
    /// Used to generate clickable file path links.
    pub fn set_file_link_resolution_context(
        &mut self,
        file_link_resolution_context: Option<FileLinkResolutionContext>,
    ) {
        self.file_link_resolution_context = file_link_resolution_context;
    }

    pub fn set_default_mermaid_display_mode(
        &mut self,
        mode: MarkdownDisplayMode,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.default_mermaid_display_mode == mode {
            return;
        }
        self.default_mermaid_display_mode = mode;

        for command in self
            .child_models
            .model_handles::<NotebookCommand>()
            .collect_vec()
        {
            command.update(ctx, |command, _| {
                command.mermaid_display_mode = mode;
            });
        }

        if self.sync_mermaid_render_offsets(ctx) {
            self.rebuild_layout(ctx);
        } else {
            ctx.notify();
        }
    }

    /// Create a new model for searching the editor.
    pub fn new_search(&self, ctx: &mut ModelContext<Self>) -> ModelHandle<Searcher> {
        let buffer = self.content.clone();
        let selection_model = self.selection_model.clone();
        ctx.add_model(|ctx| Searcher::new(buffer, selection_model, ctx))
    }
    pub fn reset_with_markdown(&mut self, markdown: &str, ctx: &mut ModelContext<Self>) {
        <Self as RichTextEditorModel>::reset_with_markdown(self, markdown, ctx);
    }

    pub fn update_to_new_markdown(&mut self, markdown: &str, ctx: &mut ModelContext<Self>) {
        <Self as RichTextEditorModel>::update_to_new_markdown(self, markdown, ctx);
    }

    fn handle_render_model_event(&mut self, event: &RenderEvent, ctx: &mut ModelContext<Self>) {
        // Ignore render events until bound to a real window, and when the window is closed.
        let Some(window_id) = self.rte_window_id else {
            return;
        };
        if !ctx.is_window_open(window_id) {
            log::debug!("Ignoring render event for closed window");
            return;
        }

        match event {
            RenderEvent::NeedsResize => {
                // When a debounced resize event fires, the model is laid out from scratch, using [`Self::rebuild_layout`].
                let _ = self.resize_tx.try_send(());
            }
            RenderEvent::LayoutUpdated => {
                self.child_models.update(
                    self.interaction_state.clone(),
                    self.content.clone(),
                    self.selection_model.clone(),
                    window_id,
                    self.default_mermaid_display_mode,
                    ctx,
                );
                if self.sync_mermaid_render_offsets(ctx) {
                    self.rebuild_layout(ctx);
                }
            }
            _ => (),
        }
    }

    fn handle_interaction_state_model_event(
        &mut self,
        event: &InteractionStateModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let InteractionStateModelEvent::InteractionStateChanged { new_state } = event;
        let show_final_trailing_newline_when_non_empty = matches!(
            new_state,
            InteractionState::Editable | InteractionState::EditableWithInvalidSelection
        );
        self.render_state.update(ctx, |render_state, _| {
            render_state.set_show_final_trailing_newline_when_non_empty(
                show_final_trailing_newline_when_non_empty,
            );
        });
    }

    /// Set the Mermaid display mode (Raw or Rendered) for the block at the given offset.
    /// `block_offset` is the buffer's 0-indexed block marker position (i.e. `outline.start`).
    /// Updates `mermaid_render_offsets` in the render state and triggers relayout if changed.
    pub fn set_mermaid_render_mode(
        &mut self,
        block_offset: CharOffset,
        mode: MarkdownDisplayMode,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(command) = self.child_models.model_at::<NotebookCommand>(block_offset) {
            command.update(ctx, |cmd, _| {
                cmd.mermaid_display_mode = mode;
            });
        }

        if self.sync_mermaid_render_offsets(ctx) {
            self.rebuild_layout(ctx);
        }
    }

    fn sync_mermaid_render_offsets(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        let new_offsets: HashSet<CharOffset> = self
            .child_models
            .model_handles::<NotebookCommand>()
            .filter_map(|handle| {
                if matches!(
                    handle.as_ref(ctx).mermaid_display_mode,
                    MarkdownDisplayMode::Rendered
                ) {
                    handle
                        .as_ref(ctx)
                        .start_offset(ctx)
                        .map(|o| o + CharOffset::from(1))
                } else {
                    None
                }
            })
            .collect();

        self.render_state
            .update(ctx, |rs, _| rs.set_mermaid_render_offsets(new_offsets))
    }

    fn handle_content_model_event(&mut self, event: &BufferEvent, ctx: &mut ModelContext<Self>) {
        if let Some(window_id) = self.rte_window_id {
            if !ctx.is_window_open(window_id) {
                log::debug!("Ignoring content event for closed window");
                return;
            }
        }

        let can_edit = matches!(self.interaction_state(ctx), InteractionState::Editable);
        match event {
            BufferEvent::ContentChanged {
                delta,
                origin,
                should_autoscroll,
                buffer_version,
                ..
            } => {
                self.render_state.update(ctx, move |render_state, _| {
                    render_state.add_pending_edit(delta.clone(), *buffer_version);
                    if can_edit {
                        match should_autoscroll {
                            ShouldAutoscroll::Yes => render_state.request_autoscroll(),
                            ShouldAutoscroll::VerticalOnly => {
                                render_state.request_vertical_autoscroll()
                            }
                            ShouldAutoscroll::No => (),
                        }
                    }
                });

                ctx.emit(RichTextEditorModelEvent::ContentChanged(*origin));
            }
            BufferEvent::SelectionChanged {
                active_text_styles,
                active_block_type,
                should_autoscroll,
                buffer_version,
            } => {
                let selection_text_styles =
                    self.selection_model.as_ref(ctx).selection_text_styles(ctx);
                let mut selections = self
                    .content
                    .as_ref(ctx)
                    .to_rendered_selection_set(self.selection_model.clone(), ctx);

                for selection in selections.iter_mut() {
                    selection.head -= CharOffset::from(1);
                    selection.tail -= CharOffset::from(1);
                }

                self.render_state.update(ctx, move |render_state, _| {
                    render_state.update_selection(selections, *buffer_version);
                    if matches!(should_autoscroll, AutoScrollBehavior::Selection) && can_edit {
                        render_state.request_autoscroll();
                    }
                });

                let text_styles: TextStyles = active_text_styles.clone().into();
                self.active_text_style = text_styles.inheritable();
                self.active_block_type = active_block_type.clone();

                ctx.emit(RichTextEditorModelEvent::ActiveStylesChanged {
                    cursor_text_styles: active_text_styles.clone(),
                    selection_text_styles,
                    block_type: active_block_type.clone(),
                })
            }
            // Handled by selection model.
            BufferEvent::AnchorUpdated { .. } | BufferEvent::ContentReplaced { .. } => (),
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ModelContext<Self>) {
        // Ignore cloud events until bound to a real window, and when the window is closed.
        let Some(window_id) = self.rte_window_id else {
            return;
        };
        if !ctx.is_window_open(window_id) {
            return;
        }
        match event {
            CloudModelEvent::ObjectUpdated { type_and_id, .. }
            | CloudModelEvent::ObjectTrashed { type_and_id, .. }
            | CloudModelEvent::ObjectUntrashed { type_and_id, .. }
            | CloudModelEvent::ObjectDeleted { type_and_id, .. }
            | CloudModelEvent::ObjectMoved { type_and_id, .. } => {
                if let Some(model) = self
                    .child_models
                    .model_handles::<NotebookEmbed>()
                    .find(|model| model.as_ref(ctx).hashed_id() == type_and_id.sqlite_uid_hash())
                {
                    model.update(ctx, |model, ctx| {
                        model.refresh_item_state(ctx);
                    })
                }
            }
            _ => (),
        }
    }

    /// Find the [`NotebookCommand`] model backing a laid-out block item.
    pub fn notebook_command_for_block(
        &self,
        offset: CharOffset,
    ) -> Option<ModelHandle<NotebookCommand>> {
        self.child_models.model_at(offset)
    }

    pub fn notebook_embed_for_block(
        &self,
        offset: CharOffset,
    ) -> Option<ModelHandle<NotebookEmbed>> {
        self.child_models.model_at(offset)
    }

    pub fn markdown(&self, ctx: &AppContext) -> String {
        self.content.as_ref(ctx).markdown()
    }

    /// Returns true if the editor's markdown content is empty or contains only whitespace.
    pub fn is_empty(&self, ctx: &AppContext) -> bool {
        self.markdown(ctx).trim().is_empty()
    }

    /// Returns the markdown content without escaping any special characters.
    pub fn markdown_unescaped(&self, ctx: &AppContext) -> String {
        self.content.as_ref(ctx).markdown_unescaped()
    }

    /// Returns a debug representation of the buffer contents.
    pub fn debug_buffer(&self, app: &impl ModelAsRef) -> String {
        self.content.as_ref(app).debug()
    }

    /// Returns a debug representation of the current selection's contents.
    pub fn debug_selection(&self, app: &AppContext) -> String {
        self.content
            .as_ref(app)
            .debug_selection(self.selection_model.clone(), app)
    }

    pub fn forward_word(&mut self, select: bool, ctx: &mut ModelContext<Self>) {
        self.forward_word_with_unit(select, word_unit(ctx), ctx)
    }

    pub fn backward_word(&mut self, select: bool, ctx: &mut ModelContext<Self>) {
        self.backward_word_with_unit(select, word_unit(ctx), ctx)
    }

    pub fn toggle_style(&mut self, style: TextStyles, ctx: &mut ModelContext<Self>) {
        if self.selection_model.as_ref(ctx).all_single_cursors() {
            self.active_text_style ^= style;
        } else {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    let selections_to_offset_ranges =
                        selection_model.as_ref(ctx).selections_to_offset_ranges();
                    let selection_fully_styled = content
                        .buffer()
                        .ranges_fully_styled(selections_to_offset_ranges, style);
                    let action = if selection_fully_styled {
                        BufferEditAction::Unstyle(style)
                    } else {
                        BufferEditAction::Style(style)
                    };

                    content.apply_edit(action, EditOrigin::UserInitiated, selection_model, ctx);
                },
                ctx,
            );
        }
        self.validate(ctx);
    }

    /// Tests if the given style is active at the cursor.
    pub fn is_style_active(&self, style: BufferTextStyle) -> bool {
        self.active_text_style.exact_match_style(&style)
    }

    /// Begin selecting at the given `offset`. Returns whether there was previously a command
    /// selection.
    pub fn select_at(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.begin_selection(offset, SelectionMode::Character, !multiselect, ctx);
        self.clear_command_selections(ctx)
    }

    /// Begin semantic selection by word.
    pub fn select_word_at(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.clear_command_selections(ctx);
        let policy = SemanticSelection::as_ref(ctx).word_boundary_policy();
        self.begin_selection(offset, SelectionMode::Word(policy), !multiselect, ctx);
    }

    /// Begin semantic selection by line.
    pub fn select_line_at(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.clear_command_selections(ctx);
        self.begin_selection(offset, SelectionMode::Line, !multiselect, ctx);
    }

    fn inline_markdown_shortcut(
        &self,
        mut content: BufferUpdateWrapper,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Buffer>,
    ) {
        // We will only take an action if all selections can be styled the same way, with the exact same
        // content being selected.
        let mut previous_option = None;
        for cursor in selection_model.as_ref(ctx).selection_heads() {
            let action = self.inline_markdown_for_selection(content.buffer(), cursor);
            if let Some(previous) = &previous_option {
                if *previous != action {
                    return;
                }
            } else {
                previous_option = Some(action);
            }
        }

        if let Some(action) = previous_option {
            match action {
                InlineStyleAction::Insert {
                    matched_length,
                    text,
                    style,
                    override_text_style,
                } => {
                    NotebooksEditorModel::apply_inline_style_helper(
                        content,
                        selection_model.clone(),
                        matched_length,
                        BufferEditAction::Insert {
                            text: text.as_str(),
                            style,
                            override_text_style,
                        },
                        ctx,
                    );
                }
                InlineStyleAction::Link {
                    matched_length,
                    tag,
                    url,
                } => {
                    NotebooksEditorModel::apply_inline_style_helper(
                        content,
                        selection_model.clone(),
                        matched_length,
                        BufferEditAction::Link { tag, url },
                        ctx,
                    );
                }
                InlineStyleAction::None => (),
            }
        }
    }

    /// Compute the action that should be taken for a given selection based on the markdown shortcuts input
    /// by the user.
    fn inline_markdown_for_selection(
        &self,
        content: &mut Buffer,
        selection_offset: CharOffset,
    ) -> InlineStyleAction {
        let prefix = content.content_from_line_start_to_selection_start(selection_offset);

        if let Some(cap) = INLINE_CODE_INLINE_REGEX.captures(prefix.as_str()) {
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();

            InlineStyleAction::Insert {
                matched_length,
                text: decorate_string.to_owned(),
                style: TextStyles::default().inline_code(),
                // We want to override the text style after the insertion to keep the old text style
                // instead of inheriting the newly inserted inline code since it has been closed.
                override_text_style: Some(self.active_text_style),
            }
        } else if let Some(cap) = ITALIC_INLINE_REGEX.captures(prefix.as_str()) {
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();

            // We need to look ahead to make sure the character in front of the matched regex is not "*".
            // This is to prevent **abc* to be interpreted as italic when user intends to type in bold inline markdown.
            //
            // Unfortunately Rust Regex does not support look ahead for performance reasons
            // https://github.com/rust-lang/regex/issues/127
            // As a workaround, we need to manually assert if the previous character is * or not.
            let should_apply_inline_style = if selection_offset.as_usize() > matched_length + 1 {
                content.char_at(selection_offset - matched_length - 1) != Some('*')
            } else {
                true
            };

            if should_apply_inline_style {
                InlineStyleAction::Insert {
                    matched_length,
                    text: decorate_string.to_owned(),
                    style: TextStyles::default().italic(),
                    override_text_style: Some(self.active_text_style),
                }
            } else {
                InlineStyleAction::None
            }
        } else if let Some(cap) = BOLD_INLINE_REGEX.captures(prefix.as_str()) {
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();

            // Unfortunately Rust Regex does not support look ahead for performance reasons
            // https://github.com/rust-lang/regex/issues/127
            // As a workaround, we need to manually assert if the previous character is * or not.
            let should_apply_inline_style = if selection_offset.as_usize() > matched_length + 1 {
                content.char_at(selection_offset - matched_length - 1) != Some('*')
            } else {
                true
            };

            if should_apply_inline_style {
                InlineStyleAction::Insert {
                    matched_length,
                    text: decorate_string.to_owned(),
                    style: TextStyles::default().bold(),
                    override_text_style: Some(self.active_text_style),
                }
            } else {
                InlineStyleAction::None
            }
        } else if let Some(cap) = BOLD_ITALIC_INLINE_REGEX.captures(prefix.as_str()) {
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();

            InlineStyleAction::Insert {
                matched_length,
                text: decorate_string.to_owned(),
                style: TextStyles::default().bold().italic(),
                override_text_style: Some(self.active_text_style),
            }
        } else if let Some(action) =
            self.underscore_inline_markdown_action(prefix.as_str(), content, selection_offset)
        {
            action
        } else if let Some(cap) = LINK_INLINE_REGEX.captures(prefix.as_str()) {
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();
            let link = cap.get(2).unwrap().as_str();

            InlineStyleAction::Link {
                matched_length,
                tag: decorate_string.to_string(),
                url: link.to_string(),
            }
        } else if let Some(cap) = STRIKETHROUGH_REGEX.captures(prefix.as_str()) {
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();

            InlineStyleAction::Insert {
                matched_length,
                text: decorate_string.to_string(),
                style: TextStyles::default().strikethrough(),
                override_text_style: Some(self.active_text_style),
            }
        } else {
            InlineStyleAction::None
        }
    }

    /// Compute the inline-markdown action for the underscore variants of
    /// italic (`_x_`), bold (`__x__`), and bold+italic (`___x___`). The
    /// variants share identical structure, so we dispatch them through a
    /// single table keyed by regex. Rules for `_`:
    /// - reject a match preceded by another `_` for italic/bold so that
    /// mid-sequence matches (e.g. `__abc_` on the way to `__abc__`) don't
    /// trigger prematurely,
    /// - always reject a match preceded by an alphanumeric character,
    /// mirroring CommonMark's left-flanking rules for `_` so intra-word
    /// underscores such as `foo_bar_` are not coerced.
    fn underscore_inline_markdown_action(
        &self,
        prefix: &str,
        content: &Buffer,
        selection_offset: CharOffset,
    ) -> Option<InlineStyleAction> {
        // Ordered shortest → longest. The stricter variants (italic, bold)
        // also reject a leading `_` to avoid firing while the user is still
        // typing a longer delimiter run.
        let candidates: [(&Regex, TextStyles, bool); 3] = [
            (
                &ITALIC_UNDERSCORE_INLINE_REGEX,
                TextStyles::default().italic(),
                true,
            ),
            (
                &BOLD_UNDERSCORE_INLINE_REGEX,
                TextStyles::default().bold(),
                true,
            ),
            (
                &BOLD_ITALIC_UNDERSCORE_INLINE_REGEX,
                TextStyles::default().bold().italic(),
                false,
            ),
        ];

        for (regex, style, reject_underscore_prefix) in candidates {
            let Some(cap) = regex.captures(prefix) else {
                continue;
            };
            let matched_length = cap.get(0).unwrap().as_str().chars().count();
            let decorate_string = cap.get(1).unwrap().as_str();

            // Only inspect the character immediately before the match when the
            // match starts far enough from the buffer's beginning — i.e. when
            // there actually is a character at that position to look at.
            let should_apply_inline_style = if selection_offset.as_usize() > matched_length + 1 {
                !matches!(
                    content.char_at(selection_offset - matched_length - 1),
                    Some(c) if (reject_underscore_prefix && c == '_') || c.is_alphanumeric()
                )
            } else {
                true
            };

            if should_apply_inline_style {
                return Some(InlineStyleAction::Insert {
                    matched_length,
                    text: decorate_string.to_owned(),
                    style,
                    override_text_style: Some(self.active_text_style),
                });
            } else {
                return Some(InlineStyleAction::None);
            }
        }

        None
    }

    fn block_markdown_shortcut<'a>(
        content: &mut Buffer,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &AppContext,
        text: &'a str,
    ) -> Option<BufferEditAction<'a>> {
        // We will only return an action if all selections can be converted to the desired block type.
        let mut consensus_block_type = None;
        for range in selection_model.as_ref(ctx).selections_to_offset_ranges() {
            let found_block_type = match (
                text,
                content
                    .content_from_block_start_to_selection_start(range.start)
                    .as_str(),
            ) {
                ("`", "```") => Some(BlockType::Text(BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                })),
                (" ", "# ") => Some(BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                })),
                (" ", "## ") => Some(BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header2,
                })),
                (" ", "### ") => Some(BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header3,
                })),
                (" ", "#### ") => Some(BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header4,
                })),
                (" ", "##### ") => Some(BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header5,
                })),
                (" ", "###### ") => Some(BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header6,
                })),
                (" ", "* " | "- ") => Some(BlockType::Text(BufferBlockStyle::UnorderedList {
                    indent_level: Self::list_indent_at_selection(
                        content,
                        selection_model.clone(),
                        ctx,
                    )
                    .unwrap_or(ListIndentLevel::One),
                })),
                (" ", text) => {
                    if HORIZONTAL_RULE_SHORTCUT_PREFIX.is_match(text)
                        && content.block_type_at_point(range.start)
                            == BlockType::Text(BufferBlockStyle::PlainText)
                    {
                        Some(BlockType::Item(BufferBlockItem::HorizontalRule))
                    } else if COMPLETE_TASKLIST_INLINE_REGEX.is_match(text) {
                        Some(BlockType::Text(BufferBlockStyle::TaskList {
                            indent_level: Self::list_indent_at_selection(
                                content,
                                selection_model.clone(),
                                ctx,
                            )
                            .unwrap_or(ListIndentLevel::One),
                            complete: true,
                        }))
                    } else if INCOMPLETE_TASKLIST_INLINE_REGEX.is_match(text) {
                        Some(BlockType::Text(BufferBlockStyle::TaskList {
                            indent_level: Self::list_indent_at_selection(
                                content,
                                selection_model.clone(),
                                ctx,
                            )
                            .unwrap_or(ListIndentLevel::One),
                            complete: false,
                        }))
                    } else if let Some(captures) = NUMBERED_LIST_SHORTCUT_PREFIX.captures(text) {
                        // Calculate the new ordered list item's indent level:
                        // * If the cursor is at another type of list, reuse the indent level (for
                        //   example, to convert fron an unordered list to an ordered list)
                        // * If the cursor is already in the middle of an ordered list, skip the
                        //   shortcut because it would have no effect (see the
                        //   `BufferBlockStyle::OrderedList` case for details).
                        // * Otherwise, start at level one.
                        let active_block_type = content
                            .active_block_type_at_first_selection(selection_model.as_ref(ctx));
                        let indent_level = match active_block_type {
                            BlockType::Text(BufferBlockStyle::UnorderedList { indent_level }) => {
                                Some(indent_level)
                            }
                            BlockType::Text(BufferBlockStyle::TaskList {
                                indent_level, ..
                            }) => Some(indent_level),
                            BlockType::Text(BufferBlockStyle::OrderedList {
                                indent_level, ..
                            }) => {
                                let block_start = content.block_or_line_start(
                                    selection_model.as_ref(ctx).first_selection_head(),
                                );

                                // If the previous block is an ordered list item that's equally- or
                                // more-indented than the ordered list item at the cursor, then we're
                                // in the middle of an ordered list and cannot renumber it. For
                                // example, given this list:
                                // 1. AAAA
                                // 2. BBBB
                                //    a) CCCC
                                //    b) DDDD
                                // 3. EEEE
                                // The shortcut is valid at AAAA and CCCC, but not BBBB, DDDD, or EEEE.
                                if block_start > CharOffset::from(1) {
                                    match content.block_type_at_point(block_start - 1) {
                                        BlockType::Text(BufferBlockStyle::OrderedList {
                                            indent_level: previous_indent_level,
                                            ..
                                        }) if previous_indent_level >= indent_level => None,
                                        _ => Some(indent_level),
                                    }
                                } else {
                                    // If this is the first block in the buffer, it cannot possibly
                                    // be in the middle of a list.
                                    Some(indent_level)
                                }
                            }
                            _ => Some(ListIndentLevel::One),
                        };

                        indent_level.map(|indent_level| {
                            let number = captures
                                .get(1)
                                .and_then(|number| number.as_str().parse::<usize>().ok());
                            BlockType::Text(BufferBlockStyle::OrderedList {
                                indent_level,
                                number,
                            })
                        })
                    } else {
                        None
                    }
                }
                _ => return None,
            };

            // If we didn't find a block type to convert to, or if we found a different block type, than
            // found at a previous selection, we won't convert anything.
            match (found_block_type, &consensus_block_type) {
                (None, _) => return None,
                (found, None) => consensus_block_type = found,
                (Some(found), Some(consensus)) if found != *consensus => return None,
                _ => (),
            }
        }
        consensus_block_type.map(BufferEditAction::RemovePrefixAndStyleBlocks)
    }

    fn apply_inline_style_helper(
        mut content: BufferUpdateWrapper,
        selection_model: ModelHandle<BufferSelectionModel>,
        matched_length: usize,
        action: BufferEditAction,
        ctx: &mut ModelContext<Buffer>,
    ) {
        let new_selections = selection_model
            .as_ref(ctx)
            .selections_to_offset_ranges()
            .mapped(|range| SelectionOffsets {
                tail: range
                    .start
                    .saturating_sub(&CharOffset::from(matched_length)),
                head: range.end,
            });

        content.buffer().update_selection(
            selection_model.clone(),
            BufferSelectAction::SetSelectionOffsets {
                selections: new_selections,
            },
            AutoScrollBehavior::Selection,
            ctx,
        );

        content.apply_edit(action, EditOrigin::UserTyped, selection_model, ctx);
    }

    /// Copy the current selection. If a code block is selected, copy its entire contents.
    pub fn copy(&self, ctx: &mut ModelContext<Self>) -> Option<BlockInfo> {
        let (clipboard, block) = match self.single_selected_command_range(ctx) {
            SelectedCommandResult::Single { start, end } => {
                let clipboard = self.command_clipboard_content(start, end, ctx);
                (clipboard, Some(BlockInfo::CodeBlock))
            }
            SelectedCommandResult::None if !self.selection_is_single_cursor(ctx) => {
                (self.read_selected_text_as_clipboard_content(ctx), None)
            }
            _ => return None,
        };

        ctx.clipboard().write(clipboard);
        block
    }

    /// Cut the current text or command selection.
    pub fn cut(&mut self, ctx: &mut ModelContext<Self>) -> Option<BlockInfo> {
        match self.single_selected_command_range(ctx) {
            SelectedCommandResult::Single { start, end } => {
                self.delete_selected_command_range(start, end, true, ctx);
                Some(BlockInfo::CodeBlock)
            }
            SelectedCommandResult::None if !self.selection_is_single_cursor(ctx) => {
                let clipboard = self.read_selected_text_as_clipboard_content(ctx);
                ctx.clipboard().write(clipboard);

                let selection_model = self.selection_model.clone();
                self.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                            BufferEditAction::Backspace,
                            EditOrigin::UserInitiated,
                            selection_model,
                            ctx,
                        )
                    },
                    ctx,
                );
                self.validate(ctx);
                None
            }
            _ => None,
        }
    }

    pub fn insert_formatted_from_paste(
        &mut self,
        formatted_text: FormattedText,
        plain_text: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        match self.single_selected_command_range(ctx) {
            SelectedCommandResult::Single { start, end } => {
                self.selection.update(ctx, |selection, ctx| {
                    // Add 1 to skip past the command's starting block marker.
                    selection.update_selection(
                        BufferSelectAction::SetSelectionOffsets {
                            selections: vec1![SelectionOffsets {
                                head: end,
                                tail: start + 1
                            }],
                        },
                        AutoScrollBehavior::Selection,
                        ctx,
                    )
                });
            }
            SelectedCommandResult::Multiple => return,
            SelectedCommandResult::None => (),
        }

        // If the pasted content is a URL, and there is only one selection that is a range (not just a cursor),
        // then we will create a link for the selected text, instead of pasting the text into the buffer.
        let single_range_selection = self.selection_is_single_range(ctx);
        let selection_model = self.selection_model.clone();

        let all_selections_allow_formatting = selection_model
            .as_ref(ctx)
            .all_selections_allow_formatting(ctx);

        self.update_content(
            |mut content, ctx| {
                if all_selections_allow_formatting {
                    // If we have an active selection and the pasted in text is a url, set the active selection to
                    // a link instead of replacing the content.
                    match is_valid_url(plain_text) {
                        Some(url) if single_range_selection => {
                            let tag = content
                                .buffer()
                                .selected_text_as_plain_text(selection_model.clone(), ctx)
                                .into_string();
                            content.apply_edit(
                                BufferEditAction::Link { tag, url },
                                EditOrigin::UserInitiated,
                                selection_model.clone(),
                                ctx,
                            )
                        }
                        _ => content.apply_edit(
                            BufferEditAction::InsertFormatted(formatted_text),
                            EditOrigin::UserInitiated,
                            selection_model.clone(),
                            ctx,
                        ),
                    }
                } else {
                    content.apply_edit(
                        BufferEditAction::Insert {
                            text: plain_text,
                            style: TextStyles::default(),
                            override_text_style: None,
                        },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    )
                }
            },
            ctx,
        );
        self.validate(ctx);
    }

    fn delete_selected_command_range(
        &mut self,
        start: CharOffset,
        end: CharOffset,
        cut: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if cut {
            let clipboard = self.command_clipboard_content(start, end, ctx);
            ctx.clipboard().write(clipboard);
        }

        let selection_model = self.buffer_selection_model().clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Delete(vec1![start..end]),
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
        self.validate(ctx);
    }

    pub fn selected_text(&self, ctx: &AppContext) -> String {
        self.content
            .as_ref(ctx)
            .selected_text_as_plain_text(self.selection_model.clone(), ctx)
            .into_string()
    }

    pub fn has_single_exact_rendered_mermaid_selection(&self, ctx: &AppContext) -> bool {
        if !self.selection_is_single_range(ctx) {
            return false;
        }

        self.selection_model
            .as_ref(ctx)
            .selections_to_offset_ranges()
            .into_iter()
            .exactly_one()
            .ok()
            .is_some_and(|range| {
                self.render_state
                    .as_ref(ctx)
                    .is_entire_range_of_type(&range, |item| {
                        matches!(item, BlockItem::MermaidDiagram { .. })
                    })
            })
    }

    /// Collapse all selections to cursors (set head == tail), effectively clearing
    /// the visual selection highlight without changing cursor positions.
    pub fn collapse_selections_to_cursors(&self, ctx: &mut ModelContext<Self>) {
        self.selection.update(ctx, |selection, ctx| {
            let first_selection = *selection.selections(ctx).first();
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets {
                    selections: vec1![SelectionOffsets {
                        head: first_selection.head,
                        tail: first_selection.head,
                    }],
                },
                AutoScrollBehavior::None,
                ctx,
            );
        });
    }

    fn is_rendered_mermaid_block(&self, block_start: CharOffset, ctx: &AppContext) -> bool {
        matches!(
            self.content
                .as_ref(ctx)
                .block_type_at_point(block_start + 1),
            BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Mermaid,
            })
        ) && self
            .child_models
            .model_at::<NotebookCommand>(block_start)
            .is_some_and(|command| {
                matches!(
                    command.as_ref(ctx).mermaid_display_mode,
                    MarkdownDisplayMode::Rendered
                )
            })
            && Self::editable_markdown_mermaid_enabled()
            && matches!(
                self.interaction_state(ctx),
                InteractionState::Selectable
                    | InteractionState::Editable
                    | InteractionState::EditableWithInvalidSelection
            )
    }

    fn rendered_mermaid_ranges(&self, ctx: &AppContext) -> Vec<Range<CharOffset>> {
        self.child_models
            .models
            .iter()
            .filter_map(|(start, command)| {
                let end = command.end_offset(ctx)?;
                self.is_rendered_mermaid_block(*start, ctx)
                    .then_some(*start..end)
            })
            .sorted_by_key(|range| range.start)
            .collect_vec()
    }

    pub fn link_at_selection_head(&self, ctx: &AppContext) -> Option<String> {
        self.content
            .as_ref(ctx)
            .link_url_at_selection_head(self.selection_model.clone(), ctx)
    }

    /// Update the rich text styling used to render the buffer. This will generally re-layout the
    /// entire buffer, unless the styles haven't changed.
    pub fn update_rich_text_styles(
        &self,
        new_styles: RichTextStyles,
        ctx: &mut ModelContext<Self>,
    ) {
        // Inline code colors and the underline color (derived from base_text.text_color) are
        // baked into the layout cache. Capture whether they changed before the styles are
        // replaced, so we can force a relayout for notebooks when only those colors differ.
        let notebook_colors_changed = {
            let current = self.render_state.as_ref(ctx).styles();
            current.inline_code_style != new_styles.inline_code_style
                || current.base_text.text_color != new_styles.base_text.text_color
        };

        let style_update = self.render_state.update(ctx, |render_state, _| {
            render_state.update_styles(new_styles)
        });
        // TODO: We can skip work here based on what's changed. For example, if the code text style
        // changes, we only need to re-layout code blocks.
        match style_update {
            StyleUpdateAction::Relayout => self.rebuild_layout(ctx),
            StyleUpdateAction::Repaint if notebook_colors_changed => self.rebuild_layout(ctx),
            StyleUpdateAction::Repaint => (),
            StyleUpdateAction::None => return,
        };

        // Re-apply cached highlighting for models when there is a theme update.
        for model in self.child_models.model_handles::<NotebookEmbed>() {
            model.update(ctx, |model, ctx| model.try_apply_cached_highlighting(ctx));
        }

        for model in self.child_models.model_handles::<NotebookCommand>() {
            model.update(ctx, |model, ctx| model.try_apply_cached_highlighting(ctx));
        }
    }

    pub fn link_url_at(&self, offset: CharOffset, app: &AppContext) -> Option<String> {
        self.content.as_ref(app).link_url_at_offset(offset)
    }

    pub fn scroll_to_matching_header(
        &mut self,
        fragment: &str,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(range) = self.find_matching_header(fragment, ctx) else {
            return false;
        };

        self.render_state.update(ctx, |render_state, _| {
            render_state
                .request_autoscroll_to(AutoScrollMode::PositionOffsetInViewportCenter(range.start));
        });
        true
    }

    fn find_matching_header(&self, fragment: &str, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let target = fragment.strip_prefix('#')?;
        if target.is_empty() {
            return None;
        }
        let target = urlencoding::decode(target).ok()?;
        let target = target.trim().to_lowercase();
        if target.is_empty() {
            return None;
        }

        let content = self.content.as_ref(ctx);
        for outline in content.outline_blocks() {
            if !matches!(
                &outline.block_type,
                BlockType::Text(BufferBlockStyle::Header { .. })
            ) {
                continue;
            }

            let heading = content
                .text_in_range(outline.start + 1..outline.end)
                .into_string();
            if heading.trim().to_lowercase() == target {
                return Some(outline.start..outline.end);
            }
        }

        None
    }

    /// Whether or not there's an active command block selection.
    pub fn has_command_selection(&self, ctx: &AppContext) -> bool {
        self.child_models
            .models
            .values()
            .any(|model| model.selected(ctx))
    }

    /// Selects the command block at the given start offset. If the offset does not point to a
    /// command block, this has no effect.
    pub fn select_command_at(&mut self, block_start: CharOffset, ctx: &mut ModelContext<Self>) {
        let child_model = match self.child_models.models.get(&block_start) {
            Some(handle) => handle.clone_boxed(),
            None => return,
        };

        // If the selection is on a valid block, we clear selections first so that if there
        // were any other selected commands, the end result is that _just_ the given command
        // is selected.
        let had_command_selection = self.clear_command_selections(ctx);

        self.cursor_at(block_start, ctx);

        if !child_model.selectable(ctx) {
            return;
        }

        child_model.set_selected(true, ctx);

        if let Some(command) = child_model.executable_command(ctx) {
            ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                format!("Selected workflow: {command}"),
                WarpA11yRole::TextareaRole,
            ));
        }

        match child_model.end_offset(ctx) {
            Some(end_offset) => {
                let range = block_start..end_offset;
                self.render_state.update(ctx, |render_state, _| {
                    render_state
                        .request_autoscroll_to(AutoScrollMode::ScrollOffsetsIntoViewport(range));
                });
            }
            None => {
                log::error!("Child model at {block_start} has end offset with value None");
            }
        };

        self.interaction_state
            .update(ctx, |interaction_state, ctx| {
                interaction_state.set_is_block_selected(true, ctx);
            });

        if !had_command_selection {
            ctx.emit(RichTextEditorModelEvent::SwitchedSelectionMode {
                new_mode: TelemetrySelectionMode::Command,
            });
        };

        ctx.notify();
    }

    /// Selects the command block containing the text cursor. If the cursor is not inside a command
    /// block, this has no effect.
    ///
    /// Unlike `select_command_up`, the text cursor must be _within_ the command block. This will
    /// not seek up to the previous command block if the cursor is within another type of block.
    pub fn select_command_at_cursor(&mut self, ctx: &mut ModelContext<Self>) {
        let cursor = self.selection_model.as_ref(ctx).first_selection_head();
        let content = self.content.as_ref(ctx);
        // Subtract 1 to get to the start marker for the block.
        self.select_command_at(content.block_or_line_start(cursor) - 1, ctx);
    }

    /// Selects the next command block up from the current selection. This is either:
    /// * The command block before the first selected command in the buffer.
    /// * The nearest command block above the text cursor.
    pub fn select_command_up(&mut self, ctx: &mut ModelContext<Self>) {
        let selected_command_start = self.selected_commands(ctx).map(|(start, _)| start).min();

        let selectable_offsets = self.child_models.selectable_offsets(ctx);

        if selectable_offsets.is_empty() {
            return;
        }

        // Calculates the char offset of the previous selectable block. We first get the index of the current
        // selection offset (if we have a block selection, use the starting offset of the block instead) in the
        // selectable offsets. If the index is 0, this is a no-op. Otherwise, select the block with index - 1.
        let has_command_selection = selected_command_start.is_some();
        let current_offset = selected_command_start
            .unwrap_or_else(|| self.selection_model.as_ref(ctx).first_selection_head());

        let previous_command_offset = match selectable_offsets.binary_search(&current_offset) {
            Ok(0) if !has_command_selection => None,
            Ok(0) => selectable_offsets.first(),
            Err(0) => None,
            Ok(num) | Err(num) => selectable_offsets.get(num - 1),
        };

        if let Some(offset) = previous_command_offset {
            self.select_command_at(*offset, ctx)
        }
    }

    /// Selects the next command block down from the current selection. This is either:
    /// * The command block after the last selected command in the buffer.
    /// * The nearest command block below the text cursor.
    pub fn select_command_down(&mut self, ctx: &mut ModelContext<Self>) {
        let selected_command_start = self.selected_commands(ctx).map(|(start, _)| start).max();

        let selectable_offsets = self.child_models.selectable_offsets(ctx);

        if selectable_offsets.is_empty() {
            return;
        }

        // Calculates the char offset of the next selectable block. We first get the index of the current
        // selection offset (if we have a block selection, use the starting offset of the block instead) in the
        // selectable offsets. If the index is at last index, this is a no-op. Otherwise, select the block with index + 1.
        let has_command_selection = selected_command_start.is_some();
        let current_offset = selected_command_start
            .unwrap_or_else(|| self.selection_model.as_ref(ctx).first_selection_head());

        // Should be safe since we checked selectable_offset is not empty above.
        let last_idx = selectable_offsets.len() - 1;
        let next_command_offset = match selectable_offsets.binary_search(&current_offset) {
            // If we are already at the end of the selectable block or if we don't have a command selection currently,
            // select the block matching the index.
            Ok(num) if num == last_idx || !has_command_selection => selectable_offsets.get(num),
            Ok(num) => selectable_offsets.get(num + 1),
            Err(num) => selectable_offsets.get(num),
        };

        if let Some(offset) = next_command_offset {
            self.select_command_at(*offset, ctx)
        }
    }

    /// Switch from command block selection to text selection. This de-selects all commands in the
    /// buffer and moves the text cursor to the end of the first selected block.
    pub fn exit_command_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let new_cursor_location = self
            .selected_commands(ctx)
            .min_by_key(|(start, _)| *start)
            .and_then(|(_, command)| command.end_offset(ctx));

        if self.clear_command_selections(ctx) {
            ctx.emit(RichTextEditorModelEvent::SwitchedSelectionMode {
                new_mode: TelemetrySelectionMode::Text,
            });
        }

        if let Some(cursor_location) = new_cursor_location {
            self.cursor_at(cursor_location, ctx);
        }
        ctx.notify();
    }

    /// Marks all command blocks as not selected and resets the last active select block state.
    ///
    /// Returns whether or not any commands were previously selected.
    pub fn clear_command_selections(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        let mut had_command_selection = false;
        for command in self.child_models.models.values() {
            if command.selectable(ctx) {
                had_command_selection |= command.set_selected(false, ctx)
            }
        }
        self.interaction_state
            .update(ctx, |interaction_state, ctx| {
                interaction_state.set_is_block_selected(false, ctx);
            });

        had_command_selection
    }

    /// Returns true if any of the subviews store on any of the NotebookCommand models are focused
    /// (such as the dropdown view for picking different block types)
    pub fn any_notebook_command_submodel_focused(&self, ctx: &AppContext) -> bool {
        self.child_models
            .models::<NotebookCommand>(ctx)
            .any(|command| command.is_dropdown_focused(ctx))
    }

    /// The currently-selected command blocks and their starting offsets.
    fn selected_commands<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = (CharOffset, &'a Box<dyn ChildModelHandle>)> + 'a {
        self.child_models
            .models
            .values()
            .filter_map(|handle| match handle.start_offset(ctx) {
                Some(offset) if handle.selected(ctx) => Some((offset, handle)),
                _ => None,
            })
    }

    /// The currently-selected command block, as a [`NotebookWorkflow`]
    pub fn selected_command_workflow(&self, app: &AppContext) -> Option<NotebookWorkflow> {
        // TODO(ben): Support executing multiple commands. It's TBD how we'll queue them up.
        self.selected_commands(app)
            .exactly_one()
            .ok()
            .and_then(|(_, command)| command.executable_workflow(app))
    }

    /// The range of the currently-selected command.
    fn single_selected_command_range(&self, ctx: &AppContext) -> SelectedCommandResult {
        match self.selected_commands(ctx).at_most_one() {
            Ok(Some((start, command))) => match command.end_offset(ctx) {
                Some(end) => SelectedCommandResult::Single { start, end },
                None => SelectedCommandResult::None,
            },
            Ok(None) => SelectedCommandResult::None,
            Err(_) => SelectedCommandResult::Multiple,
        }
    }

    fn maybe_rendered_mermaid_deletion_ranges(
        &self,
        direction: TextDirection,
        unit: TextUnit,
        ctx: &mut ModelContext<Self>,
    ) -> Option<Vec1<Range<CharOffset>>> {
        let mermaid_ranges = self.rendered_mermaid_ranges(ctx);
        if mermaid_ranges.is_empty() {
            return None;
        }
        let all_single_cursors = self.selection_model.as_ref(ctx).all_single_cursors();
        if !all_single_cursors {
            let ranges = self
                .selection_model
                .as_ref(ctx)
                .selections_to_offset_ranges();
            return (ranges.len() == 1
                && mermaid_ranges.iter().any(|range| range == ranges.first()))
            .then_some(ranges);
        }

        let ranges = self.replacement_range_for_deletion(direction, unit.clone(), ctx)?;

        let allow_adjacent_boundary = matches!(unit, TextUnit::Character);
        let mut changed = false;
        let ranges = ranges.mapped_ref(|range| {
            let expanded = mermaid_ranges
                .iter()
                .fold(range.clone(), |current, mermaid_range| {
                    let intersects =
                        current.start < mermaid_range.end && mermaid_range.start < current.end;
                    let is_adjacent_boundary = allow_adjacent_boundary
                        && ((matches!(direction, TextDirection::Backwards)
                            && current.start == mermaid_range.end)
                            || (matches!(direction, TextDirection::Forwards)
                                && current.end == mermaid_range.start));
                    if intersects {
                        current.start.min(mermaid_range.start)..current.end.max(mermaid_range.end)
                    } else if is_adjacent_boundary {
                        mermaid_range.clone()
                    } else {
                        current
                    }
                });
            if expanded != *range {
                changed = true;
            }
            expanded
        });

        changed.then_some(ranges)
    }

    fn clipboard_content_for_ranges(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        ctx: &AppContext,
    ) -> ClipboardContent {
        if ranges.len() == 1 {
            let range = ranges.first();
            if self
                .render_state
                .as_ref(ctx)
                .is_entire_range_of_type(range, |item| {
                    matches!(item, BlockItem::MermaidDiagram { .. })
                })
            {
                return self.command_clipboard_content(range.start, range.end, ctx);
            }
        }

        let buffer = self.content.as_ref(ctx);
        ClipboardContent {
            plain_text: buffer.text_in_ranges_with_expanded_embedded_items(ranges.clone(), ctx),
            html: buffer.ranges_as_html(ranges, ctx),
            ..Default::default()
        }
    }

    fn delete_ranges(
        &mut self,
        ranges: Vec1<Range<CharOffset>>,
        cut: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if cut {
            let clipboard = self.clipboard_content_for_ranges(ranges.clone(), ctx);
            ctx.clipboard().write(clipboard);
        }

        let selection_model = self.buffer_selection_model().clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Delete(ranges),
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
        self.validate(ctx);
    }

    fn command_clipboard_content(
        &self,
        start: CharOffset,
        end: CharOffset,
        ctx: &AppContext,
    ) -> ClipboardContent {
        let mut clipboard = self.read_text_as_clipboard_content(start + 1..end, ctx);
        let Some(source) = self.mermaid_block_source(start, ctx) else {
            return clipboard;
        };
        if Self::editable_markdown_mermaid_enabled() {
            clipboard.plain_text = format!("```mermaid\n{source}\n```");
        }
        let Some(image_html) = render_mermaid_clipboard_html(&source) else {
            return clipboard;
        };
        clipboard.html = Some(match clipboard.html {
            Some(existing_html) => format!("{existing_html}{image_html}"),
            None => image_html,
        });
        clipboard
    }

    fn mermaid_block_source(&self, block_start: CharOffset, ctx: &AppContext) -> Option<String> {
        if !matches!(
            self.content
                .as_ref(ctx)
                .block_type_at_point(block_start + 1),
            BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Mermaid,
            })
        ) {
            return None;
        }
        let end = self
            .child_models
            .models
            .get(&block_start)?
            .end_offset(ctx)?;
        Some(
            self.content
                .as_ref(ctx)
                .text_in_range(block_start + 1..end)
                .into_string(),
        )
    }

    /// Returns the font size at the current cursor location.
    /// We use this to report cursor information to the OS.
    pub fn cursor_font_size<C: ModelAsRef>(&self, ctx: &C) -> f32 {
        let styles = self.render_state.as_ref(ctx).styles();
        match &self.active_block_type {
            BlockType::Item(_) => styles.base_text.font_size,
            BlockType::Text(block_style) => styles.paragraph_styles(block_style).font_size,
        }
    }

    /// Accessibility content for toggling an inline style.
    pub fn style_toggle_a11y(&self, style: BufferTextStyle) -> ActionAccessibilityContent {
        let action = if self.is_style_active(style) {
            "off"
        } else {
            "on"
        };
        let text = format!("{style:?} {action}");
        ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
            text,
            WarpA11yRole::UserAction,
        ))
    }

    /// Returns the previous character before the character at current cursor location,
    /// in any text block other than code blocks.
    pub fn prev_char_in_non_code_block(&self, ctx: &AppContext) -> Option<char> {
        if let BlockType::Text(BufferBlockStyle::CodeBlock { .. }) = self.active_block_type {
            return None;
        } else if self
            .selection_model
            .as_ref(ctx)
            .first_selection_is_single_cursor()
        {
            return self.content.read(ctx, |content, _| {
                let first_selection_head = self.selection_model.as_ref(ctx).first_selection_head();
                match content
                    .text_in_range(first_selection_head - CharOffset::from(1)..first_selection_head)
                    .as_str()
                    .chars()
                    .next()
                {
                    Some(c) => Some(c),
                    None => Some('\n'),
                }
            });
        }
        None
    }

    /// Apply a vector of diffs on the current buffer by working with raw markdown content.
    /// This gets the current markdown, applies the diffs, and then resets the editor with the new markdown.
    pub fn apply_diffs(
        &mut self,
        diffs: Vec<ai::diff_validation::DiffDelta>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Get current markdown content
        let current_markdown = self.markdown(ctx);

        let lines: Vec<&str> = current_markdown.lines().collect();
        let mut result_lines = Vec::new();
        let mut current_line = 0;

        for delta in &diffs {
            // Convert 1-indexed line ranges to 0-indexed
            let start_line = delta.replacement_line_range.start.saturating_sub(1);
            let end_line = delta.replacement_line_range.end.saturating_sub(1);

            // Add lines before this delta
            while current_line < start_line {
                if current_line < lines.len() {
                    result_lines.push(lines[current_line]);
                }
                current_line += 1;
            }

            // Skip the lines being replaced
            current_line = end_line;

            // Add the insertion content
            for line in delta.insertion.lines() {
                result_lines.push(line);
            }
        }

        // Add any remaining lines
        while current_line < lines.len() {
            result_lines.push(lines[current_line]);
            current_line += 1;
        }

        let new_markdown = result_lines.join("\n");

        // Reset the editor with the new markdown content
        self.reset_with_markdown(&new_markdown, ctx);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RichTextEditorModelEvent {
    /// The active styles at the cursor/selection have changed.
    ActiveStylesChanged {
        /// The text styles active at the cursor position.
        cursor_text_styles: TextStylesWithMetadata,
        /// The text styles active over the whole selection. This will differ
        /// from `cursor_text_styles` if the selected range is not all formatted
        /// the same way. A given style is only active if it applies to the entire
        /// selection.
        selection_text_styles: TextStylesWithMetadata,
        block_type: BlockType,
    },
    ContentChanged(EditOrigin),
    /// The user switched selection modes.
    SwitchedSelectionMode {
        new_mode: TelemetrySelectionMode,
    },
}

impl Entity for NotebooksEditorModel {
    type Event = RichTextEditorModelEvent;
}

impl CoreEditorModel for NotebooksEditorModel {
    type T = NotebooksEditorModel;

    fn content(&self) -> &ModelHandle<Buffer> {
        &self.content
    }

    fn buffer_selection_model(&self) -> &ModelHandle<BufferSelectionModel> {
        &self.selection_model
    }

    fn selection_model(&self) -> &ModelHandle<SelectionModel> {
        &self.selection
    }

    fn render_state(&self) -> &ModelHandle<RenderState> {
        &self.render_state
    }

    fn backspace(&mut self, ctx: &mut ModelContext<Self::T>) {
        match self.single_selected_command_range(ctx) {
            SelectedCommandResult::Single { start, end } => {
                self.delete_selected_command_range(start, end, false, ctx);
                return;
            }
            // Do not allow editing if multiple commands are selected.
            SelectedCommandResult::Multiple => return,
            SelectedCommandResult::None => {
                if let Some(ranges) = self.maybe_rendered_mermaid_deletion_ranges(
                    TextDirection::Backwards,
                    TextUnit::Character,
                    ctx,
                ) {
                    self.delete_ranges(ranges, false, ctx);
                    return;
                }
            }
        }

        let selection_model = self.buffer_selection_model().clone();
        // Edit the internal content model.
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Backspace,
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                )
            },
            ctx,
        );

        self.validate(ctx);
    }

    /// Validate the model state. For performance, this only runs in local builds.
    fn validate(&self, _ctx: &impl ModelAsRef) {
        #[cfg(debug_assertions)]
        {
            self.selection_model.as_ref(_ctx).validate_buffer(_ctx);
            log::trace!("Validated content model");
        }
    }

    fn active_text_style(&self) -> TextStyles {
        self.active_text_style
    }

    fn insert(&mut self, text: &str, origin: EditOrigin, ctx: &mut ModelContext<Self::T>) {
        let selection_model = self.buffer_selection_model().clone();
        // Edit the internal content model.
        self.update_content(
            |mut content, ctx| {
                // Markdown triggers should only be active when the selection is a cursor in styleable
                // text. Check this _before_ inserting, because the insertion will always result in a
                // single cursor.
                let markdown_shortcuts_active = content
                    .buffer()
                    .can_format_at_cursor(selection_model.clone(), ctx);

                content.apply_edit(
                    BufferEditAction::Insert {
                        text,
                        style: self.active_text_style,
                        override_text_style: None,
                    },
                    origin,
                    selection_model.clone(),
                    ctx,
                );

                if markdown_shortcuts_active {
                    let triggered_markdown_shortcut = NotebooksEditorModel::block_markdown_shortcut(
                        content.buffer(),
                        selection_model.clone(),
                        ctx,
                        text,
                    );

                    if let Some(action) = triggered_markdown_shortcut {
                        content.apply_edit(action, EditOrigin::UserTyped, selection_model, ctx);
                    } else if matches!(text, "*" | "_" | ")" | "`" | "~") {
                        self.inline_markdown_shortcut(content, selection_model, ctx);
                    }
                }
            },
            ctx,
        );
        self.validate(ctx);
    }
}

impl RichTextEditorModel for NotebooksEditorModel {
    fn delete(
        &mut self,
        direction: TextDirection,
        unit: TextUnit,
        cut: bool,
        ctx: &mut ModelContext<Self::T>,
    ) {
        match self.single_selected_command_range(ctx) {
            SelectedCommandResult::Single { start, end } => {
                self.delete_selected_command_range(start, end, cut, ctx);
            }
            SelectedCommandResult::Multiple => (),
            SelectedCommandResult::None => {
                if let Some(ranges) =
                    self.maybe_rendered_mermaid_deletion_ranges(direction, unit.clone(), ctx)
                {
                    self.delete_ranges(ranges, cut, ctx);
                } else {
                    self.delete_internal(
                        direction,
                        unit,
                        cut,
                        move |buffer, selection_model, override_range, ctx| {
                            let buffer = buffer.as_ref(ctx);
                            let ranges = match override_range {
                                Some(range) => range,
                                None => selection_model.as_ref(ctx).selections_to_offset_ranges(),
                            };

                            let content = ClipboardContent {
                                plain_text: buffer.text_in_ranges_with_expanded_embedded_items(
                                    ranges.clone(),
                                    ctx,
                                ),
                                html: buffer.ranges_as_html(ranges.clone(), ctx),
                                ..Default::default()
                            };
                            ctx.clipboard().write(content);
                        },
                        ctx,
                    );
                }
            }
        }
    }
}

/// Result for checking the currently-selected command block.
#[derive(Debug, Clone, Copy)]
enum SelectedCommandResult {
    /// A single command block is selected.
    Single {
        /// The offset of the command block's marker.
        start: CharOffset,
        /// The offset of the end of the command block.
        end: CharOffset,
    },
    /// No command blocks are selected.
    None,
    /// More than one command block is selected.
    Multiple,
}

/// Text unit for by-word navigation with the current word boundary policy.
pub fn word_unit(ctx: &AppContext) -> TextUnit {
    TextUnit::Word(SemanticSelection::as_ref(ctx).word_boundary_policy())
}

/// Container for logical editor models that are nested within a notebooks editor model.
///
/// Sub-models include:
/// * [`NotebookCommand`] for command/code blocks
/// * (Soon) embedded workflows
pub struct ChildModels {
    /// Sub-models mapped by their starting offset as of the last cycle of text layout. This
    /// prevents flicker when updating the content model, as the [`Anchor`]s within the command
    /// models will update before the [`RenderState`] does.
    models: HashMap<CharOffset, Box<dyn ChildModelHandle>>,
}

/// Handle to a logical sub-model of the notebook editor.
pub trait ChildModelHandle {
    /// The starting offset for the block that this model manages (for example, the start of a code
    /// block).
    fn start_offset(&self, app: &AppContext) -> Option<CharOffset>;

    /// The end offset for the block that this model manages (for example, the end of a code
    /// block).
    fn end_offset(&self, app: &AppContext) -> Option<CharOffset>;

    /// Whether the model is selected.
    fn selected(&self, app: &AppContext) -> bool;

    /// Whether the represented child model is selectable.
    fn selectable(&self, app: &AppContext) -> bool;

    /// Returns the workflow representation of this child model (if applies).
    fn executable_workflow(&self, app: &AppContext) -> Option<NotebookWorkflow>;

    /// Returns the plain text of the underlying executable command for this child model (if
    /// applies).
    fn executable_command<'a>(&'a self, app: &'a AppContext) -> Option<Cow<'a, str>>;

    /// Update the selection state of the child model, returning its previous selection state.
    fn set_selected(&self, selected: bool, ctx: &mut AppContext) -> bool;

    /// Coerce to [`Any`] for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Clone this handle. Because this is a handle, not the model itself, it's expected to be cheap.
    fn clone_boxed(&self) -> Box<dyn ChildModelHandle>;
}

impl ChildModels {
    pub fn new() -> Self {
        Self {
            models: Default::default(),
        }
    }

    pub fn selectable_offsets(&self, app: &AppContext) -> Vec<CharOffset> {
        self.models
            .iter()
            .filter_map(|(offset, model)| {
                if model.selectable(app) {
                    Some(*offset)
                } else {
                    None
                }
            })
            .sorted()
            .collect()
    }

    /// Retrieve the child model of the given type backing the block at `offset`.
    ///
    /// Returns `None` if:
    /// * There is not a block of the expected type at `offset`.
    /// * The child model state is out of sync with the underlying buffer.
    pub fn model_at<T: 'static>(&self, offset: CharOffset) -> Option<ModelHandle<T>> {
        Some(
            self.models
                .get(&offset)?
                .as_any()
                .downcast_ref::<ModelHandle<T>>()?
                .clone(),
        )
    }

    /// Iterate over all child models of the given type.
    pub fn models<'a, T: Entity>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = &'a T> + 'a {
        self.model_handles().map(|handle| handle.as_ref(ctx))
    }

    /// Iterates over all child model handles of the given type.
    pub fn model_handles<T: Entity>(&self) -> impl Iterator<Item = ModelHandle<T>> + '_ {
        self.models
            .values()
            .filter_map(|handle| handle.as_any().downcast_ref())
            .cloned()
    }

    /// Update the sub-model state with [`NotebookCommand`] models for every runnable command
    /// in the buffer. This should be called after text layout completes, so that the offsets of
    /// each block line up between the render and content models.
    pub fn update<T: Entity>(
        &mut self,
        interaction_state: ModelHandle<InteractionStateModel>,
        content: ModelHandle<Buffer>,
        selection_model: ModelHandle<BufferSelectionModel>,
        rte_window_id: WindowId,
        default_mermaid_display_mode: MarkdownDisplayMode,
        ctx: &mut ModelContext<T>,
    ) {
        // Resolve each existing model to its current offsets in the buffer, filtering out models
        // whose anchors have been deleted in the process.
        let mut existing_models: HashMap<_, _> = self
            .models
            .drain()
            .filter_map(|(_, handle)| {
                let start = handle.start_offset(ctx)?;
                let end = handle.end_offset(ctx)?;
                Some(((start, end), handle))
            })
            .collect();

        // Ensure there's a model for every code block in the buffer.
        // - If it's an existing block, there will be an entry in `existing_models`
        // - If it's a new block, we'll create a new model for it
        // - If a block were unstyled, its anchors may still be valid, but it won't be in the new
        //   outline, so the existing model handle will be dropped at the end of the method.
        let mut to_add = vec![];
        let mut new_embedded_item = vec![];
        let mut reset_selection = vec![];

        for outline in content.as_ref(ctx).outline_blocks() {
            match outline.block_type {
                BlockType::Text(BufferBlockStyle::CodeBlock { .. }) => {
                    match existing_models.remove(&(outline.start, outline.end)) {
                        Some(existing_model)
                            if existing_model.as_any().is::<ModelHandle<NotebookCommand>>() =>
                        {
                            log::trace!(
                                "Reusing existing NotebookCommand model at {}..{}",
                                outline.start,
                                outline.end
                            );

                            if !existing_model.selectable(ctx) && existing_model.selected(ctx) {
                                reset_selection.push((outline.start, existing_model));
                            } else {
                                self.models.insert(outline.start, existing_model);
                            }
                        }
                        _ => to_add.push(outline),
                    }
                }
                BlockType::Item(BufferBlockItem::Embedded { item }) => {
                    match existing_models.remove(&(outline.start, outline.end)) {
                        Some(existing_model)
                            if existing_model.as_any().is::<ModelHandle<NotebookEmbed>>() =>
                        {
                            log::trace!("Reusing existing EmbeddedItem model at {}", outline.start);

                            if !existing_model.selectable(ctx) && existing_model.selected(ctx) {
                                reset_selection.push((outline.start, existing_model));
                            } else {
                                self.models.insert(outline.start, existing_model);
                            }
                        }
                        _ => new_embedded_item.push((item.hashed_id().to_string(), outline.start)),
                    }
                }
                _ => (),
            }
        }

        // We have to add new models in a separate pass, because creating anchors requires a
        // mutable borrow of `content`, while the `outline_blocks` iterator already immutably
        // borrows it.
        self.models
            .reserve(to_add.len() + new_embedded_item.len() + reset_selection.len());

        for (model_start, model) in reset_selection {
            model.set_selected(false, ctx);
            self.models.insert(model_start, model);
        }

        for outline in to_add {
            log::debug!(
                "Adding NotebookCommand model at {}..{}",
                outline.start,
                outline.end
            );
            let new_model = ctx.add_model(|ctx| {
                let mut command = NotebookCommand::new(
                    outline.start,
                    outline.end,
                    interaction_state.clone(),
                    content.clone(),
                    selection_model.clone(),
                    rte_window_id,
                    ctx,
                );
                command.mermaid_display_mode = default_mermaid_display_mode;
                command
            });

            self.models.insert(outline.start, Box::new(new_model));
        }

        for (hashed_id, start_offset) in new_embedded_item {
            log::debug!("Adding EmbeddedItem model at {start_offset}");
            let new_model: ModelHandle<_> = ctx.add_model(|ctx| {
                NotebookEmbed::new(
                    start_offset,
                    hashed_id,
                    content.clone(),
                    selection_model.clone(),
                    ctx,
                )
            });

            self.models.insert(start_offset, Box::new(new_model));
        }
    }
}

/// Check if a content string is fully a valid URL.
fn is_valid_url(content: &str) -> Option<String> {
    match Url::parse(content) {
        Ok(_) => Some(content.to_string()),
        Err(_) => None,
    }
}

fn notebook_tab_indentation(block_style: &BufferBlockStyle, shift: bool) -> IndentBehavior {
    if !shift {
        match block_style {
            BufferBlockStyle::UnorderedList {
                indent_level: ListIndentLevel::Three,
            }
            | BufferBlockStyle::OrderedList {
                indent_level: ListIndentLevel::Three,
                ..
            } => IndentBehavior::Ignore,
            BufferBlockStyle::OrderedList {
                indent_level,
                number,
            } => IndentBehavior::Restyle(BufferBlockStyle::OrderedList {
                indent_level: indent_level.shift_right(),
                number: *number,
            }),
            BufferBlockStyle::UnorderedList { indent_level } => {
                IndentBehavior::Restyle(BufferBlockStyle::UnorderedList {
                    indent_level: indent_level.shift_right(),
                })
            }
            BufferBlockStyle::TaskList {
                indent_level,
                complete,
            } => IndentBehavior::Restyle(BufferBlockStyle::TaskList {
                indent_level: indent_level.shift_right(),
                complete: *complete,
            }),
            BufferBlockStyle::CodeBlock { .. } => IndentBehavior::TabIndent(IndentUnit::Space(4)),
            _ => IndentBehavior::Ignore,
        }
    } else {
        match block_style {
            BufferBlockStyle::UnorderedList {
                indent_level: ListIndentLevel::One,
            }
            | BufferBlockStyle::OrderedList {
                indent_level: ListIndentLevel::One,
                ..
            } => IndentBehavior::Ignore,
            BufferBlockStyle::OrderedList {
                indent_level,
                number,
            } => IndentBehavior::Restyle(BufferBlockStyle::OrderedList {
                indent_level: indent_level.shift_left(),
                number: *number,
            }),
            BufferBlockStyle::UnorderedList { indent_level } => {
                IndentBehavior::Restyle(BufferBlockStyle::UnorderedList {
                    indent_level: indent_level.shift_left(),
                })
            }
            BufferBlockStyle::TaskList {
                indent_level,
                complete,
            } => IndentBehavior::Restyle(BufferBlockStyle::TaskList {
                indent_level: indent_level.shift_left(),
                complete: *complete,
            }),
            BufferBlockStyle::CodeBlock { .. } => IndentBehavior::TabIndent(IndentUnit::Space(4)),
            _ => IndentBehavior::Ignore,
        }
    }
}

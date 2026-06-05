use std::cell::RefCell;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::RenderState;
use warpui::elements::{
    Border, ChildView, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    MainAxisAlignment, MainAxisSize, ParentElement, Radius, Shrinkable, Text,
};
use warpui::keymap::Keystroke;
use warpui::text_layout::ClipConfig;
use warpui::units::Pixels;
use warpui::{
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::code::editor::comments::{EditorCommentsModel, PendingCommentEvent};
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::{CommentId, CommentOrigin};
use crate::editor::InteractionState;
use crate::features::FeatureFlag;
use crate::notebooks::editor::model::NotebooksEditorModel;
use crate::notebooks::editor::rich_text_styles;
use crate::notebooks::editor::view::{EditorViewEvent, RichTextEditorConfig, RichTextEditorView};
use crate::notebooks::link::{NotebookLinks, SessionSource};
use crate::settings::FontSettings;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, DangerNakedTheme, KeystrokeSource, NakedTheme, PrimaryTheme,
};

/// Default width of the comment editor, in pixels.
pub(crate) const DEFAULT_COMMENT_MAX_WIDTH: f32 = 750.0;

/// Maximum height of the comment editor, in pixels. Past this height the editor scrolls its content
/// internally instead of growing further.
pub(crate) const MAX_COMMENT_HEIGHT: f32 = 200.0;

// ── Comment-shell layout constants ───────────────────────────────────────────
// These values mirror the padding/border used in `render_inline_comment_shell`.
// Keeping them as named constants means `COMMENT_CHROME_HEIGHT` stays in sync
// with the shell's visual layout automatically.

/// Top padding of the body area inside the comment shell.
const COMMENT_BODY_TOP_PADDING: f32 = 8.0;
/// Bottom padding of the body area inside the comment shell.
const COMMENT_BODY_BOTTOM_PADDING: f32 = 4.0;
/// Vertical padding of the footer row (top + bottom).
const COMMENT_FOOTER_VERTICAL_PADDING: f32 = 4.0 * 2.0;
/// Top border separating body from footer.
const COMMENT_FOOTER_BORDER: f32 = 1.0;
/// Outer container border (top + bottom).
const COMMENT_OUTER_BORDER: f32 = 1.0 * 2.0;
/// Height of the footer button row (`ButtonSize::Small` at default appearance).
const COMMENT_BUTTON_ROW_HEIGHT: f32 = 24.0;

/// Fixed vertical chrome around the inner comment editor.
/// Slightly generous so the reserved inline block is never shorter than the painted shell.
pub(crate) const COMMENT_CHROME_HEIGHT: f32 = COMMENT_BODY_TOP_PADDING
    + COMMENT_BODY_BOTTOM_PADDING
    + COMMENT_FOOTER_VERTICAL_PADDING
    + COMMENT_FOOTER_BORDER
    + COMMENT_BUTTON_ROW_HEIGHT
    + COMMENT_OUTER_BORDER
    + 1.0; // extra pixel of tolerance

pub(crate) fn inline_comment_background(appearance: &Appearance) -> ColorU {
    blended_colors::neutral_2(appearance.theme())
}

pub(crate) fn inline_comment_border_color(appearance: &Appearance) -> ColorU {
    blended_colors::neutral_4(appearance.theme())
}

pub(crate) fn render_inline_comment_shell(
    body: Box<dyn Element>,
    footer_row: Box<dyn Element>,
    max_height: Option<f32>,
    footer_horizontal_padding: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let background = inline_comment_background(appearance);
    let border_color = inline_comment_border_color(appearance);

    let body = Container::new(Clipped::new(body).finish())
        .with_padding_bottom(COMMENT_BODY_BOTTOM_PADDING)
        .with_padding_top(COMMENT_BODY_TOP_PADDING)
        .with_horizontal_padding(12.)
        .finish();
    let body = if max_height.is_some() {
        Shrinkable::new(1., body).finish()
    } else {
        body
    };

    let content = Flex::column()
        .with_child(body)
        .with_child(
            Container::new(footer_row)
                .with_vertical_padding(COMMENT_FOOTER_VERTICAL_PADDING / 2.0)
                .with_horizontal_padding(footer_horizontal_padding)
                .with_border(Border::top(COMMENT_FOOTER_BORDER).with_border_fill(border_color))
                .finish(),
        )
        .finish();

    let mut constrained = ConstrainedBox::new(content).with_max_width(DEFAULT_COMMENT_MAX_WIDTH);
    if let Some(max_height) = max_height {
        constrained = constrained.with_max_height(max_height);
    }

    Container::new(constrained.finish())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_background_color(background)
        .with_border(Border::all(COMMENT_OUTER_BORDER / 2.0).with_border_fill(border_color))
        .finish()
}

#[derive(Debug)]
pub enum CommentEditorEvent {
    ContentChanged,
    CommentSaved {
        id: Option<CommentId>,
        comment_text: String,
        #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
        line: Option<EditorLineLocation>,
    },
    CloseEditor,
    DeleteComment {
        id: CommentId,
    },
}

#[derive(Debug)]
pub enum CommentEditorAction {
    SaveComment,
    CloseEditor,
    RemoveComment,
}

pub struct CommentEditor {
    /// Comment ID if editing an existing comment, None for new comments.
    comment_id: Option<CommentId>,
    editor: ViewHandle<RichTextEditorView>,
    save_button: ViewHandle<ActionButton>,
    close_button: ViewHandle<ActionButton>,
    remove_button: ViewHandle<ActionButton>,
    line: Option<EditorLineLocation>,
    show_remove_button: bool,
    save_button_disabled: bool,
    laid_out_size: RefCell<Option<Vector2F>>,
    is_imported_comment: bool,
}

impl CommentEditor {
    pub fn new(
        ctx: &mut ViewContext<Self>,
        comment_model: ModelHandle<EditorCommentsModel>,
    ) -> Self {
        let editor = create_editable_comment_markdown_editor(None, ctx);

        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&comment_model, |me, _, event, ctx| {
            me.handle_comment_model_event(event, ctx);
        });

        let (save_button, close_button, remove_button) = Self::create_buttons(ctx);

        let mut me = Self {
            comment_id: None,
            editor,
            save_button,
            close_button,
            remove_button,
            line: None,
            show_remove_button: false,
            save_button_disabled: true,
            laid_out_size: RefCell::new(None),
            is_imported_comment: false,
        };
        me.update_save_button_state(ctx);
        me
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused))]
    pub fn comment_text(&self, app: &AppContext) -> String {
        self.editor.as_ref(app).model().as_ref(app).markdown(app)
    }

    /// The render state backing the inner markdown editor. Observing it lets a host re-measure the
    /// composer's reserved inline height when its content (and therefore laid-out height) changes.
    pub fn inner_render_state(&self, app: &AppContext) -> ModelHandle<RenderState> {
        self.editor
            .as_ref(app)
            .model()
            .as_ref(app)
            .render_state()
            .clone()
    }

    /// The height, in pixels, that this composer needs to render inline at its line: the inner
    /// editor's laid-out content height plus fixed chrome, capped at [`MAX_COMMENT_HEIGHT`]. The
    /// content height is current as soon as the inner render state finishes laying out, so a host
    /// observing [`Self::inner_render_state`] can keep the reserved block height in sync as the
    /// draft grows or shrinks.
    #[allow(unused)]
    pub fn inline_height(&self, app: &AppContext) -> Pixels {
        let content_height = self.inner_render_state(app).as_ref(app).height().as_f32();
        Pixels::new((content_height + COMMENT_CHROME_HEIGHT).min(MAX_COMMENT_HEIGHT))
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused))]
    pub fn get_laid_out_size(&self) -> Option<Vector2F> {
        self.laid_out_size.borrow().as_ref().cloned()
    }

    /// Whether the primary ("Comment"/"Update") button is currently disabled (true while the draft
    /// body is empty). Test-only accessor.
    #[cfg(feature = "integration_tests")]
    pub fn save_button_disabled_for_test(&self) -> bool {
        self.save_button_disabled
    }

    /// The current label of the primary button ("Comment" for a new comment, "Update" when editing
    /// an existing one). Test-only accessor.
    #[cfg(feature = "integration_tests")]
    pub fn primary_button_label_for_test(&self, app: &AppContext) -> String {
        self.save_button.as_ref(app).label().to_string()
    }

    /// Whether the "Remove" button is shown (true when editing an existing comment). Test-only.
    #[cfg(feature = "integration_tests")]
    pub fn show_remove_button_for_test(&self) -> bool {
        self.show_remove_button
    }

    /// Whether the composer is editing a comment imported from GitHub (shows the GitHub indicator).
    /// Test-only accessor.
    #[cfg(feature = "integration_tests")]
    pub fn is_imported_for_test(&self) -> bool {
        self.is_imported_comment
    }

    /// Whether the inner text editor (where typing lands) currently holds focus. Test-only.
    #[cfg(feature = "integration_tests")]
    pub fn inner_editor_focused_for_test(&self, app: &AppContext) -> bool {
        self.editor.is_focused(app)
    }

    /// Focus the inner text editor, mirroring what opening the composer does. Test-only.
    #[cfg(feature = "integration_tests")]
    pub fn focus_inner_editor_for_test(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.editor);
    }

    /// Invoke the same path Cmd/Ctrl+Enter triggers (save the comment). Test-only drive helper.
    #[cfg(feature = "integration_tests")]
    pub fn cmd_enter_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.save_comment(ctx);
    }

    /// Invoke the same path the Escape key triggers (close only when the draft is empty). Test-only.
    #[cfg(feature = "integration_tests")]
    pub fn escape_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.handle_escape(ctx);
    }

    /// Append `text` to the draft body through the inner markdown editor, mirroring what typing
    /// into the focused composer produces (updating the save-button state and notifying the host so
    /// the inline block re-measures). Test-only drive helper.
    #[cfg(feature = "integration_tests")]
    pub fn type_text_for_test(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let mut markdown = self.editor.as_ref(ctx).model().as_ref(ctx).markdown(ctx);
        markdown.push_str(text);
        self.editor.update(ctx, |editor, ctx| {
            editor.model().update(ctx, |model, ctx| {
                model.reset_with_markdown(&markdown, ctx);
            });
        });
        self.update_save_button_state(ctx);
        ctx.emit(CommentEditorEvent::ContentChanged);
        ctx.notify();
    }

    /// Replace the entire draft body with `text` (mirrors selecting all and retyping / deleting
    /// lines), updating the save-button state and notifying the host so the inline block
    /// re-measures. Test-only drive helper.
    #[cfg(feature = "integration_tests")]
    pub fn set_body_for_test(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.model().update(ctx, |model, ctx| {
                model.reset_with_markdown(text, ctx);
            });
        });
        self.update_save_button_state(ctx);
        ctx.emit(CommentEditorEvent::ContentChanged);
        ctx.notify();
    }

    /// The inner markdown editor's full laid-out content height (independent of the composer's
    /// max-height cap). When this exceeds the composer's visible height the composer is internally
    /// scrollable. Test-only accessor.
    #[cfg(feature = "integration_tests")]
    pub fn inner_content_height_for_test(&self, app: &AppContext) -> f32 {
        self.inner_render_state(app).as_ref(app).height().as_f32()
    }

    /// Whether the composer's reserved inline height is pinned at the [`MAX_COMMENT_HEIGHT`] cap
    /// (so further content scrolls internally rather than growing the block). Test-only accessor.
    #[cfg(feature = "integration_tests")]
    pub fn is_at_max_height_for_test(&self, app: &AppContext) -> bool {
        self.inline_height(app).as_f32() >= MAX_COMMENT_HEIGHT - 0.5
    }

    pub fn set_laid_out_size(&self, value: Vector2F) {
        self.laid_out_size.replace(Some(value));
    }

    fn create_buttons(
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<ActionButton>,
        ViewHandle<ActionButton>,
        ViewHandle<ActionButton>,
    ) {
        let save_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("Comment", PrimaryTheme)
                .with_keybinding(
                    KeystrokeSource::Fixed(Keystroke::parse("cmdorctrl-enter").unwrap_or_default()),
                    ctx,
                )
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CommentEditorAction::SaveComment);
                })
                .with_size(ButtonSize::Small)
        });

        save_button.update(ctx, |button, ctx| {
            button.set_disabled(true, ctx);
        });

        let close_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Cancel", NakedTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CommentEditorAction::CloseEditor);
                })
                .with_size(ButtonSize::Small)
        });

        let remove_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Remove", DangerNakedTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CommentEditorAction::RemoveComment);
                })
                .with_size(ButtonSize::Small)
        });

        (save_button, close_button, remove_button)
    }

    fn update_save_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        let is_empty = self.editor.as_ref(ctx).model().as_ref(ctx).is_empty(ctx);
        if is_empty != self.save_button_disabled {
            self.save_button_disabled = is_empty;
            self.save_button.update(ctx, |button, ctx| {
                button.set_disabled(is_empty, ctx);
            });
        }
    }

    fn handle_comment_model_event(
        &mut self,
        event: &PendingCommentEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PendingCommentEvent::NewPendingComment(line) => self.attach_to_line(line, ctx),
            PendingCommentEvent::ReopenPendingComment {
                id,
                line,
                comment_text,
                origin,
            } => {
                self.reopen_saved_comment(id, Some(line.clone()), comment_text, origin, ctx);
            }
        }
    }

    fn handle_editor_event(&mut self, event: &EditorViewEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorViewEvent::Edited => {
                self.update_save_button_state(ctx);
                ctx.emit(CommentEditorEvent::ContentChanged);
            }
            EditorViewEvent::CmdEnter => {
                self.save_comment(ctx);
            }
            EditorViewEvent::EscapePressed => {
                self.handle_escape(ctx);
            }
            EditorViewEvent::Focused
            | EditorViewEvent::Navigate(_)
            | EditorViewEvent::OpenFile { .. }
            | EditorViewEvent::RunWorkflow(_)
            | EditorViewEvent::EditWorkflow(_)
            | EditorViewEvent::OpenedBlockInsertionMenu(_)
            | EditorViewEvent::OpenedEmbeddedObjectSearch
            | EditorViewEvent::OpenedFindBar
            | EditorViewEvent::InsertedEmbeddedObject(_)
            | EditorViewEvent::CopiedBlock { .. }
            | EditorViewEvent::NavigatedCommands
            | EditorViewEvent::ChangedSelectionMode(_)
            | EditorViewEvent::TextSelectionChanged => {}
        }
    }

    /// Dismiss the composer only when pressing Escape on an empty draft (a non-empty draft is
    /// preserved). Shared by the Escape key handler and the test driver.
    fn handle_escape(&mut self, ctx: &mut ViewContext<Self>) {
        if self.editor.as_ref(ctx).model().as_ref(ctx).is_empty(ctx) {
            self.reset(ctx);
            ctx.emit(CommentEditorEvent::CloseEditor);
        }
    }

    fn attach_to_line(&mut self, line: &EditorLineLocation, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            // TODO: clear_buffer doesn't properly clear code blocks.
            // The `reset_with_markdown` call below is a band-aid fix.
            editor.reset_with_markdown("", ctx);
        });
        self.comment_id = None;
        self.line = Some(line.clone());
        self.show_remove_button = false;
        self.is_imported_comment = false;
        self.save_button.update(ctx, |button, ctx| {
            button.set_label("Comment", ctx);
        });
        self.update_save_button_state(ctx);
    }

    pub fn reopen_saved_comment(
        &mut self,
        id: &CommentId,
        line: Option<EditorLineLocation>,
        comment_text: &str,
        origin: &CommentOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor.update(ctx, |editor, ctx| {
            editor.model().update(ctx, |model, ctx| {
                model.reset_with_markdown(comment_text, ctx);
            });
        });

        self.comment_id = Some(*id);
        self.line = line;
        self.show_remove_button = true;
        self.is_imported_comment = origin.is_imported_from_github();

        self.save_button.update(ctx, |button, ctx| {
            button.set_label("Update", ctx);
        });
        ctx.notify();

        self.update_save_button_state(ctx);
    }

    fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            // TODO: system_clear_buffer doesn't properly clear code blocks.
            // The `reset_with_markdown` call below is a band-aid fix.
            editor.reset_with_markdown("", ctx);
        });
        self.comment_id = None;
        self.line = None;
        self.show_remove_button = false;
        self.is_imported_comment = false;

        self.save_button.update(ctx, |button, ctx| {
            button.set_label("Comment", ctx);
        });
        ctx.notify();

        self.update_save_button_state(ctx);
    }

    pub fn save_comment(&mut self, ctx: &mut ViewContext<Self>) {
        let comment_text = self.editor.as_ref(ctx).model().as_ref(ctx).markdown(ctx);

        if comment_text.trim().is_empty() {
            log::debug!("CommentEditor attempted to save empty comment, ignoring");
            return;
        }

        ctx.emit(CommentEditorEvent::CommentSaved {
            id: self.comment_id,
            comment_text: comment_text.clone(),
            line: self.line.clone(),
        });
        ctx.emit(CommentEditorEvent::CloseEditor);
    }

    fn render_github_import_indicator(
        &self,
        appearance: &Appearance,
        background: ColorU,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_text_color = theme.sub_text_color(Fill::Solid(background)).into_solid();
        let icon = Icon::Github
            .to_warpui_icon(Fill::Solid(sub_text_color))
            .finish();

        let label = Text::new(
            "Comment imported from GitHub".to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .soft_wrap(false)
        .with_clip(ClipConfig::end())
        .with_color(sub_text_color)
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(
                ConstrainedBox::new(icon)
                    .with_width(14.)
                    .with_height(14.)
                    .finish(),
            )
            .with_child(Shrinkable::new(1., label).finish())
            .finish()
    }

    fn render_action_buttons(&self) -> Box<dyn Element> {
        let mut action_buttons = vec![ChildView::new(&self.close_button).finish()];
        if self.show_remove_button {
            action_buttons.push(ChildView::new(&self.remove_button).finish());
        }
        action_buttons.push(ChildView::new(&self.save_button).finish());

        Flex::row()
            .with_spacing(4.)
            .with_children(action_buttons)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .finish()
    }

    fn render_footer_row(&self, appearance: &Appearance, background: ColorU) -> Box<dyn Element> {
        let action_buttons = self.render_action_buttons();
        let footer_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if self.is_imported_comment {
            footer_row
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(
                    Shrinkable::new(
                        1.,
                        self.render_github_import_indicator(appearance, background),
                    )
                    .finish(),
                )
                .with_child(action_buttons)
                .finish()
        } else {
            footer_row
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(action_buttons)
                .finish()
        }
    }
}

impl View for CommentEditor {
    fn ui_name() -> &'static str {
        "CommentEditor"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(ctx).as_ref(ctx);
        let background = inline_comment_background(appearance);

        let footer_row = self.render_footer_row(appearance, background);

        let footer_padding = if FeatureFlag::EmbeddedCodeReviewComments.is_enabled() {
            12.
        } else {
            4.
        };
        render_inline_comment_shell(
            ChildView::new(&self.editor).finish(),
            footer_row,
            Some(MAX_COMMENT_HEIGHT),
            footer_padding,
            appearance,
        )
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
        }
    }
}

impl TypedActionView for CommentEditor {
    type Action = CommentEditorAction;

    fn handle_action(&mut self, action: &CommentEditorAction, ctx: &mut ViewContext<Self>) {
        match action {
            CommentEditorAction::SaveComment => self.save_comment(ctx),
            CommentEditorAction::CloseEditor => {
                self.reset(ctx);
                ctx.emit(CommentEditorEvent::CloseEditor);
            }
            CommentEditorAction::RemoveComment => {
                if let Some(comment_id) = self.comment_id {
                    self.reset(ctx);
                    ctx.emit(CommentEditorEvent::DeleteComment { id: comment_id });
                    ctx.emit(CommentEditorEvent::CloseEditor);
                }
            }
        }
    }
}

impl Entity for CommentEditor {
    type Event = CommentEditorEvent;
}

pub(crate) fn create_editable_comment_markdown_editor<V>(
    markdown_content: Option<&str>,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<RichTextEditorView>
where
    V: View,
{
    create_comment_markdown_editor_inner(
        markdown_content,
        false,
        Some(Pixels::new(DEFAULT_COMMENT_MAX_WIDTH)),
        ctx,
    )
}

pub(crate) fn create_readonly_comment_markdown_editor<V>(
    markdown_content: &str,
    disable_scrolling: bool,
    max_width: Option<Pixels>,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<RichTextEditorView>
where
    V: View,
{
    let editor = create_comment_markdown_editor_inner(
        Some(markdown_content),
        disable_scrolling,
        max_width,
        ctx,
    );
    editor.update(ctx, |editor, ctx| {
        editor.set_interaction_state(InteractionState::Selectable, ctx);
    });
    editor
}

fn create_comment_markdown_editor_inner<V>(
    markdown_content: Option<&str>,
    disable_scrolling: bool,
    max_width: Option<Pixels>,
    ctx: &mut ViewContext<V>,
) -> ViewHandle<RichTextEditorView>
where
    V: View,
{
    let rich_text_styles = rich_text_styles(Appearance::as_ref(ctx), FontSettings::as_ref(ctx));
    let window_id = ctx.window_id();
    let parent_view_id = ctx.view_id();

    let model = ctx.add_model(|ctx| NotebooksEditorModel::new(rich_text_styles, window_id, ctx));
    let links = ctx.add_model(|ctx| NotebookLinks::new(SessionSource::Active(window_id), ctx));

    let parent_view_name = ctx.view_name(window_id, parent_view_id).unwrap_or_default();
    let parent_position_id = format!("{}_{}", parent_view_name, parent_view_id);

    // Embedded objects (notebooks, workflows) are disabled since comments don't support them.
    // Shell command execution is disabled so Cmd/Ctrl+Enter submits the comment instead.
    // Block insertion menu (slash menu) is disabled since the comment editor is small.
    let editor = ctx.add_typed_action_view(|ctx| {
        RichTextEditorView::new(
            parent_position_id,
            model.clone(),
            links,
            RichTextEditorConfig {
                gutter_width: Some(0.0),
                embedded_objects_enabled: Some(false),
                vertical_expansion_behavior: Some(VerticalExpansionBehavior::GrowToMaxHeight),
                max_width,
                can_execute_shell_commands: Some(false),
                disable_block_insertion_menu: true,
                disable_scrolling,
            },
            ctx,
        )
    });

    if let Some(comment_content) = markdown_content {
        model.update(ctx, |m, ctx| {
            m.reset_with_markdown(comment_content, ctx);
        });
    }

    editor
}

#[cfg(test)]
#[path = "comment_editor_tests.rs"]
mod tests;

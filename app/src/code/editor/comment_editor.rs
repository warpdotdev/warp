use crate::code::editor::comments::{EditorCommentsModel, PendingCommentEvent};
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::{CommentId, CommentOrigin};
use crate::editor::InteractionState;
use crate::notebooks::editor::{
    model::NotebooksEditorModel,
    rich_text_styles,
    view::{EditorViewEvent, RichTextEditorConfig, RichTextEditorView},
};
use crate::notebooks::link::{NotebookLinks, SessionSource};
use crate::settings::FontSettings;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, DangerNakedTheme, KeystrokeSource, NakedTheme, PrimaryTheme,
};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use std::cell::RefCell;
use warp_core::ui::{appearance::Appearance, theme::Fill};
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::{
    elements::{
        Border, ChildView, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Flex, MainAxisAlignment, MainAxisSize, ParentElement, Radius, Shrinkable, Text,
    },
    keymap::Keystroke,
    text_layout::ClipConfig,
    units::Pixels,
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

/// Default width of the comment editor, in pixels.
pub(crate) const DEFAULT_COMMENT_MAX_WIDTH: f32 = 750.0;

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

    #[allow(unused)] // TODO(CODE-1464): use this
    pub fn new_embedded(
        ctx: &mut ViewContext<Self>,
        comment_model: ModelHandle<EditorCommentsModel>,
        comment_id: Option<CommentId>,
        line: EditorLineLocation,
    ) -> Self {
        let editor = create_editable_comment_markdown_editor(None, ctx);

        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&comment_model, |me, _, event, ctx| {
            me.handle_comment_model_event(event, ctx);
        });

        let (save_button, close_button, remove_button) = Self::create_buttons(ctx);

        let show_remove_button = comment_id.is_some();

        let mut me = Self {
            comment_id,
            editor,
            save_button,
            close_button,
            remove_button,
            line: Some(line),
            show_remove_button,
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

    #[cfg_attr(not(feature = "local_fs"), allow(unused))]
    pub fn get_laid_out_size(&self) -> Option<Vector2F> {
        self.laid_out_size.borrow().as_ref().cloned()
    }

    #[allow(unused)] // TODO(CODE-1464): use this
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
            ActionButton::new(crate::i18n::tr_static(ctx, "Comment"), PrimaryTheme)
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

        let close_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Cancel"), NakedTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CommentEditorAction::CloseEditor);
                })
                .with_size(ButtonSize::Small)
        });

        let remove_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Remove"), DangerNakedTheme)
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
                // Dismiss the comment composer when pressing Escape on an empty draft.
                if self.editor.as_ref(ctx).model().as_ref(ctx).is_empty(ctx) {
                    self.reset(ctx);
                    ctx.emit(CommentEditorEvent::CloseEditor);
                }
            }
            _ => {}
        }
    }

    fn attach_to_line(&mut self, line: &EditorLineLocation, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            // TODO: clear_buffer doesn't properly clear code blocks.
            // The `reset_with_markdown` call below is a band-aid fix.
            editor.reset_with_markdown("", ctx);
        });
        self.line = Some(line.clone());
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
            button.set_label(crate::i18n::tr_static(ctx, "Update"), ctx);
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
            button.set_label(crate::i18n::tr_static(ctx, "Comment"), ctx);
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
        self.reset(ctx);
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
        let theme = appearance.theme();
        let background = blended_colors::neutral_2(theme);
        let border_color = blended_colors::neutral_4(theme);

        let footer_row = self.render_footer_row(appearance, background);

        Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_child(
                        Shrinkable::new(
                            1.,
                            Container::new(
                                Clipped::new(ChildView::new(&self.editor).finish()).finish(),
                            )
                            .with_padding_bottom(4.)
                            .with_padding_top(8.)
                            .with_horizontal_padding(12.)
                            .finish(),
                        )
                        .finish(),
                    )
                    .with_child(
                        Container::new(footer_row)
                            .with_vertical_padding(8.)
                            .with_horizontal_padding(8.)
                            .with_border(Border::top(1.).with_border_fill(border_color))
                            .finish(),
                    )
                    .finish(),
            )
            .with_max_height(200.)
            .with_max_width(DEFAULT_COMMENT_MAX_WIDTH)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_background_color(background)
        .with_border(Border::all(1.).with_border_fill(border_color))
        .finish()
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

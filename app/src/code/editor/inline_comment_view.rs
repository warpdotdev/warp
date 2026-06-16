use chrono::{DateTime, Local};
use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warp_editor::render::model::RenderState;
use warpui::elements::{
    ChildView, ConstrainedBox, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Shrinkable, Text,
};
use warpui::text_layout::ClipConfig;
use warpui::units::Pixels;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::code::editor::comment_editor::{
    comment_chrome_height, create_editable_comment_markdown_editor, inline_comment_background,
    render_inline_comment_shell,
};
use crate::code::editor::line::EditorLineLocation;
use crate::code::editor::EditorReviewComment;
use crate::code_review::comments::{CommentId, CommentOrigin};
use crate::editor::InteractionState;
use crate::notebooks::editor::view::{EditorViewEvent, RichTextEditorView};
use crate::ui_components::icons::Icon;
use crate::util::time_format::human_readable_approx_duration;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, DangerNakedTheme, NakedTheme, PrimaryTheme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineCommentMode {
    Saved,
    NewDraft,
    EditingExisting,
}

/// Bundled configuration for constructing an [`InlineCommentView`].
struct InlineCommentConfig {
    id: CommentId,
    line: EditorLineLocation,
    saved_content: String,
    last_update_time: DateTime<Local>,
    origin: CommentOrigin,
    mode: InlineCommentMode,
}

#[derive(Debug)]
pub enum InlineCommentViewAction {
    Edit,
    Remove,
    Save,
    Cancel,
}

#[derive(Debug)]
pub enum InlineCommentViewEvent {
    RequestEdit {
        id: CommentId,
    },
    RequestRemove {
        id: CommentId,
    },
    CommentSaved {
        id: CommentId,
        line: EditorLineLocation,
        comment_text: String,
    },
    Cancelled {
        id: CommentId,
    },
    ContentChanged,
}

/// A per-comment inline code-review comment view hosted in the diff editor.
///
/// The same view owns the same inner [`RichTextEditorView`] while moving between saved and editing
/// modes, so saving an inline comment does not replace the editor subtree and cause a transient
/// sizing mismatch.
pub struct InlineCommentView {
    id: CommentId,
    line: EditorLineLocation,
    saved_content: String,
    last_update_time: DateTime<Local>,
    origin: CommentOrigin,
    mode: InlineCommentMode,
    body_editor: ViewHandle<RichTextEditorView>,
    edit_button: ViewHandle<ActionButton>,
    remove_button: ViewHandle<ActionButton>,
    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    save_button_disabled: bool,
}

impl InlineCommentView {
    pub fn new(comment: EditorReviewComment, ctx: &mut ViewContext<Self>) -> Self {
        let body_editor =
            create_editable_comment_markdown_editor(Some(&comment.comment_content), ctx);
        Self::new_inner(
            InlineCommentConfig {
                id: comment.id,
                line: comment.line,
                saved_content: comment.comment_content,
                last_update_time: comment.last_update_time,
                origin: comment.origin,
                mode: InlineCommentMode::Saved,
            },
            body_editor,
            ctx,
        )
    }

    pub fn new_draft(line: EditorLineLocation, ctx: &mut ViewContext<Self>) -> Self {
        let body_editor = create_editable_comment_markdown_editor(None, ctx);
        Self::new_inner(
            InlineCommentConfig {
                id: CommentId::new(),
                line,
                saved_content: String::new(),
                last_update_time: Local::now(),
                origin: CommentOrigin::default(),
                mode: InlineCommentMode::NewDraft,
            },
            body_editor,
            ctx,
        )
    }

    fn new_inner(
        config: InlineCommentConfig,
        body_editor: ViewHandle<RichTextEditorView>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let edit_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Edit", NakedTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(InlineCommentViewAction::Edit))
        });
        let remove_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove", DangerNakedTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(InlineCommentViewAction::Remove))
        });
        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Comment", PrimaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(InlineCommentViewAction::Save))
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", NakedTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(InlineCommentViewAction::Cancel))
        });

        ctx.subscribe_to_view(&body_editor, |me, _, event, ctx| {
            me.handle_body_editor_event(event, ctx);
        });

        let mut me = Self {
            id: config.id,
            line: config.line,
            saved_content: config.saved_content,
            last_update_time: config.last_update_time,
            origin: config.origin,
            mode: config.mode,
            body_editor,
            edit_button,
            remove_button,
            save_button,
            cancel_button,
            save_button_disabled: true,
        };
        me.apply_mode(ctx);
        me.update_save_button_state(ctx);
        me
    }

    /// Refresh this view's saved data in place when the review comment batch delivers an update.
    /// Called instead of destroying and recreating the view, which would collapse the block height
    /// and discard in-progress edits.
    ///
    /// Metadata (line, timestamp, origin) is always updated. The body editor is only reset when
    /// the view is in `Saved` mode — if the user is actively editing, their draft is preserved and
    /// only `saved_content` is updated so that Cancel reverts to the latest server version.
    pub fn update_source(&mut self, comment: EditorReviewComment, ctx: &mut ViewContext<Self>) {
        self.id = comment.id;
        self.line = comment.line;
        self.last_update_time = comment.last_update_time;
        self.origin = comment.origin;
        if self.mode == InlineCommentMode::Saved && comment.comment_content != self.saved_content {
            self.body_editor.update(ctx, |editor, ctx| {
                editor.model().update(ctx, |model, ctx| {
                    model.reset_with_markdown(&comment.comment_content, ctx);
                });
            });
        }
        self.saved_content = comment.comment_content;
        if self.mode == InlineCommentMode::Saved {
            self.apply_mode(ctx);
        }
        ctx.notify();
    }

    pub fn begin_editing(&mut self, ctx: &mut ViewContext<Self>) {
        self.mode = InlineCommentMode::EditingExisting;
        self.body_editor.update(ctx, |editor, ctx| {
            editor.model().update(ctx, |model, ctx| {
                model.reset_with_markdown(&self.saved_content, ctx);
            });
        });
        self.apply_mode(ctx);
        self.update_save_button_state(ctx);
        ctx.focus(&self.body_editor);
        ctx.notify();
    }

    pub fn complete_save(&mut self, comment: EditorReviewComment, ctx: &mut ViewContext<Self>) {
        self.id = comment.id;
        self.line = comment.line;
        self.saved_content = comment.comment_content;
        self.last_update_time = comment.last_update_time;
        self.origin = comment.origin;
        self.mode = InlineCommentMode::Saved;
        self.apply_mode(ctx);
        ctx.notify();
    }

    pub fn cancel_editing(&mut self, ctx: &mut ViewContext<Self>) {
        if self.mode == InlineCommentMode::EditingExisting {
            self.mode = InlineCommentMode::Saved;
            self.body_editor.update(ctx, |editor, ctx| {
                editor.model().update(ctx, |model, ctx| {
                    model.reset_with_markdown(&self.saved_content, ctx);
                });
            });
            self.apply_mode(ctx);
            ctx.notify();
        }
    }

    pub fn id(&self) -> CommentId {
        self.id
    }

    pub fn line(&self) -> &EditorLineLocation {
        &self.line
    }

    pub fn is_editing(&self) -> bool {
        matches!(
            self.mode,
            InlineCommentMode::NewDraft | InlineCommentMode::EditingExisting
        )
    }

    pub fn is_new_draft(&self) -> bool {
        self.mode == InlineCommentMode::NewDraft
    }

    pub fn focus_body(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.body_editor);
    }

    pub fn comment_text(&self, app: &AppContext) -> String {
        self.body_editor
            .as_ref(app)
            .model()
            .as_ref(app)
            .markdown(app)
    }

    pub fn inner_render_state(&self, app: &AppContext) -> ModelHandle<RenderState> {
        self.body_editor
            .as_ref(app)
            .model()
            .as_ref(app)
            .render_state()
            .clone()
    }

    pub fn inline_height(&self, app: &AppContext) -> Pixels {
        // Read directly from the inner render state so the reserved block height is always
        // current. The `inner_render_state` observer fires after every layout pass (including
        // non-keystroke edits like select-all + delete), so `sync_inline_comment_blocks` always
        // sees an up-to-date value here.
        let content_height = self.inner_render_state(app).as_ref(app).height().as_f32();
        Pixels::new(content_height + comment_chrome_height(&self.save_button, app))
    }

    fn apply_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let interaction_state = if self.is_editing() {
            InteractionState::Editable
        } else {
            InteractionState::Selectable
        };
        self.body_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(interaction_state, ctx);
        });
        self.save_button.update(ctx, |button, ctx| {
            button.set_label(
                if self.mode == InlineCommentMode::EditingExisting {
                    "Update"
                } else {
                    "Comment"
                },
                ctx,
            );
        });
    }

    fn update_save_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        let is_empty = self
            .body_editor
            .as_ref(ctx)
            .model()
            .as_ref(ctx)
            .is_empty(ctx);
        if is_empty != self.save_button_disabled {
            self.save_button_disabled = is_empty;
            self.save_button.update(ctx, |button, ctx| {
                button.set_disabled(is_empty, ctx);
            });
        }
    }

    fn handle_body_editor_event(&mut self, event: &EditorViewEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorViewEvent::Edited => {
                self.update_save_button_state(ctx);
                ctx.emit(InlineCommentViewEvent::ContentChanged);
            }
            EditorViewEvent::CmdEnter => self.save(ctx),
            EditorViewEvent::EscapePressed => self.handle_escape(ctx),
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

    fn save(&mut self, ctx: &mut ViewContext<Self>) {
        let comment_text = self.comment_text(ctx);
        if comment_text.trim().is_empty() {
            return;
        }
        ctx.emit(InlineCommentViewEvent::CommentSaved {
            id: self.id,
            line: self.line.clone(),
            comment_text,
        });
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(InlineCommentViewEvent::Cancelled { id: self.id });
    }

    /// Escape must not discard unsaved work: a new draft cancels only while its body is empty, and
    /// an edit of an existing comment cancels only while the body still matches the saved content.
    /// Otherwise Escape is a no-op and the footer Cancel button remains the explicit way to discard
    /// changes.
    fn handle_escape(&mut self, ctx: &mut ViewContext<Self>) {
        let can_discard = match self.mode {
            InlineCommentMode::NewDraft => self
                .body_editor
                .as_ref(ctx)
                .model()
                .as_ref(ctx)
                .is_empty(ctx),
            InlineCommentMode::EditingExisting => self.comment_text(ctx) == self.saved_content,
            InlineCommentMode::Saved => false,
        };
        if can_discard {
            self.cancel(ctx);
        }
    }

    fn render_metadata(&self, appearance: &Appearance, background: ColorU) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_text_color = theme.sub_text_color(Fill::Solid(background)).into_solid();
        let mut leading = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);

        if self.origin.is_imported_from_github() {
            leading = leading.with_child(
                ConstrainedBox::new(
                    Icon::Github
                        .to_warpui_icon(Fill::Solid(sub_text_color))
                        .finish(),
                )
                .with_width(14.)
                .with_height(14.)
                .finish(),
            );
        }

        let relative_time = human_readable_approx_duration(
            Local::now() - self.last_update_time,
            true, /* sentence_case */
        );
        leading = leading.with_child(
            Text::new(
                relative_time,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .soft_wrap(false)
            .with_clip(ClipConfig::end())
            .with_color(sub_text_color)
            .finish(),
        );
        leading.finish()
    }

    fn render_saved_actions(&self) -> Box<dyn Element> {
        Flex::row()
            .with_spacing(4.)
            .with_children([
                ChildView::new(&self.edit_button).finish(),
                ChildView::new(&self.remove_button).finish(),
            ])
            .with_main_axis_alignment(MainAxisAlignment::End)
            .finish()
    }

    fn render_editing_actions(&self) -> Box<dyn Element> {
        let mut actions = vec![ChildView::new(&self.cancel_button).finish()];
        if self.mode == InlineCommentMode::EditingExisting {
            actions.push(ChildView::new(&self.remove_button).finish());
        }
        actions.push(ChildView::new(&self.save_button).finish());

        Flex::row()
            .with_spacing(4.)
            .with_children(actions)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .finish()
    }

    fn render_footer_row(&self, appearance: &Appearance, background: ColorU) -> Box<dyn Element> {
        let action_buttons = if self.is_editing() {
            self.render_editing_actions()
        } else {
            self.render_saved_actions()
        };

        let footer_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if self.mode == InlineCommentMode::NewDraft {
            footer_row
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(action_buttons)
                .finish()
        } else {
            footer_row
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(
                    Shrinkable::new(1., self.render_metadata(appearance, background)).finish(),
                )
                .with_child(action_buttons)
                .finish()
        }
    }
}

impl Entity for InlineCommentView {
    type Event = InlineCommentViewEvent;
}

impl TypedActionView for InlineCommentView {
    type Action = InlineCommentViewAction;

    fn handle_action(&mut self, action: &InlineCommentViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            InlineCommentViewAction::Edit => {
                ctx.emit(InlineCommentViewEvent::RequestEdit { id: self.id });
            }
            InlineCommentViewAction::Remove => {
                ctx.emit(InlineCommentViewEvent::RequestRemove { id: self.id });
            }
            InlineCommentViewAction::Save => self.save(ctx),
            InlineCommentViewAction::Cancel => self.cancel(ctx),
        }
    }
}

impl View for InlineCommentView {
    fn ui_name() -> &'static str {
        "InlineCommentView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let background = inline_comment_background(appearance);
        let footer_row = self.render_footer_row(appearance, background);
        render_inline_comment_shell(
            ChildView::new(&self.body_editor).finish(),
            footer_row,
            None,
            12.,
            appearance,
        )
    }
}

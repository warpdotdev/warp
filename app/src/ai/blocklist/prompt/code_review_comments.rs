//! The code review comments chip shown in the agent view input, plus its popup.
//!
//! This mirrors [`crate::ai::blocklist::prompt::plan_and_todo_list::PlanAndTodoListView`]
//! (the chip) and [`crate::ai::agent::todos::popup::AgentTodosPopupView`] (the popup),
//! but is driven by the active conversation's `CodeReview` state instead of its todo list.
//!
//! The chip shows `resolved / total` review-comment counts (where "resolved" means the
//! agent has addressed the comment, i.e. it moved from `pending_comments` to
//! `addressed_comments`). Clicking it opens a popup that lists each comment as a one-line
//! summary that can be expanded to show the full comment body.

use std::collections::HashSet;
use std::sync::Arc;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Dismiss, DispatchEventResult, DropShadow, Empty,
    EventHandler, Expanded, Flex, Hoverable, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition, ScrollbarWidth,
    Shrinkable, Stack, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity as _, TypedActionView,
    View, ViewContext, ViewHandle,
};

use crate::ai::agent::comment::ReviewComment;
use crate::ai::agent::icons::addressed_comment_icon;
use crate::ai::blocklist::{BlocklistAIContextEvent, BlocklistAIContextModel};
use crate::code_review::comments::CommentId;
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::ui_components::blended_colors;
use crate::BlocklistAIHistoryModel;

const COMMENTS_BUTTON_SAVE_POSITION_ID: &str = "code_review_comments::comments_button";

/// Defensive upper bound on the length of a comment's one-line summary in the popup.
/// The row also clips with an ellipsis based on the available width, so this only guards
/// against laying out pathologically long single-line strings.
const COMMENT_SUMMARY_MAX_CHARS: usize = 120;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        CodeReviewCommentsPopupAction::ClosePopup,
        id!(CodeReviewCommentsPopupView::ui_name()),
    )]);
}

/// A context chip that shows the code review comments for the active conversation.
pub struct CodeReviewCommentsView {
    context_model: ModelHandle<BlocklistAIContextModel>,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    terminal_view_id: EntityId,
    comments_button_mouse_state: MouseStateHandle,
    popup: ViewHandle<CodeReviewCommentsPopupView>,
    is_popup_open: bool,
    is_in_agent_view: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeReviewCommentsAction {
    TogglePopup,
}

impl CodeReviewCommentsView {
    pub fn new(
        context_model: ModelHandle<BlocklistAIContextModel>,
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        terminal_view_id: EntityId,
        is_in_agent_view: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let popup = ctx.add_typed_action_view(|ctx| {
            CodeReviewCommentsPopupView::new(terminal_view_id, context_model.clone(), ctx)
        });
        ctx.subscribe_to_view(&popup, |me, _, event, ctx| match event {
            CodeReviewCommentsPopupEvent::Close => {
                me.is_popup_open = false;
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                if event
                    .terminal_surface_id()
                    .is_some_and(|id| id != me.terminal_view_id)
                {
                    return;
                }
                // Re-render on any conversation-level change so the counts stay in sync as
                // comments are sent to (pending) and addressed by (resolved) the agent.
                ctx.notify();
            },
        );

        ctx.subscribe_to_model(&context_model, |_, _, event, ctx| {
            if let BlocklistAIContextEvent::PendingQueryStateUpdated = event {
                ctx.notify();
            }
        });

        Self {
            context_model,
            menu_positioning_provider,
            terminal_view_id,
            comments_button_mouse_state: Default::default(),
            popup,
            is_popup_open: false,
            is_in_agent_view,
        }
    }

    pub fn should_render(&self, app: &AppContext) -> bool {
        if !FeatureFlag::CodeReviewCommentsChip.is_enabled() {
            return false;
        }
        self.context_model
            .as_ref(app)
            .selected_conversation(app)
            .and_then(|conversation| conversation.code_review())
            .is_some_and(|code_review| {
                !code_review.addressed_comments.is_empty()
                    || !code_review.pending_comments.is_empty()
            })
    }

    fn render_comments_button(
        &self,
        resolved: usize,
        total: usize,
        icon_size: f32,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let primary_color = appearance.theme().surface_1();
        let comment_icon = Container::new(
            ConstrainedBox::new(
                Icon::MessageChatSquare
                    .to_warpui_icon(if self.is_in_agent_view {
                        appearance
                            .theme()
                            .sub_text_color(blended_colors::neutral_1(appearance.theme()).into())
                    } else {
                        internal_colors::fg_overlay_7(appearance.theme())
                    })
                    .finish(),
            )
            .with_height(icon_size)
            .with_width(icon_size)
            .finish(),
        )
        .finish();

        let chip_font_size = appearance.monospace_font_size() - 1.0;
        let line_height_ratio = appearance.line_height_ratio();

        let resolved_text = Text::new_inline(
            format!("{resolved}"),
            appearance.ui_font_family(),
            chip_font_size,
        )
        .with_color(blended_colors::text_main(appearance.theme(), primary_color))
        .with_line_height_ratio(line_height_ratio)
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let slash_text = Text::new_inline("/", appearance.ui_font_family(), chip_font_size)
            .with_color(appearance.theme().sub_text_color(primary_color).into())
            .with_line_height_ratio(line_height_ratio)
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish();

        let total_text = Text::new_inline(
            format!("{total}"),
            appearance.ui_font_family(),
            chip_font_size,
        )
        .with_color(appearance.theme().sub_text_color(primary_color).into())
        .with_line_height_ratio(line_height_ratio)
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(comment_icon)
            .with_child(Container::new(resolved_text).with_margin_left(4.).finish())
            .with_child(Container::new(slash_text).with_margin_left(2.).finish())
            .with_child(Container::new(total_text).with_margin_left(2.).finish())
            .finish();

        let button = Hoverable::new(self.comments_button_mouse_state.clone(), move |state| {
            let background = if state.is_hovered() {
                internal_colors::fg_overlay_2(appearance.theme())
            } else {
                internal_colors::fg_overlay_1(appearance.theme())
            };

            let container = Container::new(content)
                .with_background(background)
                .with_padding_left(6.)
                .with_padding_right(6.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(
                    Border::all(1.0)
                        .with_border_fill(internal_colors::neutral_3(appearance.theme())),
                )
                .with_padding_top(2.)
                .with_padding_bottom(2.)
                .finish();

            if state.is_hovered() {
                let mut stack = Stack::new().with_child(container);
                let tooltip_element = appearance
                    .ui_builder()
                    .tool_tip("View code review comments".to_string())
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(
                    tooltip_element,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
                stack.finish()
            } else {
                container
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(CodeReviewCommentsAction::TogglePopup);
        })
        .finish();

        let button = SavePosition::new(button, COMMENTS_BUTTON_SAVE_POSITION_ID).finish();

        let mut button = Stack::new().with_child(button);
        if self.is_popup_open {
            let positioning = match self.menu_positioning_provider.menu_position(app) {
                MenuPositioning::BelowInputBox => {
                    OffsetPositioning::offset_from_save_position_element(
                        COMMENTS_BUTTON_SAVE_POSITION_ID,
                        vec2f(0., 4.),
                        warpui::elements::PositionedElementOffsetBounds::WindowByPosition,
                        warpui::elements::PositionedElementAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    )
                }
                MenuPositioning::AboveInputBox => {
                    OffsetPositioning::offset_from_save_position_element(
                        COMMENTS_BUTTON_SAVE_POSITION_ID,
                        vec2f(0., -4.),
                        warpui::elements::PositionedElementOffsetBounds::WindowByPosition,
                        warpui::elements::PositionedElementAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    )
                }
            };
            button.add_positioned_overlay_child(ChildView::new(&self.popup).finish(), positioning);
        }

        button.finish()
    }
}

impl Entity for CodeReviewCommentsView {
    type Event = ();
}

impl View for CodeReviewCommentsView {
    fn ui_name() -> &'static str {
        "CodeReviewCommentsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);

        let Some(code_review) = self
            .context_model
            .as_ref(app)
            .selected_conversation(app)
            .and_then(|conversation| conversation.code_review())
        else {
            return Empty::new().finish();
        };
        let resolved = code_review.addressed_comments.len();
        let total = resolved + code_review.pending_comments.len();
        if total == 0 || !FeatureFlag::CodeReviewCommentsChip.is_enabled() {
            return Empty::new().finish();
        }

        let base_icon_size = app.font_cache().line_height(
            appearance.monospace_font_size(),
            DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4,
        );
        let text_line_height = app.font_cache().line_height(
            appearance.monospace_font_size() - 1.0,
            appearance.line_height_ratio(),
        );
        let icon_size = (base_icon_size * 1.1).min(text_line_height);

        self.render_comments_button(resolved, total, icon_size, appearance, app)
    }
}

impl TypedActionView for CodeReviewCommentsView {
    type Action = CodeReviewCommentsAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodeReviewCommentsAction::TogglePopup => {
                self.is_popup_open = !self.is_popup_open;
                if self.is_popup_open {
                    ctx.focus(&self.popup);
                }
                ctx.notify();
            }
        }
    }
}

/// A popup that lists the active conversation's code review comments, each expandable
/// from a one-line summary to its full body.
pub struct CodeReviewCommentsPopupView {
    terminal_view_id: EntityId,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    scroll_state: ClippedScrollStateHandle,
    expanded: HashSet<CommentId>,
}

#[derive(Debug, Clone, Copy)]
pub enum CodeReviewCommentsPopupAction {
    ClosePopup,
    ToggleExpanded(CommentId),
}

pub enum CodeReviewCommentsPopupEvent {
    Close,
}

struct PopupStyles {
    ui_font_family: warpui::fonts::FamilyId,
    background: Fill,
    main_text_color: ColorU,
    sub_text_color: ColorU,
    detail_font_size: f32,
}

impl CodeReviewCommentsPopupView {
    pub fn new(
        terminal_view_id: EntityId,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |me, _, event, ctx| {
                if event
                    .terminal_surface_id()
                    .is_some_and(|id| id != me.terminal_view_id)
                {
                    return;
                }
                ctx.notify();
            },
        );
        ctx.subscribe_to_model(&ai_context_model, |_, _, event, ctx| {
            if let BlocklistAIContextEvent::PendingQueryStateUpdated = event {
                ctx.notify();
            }
        });
        Self {
            terminal_view_id,
            ai_context_model,
            scroll_state: Default::default(),
            expanded: HashSet::new(),
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeReviewCommentsPopupEvent::Close);
    }

    fn styles(&self, appearance: &Appearance) -> PopupStyles {
        let theme = appearance.theme();
        let background = theme.surface_1();
        PopupStyles {
            ui_font_family: appearance.ui_font_family(),
            background,
            main_text_color: blended_colors::text_main(theme, background),
            sub_text_color: blended_colors::text_sub(theme, background),
            detail_font_size: appearance.ui_font_size(),
        }
    }

    fn render_header(
        &self,
        appearance: &Appearance,
        resolved: usize,
        total: usize,
    ) -> Box<dyn Element> {
        let styles = self.styles(appearance);
        let theme = appearance.theme();
        let mut header = Text::new(
            "Comments".to_string(),
            appearance.header_font_family(),
            styles.detail_font_size + 2.,
        )
        .with_color(styles.main_text_color)
        .with_style(Properties::default().weight(Weight::Semibold));
        header.add_text_with_highlights(
            format!(" {resolved}/{total}"),
            theme.sub_text_color(theme.surface_1()).into(),
            Properties::default().weight(Weight::Semibold),
        );
        header.finish()
    }

    fn render_row(
        &self,
        comment: &ReviewComment,
        resolved: bool,
        styles: &PopupStyles,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let comment_id = comment.id;
        let is_expanded = self.expanded.contains(&comment_id);

        // Resolved comments get the same green check treatment as the blocklist's
        // "addressed comment" chips; pending comments use a neutral chat icon.
        let icon_element = if resolved {
            addressed_comment_icon(appearance).finish()
        } else {
            Icon::MessageChatSquare
                .to_warpui_icon(Fill::Solid(styles.main_text_color))
                .finish()
        };

        let mut header_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);
        header_row.add_child(
            Container::new(
                ConstrainedBox::new(icon_element)
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        );
        header_row.add_child(
            Expanded::new(
                1.0,
                Text::new(
                    comment.summary(COMMENT_SUMMARY_MAX_CHARS),
                    styles.ui_font_family,
                    styles.detail_font_size,
                )
                .with_color(if resolved {
                    styles.sub_text_color
                } else {
                    styles.main_text_color
                })
                // Keep each comment on a single line, clipping with an ellipsis when the
                // text is wider than the popup rather than wrapping across multiple lines.
                .soft_wrap(false)
                .with_clip(ClipConfig::ellipsis())
                .finish(),
            )
            .finish(),
        );

        let header_row = EventHandler::new(header_row.finish())
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(CodeReviewCommentsPopupAction::ToggleExpanded(
                    comment_id,
                ));
                DispatchEventResult::StopPropagation
            })
            .finish();

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        col.add_child(header_row);
        if is_expanded {
            col.add_child(
                Container::new(
                    Text::new(
                        comment.content.clone(),
                        styles.ui_font_family,
                        styles.detail_font_size,
                    )
                    .with_color(theme.sub_text_color(styles.background).into())
                    .finish(),
                )
                .with_margin_left(24.)
                .with_margin_top(4.)
                .finish(),
            );
        }
        col.finish()
    }
}

impl View for CodeReviewCommentsPopupView {
    fn ui_name() -> &'static str {
        "CodeReviewCommentsPopup"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let Some(code_review) = self
            .ai_context_model
            .as_ref(app)
            .selected_conversation(app)
            .and_then(|conversation| conversation.code_review())
        else {
            // No empty state: the popup is only shown when there are comments.
            return Empty::new().finish();
        };
        let resolved = code_review.addressed_comments.len();
        let total = resolved + code_review.pending_comments.len();
        if total == 0 {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);
        let theme = appearance.theme();

        let mut list_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(12.);

        // Resolved (addressed) comments first, then the still-pending ones.
        for comment in &code_review.addressed_comments {
            list_col.add_child(self.render_row(comment, true, &styles, appearance));
        }
        for comment in &code_review.pending_comments {
            list_col.add_child(self.render_row(comment, false, &styles, appearance));
        }

        let header = Container::new(self.render_header(appearance, resolved, total))
            .with_padding_top(16.)
            .with_horizontal_padding(16.)
            .with_padding_bottom(8.)
            .finish();

        let scrollable_body = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            Container::new(list_col.finish())
                .with_horizontal_padding(16.)
                .with_padding_bottom(16.)
                .finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let panel_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Shrinkable::new(1.0, scrollable_body).finish());

        Dismiss::new(
            ConstrainedBox::new(
                Container::new(panel_col.finish())
                    .with_background(styles.background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .with_drop_shadow(DropShadow::default())
                    .finish(),
            )
            .with_width(300.)
            .with_max_height(420.)
            .finish(),
        )
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(CodeReviewCommentsPopupAction::ClosePopup);
        })
        .finish()
    }
}

impl Entity for CodeReviewCommentsPopupView {
    type Event = CodeReviewCommentsPopupEvent;
}

impl TypedActionView for CodeReviewCommentsPopupView {
    type Action = CodeReviewCommentsPopupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodeReviewCommentsPopupAction::ClosePopup => {
                self.close(ctx);
            }
            CodeReviewCommentsPopupAction::ToggleExpanded(comment_id) => {
                if !self.expanded.remove(comment_id) {
                    self.expanded.insert(*comment_id);
                }
                ctx.notify();
            }
        }
    }
}

#[cfg(test)]
#[path = "code_review_comments_tests.rs"]
mod tests;

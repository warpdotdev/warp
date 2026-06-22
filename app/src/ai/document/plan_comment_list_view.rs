//! Bottom panel that lists comments attached to a planning document.
//!
//! This mirrors the code review `CommentListView` UX (a collapsible, resizable list with Cancel /
//! "Send to Agent" actions) but is tailored to plan comments: there is no diff content, file paths,
//! or GitHub origin. It reuses the shared card rendering primitives from
//! [`crate::code_review::comment_rendering`] so the cards look identical to code review.

use chrono::{Duration, Local};
use indexmap::IndexMap;
use warp_core::ui::theme::color::internal_colors::{neutral_3, neutral_4, text_sub};
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::resizable::{
    resizable_state_handle, DragBarSide, Resizable, ResizableStateHandle,
};
use warpui::elements::{
    Border, ChildView, Clipped, ClippedScrollStateHandle, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, Element, Empty, EventHandler, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, ScrollbarWidth,
    Shrinkable, Stack,
};
use warpui::platform::Cursor;
use warpui::ui_components::button::{ButtonTooltipPosition, ButtonVariant};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::units::Pixels;
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::ai::document::plan_comments::{
    PlanComment, PlanCommentBatch, PlanCommentBatchEvent, PlanCommentId, PlanCommentTarget,
};
use crate::ai::AIRequestUsageModel;
use crate::appearance::Appearance;
use crate::code::editor::comment_editor::{
    create_readonly_comment_markdown_editor, DEFAULT_COMMENT_MAX_WIDTH,
};
use crate::code_review::comment_rendering::{
    comment_card_container, render_comment_file_path_header, render_comment_text_section,
};
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};

/// Maximum length of the quoted-text snippet shown as a card title.
const TITLE_MAX_CHARS: usize = 80;

#[derive(Clone, Debug, PartialEq)]
pub enum PlanCommentListAction {
    ToggleCollapsed,
    Cancel,
    Submit,
    EditComment(PlanCommentId),
    DeleteComment(PlanCommentId),
}

#[derive(Clone, Debug)]
pub enum PlanCommentListEvent {
    Submitted,
    Cancelled,
    EditComment(PlanCommentId),
    DeleteComment(PlanCommentId),
}

struct CardState {
    body_editor: ViewHandle<crate::notebooks::editor::view::RichTextEditorView>,
    edit_button: ViewHandle<ActionButton>,
    remove_button: ViewHandle<ActionButton>,
    source: PlanComment,
    title: String,
    last_updated_duration: Duration,
}

struct ViewState {
    scroll_state: ClippedScrollStateHandle,
    chevron_mouse_state: MouseStateHandle,
    cancel_button_mouse_state: MouseStateHandle,
    submit_button_mouse_state: MouseStateHandle,
    resizable_state: ResizableStateHandle,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scroll_state: Default::default(),
            chevron_mouse_state: Default::default(),
            cancel_button_mouse_state: Default::default(),
            submit_button_mouse_state: Default::default(),
            resizable_state: resizable_state_handle(300.0),
        }
    }
}

pub struct PlanCommentListView {
    cards: IndexMap<PlanCommentId, CardState>,
    is_collapsed: bool,
    view_state: ViewState,
}

impl PlanCommentListView {
    pub fn new(comment_model: ModelHandle<PlanCommentBatch>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &comment_model,
            |me, model, _event: &PlanCommentBatchEvent, ctx| {
                me.refresh_from_model(&model, ctx);
            },
        );

        let mut me = Self {
            cards: IndexMap::new(),
            is_collapsed: false,
            view_state: ViewState::default(),
        };
        me.refresh_from_model(&comment_model, ctx);
        me
    }

    fn refresh_from_model(
        &mut self,
        model: &ModelHandle<PlanCommentBatch>,
        ctx: &mut ViewContext<Self>,
    ) {
        let comments = model.read(ctx, |batch, _| batch.comments().to_vec());
        let mut new_cards = IndexMap::with_capacity(comments.len());

        for comment in comments {
            let id = comment.id;
            let card = if let Some(mut existing) = self.cards.shift_remove(&id) {
                // Reset the body editor content in case it changed.
                existing.body_editor.update(ctx, |editor, ctx| {
                    editor.model().update(ctx, |model, ctx| {
                        model.reset_with_markdown(&comment.body, ctx);
                    });
                });
                existing.title = Self::comment_title(&comment);
                existing.last_updated_duration = Local::now() - comment.last_update_time;
                existing.source = comment;
                existing
            } else {
                Self::create_card(comment, ctx)
            };
            new_cards.insert(id, card);
        }

        self.cards = new_cards;
        ctx.notify();
    }

    fn create_card(comment: PlanComment, ctx: &mut ViewContext<Self>) -> CardState {
        let body_editor = create_readonly_comment_markdown_editor(
            &comment.body,
            false,
            Some(Pixels::new(DEFAULT_COMMENT_MAX_WIDTH)),
            ctx,
        );

        let id = comment.id;
        let edit_button = ctx.add_view(|_| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::Pencil)
                .with_size(ButtonSize::Small)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(PlanCommentListAction::EditComment(id));
                })
        });
        let remove_button = ctx.add_view(|_| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::Trash)
                .with_size(ButtonSize::Small)
                .on_click(move |ctx| {
                    ctx.dispatch_typed_action(PlanCommentListAction::DeleteComment(id));
                })
        });

        let title = Self::comment_title(&comment);
        let last_updated_duration = Local::now() - comment.last_update_time;

        CardState {
            body_editor,
            edit_button,
            remove_button,
            source: comment,
            title,
            last_updated_duration,
        }
    }

    /// The card title: a single-line, truncated snippet of the quoted text, or a generic label for
    /// document-level comments.
    fn comment_title(comment: &PlanComment) -> String {
        match &comment.target {
            PlanCommentTarget::DocumentRange { quoted_text, .. } => {
                let single_line = quoted_text.split('\n').next().unwrap_or("").trim();
                if single_line.is_empty() {
                    return "Plan comment".to_string();
                }
                if single_line.chars().count() > TITLE_MAX_CHARS {
                    let truncated: String = single_line.chars().take(TITLE_MAX_CHARS).collect();
                    format!("{truncated}…")
                } else {
                    single_line.to_string()
                }
            }
            PlanCommentTarget::General => "Plan".to_string(),
        }
    }

    pub fn expand(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_collapsed {
            self.is_collapsed = false;
            ctx.notify();
        }
    }

    fn has_active_comments(&self) -> bool {
        self.cards.values().any(|card| !card.source.outdated)
    }

    fn render_header(&self, appearance: &Appearance, ctx: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        header_row.add_child(self.render_header_left(appearance));
        header_row.add_child(self.render_header_right(appearance, ctx));

        warpui::elements::Container::new(
            Clipped::new(Shrinkable::new(1., header_row.finish()).finish()).finish(),
        )
        .with_background(neutral_3(theme))
        .with_vertical_padding(8.)
        .with_horizontal_padding(16.)
        .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
        .with_border(
            Border::new(1.)
                .with_sides(true, true, false, true)
                .with_border_fill(warp_core::ui::theme::Fill::Solid(neutral_4(theme))),
        )
        .finish()
    }

    fn render_header_left(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let count = self.cards.len();
        let label = format!("{count} comment{}", if count == 1 { "" } else { "s" });

        let icon = if self.is_collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        let icon_element = icon
            .to_warpui_icon(warp_core::ui::theme::Fill::Solid(text_sub(
                theme,
                neutral_3(theme),
            )))
            .finish();

        let toggle = Hoverable::new(self.view_state.chevron_mouse_state.clone(), move |_| {
            warpui::elements::ConstrainedBox::new(icon_element)
                .with_width(16.)
                .with_height(16.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(PlanCommentListAction::ToggleCollapsed);
        })
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                warpui::elements::Container::new(toggle)
                    .with_margin_right(8.)
                    .finish(),
            )
            .with_child(
                warpui::elements::Text::new(
                    label,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(
                    theme
                        .main_text_color(warp_core::ui::theme::Fill::Solid(neutral_3(theme)))
                        .into_solid(),
                )
                .finish(),
            )
            .finish()
    }

    fn render_header_right(&self, appearance: &Appearance, ctx: &AppContext) -> Box<dyn Element> {
        let mut right_section = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        right_section.add_child(self.render_cancel_button(appearance));
        right_section.add_child(self.render_send_button(appearance, ctx));
        right_section.finish()
    }

    fn render_cancel_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let cancel_button = EventHandler::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Text,
                    self.view_state.cancel_button_mouse_state.clone(),
                )
                .with_text_label("Cancel".to_string())
                .build()
                .finish(),
        )
        .on_left_mouse_down(|ctx, _, _| {
            ctx.dispatch_typed_action(PlanCommentListAction::Cancel);
            DispatchEventResult::StopPropagation
        })
        .finish();
        warpui::elements::Container::new(cancel_button)
            .with_margin_right(8.)
            .finish()
    }

    fn render_send_button(&self, appearance: &Appearance, ctx: &AppContext) -> Box<dyn Element> {
        let ai_available = AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx);
        let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let has_active = self.has_active_comments();
        let enable_send = ai_available && ai_enabled && has_active;

        let tooltip_text = if !ai_enabled {
            "AI must be enabled to send comments to Agent"
        } else if !ai_available {
            "Agent review requires AI credits"
        } else if !has_active {
            "No comments to send"
        } else {
            "Send plan comments to Agent"
        };

        let tooltip = appearance
            .ui_builder()
            .tool_tip(tooltip_text.to_string())
            .build()
            .finish();

        let button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.view_state.submit_button_mouse_state.clone(),
            )
            .with_text_label("Send to Agent".to_string())
            .with_tooltip(|| tooltip)
            .with_tooltip_position(ButtonTooltipPosition::AboveLeft);

        if enable_send {
            EventHandler::new(button.build().finish())
                .on_left_mouse_down(|ctx, _, _| {
                    ctx.dispatch_typed_action(PlanCommentListAction::Submit);
                    DispatchEventResult::StopPropagation
                })
                .finish()
        } else {
            let background_fill = appearance.theme().surface_3();
            let foreground_color = appearance
                .theme()
                .disabled_text_color(background_fill)
                .into_solid();
            button
                .with_style(UiComponentStyles {
                    background: Some(background_fill.into_solid().into()),
                    border_color: Some(foreground_color.into()),
                    font_color: Some(foreground_color),
                    ..Default::default()
                })
                .build()
                .finish()
        }
    }

    fn render_panel(&self, appearance: &Appearance, ctx: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let header = self.render_header(appearance, ctx);

        let mut comments_column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for card in self.cards.values() {
            comments_column.add_child(
                warpui::elements::Container::new(self.render_card(card, appearance))
                    .with_margin_bottom(12.)
                    .finish(),
            );
        }

        let scrollable_content = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.view_state.scroll_state.clone(),
                child: warpui::elements::Container::new(comments_column.finish())
                    .with_uniform_padding(16.)
                    .finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(header)
            .with_child(Expanded::new(1., scrollable_content).finish())
            .finish()
    }

    fn render_card(&self, card: &CardState, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let trailing = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&card.edit_button).finish())
            .with_child(ChildView::new(&card.remove_button).finish())
            .finish();

        let header = render_comment_file_path_header(
            &card.title,
            card.source.outdated,
            Some(trailing),
            CornerRadius::with_top(Radius::Pixels(8.)),
            None,
            appearance,
        );

        let body = render_comment_text_section(
            &card.body_editor,
            card.last_updated_duration,
            false, /* is_imported_from_github */
            None,
            appearance,
        );

        let card_inner = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children([header, body])
            .finish();

        comment_card_container(card_inner, theme)
    }
}

impl Entity for PlanCommentListView {
    type Event = PlanCommentListEvent;
}

impl View for PlanCommentListView {
    fn ui_name() -> &'static str {
        "PlanCommentListView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        if self.cards.is_empty() {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(ctx);

        if self.is_collapsed {
            return self.render_header(appearance, ctx);
        }

        let panel = self.render_panel(appearance, ctx);

        Stack::new()
            .with_child(
                Resizable::new(self.view_state.resizable_state.clone(), panel)
                    .with_dragbar_side(DragBarSide::Top)
                    .with_dragbar_color(warpui::elements::Fill::Solid(
                        warpui::color::ColorU::transparent_black(),
                    ))
                    .with_bounds_callback(Box::new(|window_size| (100.0, window_size.y() * 0.8)))
                    .on_resize(|ctx, _| {
                        ctx.notify();
                    })
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for PlanCommentListView {
    type Action = PlanCommentListAction;

    fn handle_action(&mut self, action: &PlanCommentListAction, ctx: &mut ViewContext<Self>) {
        match action {
            PlanCommentListAction::ToggleCollapsed => {
                self.is_collapsed = !self.is_collapsed;
                ctx.notify();
            }
            PlanCommentListAction::Cancel => {
                ctx.emit(PlanCommentListEvent::Cancelled);
            }
            PlanCommentListAction::Submit => {
                ctx.emit(PlanCommentListEvent::Submitted);
            }
            PlanCommentListAction::EditComment(id) => {
                ctx.emit(PlanCommentListEvent::EditComment(*id));
            }
            PlanCommentListAction::DeleteComment(id) => {
                ctx.emit(PlanCommentListEvent::DeleteComment(*id));
            }
        }
    }
}

use std::borrow::Cow;

use indexmap::IndexMap;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use string_offset::CharOffset;
use vec1::vec1;
use warp_core::features::FeatureFlag;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::color::internal_colors::{
    accent_overlay_2, accent_overlay_3, neutral_1, neutral_3, neutral_4, neutral_6, text_main,
    text_sub,
};
use warp_core::ui::theme::Fill;
use warp_editor::model::CoreEditorModel;
use warpui::clipboard::ClipboardContent;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::resizable::{
    resizable_state_handle, DragBarSide, Resizable, ResizableStateHandle,
};
use warpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Dismiss, DispatchEventResult, Element, Empty, EventHandler,
    Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentElement, PositionedElementAnchor, PositionedElementOffsetBounds,
    Radius, SavePosition, ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Shrinkable, Stack,
    Text,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::units::Pixels;
use warpui::{
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle,
};

use crate::ai::request_usage_model::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::appearance::Appearance;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::comment_editor::DEFAULT_COMMENT_MAX_WIDTH;
use crate::code::editor::view::{CodeEditorEvent, CodeEditorView};
use crate::code_review::code_review_view::CodeReviewView;
use crate::code_review::comment_rendering::CommentViewCard;
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentId, CommentOrigin,
    ReviewCommentBatch, ReviewCommentBatchEvent,
};
use crate::code_review::telemetry_event::CodeReviewTelemetryEvent;
use crate::menu::{Event, Menu, MenuItem, MenuItemFields};
use crate::notebooks::editor::view::{EditorViewEvent, RichTextEditorView};
use crate::send_telemetry_from_ctx;
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, KeystrokeSource, NakedTheme, PrimaryTheme,
    SecondaryTheme,
};
use crate::workspace::view::right_panel::ReviewDestination;

/// Header text for the outdated section when there is exactly one outdated comment.
const OUTDATED_SECTION_HEADER_SINGULAR: &str = "1 comment will be omitted because it is outdated.";
/// Header text format for the outdated section when there are multiple outdated comments.
/// Use with `format!` to insert the count.
const OUTDATED_SECTION_HEADER_PLURAL_FMT: &str =
    " comments will be omitted because they are outdated.";

/// Returns the header text for the outdated section based on the number of outdated comments.
fn outdated_section_header_text(count: usize) -> Cow<'static, str> {
    if count == 1 {
        Cow::Borrowed(OUTDATED_SECTION_HEADER_SINGULAR)
    } else {
        Cow::Owned(format!("{count}{OUTDATED_SECTION_HEADER_PLURAL_FMT}"))
    }
}

/// Convert markdown text to HTML using the editor's buffer serialization.
/// This function takes a comment editor view that has already been created with markdown content
/// and extracts the HTML representation from its buffer.
fn markdown_to_html(
    editor_view: &ViewHandle<RichTextEditorView>,
    ctx: &AppContext,
) -> Option<String> {
    editor_view.read(ctx, |view, ctx| {
        let model = view.model();
        model.read(ctx, |model, ctx| {
            let buffer = model.content();
            let range = CharOffset::from(1)..buffer.as_ref(ctx).max_charoffset();
            buffer.as_ref(ctx).ranges_as_html(vec1![range], ctx)
        })
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommentListAction {
    ToggleCollapsed,
    ToggleOutdatedCollapsed,
    CopyCommentText,
    EditComment,
    JumpToCommentLocation(CommentId),
    Cancel,
    Submit,
    ShowOverflow { comment_id: CommentId },
    DeleteComment,
    DismissOverflowMenu,
    ViewInGitHub { url: String },
}

#[derive(Clone, Debug)]
pub enum CommentListEvent {
    Submitted,
    Cancelled,
    DeleteComment { comment_id: CommentId },
    EditComment(CommentId),
    JumpToCommentLocation(CommentId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommentListDebugState {
    pub review_destination: ReviewDestination,
    pub total_comments: usize,
    pub sendable_comments: usize,
    pub is_collapsed: bool,
    pub is_outdated_section_collapsed: Option<bool>,
    pub ai_available: bool,
    pub ai_enabled: bool,
    pub send_button_tooltip_text: String,
}

struct ViewState {
    scroll_state: ClippedScrollStateHandle,
    chevron_mouse_state: MouseStateHandle,
    outdated_chevron_mouse_state: MouseStateHandle,
    cancel_button_mouse_state: MouseStateHandle,
    resizable_state: ResizableStateHandle,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scroll_state: Default::default(),
            chevron_mouse_state: Default::default(),
            outdated_chevron_mouse_state: Default::default(),
            cancel_button_mouse_state: Default::default(),
            resizable_state: resizable_state_handle(300.0),
        }
    }
}

struct CommentDisplayState {
    card: CommentViewCard,
    icon_button: ViewHandle<ActionButton>,
    mouse_state: MouseStateHandle,
}

impl CommentDisplayState {
    fn save_position_id(&self) -> String {
        Self::overflow_menu_position_id(self.card.source().id)
    }

    fn comment_position_id(&self) -> String {
        Self::comment_position_id_for(self.card.source().id)
    }

    fn comment_position_id_for(comment_id: CommentId) -> String {
        format!("comment_list_view:comment:{comment_id}")
    }

    fn overflow_menu_position_id(comment_id: CommentId) -> String {
        format!("comment_list_view:{comment_id}")
    }
}

pub struct CommentListView {
    parent: WeakViewHandle<CodeReviewView>,

    comment_model: Option<ModelHandle<ReviewCommentBatch>>,

    comments_by_id: IndexMap<CommentId, CommentDisplayState>,

    is_collapsed: bool,

    /// Set once the user has manually collapsed or expanded the outdated section.
    is_outdated_section_collapsed: Option<bool>,
    repo_path: Option<LocalOrRemotePath>,
    view_state: ViewState,
    /// The best available destination for sending review comments.
    /// Pushed down from RightPanelView.
    review_destination: ReviewDestination,
    overflow_menu: ViewHandle<Menu<CommentListAction>>,
    active_overflow_comment_id: Option<CommentId>,
    pending_scroll_to_comment: Option<CommentId>,
    comments_button: ViewHandle<ActionButton>,
    send_button: ViewHandle<ActionButton>,
}

impl CommentListView {
    pub fn new(
        initial_repo_path: Option<LocalOrRemotePath>,
        parent: WeakViewHandle<CodeReviewView>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let menu = ctx.add_view(|_| Menu::new());

        let comments_button = ctx.add_view(|_| {
            ActionButton::new("1 Comment", CustomSecondaryActionTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CommentListAction::ToggleCollapsed);
                })
        });

        let send_button = ctx.add_view(|ctx| {
            ActionButton::new("Send to Agent", PrimaryTheme)
                .with_size(ButtonSize::Small)
                .with_keybinding(
                    KeystrokeSource::Fixed(
                        Keystroke::parse(crate::code_review::CODE_REVIEW_SUBMIT_KEYSTROKE)
                            .unwrap_or_default(),
                    ),
                    ctx,
                )
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CommentListAction::Submit);
                })
        });

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            Event::ItemSelected => {}
            Event::Close { .. } => {
                me.close_overflow_menu(ctx);
            }
            Event::ItemHovered => {}
        });

        // Keep the stored button state in sync when AI availability changes.
        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |me, _, event, ctx| {
            if let AIRequestUsageModelEvent::RequestUsageUpdated = event {
                me.sync_send_button(ctx);
            }
        });

        Self {
            parent,
            comment_model: None,
            comments_by_id: IndexMap::new(),
            is_collapsed: true,
            is_outdated_section_collapsed: None,
            repo_path: initial_repo_path,
            view_state: ViewState::default(),
            overflow_menu: menu,
            review_destination: ReviewDestination::None,
            active_overflow_comment_id: None,
            pending_scroll_to_comment: None,
            comments_button,
            send_button,
        }
    }

    fn recompute_comment_button_label(&mut self, ctx: &mut ViewContext<Self>) {
        let total_count = self.comments_by_id.len();

        let label_text = if FeatureFlag::PRCommentsSlashCommand.is_enabled() {
            let non_outdated_count = self
                .comments_by_id
                .values()
                .filter(|state| !state.card.source().outdated)
                .count();

            if non_outdated_count == 0 && total_count > 0 {
                format!(
                    "{} outdated comment{}",
                    total_count,
                    if total_count == 1 { "" } else { "s" }
                )
            } else {
                format!(
                    "{} comment{}",
                    non_outdated_count,
                    if non_outdated_count == 1 { "" } else { "s" }
                )
            }
        } else {
            format!(
                "{} comment{}",
                total_count,
                if total_count == 1 { "" } else { "s" }
            )
        };

        self.comments_button
            .update(ctx, |view, ctx| view.set_label(label_text, ctx));
    }

    pub fn set_review_destination(
        &mut self,
        destination: ReviewDestination,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.review_destination != destination {
            self.review_destination = destination;
            self.sync_send_button(ctx);
            ctx.notify();
        }
    }

    fn repo_is_local(&self) -> Option<bool> {
        self.repo_path.as_ref().map(LocalOrRemotePath::is_local)
    }

    pub fn debug_state(&self, ctx: &AppContext) -> CommentListDebugState {
        let ai_available = AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx);
        let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let sendable_comments = self
            .comments_by_id
            .values()
            .filter(|state| !state.card.source().outdated)
            .count();
        let send_button_tooltip_text = Self::send_button_tooltip_text(
            &self.review_destination,
            sendable_comments > 0,
            ai_available,
            ai_enabled,
        )
        .into_owned();

        CommentListDebugState {
            review_destination: self.review_destination.clone(),
            total_comments: self.comments_by_id.len(),
            sendable_comments,
            is_collapsed: self.is_collapsed,
            is_outdated_section_collapsed: self.is_outdated_section_collapsed,
            ai_available,
            ai_enabled,
            send_button_tooltip_text,
        }
    }

    pub fn set_comment_model(
        &mut self,
        comment_model: Option<ModelHandle<ReviewCommentBatch>>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.comment_model == comment_model {
            return;
        }

        if let Some(old_model) = self.comment_model.take() {
            ctx.unsubscribe_to_model(&old_model);
        }

        self.comment_model = comment_model.clone();

        if let Some(model) = comment_model {
            ctx.subscribe_to_model(&model, Self::handle_comment_model_event);
            self.refresh_from_model(&model, ctx);
        } else {
            self.clear_comments(ctx);
        }
    }

    fn handle_comment_model_event(
        &mut self,
        model: ModelHandle<ReviewCommentBatch>,
        _event: &ReviewCommentBatchEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.comment_model.as_ref() != Some(&model) {
            // Only handle events from the current model.
            return;
        }

        self.refresh_from_model(&model, ctx);
    }

    fn refresh_from_model(
        &mut self,
        model: &ModelHandle<ReviewCommentBatch>,
        ctx: &mut ViewContext<Self>,
    ) {
        let comments = model.read(ctx, |batch, _| batch.comments.clone());
        self.set_comments_internal(comments, ctx);
    }

    fn set_comments_internal(
        &mut self,
        comments: Vec<AttachedReviewComment>,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut new_comments_by_id = IndexMap::with_capacity(comments.len());

        for comment in comments {
            let id = comment.id;

            let entry = if let Some(mut existing) = self.comments_by_id.shift_remove(&id) {
                existing
                    .card
                    .update_source(comment, self.repo_path.as_ref(), ctx);
                existing
            } else {
                let card = CommentViewCard::new(
                    comment,
                    false, /* always_use_static_diff */
                    false, /* disable_scrolling */
                    Some(Pixels::new(DEFAULT_COMMENT_MAX_WIDTH)),
                    self.repo_path.as_ref(),
                    ctx,
                );

                ctx.subscribe_to_view(
                    card.comment_editor(),
                    Self::handle_comment_editor_selection_events,
                );
                if let Some(diff_editor) = card.static_diff_editor() {
                    ctx.subscribe_to_view(
                        diff_editor,
                        Self::handle_static_diff_editor_selection_events,
                    );
                }

                let comment_id = id;
                let action_button = ActionButton::new("", NakedTheme)
                    .with_icon(Icon::DotsVertical)
                    .with_size(ButtonSize::Small)
                    .on_click(move |ctx| {
                        ctx.dispatch_typed_action(CommentListAction::ShowOverflow { comment_id })
                    });
                let action_button = ctx.add_view(|_| action_button);

                CommentDisplayState {
                    card,
                    icon_button: action_button,
                    mouse_state: Default::default(),
                }
            };

            new_comments_by_id.insert(id, entry);
        }

        self.comments_by_id = new_comments_by_id;

        if self
            .active_overflow_comment_id
            .is_some_and(|id| !self.comments_by_id.contains_key(&id))
        {
            self.active_overflow_comment_id = None;
        }

        if self
            .pending_scroll_to_comment
            .is_some_and(|id| self.comments_by_id.contains_key(&id))
        {
            let comment_id = self
                .pending_scroll_to_comment
                .take()
                .expect("pending scroll target was verified as present");
            self.ensure_outdated_section_expanded(comment_id, ctx);
            self.view_state
                .scroll_state
                .scroll_to_position(ScrollTarget {
                    position_id: CommentDisplayState::comment_position_id_for(comment_id),
                    mode: ScrollToPositionMode::TopIntoView,
                });
        }

        self.recompute_comment_button_label(ctx);
        self.sync_send_button(ctx);
        ctx.notify();
    }

    pub fn expand(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_collapsed {
            self.is_collapsed = false;
            ctx.notify();
        }
    }

    pub fn scroll_to_comment(&mut self, comment_id: CommentId, ctx: &mut ViewContext<Self>) {
        self.pending_scroll_to_comment = Some(comment_id);
        self.expand(ctx);
        self.ensure_outdated_section_expanded(comment_id, ctx);

        if self.comments_by_id.contains_key(&comment_id) {
            self.view_state
                .scroll_state
                .scroll_to_position(ScrollTarget {
                    position_id: CommentDisplayState::comment_position_id_for(comment_id),
                    mode: ScrollToPositionMode::TopIntoView,
                });
            self.pending_scroll_to_comment = None;
        }

        ctx.notify();
    }

    /// If the given comment is outdated, force-expand the outdated section so
    /// the comment is visible and the scroll position can be calculated.
    fn ensure_outdated_section_expanded(
        &mut self,
        comment_id: CommentId,
        ctx: &mut ViewContext<Self>,
    ) {
        let is_outdated = self
            .comments_by_id
            .get(&comment_id)
            .is_some_and(|state| state.card.source().outdated);

        if is_outdated && self.outdated_section_should_be_collapsed(ctx) {
            self.is_outdated_section_collapsed = Some(false);
        }
    }

    fn handle_comment_editor_selection_events(
        &mut self,
        view: ViewHandle<RichTextEditorView>,
        event: &EditorViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorViewEvent::TextSelectionChanged => {
                if view.as_ref(ctx).selected_text(ctx).is_some() {
                    self.clear_other_comment_selections(Some(view.id()), ctx);
                }
            }
            EditorViewEvent::Focused => {
                self.clear_other_comment_selections(Some(view.id()), ctx);
            }
            _ => {}
        }
    }

    fn handle_static_diff_editor_selection_events(
        &mut self,
        view: ViewHandle<CodeEditorView>,
        event: &CodeEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeEditorEvent::SelectionChanged => {
                if view.as_ref(ctx).selected_text(ctx).is_some() {
                    self.clear_other_comment_selections(Some(view.id()), ctx);
                }
            }
            CodeEditorEvent::Focused => {
                self.clear_other_comment_selections(Some(view.id()), ctx);
            }
            _ => {}
        }
    }

    fn clear_other_comment_selections(
        &mut self,
        source_view_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        for state in self.comments_by_id.values() {
            let card = &state.card;
            if source_view_id.is_none_or(|id| card.comment_editor().id() != id) {
                card.comment_editor()
                    .update(ctx, |view, ctx| view.clear_text_selection(ctx));
            }
            if let Some(diff_editor) = card.static_diff_editor() {
                if source_view_id.is_none_or(|id| diff_editor.id() != id) {
                    diff_editor.update(ctx, |view, ctx| view.clear_selection(ctx));
                }
            }
        }
    }

    pub fn clear_comments(&mut self, ctx: &mut ViewContext<Self>) {
        self.comments_by_id.clear();
        self.is_collapsed = true;
        self.pending_scroll_to_comment = None;
        ctx.notify();
    }

    fn render_panel(&self, appearance: &Appearance, ctx: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        let header = self.render_header(appearance);

        let mut comments_column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        if FeatureFlag::PRCommentsSlashCommand.is_enabled() {
            let (outdated_comments, active_comments): (Vec<_>, Vec<_>) = self
                .comments_by_id
                .values()
                .partition(|state| state.card.source().outdated);

            if !outdated_comments.is_empty() {
                comments_column.add_child(self.render_outdated_section(
                    &outdated_comments,
                    appearance,
                    ctx,
                ));
            }

            for comment_render_state in active_comments {
                comments_column.add_child(
                    Container::new(self.render_comment(comment_render_state, ctx))
                        .with_margin_bottom(12.)
                        .finish(),
                );
            }
        } else {
            for comment_render_state in self.comments_by_id.values() {
                comments_column.add_child(
                    Container::new(self.render_comment(comment_render_state, ctx))
                        .with_margin_bottom(12.)
                        .finish(),
                );
            }
        }

        let scrollable_content = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.view_state.scroll_state.clone(),
                child: Container::new(comments_column.finish())
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

    fn outdated_section_should_be_collapsed(&self, ctx: &AppContext) -> bool {
        if let Some(manually_collapsed) = self.is_outdated_section_collapsed {
            return manually_collapsed;
        }

        // If there are only outdated comments, don't collapse the outdated section.
        if let Some(comment_model) = &self.comment_model {
            let has_only_outdated_comments = comment_model.as_ref(ctx).has_only_outdated_comments();
            return !has_only_outdated_comments;
        }

        true
    }

    fn render_outdated_section(
        &self,
        outdated_comments: &[&CommentDisplayState],
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let yellow_border: ColorU = theme.terminal_colors().normal.yellow.into();
        let is_collapsed = self.outdated_section_should_be_collapsed(ctx);
        let count = outdated_comments.len();

        let mut outdated_column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        outdated_column.add_child(self.render_outdated_section_header(
            count,
            is_collapsed,
            appearance,
        ));

        if !is_collapsed {
            for comment_render_state in outdated_comments {
                outdated_column.add_child(
                    Container::new(self.render_comment(comment_render_state, ctx))
                        .with_margin_top(8.)
                        .with_horizontal_margin(8.)
                        .finish(),
                );
            }
        }

        let mut outdated_section = Container::new(outdated_column.finish())
            .with_border(Border::all(1.).with_border_fill(Fill::Solid(yellow_border)))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_bottom(8.);
        if !is_collapsed {
            outdated_section = outdated_section.with_padding_bottom(8.)
        }
        outdated_section.finish()
    }

    fn render_outdated_section_header(
        &self,
        count: usize,
        is_collapsed: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let yellow_border: ColorU = theme.terminal_colors().normal.yellow.into();
        let header_text = outdated_section_header_text(count);

        Hoverable::new(
            self.view_state.outdated_chevron_mouse_state.clone(),
            move |mouse_state| {
                let icon = if is_collapsed {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronDown
                };

                let icon_element = icon
                    .to_warpui_icon(warp_core::ui::theme::Fill::Solid(text_sub(
                        theme,
                        neutral_1(theme),
                    )))
                    .finish();

                let icon_container = Container::new(
                    ConstrainedBox::new(icon_element)
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                )
                .with_margin_right(8.)
                .finish();

                let text = Text::new(
                    header_text.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(text_main(theme, neutral_1(theme)))
                .finish();

                let row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icon_container)
                    .with_child(text)
                    .finish();

                let mut header_container = Container::new(row)
                    .with_vertical_padding(8.)
                    .with_horizontal_padding(8.)
                    .with_background(neutral_1(theme));

                if !is_collapsed {
                    header_container = header_container
                        .with_border(Border::bottom(1.).with_border_color(yellow_border));
                }

                let radius = Radius::Pixels(4.);
                let header_corners = if is_collapsed {
                    CornerRadius::with_all(radius)
                } else {
                    CornerRadius::with_top(radius)
                };

                if mouse_state.is_hovered() {
                    header_container = header_container.with_background_color(neutral_1(theme))
                }

                header_container.with_corner_radius(header_corners).finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CommentListAction::ToggleOutdatedCollapsed);
        })
        .finish()
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        header_row.add_child(self.render_header_left(appearance));
        header_row.add_child(self.render_header_right(appearance));

        Container::new(Clipped::new(Shrinkable::new(1., header_row.finish()).finish()).finish())
            .with_background(neutral_3(theme))
            .with_vertical_padding(8.)
            .with_horizontal_padding(16.)
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
            .with_border(
                Border::new(1.)
                    .with_sides(
                        true,  /* top */
                        true,  /* left */
                        false, /* bottom */
                        true,  /* right */
                    )
                    .with_border_fill(warp_core::ui::theme::Fill::Solid(neutral_4(theme))),
            )
            .finish()
    }

    fn render_header_left(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut left_section = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        left_section.add_child(self.render_visibility_toggle(appearance));
        left_section.add_child(ChildView::new(&self.comments_button).finish());

        let outdated_count = self
            .comments_by_id
            .values()
            .filter(|state| state.card.source().outdated)
            .count();

        if outdated_count > 0 {
            let circle_separator = Container::new(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background(Fill::Solid(neutral_6(theme)))
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .finish(),
                )
                .with_width(4.)
                .with_height(4.)
                .finish(),
            )
            .with_horizontal_margin(6.)
            .finish();

            let outdated_text = Text::new(
                format!("{outdated_count} outdated"),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(text_sub(theme, neutral_3(theme)))
            .finish();

            left_section.add_child(circle_separator);
            left_section.add_child(outdated_text);
        }

        left_section.finish()
    }

    fn render_visibility_toggle(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let is_collapsed = self.is_collapsed;
        Hoverable::new(
            self.view_state.chevron_mouse_state.clone(),
            move |mouse_state| {
                let icon = if is_collapsed {
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

                let container = Container::new(
                    ConstrainedBox::new(icon_element)
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                )
                .with_margin_right(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

                let container = if mouse_state.is_hovered() {
                    container.with_background(theme.surface_3())
                } else {
                    container
                };

                container.finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CommentListAction::ToggleCollapsed);
        })
        .finish()
    }

    fn render_header_right(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut right_section = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        right_section.add_child(self.render_cancel_button(appearance));
        right_section.add_child(ChildView::new(&self.send_button).finish());
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
            ctx.dispatch_typed_action(CommentListAction::Cancel);
            DispatchEventResult::StopPropagation
        })
        .finish();
        Container::new(cancel_button).with_margin_right(8.).finish()
    }

    fn has_non_outdated_comments(&self) -> bool {
        self.comments_by_id
            .values()
            .any(|state| !state.card.source().outdated)
    }

    /// Whether the queued review comments can currently be sent to an agent.
    pub fn can_send(&self, ctx: &AppContext) -> bool {
        let has_sendable_comments = self.has_non_outdated_comments();
        match &self.review_destination {
            ReviewDestination::None => false,
            // CLI agents don't consume AI credits, so bypass the ai check.
            ReviewDestination::Cli(_) => has_sendable_comments,
            ReviewDestination::Warp => {
                AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx) && has_sendable_comments
            }
        }
    }

    /// Keep the stored "Send to Agent" button's enabled state and tooltip in sync with the current
    /// destination / comment / AI-availability state.
    fn sync_send_button(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_available = AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx);
        let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let enabled = self.can_send(ctx);
        let tooltip = Self::send_button_tooltip_text(
            &self.review_destination,
            self.has_non_outdated_comments(),
            ai_available,
            ai_enabled,
        )
        .into_owned();
        self.send_button.update(ctx, |button, ctx| {
            button.set_disabled(!enabled, ctx);
            button.set_tooltip(Some(tooltip), ctx);
        });
    }

    /// Computes the tooltip text for the send button based on current state.
    fn send_button_tooltip_text(
        destination: &ReviewDestination,
        has_sendable_comments: bool,
        ai_available: bool,
        ai_enabled: bool,
    ) -> Cow<'static, str> {
        if let ReviewDestination::Cli(agent) = destination {
            if !has_sendable_comments {
                Cow::Borrowed("No non-outdated comments to send")
            } else {
                let cmd = agent.command_prefix();
                let label = if cmd.is_empty() { "CLI agent" } else { cmd };
                Cow::Owned(format!("Send diff comments to {label}"))
            }
        } else if !ai_enabled {
            Cow::Borrowed("AI must be enabled to send comments to Agent")
        } else if !ai_available {
            Cow::Borrowed("Agent code review requires AI credits")
        } else if matches!(destination, ReviewDestination::None) {
            Cow::Borrowed("All terminals are busy")
        } else if !has_sendable_comments {
            Cow::Borrowed("No non-outdated comments to send")
        } else {
            Cow::Borrowed("Send diff comments to Agent")
        }
    }

    fn render_comment(
        &self,
        comment_state: &CommentDisplayState,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let editor_lens_element = self.editor_lens_for_card(&comment_state.card, ctx);

        let overflow_menu_button = SavePosition::new(
            ChildView::new(&comment_state.icon_button).finish(),
            &comment_state.save_position_id(),
        )
        .finish();

        let comment_container = comment_state.card.render(
            editor_lens_element,
            None,
            Some(overflow_menu_button),
            None,
            ctx,
        );

        let clickable = !comment_state.card.source().outdated
            && matches!(
                comment_state.card.source().target,
                AttachedReviewCommentTarget::Line { .. } | AttachedReviewCommentTarget::File { .. }
            );

        let comment = if clickable {
            let comment_id = comment_state.card.source().id;
            Hoverable::new(comment_state.mouse_state.clone(), |_mouse_state| {
                comment_container
            })
            .on_click(move |event_ctx, _, _| {
                event_ctx
                    .dispatch_typed_action(CommentListAction::JumpToCommentLocation(comment_id))
            })
            .with_defer_events_to_children()
            .finish()
        } else {
            comment_container
        };

        SavePosition::new(comment, &comment_state.comment_position_id()).finish()
    }

    fn editor_lens_for_card(
        &self,
        card: &CommentViewCard,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !card.uses_editor_lens() {
            return None;
        }
        if let AttachedReviewCommentTarget::Line {
            absolute_file_path,
            line,
            ..
        } = &card.source().target
        {
            let parent = self.parent.upgrade(ctx)?.as_ref(ctx);
            let location = line.clone();
            parent.editor_lens_for_location(absolute_file_path, location.clone()..location, ctx)
        } else {
            None
        }
    }

    fn close_overflow_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_overflow_comment_id.take();
        ctx.notify();
    }

    fn menu_items_for_comment(
        is_file_level: bool,
        is_outdated: bool,
        html_url: Option<&str>,
        appearance: &Appearance,
    ) -> Vec<MenuItem<CommentListAction>> {
        let mut items = vec![MenuItemFields::new("Copy text")
            .with_icon(Icon::Copy)
            .with_on_select_action(CommentListAction::CopyCommentText)
            .into_item()];

        let mut edit_item = MenuItemFields::new("Edit")
            .with_icon(Icon::Pencil)
            .with_on_select_action(CommentListAction::EditComment);
        if is_file_level || is_outdated {
            let tooltip_text = if is_file_level {
                "File-level comments currently can't be edited."
            } else {
                "Outdated comments can't be edited."
            };
            edit_item = edit_item.with_disabled(true).with_tooltip(tooltip_text);
        }
        items.push(edit_item.into_item());

        if let Some(url) = html_url {
            items.push(
                MenuItemFields::new("View in GitHub")
                    .with_icon(Icon::Github)
                    .with_on_select_action(CommentListAction::ViewInGitHub {
                        url: url.to_string(),
                    })
                    .into_item(),
            );
        }

        items.push(
            MenuItemFields::new("Remove")
                .with_icon(Icon::Trash)
                .with_override_text_color(Fill::Solid(appearance.theme().ansi_fg_red()))
                .with_override_icon_color(Fill::Solid(appearance.theme().ansi_fg_red()))
                .with_on_select_action(CommentListAction::DeleteComment)
                .into_item(),
        );

        items
    }
}

impl Entity for CommentListView {
    type Event = CommentListEvent;
}

impl View for CommentListView {
    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        if self.comments_by_id.is_empty() {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(ctx);

        if self.is_collapsed {
            self.render_header(appearance)
        } else {
            let mut panel = self.render_panel(appearance, ctx);

            if let Some(comment) = self
                .active_overflow_comment_id
                .and_then(|id| self.comments_by_id.get(&id))
            {
                let mut stack = Stack::new().with_child(panel);
                stack.add_positioned_child(
                    Dismiss::new(ChildView::new(&self.overflow_menu).finish())
                        .on_dismiss(|ctx, _app| {
                            ctx.dispatch_typed_action(CommentListAction::DismissOverflowMenu)
                        })
                        .prevent_interaction_with_other_elements()
                        .finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        comment.save_position_id(),
                        vec2f(0., 1.),
                        PositionedElementOffsetBounds::ParentByPosition,
                        PositionedElementAnchor::BottomRight,
                        ChildAnchor::TopRight,
                    ),
                );

                panel = stack.finish();
            }

            Resizable::new(self.view_state.resizable_state.clone(), panel)
                .with_dragbar_side(DragBarSide::Top)
                .with_dragbar_color(warpui::elements::Fill::Solid(
                    warpui::color::ColorU::transparent_black(),
                ))
                .with_bounds_callback(Box::new(|window_size| (100.0, window_size.y() * 0.8)))
                .on_resize(|ctx, _| {
                    ctx.notify();
                })
                .finish()
        }
    }

    fn ui_name() -> &'static str {
        "CommentListView"
    }
}

impl TypedActionView for CommentListView {
    type Action = CommentListAction;

    fn handle_action(&mut self, action: &CommentListAction, ctx: &mut ViewContext<Self>) {
        match action {
            CommentListAction::ToggleCollapsed => {
                self.is_collapsed = !self.is_collapsed;

                // If the comment list is now open, recompute the last updated timestamps.
                if !self.is_collapsed {
                    for comment in self.comments_by_id.values_mut() {
                        comment.card.refresh_last_updated_duration();
                    }

                    // Telemetry: comment list view expanded.
                    send_telemetry_from_ctx!(
                        CodeReviewTelemetryEvent::CommentListExpanded {
                            is_local: self.repo_is_local(),
                            comment_count: self.comments_by_id.len(),
                        },
                        ctx
                    );
                }
                ctx.notify();
            }
            CommentListAction::ToggleOutdatedCollapsed => {
                self.is_outdated_section_collapsed =
                    Some(!self.outdated_section_should_be_collapsed(ctx));
                ctx.notify();
            }
            CommentListAction::Cancel => {
                ctx.emit(CommentListEvent::Cancelled);
            }
            CommentListAction::Submit => {
                if self.can_send(ctx) {
                    ctx.emit(CommentListEvent::Submitted);
                }
            }
            CommentListAction::ShowOverflow { comment_id } => {
                let current_overflow = self.active_overflow_comment_id.take();
                if current_overflow != Some(*comment_id) {
                    self.active_overflow_comment_id = Some(*comment_id);

                    // File-level comments cannot be edited (no line to jump to).
                    // Outdated comments cannot be edited.
                    let (is_file_level, is_outdated, html_url) = self
                        .comments_by_id
                        .get(comment_id)
                        .map(|state| {
                            let source = state.card.source();
                            let html_url = match &source.origin {
                                CommentOrigin::ImportedFromGitHub(details) => {
                                    details.html_url.clone()
                                }
                                CommentOrigin::Native => None,
                            };
                            (
                                matches!(source.target, AttachedReviewCommentTarget::File { .. }),
                                source.outdated,
                                html_url,
                            )
                        })
                        .unwrap_or((true, true, None));

                    self.overflow_menu.update(ctx, |menu, ctx| {
                        let appearance = Appearance::handle(ctx).as_ref(ctx);
                        menu.set_items(
                            Self::menu_items_for_comment(
                                is_file_level,
                                is_outdated,
                                html_url.as_deref(),
                                appearance,
                            ),
                            ctx,
                        );
                    });
                }
                ctx.notify();
            }
            CommentListAction::DeleteComment => {
                if let Some(id) = self.active_overflow_comment_id.take() {
                    ctx.emit(CommentListEvent::DeleteComment { comment_id: id })
                }
                ctx.notify();
            }
            CommentListAction::DismissOverflowMenu => self.close_overflow_menu(ctx),
            CommentListAction::CopyCommentText => {
                if let Some(id) = self.active_overflow_comment_id.take() {
                    if let Some(state) = self.comments_by_id.get(&id) {
                        let content = state.card.source().content.clone();
                        let mut clipboard = ClipboardContent::plain_text(content.clone());
                        clipboard.html = markdown_to_html(state.card.comment_editor(), ctx);
                        ctx.clipboard().write(clipboard);
                    }
                }
                ctx.notify();
            }
            CommentListAction::EditComment => {
                if let Some(id) = self.active_overflow_comment_id.take() {
                    ctx.emit(CommentListEvent::EditComment(id));
                }
                ctx.notify();
            }
            CommentListAction::ViewInGitHub { url } => {
                ctx.open_url(url);
                self.close_overflow_menu(ctx);
            }
            CommentListAction::JumpToCommentLocation(comment_id) => {
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::CommentListItemClicked {
                        is_local: self.repo_is_local(),
                    },
                    ctx
                );
                ctx.emit(CommentListEvent::JumpToCommentLocation(*comment_id));
            }
        }
    }
}

struct CustomSecondaryActionTheme;

impl ActionButtonTheme for CustomSecondaryActionTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        let background = Fill::Solid(neutral_3(appearance.theme()));
        if hovered {
            Some(background.blend(&accent_overlay_3(appearance.theme())))
        } else {
            Some(background.blend(&accent_overlay_2(appearance.theme())))
        }
    }

    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        let theme = appearance.theme();
        theme
            .font_color(
                background
                    .or(self.background(hovered, appearance))
                    .unwrap_or(theme.background()),
            )
            .into_solid()
    }

    fn adjoined_side_border(&self, appearance: &Appearance) -> Option<ColorU> {
        SecondaryTheme.adjoined_side_border(appearance)
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(appearance.theme().accent().into_solid())
    }

    fn keyboard_shortcut_border(
        &self,
        text_color: ColorU,
        appearance: &Appearance,
    ) -> Option<ColorU> {
        SecondaryTheme.keyboard_shortcut_border(text_color, appearance)
    }

    fn keyboard_shortcut_background(&self, appearance: &Appearance) -> Option<ColorU> {
        SecondaryTheme.keyboard_shortcut_background(appearance)
    }
}

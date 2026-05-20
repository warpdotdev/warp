use std::sync::Arc;

use parking_lot::RwLock;
use warp_core::features::FeatureFlag;
use warp_core::semantic_selection::SemanticSelection;
use warpui::{
    elements::{
        get_rich_content_position_id, ChildView, Container, CrossAxisAlignment, Expanded, Flex,
        ParentElement, SavePosition, SelectableArea, SelectionHandle, Text,
    },
    fonts::{Properties, Style, Weight},
    AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    ai::blocklist::block::view_impl::{
        common::render_user_avatar, CONTENT_HORIZONTAL_PADDING, CONTENT_ITEM_VERTICAL_MARGIN,
    },
    appearance::Appearance,
    terminal::{block_list_element::BlockListMenuSource, view::TerminalAction},
    ui_components::{blended_colors, icons::Icon},
    view_components::action_button::{ActionButton, ButtonSize, NakedTheme},
};

/// Renders a pending user query block with dimmed text and a "Queued" badge.
/// Displayed when a follow-up prompt is queued via `/fork-and-compact <prompt>`,
/// `/compact-and <prompt>`, `/queue <prompt>`, or for the initial prompt of a
/// Cloud Mode run waiting for its real shared-session transcript query to arrive.
pub struct PendingUserQueryBlock {
    prompt: String,
    user_display_name: String,
    profile_image_path: Option<String>,
    view_id: EntityId,
    selection_handle: SelectionHandle,
    /// In an `RwLock` so the `SelectableArea` can update it synchronously when a selection ends,
    /// allowing the terminal view to read the value immediately for copy-on-select.
    selected_text: Arc<RwLock<Option<String>>>,
    close_button: Option<ViewHandle<ActionButton>>,
    send_now_button: Option<ViewHandle<ActionButton>>,
}

impl PendingUserQueryBlock {
    pub fn new(
        prompt: String,
        user_display_name: String,
        profile_image_path: Option<String>,
        show_close_button: bool,
        show_send_now_button: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let close_button = show_close_button.then(|| {
            ctx.add_typed_action_view(|_| {
                ActionButton::new("Remove queued prompt", NakedTheme)
                    .with_icon(Icon::X)
                    .with_size(ButtonSize::XSmall)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(PendingUserQueryBlockAction::Dismiss);
                    })
            })
        });
        let send_now_button = show_send_now_button.then(|| {
            ctx.add_typed_action_view(|_| {
                ActionButton::new("Send now", NakedTheme)
                    .with_icon(Icon::Play)
                    .with_size(ButtonSize::XSmall)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(PendingUserQueryBlockAction::SendNow);
                    })
            })
        });
        Self {
            prompt,
            user_display_name,
            profile_image_path,
            view_id: ctx.view_id(),
            selection_handle: Default::default(),
            selected_text: Default::default(),
            close_button,
            send_now_button,
        }
    }

    /// Returns the currently selected prompt text within this block.
    pub fn selected_text(&self, _ctx: &AppContext) -> Option<String> {
        self.selected_text.read().clone()
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Clears the text selection state and visual selection highlight.
    pub fn clear_selection(&mut self, ctx: &mut ViewContext<Self>) {
        self.selection_handle.clear();
        *self.selected_text.write() = None;
        ctx.notify();
    }
}

#[derive(Clone, Debug)]
pub enum PendingUserQueryBlockAction {
    Dismiss,
    SendNow,
    SelectText,
}

pub enum PendingUserQueryBlockEvent {
    Dismissed,
    SendNow,
    TextSelected,
}

impl Entity for PendingUserQueryBlock {
    type Event = PendingUserQueryBlockEvent;
}

impl TypedActionView for PendingUserQueryBlock {
    type Action = PendingUserQueryBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PendingUserQueryBlockAction::Dismiss => {
                ctx.emit(PendingUserQueryBlockEvent::Dismissed);
            }
            PendingUserQueryBlockAction::SendNow => {
                ctx.emit(PendingUserQueryBlockEvent::SendNow);
            }
            PendingUserQueryBlockAction::SelectText => {
                ctx.emit(PendingUserQueryBlockEvent::TextSelected);
            }
        }
    }
}

impl View for PendingUserQueryBlock {
    fn ui_name() -> &'static str {
        "PendingUserQueryBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let dimmed_color = blended_colors::text_sub(theme, theme.surface_1());

        let avatar = Container::new(render_user_avatar(
            &self.user_display_name,
            self.profile_image_path.as_ref(),
            None,
            app,
        ))
        .with_margin_right(16.)
        .finish();

        let properties = Properties {
            style: Style::Normal,
            weight: Weight::Bold,
        };

        let prompt_text = Text::new(
            self.prompt.clone(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_style(properties)
        .with_color(dimmed_color)
        .with_selectable(true)
        .finish();

        let queued_badge = Text::new(
            "Queued",
            appearance.ui_font_family(),
            appearance.monospace_font_size().max(4.) - 2.,
        )
        .with_style(Properties {
            style: Style::Italic,
            weight: Weight::Normal,
        })
        .with_color(dimmed_color)
        .with_selectable(false)
        .finish();

        let selectable_child = Flex::column()
            .with_child(prompt_text)
            .with_child(Container::new(queued_badge).with_margin_top(4.).finish())
            .finish();

        let semantic_selection = SemanticSelection::as_ref(app);
        let selected_text = self.selected_text.clone();
        let view_id = self.view_id;
        let mut text_column = SelectableArea::new(
            self.selection_handle.clone(),
            move |selection_args, _, _| {
                *selected_text.write() = selection_args.selection;
            },
            SavePosition::new(
                selectable_child,
                get_rich_content_position_id(&view_id).as_str(),
            )
            .finish(),
        )
        .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
        .with_smart_select_fn(semantic_selection.smart_select_fn())
        .on_selection_updated(|ctx, _| {
            ctx.dispatch_typed_action(PendingUserQueryBlockAction::SelectText)
        })
        .on_selection_right_click(move |ctx, position| {
            ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(
                BlockListMenuSource::RichContentTextRightClick {
                    rich_content_view_id: view_id,
                    position_in_rich_content: position,
                },
            ))
        });

        if FeatureFlag::RectSelection.is_enabled() {
            text_column = text_column.should_support_rect_select();
        }

        let mut buttons_column = Flex::column().with_spacing(2.);
        if let Some(close_button) = &self.close_button {
            buttons_column.add_child(ChildView::new(close_button).finish());
        }
        if let Some(send_now_button) = &self.send_now_button {
            buttons_column.add_child(ChildView::new(send_now_button).finish());
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(avatar)
            .with_child(Expanded::new(1., text_column.finish()).finish());
        if self.close_button.is_some() || self.send_now_button.is_some() {
            let buttons = Container::new(buttons_column.finish())
                .with_margin_left(8.)
                .finish();
            row.add_child(buttons);
        }
        let row = row.finish();

        Container::new(row)
            .with_horizontal_padding(CONTENT_HORIZONTAL_PADDING)
            .with_padding_top(CONTENT_ITEM_VERTICAL_MARGIN)
            .with_padding_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
            .finish()
    }
}

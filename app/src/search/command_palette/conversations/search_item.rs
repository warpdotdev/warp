use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::appearance::Appearance;
use crate::search::command_palette::conversations::search::MatchedConversation;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::command_palette::view::Action;
use crate::search::item::IconLocation;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::SearchItem;
use crate::ui_components::buttons::icon_button;
use crate::util::time_format::format_approx_duration_from_now;
use ordered_float::OrderedFloat;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::color::{blend::Blend, coloru_with_opacity};
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    AnchorPair, Container, CrossAxisAlignment, Expanded, Fill, Flex, Highlight, MainAxisSize,
    MouseStateHandle, OffsetPositioning, OffsetType, ParentElement, ParentOffsetBounds,
    PositioningAxis, Stack, Text, XAxisAnchor, YAxisAnchor,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::button::ButtonTooltipPosition;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Gradient, SingletonEntity};

/// Information about which action to take once the conversation item is accepted.
#[derive(Debug)]
pub enum ConversationAction {
    /// Start a new conversation in the current view.
    New,
    /// Fork the current active conversation into a new view.
    Fork {
        conversation_id: AIConversationId,
        title: String,
    },
    /// Resume the matched conversation in its associated view.
    Resume(Box<MatchedConversation>),
}

/// Search item to render a conversation within the command palette.
/// When matched_conversation is None, we render this as a new conversation item.
#[derive(Debug)]
pub struct ConversationSearchItem {
    action_info: ConversationAction,
    action_button_mouse_state: MouseStateHandle,
}

impl ConversationSearchItem {
    pub fn new(action_info: ConversationAction) -> Self {
        Self {
            action_info,
            action_button_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Renders the new conversation item for the command palette.
    pub fn render_new_conversation_action_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        Flex::row()
            .with_child(
                Text::new_inline(
                    "New conversation",
                    appearance.ui_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(highlight_state.sub_text_fill(appearance).into_solid())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }

    pub fn render_fork_conversation_action_item(
        &self,
        highlight_state: ItemHighlightState,
        conversation_title: &str,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let action_title = Text::new_inline(
            "Fork current conversation",
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold));

        let conversation_title = Text::new_inline(
            conversation_title.to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        Flex::column()
            .with_child(action_title.finish())
            .with_child(conversation_title.finish())
            .with_spacing(4.)
            .finish()
    }

    fn render_matched_conversation_item(
        &self,
        matched_conversation: &MatchedConversation,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let conversation = matched_conversation.conversation.clone();
        let sub_text_font_size = appearance.monospace_font_size() - 2.;

        let mut conversation_title_element = Text::new_inline(
            conversation.title().to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold));

        let mut working_directory_element = Text::new_inline(
            conversation
                .initial_working_directory
                .clone()
                .unwrap_or_default(),
            appearance.ui_font_family(),
            sub_text_font_size,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        // When the search query is empty, we only show the conversation's title and working directory.
        // Otherwise, we show the conversation's title, initial user query, and working directory.
        // We also highlight the indices in those elements that match the search query.
        let mut left_container = Flex::column().with_spacing(4.);
        if !self.query_is_empty() {
            // The first user query that was submitted for this conversation.
            let mut initial_query_element = Text::new_inline(
                conversation.initial_query.clone().unwrap_or_default(),
                appearance.ui_font_family(),
                sub_text_font_size,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid());

            // Apply highlights for the search query's matching indices.
            let highlight = Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid());
            let highlight_indices = matched_conversation.highlight_indices();
            if !highlight_indices.title_indices().is_empty() {
                conversation_title_element = conversation_title_element
                    .with_single_highlight(highlight, highlight_indices.title_indices().clone());
            }
            if !highlight_indices.initial_query_indices().is_empty() {
                initial_query_element = initial_query_element.with_single_highlight(
                    highlight,
                    highlight_indices.initial_query_indices().clone(),
                );
            }
            if !highlight_indices.working_directory_indices().is_empty() {
                working_directory_element = working_directory_element.with_single_highlight(
                    highlight,
                    highlight_indices.working_directory_indices().clone(),
                );
            }

            // Add the conversation title and initial user query to the left container.
            left_container = left_container
                .with_child(conversation_title_element.finish())
                .with_child(initial_query_element.finish());
        } else {
            // When the search query is empty, we only show the conversation's title and working directory.
            left_container = left_container.with_child(conversation_title_element.finish());
        }
        // In all cases, we show the conversation's working directory last.
        left_container = left_container.with_child(working_directory_element.finish());

        let last_updated = format_approx_duration_from_now(conversation.last_updated());
        let last_updated_element = Container::new(
            Text::new_inline(
                last_updated,
                appearance.ui_font_family(),
                sub_text_font_size,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid())
            .finish(),
        )
        .with_padding_left(8.)
        .finish();

        let search_item_content = Flex::row()
            .with_child(Expanded::new(1.0, left_container.finish()).finish())
            .with_child(last_updated_element)
            .with_main_axis_size(MainAxisSize::Max)
            .finish();

        // We only want to show the fork button if the conversation is completed
        // (i.e. the agent has finished responding and there are no blocked commands).
        let conversation_is_done = BlocklistAIHistoryModel::as_ref(app)
            .conversation(&conversation.id())
            .map(|c| c.status().is_done())
            .unwrap_or(true);

        if highlight_state.is_hovered() && conversation_is_done && !cfg!(target_family = "wasm") {
            // Base row content (unchanged layout for existing children)
            let base_row = Flex::row()
                .with_child(Expanded::new(1.0, search_item_content).finish())
                .with_main_axis_size(MainAxisSize::Max)
                .finish();

            // Overlay fork button on the right, positioned absolutely so it doesn't affect the layout.
            let fork_button_positioning = OffsetPositioning::from_axes(
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            );

            // We create a gradient background that is semi-transparent on the left and the item background color on the right.
            // The end color is the highlight_bg_color over surface_2 at the given highlight state's opacity.
            // The start color is fully transparent.
            let base_bg = appearance.theme().surface_2().into_solid();
            let end_color = base_bg.blend(&coloru_with_opacity(
                Fill::from(appearance.theme().accent()).start_color(),
                highlight_state.container_background_opacity(),
            ));
            let start_color = ColorU::new(end_color.r, end_color.g, end_color.b, 0);

            let fork_button_tool_tip = appearance
                .ui_builder()
                .tool_tip(crate::i18n::tr_static(app, "Fork conversation").to_string())
                .build();

            let fork_button_inner = icon_button(
                appearance,
                Icon::ArrowSplit,
                false,
                self.action_button_mouse_state.clone(),
            )
            .with_hovered_styles(
                UiComponentStyles::default()
                    .set_background(internal_colors::fg_overlay_3(appearance.theme()).into()),
            )
            .with_clicked_styles(
                UiComponentStyles::default()
                    .set_background(internal_colors::fg_overlay_5(appearance.theme()).into()),
            )
            .with_tooltip(|| fork_button_tool_tip.finish())
            .with_tooltip_position(ButtonTooltipPosition::AboveRight)
            .build()
            .on_click(move |ctx, _app, _pos| {
                ctx.dispatch_typed_action(Action::ResultClicked {
                    action: CommandPaletteItemAction::ForkConversation {
                        conversation_id: conversation.id(),
                    },
                });
            })
            .finish();

            // When the fork button itself is hovered, we use a solid background equal to the
            // gradient's end color. Otherwise, we use the original gradient.
            let is_hovered = self
                .action_button_mouse_state
                .lock()
                .map(|s| s.is_hovered())
                .unwrap_or(false);
            let fork_button = if is_hovered {
                Container::new(fork_button_inner)
                    .with_background_color(end_color)
                    .finish()
            } else {
                Container::new(fork_button_inner)
                    .with_background_gradient(
                        vec2f(0.0, 0.0),
                        vec2f(0.2, 0.0),
                        Gradient {
                            start: start_color,
                            end: end_color,
                        },
                    )
                    .finish()
            };

            let mut stack = Stack::new().with_child(base_row);
            stack.add_positioned_child(fork_button, fork_button_positioning);
            stack.finish()
        } else {
            search_item_content
        }
    }

    fn query_is_empty(&self) -> bool {
        match &self.action_info {
            ConversationAction::Resume(matched_conversation) => {
                // If the score is empty, the query must be empty (otherwise, we would not be showing this item)
                matched_conversation.as_ref().match_result.score() == 0
            }
            ConversationAction::Fork { .. } | ConversationAction::New => {
                // We only show these items when the search query is empty.
                true
            }
        }
    }
}

impl SearchItem for ConversationSearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        true
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (color, icon) = match &self.action_info {
            ConversationAction::Resume(..) => (
                appearance.theme().foreground().into_solid(),
                Icon::Conversation,
            ),
            ConversationAction::New => (appearance.theme().foreground().into_solid(), Icon::Plus),
            ConversationAction::Fork { .. } => (
                appearance.theme().foreground().into_solid(),
                Icon::ArrowSplit,
            ),
        };

        render_search_item_icon(appearance, icon, color, highlight_state)
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        if matches!(self.action_info, ConversationAction::New) {
            IconLocation::Centered
        } else {
            // The icon has the size of the monospace font, whereas the text has a height of
            // `line_height_ratio * font_size`. Offset the icon by this difference so it is rendered
            // centered with the text.
            let margin_top = (appearance.line_height_ratio() * appearance.monospace_font_size())
                - appearance.monospace_font_size();
            IconLocation::Top { margin_top }
        }
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        match &self.action_info {
            ConversationAction::Resume(matched_conversation) => self
                .render_matched_conversation_item(
                    matched_conversation.as_ref(),
                    highlight_state,
                    app,
                ),
            ConversationAction::New => {
                self.render_new_conversation_action_item(highlight_state, app)
            }
            ConversationAction::Fork { title, .. } => {
                self.render_fork_conversation_action_item(highlight_state, title, app)
            }
        }
    }

    fn score(&self) -> OrderedFloat<f64> {
        let score = match &self.action_info {
            ConversationAction::Resume(matched_conversation) => matched_conversation.score() as f64,
            ConversationAction::Fork { .. } => f64::NAN,
            ConversationAction::New => f64::NAN,
        };
        OrderedFloat::from(score)
    }

    fn accept_result(&self) -> Self::Action {
        match &self.action_info {
            ConversationAction::Resume(matched_conversation) => {
                let conversation = &matched_conversation.as_ref().conversation;
                CommandPaletteItemAction::NavigateToConversation {
                    pane_view_locator: conversation.pane_view_locator(),
                    window_id: conversation.window_id(),
                    conversation_id: conversation.id(),
                    terminal_view_id: conversation.terminal_view_id,
                }
            }
            ConversationAction::Fork {
                conversation_id, ..
            } => CommandPaletteItemAction::ForkConversation {
                conversation_id: *conversation_id,
            },
            ConversationAction::New => CommandPaletteItemAction::NewConversation,
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        match &self.action_info {
            ConversationAction::Resume(matched_conversation) => {
                format!(
                    "Conversation: {}",
                    matched_conversation.as_ref().conversation.title()
                )
            }
            ConversationAction::Fork { title, .. } => {
                format!("Fork current conversation ({title})")
            }
            ConversationAction::New => "New conversation".to_string(),
        }
    }

    fn accessibility_help_message(&self) -> Option<String> {
        match &self.action_info {
            ConversationAction::Resume(matched_conversation) => Some(format!(
                "Press enter to navigate to conversation \"{}\".",
                matched_conversation.as_ref().conversation.title()
            )),
            ConversationAction::Fork { .. } => {
                Some("Press enter to fork the current conversation into a new conversation.".into())
            }
            ConversationAction::New => Some("Press enter to create a new conversation.".into()),
        }
    }
}

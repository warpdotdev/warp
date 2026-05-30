use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use super::ConversationContextItem;
use crate::appearance::Appearance;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::styles;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::util::time_format::format_approx_duration_from_now_utc;
use crate::util::truncation::truncate_from_end;

const MAX_TITLE_LENGTH: usize = 45;

#[derive(Debug)]
pub(super) struct ConversationSearchItem {
    item: ConversationContextItem,
    match_result: FuzzyMatchResult,
}

impl ConversationSearchItem {
    pub fn new(item: ConversationContextItem, match_result: FuzzyMatchResult) -> Self {
        Self { item, match_result }
    }
}

impl SearchItem for ConversationSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/conversation.svg",
                    highlight_state.icon_fill(appearance).into_solid(),
                )
                .finish(),
            )
            .with_width(styles::ICON_SIZE)
            .with_height(styles::ICON_SIZE)
            .finish(),
        )
        .with_margin_right(styles::MARGIN_RIGHT)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let char_count = self.item.title.chars().count();
        let highlight_limit = if char_count > MAX_TITLE_LENGTH {
            MAX_TITLE_LENGTH.saturating_sub(1)
        } else {
            char_count
        };
        let title = truncate_from_end(&self.item.title, MAX_TITLE_LENGTH);

        let mut name_text = Text::new(
            title,
            appearance.ui_font_family(),
            (appearance.monospace_font_size() - 1.0).max(1.0),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if !self.match_result.matched_indices.is_empty() {
            let filtered_indices: Vec<usize> = self
                .match_result
                .matched_indices
                .iter()
                .copied()
                .filter(|&i| i < highlight_limit)
                .collect();
            if !filtered_indices.is_empty() {
                name_text = name_text.with_single_highlight(
                    Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                    filtered_indices,
                );
            }
        }

        let timestamp_text = Text::new(
            format_approx_duration_from_now_utc(self.item.last_updated),
            appearance.ui_font_family(),
            (appearance.monospace_font_size() - 2.0).max(1.0),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        Flex::row()
            .with_child(name_text.finish())
            .with_child(
                Container::new(timestamp_text.finish())
                    .with_padding_left(6.)
                    .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        AIContextMenuSearchableAction::InsertConversation {
            conversation_id: self.item.server_conversation_token.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Conversation: {}", self.item.title)
    }
}

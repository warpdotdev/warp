//! SearchItem implementation for user query menu items.

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::Icon;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::Fill;
use warpui::elements::{ConstrainedBox, Container, Highlight, Shrinkable, Text};
use warpui::fonts::{Properties, Weight};
use warpui::scene::{CornerRadius, Radius};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::agent::AIAgentExchangeId;
use crate::appearance::Appearance;
use crate::search::{ItemHighlightState, SearchItem};
use crate::terminal::input::inline_menu::styles::{
    ICON_MARGIN, ITEM_CORNER_RADIUS, ITEM_HORIZONTAL_PADDING, font_size, icon_color,
    item_background, menu_background_color, primary_text_color,
};
use crate::terminal::input::user_query::data_source::SelectUserQuery;

const ICON_PADDING: f32 = 4.;

/// Search item for rendering a user query in the user query menu.
#[derive(Debug, Clone)]
pub struct UserQuerySearchItem {
    exchange_id: AIAgentExchangeId,
    query_text: String,
    query_match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
}

impl UserQuerySearchItem {
    pub fn new(exchange_id: AIAgentExchangeId, query_text: String) -> Self {
        Self {
            exchange_id,
            query_text,
            query_match_result: None,
            score: OrderedFloat(0.0),
        }
    }

    pub fn with_query_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.query_match_result = result;
        self
    }

    pub fn with_score(mut self, score: OrderedFloat<f64>) -> Self {
        self.score = score;
        self
    }
}

impl SearchItem for UserQuerySearchItem {
    type Action = SelectUserQuery;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let color = icon_color(appearance);

        // Once this search item is used by multiple sources,
        // we'll want to set this icon based on which source is generating this item.
        let icon = Container::new(
            ConstrainedBox::new(Icon::ArrowSplit.to_warpui_icon(color).finish())
                .with_width(appearance.monospace_font_size())
                .with_height(appearance.monospace_font_size())
                .finish(),
        )
        .with_uniform_padding(ICON_PADDING)
        .with_background(coloru_with_opacity(color.into(), 10))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ITEM_CORNER_RADIUS)))
        .finish();

        Container::new(icon).with_margin_right(ICON_MARGIN).finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let background = menu_background_color(app);

        let mut query_text = Text::new_inline(
            self.query_text.clone(),
            appearance.ui_font_family(),
            font_size(appearance),
        )
        .with_color(primary_text_color(theme, background.into()).into())
        .with_clip(ClipConfig::ellipsis());

        if let Some(match_result) = &self.query_match_result
            && !match_result.matched_indices.is_empty()
        {
            query_text = query_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                match_result.matched_indices.clone(),
            );
        }

        Container::new(Shrinkable::new(1., query_text.finish()).finish())
            .with_padding_right(ITEM_HORIZONTAL_PADDING)
            .finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Fill> {
        item_background(highlight_state, appearance)
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.score
    }

    fn accept_result(&self) -> Self::Action {
        SelectUserQuery {
            exchange_id: self.exchange_id,
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Query: {}", self.query_text)
    }
}

//! SearchItem implementation for rewind menu items.
//! Renders two lines: query text and code changes summary.

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::Icon;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::scene::{CornerRadius, Radius};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::agent::AIAgentExchangeId;
use crate::appearance::Appearance;
use crate::code::editor::{add_color, remove_color};
use crate::search::{ItemHighlightState, SearchItem};
use crate::terminal::input::inline_menu::styles::{
    ICON_MARGIN, ITEM_CORNER_RADIUS, ITEM_HORIZONTAL_PADDING, font_size, icon_color,
    item_background, menu_background_color, primary_text_color,
};
use crate::terminal::input::rewind::data_source::{FileChangesInfo, SelectRewindPoint};

const ICON_PADDING: f32 = 4.;

/// Search item for rendering a rewind point in the rewind menu.
#[derive(Debug, Clone)]
pub struct RewindSearchItem {
    /// The exchange ID to rewind to, or None for "Current".
    exchange_id: Option<AIAgentExchangeId>,
    query_text: String,
    file_changes: FileChangesInfo,
    query_match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
    is_current: bool,
}

impl RewindSearchItem {
    /// Create a "Current" item that dismisses the menu without rewinding.
    pub fn new_current() -> Self {
        Self {
            exchange_id: None,
            query_text: "Current".to_string(),
            file_changes: FileChangesInfo::default(),
            query_match_result: None,
            score: OrderedFloat(0.0),
            is_current: true,
        }
    }

    /// Create a rewind point item.
    pub fn new_rewind_point(
        exchange_id: AIAgentExchangeId,
        query_text: String,
        file_changes: FileChangesInfo,
    ) -> Self {
        Self {
            exchange_id: Some(exchange_id),
            query_text,
            file_changes,
            query_match_result: None,
            score: OrderedFloat(0.0),
            is_current: false,
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

    fn has_code_changes(&self) -> bool {
        self.file_changes.lines_added > 0 || self.file_changes.lines_removed > 0
    }
}

impl SearchItem for RewindSearchItem {
    type Action = SelectRewindPoint;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let color = icon_color(appearance);

        let icon = Container::new(
            ConstrainedBox::new(Icon::ClockRewind.to_warpui_icon(color).finish())
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
        let secondary_font_size = font_size(appearance) - 2.;

        // Line 1: Query text with fuzzy match highlighting
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

        // Line 2: Code changes summary
        let secondary_text_color: warpui::color::ColorU =
            theme.sub_text_color(theme.surface_1()).into();

        let changes_element: Box<dyn Element> = if self.is_current {
            // "Current" item shows "No code to be restored"
            Text::new_inline(
                "No code to be restored".to_string(),
                appearance.ui_font_family(),
                secondary_font_size,
            )
            .with_color(secondary_text_color)
            .finish()
        } else if self.has_code_changes() {
            // Format: "+{added} -{removed}"
            let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            row.add_child(
                Text::new_inline(
                    format!("+{}", self.file_changes.lines_added),
                    appearance.ui_font_family(),
                    secondary_font_size,
                )
                .with_color(add_color(appearance))
                .finish(),
            );

            row.add_child(
                Text::new_inline(
                    format!(" -{}", self.file_changes.lines_removed),
                    appearance.ui_font_family(),
                    secondary_font_size,
                )
                .with_color(remove_color(appearance))
                .finish(),
            );

            row.finish()
        } else {
            Text::new_inline(
                "No code to be restored".to_string(),
                appearance.ui_font_family(),
                secondary_font_size,
            )
            .with_color(secondary_text_color)
            .finish()
        };

        // Stack the two lines vertically
        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(query_text.finish())
            .with_child(Container::new(changes_element).with_margin_top(2.).finish())
            .finish();

        Container::new(content)
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
        SelectRewindPoint {
            exchange_id: self.exchange_id,
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        if self.is_current {
            "Current state (no rewind)".to_string()
        } else if self.has_code_changes() {
            format!(
                "Rewind to: {} (+{} -{})",
                self.query_text, self.file_changes.lines_added, self.file_changes.lines_removed
            )
        } else {
            format!("Rewind to: {} (no code changes)", self.query_text)
        }
    }
}

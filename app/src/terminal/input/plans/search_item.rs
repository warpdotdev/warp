//! SearchItem implementation for plan menu items.

use ai::document::AIDocumentId;
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::Icon;
use warp_core::ui::theme::Fill;
use warpui::elements::{ConstrainedBox, Container, Highlight, ParentElement, Shrinkable, Text};
use warpui::fonts::{Properties, Weight};
use warpui::prelude::{Align, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::document::ai_document_model::{AIDocument, AIDocumentVersion};
use crate::appearance::Appearance;
use crate::search::{ItemHighlightState, SearchItem};
use crate::terminal::input::inline_menu::styles as inline_styles;
use crate::terminal::input::plans::AcceptPlan;
use crate::util::time_format::format_approx_duration_from_now_utc;

const ICON_SIZE: f32 = 14.;

/// Search item for rendering a plan in the inline plan menu.
#[derive(Debug, Clone)]
pub(super) struct PlanSearchItem {
    document_id: AIDocumentId,
    title: String,
    version: AIDocumentVersion,
    created_at: chrono::DateTime<chrono::Local>,
    name_match_result: Option<FuzzyMatchResult>,
    /// Used as a stable ordering score (index in the list, or fuzzy match score).
    score: OrderedFloat<f64>,
}

impl PlanSearchItem {
    pub fn new(document_id: AIDocumentId, doc: AIDocument, index: usize) -> Self {
        Self {
            document_id,
            title: doc.title,
            version: doc.version,
            created_at: doc.created_at,
            name_match_result: None,
            score: OrderedFloat(index as f64),
        }
    }

    pub fn with_name_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.name_match_result = result;
        self
    }

    pub fn with_score(mut self, score: OrderedFloat<f64>) -> Self {
        self.score = score;
        self
    }
}

impl SearchItem for PlanSearchItem {
    type Action = AcceptPlan;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());

        let icon = Container::new(
            ConstrainedBox::new(Icon::Compass.to_warpui_icon(icon_color).finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish(),
        )
        .finish();

        Container::new(icon)
            .with_margin_right(inline_styles::ICON_MARGIN)
            .finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = inline_styles::font_size(appearance);
        let background_color = inline_styles::menu_background_color(app);

        let primary_text_color = inline_styles::primary_text_color(theme, background_color.into());
        let secondary_text_color = theme.disabled_text_color(background_color.into());

        let mut name_text =
            Text::new_inline(self.title.clone(), appearance.ui_font_family(), font_size)
                .with_color(primary_text_color.into())
                .with_clip(ClipConfig::ellipsis());

        if let Some(name_match) = &self.name_match_result
            && !name_match.matched_indices.is_empty()
        {
            name_text = name_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                name_match.matched_indices.clone(),
            );
        }

        let mut primary_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., name_text.finish()).finish());

        let timestamp_font_size = font_size * 12. / 14.;
        let timestamp = Text::new_inline(
            format_approx_duration_from_now_utc(self.created_at.to_utc()),
            appearance.ui_font_family(),
            timestamp_font_size,
        )
        .with_color(secondary_text_color.into())
        .finish();

        let max_timestamp_width = app
            .font_cache()
            .em_width(appearance.ui_font_family(), timestamp_font_size)
            * 10.;
        primary_row.add_child(
            ConstrainedBox::new(Align::new(timestamp).right().finish())
                .with_width(max_timestamp_width)
                .finish(),
        );

        primary_row.finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Fill> {
        inline_styles::item_background(highlight_state, appearance)
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.score
    }

    fn accept_result(&self) -> Self::Action {
        AcceptPlan {
            document_id: self.document_id,
            document_version: self.version,
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Plan: {}", self.title)
    }
}

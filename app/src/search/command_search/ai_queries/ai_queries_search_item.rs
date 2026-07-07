use chrono::{DateTime, Local};
use ordered_float::OrderedFloat;
use warp_core::ui::builder::MIN_FONT_SIZE;
use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, MainAxisAlignment,
    MainAxisSize, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::blocklist::AIQueryHistoryOutputStatus;
use crate::appearance::Appearance;
use crate::search::ai_queries::fuzzy_match::FuzzyMatchAIQueryResults;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::terminal::rich_history::{
    render_row_with_icon_and_paragraph, DETAILS_PARAGRAPH_SPACING,
};
use crate::ui_components::icons::Icon as UiIcon;
use crate::util::time_format::format_approx_duration_from_now;

/// Stores data needed to display an AI query search result item in Command Search.
#[derive(Clone, Debug)]
pub struct AIQuerySearchResultItem {
    /// The query text of the [`crate::ai::blocklist::AIQueryHistory`].
    pub query_text: String,
    /// When the query was originally submitted by the user.
    pub start_time: DateTime<Local>,
    /// The output status of the [`crate::ai::blocklist::AIQueryHistory`].
    pub output_status: AIQueryHistoryOutputStatus,
    /// The directory the AI query was submitted in.
    pub(crate) working_directory: Option<String>,
    // Match result on the [`crate::ai::blocklist::AIQueryHistory`]'s query text including its
    // score and matching string indices.
    pub fuzzy_match_results: FuzzyMatchAIQueryResults,
}

impl SearchItem for AIQuerySearchResultItem {
    type Action = CommandSearchItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(UiIcon::Prompt.into(), highlight_state.icon_fill(appearance)).finish(),
            )
            .with_width(appearance.ui_font_size())
            .with_height(appearance.ui_font_size())
            .finish(),
        )
        .with_margin_right(12.)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let query_text = Text::new_inline(
            self.query_text.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .autosize_text(MIN_FONT_SIZE)
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .with_single_highlight(
            Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
            self.fuzzy_match_results
                .query_text_match_result
                .matched_indices
                .clone(),
        )
        .finish();

        let query_text_col = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(query_text)
            .finish();

        let metadata_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                ConstrainedBox::new(
                    Icon::new(
                        self.output_status.icon().into(),
                        highlight_state.main_text_fill(appearance).into_solid(),
                    )
                    .finish(),
                )
                .with_max_height(appearance.ui_font_size())
                .with_max_width(appearance.ui_font_size())
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .span(format_approx_duration_from_now(self.start_time))
                    .with_style(UiComponentStyles {
                        margin: Some(Coords::uniform(0.).left(8.)),
                        font_color: Some(highlight_state.main_text_fill(appearance).into_solid()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., Align::new(query_text_col).left().finish()).finish())
            .with_child(metadata_row)
            .finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let ui_builder = appearance.ui_builder();

        let mut details_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(render_row_with_icon_and_paragraph(
                self.output_status.icon().into(),
                self.output_status.display_text(),
                appearance,
            ));

        if let Some(working_directory) = &self.working_directory {
            details_column.add_child(
                Container::new(render_row_with_icon_and_paragraph(
                    UiIcon::Folder.into(),
                    working_directory.clone(),
                    appearance,
                ))
                .with_margin_top(DETAILS_PARAGRAPH_SPACING)
                .finish(),
            );
        }

        details_column.add_child(
            Container::new(
                ui_builder
                    .paragraph(format!(
                        "Ran {}",
                        format_approx_duration_from_now(self.start_time)
                    ))
                    .build()
                    .finish(),
            )
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );

        Some(details_column.finish())
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.fuzzy_match_results.score()
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::AcceptAIQuery(self.query_text.clone())
    }

    fn execute_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::RunAIQuery(self.query_text.clone())
    }

    fn accessibility_label(&self) -> String {
        format!("AI query: {}", self.query_text)
    }
}

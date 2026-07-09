use std::sync::Arc;

use ordered_float::OrderedFloat;
use warp_errors::report_error;
use warpui::elements::{
    ConstrainedBox, Container, Flex, Highlight, Icon, MainAxisAlignment, MainAxisSize,
    ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::notebooks::CloudNotebookModel;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::item::SearchItem;
use crate::search::notebooks::fuzzy_match::render_notebook_matched_content_with_highlight;
use crate::search::result_renderer::ItemHighlightState;
use crate::server::ids::SyncId;

const CONTENT_WEIGHT: f64 = 0.4;
const NAME_WEIGHT: f64 = 0.6;

/// Struct designed to be the implementation of CommandSearchItem for Notebooks.
#[derive(Clone, Debug)]
pub struct NotebookSearchItem {
    pub id: SyncId,
    pub model: Arc<CloudNotebookModel>,
    pub name_match_result: Option<fuzzy_match::FuzzyMatchResult>,
    pub content_match_result: Option<fuzzy_match::FuzzyMatchResult>,
}

impl SearchItem for NotebookSearchItem {
    type Action = CommandSearchItemAction;

    /// Returns an text 'icon' containing the appropriate display abbreviation for the Notebook's
    /// source.
    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/notebook.svg",
                    highlight_state.icon_fill(appearance),
                )
                .finish(),
            )
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
            .finish(),
        )
        .with_margin_left(1.)
        .with_margin_right(8.)
        .finish()
    }

    /// Renders the name and block content of the Notebook.
    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut name_text = Text::new_inline(
            self.model.title.to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if let Some(name_match_result) = &self.name_match_result {
            name_text = name_text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                name_match_result.matched_indices.clone(),
            );
        }

        let content_text = render_notebook_matched_content_with_highlight(
            self.id,
            &self.model.data,
            &self.content_match_result,
            highlight_state,
            app,
        );

        let mut item = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(name_text.finish());

        item.add_child(content_text.finish());

        item.finish()
    }

    fn render_details(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        match (&self.name_match_result, &self.content_match_result) {
            (Some(name_match_result), Some(content_match_result)) => OrderedFloat(
                (name_match_result.score as f64 * NAME_WEIGHT)
                    + (content_match_result.score as f64 * CONTENT_WEIGHT),
            ),
            (None, Some(content_match_result)) => {
                OrderedFloat(content_match_result.score as f64 * CONTENT_WEIGHT / NAME_WEIGHT)
            }
            (Some(name_match_result), None) => {
                OrderedFloat(name_match_result.score as f64 * NAME_WEIGHT / CONTENT_WEIGHT)
            }
            (None, None) => {
                // This branch should never be executed because a Notebooks search result should
                // always have some match with the query, otherwise it should not appear as a
                // result.
                report_error!(
                    "Notebook in search results has neither a name nor command match result."
                );
                OrderedFloat(f64::MIN)
            }
        }
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::AcceptNotebook(self.id)
    }

    /// Notebooks aren't directly executable (the user needs to choose
    /// a command block within the notebook to execute) so we will just
    /// accept the result since we cannot execute it.
    fn execute_result(&self) -> CommandSearchItemAction {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Notebook: {}", self.model.title)
    }
}

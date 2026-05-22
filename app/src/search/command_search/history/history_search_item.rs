use std::sync::Arc;

use ordered_float::OrderedFloat;
use warp_core::ui::builder;
use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, MainAxisAlignment,
    MainAxisSize, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::command_search::searcher::{AcceptedHistoryItem, CommandSearchItemAction};
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::terminal::rich_history::render_rich_history;
use crate::terminal::HistoryEntry;
use crate::ui_components::icons::Icon as UiIcon;
use crate::util::time_format::format_approx_duration_from_now;

const COMMAND_METADATA_LEFT_MARGIN_FROM_METADATA: f32 = 8.;

#[derive(Clone, Debug)]
pub struct HistorySearchItem {
    pub entry: Arc<HistoryEntry>,
    pub match_result: fuzzy_match::FuzzyMatchResult,
}

impl SearchItem for HistorySearchItem {
    type Action = CommandSearchItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/history.svg",
                    highlight_state.icon_fill(appearance),
                )
                .finish(),
            )
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
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

        let command = Align::new(
            Text::new_inline(
                self.entry.command.clone(),
                appearance.monospace_font_family(),
                appearance.monospace_font_size(),
            )
            .autosize_text(builder::MIN_FONT_SIZE)
            .with_color(highlight_state.main_text_fill(appearance).into_solid())
            .with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                self.match_result.matched_indices.clone(),
            )
            .finish(),
        )
        .left()
        .finish();

        let mut command_and_workflow = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(command);

        if let Some(workflow) = self.entry.linked_workflow(app) {
            command_and_workflow.add_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([
                        ConstrainedBox::new(
                            Icon::new(
                                "bundled/svg/workflow.svg",
                                highlight_state.sub_text_fill(appearance).into_solid(),
                            )
                            .finish(),
                        )
                        .with_height(appearance.monospace_font_size() - 4.)
                        .with_width(appearance.monospace_font_size() - 4.)
                        .finish(),
                        Shrinkable::new(
                            1.,
                            Align::new(
                                Container::new(
                                    Text::new_inline(
                                        workflow.name().to_owned(),
                                        appearance.ui_font_family(),
                                        appearance.monospace_font_size() - 2.,
                                    )
                                    .with_color(
                                        highlight_state.sub_text_fill(appearance).into_solid(),
                                    )
                                    .finish(),
                                )
                                .with_margin_left(4.)
                                .finish(),
                            )
                            .left()
                            .finish(),
                        )
                        .finish(),
                    ])
                    .finish(),
            );
        }

        let mut item = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., command_and_workflow.finish()).finish());

        if let Some(metadata) = self.render_command_level_metadata(&highlight_state, appearance) {
            item.add_child(metadata);
        }
        item.finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        self.entry
            .has_metadata()
            .then(|| render_rich_history(self.entry.as_ref(), ctx))
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
            command: self.entry.command.clone(),
            linked_workflow_data: self.entry.linked_workflow_data(),
        })
    }

    fn execute_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::ExecuteHistory(self.entry.command.clone())
    }

    fn accessibility_label(&self) -> String {
        format!("History item: {}", self.entry.command)
    }
}

impl HistorySearchItem {
    fn render_command_level_metadata(
        &self,
        item_highlight_state: &ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if self.entry.start_ts.is_none() && self.entry.exit_code.is_none() {
            return None;
        }

        let mut metadata_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(exit_code) = self.entry.exit_code {
            if !exit_code.was_successful() {
                metadata_row.add_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::new(
                                UiIcon::AlertTriangle.into(),
                                item_highlight_state.main_text_fill(appearance).into_solid(),
                            )
                            .finish(),
                        )
                        .with_max_height(appearance.ui_font_size())
                        .with_max_width(appearance.ui_font_size())
                        .finish(),
                    )
                    .finish(),
                );
            }
        }

        if let Some(start) = self.entry.start_ts {
            metadata_row.add_child(
                appearance
                    .ui_builder()
                    .span(format_approx_duration_from_now(start))
                    .with_style(UiComponentStyles {
                        margin: Some(
                            Coords::uniform(0.).left(COMMAND_METADATA_LEFT_MARGIN_FROM_METADATA),
                        ),
                        font_color: Some(
                            item_highlight_state.main_text_fill(appearance).into_solid(),
                        ),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );
        }

        Some(Container::new(metadata_row.finish()).finish())
    }
}

use ordered_float::OrderedFloat;
use warpui::elements::{Clipped, Container, Flex, Highlight, ParentElement, Shrinkable, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::cloud_object::CloudObject;
use crate::drive::cloud_object_styling::warp_drive_icon_color;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::command_palette::styles::SEARCH_ITEM_TEXT_PADDING;
use crate::search::item::{IconLocation, SearchItem};
use crate::search::result_renderer::ItemHighlightState;
use crate::search::workflows::fuzzy_match::FuzzyMatchWorkflowResult;
use crate::ui_components::icons::Icon;
use crate::workflows::CloudWorkflow;

/// Search item result for a cloud workflow.
#[derive(Debug)]
pub struct WorkflowSearchItem {
    pub match_result: FuzzyMatchWorkflowResult,
    pub cloud_workflow: CloudWorkflow,
}

impl SearchItem for WorkflowSearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        true
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (icon, icon_color) = if self.cloud_workflow.model().data.is_agent_mode_workflow() {
            (
                Icon::Prompt,
                warp_drive_icon_color(appearance, DriveObjectType::AgentModeWorkflow),
            )
        } else {
            (
                Icon::Workflow,
                warp_drive_icon_color(appearance, DriveObjectType::Workflow),
            )
        };
        render_search_item_icon(appearance, icon, icon_color, highlight_state)
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        // The icon is has the size of the monospace font, whereas the text have a height of
        // `line_height_ratio * font_size`. Offset the icon by this difference so it is rendered
        // centered with the text.
        let margin_top = (appearance.line_height_ratio() * appearance.monospace_font_size())
            - appearance.monospace_font_size();
        IconLocation::Top { margin_top }
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut name_text = Text::new_inline(
            self.cloud_workflow.model().data.name().to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold));

        if let Some(name_match_result) = &self.match_result.name_match_result {
            name_text = name_text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                name_match_result.matched_indices.clone(),
            );
        }

        let mut breadcrumbs_text: Text = Text::new_inline(
            self.cloud_workflow.breadcrumbs(app),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        if let Some(folder_match_result) = &self.match_result.folder_match_result {
            breadcrumbs_text = breadcrumbs_text.with_single_highlight(
                Highlight::new()
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                folder_match_result.matched_indices.clone(),
            );
        }

        let mut content_text = Text::new_inline(
            self.cloud_workflow.model().data.content().to_owned(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        if let Some(command_match_result) = &self.match_result.content_match_result {
            content_text = content_text.with_single_highlight(
                Highlight::new()
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                command_match_result.matched_indices.clone(),
            );
        }

        let contents = Flex::column()
            .with_child(Container::new(name_text.finish()).finish())
            .with_child(
                Container::new(breadcrumbs_text.finish())
                    .with_padding_top(SEARCH_ITEM_TEXT_PADDING)
                    .finish(),
            )
            .with_child(
                Container::new(content_text.finish())
                    .with_padding_top(SEARCH_ITEM_TEXT_PADDING)
                    .finish(),
            )
            .finish();

        Clipped::new(Shrinkable::new(1., contents).finish()).finish()
    }

    fn render_details(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.match_result.score()
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::ExecuteWorkflow {
            id: self.cloud_workflow.id,
        }
    }

    fn execute_result(&self) -> Self::Action {
        CommandPaletteItemAction::ViewInWarpDrive {
            id: CloudObjectTypeAndId::Workflow(self.cloud_workflow.id),
        }
    }

    fn accessibility_label(&self) -> String {
        format!("Workflow: {}", self.cloud_workflow.model().data.name())
    }
}

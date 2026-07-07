use std::fmt::Debug;

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::cloud_object::ObjectType;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::{safe_truncate, styles};
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

const MAX_COMBINED_LENGTH: usize = 55;

#[derive(Debug)]
pub struct WorkflowSearchItem {
    pub workflow_name: String,
    pub workflow_description: Option<String>,
    pub workflow_uid: String,
    pub match_result: FuzzyMatchResult,
    /// True if match_result was computed against the workflow name (vs description)
    pub is_match_on_name: bool,
}

impl SearchItem for WorkflowSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/workflow.svg",
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

        let mut workflow_name = self.workflow_name.clone();
        let mut workflow_description = self
            .workflow_description
            .as_deref()
            .unwrap_or("")
            .to_string();

        // Track if we truncated anything for highlight adjustment
        let mut name_truncated = false;

        // Ensure combined length is reasonable
        let combined_length = workflow_name.len() + workflow_description.len();

        if combined_length > MAX_COMBINED_LENGTH {
            // Prioritize showing the workflow name
            if workflow_name.len() >= MAX_COMBINED_LENGTH {
                safe_truncate(&mut workflow_name, MAX_COMBINED_LENGTH - 3);
                workflow_name.push_str("...");
                name_truncated = true;
                workflow_description.clear();
            } else {
                // Workflow name fits, truncate description
                let available_for_description = MAX_COMBINED_LENGTH - workflow_name.len();
                if workflow_description.len() > available_for_description {
                    safe_truncate(
                        &mut workflow_description,
                        available_for_description.saturating_sub(3),
                    );
                    workflow_description.push_str("...");
                }
            }
        }

        // Calculate highlight indices based on where match occurred
        let name_highlights = if !self.match_result.matched_indices.is_empty()
            && !name_truncated
            && self.is_match_on_name
        {
            self.match_result.matched_indices.clone()
        } else {
            vec![]
        };

        let description_highlights = if !self.match_result.matched_indices.is_empty()
            && !self.is_match_on_name
            && !workflow_description.is_empty()
        {
            self.match_result.matched_indices.clone()
        } else {
            vec![]
        };

        // Create workflow name with match highlighting
        let mut name_text = Text::new(
            workflow_name,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if !name_highlights.is_empty() {
            name_text = name_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                name_highlights,
            );
        }

        // Create description text with lighter color
        let description_text = if !workflow_description.is_empty() {
            let mut desc_text = Text::new(
                workflow_description,
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.0,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid());

            if !description_highlights.is_empty() {
                desc_text = desc_text.with_single_highlight(
                    Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                    description_highlights,
                );
            }

            Some(desc_text)
        } else {
            None
        };

        // Create row with workflow name and description
        let mut row = Flex::row()
            .with_child(name_text.finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(description) = description_text {
            row.add_child(
                Container::new(description.finish())
                    .with_padding_left(6.)
                    .finish(),
            );
        }

        row.finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        AIContextMenuSearchableAction::InsertDriveObject {
            object_type: ObjectType::Workflow,
            object_uid: self.workflow_uid.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        if let Some(description) = &self.workflow_description {
            format!("Workflow: {} - {}", self.workflow_name, description)
        } else {
            format!("Workflow: {}", self.workflow_name)
        }
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);

        let name_element = Text::new(
            self.workflow_name.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(appearance.theme().active_ui_text_color().into());

        let details = if let Some(description) = &self.workflow_description {
            let content_element = Text::new(
                description.clone(),
                appearance.monospace_font_family(),
                appearance.monospace_font_size() - 2.0,
            )
            .with_color(appearance.theme().nonactive_ui_text_color().into());

            Flex::column()
                .with_child(name_element.finish())
                .with_child(
                    Container::new(content_element.finish())
                        .with_padding_top(4.0)
                        .finish(),
                )
                .finish()
        } else {
            Flex::column().with_child(name_element.finish()).finish()
        };

        Some(details)
    }
}

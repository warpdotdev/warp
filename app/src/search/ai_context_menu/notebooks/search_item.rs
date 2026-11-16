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
pub struct NotebookSearchItem {
    pub notebook_name: String,
    pub notebook_description: Option<String>,
    pub notebook_uid: String,
    pub match_result: FuzzyMatchResult,
    pub ai_document_uid: Option<String>,
    /// True if match_result was computed against the notebook name (vs description)
    pub is_match_on_name: bool,
}

impl SearchItem for NotebookSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    if self.ai_document_uid.is_some() {
                        "bundled/svg/compass-3.svg"
                    } else {
                        "bundled/svg/notebook.svg"
                    },
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

        let mut notebook_name = self.notebook_name.clone();
        let mut notebook_description = self
            .notebook_description
            .as_deref()
            .unwrap_or("")
            .to_string();

        // Track if we truncated anything for highlight adjustment
        let mut name_truncated = false;

        // Ensure combined length is reasonable
        let combined_length = notebook_name.len() + notebook_description.len();

        if combined_length > MAX_COMBINED_LENGTH {
            // Prioritize showing the notebook name
            if notebook_name.len() >= MAX_COMBINED_LENGTH {
                safe_truncate(&mut notebook_name, MAX_COMBINED_LENGTH - 3);
                notebook_name.push_str("...");
                name_truncated = true;
                notebook_description.clear();
            } else {
                // Notebook name fits, truncate description
                let available_for_description = MAX_COMBINED_LENGTH - notebook_name.len();
                if notebook_description.len() > available_for_description {
                    safe_truncate(
                        &mut notebook_description,
                        available_for_description.saturating_sub(3),
                    );
                    notebook_description.push_str("...");
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
            && !notebook_description.is_empty()
        {
            self.match_result.matched_indices.clone()
        } else {
            vec![]
        };

        // Create notebook name with match highlighting
        let mut name_text = Text::new(
            notebook_name,
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
        let description_text = if !notebook_description.is_empty() {
            let mut desc_text = Text::new(
                notebook_description,
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

        // Create row with notebook name and description
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
        if let Some(ai_document_uid) = &self.ai_document_uid {
            return AIContextMenuSearchableAction::InsertPlan {
                ai_document_uid: ai_document_uid.clone(),
            };
        }
        AIContextMenuSearchableAction::InsertDriveObject {
            object_type: ObjectType::Notebook,
            object_uid: self.notebook_uid.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        if let Some(description) = &self.notebook_description {
            format!("Notebook: {} - {}", self.notebook_name, description)
        } else {
            format!("Notebook: {}", self.notebook_name)
        }
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);

        // Use notebook name, or "Untitled" if empty
        let display_name = if self.notebook_name.is_empty() {
            "Untitled".to_string()
        } else {
            self.notebook_name.clone()
        };

        let name_element = Text::new(
            display_name,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(appearance.theme().active_ui_text_color().into());

        let details = if let Some(content) = &self.notebook_description {
            let content_element = Text::new(
                content.clone(),
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

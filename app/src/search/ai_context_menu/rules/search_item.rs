use std::fmt::Debug;

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::cloud_object::{GenericStringObjectFormat, JsonObjectType, ObjectType};
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::{safe_truncate, styles};
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

const MAX_COMBINED_LENGTH: usize = 55;

#[derive(Debug)]
pub struct RuleSearchItem {
    pub rule_uid: String,
    pub rule_name: Option<String>,
    pub rule_content: String,
    pub match_result: FuzzyMatchResult,
    /// True if match_result was computed against the rule name (vs content)
    pub is_match_on_rule_name: bool,
}

impl SearchItem for RuleSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/book-open.svg",
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

        // Use rule_name if available, otherwise fall back to rule_content
        let (primary_text, secondary_text, is_match_on_primary) = match &self.rule_name {
            Some(name) if !name.is_empty() => (
                name.clone(),
                Some(self.rule_content.clone()),
                self.is_match_on_rule_name,
            ),
            _ => (self.rule_content.clone(), None, true),
        };

        let mut display_primary = primary_text;
        let mut display_secondary = secondary_text.unwrap_or_default();
        let mut primary_truncated = false;

        // Ensure combined length is reasonable
        let combined_length = display_primary.len() + display_secondary.len();

        if combined_length > MAX_COMBINED_LENGTH {
            if display_primary.len() >= MAX_COMBINED_LENGTH {
                safe_truncate(&mut display_primary, MAX_COMBINED_LENGTH - 3);
                display_primary.push_str("...");
                primary_truncated = true;
                display_secondary.clear();
            } else {
                let available_for_secondary = MAX_COMBINED_LENGTH - display_primary.len();
                if display_secondary.len() > available_for_secondary {
                    safe_truncate(
                        &mut display_secondary,
                        available_for_secondary.saturating_sub(3),
                    );
                    display_secondary.push_str("...");
                }
            }
        }

        // Calculate highlight indices for primary or secondary text based on where match occurred
        let primary_highlights = if !self.match_result.matched_indices.is_empty()
            && !primary_truncated
            && is_match_on_primary
        {
            self.match_result.matched_indices.clone()
        } else {
            vec![]
        };

        let secondary_highlights = if !self.match_result.matched_indices.is_empty()
            && !is_match_on_primary
            && !display_secondary.is_empty()
        {
            self.match_result.matched_indices.clone()
        } else {
            vec![]
        };

        let mut primary_text_element = Text::new(
            display_primary,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if !primary_highlights.is_empty() {
            primary_text_element = primary_text_element.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                primary_highlights,
            );
        }

        let secondary_text_element = if !display_secondary.is_empty() {
            let mut secondary_text = Text::new(
                display_secondary,
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.0,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid());

            if !secondary_highlights.is_empty() {
                secondary_text = secondary_text.with_single_highlight(
                    Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                    secondary_highlights,
                );
            }

            Some(secondary_text)
        } else {
            None
        };

        let mut row = Flex::row()
            .with_child(primary_text_element.finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(secondary) = secondary_text_element {
            row.add_child(
                Container::new(secondary.finish())
                    .with_padding_left(6.)
                    .finish(),
            );
        }

        row.finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();

        // Determine what to show as the main title
        let title = if let Some(name) = &self.rule_name {
            if !name.is_empty() {
                name.clone()
            } else {
                "Rule".to_string()
            }
        } else {
            "Rule".to_string()
        };

        // Create title element
        let title_element = Text::new(
            title,
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(theme.active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        // Create content element - show the full rule content
        let content_element = Text::new(
            self.rule_content.clone(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // Create the details content
        let content = Flex::column()
            .with_child(title_element)
            .with_child(
                Container::new(content_element)
                    .with_padding_top(8.0)
                    .finish(),
            )
            .finish();

        Some(content)
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        AIContextMenuSearchableAction::InsertDriveObject {
            object_type: ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                JsonObjectType::AIFact,
            )),
            object_uid: self.rule_uid.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Rule: {}", self.rule_content)
    }
}

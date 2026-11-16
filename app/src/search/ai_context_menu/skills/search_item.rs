use ai::skills::SkillProvider;
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::icons::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::styles;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

const MAX_DESCRIPTION_LEN: usize = 60;

#[derive(Debug)]
pub struct SkillSearchItem {
    pub name: String,
    pub description: String,
    pub provider: SkillProvider,
    pub icon_override: Option<Icon>,
    pub match_result: FuzzyMatchResult,
}

impl SearchItem for SkillSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color = highlight_state.icon_fill(appearance).into_solid();

        let icon_element = if let Some(override_icon) = self.icon_override {
            override_icon.to_warpui_icon(icon_color.into()).finish()
        } else {
            self.provider
                .icon()
                .to_warpui_icon(self.provider.icon_fill(icon_color.into()))
                .finish()
        };

        Container::new(
            ConstrainedBox::new(icon_element)
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
        let font_size = appearance.monospace_font_size() - 1.0;

        let mut name_text = Text::new(self.name.clone(), appearance.ui_font_family(), font_size)
            .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if !self.match_result.matched_indices.is_empty() {
            name_text = name_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                self.match_result.matched_indices.clone(),
            );
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(name_text.finish());

        if !self.description.is_empty() {
            let mut display_description = self.description.clone();
            if display_description.len() > MAX_DESCRIPTION_LEN {
                let truncate_at = display_description
                    .char_indices()
                    .map(|(i, _)| i)
                    .take_while(|&i| i <= MAX_DESCRIPTION_LEN - 3)
                    .last()
                    .unwrap_or(0);
                display_description.truncate(truncate_at);
                display_description.push_str("...");
            }

            let description_text = Text::new(
                display_description,
                appearance.ui_font_family(),
                font_size - 1.0,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid());

            row.add_child(
                Shrinkable::new(
                    1.0,
                    Container::new(description_text.finish())
                        .with_padding_left(6.0)
                        .finish(),
                )
                .finish(),
            );
        }

        row.finish()
    }

    fn render_details(&self, _app: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        AIContextMenuSearchableAction::InsertSkill {
            name: self.name.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Skill: {}", self.name)
    }
}

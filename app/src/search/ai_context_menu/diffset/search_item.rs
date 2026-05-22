use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Icon, ParentElement, Text,
};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::code_review::diff_state::DiffMode;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::styles;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

#[derive(Debug, Clone)]
pub struct DiffSetSearchItem {
    pub diff_mode: DiffMode,
    pub match_result: FuzzyMatchResult,
}

impl DiffSetSearchItem {
    pub fn name(&self) -> String {
        match &self.diff_mode {
            DiffMode::Head => "Uncommitted changes".to_string(),
            DiffMode::MainBranch => "Changes vs. main branch".to_string(),
            DiffMode::OtherBranch(branch) => format!("Changes vs. {branch}"),
        }
    }

    pub fn description(&self) -> String {
        match &self.diff_mode {
            DiffMode::Head => "All uncommitted changes in the working directory".to_string(),
            DiffMode::MainBranch => "All changes compared to the main branch".to_string(),
            DiffMode::OtherBranch(branch) => format!("All changes compared to {branch}"),
        }
    }
}

impl SearchItem for DiffSetSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/diff.svg",
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

        let name_text = Text::new(
            self.name(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        let description_text = Text::new(
            self.description(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.0,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(name_text.finish())
            .with_child(
                Container::new(description_text.finish())
                    .with_padding_left(6.)
                    .finish(),
            )
            .finish()
    }

    fn priority_tier(&self) -> u8 {
        // Prioritize diffsets above other items.
        1
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        AIContextMenuSearchableAction::InsertDiffSet {
            diff_mode: self.diff_mode.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("{} - {}", self.name(), self.description())
    }
}

#[cfg(test)]
#[path = "search_item_tests.rs"]
mod tests;

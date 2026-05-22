use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{ConstrainedBox, Container, Icon, Text};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

#[derive(Clone, Debug)]
pub struct CommandSearchItem {
    pub command: String,
    pub match_result: FuzzyMatchResult,
}

impl SearchItem for CommandSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/terminal.svg",
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

        Text::new_inline(
            self.command.clone(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .with_single_highlight(
            warpui::elements::Highlight::new()
                .with_properties(
                    warpui::fonts::Properties::default().weight(warpui::fonts::Weight::Bold),
                )
                .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
            self.match_result.matched_indices.clone(),
        )
        .finish()
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> AIContextMenuSearchableAction {
        AIContextMenuSearchableAction::InsertText {
            text: self.command.clone(),
        }
    }

    fn execute_result(&self) -> AIContextMenuSearchableAction {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Command: {}", self.command)
    }
}

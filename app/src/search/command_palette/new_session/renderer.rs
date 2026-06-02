use warpui::elements::{Container, Flex, Highlight, ParentElement, Text};
use warpui::fonts::{Properties, Weight};
use warpui::Element;

use super::new_session_option::NewSessionOption;
use crate::appearance::Appearance;
use crate::search::command_palette::styles::SEARCH_ITEM_TEXT_PADDING;
use crate::search::result_renderer::ItemHighlightState;

impl NewSessionOption {
    pub(super) fn render(
        &self,
        appearance: &Appearance,
        highlight_state: ItemHighlightState,
        highlight_indices: Vec<usize>,
    ) -> Box<dyn Element> {
        let highlight = Highlight::new()
            .with_properties(Properties::default().weight(Weight::Bold))
            .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid());

        let display_text = Text::new_inline(
            self.description().to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold))
        .with_single_highlight(highlight, highlight_indices)
        .finish();

        let details = Text::new_inline(
            self.details().to_string(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .finish();

        Flex::column()
            .with_child(Container::new(display_text).finish())
            .with_child(
                Container::new(details)
                    .with_padding_top(SEARCH_ITEM_TEXT_PADDING)
                    .finish(),
            )
            .finish()
    }
}

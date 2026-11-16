use ordered_float::OrderedFloat;
use warpui::elements::{Empty, Text};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

/// A simple separator item that displays a title to visually separate sections in search results.
#[derive(Debug)]
pub struct SeparatorSearchItem {
    pub title: String,
}

impl SeparatorSearchItem {
    pub fn new(title: String) -> Self {
        Self { title }
    }
}

impl SearchItem for SeparatorSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        _appearance: &Appearance,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        Text::new_inline(
            self.title.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() * 0.85,
        )
        .with_color(
            appearance
                .theme()
                .disabled_text_color(appearance.theme().surface_2())
                .into_solid(),
        )
        .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        // Give separators a neutral score - they should be positioned explicitly
        OrderedFloat(0.0)
    }

    /// Separators are non-interactable, so we should not do anything when they are accepted.
    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::NoOp
    }

    fn execute_result(&self) -> Self::Action {
        CommandPaletteItemAction::NoOp
    }

    fn accessibility_label(&self) -> String {
        format!("Section: {}", self.title)
    }

    fn is_static_separator(&self) -> bool {
        true
    }
}

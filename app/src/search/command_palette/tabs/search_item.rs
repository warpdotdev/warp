use crate::appearance::Appearance;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::{IconLocation, SearchItem as SearchItemTrait};
use crate::search::result_renderer::ItemHighlightState;
use crate::session_management::TabNavigationData;
use crate::ui_components::icons::Icon;
use ordered_float::OrderedFloat;
use warpui::elements::{ConstrainedBox, Container, Flex, ParentElement, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

/// These items appear in the ctrl-tab palette only, not the main command palette.
/// Scoring matches against queries is not supported since only ranking by recency is needed.
pub struct SearchItem {
    tab: TabNavigationData,
    mru_rank: usize,
}

impl SearchItem {
    pub fn new(tab: TabNavigationData, mru_rank: usize) -> Self {
        Self { tab, mru_rank }
    }
}

impl SearchItemTrait for SearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        self.tab.subtitle.is_some()
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let color = highlight_state.icon_fill(appearance).into_solid();
        render_search_item_icon(appearance, Icon::Navigation, color, highlight_state)
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
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

        let title_text = Text::new_inline(
            format!("[Tab {}] {}", self.tab.tab_index, self.tab.title),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        if let Some(subtitle) = &self.tab.subtitle {
            let subtitle_text = Text::new_inline(
                subtitle.clone(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid())
            .finish();

            let contents = Flex::column()
                .with_child(title_text)
                .with_child(Container::new(subtitle_text).with_padding_top(4.).finish())
                .finish();
            ConstrainedBox::new(contents).with_height(50.).finish()
        } else {
            ConstrainedBox::new(Flex::column().with_child(title_text).finish())
                .with_height(50.)
                .finish()
        }
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat::from(1000.0 - self.mru_rank as f64)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::NavigateToTab {
            pane_group_id: self.tab.pane_group_id,
            window_id: self.tab.window_id,
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Selected tab: {}.", self.tab.title)
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some(format!(
            "Press enter to navigate to tab: {}.",
            self.tab.title
        ))
    }
}

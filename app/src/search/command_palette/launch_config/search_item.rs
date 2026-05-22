use std::sync::Arc;

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon;

/// SearchItem for a matching [`LaunchConfig`].
#[derive(Debug)]
pub struct SearchItem {
    match_result: FuzzyMatchResult,
    launch_config: Arc<LaunchConfig>,
}

impl SearchItem {
    pub fn new(launch_config: Arc<LaunchConfig>, match_result: FuzzyMatchResult) -> Self {
        Self {
            match_result,
            launch_config,
        }
    }
}

impl crate::search::item::SearchItem for SearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let color = appearance.theme().foreground().into_solid();
        render_search_item_icon(appearance, Icon::Navigation, color, highlight_state)
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.launch_config.render(
            appearance,
            highlight_state,
            self.match_result.matched_indices.clone(),
        )
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat::from(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::OpenLaunchConfiguration {
            config: self.launch_config.clone(),
            open_in_active_window: false,
        }
    }

    fn execute_result(&self) -> Self::Action {
        CommandPaletteItemAction::OpenLaunchConfiguration {
            config: self.launch_config.clone(),
            open_in_active_window: true,
        }
    }

    fn accessibility_label(&self) -> String {
        format!("Selected {}.", self.launch_config.name)
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some("Press enter to use this launch configuration.".into())
    }
}

use serde::{Deserialize, Serialize};
use warpui::AppContext;
use warpui::keymap::Keystroke;

pub mod tip_view;
pub use tip_view::{TipsEvent, TipsView};

use crate::util::bindings::trigger_to_keystroke;

#[derive(Clone, Copy, Debug, Hash, PartialEq, std::cmp::Eq, Serialize, Deserialize)]

// TODO: Rename and move to resource center
pub enum WelcomeTipFeature {
    Workflows,
    CommandPalette,
    SplitPane,
    ThemePicker,
    HistorySearch,
    AiCommandSearch,
}

pub const WELCOME_TIP_FEATURE_LENGTH: usize = 6;

impl WelcomeTipFeature {
    pub fn editable_binding_name(&self) -> &'static str {
        match self {
            WelcomeTipFeature::Workflows => "input:toggle_workflows",
            WelcomeTipFeature::CommandPalette => "workspace:toggle_command_palette",
            WelcomeTipFeature::SplitPane => "pane_group:add_right",
            WelcomeTipFeature::HistorySearch => "input:search_command_history",
            WelcomeTipFeature::AiCommandSearch => "input:toggle_natural_language_command_search",
            WelcomeTipFeature::ThemePicker => "workspace:show_theme_chooser",
        }
    }

    pub fn keyboard_shortcut(&self, ctx: &mut AppContext) -> Option<Keystroke> {
        ctx.editable_bindings()
            .find(|binding| binding.name == self.editable_binding_name())
            .and_then(|binding| trigger_to_keystroke(binding.trigger))
    }
}

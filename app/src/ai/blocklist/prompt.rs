//! This module implements an AI "prompt" view shown in the terminal input when in AI mode.
//!
//! The AI prompt relays important information to the user including the underlying model powering
//! AI, indication of any context attached to the query, and a button for starting a new conversation.

use pathfinder_color::ColorU;

use crate::themes::theme::Fill;
use crate::util::color::coloru_with_opacity;
use crate::view_components::action_button::{ActionButtonTheme, NakedTheme};
use crate::Appearance;

pub mod plan_and_todo_list;
pub mod prompt_alert;

const BLURRED_OPACITY: u8 = 50;

/// Shared theme for icon-only prompt buttons (used by UDI and compact model selector)
#[derive(Clone)]
pub struct PromptIconButtonTheme {
    is_blurred: bool,
}

impl PromptIconButtonTheme {
    pub fn new(is_blurred: bool) -> Self {
        Self { is_blurred }
    }
}

impl ActionButtonTheme for PromptIconButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        NakedTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        let color = appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into_solid();
        if self.is_blurred {
            coloru_with_opacity(color, BLURRED_OPACITY)
        } else {
            color
        }
    }
}

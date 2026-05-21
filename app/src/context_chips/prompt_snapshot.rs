use itertools::Itertools;
use serde::{Deserialize, Serialize};
use warpui::{AppContext, SingletonEntity};

use crate::context_chips::ContextChipKind;

use super::current_prompt::CurrentPrompt;
use super::prompt::Prompt;
use super::{chips_to_string, ChipResult, ChipValue};
use crate::settings::WarpPromptSeparator;

/// Struct that holds a point in time snapshot of a prompt (chips are no longer interactive)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptSnapshot {
    chips: Vec<ChipResult>,

    same_line_prompt_enabled: bool,
    /// The separator to use as a trailing character at the end of Warp prompt, if any.
    separator: WarpPromptSeparator,
}

impl PromptSnapshot {
    pub fn from_current_prompt(current_prompt: &CurrentPrompt, ctx: &AppContext) -> Self {
        let prompt = Prompt::as_ref(ctx);
        let current_prompt_snapshot = current_prompt.snapshot();
        let current_prompt_on_click_snapshot = current_prompt.on_click_snapshot();

        // Get base chip kinds from prompt configuration
        let all_chip_kinds = prompt.chip_kinds();

        // Re-sort current prompt snapshot so that it matches the order of elements in prompt
        let chips = all_chip_kinds
            .iter()
            .map(|chip_kind| {
                let value = current_prompt_snapshot
                    .get(chip_kind)
                    .cloned()
                    .unwrap_or_default();
                let on_click_values = current_prompt_on_click_snapshot
                    .get(chip_kind)
                    .cloned()
                    .unwrap_or_default();
                ChipResult {
                    kind: chip_kind.clone(),
                    value,
                    on_click_values,
                }
            })
            .collect_vec();

        log::debug!("Current prompt snapshot: {chips:?}");
        Self {
            chips,
            same_line_prompt_enabled: current_prompt.same_line_prompt_enabled(),
            separator: current_prompt.separator(),
        }
    }

    pub fn from_chips(
        chips: Vec<ChipResult>,
        same_line_prompt_enabled: bool,
        separator: WarpPromptSeparator,
    ) -> Self {
        Self {
            chips,
            same_line_prompt_enabled,
            separator,
        }
    }

    /// The value of the given chip, in this snapshot.
    pub fn chip_value(&self, chip: &ContextChipKind) -> Option<ChipValue> {
        self.chips.iter().find_map(|chip_result| {
            if chip_result.kind == *chip {
                chip_result.value.clone()
            } else {
                None
            }
        })
    }

    pub(crate) fn set_chip_value(
        &mut self,
        chip: &ContextChipKind,
        value: Option<ChipValue>,
    ) -> bool {
        let Some(chip_result) = self
            .chips
            .iter_mut()
            .find(|chip_result| chip_result.kind == *chip)
        else {
            return false;
        };

        if chip_result.value == value {
            return false;
        }

        chip_result.value = value;
        true
    }

    pub(crate) fn chips(&self) -> &Vec<ChipResult> {
        &self.chips
    }

    pub(super) fn same_line_prompt_enabled(&self) -> bool {
        self.same_line_prompt_enabled
    }

    pub(super) fn separator(&self) -> WarpPromptSeparator {
        self.separator
    }
}

impl std::fmt::Display for PromptSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&chips_to_string(self.chips.clone().into_iter()))
    }
}

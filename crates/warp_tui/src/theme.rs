//! Fixed color roles for the TUI transcript design.

use warpui_core::elements::tui::Color;

/// Text color for submitted agent prompts.
pub(super) const AGENT_INPUT_TEXT: Color = Color::from_u32(0xffffff);

/// Background color for submitted agent prompt rows.
pub(super) const AGENT_INPUT_BACKGROUND: Color = Color::from_u32(0x2c2d34);

/// Text color for streamed plain-text agent output.
pub(super) const AGENT_OUTPUT_TEXT: Color = Color::from_u32(0xf1f1f1);

/// Border color for the prompt input box.
pub(super) const INPUT_BORDER: Color = Color::from_u32(0xd0d1fe);

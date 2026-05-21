//! Shared display metadata for [`Harness`] variants.
//!
//! Any UI surface that shows a harness to the user — the harness selector
//! dropdown, the conversation details sidebar, etc. — should source its label,
//! icon, and brand color from here so the two surfaces cannot drift.

use pathfinder_color::ColorU;
use warp_cli::agent::Harness;

use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill as WarpThemeFill, WarpTheme};

use crate::ai::agent::conversation::AIAgentHarness;
use crate::ai::blocklist::CLAUDE_ORANGE;
use crate::terminal::cli_agent::{GEMINI_BLUE, OPENAI_COLOR, OPENCODE_COLOR};
use crate::ui_components::icons::Icon;

/// User-visible display name for a [`Harness`].
pub fn display_name(harness: Harness) -> &'static str {
    match harness {
        Harness::Oz => "Warp",
        Harness::Claude => "Claude Code",
        Harness::OpenCode => "OpenCode",
        Harness::Gemini => "Gemini CLI",
        Harness::Codex => "Codex",
        Harness::Unknown => "Unknown",
    }
}

/// Leading icon for a [`Harness`].
pub fn icon_for(harness: Harness) -> Icon {
    match harness {
        Harness::Oz => Icon::Warp,
        Harness::Claude => Icon::ClaudeLogo,
        Harness::OpenCode => Icon::OpenCodeLogo,
        Harness::Gemini => Icon::GeminiLogo,
        Harness::Codex => Icon::OpenAILogo,
        Harness::Unknown => Icon::HelpCircle,
    }
}

/// Brand tint for a [`Harness`]'s icon. `None` means "use the surface's
/// default foreground color".
pub fn brand_color(harness: Harness) -> Option<ColorU> {
    match harness {
        Harness::Oz => None,
        Harness::Claude => Some(CLAUDE_ORANGE),
        Harness::OpenCode => None,
        Harness::Gemini => Some(GEMINI_BLUE),
        Harness::Codex => Some(OPENAI_COLOR),
        Harness::Unknown => None,
    }
}

/// Circle background fill for a [`Harness`] icon rendered in a branded circle.
/// Matches the treatment used in the vertical-tabs sidebar.
pub fn circle_background(harness: Harness, theme: &WarpTheme) -> WarpThemeFill {
    match harness {
        Harness::Oz => theme.background(),
        Harness::Claude => WarpThemeFill::Solid(CLAUDE_ORANGE),
        Harness::Codex => WarpThemeFill::Solid(OPENAI_COLOR),
        Harness::Gemini => WarpThemeFill::Solid(GEMINI_BLUE),
        Harness::OpenCode => WarpThemeFill::Solid(OPENCODE_COLOR),
        Harness::Unknown => internal_colors::fg_overlay_2(theme),
    }
}

/// Icon fill color when rendered on the branded circle background.
pub fn icon_fill_on_circle(harness: Harness, theme: &WarpTheme) -> WarpThemeFill {
    match harness {
        Harness::Oz => theme.main_text_color(theme.background()),
        Harness::Claude | Harness::Codex | Harness::Gemini | Harness::OpenCode => {
            WarpThemeFill::Solid(ColorU::white())
        }
        Harness::Unknown => theme.main_text_color(internal_colors::fg_overlay_2(theme)),
    }
}

/// Map [`AIAgentHarness`] (from `ServerAIConversationMetadata`) to the
/// canonical [`Harness`].
impl From<AIAgentHarness> for Harness {
    fn from(harness: AIAgentHarness) -> Self {
        match harness {
            AIAgentHarness::Oz => Harness::Oz,
            AIAgentHarness::ClaudeCode => Harness::Claude,
            AIAgentHarness::Gemini => Harness::Gemini,
            AIAgentHarness::Codex => Harness::Codex,
            AIAgentHarness::Unknown => Harness::Unknown,
        }
    }
}

impl PartialEq<Harness> for AIAgentHarness {
    fn eq(&self, other: &Harness) -> bool {
        Harness::from(*self) == *other
    }
}

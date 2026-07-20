use std::path::PathBuf;

use warpui::Element;
use warpui::elements::{MouseStateHandle, Text};

use super::{
    InlineBannerButtonState, InlineBannerCloseButton, InlineBannerContent, InlineBannerStyle,
    InlineBannerTextButton, InlineBannerTextButtonFont, InlineBannerTextButtonVariant,
    render_inline_block_list_banner,
};
use crate::appearance::Appearance;
use crate::terminal::view::inline_banner::InlineBannerIcon;
use crate::terminal::view::{InlineBannerId, TerminalAction};

const SPEEDBUMP_HEADER: &str = "Optimize Warp for this codebase?";
const SPEEDBUMP_TEXT: &str = "Unlock smarter, more consistent responses by letting the Agent understand your codebase and generate rules for it. You can also do this at any point by running /init";
/// Text for the button that allows execution
const ALLOW_BUTTON_TEXT: &str = "Optimize";

#[derive(Clone, Copy, Debug)]
pub enum AgentModeSetupSpeedbumpBannerAction {
    SetupAgentMode,
    Close,
}

pub struct AgentModeSetupSpeedbumpBannerState {
    pub id: InlineBannerId,
    // Mouse state for the allow button that confirms and executes indexing.
    pub allow_button_mouse_state: MouseStateHandle,
    // Mouse state for the close button that dismisses the banner without executing indexing.
    pub close_button_mouse_state: MouseStateHandle,

    // The path to the repo that the banner is for.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub repo_path: PathBuf,
}

impl AgentModeSetupSpeedbumpBannerState {
    pub fn new(id: InlineBannerId, repo_path: PathBuf) -> Self {
        Self {
            id,
            allow_button_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
            repo_path,
        }
    }
}

pub fn render_agent_mode_setup_banner(
    state: &AgentModeSetupSpeedbumpBannerState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let open_button = InlineBannerTextButton {
        text: ALLOW_BUTTON_TEXT.to_string(),
        text_color: appearance.theme().active_ui_text_color().into_solid(),
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::AgentModeSetupSpeedbumpBanner(
                AgentModeSetupSpeedbumpBannerAction::SetupAgentMode,
            ),
            mouse_state_handle: state.allow_button_mouse_state.clone(),
        },
        font: InlineBannerTextButtonFont::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Primary,
    };

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::AgentModeSetupSpeedbumpBanner(
            AgentModeSetupSpeedbumpBannerAction::Close,
        ),
        mouse_state_handle: state.close_button_mouse_state.clone(),
    });

    render_inline_block_list_banner(
        InlineBannerStyle::Recommendation,
        appearance,
        InlineBannerContent {
            title: SPEEDBUMP_HEADER.to_string(),
            buttons: vec![open_button],
            close_button: Some(close_button),
            header_icon: Some(InlineBannerIcon {
                asset_path: "bundled/svg/info.svg",
                aspect_ratio: 1.,
                color_override: Some(appearance.theme().active_ui_text_color().into_solid()),
            }),
            content: Some(vec![
                Text::new(
                    SPEEDBUMP_TEXT,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 2.,
                )
                .with_color(appearance.theme().nonactive_ui_text_color().into_solid())
                .soft_wrap(true),
            ]),
            vertical_align_title_content: true,
        },
    )
}

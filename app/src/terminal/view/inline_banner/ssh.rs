use warpui::{elements::MouseStateHandle, AppContext, Element};

use crate::{appearance::Appearance, terminal::view::TerminalAction};

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerContent,
    InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};

#[derive(Clone, Copy, Debug)]
pub enum SSHBannerAction {
    LearnMore,
    Settings,
}

#[derive(Default)]
pub struct SSHBannerMouseStates {
    /// Hover state for the "Learn more" button in the SSH wrapper banner.
    pub learn_more: MouseStateHandle,
    /// Hover state for the "Settings" button in the SSH wrapper banner.
    pub settings: MouseStateHandle,
}

/// State necessary to render an SSH banner.
pub struct SSHBannerState {
    /// Whether this is a "wrapper enabled" or "wrapper disabled" version of the
    /// banner.
    pub wrapper_enabled: bool,
    pub mouse_states: SSHBannerMouseStates,
}

pub fn render_inline_ssh_wrapper_banner(
    state: &SSHBannerState,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let label_text_color = appearance.theme().active_ui_text_color().into_solid();

    let (style, title) = if state.wrapper_enabled {
        (
            InlineBannerStyle::LowPriority,
            crate::i18n::tr_static(app, "Warp SSH wrapper enabled").to_string(),
        )
    } else {
        (
            InlineBannerStyle::VeryLowPriority,
            crate::i18n::tr_static(app, "Warp SSH wrapper disabled").to_string(),
        )
    };
    let buttons = vec![
        InlineBannerTextButton {
            text: crate::i18n::tr_static(app, "Learn more").to_string(),
            text_color: label_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::LegacySSHBanner(SSHBannerAction::LearnMore),
                mouse_state_handle: state.mouse_states.learn_more.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Secondary,
        },
        InlineBannerTextButton {
            text: crate::i18n::tr_static(app, "Settings").to_string(),
            text_color: label_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::LegacySSHBanner(SSHBannerAction::Settings),
                mouse_state_handle: state.mouse_states.settings.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Primary,
        },
    ];

    render_inline_block_list_banner(
        style,
        appearance,
        InlineBannerContent {
            title,
            buttons,
            ..Default::default()
        },
    )
}

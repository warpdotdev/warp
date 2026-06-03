use warpui::elements::MouseStateHandle;
use warpui::Element;

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerContent,
    InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;

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
) -> Box<dyn Element> {
    let label_text_color = appearance.theme().active_ui_text_color().into_solid();

    let (style, title) = if state.wrapper_enabled {
        (
            InlineBannerStyle::LowPriority,
            i18n::t("terminal.inline_banner.ssh_wrapper.enabled"),
        )
    } else {
        (
            InlineBannerStyle::VeryLowPriority,
            i18n::t("terminal.inline_banner.ssh_wrapper.disabled"),
        )
    };
    let buttons = vec![
        InlineBannerTextButton {
            text: i18n::t("common.learn_more"),
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
            text: i18n::t("common.settings"),
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

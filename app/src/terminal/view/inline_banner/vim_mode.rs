use warpui::elements::MouseStateHandle;
use warpui::Element;

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;

pub struct VimModeBannerState {
    pub id: usize,
    pub yes_button_mouse_state: MouseStateHandle,
    pub no_button_mouse_state: MouseStateHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimModeBannerAction {
    Enable,
    Dismiss,
}

pub fn render_vim_mode_banner(
    state: &VimModeBannerState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color();

    let buttons = vec![InlineBannerTextButton {
        text: "Enable".to_owned(),
        text_color: active_ui_text_color.into_solid(),
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::VimModeBanner(VimModeBannerAction::Enable),
            mouse_state_handle: state.yes_button_mouse_state.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Primary,
    }];

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::VimModeBanner(VimModeBannerAction::Dismiss),
        mouse_state_handle: state.no_button_mouse_state.clone(),
    });

    render_inline_block_list_banner(
        InlineBannerStyle::LowPriority,
        appearance,
        InlineBannerContent {
            title: "Enable Warp's Vim keybindings?".to_string(),
            buttons,
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}

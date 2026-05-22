use warpui::{elements::MouseStateHandle, AppContext, Element};

use crate::{appearance::Appearance, terminal::view::TerminalAction};

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};

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
    app: &AppContext,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color();

    let buttons = vec![InlineBannerTextButton {
        text: crate::i18n::tr_static(app, "Enable").to_owned(),
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
            title: crate::i18n::tr_static(app, "Enable Warp's Vim keybindings?").to_string(),
            buttons,
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}

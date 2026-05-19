use warpui::{
    elements::{MouseStateHandle, Text},
    AppContext, Element,
};

use crate::appearance::Appearance;
use crate::localization;
use crate::terminal::alias::AliasedCommand;
use crate::terminal::view::TerminalAction;

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};

#[derive(Clone, Copy, Debug)]
pub enum AliasExpansionBannerAction {
    Enable,
    Dismiss,
}

#[derive(Default)]
pub enum AliasExpansionBanner {
    #[default]
    Closed,
    Open {
        state: AliasExpansionBannerState,
    },
}

pub struct AliasExpansionBannerState {
    pub id: usize,
    pub aliased_command: AliasedCommand,
    pub yes_button_mouse_state: MouseStateHandle,
    pub no_button_mouse_state: MouseStateHandle,
}

pub fn render_alias_expansion_banner(
    state: &AliasExpansionBannerState,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color();
    let accent_color = appearance.theme().accent().into_solid();

    let buttons = vec![InlineBannerTextButton {
        text: localization::text_for_app(app, "terminal.inline_banner.alias_expansion.enable"),
        text_color: active_ui_text_color.into_solid(),
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::AliasExpansionBanner(
                AliasExpansionBannerAction::Enable,
            ),
            mouse_state_handle: state.yes_button_mouse_state.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Primary,
    }];

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::AliasExpansionBanner(AliasExpansionBannerAction::Dismiss),
        mouse_state_handle: state.no_button_mouse_state.clone(),
    });

    let content = vec![
        Text::new_inline(
            " | ",
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(active_ui_text_color.with_opacity(20).into_solid()),
        Text::new_inline(
            state.aliased_command.alias.to_string(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(accent_color),
        Text::new_inline(
            " --> ",
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(active_ui_text_color.with_opacity(50).into_solid()),
        Text::new_inline(
            state.aliased_command.alias_value.clone(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(accent_color),
    ];

    render_inline_block_list_banner(
        InlineBannerStyle::VeryLowPriority,
        appearance,
        InlineBannerContent {
            title: localization::text_for_app(app, "terminal.inline_banner.alias_expansion.title"),
            buttons,
            content: Some(content),
            close_button: Some(close_button),
            header_icon: Some(super::InlineBannerIcon {
                asset_path: "bundled/svg/info.svg",
                aspect_ratio: 1.32,
                ..Default::default()
            }),
            vertical_align_title_content: false,
        },
    )
}

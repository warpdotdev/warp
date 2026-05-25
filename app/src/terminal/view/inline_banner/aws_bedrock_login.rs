use crate::localization;
use warpui::elements::MouseStateHandle;
use warpui::AppContext;
use warpui::Element;

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerIcon, InlineBannerStyle, InlineBannerTextButton,
    InlineBannerTextButtonVariant,
};
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;

pub struct AwsBedrockLoginBannerState {
    pub id: usize,
    pub login_button_mouse_state: MouseStateHandle,
    pub dismiss_button_mouse_state: MouseStateHandle,
    pub dont_show_again_button_mouse_state: MouseStateHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AwsBedrockLoginBannerAction {
    Login,
    Dismiss,
    DontShowAgain,
}

pub fn render_aws_bedrock_login_banner(
    state: &AwsBedrockLoginBannerState,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();
    let buttons = vec![
        InlineBannerTextButton {
            text: localization::text_for_app(app, "agent.block.action.dont_show_again"),
            text_color: active_ui_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::AwsBedrockLoginBanner(
                    AwsBedrockLoginBannerAction::DontShowAgain,
                ),
                mouse_state_handle: state.dont_show_again_button_mouse_state.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Secondary,
        },
        InlineBannerTextButton {
            text: localization::text_for_app(app, "terminal.inline_banner.aws_bedrock_login.login"),
            text_color: active_ui_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::AwsBedrockLoginBanner(
                    AwsBedrockLoginBannerAction::Login,
                ),
                mouse_state_handle: state.login_button_mouse_state.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Primary,
        },
    ];

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::AwsBedrockLoginBanner(AwsBedrockLoginBannerAction::Dismiss),
        mouse_state_handle: state.dismiss_button_mouse_state.clone(),
    });

    // Use sub_text_color for description to differentiate from title
    let description_text = warpui::elements::Text::new(
        localization::text_for_app(app, "terminal.inline_banner.aws_bedrock_login.description"),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(appearance.theme().nonactive_ui_text_color().into_solid())
    .soft_wrap(true);

    render_inline_block_list_banner(
        InlineBannerStyle::Recommendation,
        appearance,
        InlineBannerContent {
            title: localization::text_for_app(
                app,
                "terminal.inline_banner.aws_bedrock_login.title",
            ),
            content: Some(vec![description_text]),
            buttons,
            close_button: Some(close_button),
            header_icon: Some(InlineBannerIcon {
                asset_path: crate::ui_components::icons::Icon::Cloud.into(),
                aspect_ratio: 1.0,
                color_override: None,
            }),
            vertical_align_title_content: true,
        },
    )
}

use warpui::elements::MouseStateHandle;
use warpui::Element;

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerIcon, InlineBannerStyle, InlineBannerTextButton,
    InlineBannerTextButtonVariant,
};
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;

const AWS_CLI_INSTALL_DOCS_URL: &str =
    "https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html";

pub struct AwsCliNotInstalledBannerState {
    pub id: usize,
    pub learn_more_button_mouse_state: MouseStateHandle,
    pub dismiss_button_mouse_state: MouseStateHandle,
}

impl AwsCliNotInstalledBannerState {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            learn_more_button_mouse_state: Default::default(),
            dismiss_button_mouse_state: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AwsCliNotInstalledBannerAction {
    LearnMore,
    Dismiss,
}

impl AwsCliNotInstalledBannerAction {
    pub fn docs_url() -> &'static str {
        AWS_CLI_INSTALL_DOCS_URL
    }
}

pub fn render_aws_cli_not_installed_banner(
    state: &AwsCliNotInstalledBannerState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();
    let buttons = vec![InlineBannerTextButton {
        text: "Learn More".to_owned(),
        text_color: active_ui_text_color,
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::AwsCliNotInstalledBanner(
                AwsCliNotInstalledBannerAction::LearnMore,
            ),
            mouse_state_handle: state.learn_more_button_mouse_state.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Primary,
    }];

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::AwsCliNotInstalledBanner(
            AwsCliNotInstalledBannerAction::Dismiss,
        ),
        mouse_state_handle: state.dismiss_button_mouse_state.clone(),
    });

    let description_text = warpui::elements::Text::new(
        "The AWS CLI is required to authenticate with your organization's AWS Bedrock. Install it to continue.",
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(appearance.theme().nonactive_ui_text_color().into_solid())
    .soft_wrap(true);

    render_inline_block_list_banner(
        InlineBannerStyle::Recommendation,
        appearance,
        InlineBannerContent {
            title: "AWS CLI Not Installed".to_string(),
            content: Some(vec![description_text]),
            buttons,
            close_button: Some(close_button),
            header_icon: Some(InlineBannerIcon {
                asset_path: crate::ui_components::icons::Icon::AlertTriangle.into(),
                aspect_ratio: 1.0,
                color_override: None,
            }),
            vertical_align_title_content: true,
        },
    )
}

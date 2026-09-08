use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warpui::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    HighlightedHyperlink, Icon, MouseStateHandle, ParentElement, Shrinkable,
};
use warpui::ui_components::components::UiComponent;
use warpui::{Action, AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;

const LOGIN_TROUBLESHOOTING_DOCS_URL: &str = "https://docs.warp.dev/support-and-community/troubleshooting-and-support/troubleshooting-login-issues";

/// Represents reasons why login failed.
pub enum LoginFailureReason {
    InvalidRedirectUrl { was_pasted: bool },
    FailedUserAuthentication,
    FailedMintCustomToken,
    InvalidStateParameter,
    MissingStateParameter,
}

impl LoginFailureReason {
    /// Returns an error message to be presented to the user when login fails.
    pub(crate) fn to_formatted_text(&self) -> FormattedText {
        fn with_troubleshooting_text(
            mut fragments: Vec<FormattedTextFragment>,
        ) -> Vec<FormattedTextFragment> {
            fragments.extend([
                FormattedTextFragment::plain_text(" Not the first time? See our "),
                FormattedTextFragment::hyperlink(
                    "troubleshooting docs",
                    LOGIN_TROUBLESHOOTING_DOCS_URL,
                ),
                FormattedTextFragment::plain_text("."),
            ]);
            fragments
        }
        let fragments = match self {
            LoginFailureReason::InvalidRedirectUrl { was_pasted } => {
                let text = if *was_pasted {
                    "An invalid auth token was entered into the modal."
                } else {
                    "Failed to log in. Try manually copying the auth token from the \
                        authentication web page and pasting into the modal."
                };
                with_troubleshooting_text(vec![FormattedTextFragment::plain_text(text)])
            }
            LoginFailureReason::FailedUserAuthentication => {
                with_troubleshooting_text(vec![FormattedTextFragment::plain_text(
                    "Request to log in failed.",
                )])
            }
            LoginFailureReason::FailedMintCustomToken => {
                with_troubleshooting_text(vec![FormattedTextFragment::plain_text(
                    "Request to sign up failed.",
                )])
            }
            LoginFailureReason::InvalidStateParameter
            | LoginFailureReason::MissingStateParameter => {
                with_troubleshooting_text(vec![FormattedTextFragment::plain_text(
                    "The redirect URL pasted did not originate from this app. Please click the button below to try again.",
                )])
            }
        };
        FormattedText::new([FormattedTextLine::Line(fragments)])
    }
}

/// Renders a dismissable notification with a message explaining why login failed.
pub fn render<A: Action + Clone>(
    login_failure_reason: &LoginFailureReason,
    close_notification_mouse_state: MouseStateHandle,
    highlighted_hyperlink_state: HighlightedHyperlink,
    dismiss_action: A,
    ctx: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(ctx);

    let mut notification_contents =
        Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    notification_contents.add_child(
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/warning.svg",
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().surface_2()),
                )
                .finish(),
            )
            .with_width(styles::NOTIFICATION_WARNING_ICON_SIZE)
            .with_height(styles::NOTIFICATION_WARNING_ICON_SIZE)
            .finish(),
        )
        .with_margin_right(styles::NOTIFICATION_WARNING_MARGIN_RIGHT)
        .finish(),
    );
    notification_contents.add_child(
        Shrinkable::new(
            1.,
            Container::new(
                FormattedTextElement::new(
                    login_failure_reason.to_formatted_text(),
                    appearance.ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.monospace_font_family(),
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().surface_2())
                        .into_solid(),
                    highlighted_hyperlink_state,
                )
                .register_default_click_handlers(|url, _, ctx| {
                    ctx.open_url(&url.url);
                })
                .finish(),
            )
            .with_margin_right(styles::NOTIFICATION_MESSAGE_MARGIN_RIGHT)
            .finish(),
        )
        .finish(),
    );
    notification_contents.add_child(
        appearance
            .ui_builder()
            .close_button(
                styles::NOTIFICATION_CLOSE_BUTTON_SIZE,
                close_notification_mouse_state,
            )
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(dismiss_action.clone()))
            .finish(),
    );
    ConstrainedBox::new(
        Container::new(notification_contents.finish())
            .with_background(appearance.theme().surface_2())
            .with_corner_radius(styles::NOTIFICATION_CONTAINER_CORNER_RADIUS)
            .with_border(
                Border::all(styles::NOTIFICATION_BORDER_WIDTH)
                    .with_border_fill(appearance.theme().split_pane_border_color()),
            )
            .with_uniform_padding(styles::NOTIFICATION_CONTAINER_PADDING)
            .with_uniform_margin(16.)
            .finish(),
    )
    .with_max_width(450.)
    .finish()
}

mod styles {
    use warpui::elements::{CornerRadius, Radius};

    pub const NOTIFICATION_CONTAINER_PADDING: f32 = 8.;
    pub const NOTIFICATION_CONTAINER_CORNER_RADIUS: CornerRadius =
        CornerRadius::with_all(Radius::Pixels(4.));
    pub const NOTIFICATION_BORDER_WIDTH: f32 = 1.;

    pub const NOTIFICATION_CLOSE_BUTTON_SIZE: f32 = 24.;

    pub const NOTIFICATION_MESSAGE_MARGIN_RIGHT: f32 = 8.;

    pub const NOTIFICATION_WARNING_ICON_SIZE: f32 = 20.;
    pub const NOTIFICATION_WARNING_MARGIN_RIGHT: f32 = 12.;
}

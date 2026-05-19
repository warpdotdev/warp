use warpui::{elements::Text, AppContext, Element};

use crate::appearance::Appearance;
use crate::localization;

use super::{
    render_inline_block_list_banner, InlineBannerContent, InlineBannerIcon, InlineBannerStyle,
};

pub fn render_shell_process_terminated_banner(
    appearance: &Appearance,
    app: &AppContext,
    was_premature_termination: bool,
) -> Box<dyn Element> {
    if was_premature_termination {
        render_inline_block_list_banner(
            InlineBannerStyle::CallToAction,
            appearance,
            InlineBannerContent {
                title: localization::text_for_app(
                    app,
                    "terminal.inline_banner.shell_process.exited_prematurely",
                ),
                header_icon: Some(InlineBannerIcon {
                    asset_path: "bundled/svg/warning.svg",
                    aspect_ratio: 1.,
                    color_override: Some(appearance.theme().foreground().into_solid()),
                }),
                content: Some(vec![Text::new(
                    localization::text_for_app(
                        app,
                        "terminal.inline_banner.shell_process.debug_output_visible",
                    ),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )]),
                ..Default::default()
            },
        )
    } else {
        render_inline_block_list_banner(
            InlineBannerStyle::LowPriority,
            appearance,
            InlineBannerContent {
                title: localization::text_for_app(
                    app,
                    "terminal.inline_banner.shell_process.exited",
                ),
                header_icon: Some(InlineBannerIcon {
                    asset_path: "bundled/svg/info.svg",
                    aspect_ratio: 1.,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
    }
}

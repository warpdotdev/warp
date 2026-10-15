use warpui::Element;
use warpui::elements::Text;

use super::{
    InlineBannerContent, InlineBannerIcon, InlineBannerStyle, render_inline_block_list_banner,
};
use crate::appearance::Appearance;

pub fn render_shell_process_terminated_banner(
    appearance: &Appearance,
    was_premature_termination: bool,
) -> Box<dyn Element> {
    if was_premature_termination {
        render_inline_block_list_banner(
            InlineBannerStyle::CallToAction,
            appearance,
            InlineBannerContent {
                title: t!("terminal_banner.shell_process_exited_prematurely").to_string(),
                header_icon: Some(InlineBannerIcon {
                    asset_path: "bundled/svg/warning.svg",
                    aspect_ratio: 1.,
                    color_override: Some(appearance.theme().foreground().into_solid()),
                }),
                content: Some(vec![Text::new(
                    t!("help_ui.the_output_from_warp_s_initialization_script_is_visible"),
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
                title: t!("terminal_banner.shell_process_exited").to_string(),
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

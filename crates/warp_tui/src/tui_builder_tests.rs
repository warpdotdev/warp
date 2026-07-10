use warp::tui_export::light_theme;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui_core::elements::tui::Color;
use warpui_core::elements::Fill as CoreFill;

use super::TuiUiBuilder;

#[test]
fn text_styles_follow_light_theme_foreground() {
    let theme = light_theme();
    let builder = TuiUiBuilder {
        warp_theme: theme.clone(),
    };

    let details = theme.details();
    let expected_primary: Color = CoreFill::from(
        theme
            .background()
            .blend(&theme.foreground().with_opacity(details.main_text_opacity)),
    )
    .into();
    let expected_muted: Color = CoreFill::from(
        theme
            .background()
            .blend(&theme.foreground().with_opacity(details.sub_text_opacity)),
    )
    .into();

    assert_eq!(builder.primary_text_style().fg, Some(expected_primary));
    assert_eq!(builder.muted_text_style().fg, Some(expected_muted));
    assert_ne!(
        builder.primary_text_style().fg,
        Some(CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.white)).into()),
    );
}

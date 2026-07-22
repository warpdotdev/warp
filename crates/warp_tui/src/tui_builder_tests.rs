use warp::tui_export::light_theme;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui_core::elements::Fill as CoreFill;
use warpui_core::elements::tui::{Color, Modifier};

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

    let slash_command_color: Color = CoreFill::from(ThemeFill::Solid(theme.ansi_fg_blue())).into();
    let selection_fill = ThemeFill::from(theme.terminal_colors().normal.cyan);
    let selection_background: Color = CoreFill::from(selection_fill).into();
    let selection_foreground: Color =
        CoreFill::from(theme.font_color(selection_fill.into_solid())).into();
    assert_eq!(
        builder.slash_command_text_style().fg,
        Some(slash_command_color)
    );
    assert_eq!(builder.link_text_style().fg, Some(slash_command_color));
    assert_eq!(
        builder.slash_command_selection_background(),
        selection_background
    );
    let selection_style = builder.slash_command_selection_text_style();
    assert_eq!(selection_style.fg, Some(selection_foreground));
    assert_eq!(selection_style.bg, Some(selection_background));
    assert!(selection_style.add_modifier.contains(Modifier::BOLD));
}

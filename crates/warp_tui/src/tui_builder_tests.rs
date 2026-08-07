#[cfg(feature = "voice_input")]
use std::time::Duration;

use pathfinder_color::ColorU;
use warp::tui_export::{dark_theme, light_theme};
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::Fill as CoreFill;
use warpui_core::elements::tui::{Color, Modifier};

use super::{TuiUiBuilder, rounded_midpoint_color};

fn rgb(value: u32) -> Color {
    let color = ColorU::from_u32(value);
    Color::Rgb(color.r, color.g, color.b)
}

#[test]
fn design_palettes_match_figma_in_dark_and_light_themes() {
    for (theme, brand_primary, brand_accent, agent_colors) in [
        (
            dark_theme(),
            0xD2B5FFFF,
            0xE2FFD4FF,
            [
                0xD0D1FEFF, 0xA5D5FEFF, 0xFF8FFDFF, 0xD2B5FFFF, 0xFF8AA6FF, 0xE2FFD4FF, 0xFBDC79FF,
            ],
        ),
        (
            light_theme(),
            0x9C58F0FF,
            0x33770BFF,
            [
                0x20A5BAFF, 0x008EC4FF, 0x523C79FF, 0x9C58F0FF, 0xFF8AA6FF, 0x33770BFF, 0xC79A18FF,
            ],
        ),
    ] {
        let builder = TuiUiBuilder { warp_theme: theme };

        assert_eq!(builder.brand_primary_style().fg, Some(rgb(brand_primary)));
        assert_eq!(builder.brand_accent_style().fg, Some(rgb(brand_accent)));
        assert_eq!(
            builder.warping_base_color(),
            ColorU::from_u32(brand_primary)
        );
        assert_eq!(
            builder
                .agent_identity_palette()
                .into_iter()
                .take(agent_colors.len())
                .map(|identity| identity.style.fg)
                .collect::<Vec<_>>(),
            agent_colors
                .into_iter()
                .map(|color| Some(rgb(color)))
                .collect::<Vec<_>>()
        );
    }
}

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
    let expected_read_only_menu_label: Color = CoreFill::from(
        theme
            .background()
            .blend(&theme.foreground().with_opacity(60)),
    )
    .into();

    assert_eq!(builder.primary_text_style().fg, Some(expected_primary));
    let expected_neutral_7: Color =
        CoreFill::from(ThemeFill::Solid(internal_colors::neutral_7(&theme))).into();
    assert_eq!(builder.neutral_7_text_style().fg, Some(expected_neutral_7));
    assert!(
        !builder
            .neutral_7_text_style()
            .add_modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(builder.muted_text_style().fg, Some(expected_muted));
    let read_only_menu_label_style = builder.read_only_menu_label_style();
    assert_eq!(
        read_only_menu_label_style.fg,
        Some(expected_read_only_menu_label)
    );
    assert!(
        !read_only_menu_label_style
            .add_modifier
            .contains(Modifier::DIM)
    );
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
    let shell_command_fill = ThemeFill::from(theme.terminal_colors().bright.green);
    let shell_command_background: Color = CoreFill::from(
        theme
            .background()
            .blend(&shell_command_fill.with_opacity(10)),
    )
    .into();
    let shortcut_accent = ThemeFill::from(theme.terminal_colors().normal.cyan);
    let read_only_menu_background: Color =
        CoreFill::from(theme.background().blend(&shortcut_accent.with_opacity(10))).into();
    assert_eq!(
        builder.read_only_menu_background(),
        read_only_menu_background
    );
    assert_eq!(builder.shell_command_background(), shell_command_background);
    let shell_command_prefix_style = builder.shell_command_prefix_style();
    assert_eq!(
        shell_command_prefix_style.fg,
        Some(CoreFill::from(shell_command_fill).into())
    );
    assert_eq!(shell_command_prefix_style.bg, None);
    assert!(
        shell_command_prefix_style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    let shell_command_row_style = builder.shell_command_row_style();
    assert_eq!(shell_command_row_style.fg, shell_command_prefix_style.fg);
    assert_eq!(shell_command_row_style.bg, Some(shell_command_background));
    assert!(
        shell_command_row_style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    let selection_style = builder.slash_command_selection_text_style();
    assert_eq!(selection_style.fg, Some(selection_foreground));
    assert_eq!(selection_style.bg, Some(selection_background));
    assert!(selection_style.add_modifier.contains(Modifier::BOLD));

    let text_selection_style = builder.selection_style();
    assert!(
        text_selection_style
            .sub_modifier
            .contains(Modifier::REVERSED)
    );
    let background = theme.background().into_solid();
    let green = ThemeFill::from(theme.terminal_colors().normal.green).into_solid();
    let selected_state_suffix_color: Color =
        CoreFill::from(ThemeFill::Solid(rounded_midpoint_color(background, green))).into();
    assert_eq!(
        builder.slash_command_selection_state_suffix_style().fg,
        Some(selected_state_suffix_color)
    );
}

#[test]
fn selected_state_suffix_midpoint_matches_figma_dark_palette() {
    assert_eq!(
        rounded_midpoint_color(
            ColorU::new(5, 5, 5, u8::MAX),
            ColorU::new(180, 250, 114, u8::MAX),
        ),
        ColorU::new(93, 128, 60, u8::MAX)
    );
}

#[test]
#[cfg(feature = "voice_input")]
fn voice_input_border_pulses_between_cyan_overlay_2_and_lilac_600() {
    let theme = light_theme();
    let builder = TuiUiBuilder {
        warp_theme: theme.clone(),
    };
    let cyan_fill = ThemeFill::from(theme.terminal_colors().normal.cyan);
    let cyan: Color = CoreFill::from(cyan_fill).into();
    let lilac_600: Color =
        CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.magenta)).into();
    let cyan_overlay_2: Color =
        CoreFill::from(theme.background().blend(&cyan_fill.with_opacity(50))).into();

    assert_eq!(builder.voice_input_status_style().fg, Some(cyan));
    assert_eq!(
        builder.voice_input_border_style(Duration::ZERO).fg,
        Some(cyan_overlay_2)
    );
    assert_eq!(
        builder.voice_input_border_style(Duration::from_secs(1)).fg,
        Some(lilac_600)
    );
    assert_eq!(
        builder.voice_input_border_style(Duration::from_secs(2)).fg,
        Some(cyan_overlay_2)
    );
}

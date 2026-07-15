use warp::tui_export::{light_theme, Appearance};
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::{App, AppContext};

use super::TuiUiBuilder;

fn render_buffer(
    ctx: &AppContext,
    mut element: impl TuiElement,
    width: u16,
    height: u16,
) -> TuiBuffer {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(width, height)),
        &mut layout_ctx,
        ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    let mut surface = TuiPaintSurface::new(&mut buffer);
    element.render(
        TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
        &mut surface,
        &mut paint_ctx,
    );
    buffer
}

#[test]
fn idle_navigation_hint_renders_key_and_action_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let buffer = render_buffer(ctx, builder.render_idle_navigation_hint(), 80, 1);
            let line = buffer.to_lines()[0].trim_end().to_owned();

            assert_eq!(line, "↑ to edit  Esc to stop  ← for conversations");

            let key_color = builder.key_hint_style().fg.expect("key hint foreground");
            let action_color = builder
                .muted_text_style()
                .fg
                .expect("action hint foreground");
            let escape_col = line
                .chars()
                .position(|character| character == 'E')
                .expect("escape hint") as u16;
            let conversations_col = line
                .chars()
                .position(|character| character == '←')
                .expect("conversation hint") as u16;
            assert_eq!(buffer[(0, 0)].fg, key_color);
            assert_eq!(buffer[(1, 0)].fg, action_color);
            assert_eq!(buffer[(escape_col, 0)].fg, key_color);
            assert_eq!(buffer[(escape_col + 3, 0)].fg, action_color);
            assert_eq!(buffer[(conversations_col, 0)].fg, key_color);
            assert_eq!(buffer[(conversations_col + 1, 0)].fg, action_color);
        });
    });
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
    assert_eq!(
        builder.slash_command_selection_background(),
        selection_background
    );
    let selection_style = builder.slash_command_selection_text_style();
    assert_eq!(selection_style.fg, Some(selection_foreground));
    assert_eq!(selection_style.bg, Some(selection_background));
    assert!(selection_style.add_modifier.contains(Modifier::BOLD));
}

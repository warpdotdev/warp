use warp::tui_export::Appearance;
use warpui_core::App;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiElement, TuiFlex, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::TuiLink;

#[test]
fn link_renders_visible_underlined_text() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let label = "https://example.com/run";
            let link = TuiLink::default();
            let style = crate::tui_builder::TuiUiBuilder::from_app(ctx).muted_text_style();
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                link.render(label, style, |_, _| {}),
                TuiRect::new(0, 0, 40, 1),
                ctx,
            );
            assert!(frame.buffer.to_lines()[0].starts_with(label));
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::UNDERLINED));
        });
    });
}

#[test]
fn link_row_in_stretched_banner_only_underlines_the_link_text() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let label = "https://example.com/run";
            let link = TuiLink::default();
            let style = crate::tui_builder::TuiUiBuilder::from_app(ctx).muted_text_style();
            let banner_content = TuiFlex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    TuiFlex::row()
                        .child(link.render(label, style, |_, _| {}))
                        .finish(),
                )
                .finish();
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(banner_content, TuiRect::new(0, 0, 40, 1), ctx);

            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::UNDERLINED));
            assert!(
                !frame.buffer[(label.len() as u16, 0)]
                    .modifier
                    .contains(Modifier::UNDERLINED)
            );
        });
    });
}

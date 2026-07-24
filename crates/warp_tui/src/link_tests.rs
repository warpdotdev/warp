use warp::tui_export::Appearance;
use warpui_core::App;
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::TuiLink;

#[test]
fn link_renders_visible_underlined_text() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let link = TuiLink::default();
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                link.render("https://example.com/run", ctx, |_, _| {}),
                TuiRect::new(0, 0, 40, 1),
                ctx,
            );
            assert!(frame.buffer.to_lines()[0].starts_with("https://example.com/run"));
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::UNDERLINED));
        });
    });
}

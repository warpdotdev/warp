use warp::appearance::Appearance;
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::render_metadata_line;
use crate::tui_builder::TuiUiBuilder;

#[test]
fn metadata_matches_the_orchestration_card_layout() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let metadata = render_metadata_line(
                "Wob the Wuilder".to_owned(),
                "gpt-5.6-sol (high)".to_owned(),
                &builder,
            );
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(metadata, TuiRect::new(0, 0, 80, 1), ctx);
            assert_eq!(
                frame.buffer.to_lines()[0].trim_end(),
                "Environment: Wob the Wuilder  •  Model: gpt-5.6-sol (high)"
            );
        });
    });
}

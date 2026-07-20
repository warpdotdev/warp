use warp::appearance::Appearance;
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::{compact_footer_path, conversation_restoring};

#[test]
fn compact_footer_path_preserves_short_paths() {
    assert_eq!(compact_footer_path("/erica/project"), "/erica/project");
}

#[test]
fn compact_footer_path_elides_middle_components() {
    assert_eq!(compact_footer_path("~/Documents/GitHub/warp"), "~/…/warp");
    assert_eq!(compact_footer_path("/long/path/to/project"), "/…/project");
    assert_eq!(
        compact_footer_path(r"C:\Users\erica\project"),
        r"C:\…\project"
    );
}

#[test]
fn conversation_loader_is_centered_and_animated() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let element = conversation_restoring(app_ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 60, 7), app_ctx);
            let lines = frame.buffer.to_lines();
            let label = lines
                .iter()
                .find(|line| line.contains("Loading session..."))
                .expect("loading label should render");
            assert!(
                lines.iter().any(|line| {
                    line.contains("Esc or Ctrl-C to cancel and start a new session")
                })
            );

            assert!(
                label.find("Loading session...").is_some_and(|x| x > 0),
                "loading label should be horizontally centered: {label:?}"
            );
            assert!(
                frame.repaint_at.is_some(),
                "loading spinner should schedule a repaint"
            );
        });
    });
}

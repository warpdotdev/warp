use warp::appearance::Appearance;
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::{compact_footer_path, conversation_restoring, login_placeholder};

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

#[test]
fn login_placeholder_is_centered_for_all_states() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let states = [
                (None, None, "Opening your browser…"),
                (
                    Some("https://warp.dev/device"),
                    None,
                    "Open https://warp.dev/device in your browser",
                ),
                (
                    Some("https://warp.dev/device"),
                    Some("ABC-123"),
                    "and enter code: ABC-123",
                ),
            ];

            for (verification_uri, user_code, expected_line) in states {
                let element = login_placeholder(verification_uri, user_code);
                let mut presenter = TuiPresenter::new();
                let frame = presenter.present_element(element, TuiRect::new(0, 0, 60, 8), app_ctx);
                let lines = frame.buffer.to_lines();
                let rendered_line = lines
                    .iter()
                    .find(|line| line.contains(expected_line))
                    .unwrap_or_else(|| {
                        panic!("login state should render {expected_line:?}: {lines:?}")
                    });
                let text_column = rendered_line
                    .find(expected_line)
                    .map(|byte_offset| rendered_line[..byte_offset].chars().count())
                    .expect("expected login text should be present");
                let row = lines
                    .iter()
                    .position(|line| line.contains(expected_line))
                    .expect("expected login text row should be present");

                assert!(
                    text_column > 0,
                    "login text should be horizontally centered: {rendered_line:?}"
                );
                assert!(
                    row > 0 && row < 7,
                    "login text should remain vertically centered: row={row}, lines={lines:?}"
                );
            }
        });
    });
}

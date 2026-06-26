use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::App;

use super::{render_content_visible_rows, AgentBlockContent};

#[test]
fn simple_agent_block_reports_full_height_and_renders_visible_rows() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let content = AgentBlockContent {
                input: "hello".to_owned(),
                output: "one\ntwo\nthree".to_owned(),
            };
            let rendered = render_content_visible_rows(&content, 2..4, 20);
            assert_eq!(rendered.measured_full_height, Some(4));

            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(rendered.element, TuiRect::new(0, 0, 20, 2), app_ctx);
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["two", "three"],
            );
        });
    });
}

#[test]
fn simple_agent_block_reflows_height_at_narrow_width() {
    let content = AgentBlockContent {
        input: "hello world".to_owned(),
        output: "streamed output".to_owned(),
    };

    let wide = render_content_visible_rows(&content, 0..10, 40);
    let narrow = render_content_visible_rows(&content, 0..10, 6);
    assert!(
        narrow.measured_full_height.unwrap() > wide.measured_full_height.unwrap(),
        "narrow text should occupy more logical rows"
    );
}

use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::App;

use super::{desired_content_height, render_content, AgentBlockContent};

#[test]
fn simple_agent_block_reports_full_height_and_renders_content() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let content = AgentBlockContent {
                input: "hello".to_owned(),
                output: "one\ntwo\nthree".to_owned(),
            };
            assert_eq!(desired_content_height(&content, 20), 4);

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_content(&content),
                TuiRect::new(0, 0, 20, 4),
                app_ctx,
            );
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["hello", "one", "two", "three"],
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

    let wide = desired_content_height(&content, 40);
    let narrow = desired_content_height(&content, 6);
    assert!(narrow > wide, "narrow text should occupy more logical rows");
}

use warp::appearance::Appearance;
use warp::tui_export::{BlocklistAIHistoryModel, CloudAgentStartupBlocker};
use warpui::AddWindowOptions;
use warpui::platform::WindowStyle;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView as _};

use super::TuiCloudRunView;
use crate::cloud_run::TuiCloudRunState;
use crate::test_fixtures::TestHostView;

#[test]
fn lightweight_cloud_view_renders_startup_and_blocker_without_terminal_state() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let window_id = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            )
            .0
        });
        let state = app.add_model(|_| TuiCloudRunState::new());
        let view = app.update(|ctx| {
            ctx.add_typed_action_tui_view(window_id, |ctx| TuiCloudRunView::new(state.clone(), ctx))
        });
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 80, 12),
                ctx,
            );
            assert!(
                frame
                    .buffer
                    .to_lines()
                    .iter()
                    .any(|line| line.contains("Starting cloud run…"))
            );
        });

        app.update(|ctx| {
            state.update(ctx, |state, ctx| {
                state.set_blocked(
                    CloudAgentStartupBlocker::GitHubAuthRequired {
                        message: "GitHub authentication required".to_string(),
                        auth_url: "https://example.com/auth".to_string(),
                    },
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 80, 12),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("GitHub authentication required"))
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("https://example.com/auth"))
            );
        });
    });
}

use warp::tui_export::{
    CloudEnvironmentCatalog, HandoffPrepareError, register_tui_session_view_test_singletons,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, SingletonEntity as _, TuiView as _};

use super::{TuiHandoffModel, TuiHandoffPhase};
use crate::handoff::TuiHandoffBlock;
use crate::test_fixtures::TestHostView;

#[test]
fn focusing_a_configuring_block_delegates_to_the_selector() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        let model = app.add_model(|ctx| TuiHandoffModel {
            source_conversation_id: None,
            pending: None,
            phase: TuiHandoffPhase::Configuring {
                page: super::TuiHandoffSelectorKind::Model,
            },
            environments: CloudEnvironmentCatalog::handle(ctx),
            forked_existing_conversation: false,
            validation_error: None,
            next_operation_id: 0,
            dismissed: false,
        });
        let block = app.update(|ctx| {
            let (window_id, _) = ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            );
            ctx.add_typed_action_tui_view(window_id, |ctx| TuiHandoffBlock::new(model, ctx))
        });
        let (window_id, selector_id) = app.read(|ctx| {
            (
                block.window_id(ctx),
                block
                    .as_ref(ctx)
                    .child_view_ids(ctx)
                    .into_iter()
                    .next()
                    .expect("handoff block has a selector"),
            )
        });

        block.update(&mut app, |_, ctx| ctx.focus_self());

        assert_eq!(app.focused_view_id(window_id), Some(selector_id));
    });
}

#[test]
fn missing_token_after_eager_cancellation_restores_only_trimmed_argument() {
    let argument = "  keep this prompt  ".to_owned();
    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::MissingServerConversationToken,
        true,
        Some(&argument),
    )
    .into_parts();
    assert_eq!(replacement.as_deref(), Some("keep this prompt"));

    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::MissingServerConversationToken,
        true,
        None,
    )
    .into_parts();
    assert_eq!(replacement.as_deref(), Some(""));

    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::LongRunningCommand,
        true,
        Some(&argument),
    )
    .into_parts();
    assert!(
        replacement.is_none(),
        "pre-cancellation guard failures keep the full slash command draft"
    );

    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::MissingServerConversationToken,
        false,
        Some(&argument),
    )
    .into_parts();
    assert!(
        replacement.is_none(),
        "idle missing-token failures did not eagerly cancel the source"
    );
}

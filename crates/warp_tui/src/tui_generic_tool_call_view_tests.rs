use std::cell::RefCell;

use futures::channel::oneshot;
use serde_json::json;
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType, AIConversationId,
    BlocklistAIActionEvent, SuggestNewConversationResult, TaskId, queue_tui_permission_action,
};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App};
use warpui_core::elements::tui::{Color, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{TuiView as _, WindowInvalidation};

use super::TuiGenericToolCallView;
use crate::test_fixtures::{TestHostView, add_test_action_model};
use crate::tui_builder::TuiUiBuilder;

#[test]
fn mcp_permission_details_are_structured_and_human_readable() {
    App::test((), |mut app| async move {
        let action_model = add_test_action_model(&mut app);
        let action = AIAgentAction {
            id: AIAgentActionId::from("mcp-action".to_owned()),
            task_id: TaskId::new("task".to_owned()),
            action: AIAgentActionType::CallMCPTool {
                server_id: None,
                name: "create_issue".to_owned(),
                input: json!({
                    "title": "Fix permission UI",
                    "priority": 1,
                }),
            },
            requires_result: true,
        };
        let action_for_queue = action.clone();
        let conversation_id = AIConversationId::new();
        let action_model_for_view = action_model.clone();
        let view = app.update(|ctx| {
            let (window_id, _) = ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            );
            ctx.add_tui_view(window_id, |ctx| {
                TuiGenericToolCallView::new(
                    action,
                    false,
                    action_model_for_view,
                    conversation_id,
                    ctx,
                )
            })
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.permission_prompt.is_none());
            assert_eq!(
                view.permission_question(),
                "Is it OK if I call this MCP tool?"
            );
            let details = view.details();
            assert!(details.starts_with("create_issue\n{"));
            assert!(details.contains("\"priority\": 1"));
            assert!(details.contains("\"title\": \"Fix permission UI\""));
        });

        action_model.update(&mut app, |action_model, ctx| {
            queue_tui_permission_action(action_model, action_for_queue, conversation_id, ctx);
        });
        app.read(|ctx| {
            let prompt = view
                .as_ref(ctx)
                .active_permission_prompt(ctx)
                .expect("blocked action should materialize its permission prompt");
            assert!(prompt.as_ref(ctx).is_active(ctx));
        });
        let mut presenter = TuiPresenter::new();
        let frame = app.update(|ctx| {
            let prompt = view
                .as_ref(ctx)
                .active_permission_prompt(ctx)
                .expect("blocked action should materialize its permission prompt");
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(view.id());
            invalidation.updated.insert(prompt.id());
            invalidation
                .updated
                .extend(prompt.as_ref(ctx).child_view_ids(ctx));
            presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
            presenter.present(ctx, &view, TuiRect::new(0, 0, 80, 16))
        });
        let lines = frame
            .buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let (header_background, surface_background) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            (
                builder.permission_header_background(),
                builder.permission_surface_background(),
            )
        });
        assert_ne!(header_background, surface_background);
        assert_eq!(frame.buffer[(79, 0)].bg, header_background);
        assert_eq!(frame.buffer[(79, 1)].bg, surface_background);
        let footer_row = frame
            .buffer
            .to_lines()
            .iter()
            .position(|line| line.contains("Esc to cancel"))
            .expect("permission footer");
        let footer_row = u16::try_from(footer_row).expect("footer row fits in the TUI");
        assert_eq!(frame.buffer[(79, footer_row)].bg, Color::Reset);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("■ Is it OK if I call this MCP tool?"))
        );
        assert!(lines.iter().any(|line| line.contains("create_issue")));
        assert!(lines.iter().any(|line| line.contains("(1) yes")));
        assert!(lines.iter().any(|line| line.contains("(3) Other")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Esc to cancel  Enter to run"))
        );
    });
}

#[test]
fn accepting_new_conversation_suggestion_completes_the_executor() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
        let action_model = add_test_action_model(&mut app);
        let conversation_id = AIConversationId::new();
        let action = AIAgentAction {
            id: AIAgentActionId::from("suggest-conversation".to_owned()),
            task_id: TaskId::new("task".to_owned()),
            action: AIAgentActionType::SuggestNewConversation {
                message_id: "next-step".to_owned(),
            },
            requires_result: true,
        };
        let action_for_queue = action.clone();
        let action_id = action.id.clone();
        let action_model_for_view = action_model.clone();
        let view = app.update(|ctx| {
            let (window_id, _) = ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            );
            ctx.add_tui_view(window_id, |ctx| {
                TuiGenericToolCallView::new(
                    action,
                    false,
                    action_model_for_view,
                    conversation_id,
                    ctx,
                )
            })
        });
        let (finished_tx, finished_rx) = oneshot::channel();
        let finished_tx = RefCell::new(Some(finished_tx));
        app.update(|ctx| {
            ctx.subscribe_to_model(&action_model, move |_, event, _| {
                if matches!(
                    event,
                    BlocklistAIActionEvent::FinishedAction { action_id: id, .. } if id == &action_id
                ) && let Some(tx) = finished_tx.borrow_mut().take()
                {
                    let _ = tx.send(());
                }
            });
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(model, action_for_queue, conversation_id, ctx);
        });

        view.update(&mut app, |view, ctx| view.accept(ctx));
        finished_rx
            .await
            .expect("accepted suggestion should reach a terminal result");

        app.read(|ctx| {
            let result = action_model
                .as_ref(ctx)
                .get_action_result(&AIAgentActionId::from("suggest-conversation".to_owned()))
                .expect("suggestion result");
            assert!(matches!(
                &result.result,
                AIAgentActionResultType::SuggestNewConversation(
                    SuggestNewConversationResult::Accepted { message_id }
                ) if message_id == "next-step"
            ));
        });
    });
}

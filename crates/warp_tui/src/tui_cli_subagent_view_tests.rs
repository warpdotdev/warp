use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentPtyWriteMode, AIConversationId,
    BlockId, BlocklistAIActionEvent, CancellationReason, LongRunningCommandControlState, TaskId,
    UserTakeOverReason, register_tui_session_view_test_singletons,
};
use warpui_core::App;

use super::{
    BlockedActionPresentation, blocked_action_presentation, cancel_blocked_action,
    display_pty_input, execute_blocked_action, format_next_check_remaining,
    remaining_for_fixed_delay, resolve_latest_instruction, terminal_use_status_text,
};
use crate::test_fixtures::add_test_action_model;

#[test]
fn terminal_use_status_covers_control_and_lifecycle_states() {
    let agent = LongRunningCommandControlState::Agent {
        is_blocked: false,
        should_hide_responses: false,
    };
    assert_eq!(
        terminal_use_status_text(&agent, false, true),
        "Agent is monitoring command · ctrl-c to take control"
    );
    assert_eq!(
        terminal_use_status_text(&agent, false, false),
        "Agent waiting for instructions · ctrl-c to take control"
    );
    assert_eq!(
        terminal_use_status_text(&agent, true, true),
        "Command finished"
    );

    let blocked = LongRunningCommandControlState::Agent {
        is_blocked: true,
        should_hide_responses: false,
    };
    assert_eq!(
        terminal_use_status_text(&blocked, false, true),
        "Agent needs your input"
    );

    let manual = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::Manual,
    };
    assert_eq!(
        terminal_use_status_text(&manual, false, false),
        "User is in control · ctrl-g to hand back"
    );

    let stopped = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::Stop {
            should_auto_resume: true,
        },
    };
    assert_eq!(
        terminal_use_status_text(&stopped, false, false),
        "Agent paused · user is in control · ctrl-g to hand back"
    );

    let transferred = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::TransferFromAgent {
            reason: "enter password".to_owned(),
        },
    };
    assert_eq!(
        terminal_use_status_text(&transferred, false, false),
        "Agent handed control to you · ctrl-g to hand back"
    );
}

fn test_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("terminal-use-task".to_owned()),
        action: AIAgentActionType::WriteToLongRunningShellCommand {
            block_id: BlockId::new(),
            input: b"input".to_vec().into(),
            mode: AIAgentPtyWriteMode::Raw,
        },
        requires_result: true,
    }
}

#[test]
fn allow_executes_the_exact_displayed_action() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        let action_model = add_test_action_model(&mut app);
        let conversation_id = AIConversationId::new();
        let first = test_action("first");
        let displayed = test_action("displayed");
        let executing_ids = Rc::new(RefCell::new(Vec::new()));
        let executing_ids_for_event = executing_ids.clone();

        app.update(|ctx| {
            ctx.subscribe_to_model(&action_model, move |_, event, _| {
                if let BlocklistAIActionEvent::ExecutingAction(action_id) = event {
                    executing_ids_for_event.borrow_mut().push(action_id.clone());
                }
            });
            action_model.update(ctx, |action_model, ctx| {
                action_model.queue_confirmation_action(first, conversation_id, ctx);
                action_model.queue_confirmation_action(displayed.clone(), conversation_id, ctx);
                execute_blocked_action(action_model, conversation_id, &displayed, ctx);
            });
        });

        assert_eq!(executing_ids.borrow().first(), Some(&displayed.id));
    });
}

#[test]
fn reject_cancels_only_the_exact_displayed_action() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        let action_model = add_test_action_model(&mut app);
        let conversation_id = AIConversationId::new();
        let first = test_action("first");
        let displayed = test_action("displayed");
        let finished_actions = Rc::new(RefCell::new(Vec::new()));
        let finished_actions_for_event = finished_actions.clone();

        app.update(|ctx| {
            ctx.subscribe_to_model(&action_model, move |_, event, _| {
                if let BlocklistAIActionEvent::FinishedAction {
                    action_id,
                    cancellation_reason,
                    ..
                } = event
                {
                    finished_actions_for_event
                        .borrow_mut()
                        .push((action_id.clone(), *cancellation_reason));
                }
            });
            action_model.update(ctx, |action_model, ctx| {
                action_model.queue_confirmation_action(first, conversation_id, ctx);
                action_model.queue_confirmation_action(displayed.clone(), conversation_id, ctx);
                cancel_blocked_action(action_model, conversation_id, &displayed, ctx);
            });
        });

        assert!(finished_actions.borrow().iter().any(|(action_id, reason)| {
            action_id == &displayed.id && *reason == Some(CancellationReason::ManuallyCancelled)
        }));
    });
}

#[test]
fn controller_instruction_precedes_stale_exchange_input() {
    assert_eq!(
        resolve_latest_instruction(
            Some("new instruction".to_owned()),
            Some("old instruction".to_owned())
        ),
        Some("new instruction".to_owned())
    );
}

#[test]
fn next_check_countdown_decreases_and_expires() {
    assert_eq!(
        remaining_for_fixed_delay(Duration::from_secs(10), Duration::from_secs(3)),
        Some(Duration::from_secs(7))
    );
    assert_eq!(
        remaining_for_fixed_delay(Duration::from_secs(10), Duration::from_secs(10)),
        None
    );
}

#[test]
fn next_check_countdown_formats_seconds_and_minutes() {
    assert_eq!(
        format_next_check_remaining(Duration::from_secs(12)),
        " · Check in 12s"
    );
    assert_eq!(
        format_next_check_remaining(Duration::from_secs(65)),
        " · Check in 1m"
    );
}

#[test]
fn write_action_presentation_shows_input_and_mode_without_internal_ids() {
    let action = AIAgentActionType::WriteToLongRunningShellCommand {
        block_id: BlockId::new(),
        input: b"iRoses\nViolets\x1b".to_vec().into(),
        mode: AIAgentPtyWriteMode::Raw,
    };

    let presentation = blocked_action_presentation(&action);

    assert_eq!(
        presentation,
        BlockedActionPresentation {
            summary: "Agent wants to write to the running command".to_owned(),
            detail: Some("Input:\niRoses\nViolets<Esc>".to_owned()),
        }
    );
    assert!(!presentation.summary.contains("block id"));
    assert!(!presentation.detail.unwrap().contains("block id"));
}

#[test]
fn transfer_action_presentation_shows_the_agents_reason() {
    let presentation =
        blocked_action_presentation(&AIAgentActionType::TransferShellCommandControlToUser {
            reason: "Enter the sudo password".to_owned(),
        });

    assert_eq!(
        presentation,
        BlockedActionPresentation {
            summary: "Agent wants to hand command control to you".to_owned(),
            detail: Some("Reason: Enter the sudo password".to_owned()),
        }
    );
}

#[test]
fn pty_input_display_names_control_bytes_and_preserves_lines() {
    assert_eq!(
        display_pty_input(b"first\r\nsecond\x03"),
        "first<Enter>\nsecond<0x03>"
    );
}

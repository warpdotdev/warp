use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ai::agent::action::{
    CreateDocumentsRequest, DocumentDiff, DocumentToCreate, EditDocumentsRequest,
};
use ai::document::AIDocumentId;
use markdown_parser::parse_markdown;
use parking_lot::FairMutex;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentExchangeId, AIAgentInput, AIAgentOutput, AIAgentOutputMessage,
    AIAgentOutputMessageType, AIAgentText, AIAgentTextSection, AIAgentTodo, AIAgentTodoList,
    AIBlockModel, AIBlockOutputStatus, AIConversationId, AIRequestType, AgentOutputImage,
    AgentOutputImageLayout, AgentOutputMermaidDiagram, AgentOutputTable, Appearance, LLMId,
    MessageId, OutputStatusUpdateCallback, RequestCommandOutputResult, ServerOutputId, Shared,
    SummarizationType, TaskId, TerminalModel, TodoOperation, TodoStatus, UserQueryMode,
};
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, SingletonEntity};
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiRect, TuiScreenPosition,
    TuiSize,
};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::event::ModifiersState;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, EntityIdMap, TuiView, ViewContext, ViewHandle};

use super::{
    CollapsibleSectionStates, TuiAIBlock, TuiAIBlockAction, TuiAIBlockEvent, TuiAIBlockSection,
    TuiCodeBlockKey, TuiRichTextSection, TuiToolCallView,
};
use crate::agent_block_sections::{
    completed_todos_label, render_fallback_tool_call_section, render_todo_list_section,
};
use crate::test_fixtures::{add_test_action_model_and_events, TestHostView};
use crate::tui_plan_view::TuiPlanViewAction;
use crate::tui_shell_command_view::TuiShellCommandViewAction;

#[test]
fn simple_agent_block_reports_full_height_and_renders_content() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: vec![query_input("hello")],
                status: complete_output(vec![AIAgentTextSection::PlainText {
                    text: "one\ntwo\nthree".to_owned().into(),
                }]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(desired_height(block, 20, app_ctx), 6);

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                block.render_element(app_ctx),
                TuiRect::new(0, 0, 20, 6),
                app_ctx,
            );
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["", "> hello", "", "one", "two", "three"],
            );
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                expected_prompt_prefix_color(app_ctx)
            );
            assert_eq!(frame.buffer[(0, 1)].bg, expected_input_background(app_ctx));
            assert!(frame.buffer[(0, 1)].modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(2, 1)].fg, expected_prompt_text_color(app_ctx));
            assert_eq!(frame.buffer[(19, 1)].bg, expected_input_background(app_ctx));
            assert_eq!(frame.buffer[(0, 3)].fg, expected_output_text_color(app_ctx));
            // The block paints no background of its own, so output rows show the
            // terminal's own background.
            assert_eq!(frame.buffer[(0, 3)].bg, Color::Reset);
            assert_eq!(frame.buffer[(19, 3)].bg, Color::Reset);
        });
    });
}

#[test]
fn agent_block_uses_fallback_row_for_edit_without_rich_body() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let edit = test_edit_documents_action("edit-1");
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message("message-1", edit)]),
            },
        );

        app.read(|ctx| {
            let rendered = render_block_lines(block.as_ref(ctx), 60, ctx);
            assert_eq!(rendered.len(), 1);
            assert!(rendered[0].contains("Update"));
        });
    });
}

#[test]
fn simple_agent_block_reflows_height_at_narrow_width() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: vec![query_input("hello world")],
                status: complete_output(vec![AIAgentTextSection::PlainText {
                    text: "streamed output".to_owned().into(),
                }]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            let wide = desired_height(block, 40, app_ctx);
            let narrow = desired_height(block, 6, app_ctx);
            assert!(narrow > wide, "narrow text should occupy more logical rows");
        });
    });
}

fn expected_prompt_text_color(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    CoreFill::from(theme.foreground()).into()
}
fn expected_prompt_prefix_color(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.cyan)).into()
}

fn expected_input_background(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    let accent = ThemeFill::from(theme.terminal_colors().normal.cyan);
    CoreFill::from(
        theme
            .background()
            .blend(&accent.with_opacity(10))
            .blend(&accent.with_opacity(10)),
    )
    .into()
}

fn expected_output_text_color(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    let opacity = theme.details().main_text_opacity;
    CoreFill::from(
        theme
            .background()
            .blend(&theme.foreground().with_opacity(opacity)),
    )
    .into()
}

fn expected_tool_call_text_color(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    let opacity = theme.details().sub_text_opacity;
    CoreFill::from(
        theme
            .background()
            .blend(&theme.foreground().with_opacity(opacity)),
    )
    .into()
}

#[test]
fn agent_block_extracts_input_and_plain_text_from_model() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: vec![query_input("hello")],
                status: complete_output(vec![
                    AIAgentTextSection::PlainText {
                        text: "one".to_owned().into(),
                    },
                    AIAgentTextSection::PlainText {
                        text: "two".to_owned().into(),
                    },
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    TuiAIBlockSection::Input("hello".to_owned()),
                    rich_text("one"),
                    rich_text("two"),
                ]
            );
        });
    });
}

#[test]
fn agent_block_renders_tool_calls_in_message_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action = test_action("action-1");
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    plain_text_message("message-1", "before"),
                    action_message("message-2", action.clone()),
                    plain_text_message("message-3", "after"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    rich_text("before"),
                    TuiAIBlockSection::ToolCall(Box::new(action.clone())),
                    rich_text("after"),
                ]
            );

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                block.render_element(app_ctx),
                TuiRect::new(0, 0, 40, 6),
                app_ctx,
            );
            // The block starts with one row of top padding, and a blank row
            // separates adjacent sections.
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["", "before", "", "○ Init project", "", "after"],
            );
            // A pending tool call renders a dim grey glyph and a dim label.
            assert_eq!(
                frame.buffer[(0, 3)].fg,
                expected_tool_call_text_color(app_ctx)
            );
            assert!(frame.buffer[(0, 3)].modifier.contains(Modifier::DIM));
            assert_eq!(
                frame.buffer[(2, 3)].fg,
                expected_tool_call_text_color(app_ctx)
            );
            assert!(frame.buffer[(2, 3)].modifier.contains(Modifier::DIM));
        });
    });
}

#[test]
fn agent_block_renders_multiple_tool_calls_in_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let first = test_action("action-1");
        let second = test_action("action-2");
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    action_message("message-1", first.clone()),
                    action_message("message-2", second.clone()),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    TuiAIBlockSection::ToolCall(Box::new(first)),
                    TuiAIBlockSection::ToolCall(Box::new(second)),
                ]
            );

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                block.render_element(app_ctx),
                TuiRect::new(0, 0, 40, 4),
                app_ctx,
            );
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["", "○ Init project", "", "○ Init project"],
            );
        });
    });
}

#[test]
fn tool_call_row_glyph_and_colors_reflect_state() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let theme = Appearance::as_ref(app_ctx).theme();
            let green: Color =
                CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.green)).into();
            let yellow: Color =
                CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.yellow)).into();
            let red: Color =
                CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.red)).into();
            let primary = expected_output_text_color(app_ctx);
            let muted = expected_tool_call_text_color(app_ctx);

            let render = |action: &AIAgentAction, status: Option<&AIActionStatus>| {
                let mut presenter = TuiPresenter::new();
                presenter.present_element(
                    render_fallback_tool_call_section(action, status, false, None, app_ctx),
                    TuiRect::new(0, 0, 40, 1),
                    app_ctx,
                )
            };

            // Succeeded: green check in the gutter, normal-foreground label.
            let action = test_action("action-1");
            let succeeded = finished_status(&action, AIAgentActionResultType::InitProject);
            let frame = render(&action, Some(&succeeded));
            assert_eq!(
                frame.buffer.to_lines()[0].trim_end(),
                "✓ Init project — done"
            );
            assert_eq!(frame.buffer[(0, 0)].fg, green);
            assert_eq!(frame.buffer[(2, 0)].fg, primary);
            assert!(!frame.buffer[(2, 0)].modifier.contains(Modifier::DIM));

            // Running: yellow dot.
            let frame = render(&action, Some(&AIActionStatus::RunningAsync));
            assert_eq!(frame.buffer.to_lines()[0].trim_end(), "● Init project…");
            assert_eq!(frame.buffer[(0, 0)].fg, yellow);
            assert_eq!(frame.buffer[(2, 0)].fg, primary);

            // Failed (denylisted command): red x, normal-foreground label.
            let command_action = test_command_action("action-2", "git status");
            let failed = finished_status(
                &command_action,
                AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::Denylisted {
                        command: "git status".to_owned(),
                    },
                ),
            );
            let frame = render(&command_action, Some(&failed));
            assert_eq!(
                frame.buffer.to_lines()[0].trim_end(),
                "× `git status` denied (denylisted)"
            );
            assert_eq!(frame.buffer[(0, 0)].fg, red);
            assert_eq!(frame.buffer[(2, 0)].fg, primary);

            // Cancelled: grey block, normal-foreground label.
            let cancelled = finished_status(
                &command_action,
                AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::CancelledBeforeExecution,
                ),
            );
            let frame = render(&command_action, Some(&cancelled));
            assert_eq!(
                frame.buffer.to_lines()[0].trim_end(),
                "■ Cancelled `git status`"
            );
            assert_eq!(frame.buffer[(0, 0)].fg, muted);
            assert!(!frame.buffer[(0, 0)].modifier.contains(Modifier::DIM));
            assert_eq!(frame.buffer[(2, 0)].fg, primary);
        });
    });
}

#[test]
fn agent_block_desired_height_accounts_for_tool_call_stub() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message(
                    "message-1",
                    test_action("action-1"),
                )]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            // One tool-call stub line plus the block's top padding row.
            assert_eq!(desired_height(block, 40, app_ctx), 2);
        });
    });
}

#[test]
fn shell_command_disclosure_invalidates_agent_block_layout() {
    App::test((), |mut app| async move {
        let action = test_command_action("action-1", "printf result");
        let action_id = action.id.clone();
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message("message-1", action)]),
            },
        );
        let layout_invalidations = Rc::new(Cell::new(0));
        let invalidations_for_subscription = layout_invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&block, move |_, event, _| match event {
                TuiAIBlockEvent::LayoutInvalidated => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
                TuiAIBlockEvent::BlockingStateChanged => {}
            });
        });

        let shell_view = app.read(|ctx| {
            let Some(TuiToolCallView::ShellCommand(view)) =
                block.as_ref(ctx).action_views.get(&action_id)
            else {
                panic!("shell-command child view");
            };
            view.clone()
        });
        app.update(|ctx| {
            let window_id = shell_view.window_id(ctx);
            ctx.dispatch_typed_action_for_view(
                window_id,
                shell_view.id(),
                &TuiShellCommandViewAction::ToggleExpanded,
            );
        });

        assert_eq!(layout_invalidations.get(), 1);
    });
}

#[test]
fn agent_block_registers_create_and_edit_plan_children() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let create = test_create_documents_action(
            "create-1",
            vec![DocumentToCreate {
                title: "Plan".to_owned(),
                content: "# Overview\n\nRich body".to_owned(),
            }],
        );
        let edit = test_edit_documents_action("edit-1");
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    action_message("message-1", create.clone()),
                    action_message("message-2", edit.clone()),
                ]),
            },
        );

        app.read(|ctx| {
            let block = block.as_ref(ctx);
            let Some(TuiToolCallView::Plan(create_view)) = block.action_views.get(&create.id)
            else {
                panic!("create action has a plan child");
            };
            let Some(TuiToolCallView::Plan(edit_view)) = block.action_views.get(&edit.id) else {
                panic!("edit action has a plan child");
            };
            assert!(create_view.as_ref(ctx).renders_rich_body());
            assert!(!edit_view.as_ref(ctx).renders_rich_body());
            assert_eq!(block.child_view_ids(ctx).len(), 2);
            let rendered = render_tui_view_lines(create_view.as_ref(ctx), 60, 20, ctx);
            assert!(rendered.iter().any(|line| line.trim() == "Overview"));
            assert!(rendered.iter().any(|line| line.trim() == "Rich body"));
        });
    });
}

#[test]
fn plan_collapse_invalidates_agent_block_layout() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action = test_create_documents_action(
            "create-1",
            vec![DocumentToCreate {
                title: "Plan".to_owned(),
                content: "body".to_owned(),
            }],
        );
        let action_id = action.id.clone();
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message("message-1", action)]),
            },
        );
        let layout_invalidations = Rc::new(Cell::new(0));
        let invalidations_for_subscription = layout_invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&block, move |_, event, _| match event {
                TuiAIBlockEvent::LayoutInvalidated => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
                TuiAIBlockEvent::BlockingStateChanged => {}
            });
        });

        let plan_view = app.read(|ctx| {
            let Some(TuiToolCallView::Plan(view)) = block.as_ref(ctx).action_views.get(&action_id)
            else {
                panic!("create action has a plan child");
            };
            view.clone()
        });
        app.update(|ctx| {
            ctx.dispatch_typed_action_for_view(
                plan_view.window_id(ctx),
                plan_view.id(),
                &TuiPlanViewAction::SetCollapsed(true),
            );
        });

        assert_eq!(layout_invalidations.get(), 1);
        app.read(|ctx| {
            assert_eq!(
                render_tui_view_lines(plan_view.as_ref(ctx), 40, 5, ctx),
                vec!["○ Create plan ▸"]
            );
        });
    });
}

#[test]
fn keyboard_toggle_targets_latest_exposed_plan_in_message_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let first = test_create_documents_action(
            "create-1",
            vec![DocumentToCreate {
                title: "First".to_owned(),
                content: "first body".to_owned(),
            }],
        );
        let second = test_create_documents_action(
            "create-2",
            vec![DocumentToCreate {
                title: "Second".to_owned(),
                content: "second body".to_owned(),
            }],
        );
        let first_id = first.id.clone();
        let second_id = second.id.clone();
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    action_message("message-1", first),
                    action_message("message-2", second),
                ]),
            },
        );

        app.read(|ctx| assert!(block.as_ref(ctx).has_exposed_plan(ctx)));
        assert!(block.update(&mut app, |block, ctx| block.toggle_latest_plan(ctx)));

        app.read(|ctx| {
            let block = block.as_ref(ctx);
            let Some(TuiToolCallView::Plan(first)) = block.action_views.get(&first_id) else {
                panic!("first action has a plan child");
            };
            let Some(TuiToolCallView::Plan(second)) = block.action_views.get(&second_id) else {
                panic!("second action has a plan child");
            };
            assert!(render_tui_view_lines(first.as_ref(ctx), 40, 8, ctx)
                .iter()
                .any(|line| line.trim() == "first body"));
            assert_eq!(
                render_tui_view_lines(second.as_ref(ctx), 40, 8, ctx),
                vec!["○ Create plan ▸"]
            );
        });
    });
}
#[test]
fn ask_user_question_action_registers_a_stateful_child_view() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action = ask_user_question_action("ask-1", "Which one?");
        let action_id = action.id.clone();
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message("message-1", action)]),
            },
        );
        app.read(|ctx| {
            assert!(matches!(
                block.as_ref(ctx).action_views.get(&action_id),
                Some(TuiToolCallView::AskQuestion(_))
            ));
        });
    });
}

#[test]
fn streamed_ask_user_question_payload_replaces_the_initial_empty_child_view() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action_id = AIAgentActionId::from("ask-1".to_owned());
        let initial_action = AIAgentAction {
            id: action_id.clone(),
            task_id: TaskId::new("task-1".to_owned()),
            action: AIAgentActionType::AskUserQuestion {
                questions: Vec::new(),
            },
            requires_result: true,
        };
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message("message-1", initial_action)]),
            },
        );
        let initial_view_id = app.read(|ctx| {
            let Some(TuiToolCallView::AskQuestion(view)) =
                block.as_ref(ctx).action_views.get(&action_id)
            else {
                panic!("initial ask-question child view");
            };
            assert!(view.as_ref(ctx).matches_action(&action_id, &[]));
            view.id()
        });

        block.update(&mut app, |block, ctx| {
            block.replace_model(
                block.conversation_id,
                Rc::new(FakeAgentBlockModel {
                    inputs: Vec::new(),
                    status: complete_output_messages(vec![action_message(
                        "message-1",
                        ask_user_question_action("ask-1", "Which one?"),
                    )]),
                }),
            );
            let action_model = block.action_model.clone();
            block.sync_action_views(&action_model, ctx);
        });

        app.read(|ctx| {
            let Some(TuiToolCallView::AskQuestion(view)) =
                block.as_ref(ctx).action_views.get(&action_id)
            else {
                panic!("updated ask-question child view");
            };
            assert_ne!(view.id(), initial_view_id);
            assert!(view
                .as_ref(ctx)
                .matches_action(&action_id, &ask_user_question_items("Which one?")));
        });
    });
}
#[test]
fn agent_block_ignores_unsupported_message_variants() {
    App::test((), |mut app| async move {
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    plain_text_message("message-1", "before"),
                    debug_output_message("message-2", "debug noise"),
                    plain_text_message("message-3", "after"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![rich_text("before"), rich_text("after"),]
            );
        });
    });
}

#[test]
fn agent_block_preserves_and_renders_code_sections_in_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output(vec![
                    AIAgentTextSection::Code {
                        code: "println!(\"hi\");".to_owned(),
                        language: None,
                        source: None,
                    },
                    AIAgentTextSection::PlainText {
                        text: "visible".to_owned().into(),
                    },
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            let code_key = TuiCodeBlockKey {
                message_id: MessageId::new("message-1".to_owned()),
                section_index: 0,
            };
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    TuiAIBlockSection::RichText(TuiRichTextSection::Code(code_key.clone())),
                    rich_text("visible"),
                ]
            );
            assert!(block.code_block_views.contains_key(&code_key));
            assert_eq!(block.child_view_ids(app_ctx).len(), 1);
            let code_view = block.code_block_views[&code_key].as_ref(app_ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                code_view.render(app_ctx),
                TuiRect::new(0, 0, 40, 3),
                app_ctx,
            );
            assert!(frame
                .buffer
                .to_lines()
                .iter()
                .any(|line| line.contains("println!")));

            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered.last().map(String::as_str), Some("visible"));
        });
    });
}

#[test]
fn agent_block_preserves_table_image_and_mermaid_source_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let table_content = "Name\tValue\nAlpha\t1";
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![text_message(
                    "rich-1",
                    vec![
                        AIAgentTextSection::PlainText {
                            text: "before".to_owned().into(),
                        },
                        AIAgentTextSection::Table {
                            table: AgentOutputTable::legacy(table_content.to_owned()),
                        },
                        AIAgentTextSection::Image {
                            image: AgentOutputImage {
                                alt_text: "architecture".to_owned(),
                                source: "diagram.png".to_owned(),
                                title: None,
                                markdown_source: "![architecture](diagram.png)".to_owned(),
                                layout: AgentOutputImageLayout::Block,
                            },
                        },
                        AIAgentTextSection::MermaidDiagram {
                            diagram: AgentOutputMermaidDiagram {
                                source: "graph TD\nA-->B".to_owned(),
                                markdown_source: "```mermaid\ngraph TD\nA-->B\n```".to_owned(),
                            },
                        },
                        AIAgentTextSection::PlainText {
                            text: "after".to_owned().into(),
                        },
                    ],
                )]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            let mermaid_key = TuiCodeBlockKey {
                message_id: MessageId::new("rich-1".to_owned()),
                section_index: 3,
            };
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    rich_text("before"),
                    TuiAIBlockSection::RichText(TuiRichTextSection::Table {
                        structured: None,
                        fallback: table_content.to_owned(),
                    }),
                    TuiAIBlockSection::RichText(TuiRichTextSection::Image {
                        alt_text: "architecture".to_owned(),
                        source: "diagram.png".to_owned(),
                    }),
                    TuiAIBlockSection::RichText(TuiRichTextSection::Code(mermaid_key.clone())),
                    rich_text("after"),
                ]
            );
            assert!(block.code_block_views.contains_key(&mermaid_key));

            let rendered = render_block_lines(block, 40, app_ctx);
            let joined = rendered.join("\n");
            assert!(joined.contains("Name"));
            assert!(joined.contains("Image: architecture (diagram.png)"));
            assert!(joined.ends_with("after"));
        });
    });
}

#[test]
fn code_children_reconcile_across_streamed_section_boundaries() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![text_message(
                    "stream-1",
                    vec![AIAgentTextSection::Code {
                        code: "first".to_owned(),
                        language: None,
                        source: None,
                    }],
                )]),
            },
        );
        let old_key = TuiCodeBlockKey {
            message_id: MessageId::new("stream-1".to_owned()),
            section_index: 0,
        };
        let original_id = app.read(|ctx| block.as_ref(ctx).code_block_views[&old_key].id());
        let invalidations = Rc::new(Cell::new(0));
        let invalidations_for_subscription = invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&block, move |_, event, _| match event {
                TuiAIBlockEvent::LayoutInvalidated => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
                TuiAIBlockEvent::BlockingStateChanged => {}
            });
        });

        block.update(&mut app, |block, ctx| {
            block.block_model = Rc::new(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![text_message(
                    "stream-1",
                    vec![AIAgentTextSection::Code {
                        code: "second".to_owned(),
                        language: None,
                        source: None,
                    }],
                )]),
            });
            block.sync_code_block_views(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                block.as_ref(ctx).code_block_views[&old_key].id(),
                original_id
            );
        });
        assert!(invalidations.get() > 0);

        let new_key = TuiCodeBlockKey {
            message_id: MessageId::new("stream-1".to_owned()),
            section_index: 1,
        };
        block.update(&mut app, |block, ctx| {
            block.block_model = Rc::new(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![text_message(
                    "stream-1",
                    vec![
                        AIAgentTextSection::PlainText {
                            text: "prefix".to_owned().into(),
                        },
                        AIAgentTextSection::Code {
                            code: "second".to_owned(),
                            language: None,
                            source: None,
                        },
                    ],
                )]),
            });
            block.sync_code_block_views(ctx);
        });
        app.read(|ctx| {
            let block = block.as_ref(ctx);
            assert!(!block.code_block_views.contains_key(&old_key));
            assert!(block.code_block_views.contains_key(&new_key));
            assert_ne!(block.code_block_views[&new_key].id(), original_id);
        });

        block.update(&mut app, |block, ctx| {
            block.block_model = Rc::new(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![plain_text_message("stream-1", "finished")]),
            });
            block.sync_code_block_views(ctx);
        });
        app.read(|ctx| assert!(block.as_ref(ctx).code_block_views.is_empty()));
    });
}
#[test]
fn streaming_reasoning_renders_thinking_header_with_body() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: reasoning_status(None, "line one\nline two"),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![TuiAIBlockSection::Thinking {
                    message_id: MessageId::new("reasoning-1".to_owned()),
                    finished_duration: None,
                    body: rich_body("line one\nline two"),
                }]
            );

            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "Thinking... ▾");
            // Body lines are indented four spaces beneath the header.
            assert_eq!(rendered[1], "    line one");
            assert_eq!(rendered[2], "    line two");
        });
    });
}

#[test]
fn finished_reasoning_renders_collapsed_thought_for_header() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: reasoning_status(Some(Duration::from_secs(15)), "hidden body"),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "Thought for 15 seconds ▸");
            // Collapsed by default once finished: the reasoning body is not rendered.
            assert!(rendered.iter().all(|line| !line.contains("hidden body")));
        });
    });
}

/// Duration-only reasoning records do not create empty thinking sections.
#[test]
fn empty_finished_reasoning_is_omitted() {
    App::test((), |mut app| async move {
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    plain_text_message("m1", "before"),
                    reasoning_message("r1", Some(Duration::from_secs(15)), ""),
                    plain_text_message("m2", "after"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![rich_text("before"), rich_text("after")]
            );
        });
    });
}

#[test]
fn manual_expand_override_shows_finished_reasoning_body() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: reasoning_status(Some(Duration::from_secs(2)), "revealed body"),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            // A manual expand wins over the collapsed-when-finished default.
            block
                .collapsible_states
                .set_collapsed(MessageId::new("reasoning-1".to_owned()), false);

            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "Thought for 2 seconds ▾");
            assert!(rendered.iter().any(|line| line.contains("revealed body")));
        });
    });
}

#[test]
fn thinking_action_records_a_manual_collapse_override() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: reasoning_status(None, "body"),
            },
        );
        let message_id = MessageId::new("reasoning-1".to_owned());
        app.update(|ctx| {
            ctx.dispatch_typed_action_for_view(
                block.window_id(ctx),
                block.id(),
                &TuiAIBlockAction::SetSectionCollapsed {
                    message_id: message_id.clone(),
                    collapsed: true,
                },
            );
        });
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert!(block.collapsible_states.is_collapsed(&message_id, false));
        });
    });
}

#[test]
fn reasoning_interleaves_with_plain_text_in_message_order() {
    App::test((), |mut app| async move {
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    plain_text_message("m1", "before"),
                    reasoning_message("r1", None, "thinking"),
                    plain_text_message("m2", "after"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    rich_text("before"),
                    TuiAIBlockSection::Thinking {
                        message_id: MessageId::new("r1".to_owned()),
                        finished_duration: None,
                        body: rich_body("thinking"),
                    },
                    rich_text("after"),
                ]
            );
        });
    });
}

#[test]
fn completed_conversation_summary_renders_collapsed_in_message_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    plain_text_message("m1", "before"),
                    summarization_message(
                        "summary-1",
                        Some(Duration::from_secs(3)),
                        SummarizationType::ConversationSummary,
                        "condensed context",
                    ),
                    plain_text_message("m2", "after"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    rich_text("before"),
                    TuiAIBlockSection::Summarization {
                        message_id: MessageId::new("summary-1".to_owned()),
                        finished: true,
                        body: rich_body("condensed context"),
                    },
                    rich_text("after"),
                ]
            );
            assert_eq!(
                render_block_lines(block, 40, app_ctx),
                vec!["before", "Conversation summarized ▸", "after"]
            );
        });
    });
}

#[test]
fn expanded_conversation_summary_shows_its_body() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![summarization_message(
                    "summary-1",
                    Some(Duration::from_secs(3)),
                    SummarizationType::ConversationSummary,
                    "condensed context",
                )]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            block
                .collapsible_states
                .set_collapsed(MessageId::new("summary-1".to_owned()), false);
            assert_eq!(
                render_block_lines(block, 40, app_ctx),
                vec!["Conversation summarized ▾", "    condensed context"]
            );
        });
    });
}

#[test]
fn tool_call_result_summaries_remain_hidden() {
    App::test((), |mut app| async move {
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![summarization_message(
                    "summary-1",
                    Some(Duration::from_secs(3)),
                    SummarizationType::ToolCallResultSummary,
                    "tool output",
                )]),
            },
        );
        app.read(|app_ctx| {
            assert!(block.as_ref(app_ctx).sections(app_ctx).is_empty());
        });
    });
}

#[test]
fn multiple_reasoning_blocks_render_independent_collapse_state() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    reasoning_message("r1", Some(Duration::from_secs(3)), "done body"),
                    reasoning_message("r2", None, "still going"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            // The finished block collapses; the streaming one stays expanded.
            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "Thought for 3 seconds ▸");
            assert_eq!(rendered[1], "Thinking... ▾");
            assert_eq!(rendered[2], "    still going");
            assert!(rendered.iter().all(|line| !line.contains("done body")));
        });
    });
}

#[test]
fn todo_operations_map_to_sections_in_message_order() {
    App::test((), |mut app| async move {
        let todos = vec![todo("t1", "Compile list"), todo("t2", "Create suggestions")];
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    plain_text_message("m1", "before"),
                    update_todos_message("m2", todos.clone()),
                    mark_completed_message("m3", vec![todo("t1", "Compile list")]),
                    plain_text_message("m4", "after"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(
                block.sections(app_ctx),
                vec![
                    rich_text("before"),
                    TuiAIBlockSection::TodoList {
                        message_id: MessageId::new("m2".to_owned()),
                        todos: todos.clone(),
                    },
                    TuiAIBlockSection::CompletedTodos {
                        completed: vec![todo("t1", "Compile list")],
                    },
                    rich_text("after"),
                ]
            );
        });
    });
}

#[test]
fn empty_todo_operations_are_ignored() {
    App::test((), |mut app| async move {
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    update_todos_message("m1", Vec::new()),
                    mark_completed_message("m2", Vec::new()),
                    plain_text_message("m3", "visible"),
                ]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            assert_eq!(block.sections(app_ctx), vec![rich_text("visible")]);
        });
    });
}

#[test]
fn task_list_renders_header_and_status_glyph_rows() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let theme = Appearance::as_ref(app_ctx).theme();
            let yellow: Color =
                CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.yellow)).into();
            let green: Color =
                CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.green)).into();
            let primary = expected_output_text_color(app_ctx);
            let muted = expected_tool_call_text_color(app_ctx);

            let rows = vec![
                ("Compile list".to_owned(), TodoStatus::Completed),
                ("Determine duplications".to_owned(), TodoStatus::InProgress),
                ("Create suggestions".to_owned(), TodoStatus::Pending),
                ("Old task".to_owned(), TodoStatus::Cancelled),
            ];
            let states = CollapsibleSectionStates::default();
            let message_id = MessageId::new("m1".to_owned());
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_todo_list_section(&states, &message_id, &rows, app_ctx),
                TuiRect::new(0, 0, 40, 5),
                app_ctx,
            );
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec![
                    "≡ Tasks 4 ▾",
                    "  ✓ Compile list",
                    "  • Determine duplications",
                    "  ◌ Create suggestions",
                    "  ■ Old task",
                ],
            );
            // Header is bold primary text (the design's prominent header).
            assert_eq!(frame.buffer[(0, 0)].fg, primary);
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            // Completed: green check, primary title.
            assert_eq!(frame.buffer[(2, 1)].fg, green);
            assert_eq!(frame.buffer[(4, 1)].fg, primary);
            // In progress: yellow bullet, primary title.
            assert_eq!(frame.buffer[(2, 2)].fg, yellow);
            assert_eq!(frame.buffer[(4, 2)].fg, primary);
            // Pending: primary glyph and title.
            assert_eq!(frame.buffer[(2, 3)].fg, primary);
            // Cancelled: muted glyph, struck-through muted title.
            assert_eq!(frame.buffer[(2, 4)].fg, muted);
            assert_eq!(frame.buffer[(4, 4)].fg, muted);
            assert!(frame.buffer[(4, 4)]
                .modifier
                .contains(Modifier::CROSSED_OUT));
        });
    });
}

#[test]
fn task_list_collapse_override_hides_rows() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let rows = vec![("Compile list".to_owned(), TodoStatus::Pending)];
            let states = CollapsibleSectionStates::default();
            let message_id = MessageId::new("m1".to_owned());

            // Default: expanded, even though nothing is streaming — task
            // lists never default to collapsed.
            assert!(!states.is_collapsed(&message_id, false));

            states.set_collapsed(message_id.clone(), true);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_todo_list_section(&states, &message_id, &rows, app_ctx),
                TuiRect::new(0, 0, 40, 2),
                app_ctx,
            );
            let lines = frame.buffer.to_lines();
            assert_eq!(lines[0].trim_end(), "≡ Tasks 1 ▸");
            assert!(lines.iter().all(|line| !line.contains("Compile list")));
        });
    });
}

#[test]
fn task_list_header_hover_underlines_only_the_label() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let rows = vec![("Compile list".to_owned(), TodoStatus::Pending)];
            let states = CollapsibleSectionStates::default();
            let message_id = MessageId::new("m1".to_owned());
            let area = TuiRect::new(0, 0, 40, 2);

            // Move the pointer onto the header row so the shared hover state
            // reports it hovered, as the runtime would.
            let mut element = render_todo_list_section(&states, &message_id, &rows, app_ctx);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(TuiConstraint::loose(TuiSize::new(40, 2)), &mut ctx, app_ctx);
            // Paint once so the element retains its scene geometry for
            // hit-testing the hover move.
            let scene = {
                let mut buffer = TuiBuffer::empty(area);
                let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
                Rc::new(paint_ctx.scene.clone())
            };
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            element.dispatch_event(
                &TuiEvent::MouseMoved {
                    position: TuiPoint::new(0, 0),
                    modifiers: ModifiersState::default(),
                    is_synthetic: false,
                },
                &mut event_ctx,
                app_ctx,
            );

            // Re-render hovered: `≡ Tasks 1 ▾` underlines exactly the label's
            // cells — not the ≡ glyph, the chevron, or trailing cells. The
            // label's start column is located from the buffer since the
            // glyph's cell width varies by rendering backend.
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_todo_list_section(&states, &message_id, &rows, app_ctx),
                area,
                app_ctx,
            );
            assert_eq!(frame.buffer.to_lines()[0].trim_end(), "≡ Tasks 1 ▾");
            let label_start = (0..40u16)
                .find(|&x| frame.buffer[(x, 0)].symbol() == "T")
                .expect("the header row contains the label");
            let underlined: Vec<u16> = (0..40u16)
                .filter(|&x| frame.buffer[(x, 0)].modifier.contains(Modifier::UNDERLINED))
                .collect();
            // "Tasks 1" spans seven cells.
            let label_cells: Vec<u16> = (label_start..label_start + 7).collect();
            assert_eq!(underlined, label_cells);
        });
    });
}

#[test]
fn task_list_desired_height_accounts_for_rows_and_collapse() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![update_todos_message(
                    "m1",
                    vec![todo("t1", "one"), todo("t2", "two")],
                )]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            // Top padding + header + two task rows.
            assert_eq!(desired_height(block, 40, app_ctx), 4);

            block
                .collapsible_states
                .set_collapsed(MessageId::new("m1".to_owned()), true);
            // Collapsed: top padding + header only.
            assert_eq!(desired_height(block, 40, app_ctx), 2);
        });
    });
}

#[test]
fn task_list_without_conversation_state_falls_back_to_cancelled() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // The block's conversation is unknown to the (default) history model,
        // so every item resolves to the Cancelled fallback.
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![update_todos_message(
                    "m1",
                    vec![todo("t1", "orphaned task")],
                )]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "≡ Tasks 1 ▾");
            assert_eq!(rendered[1], "  ■ orphaned task");
        });
    });
}

#[test]
fn completed_todos_render_muted_completion_row() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![mark_completed_message(
                    "m1",
                    vec![todo("t1", "Compile list")],
                )]),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            // No active list is known to the history model, so the position
            // suffix is omitted.
            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "✓ Completed Compile list");

            let frame = {
                let mut presenter = TuiPresenter::new();
                presenter.present_element(
                    block.render_element(app_ctx),
                    TuiRect::new(0, 0, 40, 2),
                    app_ctx,
                )
            };
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                expected_tool_call_text_color(app_ctx)
            );
        });
    });
}

#[test]
fn completed_todos_label_includes_active_list_positions() {
    let list = AIAgentTodoList::default()
        .with_completed_items(vec![todo("t1", "one")])
        .with_pending_items(vec![todo("t2", "two"), todo("t3", "three")]);

    // Items in the active list carry their (n/m) position; unknown items omit it.
    assert_eq!(
        completed_todos_label(&[todo("t1", "one")], Some(&list)),
        "Completed one (1/3)"
    );
    assert_eq!(
        completed_todos_label(&[todo("t1", "one"), todo("t3", "three")], Some(&list)),
        "Completed one (1/3), three (3/3)"
    );
    assert_eq!(
        completed_todos_label(&[todo("t9", "unknown")], Some(&list)),
        "Completed unknown"
    );
    assert_eq!(
        completed_todos_label(&[todo("t1", "one")], None),
        "Completed one"
    );
    assert_eq!(completed_todos_label(&[], Some(&list)), "");
}

struct FakeAgentBlockModel {
    inputs: Vec<AIAgentInput>,
    status: AIBlockOutputStatus,
}

/// Builds an agent block with fresh test identity, registered in a fresh TUI
/// window and backed by a real action model.
fn test_agent_block(app: &mut App, model: FakeAgentBlockModel) -> ViewHandle<TuiAIBlock> {
    let (action_model, model_events) = add_test_action_model_and_events(app);
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiAIBlock::new(
                AIConversationId::new(),
                AIAgentExchangeId::new(),
                Rc::new(model),
                action_model,
                &model_events,
                terminal_model,
                ctx,
            )
        })
    })
}

fn ask_user_question_action(id: &str, question: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::AskUserQuestion {
            questions: ask_user_question_items(question),
        },
        requires_result: true,
    }
}

fn ask_user_question_items(question: &str) -> Vec<ai::agent::action::AskUserQuestionItem> {
    vec![ai::agent::action::AskUserQuestionItem {
        question_id: "q1".to_owned(),
        question: question.to_owned(),
        question_type: ai::agent::action::AskUserQuestionType::MultipleChoice {
            is_multiselect: false,
            options: vec![ai::agent::action::AskUserQuestionOption {
                label: "A".to_owned(),
                recommended: false,
            }],
            supports_other: false,
        },
    }]
}
impl AIBlockModel for FakeAgentBlockModel {
    type View = TuiAIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        self.status.clone()
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        None
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        None
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
        None
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
        &self.inputs
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        None
    }

    fn on_updated_output(
        &self,
        _callback: OutputStatusUpdateCallback<Self::View>,
        _ctx: &mut ViewContext<Self::View>,
    ) {
    }

    fn request_type(&self, _app: &AppContext) -> AIRequestType {
        AIRequestType::Active
    }
}

/// Builds a completed output status with one text message.
fn complete_output(sections: Vec<AIAgentTextSection>) -> AIBlockOutputStatus {
    complete_output_messages(vec![text_message("message-1", sections)])
}

/// Builds a completed output status from explicit output messages.
fn complete_output_messages(messages: Vec<AIAgentOutputMessage>) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages,
            ..Default::default()
        }),
    }
}

/// Builds a text output message from plain-text sections.
fn text_message(id: &str, sections: Vec<AIAgentTextSection>) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::Text(AIAgentText { sections }),
        citations: Vec::new(),
    }
}

/// Builds an action (tool call) output message.
fn action_message(id: &str, action: AIAgentAction) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::Action(action),
        citations: Vec::new(),
    }
}

/// Builds a debug output message (a variant the TUI does not render).
fn debug_output_message(id: &str, text: &str) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::DebugOutput {
            text: text.to_owned(),
        },
        citations: Vec::new(),
    }
}

/// Builds a todo item for task-list tests.
fn todo(id: &str, title: &str) -> AIAgentTodo {
    AIAgentTodo::new(id.to_owned().into(), title.to_owned(), String::new())
}

/// Builds an `UpdateTodos` todo-operation output message.
fn update_todos_message(id: &str, todos: Vec<AIAgentTodo>) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::TodoOperation(TodoOperation::UpdateTodos { todos }),
        citations: Vec::new(),
    }
}

/// Builds a `MarkAsCompleted` todo-operation output message.
fn mark_completed_message(id: &str, completed_todos: Vec<AIAgentTodo>) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::TodoOperation(TodoOperation::MarkAsCompleted {
            completed_todos,
        }),
        citations: Vec::new(),
    }
}

/// Builds a tool-call action for message-ordering tests.
fn test_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::InitProject,
        requires_result: true,
    }
}

/// Builds a shell-command tool-call action.
fn test_command_action(id: &str, command: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: command.to_owned(),
            is_read_only: None,
            is_risky: None,
            wait_until_completion: true,
            uses_pager: None,
            rationale: None,
            citations: Vec::new(),
        },
        requires_result: true,
    }
}

fn test_create_documents_action(id: &str, documents: Vec<DocumentToCreate>) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::CreateDocuments(CreateDocumentsRequest { documents }),
        requires_result: true,
    }
}

fn test_edit_documents_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::EditDocuments(EditDocumentsRequest {
            diffs: vec![DocumentDiff {
                document_id: AIDocumentId::new(),
                search: "old".to_owned(),
                replace: "new".to_owned(),
            }],
        }),
        requires_result: true,
    }
}

/// Builds a `Finished` status carrying `result` for `action`.
fn finished_status(action: &AIAgentAction, result: AIAgentActionResultType) -> AIActionStatus {
    AIActionStatus::Finished(Arc::new(AIAgentActionResult {
        id: action.id.clone(),
        task_id: action.task_id.clone(),
        result,
    }))
}

/// Builds an output status with a single reasoning message (id `reasoning-1`)
/// whose body is one plain-text section.
fn reasoning_status(finished_duration: Option<Duration>, body: &str) -> AIBlockOutputStatus {
    complete_output_messages(vec![reasoning_message(
        "reasoning-1",
        finished_duration,
        body,
    )])
}

/// Builds a reasoning output message with a single plain-text body section.
fn reasoning_message(
    id: &str,
    finished_duration: Option<Duration>,
    body: &str,
) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::Reasoning {
            text: AIAgentText {
                sections: vec![AIAgentTextSection::PlainText {
                    text: body.to_owned().into(),
                }],
            },
            finished_duration,
        },
        citations: Vec::new(),
    }
}

fn summarization_message(
    id: &str,
    finished_duration: Option<Duration>,
    summarization_type: SummarizationType,
    body: &str,
) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::Summarization {
            text: AIAgentText {
                sections: vec![AIAgentTextSection::PlainText {
                    text: body.to_owned().into(),
                }],
            },
            finished_duration,
            summarization_type,
            token_count: None,
        },
        citations: Vec::new(),
    }
}

/// Builds a text output message from a single plain-text string.
fn plain_text_message(id: &str, text: &str) -> AIAgentOutputMessage {
    text_message(
        id,
        vec![AIAgentTextSection::PlainText {
            text: text.to_owned().into(),
        }],
    )
}

fn rich_text(text: &str) -> TuiAIBlockSection {
    TuiAIBlockSection::RichText(rich_text_section(text))
}

fn rich_body(text: &str) -> Vec<TuiRichTextSection> {
    vec![rich_text_section(text)]
}

fn rich_text_section(text: &str) -> TuiRichTextSection {
    TuiRichTextSection::Markdown(Arc::new(parse_markdown(text).expect("valid test Markdown")))
}

/// Measures the block by laying out its rendered element with an empty layout
/// context; these tests exercise blocks with no registered child views.
fn desired_height(block: &TuiAIBlock, width: u16, app: &AppContext) -> usize {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let mut element = block.render_element(app);
    usize::from(
        element
            .layout(
                TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                &mut ctx,
                app,
            )
            .height,
    )
}

/// Renders the block at `width` and returns its non-empty rows, trimmed of
/// trailing padding, so header/body assertions ignore blank rows.
fn render_block_lines(block: &TuiAIBlock, width: u16, app: &AppContext) -> Vec<String> {
    let height = desired_height(block, width, app).max(1) as u16;
    let mut presenter = TuiPresenter::new();
    let frame = presenter.present_element(
        block.render_element(app),
        TuiRect::new(0, 0, width, height),
        app,
    );
    frame
        .buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

fn render_tui_view_lines(
    view: &impl TuiView,
    width: u16,
    height: u16,
    app: &AppContext,
) -> Vec<String> {
    let mut presenter = TuiPresenter::new();
    presenter
        .present_element(view.render(app), TuiRect::new(0, 0, width, height), app)
        .buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Builds one user-query input for model-backed extraction tests.
fn query_input(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_owned(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::default(),
        running_command: None,
        intended_agent: None,
    }
}

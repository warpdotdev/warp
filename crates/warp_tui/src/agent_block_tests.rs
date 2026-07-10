use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentExchangeId, AIAgentInput, AIAgentOutput, AIAgentOutputMessage,
    AIAgentOutputMessageType, AIAgentText, AIAgentTextSection, AIBlockModel, AIBlockOutputStatus,
    AIConversationId, AIRequestType, Appearance, LLMId, MessageId, OutputStatusUpdateCallback,
    RequestCommandOutputResult, ServerOutputId, Shared, TaskId, TerminalModel, UserQueryMode,
};
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, SingletonEntity};
use warpui_core::elements::tui::{
    Color, Modifier, TuiBufferExt, TuiConstraint, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPoint, TuiRect, TuiSize,
};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::event::ModifiersState;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, EntityIdMap, ViewContext, ViewHandle};

use super::{TuiAIBlock, TuiAIBlockSection};
use crate::agent_block_sections::render_fallback_tool_call_section;
use crate::test_fixtures::{add_test_action_model_and_events, TestHostView};

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
                vec!["", "≫ hello", "", "one", "two", "three"],
            );
            assert_eq!(frame.buffer[(0, 1)].fg, expected_prompt_text_color(app_ctx));
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

fn expected_input_background(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    let accent = ThemeFill::from(theme.terminal_colors().normal.cyan);
    CoreFill::from(theme.background().blend(&accent.with_opacity(20))).into()
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
                    TuiAIBlockSection::PlainText("one".to_owned()),
                    TuiAIBlockSection::PlainText("two".to_owned()),
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
                    TuiAIBlockSection::PlainText("before".to_owned()),
                    TuiAIBlockSection::ToolCall(Box::new(action.clone())),
                    TuiAIBlockSection::PlainText("after".to_owned()),
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
                "✗ `git status` denied (denylisted)"
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
                vec![
                    TuiAIBlockSection::PlainText("before".to_owned()),
                    TuiAIBlockSection::PlainText("after".to_owned()),
                ]
            );
        });
    });
}

#[test]
fn agent_block_omits_unsupported_sections_until_the_tui_can_render_them() {
    App::test((), |mut app| async move {
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
            assert_eq!(
                block.sections(app_ctx),
                vec![TuiAIBlockSection::PlainText("visible".to_owned())]
            );
        });
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
                    body: "line one\nline two".to_owned(),
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
                .thinking_states
                .set_collapsed(MessageId::new("reasoning-1".to_owned()), false);

            let rendered = render_block_lines(block, 40, app_ctx);
            assert_eq!(rendered[0], "Thought for 2 seconds ▾");
            assert!(rendered.iter().any(|line| line.contains("revealed body")));
        });
    });
}

#[test]
fn header_click_records_a_manual_collapse_override() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let block = test_agent_block(
            &mut app,
            FakeAgentBlockModel {
                inputs: Vec::new(),
                status: reasoning_status(None, "body"),
            },
        );
        app.read(|app_ctx| {
            let block = block.as_ref(app_ctx);
            let mut element = block.render_element(app_ctx);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let area = TuiRect::new(0, 0, 40, 5);
            element.layout(TuiConstraint::loose(TuiSize::new(40, 5)), &mut ctx, app_ctx);

            // Click the `Thinking...` header row (row 1, below the block's top
            // padding): the press arms the header's click and the release fires
            // it. The runtime attributes dispatch to a rendered view, so give
            // the context an origin view for the toggle's `notify()`.
            let mut event_ctx = TuiEventContext::default();
            event_ctx.set_origin_view(Some(EntityId::new()));
            let handled = element.dispatch_event(
                &TuiEvent::LeftMouseDown {
                    position: TuiPoint::new(0, 1),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(handled, "the press arming the click must be consumed");
            let handled = element.dispatch_event(
                &TuiEvent::LeftMouseUp {
                    position: TuiPoint::new(0, 1),
                    modifiers: ModifiersState::default(),
                },
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(handled, "the release completing the click must be consumed");

            // The streaming block was expanded, so the click records a collapse
            // override that wins over the expanded-while-streaming default.
            let message_id = MessageId::new("reasoning-1".to_owned());
            assert!(block.thinking_states.is_collapsed(&message_id, false));
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
                    TuiAIBlockSection::PlainText("before".to_owned()),
                    TuiAIBlockSection::Thinking {
                        message_id: MessageId::new("r1".to_owned()),
                        finished_duration: None,
                        body: "thinking".to_owned(),
                    },
                    TuiAIBlockSection::PlainText("after".to_owned()),
                ]
            );
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
        ctx.add_tui_view(window_id, move |ctx| {
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

/// Builds a text output message from a single plain-text string.
fn plain_text_message(id: &str, text: &str) -> AIAgentOutputMessage {
    text_message(
        id,
        vec![AIAgentTextSection::PlainText {
            text: text.to_owned().into(),
        }],
    )
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

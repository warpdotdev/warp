use std::rc::Rc;

use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId, AIAgentInput,
    AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentText, AIAgentTextSection,
    AIBlockModel, AIBlockOutputStatus, AIConversationId, AIRequestType, Appearance, LLMId,
    MessageId, OutputStatusUpdateCallback, ServerOutputId, Shared, TaskId, UserQueryMode,
};
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{Color, Modifier, TuiBufferExt, TuiRect};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, ViewContext};

use super::{TuiAIBlock, TuiAIBlockSection};

#[test]
fn simple_agent_block_reports_full_height_and_renders_content() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: vec![query_input("hello")],
                status: complete_output(vec![AIAgentTextSection::PlainText {
                    text: "one\ntwo\nthree".to_owned().into(),
                }]),
            });
            assert_eq!(block.desired_height(20, app_ctx), 6);

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
                vec!["≫ hello", "", "one", "two", "three", ""],
            );
            assert_eq!(frame.buffer[(0, 0)].fg, expected_prompt_text_color(app_ctx));
            assert_eq!(frame.buffer[(0, 0)].bg, expected_input_background(app_ctx));
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(2, 0)].fg, expected_prompt_text_color(app_ctx));
            assert_eq!(frame.buffer[(19, 0)].bg, expected_input_background(app_ctx));
            assert_eq!(frame.buffer[(0, 2)].fg, expected_output_text_color(app_ctx));
            // The block paints no background of its own, so output rows show the
            // terminal's own background.
            assert_eq!(frame.buffer[(0, 2)].bg, Color::Reset);
            assert_eq!(frame.buffer[(19, 2)].bg, Color::Reset);
        });
    });
}

#[test]
fn simple_agent_block_reflows_height_at_narrow_width() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: vec![query_input("hello world")],
                status: complete_output(vec![AIAgentTextSection::PlainText {
                    text: "streamed output".to_owned().into(),
                }]),
            });

            let wide = block.desired_height(40, app_ctx);
            let narrow = block.desired_height(6, app_ctx);
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
    CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.white)).into()
}

fn expected_tool_call_text_color(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    CoreFill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into()
}

#[test]
fn agent_block_extracts_input_and_plain_text_from_model() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: vec![query_input("hello")],
                status: complete_output(vec![
                    AIAgentTextSection::PlainText {
                        text: "one".to_owned().into(),
                    },
                    AIAgentTextSection::PlainText {
                        text: "two".to_owned().into(),
                    },
                ]),
            });
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
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let action = test_action("action-1");
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    text_message(
                        "message-1",
                        vec![AIAgentTextSection::PlainText {
                            text: "before".to_owned().into(),
                        }],
                    ),
                    action_message("message-2", action.clone()),
                    text_message(
                        "message-3",
                        vec![AIAgentTextSection::PlainText {
                            text: "after".to_owned().into(),
                        }],
                    ),
                ]),
            });

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
                vec!["before", "executed a tool call", "after", ""],
            );
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                expected_tool_call_text_color(app_ctx)
            );
            assert!(frame.buffer[(0, 1)].modifier.contains(Modifier::DIM));
        });
    });
}

#[test]
fn agent_block_renders_multiple_tool_calls_in_order() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let first = test_action("action-1");
            let second = test_action("action-2");
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    action_message("message-1", first.clone()),
                    action_message("message-2", second.clone()),
                ]),
            });

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
                TuiRect::new(0, 0, 40, 3),
                app_ctx,
            );
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["executed a tool call", "executed a tool call", ""],
            );
        });
    });
}

#[test]
fn agent_block_desired_height_accounts_for_tool_call_stub() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![action_message(
                    "message-1",
                    test_action("action-1"),
                )]),
            });
            // One tool-call stub line plus the block's bottom padding row.
            assert_eq!(block.desired_height(40, app_ctx), 2);
        });
    });
}

#[test]
fn agent_block_ignores_unsupported_message_variants() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let block = test_agent_block(FakeAgentBlockModel {
                inputs: Vec::new(),
                status: complete_output_messages(vec![
                    text_message(
                        "message-1",
                        vec![AIAgentTextSection::PlainText {
                            text: "before".to_owned().into(),
                        }],
                    ),
                    reasoning_message("message-2"),
                    text_message(
                        "message-3",
                        vec![AIAgentTextSection::PlainText {
                            text: "after".to_owned().into(),
                        }],
                    ),
                ]),
            });

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
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let block = test_agent_block(FakeAgentBlockModel {
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
            });

            assert_eq!(
                block.sections(app_ctx),
                vec![TuiAIBlockSection::PlainText("visible".to_owned())]
            );
        });
    });
}

struct FakeAgentBlockModel {
    inputs: Vec<AIAgentInput>,
    status: AIBlockOutputStatus,
}

/// Builds an agent block with fresh test identity.
fn test_agent_block(model: FakeAgentBlockModel) -> TuiAIBlock {
    TuiAIBlock::new(
        AIConversationId::new(),
        AIAgentExchangeId::new(),
        Rc::new(model),
    )
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

/// Builds a reasoning output message (an unsupported variant for the TUI).
fn reasoning_message(id: &str) -> AIAgentOutputMessage {
    AIAgentOutputMessage {
        id: MessageId::new(id.to_owned()),
        message: AIAgentOutputMessageType::Reasoning {
            text: AIAgentText {
                sections: vec![AIAgentTextSection::PlainText {
                    text: "thinking".to_owned().into(),
                }],
            },
            finished_duration: None,
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

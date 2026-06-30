use std::rc::Rc;

use crate::theme::{AGENT_INPUT_BACKGROUND, AGENT_INPUT_TEXT, AGENT_OUTPUT_TEXT};
use warp::tui_export::{
    AIAgentInput, AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentText,
    AIAgentTextSection, AIBlockModel, AIBlockOutputStatus, AIConversationId, AIRequestType, LLMId,
    MessageId, OutputStatusUpdateCallback, ServerOutputId, Shared, UserQueryMode,
};
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, ViewContext};

use super::{TuiAgentBlockElement, TuiAgentBlockSection, TuiAgentBlockView};

#[test]
fn simple_agent_block_reports_full_height_and_renders_content() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let element = TuiAgentBlockElement::new(vec![
                TuiAgentBlockSection::Input("hello".to_owned()),
                TuiAgentBlockSection::PlainText("one\ntwo\nthree".to_owned()),
            ]);
            assert_eq!(element.desired_height(20, app_ctx), 6);

            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(Box::new(element), TuiRect::new(0, 0, 20, 6), app_ctx);
            assert_eq!(
                frame
                    .buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_owned())
                    .collect::<Vec<_>>(),
                vec!["≫ hello", "", "one", "two", "three", ""],
            );
            assert_eq!(frame.buffer[(0, 0)].fg, AGENT_INPUT_TEXT);
            assert_eq!(frame.buffer[(0, 0)].bg, AGENT_INPUT_BACKGROUND);
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(2, 0)].fg, AGENT_INPUT_TEXT);
            assert_eq!(frame.buffer[(19, 0)].bg, AGENT_INPUT_BACKGROUND);
            assert_eq!(frame.buffer[(0, 2)].fg, AGENT_OUTPUT_TEXT);
        });
    });
}

#[test]
fn simple_agent_block_reflows_height_at_narrow_width() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let element = TuiAgentBlockElement::new(vec![
                TuiAgentBlockSection::Input("hello world".to_owned()),
                TuiAgentBlockSection::PlainText("streamed output".to_owned()),
            ]);

            let wide = element.desired_height(40, app_ctx);
            let narrow = element.desired_height(6, app_ctx);
            assert!(narrow > wide, "narrow text should occupy more logical rows");
        });
    });
}

#[test]
fn agent_block_extracts_input_and_plain_text_from_model() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let block = TuiAgentBlockView::new(Rc::new(FakeAgentBlockModel {
                inputs: vec![query_input("hello")],
                status: complete_output(vec![
                    AIAgentTextSection::PlainText {
                        text: "one".to_owned().into(),
                    },
                    AIAgentTextSection::PlainText {
                        text: "two".to_owned().into(),
                    },
                ]),
            }));
            assert_eq!(
                block.element(app_ctx).sections,
                vec![
                    TuiAgentBlockSection::Input("hello".to_owned()),
                    TuiAgentBlockSection::PlainText("one".to_owned()),
                    TuiAgentBlockSection::PlainText("two".to_owned()),
                ]
            );
        });
    });
}

#[test]
fn agent_block_omits_unsupported_sections_until_the_tui_can_render_them() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let block = TuiAgentBlockView::new(Rc::new(FakeAgentBlockModel {
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
            }));

            assert_eq!(
                block.element(app_ctx).sections,
                vec![TuiAgentBlockSection::PlainText("visible".to_owned())]
            );
        });
    });
}

struct FakeAgentBlockModel {
    inputs: Vec<AIAgentInput>,
    status: AIBlockOutputStatus,
}

impl AIBlockModel for FakeAgentBlockModel {
    type View = TuiAgentBlockView;

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
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("message-1".to_owned()),
                message: AIAgentOutputMessageType::Text(AIAgentText { sections }),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
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

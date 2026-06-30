use std::rc::Rc;

use warp::tui_export::{
    AIAgentInput, AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentText,
    AIAgentTextSection, AIBlockModel, AIBlockOutputStatus, AIConversationId, AIRequestType,
    Appearance, LLMId, MessageId, OutputStatusUpdateCallback, ServerOutputId, Shared,
    UserQueryMode,
};
use warp_core::ui::color::blend::Blend;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Color, Modifier, TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiRect, TuiSize,
};
use warpui_core::elements::Fill as GuiFill;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityIdMap, ViewContext};

use super::{TuiAgentBlockSection, TuiAgentBlockView};

#[test]
fn simple_agent_block_reports_full_height_and_renders_content() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let sections = vec![
                TuiAgentBlockSection::Input("hello".to_owned()),
                TuiAgentBlockSection::PlainText("one\ntwo\nthree".to_owned()),
            ];
            assert_eq!(desired_height_for_sections(&sections, 20, app_ctx), 6);

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                TuiAgentBlockView::render_sections(&sections, app_ctx),
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
            assert_eq!(frame.buffer[(0, 0)].fg, expected_text_color(app_ctx));
            assert_eq!(frame.buffer[(0, 0)].bg, expected_input_background(app_ctx));
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(2, 0)].fg, expected_text_color(app_ctx));
            assert_eq!(frame.buffer[(19, 0)].bg, expected_input_background(app_ctx));
            assert_eq!(frame.buffer[(0, 2)].fg, expected_text_color(app_ctx));
        });
    });
}

/// Measures generic agent-block sections at a target width.
fn desired_height_for_sections(
    sections: &[TuiAgentBlockSection],
    width: u16,
    app: &AppContext,
) -> usize {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let mut element = TuiAgentBlockView::render_sections(sections, app);
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

#[test]
fn simple_agent_block_reflows_height_at_narrow_width() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let sections = vec![
                TuiAgentBlockSection::Input("hello world".to_owned()),
                TuiAgentBlockSection::PlainText("streamed output".to_owned()),
            ];

            let wide = desired_height_for_sections(&sections, 40, app_ctx);
            let narrow = desired_height_for_sections(&sections, 6, app_ctx);
            assert!(narrow > wide, "narrow text should occupy more logical rows");
        });
    });
}

fn expected_text_color(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    GuiFill::from(theme.main_text_color(theme.surface_1())).into()
}

fn expected_input_background(app: &AppContext) -> Color {
    let theme = Appearance::as_ref(app).theme();
    GuiFill::from(theme.background().blend(&theme.ai_blocks_overlay())).into()
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
                block.sections(app_ctx),
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
                block.sections(app_ctx),
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

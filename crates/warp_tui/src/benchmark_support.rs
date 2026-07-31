//! Production-shaped fixtures for retained TUI transcript benchmarks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId, AIAgentInput,
    AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentText, AIAgentTextSection,
    AIBlockModel, AIBlockOutputStatus, AIConversationId, AIRequestType, Appearance, BlockId, LLMId,
    MessageId, OutputStatusUpdateCallback, RichContentItem, RichContentType, ServerOutputId,
    Shared, TaskId, TerminalModel,
};
use warpui::platform::WindowStyle;
use warpui::{
    AddWindowOptions, App, AppContext, Entity, EntityId, EntityIdSet, TuiView, TypedActionView,
    ViewContext, ViewHandle, WindowInvalidation,
};
use warpui_core::elements::tui::{
    TuiClipped, TuiElement, TuiRect, TuiViewportPosition, TuiViewportVerticalAlignment,
    TuiViewportedList, TuiViewportedListState,
};
use warpui_core::presenter::tui::TuiPresenter;

use crate::agent_block::TuiAIBlock;
use crate::terminal_block::{TerminalBlockElement, block_content_rows};
use crate::test_fixtures::add_test_action_model_and_events;
use crate::tui_block_list_viewport_source::{
    AgentBlockRegistry, CLISubagentBlockRegistry, HandoffBlockRegistry, TuiBlockListViewportSource,
};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_shell_command_view::TuiShellCommandViewAction;

/// Shape of the retained transcript fixture.
#[derive(Clone, Copy, Debug)]
pub enum TranscriptDataset {
    /// Many independently indexed terminal blocks.
    ManySmallBlocks { blocks: usize },
    /// One rich agent response containing `rows` hard lines.
    LongAgentResponse { rows: usize },
    /// One expanded agent-requested command that remains active at `rows` output rows.
    RunningAgentCommand { rows: usize },
    /// A streaming rich response below `preceding_rows` of fixed history.
    OffscreenStreamingTail {
        preceding_rows: usize,
        tail_rows: usize,
    },
}

/// One inline terminal block painted through a fixed-height clipped viewport.
pub struct ClippedTerminalBlockBenchmark {
    app: App,
    model: Arc<FairMutex<TerminalModel>>,
    block_id: BlockId,
    viewport_origin_y: usize,
    presenter: TuiPresenter,
    area: TuiRect,
}

impl ClippedTerminalBlockBenchmark {
    /// Builds and primes a long terminal block with `rows` output rows.
    pub fn new(rows: usize, width: u16, height: u16) -> Self {
        App::test((), move |app| async move {
            let mut terminal_model = TerminalModel::mock(None, None);
            let output = "benchmark terminal output\r\n".repeat(rows);
            terminal_model.simulate_block("printf benchmark", output.as_str());
            let block = terminal_model
                .block_list()
                .blocks()
                .iter()
                .rev()
                .find(|block| block.finished())
                .expect("simulated block should exist");
            let block_id = block.id().clone();
            let content_height = block_content_rows(block).len();
            let mut benchmark = Self {
                app,
                model: Arc::new(FairMutex::new(terminal_model)),
                block_id,
                viewport_origin_y: content_height.saturating_sub(usize::from(height)),
                presenter: TuiPresenter::new(),
                area: TuiRect::new(0, 0, width, height),
            };
            benchmark.present();
            benchmark
        })
    }

    /// Lays out and paints one clipped frame and returns a cheap checksum.
    pub fn present(&mut self) -> u64 {
        let element = TuiClipped::new(
            TerminalBlockElement::content(self.model.clone(), self.block_id.clone()).finish(),
        )
        .with_viewport_origin_y(self.viewport_origin_y)
        .finish();
        let frame = self
            .app
            .read(|ctx| self.presenter.present_element(element, self.area, ctx));
        frame.buffer.content.iter().fold(0u64, |checksum, cell| {
            checksum.wrapping_add(cell.symbol().len() as u64)
        })
    }
}

/// One production-shaped retained transcript benchmark.
pub struct TranscriptBenchmark {
    app: App,
    root: ViewHandle<BenchmarkTranscriptView>,
    model: Arc<FairMutex<TerminalModel>>,
    agent_block_id: Option<EntityId>,
    presenter: TuiPresenter,
    area: TuiRect,
}

impl TranscriptBenchmark {
    /// Builds and primes a benchmark at `width × height`.
    pub fn new(dataset: TranscriptDataset, width: u16, height: u16) -> Self {
        App::test((), move |mut app| async move {
            app.add_singleton_model(|_| Appearance::mock());
            let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
            if let TranscriptDataset::ManySmallBlocks { blocks } = dataset {
                let mut model = terminal_model.lock();
                for index in 0..blocks {
                    let command = format!("printf block-{index}");
                    let output = format!("output-{index}\r\n");
                    model.simulate_block(command.as_str(), output.as_str());
                }
            }
            if let TranscriptDataset::OffscreenStreamingTail { preceding_rows, .. } = dataset {
                let output = "history row\r\n".repeat(preceding_rows);
                terminal_model
                    .lock()
                    .simulate_block("history", output.as_str());
            }
            let running_command_action =
                if let TranscriptDataset::RunningAgentCommand { rows } = dataset {
                    let action = benchmark_command_action();
                    let output = "running command output\r\n".repeat(rows);
                    let mut model = terminal_model.lock();
                    model.simulate_long_running_block("printf benchmark", output.as_str());
                    model
                        .block_list_mut()
                        .active_block_mut()
                        .set_agent_interaction_mode_for_requested_command(
                            action.id.clone(),
                            None,
                            AIConversationId::new(),
                        );
                    Some(action)
                } else {
                    None
                };

            let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::new()));
            let cli_subagent_blocks = CLISubagentBlockRegistry::new(RefCell::new(HashMap::new()));
            let handoff_blocks = HandoffBlockRegistry::new(RefCell::new(HashMap::new()));
            let model_for_root = terminal_model.clone();
            let agent_blocks_for_root = agent_blocks.clone();
            let cli_subagent_blocks_for_root = cli_subagent_blocks.clone();
            let handoff_blocks_for_root = handoff_blocks.clone();
            let (window_id, root) = app.update(|ctx| {
                ctx.add_tui_window(
                    AddWindowOptions {
                        window_style: WindowStyle::NotStealFocus,
                        ..Default::default()
                    },
                    move |_| BenchmarkTranscriptView {
                        model: model_for_root,
                        agent_blocks: agent_blocks_for_root,
                        cli_subagent_blocks: cli_subagent_blocks_for_root,
                        handoff_blocks: handoff_blocks_for_root,
                        viewport: TuiViewportedListState::new_at_end(),
                    },
                )
            });

            let agent_block_id = match dataset {
                TranscriptDataset::ManySmallBlocks { .. } => None,
                TranscriptDataset::LongAgentResponse { rows }
                | TranscriptDataset::OffscreenStreamingTail {
                    tail_rows: rows, ..
                } => {
                    let (action_model, model_events) = add_test_action_model_and_events(&mut app);
                    let text = long_response(rows);
                    let status = match dataset {
                        TranscriptDataset::LongAgentResponse { .. } => completed_text_status(text),
                        TranscriptDataset::OffscreenStreamingTail { .. } => {
                            streaming_text_status(text)
                        }
                        TranscriptDataset::ManySmallBlocks { .. }
                        | TranscriptDataset::RunningAgentCommand { .. } => unreachable!(),
                    };
                    let terminal_model_for_block = terminal_model.clone();
                    let agent_block = app.update(|ctx| {
                        ctx.add_typed_action_tui_view(window_id, move |ctx| {
                            TuiAIBlock::new(
                                (AIConversationId::new(), AIAgentExchangeId::new()),
                                Rc::new(BenchmarkAgentBlockModel { status }),
                                action_model,
                                &model_events,
                                terminal_model_for_block,
                                false,
                                ctx,
                            )
                        })
                    });
                    let view_id = agent_block.id();
                    agent_blocks.borrow_mut().insert(view_id, agent_block);
                    terminal_model.lock().block_list_mut().append_rich_content(
                        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
                        false,
                    );
                    Some(view_id)
                }
                TranscriptDataset::RunningAgentCommand { .. } => {
                    let (action_model, model_events) = add_test_action_model_and_events(&mut app);
                    let action =
                        running_command_action.expect("running command dataset has an action");
                    let status = running_command_status(action);
                    let terminal_model_for_block = terminal_model.clone();
                    let agent_block = app.update(|ctx| {
                        ctx.add_typed_action_tui_view(window_id, move |ctx| {
                            TuiAIBlock::new(
                                (AIConversationId::new(), AIAgentExchangeId::new()),
                                Rc::new(BenchmarkAgentBlockModel { status }),
                                action_model,
                                &model_events,
                                terminal_model_for_block,
                                false,
                                ctx,
                            )
                        })
                    });
                    let view_id = agent_block.id();
                    agent_blocks
                        .borrow_mut()
                        .insert(view_id, agent_block.clone());
                    terminal_model.lock().block_list_mut().append_rich_content(
                        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
                        false,
                    );
                    app.update(|ctx| {
                        let shell_view_id = agent_block
                            .as_ref(ctx)
                            .child_view_ids(ctx)
                            .into_iter()
                            .next()
                            .expect("running command agent block has a shell child");
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            shell_view_id,
                            &TuiShellCommandViewAction::ToggleExpanded,
                        );
                    });
                    Some(view_id)
                }
            };

            let mut benchmark = Self {
                app,
                root,
                model: terminal_model,
                agent_block_id,
                presenter: TuiPresenter::new(),
                area: TuiRect::new(0, 0, width, height),
            };
            benchmark.invalidate_all();
            benchmark.present();
            benchmark
        })
    }

    /// Paints one frame from retained view elements and returns a cheap checksum.
    pub fn present(&mut self) -> u64 {
        let frame = self
            .app
            .update(|ctx| self.presenter.present(ctx, &self.root, self.area));
        frame.buffer.content.iter().fold(0u64, |checksum, cell| {
            checksum.wrapping_add(cell.symbol().len() as u64)
        })
    }

    /// Re-renders the transcript viewport without dirtying retained agent blocks.
    pub fn invalidate_viewport(&mut self) {
        let invalidation = WindowInvalidation {
            updated: EntityIdSet::from_iter([self.root.id()]),
            ..Default::default()
        };
        self.app.read(|ctx| {
            self.presenter
                .invalidate(&invalidation, ctx, self.root.window_id(ctx));
        });
    }

    /// Re-renders the root and long agent block before the next frame.
    pub fn invalidate_all(&mut self) {
        let mut updated = EntityIdSet::from_iter([self.root.id()]);
        if let Some(agent_block_id) = self.agent_block_id {
            updated.insert(agent_block_id);
        }
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };
        self.app.read(|ctx| {
            self.presenter
                .invalidate(&invalidation, ctx, self.root.window_id(ctx));
        });
    }

    /// Marks the rich tail dirty and re-renders its retained view.
    pub fn invalidate_streaming_tail(&mut self) {
        let agent_block_id = self
            .agent_block_id
            .expect("streaming-tail fixture has an agent block");
        self.model
            .lock()
            .block_list_mut()
            .mark_rich_content_dirty(agent_block_id);
        self.invalidate_all();
    }

    /// Moves the retained viewport to an absolute transcript row.
    pub fn scroll_to_row(&mut self, row: usize) {
        self.root.update(&mut self.app, |view, _| {
            view.viewport
                .set_position(TuiViewportPosition::RowsFromTop(row));
        });
    }

    /// Moves the retained viewport back to the transcript end.
    pub fn scroll_to_end(&mut self) {
        self.root.update(&mut self.app, |view, _| {
            view.viewport.scroll_to_end();
        });
    }
}

struct BenchmarkTranscriptView {
    model: Arc<FairMutex<TerminalModel>>,
    agent_blocks: AgentBlockRegistry,
    cli_subagent_blocks: CLISubagentBlockRegistry,
    handoff_blocks: HandoffBlockRegistry,
    viewport: TuiViewportedListState,
}

impl Entity for BenchmarkTranscriptView {
    type Event = ();
}

impl TypedActionView for BenchmarkTranscriptView {
    type Action = ();
}

impl TuiView for BenchmarkTranscriptView {
    fn ui_name() -> &'static str {
        "BenchmarkTranscriptView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.agent_blocks.borrow().keys().copied().collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let source = TuiBlockListViewportSource::new_with_rich_content(
            self.model.clone(),
            self.agent_blocks.clone(),
            self.cli_subagent_blocks.clone(),
            self.handoff_blocks.clone(),
        );
        TuiViewportedList::new(
            self.viewport.clone(),
            source,
            TuiUiBuilder::from_app(app).selection_style(),
        )
        .with_vertical_alignment(TuiViewportVerticalAlignment::GrowFromBottom)
        .finish()
    }
}

struct BenchmarkAgentBlockModel {
    status: AIBlockOutputStatus,
}

impl AIBlockModel for BenchmarkAgentBlockModel {
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
        &[]
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

fn benchmark_command_action() -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("benchmark-command".to_owned()),
        task_id: TaskId::new("benchmark-task".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: "printf benchmark".to_owned(),
            is_read_only: Some(true),
            is_risky: Some(false),
            wait_until_completion: true,
            uses_pager: Some(false),
            rationale: None,
            citations: Vec::new(),
        },
        requires_result: true,
    }
}

fn running_command_status(action: AIAgentAction) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("benchmark-command-message".to_owned()),
                message: AIAgentOutputMessageType::Action(action),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}

fn completed_text_status(text: String) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("benchmark-response".to_owned()),
                message: AIAgentOutputMessageType::Text(AIAgentText {
                    sections: vec![AIAgentTextSection::PlainText { text: text.into() }],
                }),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}

fn streaming_text_status(text: String) -> AIBlockOutputStatus {
    let AIBlockOutputStatus::Complete { output } = completed_text_status(text) else {
        unreachable!()
    };
    AIBlockOutputStatus::PartiallyReceived { output }
}

fn long_response(rows: usize) -> String {
    let mut text = String::with_capacity(rows.saturating_mul(48));
    for row in 0..rows {
        text.push_str("benchmark transcript response row ");
        text.push_str(&(row % 10_000).to_string());
        text.push('\n');
    }
    text
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentExchangeId, AIAgentInput, AIBlockModel, AIBlockOutputStatus, AIConversationId,
    AIRequestType, Appearance, BlockHeightItem, LLMId, OutputStatusUpdateCallback, RichContentItem,
    RichContentType, ServerOutputId, TerminalModel, UserQueryMode,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityId, EntityIdMap, ViewHandle};
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiLayoutContext, TuiSize, TuiText, TuiViewportContent,
    TuiViewportWindow, TuiViewportedElement,
};
use warpui_core::{App, AppContext, Entity, TuiView, TypedActionView, ViewContext};

use super::{AgentBlockRegistry, TuiBlockListViewportItemId, TuiBlockListViewportSource};
use crate::agent_block::TuiAIBlock;
use crate::terminal_block::should_render_terminal_block;

#[test]
fn tui_block_list_viewport_source_uses_canonical_block_list_order() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_block("echo 1", "1\r\n");
    model.simulate_block("echo 2", "2\r\n");
    let expected = model
        .block_list()
        .blocks()
        .iter()
        .filter(|block| should_render_terminal_block(block, model.block_list()))
        .map(|block| TuiBlockListViewportItemId::TerminalBlock(block.id().clone()))
        .collect::<Vec<_>>();
    let source = TuiBlockListViewportSource::new(
        Arc::new(FairMutex::new(model)),
        AgentBlockRegistry::new(RefCell::new(HashMap::new())),
    );

    let actual = source.item_ids_for_test();

    assert_eq!(actual, expected);
}

#[test]
fn tui_block_list_viewport_source_slices_terminal_blocks_to_visible_rows() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut model = TerminalModel::mock(None, None);
            model.simulate_block("printf", "one\r\ntwo\r\nthree\r\n");
            let source = TuiBlockListViewportSource::new(
                Arc::new(FairMutex::new(model)),
                AgentBlockRegistry::new(RefCell::new(HashMap::new())),
            );

            let content = source.visible_items(
                TuiViewportWindow {
                    scroll_top: 1,
                    viewport_height: 1,
                },
                80,
                app,
            );

            assert_eq!(content.items.len(), 1);
            let mut item = content.items.into_iter().next().unwrap();
            assert_eq!(item.origin_y, 1);

            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = item.element.layout(
                TuiConstraint::loose(TuiSize::new(80, u16::MAX)),
                &mut layout_ctx,
                app,
            );
            assert_eq!(size.height, 1);
        });
    });
}

#[test]
fn tui_agent_rich_content_stays_visible_without_gui_agent_view_state() {
    let mut model = TerminalModel::mock(None, None);
    let view_id = EntityId::new();
    model.block_list_mut().append_rich_content(
        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
        false,
    );
    model
        .block_list_mut()
        .update_rich_content_heights(&HashMap::from([(view_id, 3.0)]));

    let rich_content = model
        .block_list()
        .block_heights()
        .cursor::<(), ()>()
        .find_map(|item| match item {
            BlockHeightItem::RichContent(item) if item.view_id == view_id => Some(item),
            BlockHeightItem::Block(_)
            | BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent(_) => None,
        })
        .expect("TUI agent rich content should remain in the canonical block list");

    assert!(!rich_content.should_hide);
    assert!(rich_content.last_laid_out_height.as_f64() > 0.0);
}

#[test]
fn tui_agent_overhang_remeasures_visible_non_dirty_height() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let expected = app.read(|app| agent_block.as_ref(app).desired_height(80, app) as f64);

        // The visible block is re-measured during `visible_items`, so its height
        // is corrected before windowing without any post-layout pass.
        let content = request_top_window(&app, &source, 10);

        assert_ne!(expected, 99.0);
        assert_eq!(content.content_height, expected as usize);
        assert_eq!(
            rich_content_height(&model, agent_block.id()),
            Some(expected)
        );
    });
}

#[test]
fn tui_agent_overhang_remeasures_near_offscreen_non_dirty_height() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // A short terminal block pushes the agent block below a 1-row viewport
        // but within the overhang band.
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 3, 99.0);
        let expected = app.read(|app| agent_block.as_ref(app).desired_height(80, app) as f64);

        request_top_window(&app, &source, 1);

        assert_ne!(expected, 99.0);
        assert_eq!(
            rich_content_height(&model, agent_block.id()),
            Some(expected)
        );
    });
}

#[test]
fn tui_agent_beyond_overhang_keeps_stale_height() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // A tall terminal block pushes the agent block beyond the overhang band.
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 30, 7.0);

        request_top_window(&app, &source, 1);

        // Beyond OVERHANG_ROWS: not re-measured, so the stale height is retained.
        assert_eq!(rich_content_height(&model, agent_block.id()), Some(7.0));
    });
}

/// Builds a source over one registered agent block seeded with a stale,
/// non-dirty cached height. When `preceding_rows > 0`, a terminal block of that
/// many output rows precedes it, controlling how far below the top it sits.
fn seeded_agent_block_source(
    app: &mut App,
    preceding_rows: usize,
    stale_height: f64,
) -> (
    TuiBlockListViewportSource,
    Arc<FairMutex<TerminalModel>>,
    ViewHandle<TuiAIBlock>,
) {
    let mut model = TerminalModel::mock(None, None);
    if preceding_rows > 0 {
        model.simulate_block("printf", &"x\r\n".repeat(preceding_rows));
    }
    let terminal_model = Arc::new(FairMutex::new(model));
    let agent_block = add_agent_block(app, "hello world from rust");
    let view_id = agent_block.id();
    {
        let mut model = terminal_model.lock();
        model.block_list_mut().append_rich_content(
            RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
            false,
        );
        // Clear the dirty flag and seed a stale height so only re-measurement
        // (not the dirty path) can correct it.
        model.block_list_mut().take_dirty_rich_content_items();
        model
            .block_list_mut()
            .update_rich_content_heights(&HashMap::from([(view_id, stale_height)]));
    }
    let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::from([(
        view_id,
        agent_block.clone(),
    )])));
    let source = TuiBlockListViewportSource::new(terminal_model.clone(), agent_blocks);
    (source, terminal_model, agent_block)
}

/// Runs the overhang + windowing pass for a top-anchored viewport at width 80.
fn request_top_window(
    app: &App,
    source: &TuiBlockListViewportSource,
    viewport_height: u16,
) -> TuiViewportContent {
    app.read(|app| {
        source.visible_items(
            TuiViewportWindow {
                scroll_top: 0,
                viewport_height,
            },
            80,
            app,
        )
    })
}

/// Adds a `TuiAIBlock` backed by a single-query model in a fresh TUI
/// window and returns its handle.
fn add_agent_block(app: &mut App, query: &str) -> ViewHandle<TuiAIBlock> {
    let query = query.to_owned();
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_tui_view(window_id, move |_| {
            TuiAIBlock::new(
                AIConversationId::new(),
                AIAgentExchangeId::new(),
                Rc::new(QueryAgentBlockModel {
                    inputs: vec![query_input(&query)],
                }),
            )
        })
    })
}

struct QueryAgentBlockModel {
    inputs: Vec<AIAgentInput>,
}

struct TestHostView;

impl Entity for TestHostView {
    type Event = ();
}

impl TuiView for TestHostView {
    fn ui_name() -> &'static str {
        "TestHostView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiText::new(""))
    }
}

impl TypedActionView for TestHostView {
    type Action = ();
}

impl AIBlockModel for QueryAgentBlockModel {
    type View = TuiAIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        AIBlockOutputStatus::Pending
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

/// Builds one user-query input for wrapping-height tests.
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

/// Returns the cached rich-content height for a view ID.
fn rich_content_height(model: &Arc<FairMutex<TerminalModel>>, view_id: EntityId) -> Option<f64> {
    model
        .lock()
        .block_list()
        .block_heights()
        .cursor::<(), ()>()
        .find_map(|item| match item {
            BlockHeightItem::RichContent(item) if item.view_id == view_id => {
                Some(item.last_laid_out_height.as_f64())
            }
            BlockHeightItem::Block(_)
            | BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent(_) => None,
        })
}

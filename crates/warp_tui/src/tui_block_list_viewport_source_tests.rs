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
use warpui::{AddWindowOptions, EntityId, EntityIdMap};
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiLayoutContext, TuiSize, TuiText, TuiViewportWindow,
    TuiViewportedElement, TuiViewportedList, TuiViewportedListState,
};
use warpui_core::{App, AppContext, Entity, TuiView, TypedActionView, ViewContext};

use super::{
    block_rows, AgentBlockRegistry, TuiBlockListViewportItemId, TuiBlockListViewportSource,
};
use crate::agent_block::TuiAgentBlockView;

#[test]
fn tui_block_list_viewport_source_uses_canonical_block_list_order() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_block("echo 1", "1\r\n");
    model.simulate_block("echo 2", "2\r\n");
    let expected = model
        .block_list()
        .blocks()
        .iter()
        .filter(|block| block_rows(block, model.block_list()).is_some())
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
            assert_eq!(item.origin_y, 0);

            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = item.element.layout(
                TuiConstraint::loose(TuiSize::new(80, u16::MAX)),
                &mut layout_ctx,
                app,
            );
            assert_eq!(size.height, 4);
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
fn tui_agent_rich_content_updates_visible_height_from_viewport_layout() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::new()));
        let agent_block = app.update(|ctx| {
            let (window_id, _) = ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            );
            ctx.add_tui_view(window_id, |_| {
                TuiAgentBlockView::new(
                    AIConversationId::new(),
                    AIAgentExchangeId::new(),
                    Rc::new(QueryAgentBlockModel {
                        inputs: vec![query_input("hello world from rust")],
                    }),
                )
            })
        });
        let view_id = agent_block.id();
        {
            let mut model = terminal_model.lock();
            model.block_list_mut().append_rich_content(
                RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
                false,
            );
            model.block_list_mut().take_dirty_rich_content_items();
            model
                .block_list_mut()
                .update_rich_content_heights(&HashMap::from([(view_id, 4.0)]));
        }
        agent_blocks
            .borrow_mut()
            .insert(view_id, agent_block.clone());
        let source = TuiBlockListViewportSource::new(terminal_model.clone(), agent_blocks);

        let content = app.read(|app| {
            source.visible_items(
                TuiViewportWindow {
                    scroll_top: 0,
                    viewport_height: 10,
                },
                80,
                app,
            )
        });
        let expected_height =
            app.read(|app| agent_block.as_ref(app).desired_height(80, app) as f64);
        assert_eq!(content.content_height, 4);
        assert_eq!(rich_content_height(&terminal_model, view_id), Some(4.0));

        let mut viewport = TuiViewportedList::new(TuiViewportedListState::new_at_end(), source);
        app.read(|app| {
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            viewport.layout(
                TuiConstraint::tight(TuiSize::new(80, 10)),
                &mut layout_ctx,
                app,
            );
        });
        assert_eq!(
            rich_content_height(&terminal_model, view_id),
            Some(expected_height)
        );
    });
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
    type View = TuiAgentBlockView;

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

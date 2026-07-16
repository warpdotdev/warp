use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentExchangeId, AIAgentInput, AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType,
    AIAgentText, AIAgentTextSection, AIAgentTodo, AIBlockModel, AIBlockOutputStatus,
    AIConversationId, AIRequestType, Appearance, BlockHeightItem, BlocklistAIHistoryEvent,
    ConversationStatus, ConversationStatusUpdate, LLMId, MessageId, OutputStatusUpdateCallback,
    RichContentItem, RichContentType, ServerOutputId, Shared, TerminalModel, TodoOperation,
    UserQueryMode,
};
use warpui::event::ModifiersState;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, EntityId, EntityIdMap, TuiView};
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScene, TuiScreenPosition,
    TuiSize,
};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{AppContext, ViewContext, WindowInvalidation};

use super::TuiTranscriptView;
use crate::agent_block::TuiAIBlock;
use crate::test_fixtures::add_test_action_model_and_events;

#[test]
fn transcript_view_renders_terminal_blocks_from_canonical_order() {
    App::test((), |mut app| async move {
        let mut terminal_model = TerminalModel::mock(None, None);
        terminal_model.simulate_block("echo 1", "1\r\n");
        let terminal_model = Arc::new(FairMutex::new(terminal_model));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        EntityId::new(),
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });

        let mut presenter = TuiPresenter::new();
        let frame =
            app.update(|ctx| presenter.present(ctx, &transcript, TuiRect::new(0, 0, 80, 20)));
        let text = frame.buffer.to_lines().join("\n");

        assert!(
            text.contains("echo 1"),
            "transcript should render command input:\n{text}"
        );
        assert!(
            text.contains('1'),
            "transcript should render command output:\n{text}"
        );
    });
}

#[test]
fn transcript_clear_event_removes_only_named_conversations() {
    App::test((), |mut app| async move {
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let terminal_surface_id = EntityId::new();
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        terminal_surface_id,
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });
        let provisional_conversation_id = AIConversationId::new();
        let restored_conversation_id = AIConversationId::new();

        insert_test_agent_block(
            &mut app,
            &transcript,
            provisional_conversation_id,
            AIAgentExchangeId::new(),
            vec![query_input("provisional")],
        );
        insert_test_agent_block(
            &mut app,
            &transcript,
            restored_conversation_id,
            AIAgentExchangeId::new(),
            vec![query_input("restored")],
        );

        transcript.update(&mut app, |view, ctx| {
            view.handle_history_event(
                &BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                    terminal_surface_id,
                    active_conversation_id: Some(provisional_conversation_id),
                    cleared_conversation_ids: vec![provisional_conversation_id],
                },
                ctx,
            );
        });

        transcript.read(&app, |view, app| {
            let conversation_ids = view
                .agent_blocks
                .borrow()
                .values()
                .map(|block| block.as_ref(app).conversation_id())
                .collect::<Vec<_>>();
            assert_eq!(conversation_ids, vec![restored_conversation_id]);
        });
        assert_eq!(rich_content_count(&terminal_model), 1);
    });
}

struct FakeAgentBlockModel {
    inputs: Vec<AIAgentInput>,
    status: AIBlockOutputStatus,
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

#[test]
fn transcript_agent_block_lifecycle_updates_canonical_rich_content() {
    App::test((), |mut app| async move {
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        EntityId::new(),
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });
        let original_conversation_id = AIConversationId::new();
        let exchange_id = AIAgentExchangeId::new();

        insert_test_agent_block(
            &mut app,
            &transcript,
            original_conversation_id,
            exchange_id,
            Vec::new(),
        );
        let agent_block_id = transcript.read(&app, |view, _| {
            assert_eq!(view.agent_blocks.borrow().len(), 1);
            *view.agent_blocks.borrow().keys().next().unwrap()
        });
        assert!(
            take_dirty_rich_content_items(&terminal_model).contains(&agent_block_id),
            "appended TUI agent rich content should be dirty in the canonical block list"
        );
        assert_eq!(rich_content_count(&terminal_model), 1);

        transcript.update(&mut app, |view, ctx| {
            view.mark_exchange_dirty(exchange_id, ctx);
        });
        assert!(
            take_dirty_rich_content_items(&terminal_model).contains(&agent_block_id),
            "streaming updates should dirty canonical rich content"
        );
        transcript.read(&app, |view, app| {
            let agent_blocks = view.agent_blocks.borrow();
            let agent_block = agent_blocks
                .values()
                .next()
                .expect("agent block should remain tracked");
            assert_eq!(
                agent_block.as_ref(app).conversation_id(),
                original_conversation_id
            );
        });
        assert_eq!(rich_content_count(&terminal_model), 1);

        transcript.update(&mut app, |view, ctx| {
            view.remove_conversation(original_conversation_id, ctx)
        });
        transcript.read(&app, |view, _| {
            assert!(view.agent_blocks.borrow().is_empty());
        });
        assert_eq!(rich_content_count(&terminal_model), 0);
    });
}

#[test]
fn todo_and_conversation_status_events_dirty_affected_agent_blocks() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        terminal_surface_id,
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });
        let first_conversation_id = AIConversationId::new();
        let second_conversation_id = AIConversationId::new();

        // Only the first block renders todo content; the second is plain.
        let (todo_block_id, _plain_block_id) = transcript.update(&mut app, |view, ctx| {
            (
                append_test_agent_block(
                    view,
                    first_conversation_id,
                    AIAgentExchangeId::new(),
                    todo_output_status(),
                    ctx,
                ),
                append_test_agent_block(
                    view,
                    second_conversation_id,
                    AIAgentExchangeId::new(),
                    AIBlockOutputStatus::Pending,
                    ctx,
                ),
            )
        });
        // Drain append invalidations so each event's effects are isolated.
        take_dirty_rich_content_items(&terminal_model);

        transcript.update(&mut app, |view, ctx| {
            view.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedTodoList {
                    terminal_surface_id,
                },
                ctx,
            );
        });
        assert_eq!(
            take_dirty_rich_content_items(&terminal_model),
            [todo_block_id].into_iter().collect(),
            "a todo update restyles todo-rendering blocks only; plain blocks keep their layout"
        );

        transcript.update(&mut app, |view, ctx| {
            view.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    conversation_id: first_conversation_id,
                    terminal_surface_id,
                    update: ConversationStatusUpdate::Changed {
                        prev_status: ConversationStatus::InProgress,
                    },
                    new_status: ConversationStatus::Success,
                },
                ctx,
            );
        });
        assert_eq!(
            take_dirty_rich_content_items(&terminal_model),
            [todo_block_id].into_iter().collect(),
            "a status update should only dirty todo-rendering blocks from that conversation"
        );

        transcript.update(&mut app, |view, ctx| {
            view.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    conversation_id: second_conversation_id,
                    terminal_surface_id,
                    update: ConversationStatusUpdate::Changed {
                        prev_status: ConversationStatus::InProgress,
                    },
                    new_status: ConversationStatus::Success,
                },
                ctx,
            );
        });
        assert!(
            take_dirty_rich_content_items(&terminal_model).is_empty(),
            "a status update for a conversation without todo blocks dirties nothing"
        );
    });
}

/// A completed output whose only message is an `UpdateTodos` operation, so
/// the owning block renders todo content.
fn todo_output_status() -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("todo-1".to_owned()),
                message: AIAgentOutputMessageType::TodoOperation(TodoOperation::UpdateTodos {
                    todos: vec![AIAgentTodo::new(
                        "t1".to_owned().into(),
                        "task".to_owned(),
                        String::new(),
                    )],
                }),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}
#[test]
fn transcript_view_scrolls_only_with_the_mouse_wheel() {
    App::test((), |mut app| async move {
        let mut terminal_model = TerminalModel::mock(None, None);
        for index in 0..8 {
            let command = format!("echo {index}");
            let output = format!("{index}\r\n");
            terminal_model.simulate_block(command.as_str(), output.as_str());
        }
        let terminal_model = Arc::new(FairMutex::new(terminal_model));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        EntityId::new(),
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });
        let mut element = transcript.read(&app, |view, app| view.render(app));
        let area = TuiRect::new(0, 0, 40, 4);

        let bottom = render_element(&app, element.as_mut(), area);
        assert!(transcript.read(&app, |view, _| view.viewport.is_at_end()));
        let page_up = TuiEvent::KeyDown {
            keystroke: Keystroke {
                key: "pageup".to_owned(),
                ..Default::default()
            },
            chars: String::new(),
            details: Default::default(),
            is_composing: false,
        };
        assert!(!dispatch_event(&app, element.as_mut(), area, &page_up));
        assert_eq!(render_element(&app, element.as_mut(), area), bottom);

        assert!(dispatch_scroll(&app, element.as_mut(), area, 1));
        let scrolled = render_element(&app, element.as_mut(), area);
        assert_ne!(scrolled, bottom);
        assert!(!transcript.read(&app, |view, _| view.viewport.is_at_end()));
        for _ in 0..8 {
            dispatch_scroll(&app, element.as_mut(), area, -1);
        }
        assert_eq!(render_element(&app, element.as_mut(), area), bottom);
        assert!(transcript.read(&app, |view, _| view.viewport.is_at_end()));
    });
}

#[test]
fn presenter_draw_resolves_agent_blocks_from_cached_elements() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let (window_id, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        EntityId::new(),
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });
        let agent_block_id = insert_test_agent_block(
            &mut app,
            &transcript,
            AIConversationId::new(),
            AIAgentExchangeId::new(),
            vec![query_input("hello agent")],
        );

        // Mirror the runtime's draw: `invalidate` renders changed views into
        // the presenter's cache, then `present` resolves the agent block via
        // `TuiChildView` for both measurement and painting.
        let mut presenter = TuiPresenter::new();
        let frame = app.update(|ctx| {
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(transcript.id());
            invalidation.updated.insert(agent_block_id);
            presenter.invalidate(&invalidation, ctx, window_id);
            presenter.present(ctx, &transcript, TuiRect::new(0, 0, 40, 10))
        });
        let text = frame.buffer.to_lines().join("\n");

        assert!(
            text.contains("hello agent"),
            "agent block content should render through the presenter cache:\n{text}"
        );
        assert_eq!(
            app.read(|ctx| ctx.view_ancestors(window_id, agent_block_id)),
            vec![transcript.id(), agent_block_id],
        );
    });
}

#[test]
fn dragging_inside_markdown_highlights_transcript_text() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let model_for_view = terminal_model.clone();
        let (action_model, model_events) = add_test_action_model_and_events(&mut app);
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    TuiTranscriptView::new(
                        EntityId::new(),
                        model_for_view,
                        action_model,
                        &model_events,
                        ctx,
                    )
                },
            )
        });
        let agent_block_id = transcript.update(&mut app, |view, ctx| {
            append_test_agent_block_with_inputs(
                view,
                AIConversationId::new(),
                AIAgentExchangeId::new(),
                vec![query_input("hello agent")],
                markdown_output_status(
                    "# Overview\n\nDrag selectable text.\n\n```rust\nfn main() {}\n```",
                ),
                ctx,
            )
        });
        let agent_block = app.read(|ctx| {
            transcript
                .as_ref(ctx)
                .agent_blocks
                .borrow()
                .get(&agent_block_id)
                .expect("agent block should be registered")
                .clone()
        });
        let mut rendered_views = app.read(|ctx| {
            let mut rendered_views = EntityIdMap::default();
            rendered_views.insert(agent_block_id, agent_block.as_ref(ctx).render(ctx));
            rendered_views
        });
        let mut element = app.read(|ctx| transcript.as_ref(ctx).render(ctx));
        let area = TuiRect::new(0, 0, 40, 14);
        assert_drag_highlights_text(
            &app,
            &transcript,
            element.as_mut(),
            &mut rendered_views,
            area,
            "selectable",
            "Markdown",
        );

        assert_drag_highlights_text(
            &app,
            &transcript,
            element.as_mut(),
            &mut rendered_views,
            area,
            "hello agent",
            "background-painted input",
        );
        assert_drag_highlights_text(
            &app,
            &transcript,
            element.as_mut(),
            &mut rendered_views,
            area,
            "main",
            "bordered Markdown code",
        );
    });
}

fn assert_drag_highlights_text(
    app: &App,
    transcript: &warpui::ViewHandle<TuiTranscriptView>,
    element: &mut dyn TuiElement,
    rendered_views: &mut EntityIdMap<Box<dyn TuiElement>>,
    area: TuiRect,
    target: &str,
    description: &str,
) {
    let (initial, scene) = render_retained_element(app, element, rendered_views, area);
    let lines = initial.to_lines();
    let row = lines
        .iter()
        .position(|line| line.contains(target))
        .unwrap_or_else(|| panic!("rendered transcript should contain {description}: {target}"));
    let start_byte = lines[row]
        .find(target)
        .expect("located row should contain selection target");
    let start = lines[row][..start_byte].chars().count() as u16;
    let end = start + target.chars().count() as u16 - 1;

    assert!(dispatch_retained_event(
        app,
        transcript.id(),
        element,
        rendered_views,
        scene,
        &TuiEvent::LeftMouseDown {
            position: (start, row as u16).into(),
            modifiers: ModifiersState::default(),
            click_count: 1,
            is_first_mouse: false,
        },
    ));
    let (_, scene) = render_retained_element(app, element, rendered_views, area);
    assert!(dispatch_retained_event(
        app,
        transcript.id(),
        element,
        rendered_views,
        scene,
        &TuiEvent::LeftMouseDragged {
            position: (end, row as u16).into(),
            modifiers: ModifiersState::default(),
        },
    ));

    let (selected, scene) = render_retained_element(app, element, rendered_views, area);
    for column in start..=end {
        assert!(
            selected[(column, row as u16)]
                .modifier
                .contains(Modifier::REVERSED),
            "selected {description} cell at column {column} should be reversed"
        );
    }

    assert!(dispatch_retained_event(
        app,
        transcript.id(),
        element,
        rendered_views,
        scene,
        &TuiEvent::LeftMouseUp {
            position: (end, row as u16).into(),
            modifiers: ModifiersState::default(),
        },
    ));
    let (settled, _) = render_retained_element(app, element, rendered_views, area);
    for column in start..=end {
        assert!(
            settled[(column, row as u16)]
                .modifier
                .contains(Modifier::REVERSED),
            "{description} selection should persist after mouse-up"
        );
    }
}
/// Registers an agent block over a fake model with `inputs` on the transcript
/// and appends its canonical rich-content item, returning the block's view id.
fn insert_test_agent_block(
    app: &mut App,
    transcript: &warpui::ViewHandle<TuiTranscriptView>,
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    inputs: Vec<AIAgentInput>,
) -> EntityId {
    transcript.update(app, |view, ctx| {
        let action_model = view.action_model.clone();
        let model_events = view.model_events.clone();
        let terminal_model = view.model.clone();
        let agent_block = ctx.add_typed_action_tui_view(|ctx| {
            TuiAIBlock::new(
                conversation_id,
                exchange_id,
                Rc::new(FakeAgentBlockModel {
                    inputs,
                    status: AIBlockOutputStatus::Pending,
                }),
                action_model,
                &model_events,
                terminal_model,
                ctx,
            )
        });
        let agent_block_id = agent_block.id();
        view.agent_blocks
            .borrow_mut()
            .insert(agent_block_id, agent_block);
        view.model.lock().block_list_mut().append_rich_content(
            RichContentItem::new(Some(RichContentType::AIBlock), agent_block_id, None, false),
            false,
        );
        ctx.notify();
        agent_block_id
    })
}

/// Builds one user-query input for agent-block rendering tests.
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

fn markdown_output_status(markdown: &str) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("markdown-1".to_owned()),
                message: AIAgentOutputMessageType::Text(AIAgentText {
                    sections: vec![AIAgentTextSection::PlainText {
                        text: markdown.to_owned().into(),
                    }],
                }),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}

fn render_retained_element(
    app: &App,
    element: &mut dyn TuiElement,
    rendered_views: &mut EntityIdMap<Box<dyn TuiElement>>,
    area: TuiRect,
) -> (TuiBuffer, Rc<TuiScene>) {
    app.read(|app| {
        let mut layout_ctx = TuiLayoutContext { rendered_views };
        element.layout(
            TuiConstraint::tight(TuiSize::new(area.width, area.height)),
            &mut layout_ctx,
            app,
        );
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(
                TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
                &mut surface,
                &mut paint_ctx,
            );
        }
        let scene = Rc::new(paint_ctx.scene.clone());
        (buffer, scene)
    })
}

fn dispatch_retained_event(
    app: &App,
    origin_view_id: EntityId,
    element: &mut dyn TuiElement,
    rendered_views: &mut EntityIdMap<Box<dyn TuiElement>>,
    scene: Rc<TuiScene>,
    event: &TuiEvent,
) -> bool {
    app.read(|app| {
        let mut event_ctx = TuiEventContext::new(scene, rendered_views);
        event_ctx.set_origin_view(Some(origin_view_id));
        element.dispatch_event(event, &mut event_ctx, app)
    })
}
/// Lays out and renders a retained TUI element.
fn render_element(app: &App, element: &mut dyn TuiElement, area: TuiRect) -> Vec<String> {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        element.layout(
            TuiConstraint::tight(TuiSize::new(area.width, area.height)),
            &mut ctx,
            app,
        );
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(
                TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
                &mut surface,
                &mut paint_ctx,
            );
        }
        buffer.to_lines()
    })
}

/// Dispatches a vertical wheel movement to a retained TUI element.
fn dispatch_scroll(app: &App, element: &mut dyn TuiElement, area: TuiRect, delta_y: isize) -> bool {
    dispatch_event(
        app,
        element,
        area,
        &TuiEvent::ScrollWheel {
            position: (area.x, area.y).into(),
            delta: (0, delta_y),
            precise: false,
            modifiers: ModifiersState::default(),
        },
    )
}

/// Dispatches an event to a retained TUI element.
fn dispatch_event(
    app: &App,
    element: &mut dyn TuiElement,
    area: TuiRect,
    event: &TuiEvent,
) -> bool {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(
                TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
                &mut surface,
                &mut paint_ctx,
            );
        }
        let scene = Rc::new(paint_ctx.scene.clone());
        drop(paint_ctx);
        let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
        event_ctx.set_origin_view(Some(EntityId::new()));
        element.dispatch_event(event, &mut event_ctx, app)
    })
}

fn append_test_agent_block(
    view: &mut TuiTranscriptView,
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    status: AIBlockOutputStatus,
    ctx: &mut ViewContext<TuiTranscriptView>,
) -> EntityId {
    append_test_agent_block_with_inputs(view, conversation_id, exchange_id, Vec::new(), status, ctx)
}

fn append_test_agent_block_with_inputs(
    view: &mut TuiTranscriptView,
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    inputs: Vec<AIAgentInput>,
    status: AIBlockOutputStatus,
    ctx: &mut ViewContext<TuiTranscriptView>,
) -> EntityId {
    let action_model = view.action_model.clone();
    let model_events = view.model_events.clone();
    let terminal_model = view.model.clone();
    let agent_block = ctx.add_tui_view(|ctx| {
        TuiAIBlock::new(
            conversation_id,
            exchange_id,
            Rc::new(FakeAgentBlockModel { inputs, status }),
            action_model,
            &model_events,
            terminal_model,
            ctx,
        )
    });
    let view_id = agent_block.id();
    view.agent_blocks.borrow_mut().insert(view_id, agent_block);
    view.model.lock().block_list_mut().append_rich_content(
        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
        false,
    );
    view_id
}
fn rich_content_count(model: &Arc<FairMutex<TerminalModel>>) -> usize {
    model
        .lock()
        .block_list()
        .block_heights()
        .cursor::<(), ()>()
        .filter(|item| matches!(item, BlockHeightItem::RichContent(_)))
        .count()
}

fn take_dirty_rich_content_items(
    model: &Arc<FairMutex<TerminalModel>>,
) -> std::collections::HashSet<EntityId> {
    model
        .lock()
        .block_list_mut()
        .take_dirty_rich_content_items()
}

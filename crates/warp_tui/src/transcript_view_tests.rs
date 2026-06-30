use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentExchangeId, AIConversationId, BlockHeightItem, BlocklistAIHistoryModel, TerminalModel,
};
use warpui::event::ModifiersState;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, EntityId, EntityIdMap, TuiView};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiRect, TuiSize,
};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;

use super::TuiTranscriptView;

#[test]
fn transcript_view_renders_terminal_blocks_from_canonical_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let mut terminal_model = TerminalModel::mock(None, None);
        terminal_model.simulate_block("echo 1", "1\r\n");
        let terminal_model = Arc::new(FairMutex::new(terminal_model));
        let model_for_view = terminal_model.clone();
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| TuiTranscriptView::new(EntityId::new(), model_for_view, ctx),
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
fn transcript_agent_block_lifecycle_updates_canonical_rich_content() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let model_for_view = terminal_model.clone();
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| TuiTranscriptView::new(EntityId::new(), model_for_view, ctx),
            )
        });
        let original_conversation_id = AIConversationId::new();
        let reassigned_conversation_id = AIConversationId::new();
        let exchange_id = AIAgentExchangeId::new();

        transcript.update(&mut app, |view, ctx| {
            view.insert_agent_block(original_conversation_id, exchange_id, ctx)
        });
        transcript.read(&app, |view, _| {
            assert_eq!(view.agent_blocks.borrow().len(), 1);
            assert_eq!(view.dirty_agent_blocks.borrow().len(), 1);
        });
        assert_eq!(rich_content_count(&terminal_model), 1);

        transcript.update(&mut app, |view, ctx| {
            view.reassign_exchange(exchange_id, reassigned_conversation_id, ctx);
            view.mark_exchange_dirty(exchange_id, ctx);
        });
        transcript.read(&app, |view, _| {
            let registrations = view.agent_blocks.borrow();
            let registration = registrations
                .values()
                .next()
                .expect("agent block should remain registered");
            assert_eq!(registration.conversation_id, reassigned_conversation_id);
        });
        assert_eq!(rich_content_count(&terminal_model), 1);

        transcript.update(&mut app, |view, ctx| {
            view.remove_conversation(reassigned_conversation_id, ctx)
        });
        transcript.read(&app, |view, _| {
            assert!(view.agent_blocks.borrow().is_empty());
        });
        assert_eq!(rich_content_count(&terminal_model), 0);
    });
}

#[test]
fn transcript_view_scrolls_only_with_the_mouse_wheel() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let mut terminal_model = TerminalModel::mock(None, None);
        for index in 0..8 {
            let command = format!("echo {index}");
            let output = format!("{index}\r\n");
            terminal_model.simulate_block(command.as_str(), output.as_str());
        }
        let terminal_model = Arc::new(FairMutex::new(terminal_model));
        let model_for_view = terminal_model.clone();
        let (_, transcript) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| TuiTranscriptView::new(EntityId::new(), model_for_view, ctx),
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
        element.render(area, &mut buffer, &mut ctx);
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
            position: (area.x, area.y),
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
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut event_ctx = TuiEventContext::default();
        event_ctx.set_origin_view(Some(EntityId::new()));
        element.dispatch_event(event, area, &mut event_ctx, &mut layout_ctx, app)
    })
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

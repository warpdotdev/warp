use uuid::Uuid;
use warp::tui_export::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect,
    TuiScreenPosition, TuiSize,
};
use warpui_core::{TuiView as _, TypedActionView as _, ViewContext};

use super::{
    CALLBACK_FAILURE_MESSAGE, MANUAL_FAILURE_MESSAGE, TuiGrokOAuthBlock, TuiGrokOAuthBlockAction,
    TuiGrokOAuthPhase,
};
use crate::editor_view::TuiEditorView;
use crate::tui_builder::TuiUiBuilder;

pub(crate) fn new_block(ctx: &mut ViewContext<TuiGrokOAuthBlock>) -> TuiGrokOAuthBlock {
    TuiGrokOAuthBlock {
        active_attempt_id: Some(Uuid::new_v4()),
        manual_exchange: None,
        cancellation: None,
        code_editor: ctx.add_typed_action_tui_view(TuiEditorView::single_line),
        phase: TuiGrokOAuthPhase::Waiting { manual_error: None },
        callback_error: None,
    }
}

#[test]
fn waiting_card_uses_handoff_structure_and_only_escape_footer_hint() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, block) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                new_block,
            )
        });

        app.read(|ctx| {
            let mut element = block.as_ref(ctx).render(ctx);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = element.layout(
                TuiConstraint::loose(TuiSize::new(80, 20)),
                &mut layout_ctx,
                ctx,
            );
            let area = TuiRect::new(0, 0, size.width, size.height);
            let mut buffer = warpui_core::elements::tui::TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);

            let lines = buffer.to_lines();
            assert!(lines[0].trim().is_empty(), "{lines:#?}");
            assert!(lines[1].contains("Connect Grok"), "{lines:#?}");
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Complete sign-in in the browser")),
                "{lines:#?}"
            );
            assert!(
                lines.iter().any(|line| line.contains("paste it below")),
                "{lines:#?}"
            );
            assert_eq!(
                lines.last().map(|line| line.trim()),
                Some("Esc to close"),
                "{lines:#?}"
            );
            let rendered = lines.join("\n");
            assert!(!rendered.contains("open URL"), "{rendered}");
            assert!(!rendered.contains("Open link"), "{rendered}");

            let builder = TuiUiBuilder::from_app(ctx);
            assert_eq!(
                buffer[(1, 1)].fg,
                builder
                    .grok_oauth_accent_style()
                    .fg
                    .expect("Grok accent has a foreground")
            );
            assert_eq!(buffer[(0, 1)].bg, builder.grok_oauth_header_background());
            assert_eq!(buffer[(0, 2)].bg, builder.grok_oauth_surface_background());
        });
    });
}

#[test]
fn callback_and_manual_failures_do_not_claim_success_or_expose_raw_details() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, block) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                new_block,
            )
        });

        block.update(&mut app, |block, ctx| {
            let attempt_id = block
                .active_attempt_id
                .expect("test block has an active attempt");
            block.phase = TuiGrokOAuthPhase::ExchangingManualCode;
            block.handle_callback_result(
                attempt_id,
                Err(anyhow::anyhow!("raw-callback-secret")),
                ctx,
            );
            assert_eq!(
                block.callback_error.as_deref(),
                Some(CALLBACK_FAILURE_MESSAGE)
            );
            assert!(matches!(
                &block.phase,
                TuiGrokOAuthPhase::ExchangingManualCode
            ));

            block.handle_manual_result(attempt_id, Err(anyhow::anyhow!("raw-manual-secret")), ctx);
            assert!(matches!(
                &block.phase,
                TuiGrokOAuthPhase::Fatal(message) if message == CALLBACK_FAILURE_MESSAGE
            ));
            assert!(block.is_active());

            block.phase = TuiGrokOAuthPhase::Waiting { manual_error: None };
            block.callback_error = None;
            block.handle_manual_result(attempt_id, Err(anyhow::anyhow!("another-raw-secret")), ctx);
            assert!(matches!(
                &block.phase,
                TuiGrokOAuthPhase::Waiting {
                    manual_error: Some(message)
                } if message == MANUAL_FAILURE_MESSAGE
            ));

            block.handle_action(&TuiGrokOAuthBlockAction::Cancel, ctx);
            block.handle_callback_result(attempt_id, Err(anyhow::anyhow!("stale-raw-secret")), ctx);
            assert!(!block.is_active());
            assert!(block.callback_error.is_none());
        });
    });
}

#[test]
fn fatal_card_sanitizes_the_body_and_escape_closes_the_attempt() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, block) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                new_block,
            )
        });
        block.update(&mut app, |block, _| {
            block.phase =
                TuiGrokOAuthPhase::Fatal("Authorization failed without exposing a code".to_owned());
        });
        let rendered = app.read(|ctx| {
            let mut element = block.as_ref(ctx).render(ctx);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = element.layout(
                TuiConstraint::loose(TuiSize::new(80, 20)),
                &mut layout_ctx,
                ctx,
            );
            let area = TuiRect::new(0, 0, size.width, size.height);
            let mut buffer = warpui_core::elements::tui::TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            buffer.to_lines().join("\n")
        });
        assert!(
            rendered.contains("Authorization failed without exposing a code"),
            "{rendered}"
        );

        block.update(&mut app, |block, ctx| {
            block.handle_action(&TuiGrokOAuthBlockAction::Cancel, ctx);
            assert!(!block.is_active());
        });
    });
}

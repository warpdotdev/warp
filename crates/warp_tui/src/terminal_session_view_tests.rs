use warp::appearance::Appearance;
use warp::tui_export::{
    export_conversation_markdown, PtyIntent, PtyIntentEvent, SizeInfo, SizeUpdate,
};
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::{App, AppContext};

use super::{
    export_file_success_message, raw_prompt_if_not_blank, render_left_footer_hint,
    TuiTerminalSessionEvent,
};
use crate::tui_builder::TuiUiBuilder;

fn render_element(mut element: Box<dyn TuiElement>, ctx: &AppContext, width: u16) -> TuiBuffer {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(width, 1)),
        &mut layout_ctx,
        ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
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
    buffer
}

#[test]
fn footer_falls_back_to_conversations_callout() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let buffer = render_element(
                render_left_footer_hint(None, true, &builder)
                    .expect("empty input should show the conversations callout"),
                ctx,
                40,
            );

            assert_eq!(buffer.to_lines(), vec!["← for conversations"]);
            assert_eq!(
                buffer[(0, 0)].fg,
                builder
                    .accent_text_style()
                    .fg
                    .expect("accent text has a foreground")
            );
            assert_eq!(
                buffer[(1, 0)].fg,
                builder
                    .muted_text_style()
                    .fg
                    .expect("muted text has a foreground")
            );
        });
    });
}

#[test]
fn transient_footer_hint_replaces_conversations_callout() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let buffer = render_element(
                render_left_footer_hint(
                    Some(("temporary hint", builder.muted_text_style())),
                    false,
                    &builder,
                )
                .expect("transient hints remain visible when input has text"),
                ctx,
                40,
            );

            assert_eq!(buffer.to_lines(), vec!["temporary hint"]);
        });
    });
}

#[test]
fn conversations_callout_is_hidden_when_input_has_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);

            assert!(render_left_footer_hint(None, false, &builder).is_none());
        });
    });
}
#[test]
fn interrupt_event_projects_to_high_level_pty_intent() {
    let event = TuiTerminalSessionEvent::InterruptPty;
    assert!(matches!(event.pty_intent(), Some(PtyIntent::Interrupt)));
}

#[test]
fn user_input_event_projects_to_raw_user_bytes() {
    let event = TuiTerminalSessionEvent::WriteUserInput(b"hello\r".to_vec().into());
    let Some(PtyIntent::WriteBytes(bytes)) = event.pty_intent() else {
        panic!("user input event should map to raw PTY bytes");
    };
    assert_eq!(&*bytes, b"hello\r");
}

#[test]
fn non_command_prompt_preserves_leading_whitespace() {
    assert_eq!(raw_prompt_if_not_blank("  /compact"), Some("  /compact"));
}

#[test]
fn whitespace_only_prompt_is_ignored() {
    assert_eq!(raw_prompt_if_not_blank(" \t\n"), None);
}

#[test]
fn file_export_success_message_includes_destination_path() {
    let directory = tempfile::tempdir().expect("temp directory");
    let export = export_conversation_markdown(
        Some(directory.path().to_str().expect("UTF-8 temp path")),
        Some("conversation.md"),
        None,
        "# Conversation",
    )
    .expect("conversation export");

    assert_eq!(
        export_file_success_message(&export),
        format!("Conversation exported to {}", export.path().display())
    );
}

#[test]
fn resize_event_maps_to_pty_resize_intent() {
    let last_size = SizeInfo::new_without_font_metrics(24, 120);
    let size_update = SizeUpdate::from_cell_dimensions(last_size, 8, 42);
    let event = TuiTerminalSessionEvent::Resize(size_update);

    let Some(PtyIntent::Resize(actual_update)) = event.pty_intent() else {
        panic!("resize event should map to a PTY resize intent");
    };
    assert_eq!(actual_update.new_size().rows(), 8);
    assert_eq!(actual_update.new_size().columns(), 42);
}

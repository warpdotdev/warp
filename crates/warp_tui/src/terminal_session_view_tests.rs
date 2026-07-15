use warp::tui_export::{
    export_conversation_markdown, PtyIntent, PtyIntentEvent, SizeInfo, SizeUpdate,
};
use warpui::{App, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize, TuiText,
};
use warpui_core::AppContext;

use super::{
    append_input_area, export_file_success_message, raw_prompt_if_not_blank,
    TuiTerminalSessionEvent,
};

/// Lays out then paints `element` into a `w`x`h` cell grid and returns the
/// rendered text lines (mirrors the view-test render helpers). Layout must run
/// before render so composite elements populate child sizes.
fn render_lines(
    app_ctx: &AppContext,
    mut element: Box<dyn TuiElement>,
    w: u16,
    h: u16,
) -> Vec<String> {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(w, h)),
        &mut layout_ctx,
        app_ctx,
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
    buffer.to_lines()
}

/// The inline menu, input box, and footer must not be stacked flush: the design
/// mocks separate them with a blank row on each side of the input box.
/// `append_input_area` is what inserts that spacing.
#[test]
fn input_area_pads_between_inline_menu_input_box_and_footer() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let make_menu = || TuiText::new("menu").finish();
            let make_input = || {
                TuiContainer::new(TuiText::new("input").finish())
                    .with_border()
                    .finish()
            };
            let make_footer = || TuiText::new("footer").finish();

            // Pre-fix layout: the three regions stacked flush, with no gaps.
            // The input box border sits directly against the menu and footer.
            let flush = TuiFlex::column()
                .child(make_menu())
                .child(make_input())
                .child(make_footer());
            let flush_lines = render_lines(ctx, flush.finish(), 20, 12);
            let flush_menu = flush_lines
                .iter()
                .position(|line| line.contains("menu"))
                .expect("menu row rendered");
            let flush_footer = flush_lines
                .iter()
                .position(|line| line.contains("footer"))
                .expect("footer row rendered");
            assert!(
                !flush_lines[flush_menu + 1].trim().is_empty(),
                "flush layout should stack the menu and input box with no gap: {flush_lines:?}"
            );
            assert!(
                !flush_lines[flush_footer - 1].trim().is_empty(),
                "flush layout should stack the input box and footer with no gap: {flush_lines:?}"
            );

            // Fixed layout: append_input_area inserts a blank row on each side
            // of the input box.
            let padded = append_input_area(
                TuiFlex::column(),
                Some(make_menu()),
                make_input(),
                make_footer(),
            );
            let lines = render_lines(ctx, padded.finish(), 20, 12);
            let menu = lines
                .iter()
                .position(|line| line.contains("menu"))
                .expect("menu row rendered");
            let footer = lines
                .iter()
                .position(|line| line.contains("footer"))
                .expect("footer row rendered");
            assert!(
                lines[menu + 1].trim().is_empty(),
                "expected a blank padding row between the inline menu and the input box: {lines:?}"
            );
            assert!(
                lines[footer - 1].trim().is_empty(),
                "expected a blank padding row between the input box and the footer: {lines:?}"
            );
        });
    });
}

#[test]
fn interrupt_event_projects_to_high_level_pty_intent() {
    let event = TuiTerminalSessionEvent::InterruptPty;
    assert!(matches!(event.pty_intent(), Some(PtyIntent::Interrupt)));
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

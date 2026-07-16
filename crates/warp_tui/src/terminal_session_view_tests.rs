use warp::appearance::Appearance;
use warp::tui_export::{
    export_conversation_markdown, PtyIntent, PtyIntentEvent, SizeInfo, SizeUpdate,
};
use warpui_core::elements::tui::{
    TuiBufferExt, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiRect, TuiText,
};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::App;

use super::{export_file_success_message, raw_prompt_if_not_blank, TuiTerminalSessionEvent};
use crate::inline_menu::{
    render_inline_menu, TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuSnapshot,
    MAX_INLINE_MENU_ROWS,
};
use crate::tui_builder::TuiUiBuilder;

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

/// Verify that the inline menu is separated from the input box by exactly one
/// blank row. The fix wraps the menu in a `TuiContainer` with
/// `.with_padding_bottom(1)` so the row between the menu's last item and the
/// bordered input box is always empty, matching the Figma design.
#[test]
fn inline_menu_has_one_blank_row_of_padding_above_input_box() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let snapshot = TuiInlineMenuSnapshot {
                header: None,
                rows: vec![TuiInlineMenuRow {
                    title: "/agent".to_owned(),
                    description: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::InlineMenuItem,
                }],
                selected_index: None,
                scroll_offset: 0,
                max_visible_rows: 8,
                status: None,
            };
            let menu_element = render_inline_menu(&snapshot, &builder);
            // Mimic the layout from terminal_session_view::render: the menu is
            // wrapped in TuiContainer::with_padding_bottom(1), then the simulated
            // input box row follows immediately.
            let combined = TuiFlex::column()
                .child(
                    TuiContainer::new(
                        TuiConstrainedBox::new(menu_element)
                            .with_max_rows(MAX_INLINE_MENU_ROWS)
                            .finish(),
                    )
                    .with_padding_bottom(1)
                    .finish(),
                )
                .child(TuiText::new("[input box]").finish())
                .finish();

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(combined, TuiRect::new(0, 0, 40, 5), ctx);
            let lines = frame.buffer.to_lines();

            // Row 0: the menu item line (e.g., "/agent")
            assert!(
                lines[0].contains("/agent"),
                "first row should contain the menu item; got: {:?}",
                lines[0]
            );
            // Row 1: the blank padding row inserted by with_padding_bottom(1)
            assert_eq!(
                lines[1].trim(),
                "",
                "second row should be blank (the 1-row padding below the menu)"
            );
            // Row 2: the simulated input box
            assert!(
                lines[2].contains("[input box]"),
                "third row should contain the input box text; got: {:?}",
                lines[2]
            );
        })
    });
}

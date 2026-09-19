//! Unit tests for format_command_text and the streamed-update helpers in requested_command.rs

use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

use super::{
    CodeEditorRenderOptions, CodeEditorView, StreamedCodeUpdate,
    apply_streamed_command_editor_update, format_command_text, mcp_blocked_title_text,
    mcp_viewing_detail_title_text, streamed_code_update,
};
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[test]
fn single_line_without_newline_is_unchanged_ascii() {
    let input = "echo hello world";
    let output = format_command_text(input);
    assert_eq!(output, input);
}

#[test]
fn single_line_without_newline_preserves_multibyte_characters() {
    let input = "echo 🚀✨";
    let output = format_command_text(input);
    assert_eq!(output, input);

    // Additional sanity check: string is valid UTF-8 and can be iterated by chars without panic
    let collected: String = output.chars().collect();
    assert_eq!(collected, output);
}

#[test]
fn truncates_at_first_newline_and_appends_ellipsis_when_more_content_exists() {
    let input = "cargo build\n--release";
    let output = format_command_text(input);
    assert_eq!(output, "cargo build…");
}

#[test]
fn truncates_at_first_newline_without_ellipsis_when_rest_is_whitespace() {
    let input = "git status\n   \t  ";
    let output = format_command_text(input);
    assert_eq!(output, "git status");
}

#[test]
fn does_not_split_multibyte_char_across_utf8_boundaries_when_newline_follows() {
    // The emoji is a multi-byte sequence; ensure truncation at the newline does not split it.
    let input = "echo 🧪\nthen do something";
    let output = format_command_text(input);
    assert_eq!(output, "echo 🧪…");

    // Validate resulting string is valid UTF-8 by iterating graphemes via chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn preserves_combining_characters_when_newline_is_after_cluster() {
    // "e" + combining acute accent
    // Sanity checks that the formatter doesn't split this unicode sequence
    let composed = format!("{}{}", 'e', '\u{0301}');
    let input = format!("echo {composed}\nnext");
    let output = format_command_text(&input);
    assert_eq!(output, format!("echo {composed}…"));

    // Still valid UTF-8 and same when re-collected from chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn newline_then_multibyte_results_in_ellipsis_only() {
    let input = "\n🚀";
    let output = format_command_text(input);
    assert_eq!(output, "…");

    // Sanity: output remains valid UTF-8
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

/// Constructs a bare `CodeEditorView`, mirroring
/// `ai::blocklist::block_tests::test_code_editor`. Used to exercise
/// `apply_streamed_command_editor_update` -- the exact function
/// `RequestedCommandView::apply_streamed_update` uses to keep its editor in
/// sync with `command_text` -- against a real buffer.
fn test_code_editor(app: &mut App) -> ViewHandle<CodeEditorView> {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);

    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });

    let (_window, editor_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
            ctx,
        )
    });
    editor_view
}

#[test]
fn apply_streamed_command_editor_update_appends_suffix() {
    App::test((), |mut app| async move {
        let editor = test_code_editor(&mut app);
        let update = streamed_code_update("echo hi", "");
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, update, "echo hi", ctx);
        });
        let text = editor.update(&mut app, |editor, ctx| {
            editor.text(ctx).as_str().to_string()
        });
        assert_eq!(text, "echo hi");
    });
}

#[test]
fn apply_streamed_command_editor_update_resets_existing_editor_on_non_prefix_rewrite() {
    App::test((), |mut app| async move {
        let editor = test_code_editor(&mut app);
        let first_update = streamed_code_update("abc", "");
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, first_update, "abc", ctx);
        });
        let text = editor.update(&mut app, |editor, ctx| {
            editor.text(ctx).as_str().to_string()
        });
        assert_eq!(text, "abc");

        // Regression test for a follow-up finding on APP-5288: a boundary-aligned,
        // non-prefix rewrite on an *already-populated* editor must reset it to
        // "XYZq", not corrupt it into "abcq" the way a length-only sync would.
        let second_update = streamed_code_update("XYZq", "abc");
        assert_eq!(second_update, StreamedCodeUpdate::Reset);
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, second_update, "XYZq", ctx);
        });
        let text = editor.update(&mut app, |editor, ctx| {
            editor.text(ctx).as_str().to_string()
        });
        assert_eq!(text, "XYZq");
    });
}

#[test]
fn apply_streamed_command_editor_update_truncates_on_valid_shrink() {
    App::test((), |mut app| async move {
        let editor = test_code_editor(&mut app);
        let first_update = streamed_code_update("cargo build\n``", "");
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, first_update, "cargo build\n``", ctx);
        });

        let second_update = streamed_code_update("cargo build", "cargo build\n``");
        assert_eq!(second_update, StreamedCodeUpdate::Truncate);
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, second_update, "cargo build", ctx);
        });
        let text = editor.update(&mut app, |editor, ctx| {
            editor.text(ctx).as_str().to_string()
        });
        assert_eq!(text, "cargo build");
    });
}

#[test]
fn apply_streamed_command_editor_update_is_noop_when_unchanged() {
    App::test((), |mut app| async move {
        let editor = test_code_editor(&mut app);
        let first_update = streamed_code_update("echo hi", "");
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, first_update, "echo hi", ctx);
        });

        let second_update = streamed_code_update("echo hi", "echo hi");
        assert_eq!(second_update, StreamedCodeUpdate::NoOp);
        editor.update(&mut app, |editor, ctx| {
            apply_streamed_command_editor_update(editor, second_update, "echo hi", ctx);
        });
        let text = editor.update(&mut app, |editor, ctx| {
            editor.text(ctx).as_str().to_string()
        });
        assert_eq!(text, "echo hi");
    });
}

#[test]
fn mcp_blocked_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_blocked_title_text("create_issue", Some("github")),
        "OK if I call MCP tool create_issue on server github"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_tool_name_when_server_unknown() {
    assert_eq!(
        mcp_blocked_title_text("create_issue", None),
        "OK if I call MCP tool create_issue"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_blocked_title_text("", Some("github")),
        "OK if I call this MCP tool?"
    );
    assert_eq!(
        mcp_blocked_title_text("", None),
        "OK if I call this MCP tool?"
    );
}

#[test]
fn mcp_viewing_detail_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", Some("github")),
        "Viewing MCP tool create_issue on github"
    );
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", None),
        "Viewing MCP tool create_issue"
    );
}

#[test]
fn mcp_viewing_detail_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_viewing_detail_title_text("", Some("github")),
        "Viewing MCP tool call detail"
    );
}

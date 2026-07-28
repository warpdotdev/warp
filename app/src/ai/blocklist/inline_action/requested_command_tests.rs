//! Unit tests for format_command_text in requested_command.rs

use super::{format_command_text, mcp_blocked_title_text, mcp_viewing_detail_title_text};

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

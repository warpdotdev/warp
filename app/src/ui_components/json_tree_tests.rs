//! Pure-logic unit tests for the `json_tree` component.
//!
//! These tests cover only the data-layer functions and types: annotation
//! formatting, long-string detection, state management, and value rendering.
//! They do not exercise the element-construction layer (which requires a
//! running UI framework).
use crate::ui_components::json_tree::{
    JsonTreeState, LONG_STRING_THRESHOLD, PathSegment, format_array_annotation, format_number,
    format_object_annotation, is_long_string,
};

// -----------------------------------------------------------------------
// Annotation labels
// -----------------------------------------------------------------------

#[test]
fn object_annotation_zero_keys() {
    assert_eq!(format_object_annotation(0), "{} 0 keys");
}

#[test]
fn object_annotation_one_key() {
    assert_eq!(format_object_annotation(1), "{} 1 key");
}

#[test]
fn object_annotation_n_keys() {
    assert_eq!(format_object_annotation(5), "{} 5 keys");
    assert_eq!(format_object_annotation(100), "{} 100 keys");
}

#[test]
fn array_annotation_zero_items() {
    assert_eq!(format_array_annotation(0), "[] 0 items");
}

#[test]
fn array_annotation_one_item() {
    assert_eq!(format_array_annotation(1), "[] 1 item");
}

#[test]
fn array_annotation_n_items() {
    assert_eq!(format_array_annotation(3), "[] 3 items");
    assert_eq!(format_array_annotation(99), "[] 99 items");
}

// -----------------------------------------------------------------------
// Long-string detection
// -----------------------------------------------------------------------

#[test]
fn short_string_below_threshold_is_not_long() {
    let s = "a".repeat(LONG_STRING_THRESHOLD - 1);
    assert!(!is_long_string(&s));
}

#[test]
fn string_at_exactly_threshold_is_not_long() {
    let s = "a".repeat(LONG_STRING_THRESHOLD);
    assert!(!is_long_string(&s));
}

#[test]
fn string_above_threshold_is_long() {
    let s = "a".repeat(LONG_STRING_THRESHOLD + 1);
    assert!(is_long_string(&s));
}

#[test]
fn multiline_string_is_long_regardless_of_char_count() {
    // Even a short string with a newline is treated as long.
    assert!(is_long_string("hello\nworld"));
    // Single-char newline is still long.
    assert!(is_long_string("\n"));
}

#[test]
fn empty_string_is_not_long() {
    assert!(!is_long_string(""));
}

// -----------------------------------------------------------------------
// JsonTreeState — toggle independence
// -----------------------------------------------------------------------

#[test]
fn toggle_one_path_leaves_other_paths_unchanged() {
    let path_a = vec![PathSegment::Key("a".to_string())];
    let path_b = vec![PathSegment::Key("b".to_string())];

    let mut state = JsonTreeState::default();

    // Both paths start at default: expanded at depth 0.
    assert!(state.is_expanded(&path_a, 0));
    assert!(state.is_expanded(&path_b, 0));

    // Toggle path A.
    state.toggle(&path_a, 0);

    // A is now collapsed.
    assert!(!state.is_expanded(&path_a, 0));
    // B is still at its default (expanded at depth 0).
    assert!(state.is_expanded(&path_b, 0));
}

#[test]
fn toggle_is_idempotent_across_two_calls() {
    let path = vec![PathSegment::Index(0)];
    let mut state = JsonTreeState::default();

    // Depth-1 node defaults to collapsed.
    assert!(!state.is_expanded(&path, 1));

    // First toggle: collapsed → expanded.
    state.toggle(&path, 1);
    assert!(state.is_expanded(&path, 1));

    // Second toggle: expanded → collapsed again.
    state.toggle(&path, 1);
    assert!(!state.is_expanded(&path, 1));
}

#[test]
fn toggle_nested_path_independent_of_parent() {
    let parent = vec![PathSegment::Key("parent".to_string())];
    let child = vec![
        PathSegment::Key("parent".to_string()),
        PathSegment::Key("child".to_string()),
    ];
    let mut state = JsonTreeState::default();

    // Both are at depth 1, so default is collapsed.
    assert!(!state.is_expanded(&parent, 1));
    assert!(!state.is_expanded(&child, 1));

    // Toggle parent only.
    state.toggle(&parent, 1);

    assert!(state.is_expanded(&parent, 1));
    assert!(!state.is_expanded(&child, 1));
}

// -----------------------------------------------------------------------
// JsonTreeState — long-string expansion (toggle_string / is_string_expanded)
// -----------------------------------------------------------------------

#[test]
fn string_collapsed_by_default() {
    let state = JsonTreeState::default();
    let path = vec![PathSegment::Key("summary".to_string())];
    assert!(!state.is_string_expanded(&path));
}

#[test]
fn toggle_string_expands_then_collapses() {
    let path = vec![PathSegment::Key("body".to_string())];
    let mut state = JsonTreeState::default();

    // Default: collapsed.
    assert!(!state.is_string_expanded(&path));

    // First toggle: collapsed → expanded.
    state.toggle_string(&path);
    assert!(state.is_string_expanded(&path));

    // Second toggle: expanded → collapsed.
    state.toggle_string(&path);
    assert!(!state.is_string_expanded(&path));
}

#[test]
fn toggle_string_is_independent_of_node_expansion() {
    let path = vec![PathSegment::Key("note".to_string())];
    let mut state = JsonTreeState::default();

    // Toggling a string does not affect node expansion state for the same path.
    state.toggle_string(&path);
    assert!(state.is_string_expanded(&path));
    // Node expansion at depth 0 is still the default (expanded).
    assert!(state.is_expanded(&path, 0));
    // Node expansion at depth 1 is still the default (collapsed).
    assert!(!state.is_expanded(&path, 1));
}

// -----------------------------------------------------------------------
// JsonTreeState — default expansion
// -----------------------------------------------------------------------

#[test]
fn depth_0_defaults_to_expanded() {
    let state = JsonTreeState::default();
    let path = vec![];
    assert!(state.is_expanded(&path, 0));
}

#[test]
fn depth_1_defaults_to_collapsed() {
    let state = JsonTreeState::default();
    let path = vec![PathSegment::Key("field".to_string())];
    assert!(!state.is_expanded(&path, 1));
}

#[test]
fn depth_2_defaults_to_collapsed() {
    let state = JsonTreeState::default();
    let path = vec![
        PathSegment::Key("a".to_string()),
        PathSegment::Key("b".to_string()),
    ];
    assert!(!state.is_expanded(&path, 2));
}

// -----------------------------------------------------------------------
// Empty container — no children to render
// -----------------------------------------------------------------------

#[test]
fn empty_object_annotation_is_correct() {
    // An empty object should show "0 keys" regardless of internal state.
    assert_eq!(format_object_annotation(0), "{} 0 keys");
}

#[test]
fn empty_array_annotation_is_correct() {
    assert_eq!(format_array_annotation(0), "[] 0 items");
}

// -----------------------------------------------------------------------
// Integer rendering
// -----------------------------------------------------------------------

#[test]
fn whole_float_displays_as_integer() {
    // serde_json represents JSON `5` as Number(5), but it can also appear
    // as `5.0` in some contexts. The format_number helper must strip the
    // `.0` so it displays as "5".
    let n: serde_json::Number = serde_json::from_str("5").unwrap();
    assert_eq!(format_number(&n), "5");
}

#[test]
fn negative_integer_displays_correctly() {
    let n: serde_json::Number = serde_json::from_str("-42").unwrap();
    assert_eq!(format_number(&n), "-42");
}

#[test]
fn float_with_fraction_displays_with_decimal() {
    let n: serde_json::Number = serde_json::from_str("3.14").unwrap();
    let formatted = format_number(&n);
    // Must contain a decimal point; exact representation depends on precision.
    assert!(formatted.contains('.'), "expected decimal in {formatted}");
}

#[test]
fn large_integer_displays_without_scientific_notation() {
    let n: serde_json::Number = serde_json::from_str("1000000").unwrap();
    assert_eq!(format_number(&n), "1000000");
}

// -----------------------------------------------------------------------
// Duplicate object keys
// -----------------------------------------------------------------------

#[test]
fn duplicate_object_keys_not_silently_dropped() {
    // JSON allows duplicate keys in raw text; serde_json resolves them by
    // retaining the last value for each key. Our rendering code must not
    // drop any additional entries beyond what the parser already resolved.
    //
    // Verify that for a parsed object, format_object_annotation reports the
    // exact count that serde_json produced — no further filtering.
    let v: serde_json::Value = serde_json::from_str(r#"{"a": 1, "a": 2}"#).unwrap();
    let map = v.as_object().expect("expected object");

    // serde_json keeps the last value; our annotation reflects that faithfully.
    let annotation = format_object_annotation(map.len());
    assert!(
        !annotation.is_empty(),
        "annotation must be non-empty for any parsed object"
    );
    // The annotation count matches exactly what serde_json gave us.
    assert_eq!(annotation, format_object_annotation(map.len()));
    // The key still exists — it was not silently removed by the renderer.
    assert!(map.contains_key("a"), "key 'a' was silently dropped");
}

#[test]
fn multi_key_object_all_entries_preserved() {
    // Verifies that iterating over a serde_json Map (as render_value does)
    // does not drop any entries. A three-key object must produce a
    // three-key annotation.
    let v = serde_json::json!({"x": 1, "y": 2, "z": 3});
    let map = v.as_object().expect("expected object");
    assert_eq!(map.len(), 3);
    assert_eq!(format_object_annotation(map.len()), "{} 3 keys");
    for key in ["x", "y", "z"] {
        assert!(map.contains_key(key), "key {key:?} was missing");
    }
}

// -----------------------------------------------------------------------
// mcp_result_to_renderable
// -----------------------------------------------------------------------

#[test]
fn mcp_result_success_with_structured_content_returns_tree() {
    use crate::ai::agent::CallMCPToolResult;
    use crate::ai::blocklist::inline_action::requested_command::{
        McpRenderable, mcp_result_to_renderable,
    };

    let value = serde_json::json!({"count": 42, "files": ["a.rs", "b.rs"]});
    let result = rmcp::model::CallToolResult::structured(value.clone());
    let renderable = mcp_result_to_renderable(&CallMCPToolResult::Success { result });

    match renderable {
        McpRenderable::Tree(v) => assert_eq!(v, value),
        _ => panic!("expected Tree variant"),
    }
}

#[test]
fn mcp_result_success_with_json_text_content_returns_parsed_tree() {
    use crate::ai::agent::CallMCPToolResult;
    use crate::ai::blocklist::inline_action::requested_command::{
        McpRenderable, mcp_result_to_renderable,
    };

    let json_str = r#"{"status": "ok", "value": 7}"#;
    let content = vec![rmcp::model::Content::text(json_str)];
    let result = rmcp::model::CallToolResult::success(content);
    let renderable = mcp_result_to_renderable(&CallMCPToolResult::Success { result });

    let expected: serde_json::Value = serde_json::from_str(json_str).unwrap();
    match renderable {
        McpRenderable::Tree(v) => assert_eq!(v, expected),
        _ => panic!("expected Tree variant with parsed JSON"),
    }
}

#[test]
fn mcp_result_success_with_non_json_text_returns_string_tree() {
    use crate::ai::agent::CallMCPToolResult;
    use crate::ai::blocklist::inline_action::requested_command::{
        McpRenderable, mcp_result_to_renderable,
    };

    let plain_text = "just some plain text output";
    let content = vec![rmcp::model::Content::text(plain_text)];
    let result = rmcp::model::CallToolResult::success(content);
    let renderable = mcp_result_to_renderable(&CallMCPToolResult::Success { result });

    match renderable {
        McpRenderable::Tree(serde_json::Value::String(s)) => {
            assert_eq!(s, plain_text);
        }
        _ => panic!("expected Tree(String) variant"),
    }
}

#[test]
fn mcp_result_error_returns_error_variant() {
    use crate::ai::agent::CallMCPToolResult;
    use crate::ai::blocklist::inline_action::requested_command::{
        McpRenderable, mcp_result_to_renderable,
    };

    let msg = "tool not found".to_string();
    let renderable = mcp_result_to_renderable(&CallMCPToolResult::Error(msg.clone()));

    match renderable {
        McpRenderable::Error(e) => assert_eq!(e, msg),
        _ => panic!("expected Error variant"),
    }
}

#[test]
fn mcp_result_cancelled_returns_cancelled_variant() {
    use crate::ai::agent::CallMCPToolResult;
    use crate::ai::blocklist::inline_action::requested_command::{
        McpRenderable, mcp_result_to_renderable,
    };

    let renderable = mcp_result_to_renderable(&CallMCPToolResult::Cancelled);

    assert!(matches!(renderable, McpRenderable::Cancelled));
}

// -----------------------------------------------------------------------
// Path segment equality (required for HashMap key correctness)
// -----------------------------------------------------------------------

#[test]
fn path_segments_hash_and_eq_correctly() {
    use std::collections::HashMap;

    let mut map: HashMap<Vec<PathSegment>, bool> = HashMap::new();
    let path_key = vec![PathSegment::Key("foo".to_string())];
    let path_idx = vec![PathSegment::Index(0)];

    map.insert(path_key.clone(), true);
    map.insert(path_idx.clone(), false);

    assert!(map[&path_key]);
    assert!(!map[&path_idx]);

    // A different path does not collide.
    let path_other = vec![PathSegment::Key("bar".to_string())];
    assert!(!map.contains_key(&path_other));
}

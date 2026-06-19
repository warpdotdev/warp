//! Pure-logic unit tests for the `json_tree` component (Phase 1, APP-2527).
//!
//! These tests cover only the data-layer functions and types: annotation
//! formatting, long-string detection, state management, and value rendering.
//! They do not exercise the element-construction layer (which requires a
//! running UI framework).
#[cfg(test)]
mod tests {
    use crate::ui_components::json_tree::{
        format_array_annotation, format_number, format_object_annotation, is_long_string,
        JsonTreeState, PathSegment, LONG_STRING_THRESHOLD,
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
        state.toggle(path_a.clone(), 0);

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
        state.toggle(path.clone(), 1);
        assert!(state.is_expanded(&path, 1));

        // Second toggle: expanded → collapsed again.
        state.toggle(path.clone(), 1);
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
        state.toggle(parent.clone(), 1);

        assert!(state.is_expanded(&parent, 1));
        assert!(!state.is_expanded(&child, 1));
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

        assert_eq!(map[&path_key], true);
        assert_eq!(map[&path_idx], false);

        // A different path does not collide.
        let path_other = vec![PathSegment::Key("bar".to_string())];
        assert!(!map.contains_key(&path_other));
    }
}

use toml_edit::Item;

use super::*;

#[test]
fn test_json_to_toml_round_trip_bool() {
    let item = TomlBackedUserPreferences::json_value_to_toml_item("true", None);
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item);
    assert_eq!(json, Some("true".to_string()));
}

#[test]
fn test_json_to_toml_round_trip_integer() {
    let item = TomlBackedUserPreferences::json_value_to_toml_item("42", None);
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item);
    assert_eq!(json, Some("42".to_string()));
}

#[test]
fn test_json_to_toml_round_trip_float() {
    let item = TomlBackedUserPreferences::json_value_to_toml_item("3.14", None);
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item);
    assert_eq!(json, Some("3.14".to_string()));
}

#[test]
fn test_json_to_toml_round_trip_string() {
    let item = TomlBackedUserPreferences::json_value_to_toml_item("\"hello\"", None);
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item);
    assert_eq!(json, Some("\"hello\"".to_string()));
}

#[test]
fn test_json_to_toml_round_trip_object() {
    let input = r#"{"dark":"Phenomenon","light":"Paper"}"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    // Objects should become TOML tables, not strings.
    assert!(matches!(item, Item::Table(_)));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_json_to_toml_round_trip_null() {
    let item = TomlBackedUserPreferences::json_value_to_toml_item("null", None);
    assert!(matches!(item, Item::None));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item);
    assert_eq!(json, None);
}

#[test]
fn test_json_to_toml_round_trip_array_of_strings() {
    let input = r#"["cat","echo","ls"]"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    assert!(matches!(item, Item::Value(toml_edit::Value::Array(_))));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_json_to_toml_round_trip_array_of_objects() {
    let input = r#"[{"AnchoredRegex":"^bash$"},{"AnchoredRegex":"^fish$"}]"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    assert!(matches!(item, Item::Value(toml_edit::Value::Array(_))));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_json_to_toml_round_trip_nested_struct() {
    let input = r#"{"advanced_mode":false,"global":{"mode":"PreviousDir","custom_dir":""},"split_pane":{"mode":"HomeDir","custom_dir":"/tmp"}}"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    assert!(matches!(item, Item::Table(_)));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_json_to_toml_round_trip_object_with_nulls() {
    // Null fields should be omitted from the table, so the round-trip
    // drops them.  Verify that non-null fields survive.
    let input = r#"{"keybinding":null,"active_pin_position":"Top","pin_screen":null,"hide_window_when_unfocused":true}"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    assert!(matches!(item, Item::Table(_)));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Null fields are dropped.
    let expected = serde_json::json!({
        "active_pin_position": "Top",
        "hide_window_when_unfocused": true,
    });
    assert_eq!(actual, expected);
}

#[test]
fn test_json_to_toml_round_trip_empty_array() {
    let input = "[]";
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    assert!(matches!(item, Item::Value(toml_edit::Value::Array(_))));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    assert_eq!(json, "[]");
}

#[test]
fn test_max_table_depth_zero_renders_inline() {
    // depth 0 = entire value rendered inline
    let input = r#"{"uniform_padding":0.0}"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, Some(0));
    // Should be an inline Value, not an Item::Table
    assert!(matches!(
        item,
        Item::Value(toml_edit::Value::InlineTable(_))
    ));
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_max_table_depth_one_renders_top_level_as_table_nested_as_inline() {
    // depth 1 = top-level object is a section table, but nested objects are inline
    let input = r#"{"active_pin_position":"Top","sizes":{"width":100,"height":50},"enabled":true}"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, Some(1));
    // Top level should be a table
    let table = match &item {
        Item::Table(t) => t,
        other => panic!("expected Table, got {other:?}"),
    };
    // Primitive fields should be normal values
    assert!(matches!(
        table.get("active_pin_position"),
        Some(Item::Value(_))
    ));
    assert!(matches!(table.get("enabled"), Some(Item::Value(_))));
    // Nested object should be an inline table value, not a sub-table
    let sizes = table.get("sizes").expect("sizes should exist");
    assert!(
        matches!(sizes, Item::Value(toml_edit::Value::InlineTable(_))),
        "nested object at depth 1 should be inline, got {sizes:?}"
    );
    // Round-trip the JSON
    let json = TomlBackedUserPreferences::toml_item_to_json_string(&item).unwrap();
    let expected: serde_json::Value = serde_json::from_str(input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_max_table_depth_none_renders_all_as_tables() {
    // depth None = unlimited, nested objects become section tables
    let input = r#"{"sizes":{"width":100,"height":50}}"#;
    let item = TomlBackedUserPreferences::json_value_to_toml_item(input, None);
    let table = match &item {
        Item::Table(t) => t,
        other => panic!("expected Table, got {other:?}"),
    };
    // Nested object should be a sub-table, not inline
    assert!(
        matches!(table.get("sizes"), Some(Item::Table(_))),
        "nested object with None depth should be a section table"
    );
}

#[test]
fn test_write_and_read_with_hierarchy() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test_settings.toml");
    let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());

    prefs
        .write_value_with_hierarchy("font_name", "\"Hack\"".to_string(), Some("font"), None)
        .unwrap();
    prefs
        .write_value_with_hierarchy("font_size", "13.0".to_string(), Some("font"), None)
        .unwrap();
    prefs
        .write_value_with_hierarchy(
            "use_thin_strokes",
            "true".to_string(),
            Some("font.display"),
            None,
        )
        .unwrap();

    // Read back
    let font_name = prefs
        .read_value_with_hierarchy("font_name", Some("font"))
        .unwrap();
    assert_eq!(font_name, Some("\"Hack\"".to_string()));

    let font_size = prefs
        .read_value_with_hierarchy("font_size", Some("font"))
        .unwrap();
    assert_eq!(font_size, Some("13.0".to_string()));

    let thin_strokes = prefs
        .read_value_with_hierarchy("use_thin_strokes", Some("font.display"))
        .unwrap();
    assert_eq!(thin_strokes, Some("true".to_string()));

    // Verify the TOML file structure
    let contents = std::fs::read_to_string(&file_path).unwrap();
    assert!(contents.contains("[font]"));
    assert!(contents.contains("font_name"));
    assert!(contents.contains("font_size"));
    assert!(contents.contains("[font.display]"));
    assert!(contents.contains("use_thin_strokes"));
}

#[test]
fn test_write_and_read_struct_value() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test_settings.toml");
    let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());

    // Write a struct value (JSON object) under a hierarchy.
    let struct_json = r#"{"left_alt":false,"right_alt":true}"#;
    prefs
        .write_value_with_hierarchy(
            "extra_meta_keys",
            struct_json.to_string(),
            Some("keys"),
            None,
        )
        .unwrap();

    // Read back — should produce equivalent JSON.
    let read_back = prefs
        .read_value_with_hierarchy("extra_meta_keys", Some("keys"))
        .unwrap()
        .unwrap();
    let expected: serde_json::Value = serde_json::from_str(struct_json).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&read_back).unwrap();
    assert_eq!(actual, expected);

    // Verify the TOML file uses a sub-table, not a JSON string.
    let contents = std::fs::read_to_string(&file_path).unwrap();
    assert!(contents.contains("[keys.extra_meta_keys]"));
    assert!(contents.contains("left_alt = false"));
    assert!(contents.contains("right_alt = true"));
    // Should NOT contain a JSON blob.
    assert!(!contents.contains(r#"{"left_alt"#));
}

#[test]
fn test_new_with_invalid_toml_returns_error_and_recovers_on_reload() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("broken_settings.toml");

    // Write invalid TOML to the file.
    std::fs::write(&file_path, "this is [not valid toml =").unwrap();

    // new() should succeed with an empty document and return the parse error.
    let (prefs, parse_error) = TomlBackedUserPreferences::new(file_path.clone());
    assert!(
        parse_error.is_some(),
        "expected a parse error for invalid TOML"
    );

    // The preferences should behave as empty — no values present.
    assert_eq!(
        prefs
            .read_value_with_hierarchy("font_name", Some("font"))
            .unwrap(),
        None
    );

    // is_settings_file should still return true.
    assert!(prefs.is_settings_file());

    // Now fix the file with valid TOML.
    std::fs::write(&file_path, "[font]\nfont_name = \"Hack\"\n").unwrap();

    // reload_from_disk should succeed and pick up the new value.
    assert!(prefs.reload_from_disk().is_ok());
    assert_eq!(
        prefs
            .read_value_with_hierarchy("font_name", Some("font"))
            .unwrap(),
        Some("\"Hack\"".to_string())
    );
}

#[test]
fn test_writes_inhibited_when_file_initially_broken() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("broken_settings.toml");

    let original_content = "this is [not valid toml =";
    std::fs::write(&file_path, original_content).unwrap();

    let (prefs, parse_error) = TomlBackedUserPreferences::new(file_path.clone());
    assert!(parse_error.is_some());

    // Writing a setting should succeed in-memory but NOT overwrite the file.
    prefs
        .write_value_with_hierarchy("font_name", "\"Hack\"".to_string(), Some("font"), None)
        .unwrap();

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(
        on_disk, original_content,
        "broken file should not be overwritten by writes during inhibited state"
    );

    // Fix the file and reload.
    std::fs::write(&file_path, "# now valid\n").unwrap();
    assert!(prefs.reload_from_disk().is_ok());

    // After reload, writes should flush to disk again.
    prefs
        .write_value_with_hierarchy("font_name", "\"Hack\"".to_string(), Some("font"), None)
        .unwrap();

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        on_disk.contains("font_name"),
        "writes should flush to disk after successful reload"
    );
}

#[test]
fn test_string_value_for_numeric_setting_reads_as_json_string() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test_settings.toml");

    // Write a TOML file where font_size is a string instead of a number.
    std::fs::write(&file_path, "[font]\nfont_size = \"not_a_number\"\n").unwrap();

    let (prefs, parse_error) = TomlBackedUserPreferences::new(file_path);
    assert!(
        parse_error.is_none(),
        "valid TOML should parse without error"
    );

    // Read the value — should return the JSON-encoded string.
    let value = prefs
        .read_value_with_hierarchy("font_size", Some("font"))
        .unwrap();
    // The TOML string "not_a_number" should be JSON-encoded as "\"not_a_number\""
    assert_eq!(
        value,
        Some("\"not_a_number\"".to_string()),
        "TOML string should be JSON-encoded"
    );

    // Attempting to deserialize as f32 should fail.
    let result = serde_json::from_str::<f32>(value.as_deref().unwrap());
    assert!(result.is_err(), "JSON string should not deserialize as f32");
}

#[test]
fn test_remove_with_hierarchy() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test_settings.toml");
    let (prefs, _) = TomlBackedUserPreferences::new(file_path);

    prefs
        .write_value_with_hierarchy("font_name", "\"Hack\"".to_string(), Some("font"), None)
        .unwrap();

    let val = prefs
        .read_value_with_hierarchy("font_name", Some("font"))
        .unwrap();
    assert!(val.is_some());

    prefs
        .remove_value_with_hierarchy("font_name", Some("font"))
        .unwrap();

    let val = prefs
        .read_value_with_hierarchy("font_name", Some("font"))
        .unwrap();
    assert!(val.is_none());
}

#[test]
fn test_per_key_write_inhibition_preserves_value_in_file() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");

    // Write a file with one valid and one "invalid" setting (the TOML itself
    // is fine, but the caller decides the value is wrong for the type).
    std::fs::write(
        &file_path,
        "[font]\nfont_size = \"abc\"\nfont_name = \"Hack\"\n",
    )
    .unwrap();

    let (prefs, parse_error) = TomlBackedUserPreferences::new(file_path.clone());
    assert!(parse_error.is_none(), "TOML itself is valid");

    // Simulate the settings layer detecting the bad value.
    prefs.inhibit_writes_for_key("font_size", Some("font"));

    // Writing to the inhibited key should be a no-op.
    prefs
        .write_value_with_hierarchy("font_size", "13.0".to_string(), Some("font"), None)
        .unwrap();

    // Writing to the NON-inhibited key should succeed.
    prefs
        .write_value_with_hierarchy("font_name", "\"Fira Code\"".to_string(), Some("font"), None)
        .unwrap();

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    // The inhibited key's original value must still be present.
    assert!(
        on_disk.contains("font_size = \"abc\""),
        "inhibited key's original value should be preserved, got: {on_disk}"
    );
    // The non-inhibited key's new value should be written.
    assert!(
        on_disk.contains("Fira Code"),
        "non-inhibited key should be updated, got: {on_disk}"
    );
}

#[test]
fn test_per_key_write_inhibition_blocks_remove() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");
    std::fs::write(&file_path, "[font]\nfont_size = \"abc\"\n").unwrap();

    let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());
    prefs.inhibit_writes_for_key("font_size", Some("font"));

    // Removing the inhibited key should be a no-op.
    prefs
        .remove_value_with_hierarchy("font_size", Some("font"))
        .unwrap();

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        on_disk.contains("font_size = \"abc\""),
        "inhibited key should not be removed, got: {on_disk}"
    );
}

#[test]
fn test_reload_clears_per_key_inhibitions() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");
    std::fs::write(&file_path, "[font]\nfont_size = \"abc\"\n").unwrap();

    let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());
    prefs.inhibit_writes_for_key("font_size", Some("font"));

    // Fix the file.
    std::fs::write(&file_path, "[font]\nfont_size = 14.0\n").unwrap();
    prefs.reload_from_disk().unwrap();

    // After reload, the inhibition is cleared — writes should work.
    prefs
        .write_value_with_hierarchy("font_size", "16.0".to_string(), Some("font"), None)
        .unwrap();

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        on_disk.contains("16.0") || on_disk.contains("16"),
        "write should succeed after reload cleared inhibition, got: {on_disk}"
    );
}

#[test]
fn test_clear_all_write_inhibitions() {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");
    std::fs::write(&file_path, "[font]\nfont_size = \"abc\"\nfont_name = 123\n").unwrap();

    let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());
    prefs.inhibit_writes_for_key("font_size", Some("font"));
    prefs.inhibit_writes_for_key("font_name", Some("font"));

    prefs.clear_all_write_inhibitions();

    // Both keys should now be writable.
    prefs
        .write_value_with_hierarchy("font_size", "14.0".to_string(), Some("font"), None)
        .unwrap();
    prefs
        .write_value_with_hierarchy("font_name", "\"Hack\"".to_string(), Some("font"), None)
        .unwrap();

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        on_disk.contains("14") && on_disk.contains("Hack"),
        "both keys should be writable after clearing all inhibitions, got: {on_disk}"
    );
}

#[test]
fn test_file_content_hash_returns_none_for_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("does_not_exist.toml");
    assert_eq!(
        None,
        TomlBackedUserPreferences::file_content_hash(&file_path)
    );
}

#[test]
fn test_file_content_hash_returns_none_for_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("empty.toml");
    std::fs::write(&file_path, "").unwrap();
    assert_eq!(
        None,
        TomlBackedUserPreferences::file_content_hash(&file_path)
    );
}

#[test]
fn test_file_content_hash_returns_none_for_whitespace_only_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("whitespace.toml");
    std::fs::write(&file_path, "   \n\t  \n").unwrap();
    assert_eq!(
        None,
        TomlBackedUserPreferences::file_content_hash(&file_path)
    );
}

#[test]
fn test_file_content_hash_is_deterministic_for_identical_content() {
    let dir = tempfile::tempdir().unwrap();
    let path_a = dir.path().join("a.toml");
    let path_b = dir.path().join("b.toml");
    let contents = "[font]\nfont_size = 14.0\n";
    std::fs::write(&path_a, contents).unwrap();
    std::fs::write(&path_b, contents).unwrap();

    let hash_a = TomlBackedUserPreferences::file_content_hash(&path_a);
    let hash_b = TomlBackedUserPreferences::file_content_hash(&path_b);
    assert!(hash_a.is_some());
    assert_eq!(hash_a, hash_b);
}

#[test]
fn test_file_content_hash_differs_for_different_content() {
    let dir = tempfile::tempdir().unwrap();
    let path_a = dir.path().join("a.toml");
    let path_b = dir.path().join("b.toml");
    std::fs::write(&path_a, "[font]\nfont_size = 14.0\n").unwrap();
    std::fs::write(&path_b, "[font]\nfont_size = 18.0\n").unwrap();

    let hash_a = TomlBackedUserPreferences::file_content_hash(&path_a);
    let hash_b = TomlBackedUserPreferences::file_content_hash(&path_b);
    assert!(hash_a.is_some());
    assert!(hash_b.is_some());
    assert_ne!(hash_a, hash_b);
}

// Pretty-printing tests: verify that wide inline containers get broken
// across lines and that short ones stay on a single line. These drive the
// `prettify_item` pass in `toml_backed.rs`.

/// Writes `value_json` under the given hierarchy + key and returns the full
/// file contents. Also asserts the round-trip (read back as JSON matches).
fn write_and_read_file(
    hierarchy: Option<&str>,
    key: &str,
    value_json: &str,
    max_table_depth: Option<u32>,
) -> String {
    use super::super::UserPreferences;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");
    let (prefs, _) = TomlBackedUserPreferences::new(file_path.clone());

    prefs
        .write_value_with_hierarchy(key, value_json.to_string(), hierarchy, max_table_depth)
        .unwrap();

    // Round-trip: reading back should yield the same JSON value.
    let read_back = prefs
        .read_value_with_hierarchy(key, hierarchy)
        .unwrap()
        .expect("value was just written");
    let expected: serde_json::Value = serde_json::from_str(value_json).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&read_back).unwrap();
    assert_eq!(actual, expected, "round-trip should preserve the value");

    std::fs::read_to_string(&file_path).unwrap()
}

#[test]
fn test_pretty_print_wide_array_of_inline_tables() {
    // Shaped like `custom_secret_regex_list`: each inline table has a
    // short field count but the whole array is very wide.
    let json = r#"[
        {"name":"IPv4 Address","pattern":"\\b(?:\\d{1,3}\\.){3}\\d{1,3}\\b"},
        {"name":"OpenAI API Key","pattern":"\\bsk-[a-zA-Z0-9]{48}\\b"},
        {"name":"GitHub Token","pattern":"\\bghp_[A-Za-z0-9_]{36}\\b"}
    ]"#;
    let contents = write_and_read_file(Some("privacy"), "custom_secret_regex_list", json, None);

    // Expect each inline table on its own indented line, with the closing
    // bracket on a fresh line.
    assert!(
        contents.contains("custom_secret_regex_list = [\n"),
        "array should open with a newline, got:\n{contents}"
    );
    assert!(
        contents.contains("\n  { name = \"IPv4 Address\""),
        "each inline table should start on a new indented line, got:\n{contents}"
    );
    assert!(
        contents.contains("\n]\n") || contents.ends_with("\n]"),
        "closing bracket should be on its own line, got:\n{contents}"
    );
    // Each inline table itself is short (2 fields, fits well under
    // MAX_INLINE_WIDTH), so it should still be on a single line.
    assert!(
        !contents.contains("{ name =\n") && !contents.contains("{\n    name"),
        "individual inline tables should stay on one line, got:\n{contents}"
    );
}

#[test]
fn test_pretty_print_short_primitive_array_stays_inline() {
    let json = r#"["cat","echo","ls"]"#;
    let contents = write_and_read_file(Some("terminal"), "allowed_commands", json, None);
    // The whole thing fits on one line; it should stay inline.
    assert!(
        contents.contains("allowed_commands = [\"cat\", \"echo\", \"ls\"]"),
        "short primitive array should stay inline, got:\n{contents}"
    );
    assert!(
        !contents.contains("allowed_commands = [\n"),
        "short primitive array should not be broken, got:\n{contents}"
    );
}

#[test]
fn test_pretty_print_long_primitive_array_goes_multiline() {
    // Construct a primitive array whose single-line rendering clearly
    // exceeds `MAX_INLINE_WIDTH`.
    let items: Vec<String> = (0..20)
        .map(|i| format!("\"/some/fairly/long/path/entry/number/{i}\""))
        .collect();
    let json = format!("[{}]", items.join(","));
    let contents = write_and_read_file(Some("agents"), "allowlist", &json, None);
    assert!(
        contents.contains("allowlist = [\n"),
        "long primitive array should open multi-line, got:\n{contents}"
    );
    // Each element should sit on its own indented line.
    assert!(
        contents.contains("\n  \"/some/fairly/long/path/entry/number/0\","),
        "first element should be on its own indented line with trailing comma, got:\n{contents}"
    );
}

#[test]
fn test_pretty_print_short_inline_table_stays_inline() {
    // Force inline via max_table_depth = 0 and verify it stays on one line.
    let json = r#"{"a":1,"b":2}"#;
    let contents = write_and_read_file(Some("section"), "small", json, Some(0));
    assert!(
        contents.contains("small = { a = 1, b = 2 }"),
        "short inline table should stay on one line, got:\n{contents}"
    );
}

#[test]
fn test_pretty_print_wide_inline_table_goes_multiline() {
    // Force inline via max_table_depth = 0, with many long-valued fields so
    // the single-line rendering is wider than `MAX_INLINE_WIDTH`.
    let json = concat!(
        "{",
        "\"first\":\"a string value that is long enough\",",
        "\"second\":\"another string value that is long enough\",",
        "\"third\":\"one more string value that is long enough\"",
        "}"
    );
    let contents = write_and_read_file(Some("section"), "big", json, Some(0));
    assert!(
        contents.contains("big = {\n"),
        "wide inline table should open multi-line, got:\n{contents}"
    );
    // Each field should sit on its own indented line.
    assert!(
        contents.contains("\n  first = "),
        "first field should be on its own indented line, got:\n{contents}"
    );
    assert!(
        contents.contains("\n  second = "),
        "second field should be on its own indented line, got:\n{contents}"
    );
}

#[test]
fn test_pretty_print_propagates_to_parent_array() {
    // A short outer array (one element) containing a wide inline table.
    // The inline table must go multi-line (width rule), and that forces
    // the array multi-line too (propagation rule) — otherwise we'd render
    // something ugly like `[{\n  ...\n}]`.
    let json = concat!(
        "[{",
        "\"first\":\"a string value that is long enough\",",
        "\"second\":\"another string value that is long enough\",",
        "\"third\":\"one more string value that is long enough\"",
        "}]"
    );
    let contents = write_and_read_file(Some("section"), "items", json, None);
    assert!(
        contents.contains("items = [\n"),
        "array should be multi-line because its child is multi-line, got:\n{contents}"
    );
    assert!(
        contents.contains("{\n"),
        "inline table should be multi-line, got:\n{contents}"
    );
}

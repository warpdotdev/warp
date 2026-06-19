use serde_json::json;
use settings_value::SettingsValue;

use super::*;

#[test]
fn from_file_value_non_array_returns_none() {
    // Matches the blanket `Vec<T>: SettingsValue` impl: anything that isn't
    // a JSON array fails the load. The macro-generated load_fn surfaces the
    // storage key in `SettingsFileError::InvalidSettings`.
    let object = json!({ "not": "an array" });
    assert!(LspServerDescriptors::from_file_value(&object).is_none());

    let null = json!(null);
    assert!(LspServerDescriptors::from_file_value(&null).is_none());
}

#[test]
fn from_file_value_empty_array_returns_empty() {
    let value = json!([]);
    let parsed = LspServerDescriptors::from_file_value(&value).unwrap();
    assert!(parsed.0.is_empty());
}

#[test]
fn from_file_value_valid_entry_has_working_matcher() {
    // The whole reason for the hand-rolled from_file_value: route through
    // parse_entries so the descriptor's matcher is the *real* compiled glob,
    // not the never-matches placeholder that #[serde(skip)] would produce.
    let value = json!([{
        "name": "ruby-lsp",
        "command": "ruby-lsp",
        "filetypes": [{ "pattern": "*.rb" }],
    }]);
    let parsed = LspServerDescriptors::from_file_value(&value).unwrap();
    assert_eq!(parsed.0.len(), 1);
    let descriptor = &parsed.0[0];
    assert_eq!(descriptor.name, "ruby-lsp");
    assert_eq!(descriptor.filetypes.len(), 1);
    assert!(descriptor.filetypes[0].is_match("foo.rb"));
    assert!(!descriptor.filetypes[0].is_match("foo.py"));
}

#[test]
fn from_file_value_any_invalid_entry_fails_whole_setting() {
    // All-or-nothing, matching the blanket `Vec<T>: SettingsValue` impl in
    // `crates/settings_value/src/lib.rs`. One invalid entry alongside one
    // valid entry still fails the load — `from_file_value` returns `None`,
    // the macro surfaces `editor.language_servers` in the banner, and the
    // per-entry reasons are written to the log.
    let value = json!([
        { "command": "missing-name", "filetypes": [{ "pattern": "*.x" }] },
        { "name": "good", "command": "good", "filetypes": [{ "pattern": "*.good" }] },
    ]);
    assert!(LspServerDescriptors::from_file_value(&value).is_none());
}

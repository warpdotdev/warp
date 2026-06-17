use settings::Setting as _;

use super::FormatOnSave;

#[test]
fn format_on_save_defaults_to_enabled() {
    assert!(FormatOnSave::default_value());
}

#[test]
fn format_on_save_uses_code_editor_toml_path() {
    assert_eq!(
        FormatOnSave::toml_path(),
        Some("code.editor.format_on_save")
    );
}

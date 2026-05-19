use warpui::keymap::macros::*;

use super::{resolve_param_value, TabConfigParamsModal, DROPDOWN_ONLY_FLAG};
use crate::tab_configs::{TabConfigParam, TabConfigParamType};

#[test]
fn resolve_param_value_returns_default_for_blank_input() {
    let param = TabConfigParam {
        description: None,
        default: Some("main".to_string()),
        param_type: TabConfigParamType::Text,
    };

    assert_eq!(
        resolve_param_value("   ".to_string(), &param),
        Some("main".to_string())
    );
}

#[test]
fn resolve_param_value_returns_none_for_blank_required_input() {
    let param = TabConfigParam {
        description: None,
        default: None,
        param_type: TabConfigParamType::Text,
    };

    assert_eq!(resolve_param_value("".to_string(), &param), None);
}

#[test]
fn resolve_param_value_preserves_non_blank_input() {
    let param = TabConfigParam {
        description: None,
        default: Some("main".to_string()),
        param_type: TabConfigParamType::Text,
    };

    assert_eq!(
        resolve_param_value("feature-branch".to_string(), &param),
        Some("feature-branch".to_string())
    );
}

#[test]
fn keymap_context_omits_dropdown_only_flag_when_text_field_present() {
    // When a Text param exists, the modal must NOT advertise the
    // dropdown-only flag — otherwise Space typed in the focused editor
    // would be swallowed by the modal-level ToggleDropdown binding
    // (issue #11138).
    let context = TabConfigParamsModal::build_keymap_context(true);
    assert!(
        !context.set.contains(DROPDOWN_ONLY_FLAG),
        "modal with text fields must not expose {DROPDOWN_ONLY_FLAG}; got set = {:?}",
        context.set,
    );
}

#[test]
fn keymap_context_includes_dropdown_only_flag_when_no_text_fields() {
    // Dropdown-only modals rely on the flag so that the Space fixed
    // binding still toggles the lone dropdown (preserving the
    // keyboard shortcut documented in TabConfigParamsModal::on_open).
    let context = TabConfigParamsModal::build_keymap_context(false);
    assert!(
        context.set.contains(DROPDOWN_ONLY_FLAG),
        "dropdown-only modal must expose {DROPDOWN_ONLY_FLAG}; got set = {:?}",
        context.set,
    );
}

#[test]
fn space_binding_predicate_blocks_when_text_field_present() {
    // This is the load-bearing test for issue #11138: when the modal's
    // context omits the dropdown-only flag (i.e. a Text param exists),
    // the Space binding's predicate must NOT match — so Space falls
    // through to the focused editor as a literal character.
    let predicate = id!(DROPDOWN_ONLY_FLAG);
    let mixed_mode_ctx = TabConfigParamsModal::build_keymap_context(true);
    assert!(
        !predicate.eval(&mixed_mode_ctx),
        "space binding must not fire in mixed-mode (text + dropdown) modal",
    );
}

#[test]
fn space_binding_predicate_fires_in_dropdown_only_modal() {
    // Complement: in dropdown-only configs (no Text param) the binding
    // still matches so Space toggles the dropdown.
    let predicate = id!(DROPDOWN_ONLY_FLAG);
    let dropdown_only_ctx = TabConfigParamsModal::build_keymap_context(false);
    assert!(
        predicate.eval(&dropdown_only_ctx),
        "space binding must fire in dropdown-only modal",
    );
}

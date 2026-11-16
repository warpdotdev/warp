use super::resolve_param_value;
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

use serde_json::json;

use super::super::protocol::AcpAdvertisedSetting;
use super::find_supported_setting_id;

#[test]
fn selection_uses_categories_and_grouped_values_instead_of_adapter_ids() {
    let settings: Vec<AcpAdvertisedSetting> = serde_json::from_value(json!([
        {
            "id": "adapter-specific-model",
            "category": "model",
            "type": "select",
            "options": [{
                "group": "OpenAI",
                "options": [{"value": "openai/custom/model", "name": "Custom model"}]
            }]
        },
        {
            "id": "reasoning_effort",
            "category": "thought_level",
            "type": "select",
            "options": [{"value": "high", "name": "High"}]
        }
    ]))
    .unwrap();

    assert_eq!(
        find_supported_setting_id(&settings, "model", "openai/custom/model").unwrap(),
        "adapter-specific-model"
    );
    assert_eq!(
        find_supported_setting_id(&settings, "thought_level", "high").unwrap(),
        "reasoning_effort"
    );
}

#[test]
fn unavailable_values_are_rejected() {
    let settings: Vec<AcpAdvertisedSetting> = serde_json::from_value(json!([{
        "id": "thinking",
        "category": "thought_level",
        "type": "select",
        "options": [{"value": "off", "name": "Off"}]
    }]))
    .unwrap();

    assert!(find_supported_setting_id(&settings, "thought_level", "high").is_err());
    assert!(find_supported_setting_id(&[], "model", "openai/gpt-model").is_err());
}

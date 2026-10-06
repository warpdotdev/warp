use super::*;

#[test]
fn legacy_profile_without_web_fetch_inherits_web_search() {
    for web_search_enabled in [false, true] {
        let json = serde_json::json!({
            "name": "Legacy",
            "web_search_enabled": web_search_enabled,
        });

        let profile: AIExecutionProfile = serde_json::from_value(json).unwrap();

        assert_eq!(profile.web_search_enabled, web_search_enabled);
        assert_eq!(profile.web_fetch_enabled, web_search_enabled);
    }
}

#[test]
fn explicit_web_tool_settings_round_trip_independently() {
    for (web_search_enabled, web_fetch_enabled) in [(true, false), (false, true)] {
        let profile = AIExecutionProfile {
            name: "Independent".to_string(),
            web_search_enabled,
            web_fetch_enabled,
            ..Default::default()
        };

        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["web_fetch_enabled"], web_fetch_enabled);

        let decoded: AIExecutionProfile = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, profile);
    }
}

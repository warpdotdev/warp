use ai::LLMId;

use super::AIExecutionProfile;

#[test]
fn legacy_profile_without_orchestration_model_deserializes_to_none() {
    let profile: AIExecutionProfile =
        serde_json::from_str(r#"{"name":"Legacy"}"#).expect("legacy profile should deserialize");

    assert_eq!(profile.name, "Legacy");
    assert_eq!(profile.orchestration_model, None);
}

#[test]
fn orchestration_model_round_trips() {
    let profile = AIExecutionProfile {
        orchestration_model: Some(LLMId::from("custom-router:cloud:team-router")),
        ..Default::default()
    };

    let serialized = serde_json::to_string(&profile).expect("profile should serialize");
    let deserialized: AIExecutionProfile =
        serde_json::from_str(&serialized).expect("profile should deserialize");

    assert_eq!(
        deserialized.orchestration_model,
        Some(LLMId::from("custom-router:cloud:team-router"))
    );
}

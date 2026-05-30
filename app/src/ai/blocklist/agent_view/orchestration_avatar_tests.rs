use super::OrchestrationAvatar;

#[test]
fn agent_avatar_identity_is_display_name_based_for_pill_consistency() {
    assert_eq!(
        OrchestrationAvatar::agent("Agent 1".to_string()),
        OrchestrationAvatar::Agent {
            display_name: "Agent 1".to_string(),
        }
    );
}

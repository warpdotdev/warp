use warpui::EntityId;

use super::{
    execution_profile_event_affects_long_context_warning,
    llm_preferences_event_affects_long_context_warning,
};
use crate::ai::execution_profiles::profiles::{AIExecutionProfilesModelEvent, ClientProfileId};
use crate::ai::llms::LLMPreferencesEvent;

#[test]
fn model_catalog_and_active_model_changes_recompute_long_context_warning() {
    assert!(llm_preferences_event_affects_long_context_warning(
        &LLMPreferencesEvent::UpdatedAvailableLLMs
    ));
    assert!(llm_preferences_event_affects_long_context_warning(
        &LLMPreferencesEvent::UpdatedActiveAgentModeLLM
    ));
    assert!(!llm_preferences_event_affects_long_context_warning(
        &LLMPreferencesEvent::UpdatedActiveCodingLLM
    ));
}

#[test]
fn only_relevant_profile_changes_recompute_long_context_warning() {
    let terminal_view_id = EntityId::new();
    let other_terminal_view_id = EntityId::new();
    let active_profile_id = ClientProfileId::new();
    let inactive_profile_id = ClientProfileId::new();

    assert!(execution_profile_event_affects_long_context_warning(
        &AIExecutionProfilesModelEvent::ProfileUpdated(active_profile_id),
        active_profile_id,
        terminal_view_id,
    ));
    assert!(!execution_profile_event_affects_long_context_warning(
        &AIExecutionProfilesModelEvent::ProfileUpdated(inactive_profile_id),
        active_profile_id,
        terminal_view_id,
    ));
    assert!(execution_profile_event_affects_long_context_warning(
        &AIExecutionProfilesModelEvent::UpdatedActiveProfile { terminal_view_id },
        active_profile_id,
        terminal_view_id,
    ));
    assert!(!execution_profile_event_affects_long_context_warning(
        &AIExecutionProfilesModelEvent::UpdatedActiveProfile {
            terminal_view_id: other_terminal_view_id,
        },
        active_profile_id,
        terminal_view_id,
    ));
    assert!(execution_profile_event_affects_long_context_warning(
        &AIExecutionProfilesModelEvent::ProfileDeleted,
        active_profile_id,
        terminal_view_id,
    ));
    assert!(!execution_profile_event_affects_long_context_warning(
        &AIExecutionProfilesModelEvent::ProfileCreated,
        active_profile_id,
        terminal_view_id,
    ));
}

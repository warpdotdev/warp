use std::collections::HashMap;

// User default keys
const IS_ACTIVE_AI_ENABLED: &str = "IsActiveAIEnabled";
const INTELLIGENT_AUTOSUGGESTIONS_ENABLED: &str = "IntelligentAutosuggestionsEnabled";
const NATURAL_LANGUAGE_AUTOSUGGESTIONS_ENABLED: &str = "NaturalLanguageAutosuggestionsEnabled";
const AGENT_MODE_QUERY_SUGGESTIONS_ENABLED: &str = "AgentModeQuerySuggestionsEnabled";
const CODE_SUGGESTIONS_ENABLED: &str = "CodeSuggestionsEnabled";

pub fn user_defaults_map_with_active_ai(enabled: bool) -> HashMap<String, String> {
    HashMap::from_iter([
        (
            INTELLIGENT_AUTOSUGGESTIONS_ENABLED.to_owned(),
            enabled.to_string(),
        ),
        (
            AGENT_MODE_QUERY_SUGGESTIONS_ENABLED.to_owned(),
            enabled.to_string(),
        ),
        (CODE_SUGGESTIONS_ENABLED.to_owned(), enabled.to_string()),
        (
            NATURAL_LANGUAGE_AUTOSUGGESTIONS_ENABLED.to_owned(),
            enabled.to_string(),
        ),
        (IS_ACTIVE_AI_ENABLED.to_owned(), enabled.to_string()),
    ])
}

/// User defaults for predictable AI input behavior needed in evals.
///
/// This allows tests to more reliably enter and exit AI input mode.
///
/// * UDI is enabled
/// * Natural language detection is disabled
pub fn user_defaults_map_for_ai_input() -> HashMap<String, String> {
    HashMap::from_iter([
        (
            "AIAutoDetectionEnabled".to_owned(),
            serde_json::to_string(&false).unwrap(),
        ),
        (
            "InputBoxTypeSetting".to_owned(),
            serde_json::to_string("Universal").unwrap(),
        ),
    ])
}

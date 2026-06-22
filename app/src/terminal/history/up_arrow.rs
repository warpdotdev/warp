use std::collections::HashSet;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, EntityId, SingletonEntity};

use super::History;
use crate::ai::blocklist::{BlocklistAIHistoryModel, InputConfig};
use crate::input_suggestions::HistoryInputSuggestion;
use crate::settings::AISettings;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::model::session::SessionId;

/// Controls which item types are included in up-arrow history results.
#[derive(Copy, Clone, Debug)]
pub(crate) struct UpArrowHistoryConfig {
    pub include_commands: bool,
    pub include_prompts: bool,
}

impl UpArrowHistoryConfig {
    /// Derives the config from the current input config.
    /// When the input is locked to a specific type, only that type is included.
    /// When unlocked (auto-detection), both types are included.
    pub fn for_input_config(input_config: &InputConfig) -> Self {
        if input_config.is_locked {
            Self {
                include_commands: input_config.is_shell(),
                include_prompts: input_config.is_ai(),
            }
        } else {
            Self {
                include_commands: true,
                include_prompts: true,
            }
        }
    }
}

fn sort_and_dedupe_suggestions<'a>(
    mut suggestions: Vec<HistoryInputSuggestion<'a>>,
    session_id: Option<SessionId>,
    all_live_session_ids: &HashSet<SessionId>,
) -> Vec<HistoryInputSuggestion<'a>> {
    suggestions.sort_by(|a, b| a.cmp(b, session_id, all_live_session_ids));

    // Deduplicate commands and AI queries separately: keep the latest occurrence for each type.
    let mut seen_commands: HashSet<&str> = HashSet::new();
    let mut seen_ai_queries: HashSet<&str> = HashSet::new();
    let mut skip_indices: HashSet<usize> = HashSet::new();
    for (idx, suggestion) in suggestions.iter().enumerate().rev() {
        let text = suggestion.text();
        if suggestion.is_ai_query() {
            if seen_ai_queries.contains(text) {
                skip_indices.insert(idx);
            } else {
                seen_ai_queries.insert(text);
            }
        } else if seen_commands.contains(text) {
            skip_indices.insert(idx);
        } else {
            seen_commands.insert(text);
        }
    }

    suggestions
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !skip_indices.contains(idx))
        .map(|(_, suggestion)| suggestion)
        .collect()
}

impl History {
    pub(crate) fn up_arrow_suggestions_for_terminal_view<'a>(
        &'a self,
        terminal_view_id: EntityId,
        session_id: Option<SessionId>,
        config: UpArrowHistoryConfig,
        app: &'a AppContext,
    ) -> Vec<HistoryInputSuggestion<'a>> {
        let ignored_suggestions = IgnoredSuggestionsModel::handle(app).as_ref(app);

        let include_agent_commands = *AISettings::handle(app)
            .as_ref(app)
            .include_agent_commands_in_history;

        let commands = session_id
            .and_then(|session_id| self.commands(session_id))
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| {
                !ignored_suggestions.is_ignored(&entry.command, SuggestionType::ShellCommand)
            })
            .filter(move |entry| include_agent_commands || !entry.is_agent_executed)
            .map(|entry| HistoryInputSuggestion::Command { entry });

        let should_include_prompts = config.include_prompts
            && FeatureFlag::AgentMode.is_enabled()
            && AISettings::handle(app).as_ref(app).is_any_ai_enabled(app);
        let all_live_session_ids = self.all_live_session_ids();
        if !should_include_prompts {
            if !config.include_commands {
                return vec![];
            }
            return sort_and_dedupe_suggestions(
                commands.collect(),
                session_id,
                &all_live_session_ids,
            );
        }

        let ai_queries = BlocklistAIHistoryModel::handle(app)
            .as_ref(app)
            .all_ai_queries(Some(terminal_view_id))
            .filter(|query| {
                !ignored_suggestions.is_ignored(&query.query_text, SuggestionType::AIQuery)
            })
            .map(|entry| HistoryInputSuggestion::AIQuery { entry });

        let suggestions: Vec<HistoryInputSuggestion<'a>> =
            match (config.include_commands, config.include_prompts) {
                (true, true) => commands.chain(ai_queries).collect(),
                (true, false) => commands.collect(),
                (false, true) => ai_queries.collect(),
                (false, false) => vec![],
            };

        sort_and_dedupe_suggestions(suggestions, session_id, &all_live_session_ids)
    }
}

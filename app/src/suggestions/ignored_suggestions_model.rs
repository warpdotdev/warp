use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::GlobalResourceHandlesProvider;
use crate::persistence::ModelEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum SuggestionType {
    ShellCommand,
    AIQuery,
}

impl SuggestionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SuggestionType::ShellCommand => "shell_command",
            SuggestionType::AIQuery => "ai_query",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "shell_command" => Some(SuggestionType::ShellCommand),
            "ai_query" => Some(SuggestionType::AIQuery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IgnoredSuggestionKey {
    pub suggestion: String,
    pub suggestion_type: SuggestionType,
}

pub struct IgnoredSuggestionsModel {
    ignored_suggestions: HashSet<IgnoredSuggestionKey>,
}

impl IgnoredSuggestionsModel {
    pub fn new(persisted_ignored_suggestions: Vec<(String, SuggestionType)>) -> Self {
        let ignored_suggestions = persisted_ignored_suggestions
            .into_iter()
            .map(|(suggestion, suggestion_type)| IgnoredSuggestionKey {
                suggestion,
                suggestion_type,
            })
            .collect();

        Self {
            ignored_suggestions,
        }
    }

    pub fn add_ignored_suggestion(
        &mut self,
        suggestion: String,
        suggestion_type: SuggestionType,
        ctx: &mut ModelContext<Self>,
    ) {
        let key = IgnoredSuggestionKey {
            suggestion: suggestion.clone(),
            suggestion_type: suggestion_type.clone(),
        };

        if self.ignored_suggestions.contains(&key) {
            return;
        }

        self.ignored_suggestions.insert(key);

        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get();

        if let Some(sender) = &global_resource_handles.model_event_sender {
            let event = ModelEvent::AddIgnoredSuggestion {
                suggestion,
                suggestion_type,
            };
            if let Err(err) = sender.send(event) {
                report_error!(
                    anyhow::Error::new(err)
                        .context("Failed to save ignored suggestion to database")
                );
            }
        }

        ctx.emit(IgnoredSuggestionsModelEvent::SuggestionIgnored);
    }

    pub fn remove_ignored_suggestion(
        &mut self,
        suggestion: String,
        suggestion_type: SuggestionType,
        ctx: &mut ModelContext<Self>,
    ) {
        let key = IgnoredSuggestionKey {
            suggestion: suggestion.clone(),
            suggestion_type: suggestion_type.clone(),
        };

        if !self.ignored_suggestions.contains(&key) {
            return;
        }

        self.ignored_suggestions.remove(&key);

        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get();

        if let Some(sender) = &global_resource_handles.model_event_sender {
            let event = ModelEvent::RemoveIgnoredSuggestion {
                suggestion,
                suggestion_type,
            };
            if let Err(err) = sender.send(event) {
                report_error!(
                    anyhow::Error::new(err)
                        .context("Failed to remove ignored suggestion from database")
                );
            }
        }
    }

    pub fn is_ignored(&self, suggestion: &str, suggestion_type: SuggestionType) -> bool {
        let key = IgnoredSuggestionKey {
            suggestion: suggestion.to_string(),
            suggestion_type,
        };
        self.ignored_suggestions.contains(&key)
    }

    /// Returns a set of all ignored suggestions for a specific type
    pub fn get_ignored_suggestions_for_type(
        &self,
        suggestion_type: SuggestionType,
    ) -> HashSet<String> {
        self.ignored_suggestions
            .iter()
            .filter(|key| key.suggestion_type == suggestion_type)
            .map(|key| key.suggestion.clone())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub enum IgnoredSuggestionsModelEvent {
    SuggestionIgnored,
}

impl Entity for IgnoredSuggestionsModel {
    type Event = IgnoredSuggestionsModelEvent;
}

impl SingletonEntity for IgnoredSuggestionsModel {}

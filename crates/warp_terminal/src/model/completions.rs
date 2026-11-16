use warp_completer::completer::{Match, MatchedSuggestion, Suggestion, SuggestionType};

/// A completion result that was produced natively by the shell.
#[derive(Clone, Debug)]
pub struct ShellCompletion {
    name: String,
    description: Option<String>,
    suggestion_type: SuggestionType,
}

/// Enum indicating which field of a [`ShellCompletion`] should be updated.
pub enum ShellCompletionUpdate {
    Description { value: String },
}

impl ShellCompletion {
    pub fn new(name: String) -> Self {
        Self {
            name: name.trim().to_string(),
            description: None,
            suggestion_type: SuggestionType::Argument,
        }
    }

    pub fn update(&mut self, completion_update: ShellCompletionUpdate) {
        match completion_update {
            ShellCompletionUpdate::Description { value } => {
                if !value.is_empty() {
                    self.description = Some(value.trim().to_string());
                }
            }
        }
    }
}

impl From<ShellCompletion> for MatchedSuggestion {
    fn from(value: ShellCompletion) -> Self {
        let suggestion = Suggestion::with_same_display_and_replacement(
            value.name,
            value.description,
            value.suggestion_type,
            Default::default(),
        );
        MatchedSuggestion::new(
            suggestion,
            Match::Prefix {
                is_case_sensitive: false,
            },
        )
    }
}

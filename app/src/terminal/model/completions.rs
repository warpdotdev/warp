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

    pub(super) fn update(&mut self, completion_update: ShellCompletionUpdate) {
        match completion_update {
            ShellCompletionUpdate::Description { value } => {
                if !value.is_empty() {
                    self.description = Some(value.trim().to_string());
                }
            }
        }
    }

    /// Sorts completions by name. Lives here (rather than at the call site) because `name` is
    /// private to this module.
    pub fn sort_by_name(completions: &mut [ShellCompletion]) {
        // TODO: we need to get metadata from the shell about how the results
        // should be sorted (intra- and inter-groups).
        completions.sort_by(|a, b| a.name.cmp(&b.name));
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

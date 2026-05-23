use black_completer::completer::{Match, MatchedSuggestion, Suggestion, SuggestionType};

/// The completions data coming from the shell.
///
/// For now, we only support sourcing shell completion data for zsh,
/// but eventually parsing will require knowing which shell the data came from.
#[derive(Clone, Debug)]
pub enum ShellData {
    /// The completions will be emitted as raw shell completion data, where
    /// each individual result is delimited by whitespace.
    /// For now, we assume that the results are sorted.
    Raw { output: String },
    /// The completions will be emitted as typed completion results, one-by-one, directly from the shell.
    /// After a completion is emitted, it can still be updated; see [`ShellCompletion::update`].
    IncrementallyTyped { output: Vec<ShellCompletion> },
}

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
}

impl ShellData {
    /// Returns the corresponding `ShellData` given a format type.
    pub fn from_format_type(format: &str) -> Option<ShellData> {
        match format {
            "raw" => Some(ShellData::Raw {
                output: Default::default(),
            }),
            "incrementally_typed" => Some(ShellData::IncrementallyTyped {
                output: Default::default(),
            }),
            _ => None,
        }
    }
}

impl From<ShellData> for Vec<ShellCompletion> {
    fn from(shell_data: ShellData) -> Self {
        match shell_data {
            // TODO(suraj): Determine the correct parsing strategy for raw shell completion data.
            ShellData::Raw { output } => output
                .split_whitespace()
                .map(|name| ShellCompletion::new(name.into()))
                .collect(),
            ShellData::IncrementallyTyped { mut output } => {
                // TODO: we need to get metadata from the shell about how the results
                // should be sorted (intra- and inter-groups).
                output.sort_by(|a, b| a.name.cmp(&b.name));
                output
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

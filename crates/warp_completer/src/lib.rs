pub mod completer;
pub mod meta;
pub mod parsers;
pub mod signatures;
pub mod util;

/// Stores the vector of parsed commands at a particular point in time (for some
/// buffer text and completion context).
#[derive(Clone)]
struct ParsedCommandsSnapshot<'a, T: completer::CompletionContext> {
    buffer_text: String,
    parsed_commands: Vec<parsers::LiteCommand>,
    completion_context: &'a T,
}

/// Stores the vector of parsed tokens (for all relevant commands in buffer)
/// at a particular point in time (for some buffer text and completion context),
/// so we can cache results.
#[derive(Debug, Clone)]
pub struct ParsedTokensSnapshot {
    pub buffer_text: String,
    pub parsed_tokens: Vec<ParsedTokenData>,
}

/// Stores all relevant data to describe a single parsed token in the context
/// of a single command (not across commands).
/// Example for `ls -a && git checkout -b`:
/// We'd have (pseudo-code):
/// [
///     {
///         token: {value: "ls", span: (0, 2)},
///         token_index: 0,
///         token_description: {
///                                suggestion_type: SuggestionType::Command,
///                                ...
///                            }
///     },
///     {
///         token: {value: "-a", span: (3, 5)},
///         token_index: 1,
///         ...
///     },
///     {
///         token: {value: "git", span: (9, 12)},
///         token_index: 0,
///         ...
///     },
///     {
///         token: {value: "checkout", span: (13, 21)},
///         token_index: 1,
///         ...
///     },
///     {
///         token: {value: "-b", span: (22, 24)},
///         token_index: 2,
///         ...
///     },
/// ]
#[derive(Clone, Debug)]
pub struct ParsedTokenData {
    pub token: meta::Spanned<String>,
    pub token_index: usize, // relative to a specific command
    pub token_description: Option<completer::Description>,
}

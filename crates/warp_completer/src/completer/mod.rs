mod coalesce;
mod context;
mod describe;
mod engine;
mod matchers;
mod suggest;
pub use suggest::alias::*;

#[cfg(feature = "test-util")]
pub mod testing;

pub use context::{
    CommandExitStatus, CommandOutput, CompletionContext, GeneratorContext, PathCompletionContext,
    PathSeparators,
};
#[cfg(feature = "v2")]
pub use context::{JsExecutionContext, JsExecutionError};
pub use describe::{describe, describe_given_token, Description, TopLevelCommandCaseSensitivity};
pub use engine::{EngineDirEntry, EngineFileType, LocationType};
pub use matchers::{Match, MatchStrategy, MatchType};
pub use suggest::{
    suggestions, CompleterOptions, CompletionsFallbackStrategy, MatchedSuggestion, Priority,
    Suggestion, SuggestionResults, SuggestionType, SuggestionTypeName,
};

fn get_path_separators(ctx: &dyn CompletionContext) -> PathSeparators {
    ctx.path_completion_context()
        .map(|ctx| ctx.path_separators())
        .unwrap_or(PathSeparators::for_os())
}

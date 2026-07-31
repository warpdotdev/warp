use std::collections::HashMap;
use std::future::Future;

use warp_completer::completer::{
    self, CompleterOptions, CompletionContext, CompletionsFallbackStrategy, MatchStrategy,
    SuggestionResults,
};
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences;
use warpui::AppContext;

use crate::terminal::model::completions::ShellCompletion;
use crate::terminal::model::session::Session;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionSourcePolicy {
    use_native_shell_completions: bool,
    force_native_shell_completions: bool,
}

impl CompletionSourcePolicy {
    pub fn for_session(session: &Session, buffer_text: &str, ctx: &AppContext) -> Self {
        let force_native_shell_completions = ctx
            .private_user_preferences()
            .read_value("ForceNativeShellCompletions")
            .ok()
            .flatten()
            .and_then(|value| value.parse().ok())
            .unwrap_or(false);
        Self::from_inputs(
            FeatureFlag::NativeShellCompletions.is_enabled(),
            force_native_shell_completions,
            session.shell().supports_native_shell_completions(),
            buffer_text.contains('\n'),
        )
    }

    fn from_inputs(
        native_shell_completions_enabled: bool,
        force_native_shell_completions: bool,
        shell_supports_native_completions: bool,
        is_multiline: bool,
    ) -> Self {
        Self {
            use_native_shell_completions: (native_shell_completions_enabled
                || force_native_shell_completions)
                && shell_supports_native_completions
                && !is_multiline,
            force_native_shell_completions,
        }
    }

    pub fn should_request_native_shell_completions(self) -> bool {
        self.use_native_shell_completions
    }

    pub fn fallback_strategy(
        self,
        fallback_when_native_is_unavailable: CompletionsFallbackStrategy,
    ) -> CompletionsFallbackStrategy {
        if self.use_native_shell_completions {
            CompletionsFallbackStrategy::None
        } else {
            fallback_when_native_is_unavailable
        }
    }
}

pub async fn completion_suggestions_with_native_fallback<T, F>(
    buffer_text: &str,
    cursor_position: usize,
    session_env_vars: Option<&HashMap<String, String>>,
    mut options: CompleterOptions,
    policy: CompletionSourcePolicy,
    native_results: F,
    ctx: &T,
) -> Option<SuggestionResults>
where
    T: CompletionContext,
    F: Future<Output = Option<Vec<ShellCompletion>>>,
{
    let before_cursor_text = buffer_text.get(..cursor_position)?;
    options.fallback_strategy = policy.fallback_strategy(options.fallback_strategy);
    let warp_results = completer::suggestions(
        before_cursor_text,
        before_cursor_text.len(),
        session_env_vars,
        options,
        ctx,
    )
    .await;

    resolve_completion_results(
        warp_results,
        native_results,
        before_cursor_text,
        policy.force_native_shell_completions,
    )
    .await
}

async fn resolve_completion_results<F>(
    warp_results: Option<SuggestionResults>,
    native_results: F,
    before_cursor_text: &str,
    force_native_shell_completions: bool,
) -> Option<SuggestionResults>
where
    F: Future<Output = Option<Vec<ShellCompletion>>>,
{
    if let Some(warp_results) = warp_results
        && !warp_results.suggestions.is_empty()
        && !force_native_shell_completions
    {
        return Some(warp_results);
    }

    native_results.await.map(|results| {
        let token_end = before_cursor_text.len();
        let token_start = before_cursor_text
            .rfind(char::is_whitespace)
            .map(|position| position + 1)
            .unwrap_or_default();
        SuggestionResults {
            replacement_span: (token_start, token_end).into(),
            suggestions: results.into_iter().map(Into::into).collect(),
            match_strategy: MatchStrategy::Fuzzy,
        }
    })
}

#[cfg(test)]
#[path = "completion_source_tests.rs"]
mod tests;
